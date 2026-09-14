//! Bounded, zlib-stream JSON Gateway. Normal-user Identify remains live-unverified.
mod activity;
mod channel_events;
mod compression;
#[cfg(test)]
mod login_tests;
mod presence;
mod thread_events;
mod voice;
use client_core::{
	Event, MAX_NAV,
	auth::{Failure, SessionSecret},
};
pub use discord_protocol::activity_sessions::Observation as ActivityObservation;
use discord_protocol::*;
use futures_util::{SinkExt, StreamExt};
use model::{Freshness, Id, Member, MemberList};
use std::{
	collections::BTreeMap,
	sync::Arc,
	time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
	sync::{mpsc, watch},
	time::{Instant, interval_at, sleep, timeout},
};
use tokio_tungstenite::{
	connect_async_with_config,
	tungstenite::{Message as Frame, protocol::WebSocketConfig},
};
use zeroize::Zeroizing;

fn socket_failure(error: tokio_tungstenite::tungstenite::Error) -> Failure {
	match error {
		tokio_tungstenite::tungstenite::Error::Capacity(_) => {
			Failure::CapacityAt("Gateway frame exceeds 64 MiB; connection stopped")
		}
		_ => Failure::Network,
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconnect {
	Resume,
	Identify,
	Stop,
}
pub fn close_action(code: u16) -> Reconnect {
	match code {
		4004 | 4010..=4014 | 1008 | 1009 => Reconnect::Stop,
		4007 | 4009 => Reconnect::Identify,
		_ => Reconnect::Resume,
	}
}
pub fn validated_url(value: &str) -> Result<String, Failure> {
	let mut url = url::Url::parse(value).map_err(|_| Failure::Protocol)?;
	let host = url.host_str().ok_or(Failure::Protocol)?;
	if url.scheme() != "wss"
		|| !(host == "gateway.discord.gg"
			|| (host.starts_with("gateway-") && host.ends_with(".discord.gg")))
		|| url.port().is_some()
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.path() != "/"
		|| url.fragment().is_some()
	{
		return Err(Failure::Protocol);
	}
	url.set_query(Some("v=10&encoding=json&compress=zlib-stream"));
	Ok(url.to_string())
}

fn reaction_event(
	name: &str,
	bytes: &[u8],
	sequenced: bool,
) -> Result<client_core::reactions::Event, Failure> {
	use client_core::reactions::Event as ReactionEvent;
	if sequenced {
		match name {
			"MESSAGE_REACTION_ADD" | "MESSAGE_REACTION_REMOVE" => {
				if let Ok(delta) = decode::<ReactionDelta>(bytes) {
					return Ok(ReactionEvent::Delta {
						channel: delta.channel_id,
						message: delta.message_id,
						user: delta.user_id,
						emoji: delta.emoji,
						add: name == "MESSAGE_REACTION_ADD",
						burst: delta.burst,
					});
				}
			}
			"MESSAGE_REACTION_REMOVE_EMOJI" => {
				if let Ok(target) = decode::<ReactionEmojiTarget>(bytes) {
					return Ok(ReactionEvent::Cleared {
						channel: target.channel_id,
						message: target.message_id,
						emoji: Some(target.emoji),
					});
				}
			}
			"MESSAGE_REACTION_REMOVE_ALL" => {
				let target = decode::<ReactionTarget>(bytes).map_err(|_| Failure::Protocol)?;
				return Ok(ReactionEvent::Cleared {
					channel: target.channel_id,
					message: target.message_id,
					emoji: None,
				});
			}
			_ => {}
		}
	}
	// Unknown details or an unsequenced event cannot safely change a count. Keep
	// the existing invalidation path without retaining unvalidated wire strings.
	let target = decode::<ReactionTarget>(bytes).map_err(|_| Failure::Protocol)?;
	Ok(ReactionEvent::Changed {
		channel: target.channel_id,
		message: target.message_id,
	})
}

#[derive(Default)]
struct ResumeState {
	session: Option<Zeroizing<String>>,
	url: Option<String>,
	sequence: Option<u64>,
}
#[derive(Default)]
struct Heartbeat {
	awaiting_since: Option<Instant>,
}
impl Heartbeat {
	fn tick(&mut self, now: Instant, interval: Duration) -> Result<(), Failure> {
		if self
			.awaiting_since
			.is_some_and(|sent| now.duration_since(sent) >= interval)
		{
			return Err(Failure::Network);
		}
		self.sent(now);
		Ok(())
	}
	fn sent(&mut self, now: Instant) {
		self.awaiting_since.get_or_insert(now);
	}
	fn ack(&mut self) {
		self.awaiting_since = None;
	}
}
fn next_attempt(attempt: u32, ready_for: Option<Duration>) -> u32 {
	// A stable connection earns a fresh retry budget; READY/reconnect flapping does not.
	if ready_for.is_some_and(|duration| duration >= Duration::from_secs(60)) {
		1
	} else {
		attempt + 1
	}
}
fn jitter_ms(max: u64) -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap_or_default()
		.subsec_nanos() as u64
		% max.max(1)
}

// Explicit troubleshooting only. Each scope shares its budgets across reconnects.
// Static labels never contain received event names, credentials, IDs or payloads.
struct Diagnostics {
	scope: &'static str,
	remaining: u8,
	bytes: usize,
}
impl Diagnostics {
	fn new(scope: &'static str, enabled: bool) -> Self {
		Self {
			scope,
			remaining: if enabled { 64 } else { 0 },
			bytes: 8 * 1024,
		}
	}
	fn record(&mut self, label: &'static str) {
		self.record_to(label, &mut std::io::stderr());
	}
	fn record_to(&mut self, label: &'static str, writer: &mut impl std::io::Write) {
		let bytes = self
			.scope
			.len()
			.saturating_add(label.len())
			.saturating_add(11);
		if self.remaining == 0 || bytes > self.bytes {
			return;
		}
		// Charge attempted output even if stderr is closed or accepts only part of a line.
		self.remaining -= 1;
		self.bytes -= bytes;
		let _ = writeln!(writer, "[Serein {}] {label}", self.scope);
	}
}
fn ignored_dispatch_label(name: Option<&str>) -> &'static str {
	if name.is_none_or(str::is_empty) {
		"dispatch name missing; ignored"
	} else {
		"unsupported dispatch ignored"
	}
}

