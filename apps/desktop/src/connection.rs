use client_core::{
	COMMAND_SLOTS, Command, EVENT_SLOTS, Envelope, Event, MAX_EVENT_BYTES,
	auth::{AuthProvider, Failure, SessionSecret},
};
use discord_api::DiscordApi;
use eframe::egui;
use std::{
	collections::{BTreeMap, BTreeSet},
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
	pub share_activity: watch::Sender<bool>,
	pub own_presence: watch::Sender<model::OwnPresence>,
	/// Local edits only. Seeding from Discord does not publish through this watch.
	pub presence_edits: watch::Sender<Option<model::OwnPresence>>,
	/// The status chosen for this connection, once Discord or the local fallback is known.
	pub account_presence: watch::Receiver<Option<model::OwnPresence>>,
	pub presence_error: watch::Receiver<Option<&'static str>>,
	pub game_activity: watch::Receiver<crate::game_activity::Detection>,
	pub spotify_activity: watch::Receiver<Option<discord_protocol::spotify::Activity>>,
	/// A local Rich Presence client asked the client to show an invite: counter and code.
	pub rpc_invite: watch::Receiver<Option<(u64, String)>>,
	pub activity_observation: watch::Receiver<discord_gateway::ActivityObservation>,
	pub activity_sharing: watch::Receiver<Result<Option<bool>, Failure>>,
	pub activity_sharing_request: mpsc::Sender<bool>,
	typing_channel: Arc<AtomicU64>,
	task: JoinHandle<()>,
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
		ctx: egui::Context,
	) -> Self {
		let (commands, mut receive) = mpsc::channel(COMMAND_SLOTS);
		let (uploads, mut upload_receive) = mpsc::channel::<crate::uploads::UploadRequest>(1);
		let (send, events) = reliable_events(ctx.clone());
		let (typing_send, typing) = mpsc::channel(8);
		let (finished, terminal) = watch::channel(None);
		let (share_activity, share_receive) = watch::channel(false);
		let (own_presence, presence_receive) = watch::channel(model::OwnPresence::default());
		let (presence_edits, presence_edit_events) = watch::channel(None);
		let (account_presence_send, account_presence) = watch::channel(None);
		let (presence_error_send, presence_error) = watch::channel(None);
		let presence_send = own_presence.clone();
		let (game_report, game_activity) = watch::channel(Ok(None));
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
                let mut api=DiscordApi::new(secret.clone())?;
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
				let _activity_task=AbortTask(tokio::spawn(crate::game_activity::run(share_receive,activity_send,game_report,invite_send,wake.clone(),user.clone(),api.clone())));
				let _spotify_task=AbortTask(tokio::spawn(crate::spotify::run(api.clone(),user.id,presence_receive.clone(),spotify_send,wake.clone())));
                let dm_channels=Arc::new(Mutex::new(BTreeSet::new()));
                let gateway_channels=dm_channels.clone();
                let (voice_online,mut voice_availability)=watch::channel(false);
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
                            *gateway_channels.lock().map_err(|_|Failure::Protocol)?=channels.iter().filter(|c|private_call(c)).map(|c|c.id).collect();
                        }
                        if let Event::ChannelCreated(channel)=&event {
                            let mut channels=gateway_channels.lock().map_err(|_|Failure::Protocol)?;
                            channels.remove(&channel.id);
                            if private_call(channel) && channels.len()<client_core::MAX_NAV {channels.insert(channel.id);}
                        }
                        if let Event::Unavailable(channel)=&event {gateway_channels.lock().map_err(|_|Failure::Protocol)?.remove(channel);}
                        if let Event::RecipientRemoved {channel,user:removed}=&event && *removed==user.id {gateway_channels.lock().map_err(|_|Failure::Protocol)?.remove(channel);}
                        if let Event::ChannelChanged(patch)=&event && let model::Patch::Value(kind)=patch.kind && !matches!(kind,1|3) {gateway_channels.lock().map_err(|_|Failure::Protocol)?.remove(&patch.id);}

                        if event.ready_navigation().is_some() || matches!(&event,Event::Resumed) {let _=voice_online.send(true);}
                        if matches!(&event,Event::Disconnected|Event::Resync) {let _=voice_online.send(false);}
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
                        let failure=match &event {Event::Interaction(client_core::interactions::Event::Submitted{result:Err(f),..})=>Some(*f),Event::MessagingPermissions{result:Err(f),..}=>Some(*f),Event::ChannelAction(client_core::channel_actions::Event::Finished{result:Err(f),..})=>Some(*f),Event::ServerAdmin(client_core::server_admin::Event{result:Err(f),..})=>Some(*f),Event::ServerSettings(client_core::server_settings::Event{result:Err(f),..})=>Some(*f),Event::Failure(f)=>Some(*f),Event::ProfileEdited{result:Err(f),..} if *f != Failure::Capacity =>Some(*f),Event::Edited{result:Err(f),..}|Event::Pinned{result:Err(f),..}=>Some(*f),Event::GuildFolders(Err(f))=>Some(*f),Event::JoinInvite{result:Err(f),..}=>Some(*f),Event::SendResult{result:Err(f),..}=>Some(*f),Event::UserAction(client_core::user_actions::Event::Written{result:Err(f),..})=>Some(*f),Event::UserAction(client_core::user_actions::Event::DmOpened{result:Err(f),..})=>Some(*f),Event::ServerAction(client_core::server_actions::Event::Written{result:Err(f),..})=>Some(*f),Event::ServerAction(client_core::server_actions::Event::InviteSent{result:Err(f),..})=>Some(*f),Event::GroupAction(client_core::group_actions::Event::Written{result:Err(f),..})=>Some(*f),Event::Reactions(client_core::reactions::Event::Written{result:Err(f),..})=>Some(*f),Event::ReadState(client_core::read_state::Event::Result{result:Err(f),..})=>Some(*f),_=>None};
                        let error=write_emit(event).err().or(failure.filter(|f|f.ends_session()));
                        if let Some(error)=error {write_api.stop();let _=write_finished.send(Some(error));write_wake.request_repaint();break;}
                    }
                }));
                let mut history:Option<AbortTask>=None;
                let mut profile:Option<AbortTask>=None;
                let mut invite:Option<AbortTask>=None;
                let mut search:Option<AbortTask>=None;
                let mut gifs:Option<AbortTask>=None;
                let mut application_commands:Option<AbortTask>=None;
                let mut sticker_packs:Option<AbortTask>=None;
                let mut sticker_detail:Option<AbortTask>=None;
                let mut reaction_read:Option<AbortTask>=None;
                let mut ringing:Option<AbortTask>=None;
                let mut upload:Option<AbortTask>=None;
                let mut upload_cancel:Option<watch::Sender<bool>>=None;
                let mut voice_request=None;
                loop {
                    tokio::select! {
                        _=&mut gateway_task.0=>{break;}
                        _=&mut writes.0=>{break;}
                        changed=voice_availability.changed()=> {
                            if changed.is_err() {break;}
                            if !*voice_availability.borrow_and_update() {drop(ringing.take());drop(profile.take());drop(search.take());voice_request=None;if let Some(cancel)=&upload_cancel {let _=cancel.send(true);}}
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
                            let Some(command)=command else {break;};
                            if matches!(command,Command::CancelSearch) {drop(search.take());continue;}
                            if matches!(command,Command::CancelGifs) {drop(gifs.take());continue;}
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
								if *voice_availability.borrow() && dm_channels.lock().map_err(|_|Failure::Protocol)?.contains(channel) {
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
							if let Command::Voice(control)=command {
                                use client_core::voice::{Command as V,Event as E};
                                let (channel,request)=match control {V::Join{channel,request,..}|V::Ring{channel,request}|V::Leave{channel,request}|V::SetMute{channel,request,..}|V::SetCamera{channel,request,..}=>(channel,request),V::Decline{channel}=>(channel,0),V::Sync{..}|V::StartStream{..}|V::StopStream{..}|V::WatchStream{..}|V::StopWatching{..}=>unreachable!("sync and stream actions routed above")};
                                if !*voice_availability.borrow() {
                                    emit(Event::Voice(E::Failed{channel,request,message:"Voice is disconnected; no call was started"}))?;continue;
                                }
                                if let V::Leave{channel,request}=control && voice_request.is_some_and(|(id,r,_)|id==channel && r==request) {drop(ringing.take());}
                                let ring=match ring_action(control,user.id,&mut voice_request,dm_channels.lock().map_err(|_|Failure::Protocol)?.contains(&channel)) {
                                    Ok(action)=>action,
                                    Err(())=>{emit(Event::Voice(E::Failed{channel,request,message:"Call action expired; no ringing request was sent"}))?;continue;}
                                };
                                voice_send.try_send(control).map_err(|_|Failure::Capacity)?;
                                if let Some((recipient,stop))=ring {
                                    drop(ringing.take());
                                    let api=api.clone();let emit=emit.clone();let voice_send=voice_send.clone();let finished=finished.clone();let ring_wake=wake.clone();
                                    ringing=Some(AbortTask(tokio::spawn(async move {
                                        if let Err(failure)=api.ring_call(channel,recipient,stop).await {
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
			share_activity,
			own_presence,
			presence_edits,
			account_presence,
			presence_error,
			game_activity,
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

// Ring only after the media adapter confirms transport allocation, and only once per current call.
fn ring_action(
	control: client_core::voice::Command,
	owner: model::Id,
	active: &mut Option<(model::Id, u64, bool)>,
	dm: bool,
) -> Result<Option<(Option<model::Id>, bool)>, ()> {
	use client_core::voice::Command as V;
	if let V::Leave { channel, request } = control {
		if active.is_some_and(|(id, r, _)| id == channel && r == request) {
			*active = None;
			return Ok(dm.then_some((None, true)));
		}
		return Ok(None);
	}
	if !dm {
		return match control {
			V::Ring { .. } | V::Decline { .. } | V::Join { ring: true, .. } => Err(()),
			V::Sync { .. }
			| V::Join { .. }
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
		V::Sync { .. }
		| V::Leave { .. }
		| V::SetMute { .. }
		| V::SetCamera { .. }
		| V::StartStream { .. }
		| V::StopStream { .. }
		| V::WatchStream { .. }
		| V::StopWatching { .. } => Ok(None),
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
