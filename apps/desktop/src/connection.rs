use client_core::{
	COMMAND_SLOTS, Command, EVENT_SLOTS, Envelope, Event, MAX_EVENT_BYTES,
	auth::{AuthProvider, Failure, SessionSecret},
};
use discord_api::DiscordApi;
use eframe::egui;
use std::{
	collections::BTreeMap,
	sync::{
		Arc, Mutex,
		atomic::{AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};
use tokio::{
	runtime::Handle,
	sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
	task::JoinHandle,
};

pub struct Connection {
	pub commands: mpsc::Sender<Command>,
	pub uploads: mpsc::Sender<crate::uploads::UploadRequest>,
	pub events: ReliableEvents,
	pub typing: mpsc::Receiver<Envelope>,
	pub terminal: watch::Receiver<Option<Failure>>,
	/// One fixed-size candidate failure, independent of the account event queue.
	pub confirmation_failure: watch::Receiver<Option<ConfirmationFailure>>,
	pub share_activity: watch::Sender<bool>,
	pub custom_rich_presence: watch::Sender<Option<extensions::CustomRichPresence>>,
	pub own_presence: watch::Sender<model::OwnPresence>,
	/// Local edits only. Seeding from Discord does not publish through this watch.
	pub presence_edits: watch::Sender<Option<model::OwnPresence>>,
	/// The status chosen for this connection, once Discord or the local fallback is known.
	pub account_presence: watch::Receiver<Option<model::OwnPresence>>,
	pub presence_error: watch::Receiver<Option<&'static str>>,
	pub game_activity: watch::Receiver<crate::game_activity::Detection>,
	pub registered_games: watch::Sender<Vec<model::registered_games::RegisteredGame>>,
	pub running_game: watch::Receiver<Option<model::registered_games::RunningGame>>,
	pub spotify_activity: watch::Receiver<Option<discord_protocol::spotify::Activity>>,
	/// A local Rich Presence client asked the client to show an invite: counter and code.
	pub rpc_invite: watch::Receiver<Option<(u64, String)>>,
	pub activity_observation: watch::Receiver<discord_gateway::ActivityObservation>,
	pub activity_sharing: watch::Receiver<Result<Option<bool>, Failure>>,
	pub activity_sharing_request: mpsc::Sender<bool>,
	typing_channel: Arc<AtomicU64>,
	task: JoinHandle<()>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmationFailure {
	generation: u64,
	channel: model::Id,
	request: u64,
	revision: u64,
	message: &'static str,
}
impl ConfirmationFailure {
	fn envelope(self) -> Envelope {
		Envelope {
			generation: self.generation,
			event: Event::Voice(client_core::voice::Event::SessionConfirmationFailed {
				channel: self.channel,
				request: self.request,
				revision: self.revision,
				message: self.message,
			}),
		}
	}
}
pub fn take_confirmation_failure(
	receiver: &mut watch::Receiver<Option<ConfirmationFailure>>,
	events: &ReliableEvents,
) -> Option<Envelope> {
	// A replacement candidate/ACK may still be behind this frame's reliable batch.
	// Keep the report unseen until preceding signaling is consumed; the original
	// negotiation deadline still bounds failure if sustained events never drain.
	if !events.receive.is_empty() {
		return None;
	}
	// Ref::has_changed preserves an unread final version after publisher shutdown.
	// Release its read lock before handing the report to the event consumer.
	let failure = {
		let report = receiver.borrow_and_update();
		report.has_changed().then_some(*report).flatten()
	};
	failure.map(ConfirmationFailure::envelope)
}
impl Drop for Connection {
	fn drop(&mut self) {
		self.task.abort();
	}
}
struct AbortTask(JoinHandle<()>);
impl Drop for AbortTask {
	fn drop(&mut self) {
		self.0.abort();
	}
}
impl Connection {
	pub fn set_typing_channel(&self, channel: Option<model::Id>) {
		self.typing_channel
			.store(channel.map_or(0, |id| id.0), Ordering::Relaxed);
	}
	pub fn start(
		runtime: &Handle,
		secret: Arc<SessionSecret>,
		generation: u64,
		expected_user: Option<model::Id>,
		cached_presence: BTreeMap<model::Id, model::OwnPresence>,
		api_proxy: watch::Receiver<Option<discord_api::proxy::ApiProxy>>,
		ctx: egui::Context,
	) -> Self {
		let (commands, mut receive) = mpsc::channel(COMMAND_SLOTS);
		let (uploads, mut upload_receive) = mpsc::channel::<crate::uploads::UploadRequest>(1);
		let (send, events) = reliable_events(ctx.clone());
		let (typing_send, typing) = mpsc::channel(8);
		let (finished, terminal) = watch::channel(None);
		let (confirmation_report, confirmation_failure) = watch::channel(None);
		let (share_activity, share_receive) = watch::channel(false);
		let (custom_rich_presence, custom_receive) = watch::channel(None);
		let (own_presence, presence_receive) = watch::channel(model::OwnPresence::default());
		let (presence_edits, presence_edit_events) = watch::channel(None);
		let (account_presence_send, account_presence) = watch::channel(None);
		let (presence_error_send, presence_error) = watch::channel(None);
		let presence_send = own_presence.clone();
		let (game_report, game_activity) = watch::channel(Ok(None));
		let (registered_games, registered_receive) = watch::channel(Vec::new());
		let (running_send, running_game) = watch::channel(None);
		let (spotify_send, spotify_activity) = watch::channel(None);
		let spotify_receive = spotify_activity.clone();
		let (invite_send, rpc_invite) = watch::channel(None);
		let (activity_observed, activity_observation) =
			watch::channel(discord_gateway::ActivityObservation::Unconfirmed);
		let (sharing_report, activity_sharing) = watch::channel(Ok(None));
		let (activity_sharing_request, sharing_requests) = mpsc::channel(1);
		let wake = ctx.clone();
		let typing_channel = Arc::new(AtomicU64::new(0));
		let active_typing = typing_channel.clone();
		let typing_gate = Mutex::new(TypingGate::default());
		let status_changed = Arc::new(tokio::sync::Notify::new());
		let status_refresh = status_changed.clone();
		let task=runtime.spawn(async move {
            let emit=move |event:Event| -> Result<(),Failure> {
                if let Event::AccountSettings { status: true, .. } = &event { status_changed.notify_one(); }
                if let Event::Typing(signal) = &event {
                    let active = active_typing.load(Ordering::Relaxed);
                    if !typing_gate.lock().is_ok_and(|mut gate| gate.accept(*signal, active, Instant::now())) { return Ok(()); }
                }
                emit_event(&send, &typing_send, Envelope {generation,event}, &ctx)
            };
            let result=async {
                let mut api=DiscordApi::with_proxy(secret.clone(), api_proxy)?;
                let user=api.authenticate().await?;
                if expected_user.is_some_and(|id|id!=user.id){return Err(Failure::InvalidCredential);}
                let gateway=api.gateway_url().await?;
                let api=Arc::new(api);
				let cached = cached_presence.get(&user.id).cloned().filter(|presence| presence.valid());
				let account_presence_send = Arc::new(account_presence_send);
				let _presence_edits = AbortTask(tokio::spawn(run_presence_sync(PresenceSync {
					api: api.clone(),
					edits: presence_edit_events,
					remote_changed: status_refresh,
					presence: presence_send.clone(),
					account: account_presence_send.clone(),
					note: presence_error_send,
					finished: finished.clone(),
					wake: wake.clone(),
				})));
				let chosen = resolve_account_presence(&api, &presence_send, cached).await;
				if chosen.is_some() {
					wake.request_repaint();
				}
				let _ = account_presence_send.send_replace(chosen);
                let emit=Arc::new(emit);
                let (member_send,member_receive)=watch::channel(None);
                let (voice_send,voice_receive)=mpsc::channel(8);
				let (activity_send,activity_receive)=watch::channel(None);
				let (member_query_send, member_query_receive) = watch::channel([None, None]);
				let _sharing_task=AbortTask(tokio::spawn(run_activity_sharing(api.clone(),share_receive.clone(),sharing_requests,sharing_report,finished.clone(),wake.clone())));
				let _activity_task=AbortTask(tokio::spawn(crate::game_activity::run((share_receive,custom_receive,crate::game_activity::Registered{games:registered_receive,current:running_send}),activity_send,game_report,invite_send,wake.clone(),user.clone(),api.clone())));
				let _spotify_task=AbortTask(tokio::spawn(crate::spotify::run(api.clone(),user.id,presence_receive.clone(),spotify_send,wake.clone())));
                let dm_channels=Arc::new(Mutex::new(BTreeMap::new()));
                let (recipient_scope,mut recipient_scope_changed)=watch::channel(0u64);
                let recipient_call=Arc::new(Mutex::new(RecipientCalls::default()));
                let gateway_recipient_call=recipient_call.clone();
                let gateway_channels=dm_channels.clone();
                let (voice_online,mut voice_availability)=watch::channel(false);
                let (takeover_send,mut takeover_receive)=watch::channel(None);
                let gateway_takeover=takeover_send.clone();
                let gateway_api=api.clone();let gateway_emit=emit.clone();let terminal_send=finished.clone();
                let gateway_wake=wake.clone();
                let activity_wake=wake.clone();
                let mut gateway_task=AbortTask(tokio::spawn(async move {
                    let error=discord_gateway::run_with_activity(secret,gateway,member_receive,voice_receive,(activity_receive,presence_receive,member_query_receive,spotify_receive),move |observation| {
                        if activity_observed.send_if_modified(|current| { if *current == observation { false } else { *current = observation; true } }) { activity_wake.request_repaint(); }
                        Ok(())
                    },|event|{
                        if let Event::Interaction(client_core::interactions::Event::Session(session)) = event { return gateway_api.interaction_session(Some(session)); }
                        if matches!(&event,Event::Disconnected|Event::Resync) { gateway_api.interaction_session(None)?; }
                        if let Some((ready_user,_,channels))=event.ready_navigation() {
                            if ready_user.id!=user.id {return Err(Failure::InvalidCredential);}
                            *gateway_channels.lock().map_err(|_|Failure::Protocol)?=channels.iter().filter(|c|private_call(c)).take(client_core::MAX_NAV).map(|c|(c.id,recipient_ids(c))).collect();
                        }
                        if let Event::ChannelCreated(channel)=&event {
                            let mut channels=gateway_channels.lock().map_err(|_|Failure::Protocol)?;
                            channels.remove(&channel.id);
                            if private_call(channel) && channels.len()<client_core::MAX_NAV {channels.insert(channel.id,recipient_ids(channel));}
                        }
                        if let Event::Unavailable(channel)=&event {gateway_channels.lock().map_err(|_|Failure::Protocol)?.remove(channel);}
                        if let Event::RecipientRemoved {channel,user:removed}=&event {
                            let mut channels=gateway_channels.lock().map_err(|_|Failure::Protocol)?;
                            if *removed==user.id {channels.remove(channel);}
                            else if let Some(recipients)=channels.get_mut(channel) {recipients.retain(|id|id!=removed);}
                        }
                        if let Event::RecipientAdded {channel,user:added}=&event && let Some(recipients)=gateway_channels.lock().map_err(|_|Failure::Protocol)?.get_mut(channel) && !recipients.contains(&added.id) {
                            if recipients.len()+1<client_core::voice::MAX_PARTICIPANTS {recipients.push(added.id);} else {recipients.clear();}
                        }
                        let invalidates_recipient = {
                            let channels=gateway_channels.lock().map_err(|_|Failure::Protocol)?;
                            let mut calls=gateway_recipient_call.lock().map_err(|_|Failure::Protocol)?;
                            calls.observe(&event,user.id,&channels)
                        };
                        if invalidates_recipient {
                            recipient_scope.send_modify(|revision|*revision=revision.wrapping_add(1));
                        }
                        if let Event::ChannelChanged(patch)=&event && let model::Patch::Value(kind)=patch.kind && !matches!(kind,1|3) {gateway_channels.lock().map_err(|_|Failure::Protocol)?.remove(&patch.id);}

                        if event.ready_navigation().is_some() || matches!(&event,Event::Resumed) {let _=voice_online.send(true);}
                        if matches!(&event,Event::Disconnected|Event::Resync) {let _=voice_online.send(false);}
                        if let Event::Voice(client_core::voice::Event::TakenOver{channel,request})=&event {let _=gateway_takeover.send_replace(Some((*channel,*request)));}
                        gateway_emit(event)
                    }).await.err().unwrap_or(Failure::Network).protocol_at("Gateway connection: unsupported handshake or event");
                    gateway_api.stop();let _=terminal_send.send(Some(error));gateway_wake.request_repaint();
                }));
                // Keep hangup/mute controls responsive while an HTTP message write is awaiting Discord.
                let (write_send,mut write_receive)=mpsc::channel(COMMAND_SLOTS);
                let write_api=api.clone();let write_emit=emit.clone();let write_finished=finished.clone();let write_wake=wake.clone();
                let mut writes=AbortTask(tokio::spawn(async move {
                    while let Some(command)=write_receive.recv().await {
                        let event=write_api.execute(command).await;
                        let failure=match &event {Event::Interaction(client_core::interactions::Event::Submitted{result:Err(f),..})=>Some(*f),Event::MessagingPermissions{result:Err(f),..}=>Some(*f),Event::ChannelAction(client_core::channel_actions::Event::Finished{result:Err(f),..})=>Some(*f),Event::ServerAdmin(client_core::server_admin::Event{result:Err(f),..})=>Some(*f),Event::ServerSettings(client_core::server_settings::Event{result:Err(f),..})=>Some(*f),Event::Onboarding(client_core::onboarding::Event::Loaded{result:Err(f),..}|client_core::onboarding::Event::Submitted{result:Err(f),..})=>Some(*f),Event::Failure(f)=>Some(*f),Event::ProfileEdited{result:Err(f),..} if *f != Failure::Capacity =>Some(*f),Event::Edited{result:Err(f),..}|Event::Pinned{result:Err(f),..}=>Some(*f),Event::GuildFolders(Err(f))=>Some(*f),Event::JoinInvite{result:Err(f),..}|Event::GuildCreated{result:Err(f),..}=>Some(*f),Event::SendResult{result:Err(f),..}=>Some(*f),Event::UserAction(client_core::user_actions::Event::Written{result:Err(f),..})=>Some(*f),Event::UserAction(client_core::user_actions::Event::DmOpened{result:Err(f),..})=>Some(*f),Event::ServerAction(client_core::server_actions::Event::Written{result:Err(f),..})=>Some(*f),Event::ServerAction(client_core::server_actions::Event::InviteSent{result:Err(f),..})=>Some(*f),Event::GroupAction(client_core::group_actions::Event::Written{result:Err(f),..})=>Some(*f),Event::Reactions(client_core::reactions::Event::Written{result:Err(f),..})=>Some(*f),Event::ReadState(client_core::read_state::Event::Result{result:Err(f),..})=>Some(*f),_=>None};
                        let error=write_emit(event).err().or(failure.filter(|f|f.ends_session()));
                        if let Some(error)=error {write_api.stop();let _=write_finished.send(Some(error));write_wake.request_repaint();break;}
                    }
                }));
                let mut history:Option<AbortTask>=None;
                let mut profile:Option<AbortTask>=None;
				let mut stream_preview:Option<AbortTask>=None;
                let mut invite:Option<AbortTask>=None;
                let mut search:Option<AbortTask>=None;
                let mut gifs:Option<AbortTask>=None;
                let mut gif_favorites:Option<AbortTask>=None;
                let mut application_commands:Option<AbortTask>=None;
                let mut sticker_packs:Option<AbortTask>=None;
                let mut sticker_detail:Option<AbortTask>=None;
                let mut reaction_read:Option<AbortTask>=None;
                let mut ringing:Option<AbortTask>=None;
                let mut recipient_ringing:Option<(client_core::voice::Command,AbortTask)>=None;
                let mut upload:Option<AbortTask>=None;
                let mut upload_cancel:Option<watch::Sender<bool>>=None;
                let mut voice_request=None;
                // One local release may wait for queue space; later joins cannot overtake it.
                let mut pending_abandonment=None;
                loop {
                    tokio::select! {
                        _=&mut gateway_task.0=>{break;}
                        _=&mut writes.0=>{break;}
                        changed=recipient_scope_changed.changed()=> {
                            if changed.is_err() {break;}
                            recipient_scope_changed.borrow_and_update();
                            let channels=dm_channels.lock().map_err(|_|Failure::Protocol)?;
                            let calls=recipient_call.lock().map_err(|_|Failure::Protocol)?;
                            if recipient_ringing.as_ref().is_some_and(|(control,_)| !recipient_write_allowed(*control,&calls,&channels)) {drop(recipient_ringing.take());}
                        }
                        permit=voice_send.reserve(), if pending_abandonment.is_some()=> {
                            if let Ok(permit)=permit {
                                permit.send(pending_abandonment.take().expect("pending abandonment"));
                            } else {
                                // The Gateway task owns connection termination when its queue closes.
                                pending_abandonment=None;
                            }
                        }
                        changed=takeover_receive.changed()=> {
                            if changed.is_err() {break;}
                            if release_taken_over(&mut voice_request,*takeover_receive.borrow_and_update()) {drop(ringing.take());drop(recipient_ringing.take());recipient_call.lock().map_err(|_|Failure::Protocol)?.active=None;}
                        }
                        changed=voice_availability.changed()=> {
							if changed.is_err() {break;}
							if !*voice_availability.borrow_and_update() {drop(ringing.take());drop(recipient_ringing.take());drop(profile.take());drop(stream_preview.take());drop(search.take());voice_request=None;recipient_call.lock().map_err(|_|Failure::Protocol)?.active=None;if let Some(cancel)=&upload_cancel {let _=cancel.send(true);}}
                        }
                        request=upload_receive.recv()=>{
                            let Some(request)=request else {break;};
                            if !*voice_availability.borrow() || upload.as_ref().is_some_and(|job|!job.0.is_finished()) {
                                match request.command {
                                    Command::Interaction(request)=>emit(Event::Interaction(client_core::interactions::Event::Submitted{nonce:request.nonce,result:Err(Failure::ProtocolAt("Upload unavailable; reselect the file to retry"))}))?,
                                    Command::Send{nonce,..}=>emit(Event::SendResult{nonce,result:Err(Failure::ProtocolAt("Upload unavailable; reselect the file to retry"))})?,
                                    Command::CreatePost{parent,request,..}=>emit(Event::PostCreated{parent,request,result:Err(Failure::ProtocolAt("Upload unavailable; reselect the file to retry"))})?,
                                    _=>{}
                                }
                                request.progress.send_replace(discord_api::upload::Status::Failed("Upload unavailable; reselect the file to retry"));
                                continue;
                            }
                            upload_cancel=Some(request.cancel.clone());
                            let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
                            upload=Some(AbortTask(tokio::spawn(async move {
                                let mut updates=request.progress.subscribe();
                                let operation=api.upload_messages(request.command,request.source,request.progress,request.cancel.subscribe());
                                tokio::pin!(operation);
                                let mut observing=true;
                                let event=loop {
                                    tokio::select! {
                                        event=&mut operation=>break event,
                                        changed=updates.changed(), if observing=>{observing=changed.is_ok();wake.request_repaint();}
                                    }
                                };
                                let failure=match &event {Event::Interaction(client_core::interactions::Event::Submitted{result:Err(f),..}) | Event::SendResult{result:Err(f),..} if f.ends_session()=>Some(*f),_=>None};
                                let error=emit(event).err().or(failure);
                                if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
                                wake.request_repaint();
                            })));
                        }
                        command=receive.recv()=>{
                            // Select can admit a queued command before the changed-watch branch.
                            if release_taken_over(&mut voice_request,*takeover_receive.borrow()) {drop(ringing.take());drop(recipient_ringing.take());recipient_call.lock().map_err(|_|Failure::Protocol)?.active=None;}
                            let Some(command)=command else {break;};
                            if matches!(command,Command::CancelSearch) {drop(search.take());continue;}
                            if matches!(command,Command::CancelGifs) {drop(gifs.take());continue;}
                            if matches!(command,Command::GifFavorites{..}) {
                                drop(gif_favorites.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
                                gif_favorites=Some(AbortTask(tokio::spawn(async move {
                                    let event=api.execute(command).await;
                                    let failure=match &event {Event::GifFavorites{result:Err(f),..} if f.ends_session() && *f!=Failure::Capacity=>Some(*f),_=>None};
                                    let error=emit(event).err().or(failure);
                                    if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
                                    wake.request_repaint();
                                })));
                                continue;
                            }
                            if matches!(command,Command::ApplicationCommands{..}) {
                                drop(application_commands.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
                                application_commands=Some(AbortTask(tokio::spawn(async move {
                                    let event=api.execute(command).await;
                                    let failure=match &event {Event::ApplicationCommands{result:Err(f),..} if f.ends_session() && *f!=Failure::Capacity=>Some(*f),_=>None};
                                    let error=emit(event).err().or(failure);
                                    if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
                                    wake.request_repaint();
                                })));
                                continue;
                            }
                            if matches!(command,Command::StickerPacks|Command::Sticker(_)) {
                                let task=if matches!(command,Command::StickerPacks) {&mut sticker_packs} else {&mut sticker_detail};
                                drop(task.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
                                *task=Some(AbortTask(tokio::spawn(async move {
                                    let event=api.execute(command).await;
                                    let failure=match &event {Event::StickerPacks(Err(f))|Event::Sticker{result:Err(f),..} if f.ends_session() && *f!=Failure::Capacity=>Some(*f),_=>None};
                                    let error=emit(event).err().or(failure);
                                    if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
                                    wake.request_repaint();
                                })));
                                continue;
                            }
                            if matches!(command,Command::Gifs{..}) {
                                drop(gifs.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
                                gifs=Some(AbortTask(tokio::spawn(async move {
                                    let event=api.execute(command).await;
                                    let failure=match &event {Event::Gifs{result:Err(f),..} if f.ends_session() && *f!=Failure::Capacity=>Some(*f),_=>None};
                                    let error=emit(event).err().or(failure);
                                    if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
                                    wake.request_repaint();
                                })));
                                continue;
                            }
                            if matches!(command,Command::Search{..}|Command::Pins{..}|Command::Archives{..}) {
                                drop(search.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
                                search=Some(AbortTask(tokio::spawn(async move {
                                    let event=api.execute(command).await;
                                    let failure=match &event {Event::Search{result:Err(f),..}|Event::Archives{result:Err(f),..} if f.ends_session() && *f!=Failure::Capacity=>Some(*f),_=>None};
                                    let error=emit(event).err().or(failure);
                                    if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
                                    wake.request_repaint();
                                })));
                                continue;
                            }
                            if matches!(command,Command::Reactions(client_core::reactions::Command::Read{..})) {
                                drop(reaction_read.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
                                reaction_read=Some(AbortTask(tokio::spawn(async move {
                                    let event=api.execute(command).await;
                                    let failure=match &event {Event::Reactions(client_core::reactions::Event::Read{result:Err(f),..}) if f.ends_session()=>Some(*f),_=>None};
                                    let error=emit(event).err().or(failure);
                                    if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
                                    wake.request_repaint();
                                })));
                                continue;
                            }
							if let Command::Voice(control @ client_core::voice::Command::Sync { channel }) = &command {
								if *voice_availability.borrow() && dm_channels.lock().map_err(|_|Failure::Protocol)?.contains_key(channel) {
									voice_send.try_send(*control).map_err(|_|Failure::Capacity)?;
								}
								continue;
							}
							if let Command::Voice(control @ (client_core::voice::Command::StartStream{..}|client_core::voice::Command::StopStream{..}))=&command {
								use client_core::{screen,voice::Event as E};
								let control=*control;
								let (channel,request,stream_request)=match control {
									client_core::voice::Command::StartStream{channel,request,stream_request}
									|client_core::voice::Command::StopStream{channel,request,stream_request}=>(channel,request,stream_request),
									_=>unreachable!(),
								};
								let message=if !*voice_availability.borrow() {
									Some("Voice signaling is disconnected; screen-share action was not sent")
								} else if voice_send.try_send(control).is_err() {
									Some("Screen-share action was not sent; the voice queue is full")
								} else {None};
								if let Some(message)=message {
									emit(Event::Voice(E::Stream{channel,request,stream_request,event:screen::Event::Failed(message)}))?;
								}
								continue;
							}
							if let Command::Voice(control @ (client_core::voice::Command::WatchStream{..}|client_core::voice::Command::StopWatching{..}))=&command {
								use client_core::{screen,voice::{Command as V,Event as E}};
								let control=*control;
								let (channel,request,stream_request,streamer)=match control {
									V::WatchStream{channel,request,stream_request,streamer}=>(channel,request,stream_request,Some(streamer)),
									V::StopWatching{channel,request,stream_request}=>(channel,request,stream_request,None),
									_=>unreachable!(),
								};
								let message=if !*voice_availability.borrow() {
									Some("Voice signaling is disconnected; the stream request was not sent")
								} else if voice_send.try_send(control).is_err() {
									Some("Stream request was not sent; the voice queue is full")
								} else {None};
								if let (Some(message),Some(streamer))=(message,streamer) {
									emit(Event::Voice(E::Watch{channel,request,stream_request,streamer,event:screen::Event::Failed(message)}))?;
								}
								continue;
							}
                            if let Command::Voice(client_core::voice::Command::RingRecipient{channel,request,recipient,stop})=&command {
                                use client_core::voice::Event as E;
                                let (channel,request,recipient,stop)=(*channel,*request,*recipient,*stop);
                                // Consume prior invalidations before starting a write for the current revision.
                                recipient_scope_changed.borrow_and_update();
                                let eligible=recipient_action(channel,request,recipient,user.id,voice_request,dm_channels.lock().map_err(|_|Failure::Protocol)?.get(&channel).map(Vec::as_slice))
                                    && recipient_call.lock().map_err(|_|Failure::Protocol)?.as_ref().is_some_and(|call|call.allows(channel,request,recipient,stop));
                                let message=if !*voice_availability.borrow() {Some("Recipient ringing unavailable while disconnected")}
                                    else if !eligible {Some("Recipient ringing expired; no request was sent")}
                                    else if recipient_ringing.as_ref().is_some_and(|(_,job)|!job.0.is_finished()) {Some("Recipient ringing is already pending; wait for the result")}
                                    else {None};
                                if let Some(message)=message {emit(Event::Voice(E::RingFailed{channel,request,message}))?;continue;}
                                drop(recipient_ringing.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let ring_wake=wake.clone();
                                let takeover=takeover_receive.clone();
                                let members=recipient_scope_changed.clone();
                                let available=voice_availability.clone();
                                let call_metadata=recipient_call.clone();let call_members=dm_channels.clone();
                                let control=client_core::voice::Command::RingRecipient{channel,request,recipient,stop};
                                recipient_ringing=Some((control,AbortTask(tokio::spawn(async move {
                                    let result=tokio::select! {
                                        biased;
                                        _=wait_for_takeover(takeover,(channel,request))=>return,
                                        _=wait_for_recipient_invalidation(members,available,control,call_metadata,call_members)=>return,
                                        result=api.ring_call(channel,Some(recipient),stop)=>result,
                                    };
                                    if let Err(failure)=result {
                                        let _=emit(Event::Voice(E::RingFailed{channel,request,message:failure.label()}));
                                        if failure.ends_session(){api.stop();let _=finished.send(Some(failure));}
                                    }
                                    ring_wake.request_repaint();
                                }))));
                                continue;
                            }
							if let Command::Voice(control)=command {
                                use client_core::voice::{Command as V,Event as E};
                                let (channel,request)=match control {V::AbandonSession{channel,request}|V::ConfirmSession{channel,request,..}|V::Join{channel,request,..}|V::Ring{channel,request}|V::Leave{channel,request}|V::SetMute{channel,request,..}|V::SetCamera{channel,request,..}=>(channel,request),V::Decline{channel}=>(channel,0),V::RingRecipient{..}|V::Sync{..}|V::StartStream{..}|V::StopStream{..}|V::WatchStream{..}|V::StopWatching{..}=>unreachable!("sync and stream actions routed above")};
                                if let V::AbandonSession{channel,request}=control {
                                    let owner=voice_request.map(|(channel,request,_)|(channel,request));
                                    if voice_request.is_some_and(|(id,r,_)|id==channel && r==request) {
                                        let _=takeover_send.send_replace(Some((channel,request)));
                                        let _=release_taken_over(&mut voice_request,Some((channel,request)));
                                        drop(ringing.take());drop(recipient_ringing.take());
                                        recipient_call.lock().map_err(|_|Failure::Protocol)?.active=None;
                                    }
                                    queue_abandonment(&voice_send,&mut pending_abandonment,control,owner);
                                    continue;
                                }
                                if matches!(control,V::ConfirmSession{..}) {
                                    if let Some(error)=queue_confirmation(&voice_send,control,voice_request,*voice_availability.borrow()) {report_confirmation_failure(&confirmation_report,generation,error,voice_request,&wake);}
                                    continue;
                                }
                                if abandonment_blocks_join(pending_abandonment,control) {
                                    emit(Event::Voice(E::Failed{channel,request,message:"Previous call is releasing locally; retry joining after it finishes"}))?;continue;
                                }
                                if !*voice_availability.borrow() {
                                    emit(Event::Voice(E::Failed{channel,request,message:"Voice is disconnected; no call was started"}))?;continue;
                                }
                                if join_taken_over(control,*takeover_receive.borrow()) {
                                    emit(Event::Voice(E::Failed{channel,request,message:"Call attempt ended; join again to start a new attempt"}))?;continue;
                                }
                                if matches!(control,V::Join{..}) {drop(ringing.take());drop(recipient_ringing.take());}
                                if let V::Leave{channel,request}=control && voice_request.is_some_and(|(id,r,_)|id==channel && r==request) {drop(ringing.take());drop(recipient_ringing.take());recipient_call.lock().map_err(|_|Failure::Protocol)?.active=None;}
                                let ring=match ring_action(control,user.id,&mut voice_request,dm_channels.lock().map_err(|_|Failure::Protocol)?.contains_key(&channel)) {
                                    Ok(action)=>action,
                                    Err(())=>{emit(Event::Voice(E::Failed{channel,request,message:"Call action expired; no ringing request was sent"}))?;continue;}
                                };
                                if matches!(control,V::Join{..}) {
                                    if let Some(error)=queue_join(&voice_send,control,(channel,request),&mut voice_request) {
                                        recipient_call.lock().map_err(|_|Failure::Protocol)?.active=None;
                                        emit(Event::Voice(error))?;
                                        continue;
                                    }
                                    let private=dm_channels.lock().map_err(|_|Failure::Protocol)?.contains_key(&channel);
                                    recipient_call.lock().map_err(|_|Failure::Protocol)?.join(channel,request,private);
                                } else {
                                    voice_send.try_send(control).map_err(|_|Failure::Capacity)?;
                                }
                                if let Some((recipient,stop))=ring {
                                    drop(ringing.take());
                                    let api=api.clone();let emit=emit.clone();let voice_send=voice_send.clone();let finished=finished.clone();let ring_wake=wake.clone();
                                    let takeover=takeover_receive.clone();
                                    ringing=Some(AbortTask(tokio::spawn(async move {
                                        let result=tokio::select! {
                                            biased;
                                            _=wait_for_takeover(takeover,(channel,request))=>return,
                                            result=api.ring_call(channel,recipient,stop)=>result,
                                        };
                                        if let Err(failure)=result {
                                            if !stop {let _=voice_send.try_send(V::Leave{channel,request});}
                                            let _=emit(Event::Voice(E::Failed{channel,request,message:failure.label()}));
                                            if failure.ends_session(){api.stop();let _=finished.send(Some(failure));ring_wake.request_repaint();}
                                        }
                                    })));
                                }
                                continue;
                            }
                            if matches!(command, Command::Invite {..}) {
                                drop(invite.take());
                                let api=api.clone(); let emit=emit.clone(); let finished=finished.clone(); let wake=wake.clone();
                                invite=Some(AbortTask(tokio::spawn(async move {
                                    let event=api.execute(command).await;
                                    let failure=match &event {Event::Invite{result:Err(f),..} if f.ends_session()=>Some(*f),_=>None};
                                    if let Some(error)=emit(event).err().or(failure) {api.stop();let _=finished.send(Some(error));}
                                    wake.request_repaint();
                                })));
                                continue;
                            }
                            if matches!(command,Command::CancelProfile) {drop(profile.take());continue;}
							if matches!(command,Command::StreamPreview{..}) {
								drop(stream_preview.take());
								let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
								stream_preview=Some(AbortTask(tokio::spawn(async move {
									let event=api.execute(command).await;
									let failure=stream_preview_session_failure(&event);
									let error=emit(event).err().or(failure);
									if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
									wake.request_repaint();
								})));
								continue;
							}
                            if matches!(command,Command::Profile{..}) {
                                drop(profile.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let wake=wake.clone();
                                profile=Some(AbortTask(tokio::spawn(async move {
                                    let event=api.execute(command).await;
                                    let failure=match &event {Event::Profile{result:Err(failure),..} if failure.ends_session() && *failure!=Failure::Capacity=>Some(*failure),_=>None};
                                    let error=emit(event).err().or(failure);
                                    if let Some(error)=error {api.stop();let _=finished.send(Some(error));}
                                    wake.request_repaint();
                                })));
                                continue;
                            }
                            if let Command::MemberSearch(request) = command {
                                if request.valid() {
                                    member_query_send.send_modify(|queries| { let slot=request.slot; queries[slot]=Some(request); });
                                }
                                continue;
                            }
                            if let Command::Members {guild,channel,request,list_id,thread,ranges} = command {
                                let subscription=match (guild,channel,list_id) {
                                    (Some(guild),Some(channel),list_id) if thread || list_id.is_some() => Some(discord_gateway::MemberSubscription {guild,channel,request,thread,list_id:list_id.unwrap_or_default(),ranges}),
                                    _=>None
                                };
                                member_send.send(subscription).map_err(|_|Failure::Network)?;
                                continue;
                            }
                            if let Command::History { channel, request, .. } = &command {
                                drop(search.take());
                                drop(reaction_read.take());
                                let (channel, request) = (*channel, *request);
                                drop(history.take());
                                let api=api.clone();let emit=emit.clone();let finished=finished.clone();let history_wake=wake.clone();
                                history=Some(AbortTask(tokio::spawn(async move {
                                    let event=scope_history_failure(api.execute(command).await,channel,request);
                                    if let Event::Failure(f)=&event&& f.ends_session(){api.stop();let _=finished.send(Some(*f));}
                                    if let Err(f)=emit(event){api.stop();let _=finished.send(Some(f));}
                                    history_wake.request_repaint();
                                })));
                            } else {
                                write_send.try_send(command).map_err(|_|Failure::Capacity)?;
                            }
                        }
                    }
                }
                Ok::<(),Failure>(())
            }.await;
            if let Err(f)=result {let _=finished.send(Some(f));wake.request_repaint();}
        });
		Self {
			commands,
			uploads,
			events,
			typing,
			terminal,
			confirmation_failure,
			share_activity,
			own_presence,
			presence_edits,
			account_presence,
			presence_error,
			game_activity,
			registered_games,
			running_game,
			custom_rich_presence,
			spotify_activity,
			rpc_invite,
			activity_observation,
			activity_sharing,
			activity_sharing_request,
			typing_channel,
			task,
		}
	}
}

async fn resolve_account_presence(
	api: &DiscordApi,
	presence: &watch::Sender<model::OwnPresence>,
	cached: Option<model::OwnPresence>,
) -> Option<model::OwnPresence> {
	let baseline = presence.borrow().clone();
	let remote = tokio::time::timeout(Duration::from_secs(8), api.account_presence())
		.await
		.ok()
		.and_then(Result::ok);
	let edited = presence.borrow().clone() != baseline;
	if !edited && (remote.is_some() || cached.is_some()) {
		let chosen = remote.clone().or(cached.clone()).unwrap_or_default();
		let _ = presence.send_if_modified(|slot| {
			if *slot == baseline {
				*slot = chosen;
				true
			} else {
				false
			}
		});
	}
	let current = presence.borrow().clone();
	(remote.is_some() || cached.is_some() || current != baseline).then_some(current)
}

struct PresenceSync {
	api: Arc<DiscordApi>,
	edits: watch::Receiver<Option<model::OwnPresence>>,
	remote_changed: Arc<tokio::sync::Notify>,
	presence: watch::Sender<model::OwnPresence>,
	account: Arc<watch::Sender<Option<model::OwnPresence>>>,
	note: watch::Sender<Option<&'static str>>,
	finished: watch::Sender<Option<Failure>>,
	wake: egui::Context,
}

/// Saves local status edits and adopts status changed on other devices, one request at a
/// time so a remote echo never races a newer local edit. Unsaved edits retry with backoff.
async fn run_presence_sync(sync: PresenceSync) {
	const RETRY: [u64; 5] = [2, 5, 15, 30, 60];
	let PresenceSync {
		api,
		mut edits,
		remote_changed,
		presence,
		account,
		note,
		finished,
		wake,
	} = sync;
	let mut pending: Option<model::OwnPresence> = None;
	let mut failures = 0;
	let stop = |failure: Failure| {
		api.stop();
		let _ = finished.send(Some(failure));
		wake.request_repaint();
	};
	loop {
		let retry = pending
			.as_ref()
			.map(|_| Duration::from_secs(RETRY[failures.min(RETRY.len() - 1)]));
		let mut refresh = false;
		tokio::select! {
			changed = edits.changed() => {
				if changed.is_err() {
					return;
				}
				if let Some(next) = edits.borrow_and_update().clone().filter(model::OwnPresence::valid) {
					// Always write: the account API compares against a fresh read, while a
					// remembered "last saved" value goes stale once another device edits.
					pending = Some(next);
					failures = 0;
				}
			}
			() = remote_changed.notified() => refresh = pending.is_none(),
			() = tokio::time::sleep(retry.unwrap_or_default()), if retry.is_some() => {}
		}
		if let Some(next) = pending.clone() {
			match api.set_account_presence(&next).await {
				Ok(()) => {
					pending = None;
					failures = 0;
					if note.send_replace(None).is_some() {
						wake.request_repaint();
					}
				}
				Err(failure) if failure.ends_session() => return stop(failure),
				Err(_) => {
					failures += 1;
					if failures >= RETRY.len() {
						// Give up until the next edit or a settings change from Discord.
						pending = None;
					}
					let _ = note.send_replace(Some(
						"Could not save status to Discord. Retrying; it stays on this device until Discord accepts it.",
					));
					wake.request_repaint();
				}
			}
			continue;
		}
		if !refresh {
			continue;
		}
		let remote =
			match tokio::time::timeout(Duration::from_secs(8), api.account_presence()).await {
				Ok(Ok(remote)) if remote.valid() => remote,
				Ok(Err(failure)) if failure.ends_session() => return stop(failure),
				// Keep the current status; the next change or reconnect reads again.
				_ => continue,
			};
		// An edit made while reading is newer than what Discord returned.
		if edits.has_changed().unwrap_or(true) {
			continue;
		}
		let modified = presence.send_if_modified(|slot| {
			let modified = *slot != remote;
			if modified {
				slot.clone_from(&remote);
			}
			modified
		});
		let adopted = account.send_if_modified(|slot| {
			let modified = slot.as_ref() != Some(&remote);
			if modified {
				*slot = Some(remote);
			}
			modified
		});
		// Discord's value replaces any edit that was given up on.
		let cleared = note.send_replace(None).is_some();
		if modified || adopted || cleared {
			wake.request_repaint();
		}
	}
}

async fn run_activity_sharing(
	api: Arc<DiscordApi>,
	mut enabled: watch::Receiver<bool>,
	mut requests: mpsc::Receiver<bool>,
	report: watch::Sender<Result<Option<bool>, Failure>>,
	finished: watch::Sender<Option<Failure>>,
	wake: egui::Context,
) {
	let mut refresh = true;
	let mut request = None;
	loop {
		refresh |= enabled.has_changed().unwrap_or(true);
		let current = *enabled.borrow_and_update();
		if refresh {
			refresh = false;
			// An action queued before disabling sharing must not enable it on a later cycle.
			let _ = requests.try_recv();
			request = current.then_some(false);
			let _ = report.send_replace(Ok(None));
			wake.request_repaint();
		}
		if let Some(enable) = request.take() {
			let _ = report.send_replace(Ok(None));
			wake.request_repaint();
			let operation = async {
				if enable {
					api.set_activity_sharing(true).await
				} else {
					api.activity_sharing().await
				}
			};
			let result = tokio::select! {
				biased;
				changed = enabled.changed() => {
					if changed.is_err() { return; }
					refresh = true;
					continue;
				}
				result = operation => result,
			};
			let _ = report.send_replace(result.map(Some));
			wake.request_repaint();
			if let Err(failure) = result
				&& failure.ends_session()
			{
				api.stop();
				let _ = finished.send(Some(failure));
				wake.request_repaint();
				return;
			}
		}
		tokio::select! {
			biased;
			changed = enabled.changed() => {
				if changed.is_err() { return; }
				refresh = true;
			},
			next = requests.recv() => match next {
				Some(next) if *enabled.borrow() => request = Some(next),
				Some(_) => {},
				None => return,
			}
		}
	}
}

// Ordinary bursts retain their original budget. Startup uses one reserved slot in this
// same FIFO so subsequent dispatches cannot overtake the account snapshot.
const RELIABLE_ITEMS: usize = 4000 + EVENT_SLOTS;
const RELIABLE_BYTES: usize = EVENT_SLOTS * MAX_EVENT_BYTES;
struct ReliableSender {
	send: mpsc::Sender<(Envelope, OwnedSemaphorePermit)>,
	bytes: Arc<Semaphore>,
	startup: Arc<Semaphore>,
}
pub struct ReliableEvents {
	receive: mpsc::Receiver<(Envelope, OwnedSemaphorePermit)>,
	wake: egui::Context,
}
impl ReliableEvents {
	pub fn try_recv(&mut self) -> Result<Envelope, mpsc::error::TryRecvError> {
		let (envelope, _permit) = self.receive.try_recv()?;
		// The UI consumes a fixed batch each frame. Schedule another only for remaining work.
		if !self.receive.is_empty() {
			self.wake.request_repaint();
		}
		Ok(envelope)
	}
}
fn reliable_events(wake: egui::Context) -> (ReliableSender, ReliableEvents) {
	let (send, receive) = mpsc::channel(RELIABLE_ITEMS);
	(
		ReliableSender {
			send,
			bytes: Arc::new(Semaphore::new(RELIABLE_BYTES)),
			startup: Arc::new(Semaphore::new(1)),
		},
		ReliableEvents { receive, wake },
	)
}

fn emit_event(
	reliable: &ReliableSender,
	typing: &mpsc::Sender<Envelope>,
	envelope: Envelope,
	ctx: &egui::Context,
) -> Result<(), Failure> {
	if matches!(envelope.event, Event::Typing(_)) {
		// Ephemeral signals have separate fixed slots and may be dropped under pressure.
		if typing.try_send(envelope).is_ok() {
			ctx.request_repaint();
		}
		return Ok(());
	}
	let bytes = envelope.event.bytes();
	let startup = matches!(&envelope.event, Event::Startup(_) | Event::Ready { .. });
	let overhead = size_of::<(Envelope, OwnedSemaphorePermit)>() - size_of::<Event>();
	if startup && bytes.saturating_add(overhead) > model::account::MAX_BYTES {
		return Err(Failure::CapacityAt(
			"Account startup snapshot exceeds 128 MiB; connection stopped",
		));
	}
	if !startup && bytes > MAX_EVENT_BYTES {
		return Err(Failure::CapacityAt(
			"Account synchronization event exceeds 4 MiB; connection stopped",
		));
	}
	// Event::bytes includes Event itself; also charge envelope padding and the owned permit.
	let bytes = bytes + overhead;
	let permit = if startup {
		reliable.startup.clone().try_acquire_owned().map_err(|_| {
			Failure::CapacityAt("Account startup snapshot is already queued; connection stopped")
		})?
	} else {
		reliable
			.bytes
			.clone()
			.try_acquire_many_owned(bytes as u32)
			.map_err(|_| {
				Failure::CapacityAt(
					"Account synchronization queue exceeds 32 MiB; connection stopped",
				)
			})?
	};
	reliable
		.send
		.try_send((envelope, permit))
		.map_err(|error| match error {
			mpsc::error::TrySendError::Full(_) => Failure::CapacityAt(
				"Account synchronization event queue is full; connection stopped",
			),
			mpsc::error::TrySendError::Closed(_) => Failure::Network,
		})?;
	ctx.request_repaint();
	Ok(())
}

/// At most eight typing wakeups per two seconds in the selected conversation.
#[derive(Default)]
struct TypingGate {
	channel: u64,
	users: [Option<(model::Id, Instant)>; 8],
}
impl TypingGate {
	fn accept(&mut self, signal: client_core::typing::Signal, active: u64, now: Instant) -> bool {
		if active == 0 || signal.channel.0 != active || signal.user.0 == 0 {
			return false;
		}
		if self.channel != active {
			self.channel = active;
			self.users.fill(None);
		}
		for slot in &mut self.users {
			if slot.is_some_and(|(_, time)| {
				now.saturating_duration_since(time) >= Duration::from_secs(2)
			}) {
				*slot = None;
			}
		}
		if self
			.users
			.iter()
			.flatten()
			.any(|(user, _)| *user == signal.user)
		{
			return false;
		}
		let Some(slot) = self.users.iter_mut().find(|slot| slot.is_none()) else {
			return false;
		};
		*slot = Some((signal.user, now));
		true
	}
}

fn private_call(channel: &model::Channel) -> bool {
	channel.guild.is_none()
		&& ((channel.kind == 1 && channel.recipients.len() == 1)
			|| (channel.kind == 3
				&& channel.recipients.len() < client_core::voice::MAX_PARTICIPANTS))
}

// Match the core's 64-call discovery cache, plus one explicitly joined attempt.
// Each record owns two <=64-ID buffers: 65 KiB (66,560 bytes) of IDs total.
// Also bounded: observed Vec capacity * size_of::<Option<RecipientCall>>(),
// the active/Vec header in RecipientCalls, and the shared Arc/Mutex metadata.
struct RecipientCalls {
	active: Option<RecipientCall>,
	observed: Vec<Option<RecipientCall>>,
}
impl Default for RecipientCalls {
	fn default() -> Self {
		Self {
			active: None,
			observed: Vec::with_capacity(client_core::voice::MAX_DM_CALLS),
		}
	}
}
impl RecipientCalls {
	fn as_ref(&self) -> Option<&RecipientCall> {
		self.active.as_ref()
	}
	fn join(&mut self, channel: model::Id, request: u64, private: bool) {
		if self
			.active
			.as_ref()
			.is_none_or(|call| call.channel != channel)
		{
			self.active = self
				.observed
				.iter()
				.flatten()
				.find(|call| call.channel == channel)
				.map(RecipientCall::snapshot);
		}
		RecipientCall::join(&mut self.active, channel, request, private);
	}
	// Only events affecting the current action invalidate its HTTP worker.
	fn observe(
		&mut self,
		event: &Event,
		owner: model::Id,
		channels: &BTreeMap<model::Id, Vec<model::Id>>,
	) -> bool {
		use client_core::voice::Event as V;
		let invalidates = self.active.as_ref().is_some_and(|call| {
			event.ready_navigation().is_some()
				|| !channels.contains_key(&call.channel)
				|| match event {
					Event::Disconnected | Event::Resync => true,
					Event::ChannelCreated(channel) => channel.id == call.channel,
					Event::ChannelChanged(patch) => patch.id == call.channel,
					Event::Unavailable(channel)
					| Event::RecipientAdded { channel, .. }
					| Event::RecipientRemoved { channel, .. }
					| Event::Voice(V::Call { channel, .. } | V::Deleted { channel }) => *channel == call.channel,
					Event::Voice(
						V::TakenOver { channel, request } | V::Departed { channel, request },
					) => (*channel, Some(*request)) == (call.channel, call.request),
					Event::Voice(V::State {
						guild,
						channel,
						user,
						..
					}) => {
						if *user == owner {
							call.joined.contains(user)
								&& (guild.is_some() || *channel != Some(call.channel))
						} else {
							*channel == Some(call.channel) || call.joined.contains(user)
						}
					}
					_ => false,
				}
		});
		if event.ready_navigation().is_some()
			|| matches!(event, Event::Disconnected | Event::Resync)
		{
			self.active = None;
			self.observed.clear();
			return invalidates;
		}
		if self.active.is_some() {
			RecipientCall::observe(&mut self.active, event, owner, channels);
		}
		for call in &mut self.observed {
			RecipientCall::observe(call, event, owner, channels);
		}
		self.observed.retain(Option::is_some);
		if let Event::Voice(V::Call {
			channel,
			unavailable: false,
			..
		}) = event && channels.contains_key(channel)
			&& !self
				.observed
				.iter()
				.flatten()
				.any(|call| call.channel == *channel)
		{
			let mut call = None;
			RecipientCall::observe(&mut call, event, owner, channels);
			if call.is_some() {
				if self.observed.len() == client_core::voice::MAX_DM_CALLS {
					self.observed.remove(0);
				}
				self.observed.push(call);
			}
		}
		invalidates
	}
}

// One observed/current DM call retains metadata: two 64-ID buffers (1024 bytes).
struct RecipientCall {
	channel: model::Id,
	request: Option<u64>,
	confirmed: bool,
	ringing: Option<Vec<model::Id>>,
	joined: Vec<model::Id>,
}
impl RecipientCall {
	fn new(channel: model::Id, request: u64) -> Self {
		Self {
			channel,
			request: Some(request),
			confirmed: false,
			ringing: None,
			joined: Vec::with_capacity(client_core::voice::MAX_PARTICIPANTS),
		}
	}
	fn valid_call(event: &Event, owner: model::Id, members: &[model::Id]) -> bool {
		let Event::Voice(client_core::voice::Event::Call {
			ringing,
			participants,
			..
		}) = event
		else {
			return true;
		};
		ringing.as_ref().is_none_or(|ids| {
			ids.len() <= client_core::voice::MAX_PARTICIPANTS
				&& ids.iter().enumerate().all(|(index, id)| {
					(*id == owner || members.contains(id)) && !ids[..index].contains(id)
				})
		}) && participants.as_ref().is_none_or(|peers| {
			peers.len() <= client_core::voice::MAX_PARTICIPANTS
				&& peers.iter().enumerate().all(|(index, peer)| {
					(peer.user == owner || members.contains(&peer.user))
						&& !peers[..index].iter().any(|old| old.user == peer.user)
				})
		})
	}
	fn snapshot(&self) -> Self {
		let mut copy = Self::new(self.channel, 0);
		copy.request = None;
		copy.joined.extend(&self.joined);
		if let Some(ids) = &self.ringing {
			let mut retained = Vec::with_capacity(client_core::voice::MAX_PARTICIPANTS);
			retained.extend(ids);
			copy.ringing = Some(retained);
		}
		copy
	}
	fn join(active: &mut Option<Self>, channel: model::Id, request: u64, private: bool) {
		let retained = active.take().filter(|call| call.channel == channel);
		if private {
			let mut call = retained.unwrap_or_else(|| Self::new(channel, request));
			call.request = Some(request);
			call.confirmed = false;
			*active = Some(call);
		}
	}
	fn allows(&self, channel: model::Id, request: u64, recipient: model::Id, stop: bool) -> bool {
		self.channel == channel
			&& self.request == Some(request)
			&& self.confirmed
			&& self.ringing.as_ref().is_some_and(|ringing| {
				ringing.contains(&recipient) == stop && (stop || !self.joined.contains(&recipient))
			})
	}
	fn observe(
		active: &mut Option<Self>,
		event: &Event,
		owner: model::Id,
		channels: &BTreeMap<model::Id, Vec<model::Id>>,
	) {
		use client_core::voice::Event as V;
		if event.ready_navigation().is_some()
			|| matches!(event, Event::Disconnected | Event::Resync)
		{
			*active = None;
			return;
		}
		// Preserve one validated pre-join observation for the existing-call flow.
		// An active attempt never admits another channel's metadata.
		if let Event::Voice(V::Call {
			channel,
			unavailable: false,
			..
		}) = event && channels
			.get(channel)
			.is_some_and(|members| Self::valid_call(event, owner, members))
			&& active.is_none()
		{
			let mut call = Self::new(*channel, 0);
			call.request = None;
			*active = Some(call);
		}
		let Some(call) = active.as_mut() else {
			return;
		};
		let Some(members) = channels.get(&call.channel) else {
			*active = None;
			return;
		};

		match event {
			Event::Voice(V::SessionConfirmed {
				channel, request, ..
			}) if (*channel, Some(*request)) == (call.channel, call.request) => {
				call.confirmed = true;
			}
			Event::Voice(V::TakenOver { channel, request } | V::Departed { channel, request })
				if (*channel, Some(*request)) == (call.channel, call.request) =>
			{
				*active = None;
			}
			Event::Voice(V::Deleted { channel }) | Event::Unavailable(channel)
				if *channel == call.channel =>
			{
				*active = None;
			}
			Event::Voice(V::Call {
				channel,
				ringing,
				participants,
				unavailable,
			}) if *channel == call.channel => {
				if *unavailable {
					*active = None;
					return;
				}
				if !Self::valid_call(event, owner, members) {
					return;
				}
				if let Some(ids) = ringing {
					let retained = call.ringing.get_or_insert_with(|| {
						Vec::with_capacity(client_core::voice::MAX_PARTICIPANTS)
					});
					retained.clear();
					retained.extend(ids);
				}
				if let Some(peers) = participants {
					call.joined.clear();
					call.joined.extend(peers.iter().map(|peer| peer.user));
				}
			}
			Event::Voice(V::State {
				guild,
				channel,
				user,
				..
			}) => {
				if *user == owner
					&& call.joined.contains(user)
					&& (guild.is_some() || *channel != Some(call.channel))
				{
					// Reject already queued recipient actions before departure is acknowledged.
					call.confirmed = false;
				}
				call.joined.retain(|peer| peer != user);
				if guild.is_none()
					&& *channel == Some(call.channel)
					&& (*user == owner || members.contains(user))
				{
					if call.joined.len() == client_core::voice::MAX_PARTICIPANTS {
						*active = None;
						return;
					}
					call.joined.push(*user);
				}
			}
			Event::RecipientRemoved { channel, user } if *channel == call.channel => {
				if *user == owner {
					*active = None;
					return;
				}
				call.joined.retain(|peer| peer != user);
				if let Some(ringing) = &mut call.ringing {
					ringing.retain(|peer| peer != user);
				}
			}
			_ => {}
		}
	}
}

// Reserve the fixed byte budget once; later member additions must not grow capacity.
fn recipient_ids(channel: &model::Channel) -> Vec<model::Id> {
	let mut recipients = Vec::with_capacity(client_core::voice::MAX_PARTICIPANTS);
	recipients.extend(
		channel
			.recipients
			.iter()
			.take(client_core::voice::MAX_PARTICIPANTS - 1)
			.map(|user| user.id),
	);
	recipients
}

// A recipient write never allocates media or changes the once-only initial ring state.
fn recipient_action(
	channel: model::Id,
	request: u64,
	recipient: model::Id,
	owner: model::Id,
	active: Option<(model::Id, u64, bool)>,
	recipients: Option<&[model::Id]>,
) -> bool {
	recipient != owner
		&& active.is_some_and(|(id, r, _)| id == channel && r == request)
		&& recipients.is_some_and(|ids| {
			ids.len() < client_core::voice::MAX_PARTICIPANTS && ids.contains(&recipient)
		})
}

fn recipient_write_allowed(
	control: client_core::voice::Command,
	calls: &RecipientCalls,
	channels: &BTreeMap<model::Id, Vec<model::Id>>,
) -> bool {
	let client_core::voice::Command::RingRecipient {
		channel,
		request,
		recipient,
		stop,
	} = control
	else {
		return false;
	};
	channels.get(&channel).is_some_and(|ids| {
		ids.len() < client_core::voice::MAX_PARTICIPANTS && ids.contains(&recipient)
	}) && calls
		.as_ref()
		.is_some_and(|call| call.allows(channel, request, recipient, stop))
}

// Recheck the exact target on revisions; unrelated peer controls do not cancel HTTP.
async fn wait_for_recipient_invalidation(
	mut members: watch::Receiver<u64>,
	mut available: watch::Receiver<bool>,
	control: client_core::voice::Command,
	calls: Arc<Mutex<RecipientCalls>>,
	channels: Arc<Mutex<BTreeMap<model::Id, Vec<model::Id>>>>,
) {
	loop {
		members.borrow_and_update();
		let online = *available.borrow_and_update();
		let allowed = {
			let Ok(channels) = channels.lock() else {
				return;
			};
			let Ok(calls) = calls.lock() else {
				return;
			};
			recipient_write_allowed(control, &calls, &channels)
		};
		if !allowed || !online {
			return;
		}
		tokio::select! {
			changed=members.changed()=>if changed.is_err() {return;},
			changed=available.changed()=>if changed.is_err() {return;},
		}
	}
}

// Latest scoped invalidation is rechecked at dispatch, independent of select branch order.
fn release_taken_over(
	active: &mut Option<(model::Id, u64, bool)>,
	latest: Option<(model::Id, u64)>,
) -> bool {
	if active.is_some_and(|(channel, request, _)| latest == Some((channel, request))) {
		*active = None;
		true
	} else {
		false
	}
}

// A duplicate queued Join must not recreate the invalidated local request.
fn join_taken_over(control: client_core::voice::Command, latest: Option<(model::Id, u64)>) -> bool {
	matches!(control, client_core::voice::Command::Join { channel, request, .. } if latest == Some((channel, request)))
}

async fn wait_for_takeover(
	mut takeover: watch::Receiver<Option<(model::Id, u64)>>,
	scope: (model::Id, u64),
) {
	loop {
		let matching = *takeover.borrow_and_update() == Some(scope);
		if matching || takeover.changed().await.is_err() {
			return;
		}
	}
}

// Queue pressure must not end text signaling or allow a fresh Join ahead of local cleanup.
fn queue_abandonment(
	sender: &mpsc::Sender<client_core::voice::Command>,
	pending: &mut Option<client_core::voice::Command>,
	control: client_core::voice::Command,
	owner: Option<(model::Id, u64)>,
) {
	if let client_core::voice::Command::AbandonSession { channel, request } = control
		&& owner.is_some_and(|scope| scope != (channel, request))
	{
		return;
	}
	debug_assert!(matches!(
		control,
		client_core::voice::Command::AbandonSession { .. }
	));
	if pending.is_some() {
		// A new attempt is blocked until this first release is queued; stale releases cannot replace it.
		return;
	}
	if let Err(mpsc::error::TrySendError::Full(control)) = sender.try_send(control) {
		*pending = Some(control);
	}
}

// A stale candidate cannot fail its replacement: the desktop rechecks the tagged revision.
fn report_confirmation_failure(
	sender: &watch::Sender<Option<ConfirmationFailure>>,
	generation: u64,
	event: client_core::voice::Event,
	owner: Option<(model::Id, u64, bool)>,
	wake: &egui::Context,
) {
	let client_core::voice::Event::SessionConfirmationFailed {
		channel,
		request,
		revision,
		message,
	} = event
	else {
		unreachable!("only candidate confirmation failures use this report");
	};
	if !owner.is_some_and(|(id, attempt, _)| (id, attempt) == (channel, request)) {
		return;
	}
	let failure = ConfirmationFailure {
		generation,
		channel,
		request,
		revision,
		message,
	};
	if sender.send_if_modified(|current| {
		if current.is_some_and(|old| {
			(old.generation, old.channel, old.request) == (generation, channel, request)
				&& old.revision >= revision
		}) {
			return false;
		}
		*current = Some(failure);
		true
	}) {
		wake.request_repaint();
	}
}

fn queue_confirmation(
	sender: &mpsc::Sender<client_core::voice::Command>,
	control: client_core::voice::Command,
	owner: Option<(model::Id, u64, bool)>,
	online: bool,
) -> Option<client_core::voice::Event> {
	let client_core::voice::Command::ConfirmSession {
		channel,
		request,
		revision,
	} = control
	else {
		unreachable!("only local transport confirmations use this admission path");
	};
	let message = if !online {
		"Call confirmation was not sent; voice signaling is disconnected"
	} else if !owner.is_some_and(|(id, attempt, _)| (id, attempt) == (channel, request)) {
		"Call confirmation expired; no action was sent"
	} else {
		match sender.try_send(control) {
			Ok(()) => return None,
			Err(mpsc::error::TrySendError::Full(_)) => {
				"Call confirmation was not sent; the voice queue is full"
			}
			Err(mpsc::error::TrySendError::Closed(_)) => {
				"Call confirmation was not sent; voice signaling is disconnected"
			}
		}
	};
	Some(client_core::voice::Event::SessionConfirmationFailed {
		channel,
		request,
		revision,
		message,
	})
}

// A fresh Join can meet the still-full queue just after local release was queued.
fn queue_join(
	sender: &mpsc::Sender<client_core::voice::Command>,
	control: client_core::voice::Command,
	scope: (model::Id, u64),
	owner: &mut Option<(model::Id, u64, bool)>,
) -> Option<client_core::voice::Event> {
	debug_assert!(matches!(control, client_core::voice::Command::Join { .. }));
	let error = sender.try_send(control).err()?;
	release_taken_over(owner, Some(scope));
	Some(client_core::voice::Event::Failed {
		channel: scope.0,
		request: scope.1,
		message: match error {
			mpsc::error::TrySendError::Full(_) => "Call join was not sent; the voice queue is full",
			mpsc::error::TrySendError::Closed(_) => {
				"Call join was not sent; voice signaling is disconnected"
			}
		},
	})
}

fn abandonment_blocks_join(
	pending: Option<client_core::voice::Command>,
	control: client_core::voice::Command,
) -> bool {
	pending.is_some() && matches!(control, client_core::voice::Command::Join { .. })
}

// Ring only after the media adapter confirms transport allocation, and only once per current call.
fn ring_action(
	control: client_core::voice::Command,
	owner: model::Id,
	active: &mut Option<(model::Id, u64, bool)>,
	dm: bool,
) -> Result<Option<(Option<model::Id>, bool)>, ()> {
	use client_core::voice::Command as V;
	if let V::AbandonSession { channel, request } = control {
		if active.is_some_and(|(id, r, _)| id == channel && r == request) {
			*active = None;
		}
		return Ok(None);
	}
	if let V::Leave { channel, request } = control {
		if active.is_some_and(|(id, r, _)| id == channel && r == request) {
			*active = None;
			return Ok(dm.then_some((None, true)));
		}
		return Ok(None);
	}
	if !dm {
		return match control {
			V::Ring { .. }
			| V::RingRecipient { .. }
			| V::Decline { .. }
			| V::Join { ring: true, .. } => Err(()),
			V::Join {
				channel,
				request,
				ring: false,
				..
			} => {
				*active = Some((channel, request, true));
				Ok(None)
			}
			V::AbandonSession { .. }
			| V::ConfirmSession { .. }
			| V::Sync { .. }
			| V::Leave { .. }
			| V::SetMute { .. }
			| V::SetCamera { .. }
			| V::StartStream { .. }
			| V::StopStream { .. }
			| V::WatchStream { .. }
			| V::StopWatching { .. } => Ok(None),
		};
	}
	match control {
		V::Join {
			channel,
			request,
			ring,
			..
		} => {
			*active = Some((channel, request, !ring));
			Ok(None)
		}
		V::Ring { channel, request } => {
			let Some((current, current_request, rang)) = active else {
				return Err(());
			};
			if *current != channel || *current_request != request || *rang {
				return Err(());
			}
			*rang = true;
			Ok(Some((None, false)))
		}
		V::Decline { .. } => Ok(Some((Some(owner), true))),
		V::RingRecipient { .. } => Err(()),
		V::AbandonSession { .. }
		| V::ConfirmSession { .. }
		| V::Sync { .. }
		| V::Leave { .. }
		| V::SetMute { .. }
		| V::SetCamera { .. }
		| V::StartStream { .. }
		| V::StopStream { .. }
		| V::WatchStream { .. }
		| V::StopWatching { .. } => Ok(None),
	}
}

fn stream_preview_session_failure(event: &Event) -> Option<Failure> {
	match event {
		// An oversized optional still must not disconnect the account.
		Event::StreamPreview { result: Err(f), .. }
			if f.ends_session() && *f != Failure::Capacity =>
		{
			Some(*f)
		}
		_ => None,
	}
}

// Reads can finish after navigation/cancellation. Only a session-ending failure is global.
fn scope_history_failure(event: Event, channel: model::Id, request: u64) -> Event {
	let failure = match event {
		Event::Failure(failure) if !failure.ends_session() => failure,
		Event::Unavailable(_) => Failure::Forbidden,
		event => return event,
	};
	Event::HistoryFailed {
		channel,
		request,
		failure,
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn takeover_rejects_queued_initial_ringing_but_preserves_new_call_ownership() {
		use client_core::voice::Command as V;
		let channel = model::Id(22);
		let mut active = Some((channel, 5, false));
		let (send, receive) = watch::channel(None);
		let _ = send.send_replace(Some((channel, 5)));
		// The command is ready before the changed-watch select branch executes.
		assert!(release_taken_over(&mut active, *receive.borrow()));
		assert_eq!(active, None);
		let old_join = V::Join {
			channel,
			request: 5,
			ring: true,
			mute: false,
			deaf: false,
		};
		assert!(join_taken_over(old_join, *receive.borrow()));
		assert!(
			active.is_none(),
			"rejected duplicate Join cannot repopulate ownership"
		);
		let new_join = V::Join {
			channel,
			request: 6,
			ring: true,
			mute: false,
			deaf: false,
		};
		assert!(!join_taken_over(new_join, *receive.borrow()));
		assert!(!join_taken_over(
			V::Join {
				channel: model::Id(23),
				request: 5,
				ring: true,
				mute: false,
				deaf: false
			},
			*receive.borrow()
		));
		assert_eq!(
			ring_action(
				V::Ring {
					channel,
					request: 5
				},
				model::Id(1),
				&mut active,
				true
			),
			Err(())
		);
		assert_eq!(
			ring_action(
				V::Join {
					channel,
					request: 6,
					ring: true,
					mute: false,
					deaf: false
				},
				model::Id(1),
				&mut active,
				true
			),
			Ok(None)
		);
		assert!(!release_taken_over(&mut active, *receive.borrow()));
		assert_eq!(
			ring_action(
				V::Ring {
					channel,
					request: 6
				},
				model::Id(1),
				&mut active,
				true
			),
			Ok(Some((None, false)))
		);
		assert!(!release_taken_over(&mut active, Some((model::Id(23), 6))));
		assert_eq!(active, Some((channel, 6, true)));
		// Moving from an unringed DM attempt to a guild replaces dispatcher ownership too.
		active = Some((channel, 7, false));
		let guild_channel = model::Id(30);
		assert_eq!(
			ring_action(
				V::Join {
					channel: guild_channel,
					request: 8,
					ring: false,
					mute: false,
					deaf: false
				},
				model::Id(1),
				&mut active,
				false
			),
			Ok(None)
		);
		assert_eq!(active, Some((guild_channel, 8, true)));
		assert_eq!(
			ring_action(
				V::Ring {
					channel,
					request: 7
				},
				model::Id(1),
				&mut active,
				true
			),
			Err(())
		);
		assert!(release_taken_over(&mut active, Some((guild_channel, 8))));
	}
	#[test]
	fn initial_ringing_wait_cancels_only_matching_takeover_or_connection_teardown() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		runtime.block_on(async {
			let scope = (model::Id(22), 5);
			let (send, receive) = watch::channel(Some((model::Id(22), 4)));
			let task = tokio::spawn(wait_for_takeover(receive.clone(), scope));
			tokio::task::yield_now().await;
			assert!(!task.is_finished());
			let _ = send.send_replace(Some(scope));
			tokio::time::timeout(Duration::from_secs(1), task)
				.await
				.unwrap()
				.unwrap();
			// Match production biased selection with a synthetic HTTP future side effect.
			let contacted = std::cell::Cell::new(false);
			tokio::select! {
				biased;
				_=wait_for_takeover(receive.clone(), scope)=>{},
				_=async {contacted.set(true);std::future::pending::<()>().await}=>unreachable!(),
			}
			assert!(
				!contacted.get(),
				"already invalidated ownership must never poll the HTTP write"
			);
			// A task spawned after the notification also cancels before its HTTP future.
			let task = tokio::spawn(wait_for_takeover(receive.clone(), scope));
			tokio::time::timeout(Duration::from_secs(1), task)
				.await
				.unwrap()
				.unwrap();
			let task = tokio::spawn(wait_for_takeover(receive, (model::Id(22), 6)));
			drop(send);
			tokio::time::timeout(Duration::from_secs(1), task)
				.await
				.unwrap()
				.unwrap();
		});
	}
	#[test]
	fn stream_preview_capacity_is_local_but_auth_failures_end_the_session() {
		for (failure, expected) in [
			(Failure::Capacity, None),
			(Failure::Forbidden, None),
			(Failure::Expired, Some(Failure::Expired)),
			(Failure::Challenged, Some(Failure::Challenged)),
			(Failure::InvalidCredential, Some(Failure::InvalidCredential)),
		] {
			assert_eq!(
				stream_preview_session_failure(&Event::StreamPreview {
					guild: model::Id(1),
					channel: model::Id(2),
					user: model::Id(3),
					request: 4,
					result: Err(failure),
				}),
				expected
			);
		}
	}

	#[tokio::test]
	async fn confirmation_report_waits_for_replacement_signaling_beyond_one_frame_batch() {
		use client_core::voice::Event as E;
		let ctx = egui::Context::default();
		let (reliable, mut events) = reliable_events(ctx.clone());
		let (typing, _) = mpsc::channel(8);
		for id in 1..=EVENT_SLOTS as u64 {
			emit_event(&reliable, &typing, queued_channel(id), &ctx).unwrap();
		}
		for event in [
			E::Server {
				channel: model::Id(20),
				request: 5,
				negotiation_revision: Some(3),
				token: Some(client_core::voice::Secret::new("synthetic-token".into()).unwrap()),
				endpoint: Some("synthetic.discord.media".into()),
			},
			E::SessionConfirmed {
				channel: model::Id(20),
				request: 5,
				revision: 3,
			},
		] {
			emit_event(
				&reliable,
				&typing,
				Envelope {
					generation: 7,
					event: Event::Voice(event),
				},
				&ctx,
			)
			.unwrap();
		}
		let (report, mut failure) = watch::channel(None);
		report_confirmation_failure(
			&report,
			7,
			E::SessionConfirmationFailed {
				channel: model::Id(20),
				request: 5,
				revision: 2,
				message: "old candidate queue full",
			},
			Some((model::Id(20), 5, false)),
			&ctx,
		);
		drop(report);
		for _ in 0..EVENT_SLOTS {
			assert!(matches!(
				events.try_recv().unwrap().event,
				Event::ChannelCreated(_)
			));
		}
		assert!(take_confirmation_failure(&mut failure, &events).is_none());
		assert!(failure.borrow().has_changed());
		assert!(matches!(
			events.try_recv().unwrap().event,
			Event::Voice(E::Server {
				negotiation_revision: Some(3),
				..
			})
		));
		assert!(take_confirmation_failure(&mut failure, &events).is_none());
		assert!(failure.borrow().has_changed());
		assert!(matches!(
			events.try_recv().unwrap().event,
			Event::Voice(E::SessionConfirmed { revision: 3, .. })
		));
		let error = take_confirmation_failure(&mut failure, &events).unwrap();
		assert!(matches!(
			error.event,
			Event::Voice(E::SessionConfirmationFailed { revision: 2, .. })
		));
		assert!(take_confirmation_failure(&mut failure, &events).is_none());
	}

	#[tokio::test]
	async fn closed_confirmation_report_is_delivered_once_without_repeating_each_frame() {
		use client_core::voice::Event as E;
		let ctx = egui::Context::default();
		let (report, mut receiver) = watch::channel(None);
		let (_, events) = reliable_events(ctx.clone());
		assert!(take_confirmation_failure(&mut receiver, &events).is_none());
		report_confirmation_failure(
			&report,
			7,
			E::SessionConfirmationFailed {
				channel: model::Id(20),
				request: 5,
				revision: 3,
				message: "confirmation queue full",
			},
			Some((model::Id(20), 5, false)),
			&ctx,
		);
		drop(report);
		assert!(receiver.has_changed().is_err());
		let envelope = take_confirmation_failure(&mut receiver, &events)
			.expect("final unseen report survives publisher shutdown");
		assert_eq!(envelope.generation, 7);
		assert!(matches!(
			envelope.event,
			Event::Voice(E::SessionConfirmationFailed {
				channel: model::Id(20),
				request: 5,
				revision: 3,
				..
			})
		));
		assert!(take_confirmation_failure(&mut receiver, &events).is_none());
	}

	#[tokio::test]
	async fn confirmation_failure_bypasses_full_account_events_and_keeps_latest_candidate() {
		use client_core::voice::{Command as V, Event as E};
		let ctx = egui::Context::default();
		let (reliable, mut events) = reliable_events(ctx.clone());
		let (typing, _) = mpsc::channel(8);
		for id in 1..=RELIABLE_ITEMS as u64 {
			emit_event(&reliable, &typing, queued_channel(id), &ctx).unwrap();
		}
		// Both the item and byte budgets are exhausted; candidate delivery uses neither.
		let remaining = reliable.bytes.available_permits() as u32;
		let held = reliable
			.bytes
			.clone()
			.try_acquire_many_owned(remaining)
			.unwrap();
		let (controls, mut control_receive) = mpsc::channel(8);
		for _ in 0..8 {
			controls
				.try_send(V::Sync {
					channel: model::Id(20),
				})
				.unwrap();
		}
		let owner = Some((model::Id(20), 5, false));
		let control = V::ConfirmSession {
			channel: model::Id(20),
			request: 5,
			revision: 3,
		};
		let (report, mut failure) = watch::channel(None);
		let error = queue_confirmation(&controls, control, owner, true).unwrap();
		report_confirmation_failure(&report, 7, error, owner, &ctx);
		assert!(failure.has_changed().unwrap());
		let current = (*failure.borrow()).unwrap();
		assert!(take_confirmation_failure(&mut failure, &events).is_none());
		assert!(failure.has_changed().unwrap());
		let envelope = current.envelope();
		assert_eq!(envelope.generation, 7);
		assert!(matches!(
			envelope.event,
			Event::Voice(E::SessionConfirmationFailed {
				channel: model::Id(20),
				request: 5,
				revision: 3,
				..
			})
		));
		assert_eq!(reliable.send.capacity(), 0);
		assert_eq!(reliable.bytes.available_permits(), 0);
		assert_eq!(control_receive.len(), 8);
		for (request, revision) in [(5, 2), (5, 3), (4, 9)] {
			report_confirmation_failure(
				&report,
				7,
				E::SessionConfirmationFailed {
					channel: model::Id(20),
					request,
					revision,
					message: "stale",
				},
				owner,
				&ctx,
			);
			assert!(failure.has_changed().unwrap());
			assert_eq!(*failure.borrow(), Some(current));
		}
		drop(held);
		for id in 1..=RELIABLE_ITEMS as u64 {
			let Event::ChannelCreated(channel) = events.try_recv().unwrap().event else {
				panic!("queued account event lost");
			};
			assert_eq!(channel.id, model::Id(id));
		}
		assert!(events.try_recv().is_err());
		let envelope = take_confirmation_failure(&mut failure, &events).unwrap();
		assert!(take_confirmation_failure(&mut failure, &events).is_none());
		let mut state = client_core::State {
			generation: 7,
			auth: client_core::auth::AuthState::Authenticated,
			..Default::default()
		};
		state.apply(envelope);
		assert_eq!(state.auth, client_core::auth::AuthState::Authenticated);
		for _ in 0..8 {
			assert!(matches!(control_receive.try_recv(), Ok(V::Sync { .. })));
		}
		report_confirmation_failure(
			&report,
			7,
			E::SessionConfirmationFailed {
				channel: model::Id(20),
				request: 6,
				revision: 1,
				message: "new attempt",
			},
			Some((model::Id(20), 6, false)),
			&ctx,
		);
		assert!(failure.has_changed().unwrap());
		assert_eq!(failure.borrow_and_update().unwrap().request, 6);
		assert!(size_of::<Option<ConfirmationFailure>>() <= 64);
	}

	#[tokio::test]
	async fn confirmation_queue_pressure_is_revision_scoped_and_preserves_queued_controls() {
		use client_core::voice::{Command as V, Event as E};
		let (sender, mut receiver) = mpsc::channel(8);
		for _ in 0..8 {
			sender
				.try_send(V::Sync {
					channel: model::Id(20),
				})
				.unwrap();
		}
		let confirmation = V::ConfirmSession {
			channel: model::Id(20),
			request: 5,
			revision: 2,
		};
		let owner = Some((model::Id(20), 5, false));
		assert!(matches!(
			queue_confirmation(&sender, confirmation, owner, true),
			Some(E::SessionConfirmationFailed {
				channel: model::Id(20),
				request: 5,
				revision: 2,
				message: "Call confirmation was not sent; the voice queue is full"
			})
		));
		assert_eq!(receiver.len(), 8);
		for _ in 0..8 {
			assert!(matches!(
				receiver.recv().await,
				Some(V::Sync {
					channel: model::Id(20)
				})
			));
		}
		for (active, online) in [
			(None, true),
			(Some((model::Id(20), 6, false)), true),
			(owner, false),
		] {
			assert!(matches!(
				queue_confirmation(&sender, confirmation, active, online),
				Some(E::SessionConfirmationFailed {
					request: 5,
					revision: 2,
					..
				})
			));
			assert!(receiver.try_recv().is_err());
		}
		assert!(queue_confirmation(&sender, confirmation, owner, true).is_none());
		assert!(matches!(
			receiver.recv().await,
			Some(V::ConfirmSession {
				request: 5,
				revision: 2,
				..
			})
		));
		drop(receiver);
		assert!(matches!(
			queue_confirmation(&sender, confirmation, owner, true),
			Some(E::SessionConfirmationFailed {
				request: 5,
				revision: 2,
				message: "Call confirmation was not sent; voice signaling is disconnected",
				..
			})
		));
	}

	#[tokio::test]
	async fn abandonment_survives_a_full_queue_and_precedes_a_fresh_join() {
		use client_core::voice::Command as V;
		let (sender, mut receiver) = mpsc::channel(8);
		for _ in 0..8 {
			sender
				.try_send(V::Sync {
					channel: model::Id(20),
				})
				.unwrap();
		}
		let release = V::AbandonSession {
			channel: model::Id(20),
			request: 5,
		};
		let mut pending = None;
		queue_abandonment(&sender, &mut pending, release, Some((model::Id(21), 6)));
		assert!(pending.is_none()); // An older scope cannot displace the current owner’s later release.
		queue_abandonment(&sender, &mut pending, release, Some((model::Id(20), 5)));
		assert!(matches!(
			pending,
			Some(V::AbandonSession {
				channel: model::Id(20),
				request: 5
			})
		));
		// A delayed stale release must not discard the cleanup that actually releases the old scope.
		queue_abandonment(
			&sender,
			&mut pending,
			V::AbandonSession {
				channel: model::Id(20),
				request: 4,
			},
			None,
		);
		assert!(matches!(
			pending,
			Some(V::AbandonSession {
				channel: model::Id(20),
				request: 5
			})
		));
		let join = V::Join {
			channel: model::Id(21),
			request: 6,
			ring: false,
			mute: false,
			deaf: false,
		};
		assert!(abandonment_blocks_join(pending, join));
		assert!(receiver.recv().await.is_some());
		sender
			.reserve()
			.await
			.unwrap()
			.send(pending.take().unwrap());
		assert!(!abandonment_blocks_join(pending, join));
		// The release used the only free slot: an immediately queued Join must fail locally.
		let mut owner = Some((model::Id(21), 6, true));
		assert!(matches!(
			queue_join(&sender, join, (model::Id(21), 6), &mut owner),
			Some(client_core::voice::Event::Failed {
				channel: model::Id(21),
				request: 6,
				..
			})
		));
		assert!(owner.is_none());
		for _ in 0..7 {
			assert!(matches!(receiver.recv().await, Some(V::Sync { .. })));
		}
		assert!(queue_join(&sender, join, (model::Id(21), 6), &mut owner).is_none());
		assert!(matches!(
			receiver.recv().await,
			Some(V::AbandonSession {
				channel: model::Id(20),
				request: 5
			})
		));
		assert!(matches!(
			receiver.recv().await,
			Some(V::Join {
				channel: model::Id(21),
				request: 6,
				..
			})
		));
		drop(receiver);
		queue_abandonment(&sender, &mut pending, release, None);
		assert!(pending.is_none()); // Closed queues are handled by the existing Gateway task.
	}

	#[test]
	fn abandoning_an_unconfirmed_attempt_clears_dispatch_without_an_http_hangup() {
		use client_core::voice::Command as V;
		let mut active = Some((model::Id(20), 5, false));
		assert_eq!(
			ring_action(
				V::AbandonSession {
					channel: model::Id(20),
					request: 4
				},
				model::Id(1),
				&mut active,
				true
			),
			Ok(None)
		);
		assert_eq!(active, Some((model::Id(20), 5, false)));
		assert_eq!(
			ring_action(
				V::AbandonSession {
					channel: model::Id(20),
					request: 5
				},
				model::Id(1),
				&mut active,
				true
			),
			Ok(None)
		);
		assert!(active.is_none());
		assert_eq!(
			ring_action(
				V::Ring {
					channel: model::Id(20),
					request: 5
				},
				model::Id(1),
				&mut active,
				true
			),
			Err(())
		);
	}
	#[tokio::test]
	async fn activity_privacy_waits_for_opt_in_and_propagates_expired_session() {
		let api = Arc::new(
			DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_ACTIVITY_PRIVACY_TOKEN".into()).unwrap(),
			))
			.unwrap(),
		);
		// Stop before the worker starts: this test can never send an HTTP request.
		api.stop();
		let (enabled, receive_enabled) = watch::channel(false);
		let (requests, receive_requests) = mpsc::channel(1);
		let (send_report, mut report) = watch::channel(Ok(None));
		let (finished, mut terminal) = watch::channel(None);
		let worker = tokio::spawn(run_activity_sharing(
			api,
			receive_enabled,
			receive_requests,
			send_report,
			finished,
			egui::Context::default(),
		));
		report.changed().await.unwrap();
		assert_eq!(*report.borrow_and_update(), Ok(None));
		for request in [false, true] {
			requests.send(request).await.unwrap();
			drop(requests.reserve().await.unwrap());
		}
		assert!(
			tokio::time::timeout(Duration::from_millis(25), report.changed())
				.await
				.is_err()
		);
		assert_eq!(*terminal.borrow(), None);
		enabled.send(true).unwrap();
		tokio::time::timeout(Duration::from_secs(1), terminal.changed())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(*terminal.borrow(), Some(Failure::Expired));
		worker.await.unwrap();
		assert_eq!(*report.borrow(), Err(Failure::Expired));
	}

	fn queued_channel(id: u64) -> client_core::Envelope {
		client_core::Envelope {
			generation: 1,
			event: client_core::Event::ChannelCreated(model::Channel {
				id: model::Id(id),
				guild: Some(model::Id(1)),
				parent_id: None,
				position: 0,
				name: "Synthetic channel".into(),
				icon: None,
				kind: 0,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				last_message: None,
			}),
		}
	}

	fn large_startup() -> client_core::Startup {
		use serde_json::json;
		let guilds: Vec<_> = (0..200_u64).map(|g| {
			let id = 10 + g;
			json!({"id":id.to_string(),"name":"Synthetic large account","owner_id":"1",
				"roles":[{"id":id.to_string(),"permissions":"1024"}],
				"channels":(0..100).map(|c| json!({"id":(1000+g*100+c).to_string(),
					"name":"synthetic-channel".repeat(6),"type":0,"permission_overwrites":[]})).collect::<Vec<_>>()})
		}).collect();
		let bytes = serde_json::to_vec(&json!({"user":{"id":"1","username":"Synthetic"},
			"session_id":"synthetic","resume_gateway_url":"wss://gateway.discord.gg/","guilds":guilds}))
		.unwrap();
		let envelope = discord_protocol::ready::decode(&bytes).unwrap();
		let permissions = envelope.permissions().unwrap();
		let (mut ready, warnings) = envelope.navigation().unwrap();
		let (guilds, channels) = ready.navigation().unwrap();
		client_core::Startup {
			external_stickers: false,
			user: ready.user.into_model(),
			guilds,
			channels,
			permissions,
			read_state: client_core::read_state::Event::Snapshot {
				entries: None,
				version: None,
				partial: false,
			},
			notifications: None,
			session_dnd: None,
			warnings,
		}
	}

	#[test]
	fn large_startup_crosses_the_queue_atomically_and_keeps_ordinary_budgets() {
		let ctx = egui::Context::default();
		let (send, mut events) = reliable_events(ctx.clone());
		let (typing, _) = mpsc::channel(8);
		let startup = large_startup();
		assert!(startup.bytes() > MAX_EVENT_BYTES);
		let mut state = client_core::State::default();
		let generation = state.generation;
		emit_event(
			&send,
			&typing,
			Envelope {
				generation,
				event: Event::Startup(Box::new(startup.prepare().unwrap())),
			},
			&ctx,
		)
		.unwrap();
		assert_eq!(send.startup.available_permits(), 0);
		assert_eq!(send.bytes.available_permits(), RELIABLE_BYTES);
		// A second startup cannot multiply the reserved 128 MiB capacity.
		assert_eq!(
			emit_event(
				&send,
				&typing,
				Envelope {
					generation,
					event: Event::Startup(Box::new(large_startup().prepare().unwrap()))
				},
				&ctx
			),
			Err(Failure::CapacityAt(
				"Account startup snapshot is already queued; connection stopped"
			))
		);
		let mut later = queued_channel(25_000);
		later.generation = generation;
		emit_event(&send, &typing, later, &ctx).unwrap();
		state.apply(events.try_recv().unwrap());
		assert_eq!(state.auth, client_core::auth::AuthState::Authenticated);
		assert_eq!((state.guilds.len(), state.channels.len()), (200, 20_000));
		assert!(state.can_read_history(model::Id(1000)));
		assert!(state.can_read_history(model::Id(20_999)));
		assert_eq!(send.startup.available_permits(), 1);
		let next = events.try_recv().unwrap();
		assert!(matches!(next.event, Event::ChannelCreated(_)));
		state.apply(next);
		assert_eq!(send.bytes.available_permits(), RELIABLE_BYTES);
		state.logout();
		assert!(state.channels.is_empty() && state.guilds.is_empty());
		// Dropping the consumer also releases startup capacity.
		emit_event(
			&send,
			&typing,
			Envelope {
				generation,
				event: Event::Startup(Box::new(large_startup().prepare().unwrap())),
			},
			&ctx,
		)
		.unwrap();
		drop(events);
		assert_eq!(send.startup.available_permits(), 1);
	}

	#[test]
	fn reliable_navigation_burst_is_fifo_and_item_bounded() {
		let ctx = egui::Context::default();
		let (send, mut events) = reliable_events(ctx.clone());
		let (typing, _) = mpsc::channel(8);
		// One entire navigation fanout plus ordinary event slots fits without a UI drain.
		for id in 1..=RELIABLE_ITEMS as u64 {
			emit_event(&send, &typing, queued_channel(id), &ctx).unwrap();
		}
		let retained = send.bytes.available_permits();
		assert_eq!(
			emit_event(&send, &typing, queued_channel(0), &ctx),
			Err(Failure::CapacityAt(
				"Account synchronization event queue is full; connection stopped"
			))
		);
		assert_eq!(send.bytes.available_permits(), retained);
		for id in 1..=RELIABLE_ITEMS as u64 {
			let envelope = events.try_recv().unwrap();
			assert_eq!(envelope.generation, 1);
			let Event::ChannelCreated(channel) = envelope.event else {
				panic!("Reliable events must be retained in order");
			};
			assert_eq!(channel.id, model::Id(id));
		}
		assert!(events.try_recv().is_err());
		assert_eq!(send.bytes.available_permits(), RELIABLE_BYTES);
	}

	#[test]
	fn reliable_byte_budget_releases_on_receive_rejection_and_drop() {
		let ctx = egui::Context::default();
		let (send, mut events) = reliable_events(ctx.clone());
		let (typing, _) = mpsc::channel(8);
		let envelope = queued_channel(2);
		let charge = envelope.event.bytes() + size_of::<(Envelope, OwnedSemaphorePermit)>()
			- size_of::<Event>();
		let held = send
			.bytes
			.clone()
			.try_acquire_many_owned((RELIABLE_BYTES - charge) as u32)
			.unwrap();
		emit_event(&send, &typing, envelope, &ctx).unwrap();
		assert_eq!(send.bytes.available_permits(), 0);
		assert_eq!(
			emit_event(&send, &typing, queued_channel(3), &ctx),
			Err(Failure::CapacityAt(
				"Account synchronization queue exceeds 32 MiB; connection stopped"
			))
		);
		assert_eq!(events.receive.len(), 1);
		events.try_recv().unwrap();
		assert_eq!(send.bytes.available_permits(), charge);
		emit_event(&send, &typing, queued_channel(3), &ctx).unwrap();
		drop(held);
		let before = send.bytes.available_permits();
		let mut oversized = queued_channel(4);
		if let Event::ChannelCreated(channel) = &mut oversized.event {
			channel.name = "x".repeat(MAX_EVENT_BYTES);
		}
		assert_eq!(
			emit_event(&send, &typing, oversized, &ctx),
			Err(Failure::CapacityAt(
				"Account synchronization event exceeds 4 MiB; connection stopped"
			))
		);
		assert_eq!(send.bytes.available_permits(), before);
		drop(events);
		assert_eq!(send.bytes.available_permits(), RELIABLE_BYTES);
		assert_eq!(
			emit_event(&send, &typing, queued_channel(5), &ctx),
			Err(Failure::Network)
		);
		assert_eq!(send.bytes.available_permits(), RELIABLE_BYTES);
	}

	#[test]
	fn typing_burst_cannot_consume_reliable_message_slots() {
		let ctx = eframe::egui::Context::default();
		let (send, mut events) = super::reliable_events(ctx.clone());
		let (typing_send, mut typing) = tokio::sync::mpsc::channel(8);
		for user in 1..=100 {
			super::emit_event(
				&send,
				&typing_send,
				client_core::Envelope {
					generation: 1,
					event: client_core::Event::Typing(client_core::typing::Signal {
						channel: model::Id(10),
						user: model::Id(user),
						timestamp: 1,
					}),
				},
				&ctx,
			)
			.unwrap();
		}
		assert_eq!(typing.len(), 8);
		assert!(events.receive.is_empty());
		super::emit_event(
			&send,
			&typing_send,
			client_core::Envelope {
				generation: 1,
				event: client_core::Event::Delete {
					channel: model::Id(10),
					id: model::Id(20),
				},
			},
			&ctx,
		)
		.unwrap();
		assert!(matches!(
			events.try_recv().unwrap().event,
			client_core::Event::Delete { .. }
		));
		for _ in 0..8 {
			typing.try_recv().unwrap();
		}
	}
	#[test]
	fn typing_wakeups_are_selected_bounded_and_coalesced() {
		use client_core::typing::Signal;
		use model::Id;
		use std::time::{Duration, Instant};
		let mut gate = super::TypingGate::default();
		let now = Instant::now();
		let signal = Signal {
			channel: Id(10),
			user: Id(1),
			timestamp: 1,
		};
		assert!(!gate.accept(signal, 0, now));
		assert!(!gate.accept(signal, 11, now));
		for user in 1..=8 {
			let signal = Signal {
				user: Id(user),
				..signal
			};
			assert!(gate.accept(signal, 10, now));
			assert!(!gate.accept(signal, 10, now));
		}
		for user in 9..=1_000 {
			assert!(!gate.accept(
				Signal {
					user: Id(user),
					..signal
				},
				10,
				now
			));
		}
		assert!(!gate.accept(signal, 10, now + Duration::from_millis(1_999)));
		assert!(gate.accept(signal, 10, now + Duration::from_secs(2)));
		assert!(gate.accept(
			Signal {
				channel: Id(11),
				..signal
			},
			11,
			now
		));
		assert!(!gate.accept(signal, 11, now));
	}
	use super::*;
	#[test]
	fn recipient_writes_reject_stale_self_and_removed_members() {
		use model::Id;
		let scope = Some((Id(2), 7, true));
		let peers = [Id(3), Id(4)];
		assert!(recipient_action(
			Id(2),
			7,
			Id(3),
			Id(1),
			scope,
			Some(&peers)
		));
		for (channel, request, recipient, active, members) in [
			(Id(2), 6, Id(3), scope, Some(peers.as_slice())),
			(Id(9), 7, Id(3), scope, Some(peers.as_slice())),
			(Id(2), 7, Id(1), scope, Some(peers.as_slice())),
			(Id(2), 7, Id(8), scope, Some(peers.as_slice())),
			(Id(2), 7, Id(3), None, Some(peers.as_slice())),
			(Id(2), 7, Id(3), scope, None),
		] {
			assert!(!recipient_action(
				channel,
				request,
				recipient,
				Id(1),
				active,
				members
			));
		}
		let oversized = [Id(3); client_core::voice::MAX_PARTICIPANTS];
		assert!(!recipient_action(
			Id(2),
			7,
			Id(3),
			Id(1),
			scope,
			Some(&oversized)
		));
	}
	#[tokio::test]
	async fn multiple_prejoin_calls_preserve_target_and_unrelated_events_do_not_cancel_write() {
		use client_core::voice::Event as V;
		use model::Id;
		let (owner, first, second, recipient, other) = (Id(1), Id(2), Id(4), Id(3), Id(5));
		let channels = BTreeMap::from([(first, vec![recipient]), (second, vec![other])]);
		let call = |channel, ringing| {
			Event::Voice(V::Call {
				channel,
				ringing: Some(ringing),
				participants: Some(vec![]),
				unavailable: false,
			})
		};
		let calls = Arc::new(Mutex::new(RecipientCalls::default()));
		calls
			.lock()
			.unwrap()
			.observe(&call(first, vec![]), owner, &channels);
		calls
			.lock()
			.unwrap()
			.observe(&call(second, vec![other]), owner, &channels);
		assert_eq!(calls.lock().unwrap().observed.len(), 2);
		assert!(calls.lock().unwrap().active.is_none());
		calls.lock().unwrap().join(first, 7, true);
		calls.lock().unwrap().observe(
			&Event::Voice(V::SessionConfirmed {
				channel: first,
				request: 7,
				revision: 1,
			}),
			owner,
			&channels,
		);
		assert!(
			calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(first, 7, recipient, false)
		);
		let (changes, updates) = watch::channel(0u64);
		let (_online_send, online) = watch::channel(true);
		let control = client_core::voice::Command::RingRecipient {
			channel: first,
			request: 7,
			recipient,
			stop: false,
		};
		let cancelled = wait_for_recipient_invalidation(
			updates,
			online,
			control,
			calls.clone(),
			Arc::new(Mutex::new(channels.clone())),
		);
		tokio::pin!(cancelled);
		let unrelated_state = Event::Voice(V::State {
			guild: None,
			channel: Some(second),
			user: other,
			request: None,
			member: None,
			session: None,
			negotiation_revision: None,
			server_muted: false,
			server_deafened: false,
			muted: false,
			deafened: false,
			video: false,
			streaming: false,
		});
		for event in [
			unrelated_state,
			call(second, vec![]),
			Event::RecipientRemoved {
				channel: second,
				user: other,
			},
		] {
			if calls.lock().unwrap().observe(&event, owner, &channels) {
				changes.send_modify(|revision| *revision += 1);
			}
		}
		assert_eq!(*changes.borrow(), 0);
		assert!(
			calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(first, 7, recipient, false)
		);
		// A pending request survives unrelated updates; relevant updates still cancel it.
		assert!(
			tokio::time::timeout(Duration::from_millis(1), &mut cancelled)
				.await
				.is_err()
		);
		assert!(
			calls
				.lock()
				.unwrap()
				.observe(&call(first, vec![recipient]), owner, &channels)
		);
		changes.send_modify(|revision| *revision += 1);
		tokio::time::timeout(Duration::from_secs(1), cancelled)
			.await
			.unwrap();
		assert!(
			!calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(first, 7, recipient, false)
		);
		assert!(
			calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(first, 7, recipient, true)
		);
	}

	#[tokio::test]
	async fn recipient_worker_survives_owner_controls_but_departure_rejects_queued_actions() {
		use client_core::voice::Event as V;
		use model::Id;
		let (owner, channel, recipient) = (Id(1), Id(2), Id(3));
		let channels = BTreeMap::from([(channel, vec![recipient])]);
		let state = |user, target, guild, muted, video| {
			Event::Voice(V::State {
				guild,
				channel: target,
				user,
				request: None,
				member: None,
				session: None,
				negotiation_revision: None,
				server_muted: false,
				server_deafened: false,
				muted,
				deafened: muted,
				video,
				streaming: false,
			})
		};
		let calls = Arc::new(Mutex::new(RecipientCalls::default()));
		calls.lock().unwrap().observe(
			&Event::Voice(V::Call {
				channel,
				ringing: Some(vec![]),
				participants: Some(vec![]),
				unavailable: false,
			}),
			owner,
			&channels,
		);
		calls.lock().unwrap().join(channel, 7, true);
		calls.lock().unwrap().observe(
			&state(owner, Some(channel), None, false, false),
			owner,
			&channels,
		);
		calls.lock().unwrap().observe(
			&Event::Voice(V::SessionConfirmed {
				channel,
				request: 7,
				revision: 1,
			}),
			owner,
			&channels,
		);
		assert!(
			calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(channel, 7, recipient, false)
		);
		let (changes, updates) = watch::channel(0u64);
		let (_online, available) = watch::channel(true);
		let control = client_core::voice::Command::RingRecipient {
			channel,
			request: 7,
			recipient,
			stop: false,
		};
		let pending = wait_for_recipient_invalidation(
			updates.clone(),
			available.clone(),
			control,
			calls.clone(),
			Arc::new(Mutex::new(channels.clone())),
		);
		tokio::pin!(pending);
		for event in [
			state(owner, Some(channel), None, true, false),
			state(owner, Some(channel), None, false, true),
		] {
			assert!(!calls.lock().unwrap().observe(&event, owner, &channels));
			assert!(
				calls
					.lock()
					.unwrap()
					.as_ref()
					.unwrap()
					.allows(channel, 7, recipient, false)
			);
		}
		assert!(
			tokio::time::timeout(Duration::from_millis(1), &mut pending)
				.await
				.is_err()
		);
		// Owner departure invalidates before any TakenOver/Departed event reaches dispatch.
		assert!(calls.lock().unwrap().observe(
			&state(owner, None, None, false, false),
			owner,
			&channels
		));
		changes.send_replace(1);
		tokio::time::timeout(Duration::from_secs(1), pending)
			.await
			.unwrap();
		assert!(
			!calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(channel, 7, recipient, false)
		);
		assert!(
			!calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(channel, 7, recipient, true)
		);
		// A newer explicit join requires its own confirmation, then can ring again.
		calls.lock().unwrap().join(channel, 8, true);
		calls.lock().unwrap().observe(
			&state(owner, Some(channel), None, false, false),
			owner,
			&channels,
		);
		assert!(
			!calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(channel, 8, recipient, false)
		);
		calls.lock().unwrap().observe(
			&Event::Voice(V::SessionConfirmed {
				channel,
				request: 8,
				revision: 2,
			}),
			owner,
			&channels,
		);
		assert!(
			calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(channel, 8, recipient, false)
		);
		let mut attempt = 8;
		for (target, guild) in [(Some(Id(4)), None), (Some(channel), Some(Id(5)))] {
			assert!(calls.lock().unwrap().observe(
				&state(owner, target, guild, false, false),
				owner,
				&channels
			));
			assert!(
				!calls
					.lock()
					.unwrap()
					.as_ref()
					.unwrap()
					.allows(channel, attempt, recipient, false)
			);
			attempt += 1;
			calls.lock().unwrap().join(channel, attempt, true);
			calls.lock().unwrap().observe(
				&state(owner, Some(channel), None, false, false),
				owner,
				&channels,
			);
			calls.lock().unwrap().observe(
				&Event::Voice(V::SessionConfirmed {
					channel,
					request: attempt,
					revision: attempt,
				}),
				owner,
				&channels,
			);
		}
		let control = client_core::voice::Command::RingRecipient {
			channel,
			request: attempt,
			recipient,
			stop: false,
		};
		let pending = wait_for_recipient_invalidation(
			updates,
			available,
			control,
			calls.clone(),
			Arc::new(Mutex::new(channels.clone())),
		);
		tokio::pin!(pending);
		assert!(calls.lock().unwrap().observe(
			&state(recipient, Some(channel), None, false, false),
			owner,
			&channels
		));
		changes.send_replace(2);
		tokio::time::timeout(Duration::from_secs(1), pending)
			.await
			.unwrap();
		assert!(
			!calls
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.allows(channel, attempt, recipient, false)
		);
	}

	#[test]
	fn discovered_recipient_metadata_has_fixed_item_and_allocated_id_byte_limits() {
		use client_core::voice::{Event as V, MAX_DM_CALLS, MAX_PARTICIPANTS};
		use model::Id;
		let mut calls = RecipientCalls::default();
		let mut channels = BTreeMap::new();
		for index in 0..MAX_DM_CALLS + 1 {
			let channel = Id(index as u64 + 10);
			channels.insert(channel, vec![Id(2)]);
			calls.observe(
				&Event::Voice(V::Call {
					channel,
					ringing: Some(vec![Id(2)]),
					participants: Some(vec![]),
					unavailable: false,
				}),
				Id(1),
				&channels,
			);
		}
		assert_eq!(calls.observed.len(), MAX_DM_CALLS);
		assert_eq!(calls.observed.capacity(), MAX_DM_CALLS);
		assert!(
			calls
				.observed
				.iter()
				.flatten()
				.all(|call| call.channel != Id(10))
		);
		calls.join(Id(11), 7, true);
		let bytes = calls
			.observed
			.iter()
			.flatten()
			.chain(calls.active.iter())
			.map(|call| {
				(call.joined.capacity() + call.ringing.as_ref().map_or(0, Vec::capacity))
					* size_of::<Id>()
			})
			.sum::<usize>();
		assert_eq!(
			bytes,
			(MAX_DM_CALLS + 1) * MAX_PARTICIPANTS * 2 * size_of::<Id>()
		);
		assert!(calls.observe(&Event::Disconnected, Id(1), &channels));
		assert!(calls.active.is_none());
		assert!(calls.observed.is_empty());
	}

	#[test]
	fn existing_call_metadata_survives_join_with_only_state_and_confirmation_afterward() {
		use client_core::voice::Event as V;
		use model::Id;
		let (channel, owner, recipient) = (Id(2), Id(1), Id(3));
		let channels = BTreeMap::from([(channel, vec![recipient])]);
		let mut active = None;
		RecipientCall::observe(
			&mut active,
			&Event::Voice(V::Call {
				channel,
				ringing: Some(vec![]),
				participants: Some(vec![]),
				unavailable: false,
			}),
			owner,
			&channels,
		);
		assert_eq!(active.as_ref().unwrap().request, None);
		assert!(
			!active
				.as_ref()
				.unwrap()
				.allows(channel, 7, recipient, false)
		);
		RecipientCall::join(&mut active, channel, 7, true);
		assert!(
			!active
				.as_ref()
				.unwrap()
				.allows(channel, 7, recipient, false)
		);
		RecipientCall::observe(
			&mut active,
			&Event::Voice(V::State {
				guild: None,
				channel: Some(channel),
				user: owner,
				request: Some(7),
				member: None,
				session: None,
				negotiation_revision: Some(1),
				server_muted: false,
				server_deafened: false,
				muted: false,
				deafened: false,
				video: false,
				streaming: false,
			}),
			owner,
			&channels,
		);
		RecipientCall::observe(
			&mut active,
			&Event::Voice(V::SessionConfirmed {
				channel,
				request: 7,
				revision: 1,
			}),
			owner,
			&channels,
		);
		assert!(
			active
				.as_ref()
				.unwrap()
				.allows(channel, 7, recipient, false)
		);
		// Starting a new attempt keeps service metadata but waits for its own confirmation.
		RecipientCall::join(&mut active, channel, 8, true);
		assert!(
			!active
				.as_ref()
				.unwrap()
				.allows(channel, 7, recipient, false)
		);
		assert!(
			!active
				.as_ref()
				.unwrap()
				.allows(channel, 8, recipient, false)
		);
		RecipientCall::observe(
			&mut active,
			&Event::Voice(V::SessionConfirmed {
				channel,
				request: 7,
				revision: 1,
			}),
			owner,
			&channels,
		);
		assert!(!active.as_ref().unwrap().confirmed);
		RecipientCall::join(&mut active, Id(99), 9, false);
		assert!(active.is_none());
	}

	#[test]
	fn queued_recipient_actions_use_latest_confirmed_ringing_and_peer_presence() {
		use client_core::voice::Event as V;
		use model::Id;
		let (channel, owner, recipient) = (Id(2), Id(1), Id(3));
		let channels = BTreeMap::from([(channel, vec![recipient])]);
		let mut active = Some(RecipientCall::new(channel, 7));
		let allowed = |active: &Option<RecipientCall>, stop| {
			active
				.as_ref()
				.is_some_and(|call| call.allows(channel, 7, recipient, stop))
		};
		assert!(!allowed(&active, false));
		assert!(!allowed(&active, true));
		RecipientCall::observe(
			&mut active,
			&Event::Voice(V::SessionConfirmed {
				channel,
				request: 6,
				revision: 1,
			}),
			owner,
			&channels,
		);
		assert!(!active.as_ref().unwrap().confirmed);
		RecipientCall::observe(
			&mut active,
			&Event::Voice(V::SessionConfirmed {
				channel,
				request: 7,
				revision: 2,
			}),
			owner,
			&channels,
		);
		assert!(
			!allowed(&active, true),
			"unknown ringing cannot authorize a queued stop"
		);
		let call = |ringing| {
			Event::Voice(V::Call {
				channel,
				ringing,
				participants: Some(vec![]),
				unavailable: false,
			})
		};
		RecipientCall::observe(&mut active, &call(Some(vec![])), owner, &channels);
		assert!(allowed(&active, false));
		// A start queued earlier is rejected after service ringing begins.
		RecipientCall::observe(&mut active, &call(Some(vec![recipient])), owner, &channels);
		assert!(!allowed(&active, false));
		assert!(allowed(&active, true));
		// A stop queued earlier is rejected after the service clears ringing.
		RecipientCall::observe(&mut active, &call(Some(vec![])), owner, &channels);
		assert!(!allowed(&active, true));
		assert!(allowed(&active, false));
		let peer = |channel, guild| {
			Event::Voice(V::State {
				guild,
				channel,
				user: recipient,
				request: None,
				member: None,
				session: None,
				negotiation_revision: None,
				server_muted: false,
				server_deafened: false,
				muted: false,
				deafened: false,
				video: false,
				streaming: false,
			})
		};
		// Peer joined before dequeue: same request and membership are insufficient.
		RecipientCall::observe(&mut active, &peer(Some(channel), None), owner, &channels);
		assert!(!allowed(&active, false));
		RecipientCall::observe(
			&mut active,
			&peer(Some(Id(99)), Some(Id(98))),
			owner,
			&channels,
		);
		assert!(allowed(&active, false));
		// Malformed/duplicate service metadata never overwrites the validated state.
		RecipientCall::observe(
			&mut active,
			&call(Some(vec![recipient, recipient])),
			owner,
			&channels,
		);
		assert!(allowed(&active, false));
		let call = active.as_ref().unwrap();
		assert_eq!(
			(call.joined.capacity() + call.ringing.as_ref().unwrap().capacity()) * size_of::<Id>(),
			1024
		);
		RecipientCall::observe(
			&mut active,
			&Event::Voice(V::Departed {
				channel,
				request: 6,
			}),
			owner,
			&channels,
		);
		assert!(active.is_some());
		RecipientCall::observe(
			&mut active,
			&Event::Voice(V::TakenOver {
				channel,
				request: 7,
			}),
			owner,
			&channels,
		);
		assert!(active.is_none());
	}

	#[test]
	fn recipient_dispatch_rejects_queued_writes_after_takeover_and_local_abandon() {
		use client_core::voice::Command as V;
		use model::Id;
		let (channel, owner, recipient) = (Id(2), Id(1), Id(3));
		let peers = [recipient];
		let mut active = Some((channel, 7, true));
		assert!(recipient_action(
			channel,
			7,
			recipient,
			owner,
			active,
			Some(&peers)
		));
		assert!(release_taken_over(&mut active, Some((channel, 7))));
		assert!(!recipient_action(
			channel,
			7,
			recipient,
			owner,
			active,
			Some(&peers)
		));
		let join = |request| V::Join {
			channel,
			request,
			ring: false,
			mute: false,
			deaf: false,
		};
		assert!(join_taken_over(join(7), Some((channel, 7))));
		assert!(!join_taken_over(join(8), Some((channel, 7))));
		assert_eq!(ring_action(join(8), owner, &mut active, true), Ok(None));
		assert!(recipient_action(
			channel,
			8,
			recipient,
			owner,
			active,
			Some(&peers)
		));
		assert!(!release_taken_over(&mut active, Some((channel, 7))));
		assert_eq!(
			ring_action(
				V::AbandonSession {
					channel,
					request: 7
				},
				owner,
				&mut active,
				true
			),
			Ok(None)
		);
		assert!(recipient_action(
			channel,
			8,
			recipient,
			owner,
			active,
			Some(&peers)
		));
		assert_eq!(
			ring_action(
				V::AbandonSession {
					channel,
					request: 8
				},
				owner,
				&mut active,
				true
			),
			Ok(None)
		);
		assert!(!recipient_action(
			channel,
			8,
			recipient,
			owner,
			active,
			Some(&peers)
		));
	}

	fn recipient_worker_fixture(
		stop: bool,
	) -> (
		client_core::voice::Command,
		RecipientCalls,
		BTreeMap<model::Id, Vec<model::Id>>,
	) {
		let (channel, recipient) = (model::Id(2), model::Id(3));
		let mut call = RecipientCall::new(channel, 7);
		call.confirmed = true;
		call.ringing = Some(if stop { vec![recipient] } else { vec![] });
		let calls = RecipientCalls {
			active: Some(call),
			..Default::default()
		};
		(
			client_core::voice::Command::RingRecipient {
				channel,
				request: 7,
				recipient,
				stop,
			},
			calls,
			BTreeMap::from([(channel, vec![recipient, model::Id(4)])]),
		)
	}
	#[tokio::test]
	async fn recipient_worker_rechecks_target_and_preserves_unrelated_peer_controls() {
		use client_core::voice::Event as V;
		use model::Id;
		assert!(size_of::<client_core::voice::Command>() <= 64);
		let (control, calls, channels) = recipient_worker_fixture(false);
		let calls = Arc::new(Mutex::new(calls));
		let channels = Arc::new(Mutex::new(channels));
		let (changes, updates) = watch::channel(0u64);
		let (_online, available) = watch::channel(true);
		let pending = wait_for_recipient_invalidation(
			updates,
			available,
			control,
			calls.clone(),
			channels.clone(),
		);
		tokio::pin!(pending);
		for (user, muted, video) in [
			(Id(4), false, false),
			(Id(4), true, false),
			(Id(4), false, true),
		] {
			let event = Event::Voice(V::State {
				guild: None,
				channel: Some(Id(2)),
				user,
				request: None,
				member: None,
				session: None,
				negotiation_revision: None,
				server_muted: false,
				server_deafened: false,
				muted,
				deafened: muted,
				video,
				streaming: false,
			});
			{
				let channels = channels.lock().unwrap();
				let mut calls = calls.lock().unwrap();
				assert!(calls.observe(&event, Id(1), &channels));
				assert!(
					recipient_write_allowed(control, &calls, &channels),
					"main-loop abort guard keeps the eligible target"
				);
			}
			changes.send_modify(|revision| *revision += 1);
			assert!(
				tokio::time::timeout(Duration::from_millis(1), &mut pending)
					.await
					.is_err(),
				"unrelated peer controls retain the pending worker"
			);
		}
		// The requested recipient joining stops the obsolete start without a false error.
		let event = Event::Voice(V::State {
			guild: None,
			channel: Some(Id(2)),
			user: Id(3),
			request: None,
			member: None,
			session: None,
			negotiation_revision: None,
			server_muted: false,
			server_deafened: false,
			muted: false,
			deafened: false,
			video: false,
			streaming: false,
		});
		{
			let channels = channels.lock().unwrap();
			let mut calls = calls.lock().unwrap();
			assert!(calls.observe(&event, Id(1), &channels));
			assert!(!recipient_write_allowed(control, &calls, &channels));
		}
		changes.send_modify(|revision| *revision += 1);
		tokio::time::timeout(Duration::from_secs(1), pending)
			.await
			.unwrap();
	}
	#[tokio::test]
	async fn recipient_worker_stops_after_service_confirms_start_or_stop() {
		for stop in [false, true] {
			let (control, calls, channels) = recipient_worker_fixture(stop);
			let calls = Arc::new(Mutex::new(calls));
			let channels = Arc::new(Mutex::new(channels));
			let (changes, updates) = watch::channel(0u64);
			let (_online, available) = watch::channel(true);
			let pending = wait_for_recipient_invalidation(
				updates,
				available,
				control,
				calls.clone(),
				channels.clone(),
			);
			tokio::pin!(pending);
			assert!(
				tokio::time::timeout(Duration::from_millis(1), &mut pending)
					.await
					.is_err()
			);
			let event = Event::Voice(client_core::voice::Event::Call {
				channel: model::Id(2),
				ringing: Some(if stop { vec![] } else { vec![model::Id(3)] }),
				participants: None,
				unavailable: false,
			});
			{
				let channels = channels.lock().unwrap();
				let mut calls = calls.lock().unwrap();
				assert!(calls.observe(&event, model::Id(1), &channels));
				assert!(!recipient_write_allowed(control, &calls, &channels));
			}
			changes.send_modify(|revision| *revision += 1);
			tokio::time::timeout(Duration::from_secs(1), pending)
				.await
				.unwrap();
		}
	}

	#[tokio::test]
	async fn recipient_http_wait_checks_membership_offline_and_abandon_before_network_poll() {
		let (members, member_updates) = watch::channel(2u64);
		let (online, availability) = watch::channel(true);
		let scope = (model::Id(2), 7);
		let (takeover, ownership) = watch::channel(None);
		let (control, calls, channels) = recipient_worker_fixture(false);
		let calls = Arc::new(Mutex::new(calls));
		let channels = Arc::new(Mutex::new(channels));
		let task = tokio::spawn(wait_for_recipient_invalidation(
			member_updates.clone(),
			availability.clone(),
			control,
			calls.clone(),
			channels.clone(),
		));
		tokio::task::yield_now().await;
		assert!(!task.is_finished());
		calls.lock().unwrap().active = None;
		members.send_replace(3);
		tokio::time::timeout(Duration::from_secs(1), task)
			.await
			.unwrap()
			.unwrap();
		for invalidation in 0..3 {
			let (control, calls, channels) = recipient_worker_fixture(false);
			let calls = Arc::new(Mutex::new(calls));
			let channels = Arc::new(Mutex::new(channels));
			if invalidation == 0 {
				channels.lock().unwrap().clear();
			}
			members.send_replace(if invalidation == 0 { 3 } else { 2 });
			online.send_replace(invalidation != 1);
			takeover.send_replace((invalidation == 2).then_some(scope));
			let contacted = std::cell::Cell::new(false);
			tokio::select! {
				biased;
				_=wait_for_takeover(ownership.clone(),scope)=>{},
				_=wait_for_recipient_invalidation(member_updates.clone(),availability.clone(),control,calls.clone(),channels.clone())=>{},
				_=async {contacted.set(true);std::future::pending::<()>().await}=>unreachable!(),
			}
			assert!(
				!contacted.get(),
				"invalidated recipient action must not poll HTTP"
			);
		}
		// A task already pending when local Abandon publishes the same invalidation cancels.
		takeover.send_replace(None);
		let task = tokio::spawn(wait_for_takeover(ownership, scope));
		tokio::task::yield_now().await;
		assert!(!task.is_finished());
		takeover.send_replace(Some(scope));
		tokio::time::timeout(Duration::from_secs(1), task)
			.await
			.unwrap()
			.unwrap();
	}

	#[test]
	fn call_discovery_never_rings_or_changes_the_active_attempt() {
		use client_core::voice::Command as V;
		for dm in [false, true] {
			for mut active in [None, Some((model::Id(20), 7, false))] {
				let before = active;
				assert_eq!(
					ring_action(
						V::Sync {
							channel: model::Id(2)
						},
						model::Id(1),
						&mut active,
						dm
					),
					Ok(None)
				);
				assert_eq!(active, before);
			}
		}
	}
	#[test]
	fn ringing_waits_for_transport_confirmation_and_rejects_old_requests() {
		use client_core::voice::Command as V;
		let mut active = None;
		let channel = model::Id(2);
		let owner = model::Id(1);
		assert_eq!(
			ring_action(
				V::Join {
					channel,
					request: 7,
					ring: true,
					mute: false,
					deaf: false,
				},
				owner,
				&mut active,
				true
			),
			Ok(None)
		);
		assert!(
			ring_action(
				V::Ring {
					channel,
					request: 6
				},
				owner,
				&mut active,
				true
			)
			.is_err()
		);
		assert_eq!(
			ring_action(
				V::Ring {
					channel,
					request: 7
				},
				owner,
				&mut active,
				true
			),
			Ok(Some((None, false)))
		);
		assert!(
			ring_action(
				V::Ring {
					channel,
					request: 7
				},
				owner,
				&mut active,
				true
			)
			.is_err()
		);
		assert_eq!(
			ring_action(
				V::Leave {
					channel,
					request: 7
				},
				owner,
				&mut active,
				true
			),
			Ok(Some((None, true)))
		);
		assert!(
			ring_action(
				V::Ring {
					channel,
					request: 7
				},
				owner,
				&mut active,
				true
			)
			.is_err()
		);
		assert_eq!(
			ring_action(
				V::Join {
					channel,
					request: 8,
					ring: false,
					mute: false,
					deaf: false,
				},
				owner,
				&mut active,
				true
			),
			Ok(None)
		);
		assert_eq!(
			ring_action(
				V::Leave {
					channel,
					request: 7
				},
				owner,
				&mut active,
				true
			),
			Ok(None)
		);
		assert_eq!(active, Some((channel, 8, true)));
		assert!(
			ring_action(
				V::Ring {
					channel,
					request: 8
				},
				owner,
				&mut active,
				true
			)
			.is_err()
		);
	}
	#[test]
	fn guild_join_mute_leave_never_ring_a_dm() {
		use client_core::voice::Command as V;
		let mut active = None;
		let channel = model::Id(20);
		let owner = model::Id(1);
		for control in [
			V::Join {
				channel,
				request: 1,
				ring: false,
				mute: false,
				deaf: false,
			},
			V::SetMute {
				channel,
				request: 1,
				mute: true,
				deaf: false,
			},
			V::Leave {
				channel,
				request: 1,
			},
		] {
			assert_eq!(ring_action(control, owner, &mut active, false), Ok(None));
		}
		assert!(
			ring_action(
				V::Ring {
					channel,
					request: 1
				},
				owner,
				&mut active,
				false
			)
			.is_err()
		);
		assert!(ring_action(V::Decline { channel }, owner, &mut active, false).is_err());
		assert!(active.is_none());
	}
	#[test]
	fn reads_scope_permission_errors_but_expiry_stays_global() {
		assert!(matches!(
			scope_history_failure(Event::Unavailable(model::Id(1)), model::Id(1), 9),
			Event::HistoryFailed {
				channel: model::Id(1),
				request: 9,
				failure: Failure::Forbidden
			}
		));
		assert!(matches!(
			scope_history_failure(Event::Failure(Failure::Network), model::Id(1), 9),
			Event::HistoryFailed {
				request: 9,
				failure: Failure::Network,
				..
			}
		));
		assert!(matches!(
			scope_history_failure(Event::Failure(Failure::Expired), model::Id(1), 9),
			Event::Failure(Failure::Expired)
		));
	}
}