#[derive(Clone)]
pub struct MemberSubscription {
	pub guild: Id,
	pub channel: Id,
	pub request: u64,
	pub list_id: String,
}
fn subscription_packet(guild: Id, channel: Option<Id>) -> Frame {
	let channels = channel.map_or_else(
		|| serde_json::json!({}),
		|channel| serde_json::json!({channel.to_string():[[0,99]]}),
	);
	// Unofficial guild subscriptions require typing=true before channel ranges work.
	// This receives events; it does not send a typing notification or request members in bulk.
	Frame::Text(serde_json::json!({"op":37,"d":{"subscriptions":{guild.to_string():{"typing":channel.is_some(),"threads":false,"activities":false,"members":[],"channels":channels}}}}).to_string().into())
}
struct ActiveMembers {
	subscription: MemberSubscription,
	rows: Vec<Option<Member>>,
	synced: bool,
	total: u64,
	pending_presence: BTreeMap<Id, model::MemberPresence>,
	presence_deadline: Option<Instant>,
}
impl ActiveMembers {
	fn new(subscription: MemberSubscription) -> Self {
		Self {
			subscription,
			rows: vec![None; 100],
			synced: false,
			total: 0,
			pending_presence: BTreeMap::new(),
			presence_deadline: None,
		}
	}
	fn snapshot(&self, freshness: Freshness) -> MemberList {
		MemberList {
			guild: Some(self.subscription.guild),
			channel: self.subscription.channel,
			request: self.subscription.request,
			rows: self.rows.clone(),
			total: self.total,
			freshness,
		}
	}
	fn update(&mut self, update: MemberUpdate) -> Result<bool, Failure> {
		if update.guild_id != self.subscription.guild || update.id != self.subscription.list_id {
			return Ok(false);
		}
		// The emitted full snapshot includes the mirror's latest statuses, so a
		// separate queued delta must not race it or reference a removed row.
		self.clear_presence();
		if update.ops.len() > 200 {
			return Err(Failure::Capacity);
		}
		self.total = update.member_count;
		for op in update.ops {
			match op {
				MemberOp::Sync {
					range: [start, end],
					items,
				} => {
					if start > end
						|| items.len() > end.saturating_sub(start).saturating_add(1)
						|| items.len() > 100
					{
						return Err(Failure::Protocol);
					}
					if start >= 100 {
						continue;
					}
					if items.is_empty() {
						continue;
					} // Empty SYNC may mean unchanged or unsupported; never fabricate an empty full list.
					self.rows[start..=end.min(99)].fill(None);
					for (index, item) in items.into_iter().enumerate() {
						if start + index < 100 {
							self.rows[start + index] = item.into_model();
						}
					}
					self.synced = true;
				}
				MemberOp::Invalidate {
					range: [start, end],
				} => {
					if start > end {
						return Err(Failure::Protocol);
					}
					if start < 100 {
						self.rows[start..=end.min(99)].fill(None);
						self.synced = false;
					}
				}
				MemberOp::Update { index, item } => {
					if index < 100 && self.synced {
						self.rows[index] = item.into_model();
					}
				}
				MemberOp::Insert { index, item } => {
					if index < 100 && self.synced {
						self.rows.insert(index, item.into_model());
						self.rows.truncate(100);
					}
				}
				MemberOp::Delete { index } => {
					if index < 100 && self.synced {
						self.rows.remove(index);
						self.rows.push(None);
					}
				}
			}
		}
		if self.rows.iter().flatten().map(Member::bytes).sum::<usize>() > 128 * 1024 {
			return Err(Failure::Capacity);
		}
		Ok(true)
	}
	fn clear_presence(&mut self) {
		self.pending_presence.clear();
		self.presence_deadline = None;
	}
	fn presence(&mut self, update: discord_protocol::presence::PresenceUpdate, now: Instant) {
		if !self.synced || update.guild != Some(self.subscription.guild) {
			return;
		}
		let Some(previous) = self
			.rows
			.iter()
			.flatten()
			.find(|row| row.user.id == update.user)
		else {
			return;
		};
		let explicit_unknown = matches!(&update.status, model::Patch::Null)
			|| matches!(&update.status, model::Patch::Value(status) if !matches!(status.as_str(), "online" | "idle" | "dnd" | "offline"));
		let status = match update.status {
			model::Patch::Absent => previous.status.clone(),
			model::Patch::Null => None,
			model::Patch::Value(status) => match status.as_str() {
				"online" | "idle" | "dnd" | "offline" => Some(status.as_str().to_owned()),
				_ => None,
			},
		};
		let activities = match update.activities {
			model::Patch::Absent => previous.activities.clone(),
			model::Patch::Null => vec![],
			model::Patch::Value(activities) => activities,
		};
		let mut custom_status = match update.custom_status {
			model::Patch::Absent => previous.custom_status.clone(),
			model::Patch::Null => None,
			model::Patch::Value(text) => Some(text.as_str().to_owned()),
		};
		if explicit_unknown || status.as_deref() == Some("offline") {
			custom_status = None;
		}
		let resolved = model::MemberPresence {
			user: update.user,
			activities: if explicit_unknown || status.as_deref() == Some("offline") {
				vec![]
			} else {
				activities
			},
			status,
			custom_status,
		};
		if !resolved.valid() {
			return;
		}
		if self.pending_presence.len() == 100 && !self.pending_presence.contains_key(&update.user) {
			return;
		}
		let projected = self
			.rows
			.iter()
			.flatten()
			.map(|row| {
				if row.user.id == update.user {
					client_core::presence::projected_row_bytes(row, &resolved)
				} else {
					row.bytes()
				}
			})
			.sum::<usize>();
		if projected > 128 * 1024 {
			return;
		}
		let mut changed = false;
		for row in self
			.rows
			.iter_mut()
			.flatten()
			.filter(|row| row.user.id == update.user)
		{
			if row.status != resolved.status
				|| row.custom_status != resolved.custom_status
				|| row.activities != resolved.activities
			{
				row.status = resolved.status.clone();
				row.custom_status = resolved.custom_status.clone();
				row.activities = resolved.activities.clone();
				changed = true;
			}
		}
		if changed {
			// <=100 entries within the member-row byte budget. The
			// deadline belongs to the first change, never to the latest packet.
			self.pending_presence.insert(update.user, resolved);
			self.presence_deadline
				.get_or_insert(now + Duration::from_millis(100));
		}
	}
	fn take_presence(&mut self) -> Option<Event> {
		self.presence_deadline = None;
		let pending = std::mem::take(&mut self.pending_presence);
		(self.synced && !pending.is_empty()).then(|| Event::MemberPresence {
			guild: self.subscription.guild,
			channel: self.subscription.channel,
			request: self.subscription.request,
			updates: pending.into_values().collect(),
		})
	}
}
pub async fn run(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	emit: impl Fn(Event) -> Result<(), Failure>,
) -> Result<(), Failure> {
	run_inner(
		secret,
		initial_url,
		subscriptions,
		mpsc::channel(1).1,
		None,
		emit,
		#[cfg(test)]
		None,
	)
	.await
}
/// Voice controls are admitted only after READY and are never replayed after a disconnect.
pub async fn run_with_voice(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	controls: mpsc::Receiver<client_core::voice::Command>,
	emit: impl Fn(Event) -> Result<(), Failure>,
) -> Result<(), Failure> {
	run_inner(
		secret,
		initial_url,
		subscriptions,
		controls,
		None,
		emit,
		#[cfg(test)]
		None,
	)
	.await
}
/// Publishes the latest bounded game activity and session presence after READY/RESUMED.
/// The documented wire shape does not establish normal-user compatibility.
pub async fn run_with_activity(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	controls: mpsc::Receiver<client_core::voice::Command>,
	activity: (
		watch::Receiver<Option<discord_protocol::rpc::Activity>>,
		watch::Receiver<model::OwnPresence>,
	),
	observe: impl Fn(ActivityObservation) -> Result<(), Failure> + Sync,
	emit: impl Fn(Event) -> Result<(), Failure>,
) -> Result<(), Failure> {
	run_inner(
		secret,
		initial_url,
		subscriptions,
		controls,
		Some(ActivityInput {
			receiver: activity.0,
			own_presence: activity.1,
			observe: &observe,
		}),
		emit,
		#[cfg(test)]
		None,
	)
	.await
}
struct ActivityInput<'a> {
	receiver: watch::Receiver<Option<discord_protocol::rpc::Activity>>,
	own_presence: watch::Receiver<model::OwnPresence>,
	observe: &'a (dyn Fn(ActivityObservation) -> Result<(), Failure> + Sync),
}
async fn run_inner(
	secret: Arc<SessionSecret>,
	initial_url: String,
	mut subscriptions: watch::Receiver<Option<MemberSubscription>>,
	mut voice_controls: mpsc::Receiver<client_core::voice::Command>,
	activity: Option<ActivityInput<'_>>,
	emit: impl Fn(Event) -> Result<(), Failure>,
	#[cfg(test)] test_endpoint: Option<&str>,
) -> Result<(), Failure> {
	let initial_url = validated_url(&initial_url)?;
	let activity_enabled = activity.is_some();
	let mut activity_open = activity_enabled;
	let mut presence_open = activity_enabled;
	let ignore_observation = |_| Ok(());
	let ActivityInput {
		receiver: mut activity,
		mut own_presence,
		observe,
	} = activity.unwrap_or_else(|| ActivityInput {
		receiver: watch::channel(None).1,
		own_presence: watch::channel(model::OwnPresence::default()).1,
		observe: &ignore_observation,
	});
	let mut last_observation = None;
	let mut outgoing_activity = activity::Pending::default();
	outgoing_activity.update(&activity.borrow_and_update())?;
	outgoing_activity.update_presence(&own_presence.borrow_and_update())?;
	let mut member_diagnostics = Diagnostics::new(
		"members",
		std::env::var_os("SEREIN_MEMBER_DIAGNOSTICS").as_deref() == Some(std::ffi::OsStr::new("1")),
	);
	let mut gateway_diagnostics = Diagnostics::new(
		"gateway",
		std::env::var_os("SEREIN_GATEWAY_DIAGNOSTICS").as_deref()
			== Some(std::ffi::OsStr::new("1")),
	);
	let mut state = ResumeState::default();
	let mut was_ready = false;
	let mut owner_id = None;
	let mut attempt = 0;
	let mut calls = voice::Calls::default();
	let mut known_guilds = std::collections::BTreeSet::new();
	let mut voice_open = true;
	while attempt < 6 {
		if attempt > 0 {
			calls.disconnected();
			while voice_controls.try_recv().is_ok() {}
			emit(Event::Disconnected)?;
			sleep(Duration::from_millis(
				(1000_u64 << attempt.min(5)) + jitter_ms(1000),
			))
			.await;
		}
		let url = state.url.as_deref().unwrap_or(&initial_url);
		// Compiled out of shipped builds. Tests replace only dialing, never URL validation.
		#[cfg(test)]
		let url = test_endpoint.unwrap_or(url);
		let config = WebSocketConfig::default()
			.max_message_size(Some(MAX_GATEWAY_WIRE))
			.max_frame_size(Some(MAX_GATEWAY_WIRE))
			.write_buffer_size(0)
			.max_write_buffer_size(64 * 1024);
		let connection = timeout(
			Duration::from_secs(15),
			connect_async_with_config(url, Some(config), false),
		)
		.await;
		let Ok(Ok((mut socket, _))) = connection else {
			attempt += 1;
			continue;
		};
		let mut compression = compression::Decoder::default();
		let hello = timeout(Duration::from_secs(10), async {
			while let Some(frame) = socket.next().await {
				let frame = frame.map_err(socket_failure)?;
				if let Some(frame) = compression.frame(frame)? {
					match frame {
						Frame::Ping(_) | Frame::Pong(_) => continue,
						frame => return Ok(frame),
					}
				}
			}
			Err(Failure::Network)
		})
		.await;
		if let Ok(Err(failure)) = hello
			&& failure.ends_session()
		{
			return Err(failure);
		}
		let Ok(Ok(Frame::Text(text))) = hello else {
			attempt += 1;
			continue;
		};
		let packet: GatewayPacket =
			decode_gateway(text.as_bytes()).map_err(|_| Failure::Protocol)?;
		if packet.op != 10 {
			return Err(Failure::Protocol);
		}
		let hello: Hello = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?;
		if !(1000..=120_000).contains(&hello.heartbeat_interval) {
			return Err(Failure::Protocol);
		}
		outgoing_activity.update_presence(&own_presence.borrow_and_update())?;
		let handshake = if let (Some(session), Some(sequence)) = (&state.session, state.sequence) {
			serde_json::json!({"op":6,"d":{"token":secret.expose(),"session_id":session.as_str(),"seq":sequence}})
		} else {
			// Normal-user Identify with the same browser fingerprint as REST; a mismatched or
			// custom identity is what gets the account quarantined as spam.
			let properties: serde_json::Value =
				serde_json::from_str(&client_core::fingerprint::properties()).unwrap_or_default();
			serde_json::json!({"op":2,"d":{"token":secret.expose(),"compress":false,"properties":properties,"presence":outgoing_activity.identify_presence()}})
		};
		let encoded = Zeroizing::new(handshake.to_string());
		drop(handshake);
		if !matches!(
			timeout(
				Duration::from_secs(5),
				socket.send(Frame::Text(encoded.as_str().to_owned().into())),
			)
			.await,
			Ok(Ok(()))
		) {
			attempt += 1;
			continue;
		}
		let mut heartbeat = Heartbeat::default();
		let interval = Duration::from_millis(hello.heartbeat_interval);
		let mut timer = interval_at(
			Instant::now() + Duration::from_millis(jitter_ms(hello.heartbeat_interval)),
			interval,
		);
		timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		let mut ready_at: Option<Instant> = None;
		let ready_deadline = Instant::now() + Duration::from_secs(30);
		let mut active_members: Option<ActiveMembers> = None;
		let mut direct_presence = presence::Pending::default();
		let mut sent_members = false;
		let mut members_deadline: Option<Instant> = None;
		let mut subscriptions_open = true;
		outgoing_activity.reconnect();
		loop {
			if activity_enabled && last_observation != Some(outgoing_activity.observation) {
				observe(outgoing_activity.observation)?;
				last_observation = Some(outgoing_activity.observation);
			}
			if ready_at.is_some() && !sent_members {
				let subscription = subscriptions.borrow_and_update().clone();
				if let Some(subscription) = subscription {
					if subscription.list_id.len() > 32 {
						return Err(Failure::Protocol);
					}
					if !matches!(
						timeout(
							Duration::from_secs(5),
							socket.send(subscription_packet(
								subscription.guild,
								Some(subscription.channel)
							))
						)
						.await,
						Ok(Ok(()))
					) {
						break;
					}
					member_diagnostics.record("subscription sent: first 100 positions");
					if let Some(active) = &mut active_members {
						// Channels with the same list identity share the existing SYNC.
						// Keep its rows: another subscription may only receive deltas.
						active.subscription = subscription;
						active.clear_presence();
						let freshness = if active.synced {
							Freshness::Fresh
						} else if members_deadline.is_some() {
							Freshness::Loading
						} else {
							Freshness::Unavailable
						};
						emit(Event::Members(active.snapshot(freshness)))?;
					} else {
						active_members = Some(ActiveMembers::new(subscription));
						members_deadline = Some(Instant::now() + Duration::from_secs(15));
					}
				}
				if active_members.is_none() {
					member_diagnostics.record(
						"no active server subscription (closed pane or unavailable metadata)",
					);
				}
				sent_members = true;
			}
			let presence_deadline = active_members
				.as_ref()
				.and_then(|active| active.presence_deadline);
			let activity_deadline = if activity_enabled && ready_at.is_some() {
				outgoing_activity.deadline()
			} else {
				None
			};
			tokio::select! {
				changed = own_presence.changed(), if presence_open => {
					presence_open = changed.is_ok();
					outgoing_activity.update_presence(&own_presence.borrow_and_update())?;
				}
				changed = activity.changed(), if activity_open => {
					activity_open = changed.is_ok();
					outgoing_activity.update(&activity.borrow_and_update())?;
				}
				_ = tokio::time::sleep_until(activity_deadline.unwrap_or(ready_deadline)), if activity_deadline.is_some() => {
					// Read the latest value even if a watch notification races the timer.
					outgoing_activity.update(&activity.borrow_and_update())?;
					outgoing_activity.update_presence(&own_presence.borrow_and_update())?;
					if let Some(packet) = outgoing_activity.packet(Instant::now())
						&& !matches!(timeout(Duration::from_secs(5), socket.send(packet)).await, Ok(Ok(()))) { break; }
				}
				_=tokio::time::sleep_until(direct_presence.deadline.unwrap_or(ready_deadline)), if direct_presence.deadline.is_some() && ready_at.is_some() => {
					if let Some(event)=direct_presence.take() { emit(event)?; }
				}
				command=voice_controls.recv(), if voice_open && ready_at.is_some() => {
					let Some(command)=command else {voice_open=false;continue;};
					let connect=if let client_core::voice::Command::Join{channel,..}=command {Some(channel)}else{None};
					let stream=matches!(command,client_core::voice::Command::StartStream{..}|client_core::voice::Command::StopStream{..}|client_core::voice::Command::WatchStream{..}|client_core::voice::Command::StopWatching{..});
					let packet=match if stream {calls.stream_packet(command,owner_id)} else {calls.packet(command)} {
						Ok(packet)=>packet,
						Err(_) => {
							match command {
								client_core::voice::Command::Join{channel,request,..} => emit(Event::Voice(client_core::voice::Event::Failed{channel,request,message:"Previous call is still leaving, or the channel is unavailable; wait for departure or reconnect"}))?,
								client_core::voice::Command::StartStream{channel,request,stream_request} => emit(Event::Voice(client_core::voice::Event::Stream{channel,request,stream_request,event:client_core::screen::Event::Failed("A screen share is already active, stopping, or the call is unavailable")}))?,
								client_core::voice::Command::WatchStream{channel,request,stream_request,streamer} => emit(Event::Voice(client_core::voice::Event::Watch{channel,request,stream_request,streamer,event:client_core::screen::Event::Failed("Another stream is already being watched, or the call is unavailable")}))?,
								_=>{}
							}
							continue;
						}
					};
					if let Some(channel)=connect && let Some(packet)=calls.packet(client_core::voice::Command::Sync { channel })?
						&& !matches!(timeout(Duration::from_secs(5),socket.send(packet)).await,Ok(Ok(()))) {break;}
					if let Some(packet)=packet && !matches!(timeout(Duration::from_secs(5),socket.send(packet)).await,Ok(Ok(()))) {break;}
				}

				_=tokio::time::sleep_until(calls.departure_deadline.unwrap_or(ready_deadline)), if calls.departure_deadline.is_some() => {
					if let Some(event)=calls.departure_expired() {emit(event)?;}
				}
				changed=subscriptions.changed(), if subscriptions_open && ready_at.is_some() => {
					subscriptions_open=changed.is_ok();
					let same_list = subscriptions_open && (members_deadline.is_some() || active_members.as_ref().is_some_and(|active| active.synced)) && active_members.as_ref().is_some_and(|active| {
						subscriptions.borrow().as_ref().is_some_and(|next| {
							next.guild == active.subscription.guild && next.list_id == active.subscription.list_id
						})
					});
					if !same_list {
						if let Some(old)=active_members.take() && !matches!(timeout(Duration::from_secs(5),socket.send(subscription_packet(old.subscription.guild,None))).await,Ok(Ok(()))) {break;}
						members_deadline=None;
					}
					member_diagnostics.record("subscription replaced or canceled");
					sent_members = !subscriptions_open;
				}
				_=tokio::time::sleep_until(members_deadline.unwrap_or(ready_deadline)), if members_deadline.is_some() => {
					member_diagnostics.record("timeout: no populated member SYNC within 15 seconds");
					if let Some(active)=&mut active_members {active.clear_presence();active.rows.clear();active.rows.resize(100,None);active.synced=false;emit(Event::Members(active.snapshot(Freshness::Unavailable)))?;}
					members_deadline=None;
				}
				_=tokio::time::sleep_until(presence_deadline.unwrap_or(ready_deadline)), if presence_deadline.is_some() => {
					if let Some(active)=&mut active_members {
						if subscriptions.has_changed().unwrap_or(true) {active.clear_presence();}
						else if let Some(event)=active.take_presence() {emit(event)?;}
					}
				}
				_ = tokio::time::sleep_until(ready_deadline), if ready_at.is_none() => break,
				_ = timer.tick() => {
					if heartbeat.tick(Instant::now(), interval).is_err() { break; }
					let packet = serde_json::json!({"op":1,"d":state.sequence}).to_string();
					if !matches!(timeout(Duration::from_secs(5), socket.send(Frame::Text(packet.into()))).await, Ok(Ok(()))) { break; }
				}
				frame = socket.next() => {
					let frame = match frame {
						Some(Ok(frame)) => match compression.frame(frame)? {
							Some(frame) => Some(Ok(frame)),
							None => continue,
						},
						Some(Err(error)) => {
							let failure = socket_failure(error);
							if failure.ends_session() { return Err(failure); }
							break;
						},
						other => other,
					};
					match frame {
						Some(Ok(Frame::Text(text))) => {
							let packet: GatewayPacket = decode_gateway(text.as_bytes()).map_err(|_| Failure::Protocol)?;
							// Reaction counts are additive: do not apply a repeated dispatch or
							// move the resume cursor backwards when one is replayed.
							if packet.op == 0
								&& matches!(packet.t.as_deref(), Some("MESSAGE_REACTION_ADD" | "MESSAGE_REACTION_REMOVE" | "MESSAGE_REACTION_REMOVE_ALL" | "MESSAGE_REACTION_REMOVE_EMOJI"))
								&& packet.s.zip(state.sequence).is_some_and(|(next, last)| next <= last)
							{ continue; }
							if let Some(sequence) = packet.s { state.sequence = Some(sequence); }
							match packet.op {
								11 => heartbeat.ack(),
								1 => {
									let packet = serde_json::json!({"op":1,"d":state.sequence}).to_string();
									if !matches!(timeout(Duration::from_secs(5), socket.send(Frame::Text(packet.into()))).await, Ok(Ok(()))) { break; }
									heartbeat.sent(Instant::now());
								}
								7 => break,
								9 => {
									let resumable: bool = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?;
									if !resumable { state = ResumeState::default(); }
									emit(Event::Disconnected)?;
									sleep(Duration::from_millis(1000 + jitter_ms(4000))).await;
									break;
								}
								0 => match packet.t.as_deref().unwrap_or("") {
									"READY" => {
										direct_presence=presence::Pending::default();
										active_members=None;members_deadline=None;sent_members = !subscriptions_open;
										let envelope = ready::decode(packet.d.get().as_bytes()).map_err(|_| Failure::ProtocolAt("Gateway login: invalid READY identity or relationships"))?;
										if envelope.user.bot { return Err(Failure::InvalidCredential); }
										let permissions = envelope.permissions().map_err(|_|Failure::ProtocolAt("Gateway login: invalid permission metadata"))?;
										let (mut ready, warnings) = envelope.navigation().map_err(|_| Failure::ProtocolAt("Gateway login: invalid READY guild or channel metadata"))?;
										owner_id=Some(ready.user.id);
										if ready.user.username.is_empty() || ready.user.username.len() > 128 || ready.user.username.chars().any(char::is_control) || ready.session_id.is_empty() || ready.session_id.chars().any(char::is_control) {
											return Err(Failure::ProtocolAt("Gateway login: invalid account identity or session ID"));
										}
										if ready.session_id.len() > 2048 { return Err(Failure::CapacityAt("Gateway session ID exceeds 2 KiB; connection stopped")); }
										state.url = Some(validated_url(&ready.resume_gateway_url).map_err(|f|f.protocol_at("Gateway login: resume address rejected"))?);
										state.session = Some(Zeroizing::new(std::mem::take(&mut ready.session_id)));
										let friends = ready.relationships.as_ref().map(|s| s.friends(&ready.users)).transpose().map_err(|_| Failure::ProtocolAt("Invalid friend metadata"))?;
										let requests = ready.relationships.as_ref().map(|s| s.requests(&ready.users)).transpose().map_err(|_| Failure::ProtocolAt("Invalid friend request metadata"))?;
										calls.session_reset();
										calls.remember_users(std::mem::take(&mut ready.users));
										known_guilds=channel_events::ready_calls(&ready,&mut calls)?;
										let mut participants = Vec::new();
										let mut roster_bytes = 0;
										for guild in &mut ready.guilds {
											if guild.voice_states.is_empty() { continue; }
											if let Event::Voice(client_core::voice::Event::Snapshot { participants: mut rows, .. }) = calls.snapshot(guild, false)? {
												roster_bytes += rows.iter().map(client_core::voice::RosterEntry::bytes).sum::<usize>();
												if participants.len() + rows.len() > client_core::voice::MAX_ROSTER { return Err(Failure::CapacityAt("Voice roster participant limit exceeded")); }
												if roster_bytes > client_core::voice::MAX_ROSTER_BYTES { return Err(Failure::CapacityAt("Voice roster byte limit exceeded")); }
												participants.append(&mut rows);
											}
										}
										let (guilds, channels) = ready.navigation().map_err(|_| Failure::ProtocolAt("Gateway login: invalid or oversized channel/thread navigation"))?;
										let (read_entries,read_version,partial)=ready.read_state.take().map_or((None,None,false),|snapshot|(Some(snapshot.entries.into_iter().filter(|e|e.kind==0).map(|e|(e.id,e.last_message_id,e.mention_count)).collect()),snapshot.version,snapshot.partial));
										if guilds.len() + channels.len() > MAX_NAV { return Err(Failure::CapacityAt("Account navigation exceeds 131,072 entries; connection stopped")); }
										direct_presence.bootstrap_users=friends.as_ref().into_iter().flatten().map(|(u,_)|u.id).chain(channels.iter().filter(|c|c.guild.is_none() && matches!(c.kind,1|3)).flat_map(|c|c.recipients.iter().map(|u|u.id))).take(client_core::presence::MAX_DIRECT_PRESENCES).collect();
										calls.allowed=channels.iter().filter(|c|(c.guild.is_none() && c.kind==1 && c.recipients.len()==1) || (c.guild.is_some() && c.kind==2)).map(|c|(c.id,c.guild)).collect();
										if was_ready { emit(Event::Resync)?; }
										let notifications = ready.user_guild_settings.take().map(|snapshot| {
											let (entries, replace) = snapshot.entries();
											notification_preferences(entries, replace)
										});
										emit(Event::Startup(Box::new(client_core::Startup {
											user: ready.user.into_model(), guilds, channels, permissions,
											read_state: client_core::read_state::Event::Snapshot {entries:read_entries,version:read_version,partial},
											notifications, session_dnd: ready.sessions.as_ref().and_then(|s| s.dnd()), warnings,
										}.prepare()?)))?; was_ready = true;

										let nicknames = ready.relationships.as_ref().map(|s| s.nicknames());
										emit(Event::UserAction(client_core::user_actions::Event::Relationships(ready.relationships.take().map(|s| s.entries()))))?;
										emit(Event::UserAction(client_core::user_actions::Event::Friends(friends)))?;
										emit(Event::UserAction(client_core::user_actions::Event::Requests(requests)))?;
										if let Some(nicknames) = nicknames { emit(Event::UserAction(client_core::user_actions::Event::Nicknames(nicknames)))?; }
										if let Some(friends) = ready.merged_presences.as_ref().and_then(|m| m.friends.as_deref()).or(ready.presences.as_deref()) {
											direct_presence.friends(friends, Instant::now(), &emit)?;
										}
										if !participants.is_empty() { emit(Event::Voice(client_core::voice::Event::Snapshot { partial: false, guild: None, participants }))?; }
										ready_at = Some(Instant::now());
									}
									"READY_SUPPLEMENTAL" => {
										let (mut extra, warnings) = ready::supplemental(packet.d.get().as_bytes()).map_err(|_|Failure::ProtocolAt("Gateway login: invalid supplemental guild or voice metadata"))?;
										if warnings != model::account::Warnings::default() { emit(Event::StartupWarnings(warnings))?; }
										if let Some(friends) = extra.merged_presences.as_ref().and_then(|m| m.friends.as_deref()).or(extra.presences.as_deref()) {
											direct_presence.friends(friends, Instant::now(), &emit)?;
										}
										direct_presence.bootstrap_users.clear();
										if let Some(owner)=owner_id {
											let updates=permissions::supplemental(packet.d.get().as_bytes(),owner).map_err(|_|Failure::Protocol)?;
											if !updates.is_empty() {emit(Event::Permissions(client_core::permissions::Event::Members(updates)))?;}
										}

										if extra.guilds.len() > MAX_NAV || extra.merged_members.len() > MAX_NAV { return Err(Failure::CapacityAt("Supplemental login exceeds 131,072 server groups; connection stopped")); }
										let mut participants = Vec::new();
										let mut roster_bytes = 0;
										for (index, guild) in extra.guilds.iter_mut().enumerate() {
											if let Some(members) = extra.merged_members.get_mut(index) { guild.members.append(members); }
											if let Event::Voice(client_core::voice::Event::Snapshot { participants: mut rows, .. }) = calls.snapshot(guild, true)? {
												roster_bytes += rows.iter().map(client_core::voice::RosterEntry::bytes).sum::<usize>();
												if participants.len() + rows.len() > client_core::voice::MAX_ROSTER { return Err(Failure::CapacityAt("Voice roster participant limit exceeded")); }
												if roster_bytes > client_core::voice::MAX_ROSTER_BYTES { return Err(Failure::CapacityAt("Voice roster byte limit exceeded")); }
												participants.append(&mut rows);
											}
										}
										if !participants.is_empty() { emit(Event::Voice(client_core::voice::Event::Snapshot { partial: true, guild: None, participants }))?; }
										calls.users.clear();
									}
									"RESUMED" => { emit(Event::Resumed)?; ready_at = Some(Instant::now()); },
									"CALL_CREATE" | "CALL_UPDATE" | "CALL_DELETE" | "VOICE_STATE_UPDATE" | "VOICE_SERVER_UPDATE" | "STREAM_CREATE" | "STREAM_SERVER_UPDATE" | "STREAM_DELETE" => calls.dispatch(packet.t.as_deref().unwrap_or(""),packet.d.get().as_bytes(),owner_id,&emit)?,
									"GUILD_MEMBER_LIST_UPDATE" => {
										if let Some(active)=&mut active_members {
											let decoded = decode::<MemberUpdate>(packet.d.get().as_bytes());
											match &decoded {
												Err(_) => member_diagnostics.record("reply decode failed: unsupported member payload"),
												Ok(update) if update.guild_id != active.subscription.guild || update.id != active.subscription.list_id => member_diagnostics.record("reply ignored: different guild or list identity"),
												Ok(update) if update.ops.iter().any(|op| matches!(op, MemberOp::Sync {items, ..} if !items.is_empty())) => member_diagnostics.record("reply: populated SYNC received"),
												Ok(update) if update.ops.iter().any(|op| matches!(op, MemberOp::Sync {items, ..} if items.is_empty())) => member_diagnostics.record("reply: empty SYNC received"),
												Ok(_) => member_diagnostics.record("reply: incremental operations only; no SYNC"),
											}
											match decoded.map_err(|_|Failure::Protocol).and_then(|update|active.update(update)) {
												Ok(true)=>{member_diagnostics.record(if active.synced {"snapshot synchronized"} else {"snapshot still awaiting populated SYNC"});let freshness=if active.synced {Freshness::Fresh}else{Freshness::Stale};emit(Event::Members(active.snapshot(freshness)))?;if active.synced {members_deadline=None;}else{members_deadline.get_or_insert(Instant::now()+Duration::from_secs(15));}},
												Ok(false)=>{},
												Err(_)=>{member_diagnostics.record("snapshot unavailable: decode, range or capacity failure");active.clear_presence();active.rows.fill(None);active.synced=false;emit(Event::Members(active.snapshot(Freshness::Unavailable)))?;members_deadline=None;}
											}
										}
									}
									"PRESENCE_UPDATE" => {
										if let Ok(update)=discord_protocol::presence::decode(packet.d.get().as_bytes()) {
											if update.guild.is_none() { direct_presence.push(update,Instant::now()); }
											else if let Some(active)=&mut active_members && active.synced { active.presence(update,Instant::now()); }
										}
									}
									"TYPING_START" => {
										// Malformed ephemeral signals must not interrupt message delivery.
										if let Ok(typing)=discord_protocol::typing::decode(packet.d.get().as_bytes()) {
											emit(Event::Typing(client_core::typing::Signal { channel:typing.channel_id,user:typing.user_id,timestamp:typing.timestamp }))?;
										}
									}
									"CHANNEL_RECIPIENT_ADD" => {let d:RecipientAdded=decode(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;emit(Event::RecipientAdded {channel:d.channel_id,user:d.user.into_model()})?;}
									"CHANNEL_RECIPIENT_REMOVE" => {let d:RecipientRemoved=decode(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;emit(Event::RecipientRemoved {channel:d.channel_id,user:d.user.id})?;}
									"USER_NOTE_UPDATE" => {
										#[derive(serde::Deserialize)]
										struct NoteUpdate { id: Id, note: Option<String> }
										let note: NoteUpdate = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?;
										emit(Event::UserAction(client_core::user_actions::Event::NoteChanged { user: note.id, text: note.note.unwrap_or_default() }))?;
									}
									"RELATIONSHIP_ADD" | "RELATIONSHIP_UPDATE" | "RELATIONSHIP_REMOVE" => {
										let relationship: discord_protocol::relationships::Relationship = decode(packet.d.get().as_bytes()).map_err(|_| Failure::ProtocolAt("Unsupported relationship update"))?;
										emit(Event::UserAction(client_core::user_actions::Event::Relationship { user: relationship.id, blocked: packet.t.as_deref() != Some("RELATIONSHIP_REMOVE") && relationship.kind == 2 }))?;
										let friend = packet.t.as_deref() != Some("RELATIONSHIP_REMOVE") && relationship.kind == 1;
										let profile = relationship.user.map(discord_protocol::relationships::friend).transpose().map_err(|_| Failure::ProtocolAt("Invalid friend metadata"))?;
										let incoming = (packet.t.as_deref() != Some("RELATIONSHIP_REMOVE") && matches!(relationship.kind,3|4)).then_some(relationship.kind==3);
										emit(Event::UserAction(client_core::user_actions::Event::Friend { user: relationship.id, friend, profile: profile.clone() }))?;
										emit(Event::UserAction(client_core::user_actions::Event::Request { user: relationship.id, incoming, profile }))?;
										if friend { match relationship.nickname {
											model::Patch::Absent => {},
											model::Patch::Null => emit(Event::UserAction(client_core::user_actions::Event::Nickname { user: relationship.id, text: String::new() }))?,
											model::Patch::Value(text) => emit(Event::UserAction(client_core::user_actions::Event::Nickname { user: relationship.id, text }))?,
										} }
									}
									"USER_UPDATE" => {
										let user: discord_protocol::UserDto = decode(packet.d.get().as_bytes()).map_err(|_| Failure::ProtocolAt("Invalid user update"))?;
										let profile = discord_protocol::relationships::friend(user).map_err(|_| Failure::ProtocolAt("Invalid user update"))?;
										emit(Event::UserAction(client_core::user_actions::Event::FriendProfile(profile)))?;
									}
									"USER_GUILD_SETTINGS_UPDATE" => {
										let setting=decode::<discord_protocol::notifications::Setting>(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										emit(notification_settings(vec![setting],false))?;
									}
									"SESSIONS_REPLACE" => {
										if activity_enabled { outgoing_activity.observe(packet.d.get().as_bytes(), state.session.as_deref().map(String::as_str).unwrap_or_default()); }
										let sessions=decode::<discord_protocol::notifications::Sessions>(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										emit(Event::NotificationPreferences(client_core::notifications::Event::Presence(sessions.dnd())))?;
									}
									"USER_SETTINGS_PROTO_UPDATE" => emit(Event::NotificationPreferences(client_core::notifications::Event::Invalidate))?,
									"MESSAGE_CREATE" => emit(Event::Message(decode::<MessageDto>(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?.into_model()))?,
									"MESSAGE_ACK" => {
										let ack=decode::<read_state::Ack>(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										emit(Event::ReadState(client_core::read_state::Event::Ack{channel:ack.channel_id,message:ack.message_id,manual:ack.manual,mention_count:ack.mention_count,version:ack.version}))?;
									}
									"PASSIVE_UPDATE_V2" => {
										let envelope = ready::passive(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										if let Some(owner)=owner_id && let Some((guild,roles,timeout_until))=envelope.permissions(owner).map_err(|_|Failure::Protocol)? {
											emit(Event::Permissions(client_core::permissions::Event::Member {guild,roles,timeout_until}))?;
										}
										let mut update = envelope.voice().map_err(|_|Failure::Protocol)?;
										let latest = std::mem::take(&mut update.updated_channels);
										calls.passive(update,owner_id,&emit)?;
										emit(Event::ReadState(client_core::read_state::Event::Latest(latest.into_iter().map(|c|(c.id,c.last_message_id)).collect())))?;
									}
									"NOTIFICATION_CENTER_ITEM_CREATE" => {
										if let Ok(Some(item))=notification_settings::social_notification(packet.d.get().as_bytes()){emit(Event::SocialNotification(item))?;}
									}
									"MESSAGE_UPDATE" => emit(Event::Patch(decode::<PatchDto>(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?.into_model()))?,
									"MESSAGE_REACTION_ADD" | "MESSAGE_REACTION_REMOVE" | "MESSAGE_REACTION_REMOVE_ALL" | "MESSAGE_REACTION_REMOVE_EMOJI" => {
										emit(Event::Reactions(reaction_event(packet.t.as_deref().unwrap_or(""), packet.d.get().as_bytes(), packet.s.is_some())?))?;
									}
									"MESSAGE_DELETE" => { let d: Deleted = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?; emit(Event::Delete { channel:d.channel_id, id:d.id })?; }
									"MESSAGE_DELETE_BULK" => { let d: BulkDeleted = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?; if d.ids.len() > 100 { return Err(Failure::Capacity); } emit(Event::DeleteBulk { channel:d.channel_id, ids: d.ids })?; }
									"AUTH_SESSION_CHANGE" => return Err(Failure::Expired),
									"CHANNEL_DELETE" => { let c: ChannelDto = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?; calls.invalidate(c.id); emit(Event::Unavailable(c.id))?; }
									"CHANNEL_CREATE" => {
										let permissions=owner_id.map(|owner|channel_events::permission_metadata(packet.d.get().as_bytes(),owner)).transpose()?.flatten();
										let event=channel_events::create(packet.d.get().as_bytes())?;
										match &event {
											Event::ChannelCreated(c) => channel_events::admit_call(c,&known_guilds,&mut calls),
											Event::Unavailable(id) => calls.invalidate(*id),
											_=>{}
										}
										emit(event)?;
										if let Some(permissions)=permissions {emit(permissions)?;}
									}
									"CHANNEL_UPDATE" => {
										let permissions=owner_id.map(|owner|channel_events::permission_metadata(packet.d.get().as_bytes(),owner)).transpose()?.flatten();
										let update=channel_events::update(packet.d.get().as_bytes())?;
										if let Some(channel)=update.restored {channel_events::admit_call(&channel,&known_guilds,&mut calls);emit(Event::ChannelRestored(channel))?;}
										if let Event::Unavailable(id)=&update.event {calls.invalidate(*id);}
										if let Event::ChannelChanged(patch)=&update.event && let model::Patch::Value(kind)=patch.kind && kind != 2 && kind != 1 {calls.invalidate(patch.id);}
										emit(update.event)?;
										if let Some(permissions)=permissions {emit(permissions)?;}
									}
									"THREAD_CREATE" | "THREAD_UPDATE" | "THREAD_DELETE" | "THREAD_LIST_SYNC" | "THREAD_MEMBERS_UPDATE" => {
										if let Some(event) = thread_events::decode_event(packet.t.as_deref().unwrap_or(""), packet.d.get().as_bytes(), owner_id)? { emit(event)?; }
									}
									"GUILD_EMOJIS_UPDATE" => {
										let update: GuildEmojisUpdate = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?;
										emit(Event::GuildEmojis { guild: update.guild_id, emojis: update.emojis.0 })?;
									}
									"GUILD_CREATE" => {
										member_diagnostics.record("guild refresh received");
										let permissions=owner_id.map(|owner|permissions::guild(packet.d.get().as_bytes(),owner)).transpose().map_err(|_|Failure::ProtocolAt("Gateway guild refresh: invalid permission metadata"))?;
										let mut guild: GuildDto = decode(packet.d.get().as_bytes()).map_err(|_| Failure::ProtocolAt("Gateway guild refresh: unsupported guild payload"))?;
										if guild.channels.len() + calls.allowed.len() > MAX_NAV { return Err(Failure::Capacity); }
										if !known_guilds.contains(&guild.id) {
											if known_guilds.len() >= MAX_NAV { return Err(Failure::Capacity); }
											known_guilds.insert(guild.id);
											let name = guild.properties.as_ref().and_then(|p| match &p.name { model::Patch::Value(name) => Some(name), _ => None }).unwrap_or(&guild.name).chars().take(128).collect();
											let icon = guild.properties.as_ref().and_then(|p| match &p.icon { model::Patch::Value(icon) => Some(icon.clone()), _ => None }).or_else(|| guild.icon.clone()).filter(|h| model::valid_avatar_hash(h));
											emit(Event::GuildJoined(model::Guild { id: guild.id, name, icon, emojis: None }))?;
										}

										if let Some(permissions)=permissions {emit(Event::Permissions(client_core::permissions::Event::Snapshot(permissions)))?;}
										let hidden:std::collections::BTreeSet<_>=guild.channels.iter().filter(|c|c.is_obfuscated()).map(|c|c.id).collect();
										for mut channel in std::mem::take(&mut guild.channels) {
											channel.guild_id = Some(guild.id);
											let event=if matches!(channel.kind,10..=12) && channel.parent_id.is_some_and(|id|hidden.contains(&id)) {Event::Unavailable(channel.id)} else {channel_events::created(channel)};
											match &event {
												Event::ChannelCreated(channel)=>channel_events::admit_call(channel,&known_guilds,&mut calls),
												Event::Unavailable(id)=>calls.invalidate(*id),
												_=>{}
											}
											emit(event)?;
										}
										emit(calls.snapshot(&mut guild, false)?)?;
										if let Some(emojis) = guild.emojis { emit(Event::GuildEmojis { guild: guild.id, emojis: emojis.0 })?; }
									}
									"GUILD_UPDATE" => {
										let owner=permissions::owner(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										emit(Event::GuildChanged(decode::<GuildPatchDto>(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?.into_model()))?;
										if let Some((guild,owner))=owner {emit(Event::Permissions(client_core::permissions::Event::Owner {guild,owner}))?;}
									}
									"GUILD_MEMBER_UPDATE" => {
										if let Some(owner)=owner_id && let Some((guild,roles,timeout_until))=permissions::member(packet.d.get().as_bytes(),owner).map_err(|_|Failure::Protocol)? {
											emit(Event::Permissions(client_core::permissions::Event::Member {guild,roles,timeout_until}))?;
										}
									}
									"GUILD_DELETE" => {
										let guild: GuildDto = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?;
										let removed: Vec<_> = calls.allowed.iter().filter_map(|(channel, id)| (*id == Some(guild.id)).then_some(*channel)).collect();
										for channel in removed { calls.invalidate(channel); emit(Event::Unavailable(channel))?; }
										emit(Event::GuildEmojis { guild: guild.id, emojis: Vec::new() })?;
										emit(Event::Permissions(client_core::permissions::Event::UnavailableGuild(guild.id)))?;
									}
									"GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
										let (guild,role)=permissions::role(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										emit(Event::Permissions(client_core::permissions::Event::Role {guild,role}))?;
									}
									"GUILD_ROLE_DELETE" => {
										let (guild,id)=permissions::role_removed(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										emit(Event::Permissions(client_core::permissions::Event::RoleRemoved {guild,id}))?;
									}
									_ => gateway_diagnostics.record(ignored_dispatch_label(packet.t.as_deref())),
								},
								_ => {
									gateway_diagnostics.record("unsupported opcode; connection stopped");
									return Err(Failure::Protocol);
								},
							}
						}
						Some(Ok(Frame::Close(close))) => {
							let code = close.map_or(1006, |f| u16::from(f.code));
							match close_action(code) { Reconnect::Stop => return Err(if code == 4004 { Failure::Expired } else { Failure::Protocol }), Reconnect::Identify => state = ResumeState::default(), Reconnect::Resume => {} }
							break;
						}
						Some(Ok(Frame::Ping(data))) => { if !matches!(timeout(Duration::from_secs(5), socket.send(Frame::Pong(data))).await, Ok(Ok(()))) { break; } }
						Some(Ok(Frame::Pong(_))) => {},
						Some(Ok(Frame::Binary(_))) => return Err(Failure::Protocol),
						_ => break,
					}
				}
			}
		}
		attempt = next_attempt(attempt, ready_at.map(|ready| ready.elapsed()));
	}
	Err(Failure::Network)
}
fn notification_settings(
	entries: Vec<discord_protocol::notifications::Setting>,
	replace: bool,
) -> Event {
	Event::NotificationPreferences(notification_preferences(entries, replace))
}
fn notification_preferences(
	entries: Vec<discord_protocol::notifications::Setting>,
	replace: bool,
) -> client_core::notifications::Event {
	client_core::notifications::Event::Settings {
		entries: entries
			.into_iter()
			.map(|s| client_core::notifications::Setting {
				guild: s.guild_id,
				muted: s.channel_overrides.as_ref().and(s.muted),
				level: s.message_notifications,
				suppress_everyone: s.suppress_everyone,
				suppress_roles: s.suppress_roles,
				channel_mute_until: s.channel_overrides.as_ref().map_or_else(Vec::new, |c| {
					c.0.iter()
						.filter_map(|c| {
							(c.muted == Some(true))
								.then(|| {
									c.mute_config
										.as_ref()
										.and_then(|m| m.until())
										.map(|until| (c.channel_id, until))
								})
								.flatten()
						})
						.collect()
				}),
				channels: s
					.channel_overrides
					.map_or_else(Vec::new, |c| c.0)
					.into_iter()
					.map(|c| (c.channel_id, c.muted, c.message_notifications))
					.collect(),
			})
			.collect(),
		replace,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn notification_settings_forward_explicit_mention_suppression() {
		let setting = decode::<discord_protocol::notifications::Setting>(br#"{"guild_id":"1","muted":false,"message_notifications":1,"channel_overrides":[{"channel_id":"3","muted":true,"mute_config":{"end_time":"2020-01-01T00:00:00Z"}}],"suppress_everyone":false,"suppress_roles":true}"#).unwrap();
		let Event::NotificationPreferences(client_core::notifications::Event::Settings {
			entries,
			replace,
		}) = notification_settings(vec![setting], false)
		else {
			panic!()
		};
		assert!(!replace);
		assert_eq!(entries.len(), 1);
		assert_eq!(
			(entries[0].suppress_everyone, entries[0].suppress_roles),
			(Some(false), Some(true))
		);
		assert_eq!(entries[0].guild, Some(model::Id(1)));
		assert_eq!(
			entries[0].channel_mute_until,
			vec![(model::Id(3), 1577836800)]
		);
	}
	use serde_json::{Value, json};
	use tokio::net::{TcpListener, TcpStream};
	use tokio_tungstenite::{
		WebSocketStream, accept_async,
		tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
	};

	async fn packet(socket: &mut WebSocketStream<TcpStream>) -> Value {
		let frame = timeout(Duration::from_secs(5), socket.next())
			.await
			.unwrap()
			.unwrap()
			.unwrap();
		let Frame::Text(text) = frame else {
			panic!("expected synthetic JSON frame")
		};
		serde_json::from_str(&text).unwrap()
	}
	async fn send(socket: &mut WebSocketStream<TcpStream>, value: Value) {
		socket
			.send(Frame::Text(value.to_string().into()))
			.await
			.unwrap();
	}
	async fn acknowledge(socket: &mut WebSocketStream<TcpStream>, sequence: u64) {
		send(socket, json!({"op":1,"d":null})).await;
		// A timer heartbeat may already be queued before the preceding dispatch is read.
		for _ in 0..4 {
			let heartbeat = packet(socket).await;
			assert_eq!(heartbeat["op"], 1);
			send(socket, json!({"op":11,"d":null})).await;
			if heartbeat["d"] == sequence {
				return;
			}
		}
		panic!("dispatch sequence was not reflected in heartbeat");
	}
	fn ready(sequence: u64, session: &str) -> Value {
		json!({"op":0,"t":"READY","s":sequence,"d":{
			"user":{"id":"1","username":"synthetic"}, "session_id":session,
			"resume_gateway_url":"wss://gateway.discord.gg/", "guilds":[], "private_channels":[]
		}})
	}

	#[test]
	fn reaction_dispatch_preserves_burst_and_falls_back_for_unsafe_details() {
		use client_core::reactions::Event as ReactionEvent;
		let wire = json!({"channel_id":"2","message_id":"3","user_id":"4","emoji":{"id":"5","name":null},"type":1,"burst":true});
		for (name, adding) in [
			("MESSAGE_REACTION_ADD", true),
			("MESSAGE_REACTION_REMOVE", false),
		] {
			let event = reaction_event(name, &serde_json::to_vec(&wire).unwrap(), true).unwrap();
			assert!(matches!(event, ReactionEvent::Delta {
				channel: Id(2), message: Id(3), user: Id(4), emoji, add, burst: true,
			} if add == adding && emoji.id == Some(Id(5)) && emoji.name.is_none()));
		}
		for fields in [
			json!({"type":2}),
			json!({"type":0}),
			json!({"user_id":null}),
			json!({"emoji":{"id":null,"name":"x".repeat(129)}}),
		] {
			let mut value = wire.clone();
			value
				.as_object_mut()
				.unwrap()
				.extend(fields.as_object().unwrap().clone());
			assert!(matches!(
				reaction_event(
					"MESSAGE_REACTION_ADD",
					&serde_json::to_vec(&value).unwrap(),
					true
				)
				.unwrap(),
				ReactionEvent::Changed {
					channel: Id(2),
					message: Id(3)
				}
			));
		}
		for name in [
			"MESSAGE_REACTION_ADD",
			"MESSAGE_REACTION_REMOVE",
			"MESSAGE_REACTION_REMOVE_ALL",
			"MESSAGE_REACTION_REMOVE_EMOJI",
		] {
			assert!(matches!(
				reaction_event(name, &serde_json::to_vec(&wire).unwrap(), false).unwrap(),
				ReactionEvent::Changed {
					channel: Id(2),
					message: Id(3)
				}
			));
		}
		assert!(matches!(
			reaction_event(
				"MESSAGE_REACTION_REMOVE_EMOJI",
				br#"{"channel_id":"2","message_id":"3"}"#,
				true
			)
			.unwrap(),
			ReactionEvent::Changed {
				channel: Id(2),
				message: Id(3)
			}
		));
		assert!(matches!(
			reaction_event(
				"MESSAGE_REACTION_ADD",
				br#"{"channel_id":"0","message_id":"3"}"#,
				true
			),
			Err(Failure::Protocol)
		));
	}

	#[tokio::test]
	async fn outgoing_activity_waits_for_ready_coalesces_clears_and_resumes() {
		timeout(Duration::from_secs(25), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
            let game = |name: &str| discord_protocol::rpc::ActivityFields::default().into_activity(Id(42), name.into()).unwrap();
            let (activity, receiver) = watch::channel(Some(game("osu!")));
			let (own_presence, presence_receiver) = watch::channel(model::OwnPresence {
				status: model::PresenceStatus::DoNotDisturb, custom_status: "Synthetic focus".into(),
			});
            let (observations, mut observed) = watch::channel(ActivityObservation::Unconfirmed);
            let (finished, done) = tokio::sync::oneshot::channel();
            let server = async {
                let mut previous = None;
                for connection in 0..2 {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut socket = accept_async(stream).await.unwrap();
                    send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
                    let handshake = packet(&mut socket).await;
                    assert_eq!(handshake["op"], if connection == 0 { 2 } else { 6 });
                    if connection == 0 {
                        assert_eq!(handshake["d"]["presence"]["activities"], json!([]));
						assert_eq!(handshake["d"]["presence"]["status"], "dnd");
                    } else {
                        observed.wait_for(|value| *value == ActivityObservation::Unconfirmed).await.unwrap();
                    }
                    // Before READY/RESUMED only heartbeats are allowed.
                    let gate = Instant::now() + Duration::from_millis(100);
                    while let Ok(value) = tokio::time::timeout_at(gate, packet(&mut socket)).await {
                        assert_eq!(value["op"], 1);
                        send(&mut socket, json!({"op":11,"d":null})).await;
                    }
                    send(&mut socket, if connection == 0 {
                        ready(1, "synthetic-own-activity")
                    } else { json!({"op":0,"t":"RESUMED","s":2,"d":{}}) }).await;
                    let expected = if connection == 0 { vec![Some("osu!"), Some("Minecraft")] } else { vec![Some("Minecraft"), None] };
                    for name in expected {
                        let value = loop {
                            let value = packet(&mut socket).await;
                            if value["op"] == 1 {
                                send(&mut socket, json!({"op":11,"d":null})).await;
                            } else { break value; }
                        };
                        assert_eq!(value["op"], 3);
						let mut activities = name.map_or_else(Vec::new, |name| vec![json!({"name":name,"type":0,"application_id":"42"})]);
						activities.push(json!({"name":"Custom Status","type":4,"state":if name == Some("osu!") { "Synthetic focus" } else { "On a break" }}));
                        assert_eq!(value["d"], json!({"since":null,"status":"dnd","afk":false,"activities":activities}));
                        let now = Instant::now();
                        if let Some(previous) = previous {
                            assert!(now.duration_since(previous) >= Duration::from_millis(4900));
                        }
                        previous = Some(now);
                        match name {
                            Some("osu!") => {
                                for (public, hidden, expected) in [
                                    (true, false, ActivityObservation::ServerListed),
                                    (false, true, ActivityObservation::ServerHidden),
                                    (false, false, ActivityObservation::ServerMissing),
                                ] {
                                    let game = json!({"type":0,"application_id":"42"});
                                    send(&mut socket, json!({"op":0,"t":"SESSIONS_REPLACE","s":2,"d":[{
                                        "session_id":"all","status":"online",
                                        "activities":if public {json!([game])} else {json!([])},
                                        "hidden_activities":if hidden {json!([game])} else {json!([])}
                                    }]})).await;
                                    observed.wait_for(|value| *value == expected).await.unwrap();
                                }
                                activity.send(Some(game("Skipped intermediate"))).unwrap();
								own_presence.send_replace(model::OwnPresence {status:model::PresenceStatus::DoNotDisturb,custom_status:"On a break".into()});
                                activity.send(Some(game("Minecraft"))).unwrap();
                                observed.wait_for(|value| *value == ActivityObservation::Unconfirmed).await.unwrap();
                            }
                            Some(_) if connection == 1 => activity.send(None).unwrap(),
                            Some(_) => {
                                send(&mut socket, json!({"op":0,"t":"SESSIONS_REPLACE","s":3,"d":[{
                                    "session_id":"all","status":"online","activities":[{"type":0,"application_id":"42"}]
                                }]})).await;
                                observed.wait_for(|value| *value == ActivityObservation::ServerListed).await.unwrap();
                            }
                            None => {}
                        }
                    }
                    if connection == 0 {
                        send(&mut socket, json!({"op":7,"d":null})).await;
                    } else {
                        socket.send(Frame::Close(Some(CloseFrame {
                            code: CloseCode::from(4004), reason: "synthetic stop".into()
                        }))).await.unwrap();
                        done.await.unwrap();
                        break;
                    }
                }
            };
            let client = async {
                let result = run_inner(
                    Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),
                    "wss://gateway.discord.gg/".into(), watch::channel(None).1,
                    mpsc::channel(1).1, Some(ActivityInput { receiver, own_presence: presence_receiver, observe: &|value| { observations.send_replace(value); Ok(()) } }), |_| Ok(()), Some(&endpoint),
                ).await;
                finished.send(()).unwrap();
                result
            };
            let ((), result) = tokio::join!(server, client);
            assert_eq!(result, Err(Failure::Expired));
        }).await.expect("synthetic activity lifecycle timed out");
	}

	#[tokio::test]
	async fn legacy_ready_game_reaches_known_dm_without_a_presence_update() {
		timeout(Duration::from_secs(10), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
            let (delivered, observed) = tokio::sync::oneshot::channel();
            let delivered = std::sync::Mutex::new(Some(delivered));
            let server = async {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
                let identify = packet(&mut socket).await;
                assert_eq!(identify["op"], 2);
                assert!(identify["d"].get("capabilities").is_none());
                let mut initial = ready(1, "synthetic-legacy-presence");
                initial["d"]["private_channels"] = json!([{"id":"2","type":1,"recipients":[{"id":"3","username":"Synthetic player"}]}]);
                initial["d"]["presences"] = json!([{"user":{"id":"3"},"status":"online","activities":[{"type":0,"name":"Genshin Impact"}]}]);
                initial["d"]["relationships"] = json!([{"id":"4","type":1,"user":{"id":"4","username":"Synthetic friend"}}]);
                initial["d"]["presences"].as_array_mut().unwrap().push(json!({"user":{"id":"4"},"status":"idle","activities":[]}));
                send(&mut socket, initial).await;
                // No PRESENCE_UPDATE is sent: an unchanged running game must appear at startup.
                observed.await.unwrap();
            };
            let state = std::sync::Mutex::new(client_core::State::default());
            let client = run_inner(
                Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),
                "wss://gateway.discord.gg/".into(),
                watch::channel(None).1,
                mpsc::channel(1).1,
                None,
                |event| {
                    let presence = matches!(event, Event::DirectPresence(_));
                    let mut state = state.lock().unwrap();
                    let generation = state.generation;
                    state.apply(client_core::Envelope { generation, event });
                    if presence {
                        let activity = &state.presence_for(Id(3)).unwrap().activities[0];
                        assert_eq!(activity.summary(), "Playing Genshin Impact");
                        assert_eq!(state.presence_for(Id(4)).unwrap().status.as_deref(), Some("idle"));
                        delivered.lock().unwrap().take().unwrap().send(()).unwrap();
                        return Err(Failure::Expired); // Stop the synthetic connection after delivery.
                    }
                    Ok(())
                },
                Some(&endpoint),
            );
            let ((), result) = tokio::join!(server, client);
            assert_eq!(result, Err(Failure::Expired));
        }).await.expect("legacy READY presence was not delivered");
	}

	#[tokio::test]
	async fn local_ignored_dispatches_and_typing_keep_delivering_messages() {
		timeout(Duration::from_secs(10), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
            let (client_finished, terminal_observed) = tokio::sync::oneshot::channel();
            let server = async {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
                assert_eq!(packet(&mut socket).await["op"], 2);
                send(&mut socket, ready(1, "synthetic-typing-session")).await;
                for (sequence, name) in [(2, Some("SYNTHETIC_PRIVATE_EVENT_NAME")), (3, None), (4, Some(""))] {
                    send(&mut socket, json!({"op":0,"t":name,"s":sequence,"d":{
                        "token":"SYNTHETIC_PRIVATE_PAYLOAD", "permissions":"8", "content":"not a message"
                    }})).await;
                }
                for (sequence, data) in [
                    (5, json!({"channel_id":"2","user_id":"3","timestamp":1700000000,"member":{"user":{"username":"discarded"}}})),
                    (6, json!({"channel_id":"2","user_id":"3","timestamp":"invalid"})),
                    (7, json!({"channel_id":"2","user_id":"3","timestamp":1700000000,"member":{"padding":"x".repeat(discord_protocol::typing::MAX_WIRE)}})),
                ] {
                    send(&mut socket, json!({"op":0,"t":"TYPING_START","s":sequence,"d":data})).await;
                }
                send(&mut socket, json!({"op":0,"t":"MESSAGE_CREATE","s":8,"d":{"id":"4","channel_id":"2","author":{"id":"3","username":"Synthetic"},"content":"Message after invalid typing"}})).await;
                acknowledge(&mut socket, 8).await;
                // Exercise a heartbeat reply racing the terminal close.
                send(&mut socket, json!({"op":1,"d":null})).await;
                socket.send(Frame::Close(Some(CloseFrame {
                    code: CloseCode::from(4004), reason: "synthetic stop".into(),
                }))).await.unwrap();
                // Keep TCP alive: unread heartbeat bytes on drop can reset it and lose Close.
                terminal_observed.await.unwrap();
            };
            let observed = std::sync::Mutex::new(Vec::new());
            let client = run_inner(
                Arc::new(SessionSecret::from_owner_input("SYNTHETIC_TYPING_SESSION".into()).unwrap()),
                "wss://gateway.discord.gg/".into(), watch::channel(None).1,
                mpsc::channel(1).1,
                None,
                |event| {
                    match event {
                        Event::Typing(signal) => observed.lock().unwrap().push((signal.channel, signal.user, signal.timestamp)),
                        Event::Message(message) => {
                            assert_eq!(message.content, "Message after invalid typing");
                            observed.lock().unwrap().push((message.channel, message.author.id, 0));
                        }
                        _ => {}
                    }
                    Ok(())
                }, Some(&endpoint),
            );
            let client = async {
                let result = client.await;
                let _ = client_finished.send(());
                result
            };
            let ((), result) = tokio::join!(server, client);
            assert_eq!(result, Err(Failure::Expired));
            assert_eq!(observed.into_inner().unwrap(), vec![(Id(2), Id(3), 1700000000), (Id(2), Id(3), 0)]);
        }).await.unwrap();
	}

	#[tokio::test]
	async fn local_category_create_move_permission_update_and_delete() {
		timeout(Duration::from_secs(10),async {
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint=format!("ws://{}/",listener.local_addr().unwrap());
            let (client_finished, terminal_observed)=tokio::sync::oneshot::channel();
            let server=async {
                let (stream,_)=listener.accept().await.unwrap();
                let mut socket=accept_async(stream).await.unwrap();
                send(&mut socket,json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
                assert_eq!(packet(&mut socket).await["op"],2);
                let mut initial=ready(1,"synthetic-session");
                initial["d"]["read_state"]=json!([]);
                initial["d"]["guilds"]=json!([{"id":"2","name":"Synthetic guild","properties":{"owner_id":"9"},"roles":[{"id":"2","permissions":"68608"}],"threads":[{"id":"5","parent_id":"4","type":11,"name":"Initial thread"}]}]);
                initial["d"]["merged_members"]=json!([[{"user_id":"1","roles":[]}]]);
                send(&mut socket,initial).await;
                for (sequence,name,data) in [
                    (2,"CHANNEL_CREATE",json!({"id":"3","guild_id":"2","type":4,"name":"Synthetic category","position":0})),
                    (3,"CHANNEL_CREATE",json!({"id":"4","guild_id":"2","type":0,"name":"Synthetic channel","parent_id":"3","position":1})),
                    (4,"CHANNEL_UPDATE",json!({"id":"4","parent_id":null,"position":0})),
                    (5,"CHANNEL_UPDATE",json!({"id":"4","permission_overwrites":[]})),
                    (6,"CHANNEL_DELETE",json!({"id":"3","guild_id":"2","type":4})),
                    (7,"MESSAGE_REACTION_ADD",json!({"channel_id":"4","message_id":"9","user_id":"1","emoji":{"id":null,"name":"x"}})),
                    (7,"MESSAGE_REACTION_ADD",json!({"channel_id":"4","message_id":"9","user_id":"1","emoji":{"id":null,"name":"x"}})),
                    (8,"MESSAGE_REACTION_REMOVE",json!({"channel_id":"4","message_id":"9","user_id":"1","emoji":{"id":null,"name":"x"}})),
                    (9,"MESSAGE_REACTION_REMOVE_ALL",json!({"channel_id":"4","message_id":"9"})),
                    (10,"MESSAGE_REACTION_REMOVE_EMOJI",json!({"channel_id":"4","message_id":"9","emoji":{"id":null,"name":"x"}})),
                    (11,"MESSAGE_ACK",json!({"channel_id":"4","message_id":"8","version":2})),
                    (12,"PASSIVE_UPDATE_V2",json!({"updated_channels":[{"id":"4","last_message_id":"9"}]})),
                    (13,"THREAD_CREATE",json!({"id":"6","guild_id":"2","parent_id":"4","type":12,"name":"Private thread"})),
                    (14,"THREAD_UPDATE",json!({"id":"6","guild_id":"2","name":"Renamed thread"})),
                    (15,"THREAD_LIST_SYNC",json!({"guild_id":"2","channel_ids":["4"],"threads":[{"id":"6","parent_id":"4","type":12,"name":"Synced thread"}]})),
                    (16,"THREAD_MEMBERS_UPDATE",json!({"id":"6","guild_id":"2","removed_member_ids":["99"]})),
                    (17,"THREAD_MEMBERS_UPDATE",json!({"id":"6","guild_id":"2","removed_member_ids":["1"]})),
                    (18,"THREAD_CREATE",json!({"id":"7","guild_id":"2","parent_id":"4","type":11,"name":"Archive me"})),
                    (19,"THREAD_UPDATE",json!({"id":"7","guild_id":"2","thread_metadata":{"archived":true}})),
                    (20,"THREAD_CREATE",json!({"id":"8","guild_id":"2","parent_id":"4","type":10,"name":"Delete me"})),
                    (21,"THREAD_DELETE",json!({"id":"8","guild_id":"2","parent_id":"4","type":10})),
                    (22,"GUILD_CREATE",json!({"id":"2","emojis":[{"id":"20","name":"wave","roles":[],"available":true}]})),
                    (23,"GUILD_EMOJIS_UPDATE",json!({"guild_id":"2","emojis":[{"id":"21","name":"party","animated":true,"roles":[],"available":true}]})),
                    (24,"GUILD_ROLE_CREATE",json!({"guild_id":"2","role":{"id":"30","permissions":"32768"}})),
                    (25,"GUILD_ROLE_UPDATE",json!({"guild_id":"2","role":{"id":"30","permissions":"0"}})),
                    (26,"GUILD_MEMBER_UPDATE",json!({"guild_id":"2","user":{"id":"1"},"roles":["30"],"communication_disabled_until":null})),
                    (27,"GUILD_MEMBER_UPDATE",json!({"guild_id":"2","user":{"id":"8"},"roles":["30"]})),
                    (28,"GUILD_ROLE_DELETE",json!({"guild_id":"2","role_id":"30"})),
                    (29,"GUILD_UPDATE",json!({"id":"2","owner_id":"7"})),
                    (30,"PASSIVE_UPDATE_V2",json!({"guild_id":"2","updated_members":[{"user":{"id":"1","username":"Synthetic"},"roles":[],"communication_disabled_until":null}]})),
                    (31,"READY_SUPPLEMENTAL",json!({"guilds":[{"id":"2"}],"merged_members":[[{"user_id":"1","roles":[]}]]})),
                    (32,"GUILD_DELETE",json!({"id":"2"})),
                    (33,"GUILD_CREATE",json!({"id":"2","owner_id":"7","roles":[{"id":"2","permissions":"68608"}],"members":[{"user":{"id":"1","username":"Synthetic"},"roles":[]}],"channels":[{"id":"4","type":0,"name":"Synthetic channel","position":0,"parent_id":null,"last_message_id":"9","permission_overwrites":[]}]})),
                ] {send(&mut socket,json!({"op":0,"t":name,"s":sequence,"d":data})).await;}
                // A stale replay must neither change counts nor regress the heartbeat cursor.
                send(&mut socket,json!({"op":0,"t":"MESSAGE_REACTION_ADD","s":7,"d":{"channel_id":"4","message_id":"9","user_id":"1","emoji":{"id":null,"name":"x"}}})).await;
                acknowledge(&mut socket,33).await;
                // Force a heartbeat reply to race the following terminal close.
                send(&mut socket,json!({"op":1,"d":null})).await;
                socket.send(Frame::Close(Some(CloseFrame {code:CloseCode::from(4004),reason:"synthetic expiration".into()}))).await.unwrap();
                // Do not drop TCP with an unread timer heartbeat: that can reset the socket
                // and discard the close frame, correctly making the client try to resume.
                terminal_observed.await.unwrap();
            };
            let state=std::sync::Mutex::new(client_core::State::default());
            let permission_changes=std::sync::atomic::AtomicUsize::new(0);
            let reaction_changes=std::sync::atomic::AtomicUsize::new(0);
            let thread_events=std::sync::atomic::AtomicUsize::new(0);
            let emoji_changes=std::sync::atomic::AtomicUsize::new(0);
            let client=run_inner(
                Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),
                "wss://gateway.discord.gg/".into(),watch::channel(None).1,mpsc::channel(1).1,None,
                |event| {
					if let Event::Startup(startup) = &event {
						let (channels, permissions) = (&startup.channels, startup.permission_state());
                        assert!(channels.iter().any(|c| c.id == Id(5) && c.guild == Some(Id(2))));
                        assert_eq!(permissions.guilds[&Id(2)].owner,Some(Id(9)));
                        assert_eq!(permissions.guilds[&Id(2)].member.as_ref().unwrap().roles,Vec::<Id>::new());
						let client_core::read_state::Event::Snapshot {entries,version,partial} = &startup.read_state else { panic!("startup read snapshot missing") };
                        assert!(entries.as_ref().is_some_and(Vec::is_empty));
                        assert_eq!(*version,None);
                        assert!(!partial);
                    }
                    if matches!(event,Event::Permissions(_)) {permission_changes.fetch_add(1,std::sync::atomic::Ordering::Relaxed);}
                    if let Event::Permissions(client_core::permissions::Event::Channel {channel,guild,overwrites})=&event {
                        assert_eq!((*channel,*guild),(Id(4),None));
                        assert_eq!(*overwrites,model::Patch::Value(Vec::new()));
                    }
                    if let Event::Reactions(reaction)=&event {
                        let change=reaction_changes.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                        match reaction {
                            client_core::reactions::Event::Delta{channel,message,user,emoji,add,burst} => {
                                assert!(change < 2);
                                assert_eq!((*channel,*message,*user),(Id(4),Id(9),Id(1)));
                                assert_eq!(*add,change==0);
                                assert!(!burst);
                                assert_eq!(emoji.name.as_deref(),Some("x"));
                                assert_eq!(emoji.id,None);
                            }
                            client_core::reactions::Event::Cleared{channel,message,emoji} => {
                                assert!((2..4).contains(&change));
                                assert_eq!((*channel,*message),(Id(4),Id(9)));
                                assert_eq!(emoji.as_ref().and_then(|emoji|emoji.name.as_deref()),if change==2 {None}else{Some("x")});
                                assert!(emoji.as_ref().is_none_or(|emoji|emoji.id.is_none()));
                            }
                            _ => panic!("valid sequenced reactions must keep their typed change"),
                        }
                    }
                    if let Event::GuildEmojis {guild,emojis}=&event {
                        assert_eq!(*guild,Id(2));
                        let change=emoji_changes.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
                        match change {
                            0 => assert_eq!(emojis[0].markup(),"<:wave:20>"),
                            1 => assert_eq!(emojis[0].markup(),"<a:party:21>"),
                            2 => assert!(emojis.is_empty()),
                            _ => panic!("unexpected emoji update"),
                        }
                    }
                    let mut state=state.lock().unwrap();
                    let thread_change=matches!(&event, Event::ThreadChanged{..}|Event::ThreadsSync{..}|Event::ThreadRemoved{..})
                        || matches!(&event,Event::ChannelCreated(c) if matches!(c.kind,10..=12));
                    let sync=matches!(&event,Event::ThreadsSync{..});
                    let unavailable=matches!(&event,Event::Permissions(client_core::permissions::Event::UnavailableGuild(Id(2))));
                    let generation=state.generation;
                    state.apply(client_core::Envelope {generation,event});
                    if unavailable {
                        assert!(!state.can_view(Id(4)));
                        assert_eq!(state.read_marker(Id(4)),None);
                    }
                    if thread_change {thread_events.fetch_add(1,std::sync::atomic::Ordering::Relaxed);}
                    if sync {
                        assert!(!state.channels.iter().any(|c|c.id==Id(5)));
                        assert_eq!(state.channels.iter().find(|c|c.id==Id(6)).unwrap().name,"Synced thread");
                    }
                    Ok(())
                },Some(&endpoint)
            );
            let client=async {
                let result=client.await;
                let _=client_finished.send(());
                result
            };
            let (result,())=tokio::join!(client,server);
            assert_eq!(result,Err(Failure::Expired));
            let state=state.into_inner().unwrap();
            assert_eq!(state.channels.len(),1);
            assert_eq!(state.channels[0].id,Id(4));
            assert_eq!(state.channels[0].parent_id,None);
            assert_eq!(state.channels[0].position,0);
            assert_eq!(state.channels[0].name,"Synthetic channel");
            assert_eq!(state.read_marker(Id(4)),Some(Some(Id(8))));
            assert_eq!(state.unread(Id(4)),Some(true));
            assert_eq!(permission_changes.load(std::sync::atomic::Ordering::Relaxed),11);
            assert_eq!(emoji_changes.load(std::sync::atomic::Ordering::Relaxed),3);
            assert!(state.guilds[0].emojis.as_ref().unwrap().is_empty());
            assert_eq!(reaction_changes.load(std::sync::atomic::Ordering::Relaxed),4);
            assert_eq!(thread_events.load(std::sync::atomic::Ordering::Relaxed),8);
        }).await.unwrap();
	}

	#[tokio::test]
	async fn local_socket_identify_ack_drop_resume_and_invalid_session() {
		timeout(Duration::from_secs(45), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let (client_finished, mut terminal_observed) = tokio::sync::oneshot::channel();
			let (invalid_session_consumed, mut invalid_session_observed) =
				tokio::sync::oneshot::channel();
			let invalid_session_consumed = std::sync::Mutex::new(Some(invalid_session_consumed));
			let (presence_consumed, mut presence_observed) = tokio::sync::oneshot::channel();
			let presence_consumed = std::sync::Mutex::new(Some(presence_consumed));
			let server = async {
				for connection in 0..3 {
					let (stream, _) = listener.accept().await.unwrap();
					let mut socket = accept_async(stream).await.unwrap();
					send(
						&mut socket,
						json!({"op":10,"d":{"heartbeat_interval":1000}}),
					)
					.await;
					let handshake = packet(&mut socket).await;
					assert_eq!(handshake["d"]["token"], "synthetic-owner-session");
					if connection == 1 {
						assert_eq!(handshake["op"], 6);
						assert_eq!(handshake["d"]["seq"], 41);
						assert_eq!(handshake["d"]["session_id"], "synthetic-first-session");
						send(
							&mut socket,
							json!({"op":0,"t":"PRESENCE_UPDATE","s":42,"d":{"user":{"id":"3"},"status":"idle"}}),
						)
						.await;
						// Replayed presence may precede RESUMED by longer than its batch deadline.
						sleep(Duration::from_millis(150)).await;
						send(&mut socket, json!({"op":0,"t":"RESUMED","s":43,"d":{}})).await;
						(&mut presence_observed).await.unwrap();
						acknowledge(&mut socket, 43).await;
						// Leave a heartbeat reply unread to exercise the TCP-reset race.
						send(&mut socket, json!({"op":1,"d":null})).await;
						send(&mut socket, json!({"op":9,"d":false})).await;
						// Keep TCP alive until the client processes invalid-session;
						// dropping it now can discard that frame with the unread reply.
						(&mut invalid_session_observed).await.unwrap();
					} else {
						assert_eq!(handshake["op"], 2);
						assert!(handshake["d"].get("session_id").is_none());
						assert_eq!(
							handshake["d"]["properties"]["browser"],
							client_core::fingerprint::browser()
						);
						assert_eq!(
							handshake["d"]["properties"]["browser_user_agent"],
							client_core::fingerprint::user_agent()
						);
						if connection == 0 {
							send(&mut socket, ready(41, "synthetic-first-session")).await;
							acknowledge(&mut socket, 41).await;
							// Drop TCP without a close frame: the next connection must Resume.
						} else {
							send(&mut socket, ready(1, "synthetic-new-session")).await;
							socket
								.send(Frame::Close(Some(CloseFrame {
									code: CloseCode::from(4004),
									reason: "synthetic expiration".into(),
								})))
								.await
								.unwrap();
							// An unread timer heartbeat can make dropping TCP reset the
							// socket and discard this close. Wait until the client has
							// consumed the terminal result, without adding a grace sleep.
							(&mut terminal_observed).await.unwrap();
						}
					}
				}
			};
			let events = std::sync::Mutex::new(Vec::new());
			let secret = Arc::new(
				SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap(),
			);
			assert_eq!(
				run(
					secret.clone(),
					endpoint.clone(),
					watch::channel(None).1,
					|_| Ok(())
				)
				.await,
				Err(Failure::Protocol)
			);
			let client = run_inner(
				secret,
				"wss://gateway.discord.gg/".into(),
				watch::channel(None).1,
				mpsc::channel(1).1,
				None,
				|event| {
					let label = match event {
						Event::Startup(_) => "ready",
						Event::Resumed => "resumed",
						Event::DirectPresence(_) => "presence",
						Event::Resync => "resync",
						Event::Disconnected => "disconnected",
						Event::ReadState(client_core::read_state::Event::Snapshot { .. }) => {
							return Ok(());
						}
						Event::UserAction(
							client_core::user_actions::Event::Relationships(None)
							| client_core::user_actions::Event::Requests(None)
							| client_core::user_actions::Event::Friends(None),
						) => return Ok(()),
						_ => return Err(Failure::Protocol),
					};
					let mut events = events.lock().unwrap();
					assert!(events.len() < 16);
					if label == "presence" {
						assert!(
							events.contains(&"resumed"),
							"Replay presence must wait until the reducer is connected"
						);
						presence_consumed
							.lock()
							.unwrap()
							.take()
							.unwrap()
							.send(())
							.unwrap();
					}
					// Emission is synchronous: the first connection's Disconnected
					// precedes Resumed, so it cannot release the second socket.
					if label == "disconnected"
						&& events.contains(&"resumed")
						&& let Some(consumed) = invalid_session_consumed.lock().unwrap().take()
					{
						consumed.send(()).unwrap();
					}
					events.push(label);
					Ok(())
				},
				Some(&endpoint),
			);
			let client = async {
				let result = client.await;
				let _ = client_finished.send(());
				result
			};
			let (result, ()) = tokio::join!(client, server);
			assert_eq!(result, Err(Failure::Expired));
			let events = events.into_inner().unwrap();
			assert!(
				events
					.iter()
					.filter(|event| **event == "disconnected")
					.count() >= 2
			);
			assert_eq!(
				events
					.into_iter()
					.filter(|event| *event != "disconnected")
					.collect::<Vec<_>>(),
				["ready", "resumed", "presence", "resync", "ready"]
			);
		})
		.await
		.expect("local lifecycle exceeded its bounded deadline");
	}

	#[tokio::test]
	async fn unjoined_dm_call_discovery_and_lifecycle_over_local_gateway() {
		use client_core::voice::{Command as V, Event as E};
		timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let (controls, receive) = mpsc::channel(8);
			let (deleted, mut deletion) = watch::channel(false);
			let observed = std::sync::Mutex::new(Vec::new());
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
				assert_eq!(packet(&mut socket).await["op"], 2);
				let mut snapshot = ready(1, "synthetic-main-session");
				snapshot["d"]["private_channels"] = json!([{"id":"2","type":1,"recipients":[{"id":"3","username":"Peer"}]}]);
				send(&mut socket, snapshot).await;
				loop {
					let packet = packet(&mut socket).await;
					if packet["op"] == 1 {
						send(&mut socket, json!({"op":11,"d":null})).await;
						continue;
					}
					assert_eq!(packet, json!({"op":13,"d":{"channel_id":"2"}}));
					break;
				}
				send(&mut socket, json!({"op":0,"t":"CALL_CREATE","s":2,"d":{"channel_id":"2","ringing":[],"voice_states":[{"channel_id":"2","user_id":"3","session_id":"synthetic-passive-session"}]}})).await;
				send(&mut socket, json!({"op":0,"t":"CALL_UPDATE","s":3,"d":{"channel_id":"2","ringing":[]}})).await;
				send(&mut socket, json!({"op":0,"t":"VOICE_STATE_UPDATE","s":4,"d":{"channel_id":"2","user_id":"3","session_id":"synthetic-passive-session","self_mute":true}})).await;
				send(&mut socket, json!({"op":0,"t":"CALL_DELETE","s":5,"d":{"channel_id":"2"}})).await;
				loop {
					tokio::select! {
						result = deletion.changed() => { result.unwrap(); break; }
						packet = packet(&mut socket) => {
							assert_eq!(packet["op"], 1, "discovery must not join or send other controls");
							send(&mut socket, json!({"op":11,"d":null})).await;
						}
					}
				}
				socket.close(Some(CloseFrame { code: CloseCode::Library(4004), reason: "synthetic stop".into() })).await.unwrap();
			};
			let client = run_inner(
				Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),
				"wss://gateway.discord.gg/".into(), watch::channel(None).1, receive, None,
				|event| {
					match event {
						Event::Startup(_) => controls.try_send(V::Sync { channel: Id(2) }).unwrap(),
						Event::Voice(E::Call { channel, ringing, participants, unavailable }) => {
							assert_eq!(channel, Id(2));
							assert_eq!(ringing, Some(vec![]));
							assert!(!unavailable);
							observed.lock().unwrap().push(if let Some(rows) = participants { assert_eq!(rows[0].user, Id(3)); "create" } else { "update" });
						}
						Event::Voice(E::State { request, session, .. }) => { assert_eq!(request, None); assert!(session.is_none()); observed.lock().unwrap().push("state"); }
						Event::Voice(E::Deleted { channel }) => { assert_eq!(channel, Id(2)); observed.lock().unwrap().push("delete"); deleted.send(true).unwrap(); }
						_ => {}
					}
					Ok(())
				}, Some(&endpoint),
			);
			let ((), result) = tokio::join!(server, client);
			assert_eq!(result, Err(Failure::Expired));
			assert_eq!(*observed.lock().unwrap(), ["create", "update", "state", "delete"]);
		}).await.unwrap();
	}
	#[tokio::test]
	async fn explicit_dm_join_negotiation_and_leave_over_local_gateway() {
		use client_core::voice::{Command as V, Event as E};
		timeout(Duration::from_secs(10),async {
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let endpoint=format!("ws://{}/",listener.local_addr().unwrap());
            let (controls,receive)=mpsc::channel(8);
            let server=async {
                let (stream,_)=listener.accept().await.unwrap();let mut socket=accept_async(stream).await.unwrap();
                send(&mut socket,json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
                assert_eq!(packet(&mut socket).await["op"],2);
                let mut ready=ready(1,"synthetic-main-session");ready["d"]["private_channels"]=json!([{"id":"2","type":1,"recipients":[{"id":"3","username":"Peer"}]}]);
                send(&mut socket,ready).await;
                let mut requested=false;let mut joined=false;
                loop {
                    let p=packet(&mut socket).await;
                    if p["op"]==1 {send(&mut socket,json!({"op":11,"d":null})).await;continue;}
                    if p["op"]==13 {assert_eq!(p["d"]["channel_id"],"2");requested=true;continue;}
                    assert_eq!(p["op"],4);assert!(p["d"]["guild_id"].is_null());
                    if !joined {
                        assert!(requested);assert_eq!(p["d"]["channel_id"],"2");joined=true;
                        send(&mut socket,json!({"op":0,"t":"VOICE_STATE_UPDATE","s":2,"d":{"user_id":"1","channel_id":"2","session_id":"synthetic-call-session","self_mute":false,"self_deaf":false}})).await;
                        send(&mut socket,json!({"op":0,"t":"VOICE_SERVER_UPDATE","s":3,"d":{"guild_id":null,"channel_id":"2","token":"synthetic-call-token","endpoint":"voice.discord.media:443"}})).await;
                    } else {assert!(p["d"]["channel_id"].is_null());break;}
                }
                socket.close(Some(CloseFrame{code:CloseCode::Library(4004),reason:"synthetic stop".into()})).await.unwrap();
            };
            let client=run_inner(Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),"wss://gateway.discord.gg/".into(),watch::channel(None).1,receive,None,|event| {
                match event {
                    Event::Startup(_)=>controls.try_send(V::Join{channel:Id(2),request:7,ring:false}).unwrap(),
                    Event::Voice(E::State{request,session,..})=>{assert_eq!(request,Some(7));assert_eq!(session.unwrap().expose(),"synthetic-call-session");},
                    Event::Voice(E::Server{request,token,..})=>{assert_eq!(request,7);assert_eq!(token.unwrap().expose(),"synthetic-call-token");controls.try_send(V::Leave{channel:Id(2),request}).unwrap();},
                    _=>{},
                }
                Ok(())
            },Some(&endpoint));
            let ((),result)=tokio::join!(server,client);assert_eq!(result,Err(Failure::Expired));
        }).await.unwrap();
	}
	#[test]
	fn heartbeat_resume_and_origin_boundaries() {
		let mut heartbeat = Heartbeat::default();
		let now = Instant::now();
		let interval = Duration::from_secs(10);
		heartbeat.sent(now);
		// An unsolicited heartbeat immediately before the timer is not a missed ACK.
		assert!(
			heartbeat
				.tick(now + Duration::from_millis(1), interval)
				.is_ok()
		);
		assert!(heartbeat.tick(now + interval, interval).is_err());
		heartbeat.ack();
		assert!(heartbeat.tick(now + interval, interval).is_ok());
		assert_eq!(next_attempt(5, Some(Duration::from_secs(1))), 6);
		assert_eq!(next_attempt(5, Some(Duration::from_secs(60))), 1);
		assert_eq!(next_attempt(5, None), 6);
		assert_eq!(close_action(4004), Reconnect::Stop);
		assert_eq!(close_action(4007), Reconnect::Identify);
		assert_eq!(close_action(1006), Reconnect::Resume);
		for url in [
			"ws://gateway.discord.gg/",
			"wss://gateway.discord.gg.evil.test/",
			"wss://user@gateway.discord.gg/",
			"wss://127.0.0.1/",
			"wss://gateway.discord.gg:444/",
			"wss://gateway.discord.gg/path",
		] {
			assert!(validated_url(url).is_err());
		}
		assert_eq!(
			validated_url("wss://gateway.discord.gg/?compress=zlib-stream").unwrap(),
			"wss://gateway.discord.gg/?v=10&encoding=json&compress=zlib-stream"
		);
	}
}

#[cfg(test)]
mod member_tests {
	use super::*;
	use serde_json::json;
	#[test]
	fn rich_activity_patches_preserve_absence_and_clear_on_status_loss() {
		let mut list = ActiveMembers::new(MemberSubscription {
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
		});
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Synthetic"},"presence":{"status":"online","activities":[{"type":0,"name":"Synthetic","details":"In a match"},{"type":4,"state":"Custom"}]}}}]}]}"#).unwrap()).unwrap();
		let now = Instant::now();
		for (patch, active) in [
			(json!({"status":"idle"}), true),
			(json!({"status":null}), false),
			(json!({"status":"online"}), false),
			(json!({"activities":[{"type":0,"name":"Updated"}]}), true),
			(json!({"activities":[]}), false),
		] {
			let mut wire = patch;
			wire["guild_id"] = json!("1");
			wire["user"] = json!({"id":"3"});
			list.presence(
				discord_protocol::presence::decode(&serde_json::to_vec(&wire).unwrap()).unwrap(),
				now,
			);
			assert_eq!(
				!list.rows[0].as_ref().unwrap().activities.is_empty(),
				active
			);
		}
		let event = list.take_presence().unwrap();
		assert!(event.bytes() <= client_core::MAX_MEMBER_PRESENCE_BYTES);
	}
	fn record(user: u64, status: Option<&str>, custom: Option<&str>) -> model::MemberPresence {
		model::MemberPresence {
			user: Id(user),
			status: status.map(str::to_owned),
			custom_status: custom.map(str::to_owned),
			activities: vec![],
		}
	}
	#[test]
	fn diagnostics_are_opt_in_byte_bounded_redacted_and_tolerate_closed_output() {
		let mut output = Vec::new();
		Diagnostics::new("gateway", false).record_to("disabled", &mut output);
		assert!(output.is_empty());
		let label = ignored_dispatch_label(Some("SYNTHETIC_PRIVATE_EVENT_NAME"));
		assert_eq!(label, "unsupported dispatch ignored");
		assert_eq!(
			ignored_dispatch_label(None),
			ignored_dispatch_label(Some(""))
		);
		let mut enabled = Diagnostics::new("gateway", true);
		for _ in 0..1000 {
			enabled.record_to(label, &mut output);
		}
		let line = "[Serein gateway] unsupported dispatch ignored\n";
		assert_eq!(output, line.repeat(64).as_bytes());
		assert_eq!(enabled.remaining, 0);
		assert_eq!(enabled.bytes, 8 * 1024 - output.len());
		assert!(!String::from_utf8_lossy(&output).contains("SYNTHETIC_PRIVATE"));

		// Byte accounting includes UTF-8 and formatting, independently of the line cap.
		output.clear();
		let mut short = Diagnostics::new("members", true);
		short.bytes = 20;
		short.record_to("\u{e9}", &mut output);
		short.record_to("another line", &mut output);
		assert_eq!(output, "[Serein members] \u{e9}\n".as_bytes());
		assert_eq!((short.remaining, short.bytes), (63, 0));
		static OVERSIZED: [u8; 8192] = [b'x'; 8192];
		let mut oversized = Diagnostics::new("gateway", true);
		oversized.record_to(std::str::from_utf8(&OVERSIZED).unwrap(), &mut output);
		assert_eq!(oversized.remaining, 64);
		assert_eq!(output.len(), 20);

		struct Closed;
		impl std::io::Write for Closed {
			fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
				Err(std::io::ErrorKind::BrokenPipe.into())
			}
			fn flush(&mut self) -> std::io::Result<()> {
				Ok(())
			}
		}
		let mut closed = Diagnostics::new("gateway", true);
		for _ in 0..1000 {
			closed.record_to(label, &mut Closed);
		}
		assert_eq!(
			(closed.remaining, closed.bytes),
			(enabled.remaining, enabled.bytes)
		);
	}
	#[test]
	fn custom_status_patches_preserve_replace_clear_and_coalesce() {
		let mut list = ActiveMembers::new(MemberSubscription {
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
		});
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Synthetic"},"presence":{"status":"online","activities":[{"type":4,"state":"Old custom status"}]}}}]}]}"#).unwrap()).unwrap();
		let now = Instant::now();
		for (patch, status, custom) in [
			(
				json!({"status":"online"}),
				"online",
				Some("Old custom status"),
			),
			(json!({"status":"idle"}), "idle", Some("Old custom status")),
			(
				json!({"activities":[{"type":4,"state":"New custom status"}]}),
				"idle",
				Some("New custom status"),
			),
			(json!({"status":"dnd"}), "dnd", Some("New custom status")),
			(json!({"activities":[]}), "dnd", None),
			(
				json!({"activities":[{"type":4,"state":"Again"}]}),
				"dnd",
				Some("Again"),
			),
			(json!({"activities":null}), "dnd", None),
		] {
			let mut wire = patch;
			wire["guild_id"] = json!("1");
			wire["user"] = json!({"id":"3"});
			list.presence(
				discord_protocol::presence::decode(&serde_json::to_vec(&wire).unwrap()).unwrap(),
				now,
			);
			let row = list.rows[0].as_ref().unwrap();
			assert_eq!(row.status.as_deref(), Some(status));
			assert_eq!(row.custom_status.as_deref(), custom);
		}
		assert_eq!(
			list.presence_deadline,
			Some(now + Duration::from_millis(100))
		);
		let Event::MemberPresence { updates, .. } = list.take_presence().unwrap() else {
			panic!("presence");
		};
		assert_eq!(updates, vec![record(3, Some("dnd"), None)]);
	}
	#[test]
	fn member_sync_and_updates_replace_role_membership() {
		let mut list = ActiveMembers::new(MemberSubscription {
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
		});
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Synthetic"},"roles":["12","11"]}}]}]}"#).unwrap()).unwrap();
		assert_eq!(list.rows[0].as_ref().unwrap().roles, vec![Id(11), Id(12)]);
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"UPDATE","index":0,"item":{"member":{"user":{"id":"3","username":"Synthetic"},"roles":["13"]}}}]}"#).unwrap()).unwrap();
		assert_eq!(
			list.snapshot(Freshness::Fresh).rows[0]
				.as_ref()
				.unwrap()
				.roles,
			vec![Id(13)]
		);
	}

	#[test]
	fn presence_coalesces_loaded_rows_at_a_fixed_deadline_and_snapshots_supersede_it() {
		let mut list = ActiveMembers::new(MemberSubscription {
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
		});
		let now = Instant::now();
		let presence = |guild, user, status| discord_protocol::presence::PresenceUpdate {
			guild,
			user,
			status,
			custom_status: model::Patch::Absent,
			activities: model::Patch::Absent,
		};
		list.presence(
			presence(Some(Id(1)), Id(3), model::Patch::Value("online".into())),
			now,
		);
		assert!(
			list.presence_deadline.is_none(),
			"Unsynced lists never accept presence"
		);
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"First"},"presence":{"status":"online"}}},{"member":{"user":{"id":"4","username":"Second"},"presence":{"status":"offline"}}}]}]}"#).unwrap()).unwrap();
		for (guild, user, status) in [
			(None, Id(3), model::Patch::Value("idle".into())),
			(Some(Id(9)), Id(3), model::Patch::Value("idle".into())),
			(Some(Id(1)), Id(99), model::Patch::Value("idle".into())),
			(Some(Id(1)), Id(3), model::Patch::Absent),
			(Some(Id(1)), Id(3), model::Patch::Value("online".into())),
		] {
			list.presence(presence(guild, user, status), now);
		}
		assert!(list.pending_presence.is_empty() && list.presence_deadline.is_none());
		list.presence(
			presence(Some(Id(1)), Id(3), model::Patch::Value("idle".into())),
			now,
		);
		list.presence(
			presence(Some(Id(1)), Id(3), model::Patch::Value("dnd".into())),
			now + Duration::from_millis(90),
		);
		list.presence(
			presence(Some(Id(1)), Id(4), model::Patch::Null),
			now + Duration::from_millis(95),
		);
		assert_eq!(
			list.presence_deadline,
			Some(now + Duration::from_millis(100))
		);
		assert_eq!(list.pending_presence.len(), 2);
		assert_eq!(
			list.rows[0].as_ref().unwrap().status.as_deref(),
			Some("dnd")
		);
		let Event::MemberPresence {
			guild,
			channel,
			request,
			updates,
		} = list.take_presence().unwrap()
		else {
			panic!("compact presence event");
		};
		assert_eq!((guild, channel, request), (Id(1), Id(2), 7));
		assert_eq!(
			updates,
			vec![record(3, Some("dnd"), None), record(4, None, None)]
		);
		assert!(list.take_presence().is_none() && list.presence_deadline.is_none());
		list.presence(
			presence(Some(Id(1)), Id(3), model::Patch::Value("idle".into())),
			now,
		);
		assert!(
			!list
				.update(
					decode(br#"{"guild_id":"9","id":"everyone","member_count":0,"ops":[]}"#)
						.unwrap()
				)
				.unwrap()
		);
		assert!(
			list.presence_deadline.is_some(),
			"Another guild cannot supersede this batch"
		);
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"UPDATE","index":0,"item":{"member":{"user":{"id":"3","username":"First"},"presence":{"status":"offline"}}}}]}"#).unwrap()).unwrap();
		assert!(list.take_presence().is_none());
		assert_eq!(
			list.snapshot(Freshness::Fresh).rows[0]
				.as_ref()
				.unwrap()
				.status
				.as_deref(),
			Some("offline")
		);
		list.presence(
			presence(Some(Id(1)), Id(3), model::Patch::Value("idle".into())),
			now,
		);
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"INVALIDATE","range":[0,99]}]}"#).unwrap()).unwrap();
		assert!(!list.synced && list.take_presence().is_none() && list.presence_deadline.is_none());
	}
	#[test]
	fn presence_flood_retains_only_the_hundred_loaded_users() {
		let mut list = ActiveMembers::new(MemberSubscription {
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
		});
		let items: Vec<_> = (10..110)
			.map(|id| json!({"member":{"user":{"id":id.to_string(),"username":"Synthetic"}}}))
			.collect();
		list.update(decode(&serde_json::to_vec(&json!({"guild_id":"1","id":"everyone","member_count":100,"ops":[{"op":"SYNC","range":[0,99],"items":items}]})).unwrap()).unwrap()).unwrap();
		let now = Instant::now();
		for id in 10..1010 {
			list.presence(
				discord_protocol::presence::PresenceUpdate {
					guild: Some(Id(1)),
					user: Id(id),
					status: model::Patch::Value("online".into()),
					custom_status: model::Patch::Value("\u{1f680}".repeat(128)),
					activities: model::Patch::Absent,
				},
				now,
			);
		}
		assert_eq!(list.pending_presence.len(), 100);
		assert_eq!(list.rows.len(), 100);
		let event = list.take_presence().unwrap();
		assert!(event.bytes() <= client_core::MAX_MEMBER_PRESENCE_BYTES);
		let Event::MemberPresence { updates, .. } = event else {
			panic!("presence");
		};
		assert_eq!(updates.len(), 100);
		assert!(
			updates
				.iter()
				.all(|update| (10..110).contains(&update.user.0)
					&& update.status.as_deref() == Some("online")
					&& update.custom_status.as_deref() == Some("\u{1f680}".repeat(128).as_str()))
		);
		assert!(list.presence_deadline.is_none());
	}
	#[test]
	fn presence_preserves_the_existing_loaded_row_byte_limit() {
		let mut list = ActiveMembers::new(MemberSubscription {
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
		});
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Synthetic"}}}]}]}"#).unwrap()).unwrap();
		// Fill the synthetic mirror to three bytes below its admitted budget.
		let row = list.rows[0].as_mut().unwrap();
		row.nick = Some("n".repeat(128 * 1024 - row.bytes() - 3));
		assert_eq!(row.bytes(), 128 * 1024 - 3);
		let now = Instant::now();
		let update = |status: &str| discord_protocol::presence::PresenceUpdate {
			guild: Some(Id(1)),
			user: Id(3),
			status: model::Patch::Value(status.into()),
			custom_status: model::Patch::Absent,
			activities: model::Patch::Absent,
		};
		list.presence(update("idle"), now);
		assert!(list.rows[0].as_ref().unwrap().status.is_none());
		assert!(list.pending_presence.is_empty() && list.presence_deadline.is_none());
		list.presence(update("dnd"), now);
		assert_eq!(list.rows[0].as_ref().unwrap().bytes(), 128 * 1024);
		list.presence(update("offline"), now + Duration::from_millis(50));
		assert_eq!(
			list.rows[0].as_ref().unwrap().status.as_deref(),
			Some("dnd")
		);
		assert_eq!(
			list.pending_presence.get(&Id(3)),
			Some(&record(3, Some("dnd"), None))
		);
		assert_eq!(
			list.presence_deadline,
			Some(now + Duration::from_millis(100))
		);
		list.presence(
			discord_protocol::presence::PresenceUpdate {
				guild: Some(Id(1)),
				user: Id(3),
				status: model::Patch::Null,
				custom_status: model::Patch::Absent,
				activities: model::Patch::Absent,
			},
			now,
		);
		assert_eq!(list.rows[0].as_ref().unwrap().bytes(), 128 * 1024 - 3);
	}
	#[test]
	fn member_sync_accepts_group_headers_without_summary_counts() {
		let mut list = ActiveMembers::new(MemberSubscription {
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
		});
		let update = decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"groups":[{"id":"online","count":1}],"ops":[{"op":"SYNC","range":[0,99],"items":[{"group":{"id":"online"}},{"member":{"user":{"id":"3","username":"Synthetic"}}}]}]}"#).unwrap();
		assert!(list.update(update).unwrap());
		assert!(list.synced);
		assert!(list.rows[0].is_none());
		assert_eq!(list.rows[1].as_ref().unwrap().user.id, Id(3));
		assert_eq!(list.snapshot(Freshness::Fresh).total, 1);
	}
	#[test]
	fn member_operations_preserve_indices_scope_and_bounds() {
		let mut list = ActiveMembers::new(MemberSubscription {
			guild: Id(1),
			channel: Id(2),
			request: 3,
			list_id: "everyone".into(),
		});
		let mut apply = |value: serde_json::Value| {
			list.update(decode::<MemberUpdate>(value.to_string().as_bytes()).unwrap())
		};
		assert!(!apply(json!({"guild_id":"9","id":"everyone","member_count":2,"ops":[]})).unwrap());
		assert!(apply(json!({"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"SYNC","range":[0,99],"items":[{"group":{"id":"online","count":2}},{"member":{"user":{"id":"4","username":"First"}}},{"member":{"user":{"id":"5","username":"Second"}}}]}]})).unwrap());
		assert!(list.synced);
		assert!(list.rows[0].is_none());
		assert_eq!(list.rows[2].as_ref().unwrap().user.id, Id(5));
		list.update(decode(json!({"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"DELETE","index":1},{"op":"INSERT","index":2,"item":{"member":{"user":{"id":"6","username":"Third"}}}},{"op":"UPDATE","index":1,"item":{"member":{"user":{"id":"5","username":"Updated"}}}}]}).to_string().as_bytes()).unwrap()).unwrap();
		assert_eq!(list.rows[1].as_ref().unwrap().user.name, "Updated");
		assert_eq!(list.rows[2].as_ref().unwrap().user.id, Id(6));
		assert_eq!(list.rows.len(), 100);
		list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"INVALIDATE","range":[0,99]}]}"#).unwrap()).unwrap();
		assert!(!list.synced);
		assert!(list.rows.iter().all(Option::is_none));
		assert!(list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"SYNC","range":[9,1],"items":[]}]}"#).unwrap()).is_err());
	}
	#[tokio::test]
	async fn visible_member_subscription_uses_local_socket_and_unsubscribes() {
		use tokio::net::TcpListener;
		use tokio_tungstenite::{
			accept_async,
			tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
		};
		timeout(Duration::from_secs(10),async {
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let endpoint=format!("ws://{}/",listener.local_addr().unwrap());
            let (selection,receive)=watch::channel(Some(MemberSubscription {guild:Id(1),channel:Id(2),request:7,list_id:"everyone".into()}));
            let server=async {
                let (stream,_)=listener.accept().await.unwrap();let mut socket=accept_async(stream).await.unwrap();
                socket.send(Frame::Text(json!({"op":10,"d":{"heartbeat_interval":1000}}).to_string().into())).await.unwrap();
                assert!(matches!(socket.next().await,Some(Ok(Frame::Text(_)))));
                socket.send(Frame::Text(json!({"op":0,"t":"READY","s":1,"d":{"user":{"id":"1","username":"Owner"},"session_id":"synthetic-members","resume_gateway_url":"wss://gateway.discord.gg/","guilds":[],"private_channels":[]}}).to_string().into())).await.unwrap();
                let mut subscribed=false;
                let mut switched=false;
                while let Some(Ok(Frame::Text(text)))=socket.next().await {
                    let packet:serde_json::Value=serde_json::from_str(&text).unwrap();
                    if packet["op"]==1 {socket.send(Frame::Text(json!({"op":11,"d":null}).to_string().into())).await.unwrap();continue;}
                    assert_eq!(packet["op"],37);
                    assert_eq!(packet["d"]["subscriptions"].as_object().unwrap().len(),1);
                    let subscription=&packet["d"]["subscriptions"]["1"];
                    assert_eq!(subscription["threads"],false);
                    assert_eq!(subscription["activities"],false);
                    assert_eq!(subscription["members"],json!([]));
                    if !subscribed {
                        assert_eq!(subscription["typing"],true);assert_eq!(subscription["channels"],json!({"2":[[0,99]]}));subscribed=true;
                        socket.send(Frame::Text(json!({"op":0,"t":"GUILD_MEMBER_LIST_UPDATE","s":2,"d":{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Visible","avatar":"0123456789abcdef0123456789abcdef"}}}]}]}}).to_string().into())).await.unwrap();
                        for (sequence,data) in [
                            (3,json!({"guild_id":"9","user":{"id":"3"},"status":"dnd"})),
                            (4,json!({"guild_id":"1","user":{"id":"4"},"status":"online"})),
                            (5,json!({"guild_id":"1","user":{"id":"3"},"status":"idle","activities":[{"type":4,"state":"Synthetic live update"}]})),
                            (6,json!({"guild_id":"1","user":{"id":"3"}})),
                        ] {socket.send(Frame::Text(json!({"op":0,"t":"PRESENCE_UPDATE","s":sequence,"d":data}).to_string().into())).await.unwrap();}
                    } else if !switched {
                        assert_eq!(subscription["typing"],true);assert_eq!(subscription["channels"],json!({"4":[[0,99]]}));switched=true;
                        // A shared list need not send another full SYNC on a channel switch.
                    } else {assert_eq!(subscription["typing"],false);assert_eq!(subscription["channels"],json!({}));break;}
                }
                socket.close(Some(CloseFrame{code:CloseCode::Library(4004),reason:"synthetic stop".into()})).await.unwrap();
            };
            let client=run_inner(Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),"wss://gateway.discord.gg/".into(),receive,mpsc::channel(1).1,None,|event| {
                if let Event::Members(list)=&event {
                    assert_eq!(list.rows[0].as_ref().unwrap().user.id,Id(3));
                    assert_eq!(list.freshness,Freshness::Fresh);
                    if list.request==8 {
                        assert_eq!(list.channel,Id(4));
                        assert_eq!(list.rows[0].as_ref().unwrap().status.as_deref(),Some("idle"));
                        selection.send(None).unwrap();
                    } else { assert_eq!(list.request,7);assert_eq!(list.channel,Id(2)); }
                }
                if let Event::MemberPresence {guild,channel,request,updates}=event {
                    assert_eq!((guild,channel,request),(Id(1),Id(2),7));
                    assert_eq!(updates,vec![record(3,Some("idle"),Some("Synthetic live update"))]);
                    selection.send(Some(MemberSubscription {guild:Id(1),channel:Id(4),request:8,list_id:"everyone".into()})).unwrap();
                }
                Ok(())
            },Some(&endpoint));
            let ((),result)=tokio::join!(server,client);assert_eq!(result,Err(Failure::Expired));
        }).await.unwrap();
	}
}
