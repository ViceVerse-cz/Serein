//! Bounded, zlib-stream JSON Gateway. Normal-user Identify remains live-unverified.
mod activity;
mod channel_events;
mod compression;
mod connection_diagnostics;
mod outgoing;
const PERMISSION_REFRESH: &str = "Gateway permission metadata requires an authoritative refresh";
pub use connection_diagnostics::{ConnectionDiagnostics, ConnectionReason};
mod interactions;
#[cfg(test)]
mod login_tests;
mod member_search;
mod presence;
#[cfg(test)]
mod recovery_tests;
pub mod roster;
mod thread_events;
mod voice;
#[cfg(debug_assertions)]
pub use activity::debug_spotify_check;
use client_core::{
	Event, MAX_NAV,
	auth::{Failure, SessionSecret},
};
pub use discord_protocol::activity_sessions::Observation as ActivityObservation;
use discord_protocol::*;
use futures_util::{FutureExt, SinkExt, StreamExt};
use model::{Freshness, Id, Member, MemberList};
use std::{
	collections::{BTreeMap, BTreeSet},
	sync::{Arc, Mutex},
	time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
	sync::{Notify, mpsc, watch},
	time::{Instant, interval_at, sleep, timeout},
};
use tokio_tungstenite::{
	connect_async_with_config,
	tungstenite::{Message as Frame, protocol::WebSocketConfig},
};
use zeroize::Zeroizing;

/// Queues a redacted, user-copyable decode cause ahead of the fixed failure label.
fn diagnosed(
	emit: impl Fn(Event) -> Result<(), Failure>,
	failure: Failure,
	cause: String,
) -> Failure {
	let _ = emit(Event::FailureDetail(cause.into()));
	failure
}

fn socket_failure(error: tokio_tungstenite::tungstenite::Error) -> Failure {
	match error {
		tokio_tungstenite::tungstenite::Error::Capacity(_) => {
			Failure::CapacityAt("Gateway frame exceeds 64 MiB; connection stopped")
		}
		_ => Failure::Network,
	}
}

/// One bounded delivery generation; coalesced watch notifications cannot erase a loss.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryState {
	pub blocked: bool,
	pub recovering: bool,
	pub generation: u64,
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

fn catalog_scope(bytes: &[u8]) -> Option<Id> {
	#[derive(serde::Deserialize)]
	struct Scope {
		guild_id: Id,
	}
	decode::<Scope>(bytes)
		.ok()
		.map(|s| s.guild_id)
		.filter(|id| id.0 != 0)
}

#[derive(Default)]
struct ResumeState {
	session: Option<Zeroizing<String>>,
	url: Option<String>,
	sequence: Option<u64>,
	failures: u8,
}
impl ResumeState {
	fn failed(&mut self) {
		if self.session.is_some() {
			self.failures += 1;
			// A dead resume host must not strand an otherwise reachable account.
			if self.failures == 3 {
				*self = Self::default();
			}
		}
	}
	fn close(&mut self, code: u16) -> Result<(), Failure> {
		match close_action(code) {
			Reconnect::Stop => {
				return Err(if code == 4004 {
					Failure::Expired
				} else {
					Failure::Protocol
				});
			}
			Reconnect::Identify => *self = Self::default(),
			Reconnect::Resume => {}
		}
		Ok(())
	}
}
/// How long a recovery heartbeat may wait for its acknowledgement before the socket is replaced.
#[cfg(not(test))]
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Short in tests, so the silent-socket case does not spend its deadline waiting.
#[cfg(test)]
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
#[derive(Default)]
struct Heartbeat {
	pending: std::collections::VecDeque<(u64, Instant)>,
}
impl Heartbeat {
	#[cfg(test)]
	fn tick(&mut self, now: Instant, interval: Duration) -> Result<(), Failure> {
		if self.expired(now, interval) {
			return Err(Failure::Network);
		}
		self.pending.push_back((0, now));
		Ok(())
	}
	fn expired(&self, now: Instant, interval: Duration) -> bool {
		self.pending
			.front()
			.is_some_and(|(_, sent)| now.saturating_duration_since(*sent) >= interval)
	}
	fn collect(&mut self, writer: &outgoing::Outgoing, probe_requested: &mut Option<u64>) -> bool {
		while let Some(receipt) = writer.take_heartbeat() {
			if receipt.acknowledged {
				if *probe_requested == Some(receipt.ticket) {
					*probe_requested = None;
				}
				continue;
			}
			if self.pending.len() == outgoing::PACKETS {
				return false;
			}
			self.pending.push_back((receipt.ticket, receipt.sent));
		}
		true
	}
	fn probe_deadline(&self, requested: Option<u64>) -> Option<Instant> {
		self.pending
			.iter()
			.find(|(ticket, _)| Some(*ticket) == requested)
			.map(|(_, sent)| *sent + PROBE_TIMEOUT)
	}
	fn ack(&mut self) -> Option<u64> {
		self.pending.pop_front().map(|(ticket, _)| ticket)
	}
	fn acknowledge(&mut self, writer: &outgoing::Outgoing) -> Option<u64> {
		self.ack().or_else(|| writer.acknowledge_heartbeat())
	}
}
fn next_attempt(attempt: u32, ready_for: Option<Duration>) -> u32 {
	// Reset backoff only after a stable connection; cap it during prolonged outages.
	if ready_for.is_some_and(|duration| duration >= Duration::from_secs(60)) {
		1
	} else {
		attempt.saturating_add(1).min(6)
	}
}
async fn recovery_signal(reconnect: Option<&Notify>) {
	match reconnect {
		Some(reconnect) => reconnect.notified().await,
		None => std::future::pending().await,
	}
}
/// Recovery cancels reads and delays, never a potentially accepted socket write.
async fn recoverable_wait<T>(
	operation: impl std::future::Future<Output = T>,
	reconnect: Option<&Notify>,
) -> Option<T> {
	tokio::select! {
		biased;
		() = recovery_signal(reconnect) => None,
		result = operation => Some(result),
	}
}
/// Exercises recovery without a socket, account, or network request.
#[cfg(debug_assertions)]
pub fn debug_recovery_check() {
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_time()
		.build()
		.unwrap();
	runtime.block_on(async {
		let reconnect = Arc::new(Notify::new());
		for _ in 0..8 {
			reconnect.notify_one();
		}
		assert!(
			timeout(
				Duration::from_secs(1),
				recoverable_wait(sleep(Duration::from_secs(30)), Some(&reconnect)),
			)
			.await
			.unwrap()
			.is_none()
		);
		// All queued requests coalesce to one recovery, without a closed-channel spin.
		assert_eq!(
			recoverable_wait(async { 42 }, Some(&reconnect)).await,
			Some(42)
		);
		let wake = reconnect.clone();
		let sender = tokio::spawn(async move {
			tokio::task::yield_now().await;
			wake.notify_one();
		});
		assert!(
			timeout(
				Duration::from_secs(1),
				recoverable_wait(std::future::pending::<()>(), Some(&reconnect)),
			)
			.await
			.unwrap()
			.is_none()
		);
		sender.await.unwrap();
		reconnect.notify_one();
		// Initial authentication keeps its bounded attempt limit and cannot be cancelled.
		assert_eq!(recoverable_wait(async { 42 }, None).await, Some(42));
		assert!(
			recoverable_wait(std::future::pending::<()>(), Some(&reconnect))
				.await
				.is_none()
		);
		assert_eq!(next_attempt(5, None), 6);
		assert_eq!(next_attempt(5, Some(Duration::from_secs(60))), 1);
	});
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

#[derive(Clone, PartialEq, Eq)]
pub struct MemberSubscription {
	pub thread: bool,
	pub guild: Id,
	pub channel: Id,
	pub request: u64,
	pub list_id: String,
	pub ranges: Vec<[usize; 2]>,
}

const MEMBER_LIST_BYTES: usize = 256 * 1024;
const REPLAYED_PRESENCE_BATCHES: usize = 3;
/// Busy lists batch many row moves into one dispatch; each costs at most one 200-slot shift.
const MAX_MEMBER_OPS: usize = 1024;

fn defer_replayed_presence(pending: &mut Vec<Event>, event: Event) -> Result<(), Failure> {
	if pending.len() >= REPLAYED_PRESENCE_BATCHES {
		return Err(Failure::Busy);
	}
	pending.push(event);
	Ok(())
}
/// Lists this connection recently left: late replies are never adopted, and reopening one
/// restores the identity the service used for it.
const RETIRED_LISTS: usize = 8;
/// Server, computed list identity, and the service identity when it differed.
type RetiredList = (Id, String, Option<String>);

fn validate_member_ranges(ranges: &[[usize; 2]]) -> bool {
	if !(1..=2).contains(&ranges.len()) {
		return false;
	}
	for window in ranges.windows(2) {
		if window[0][0] >= window[1][0] || window[0][1] >= window[1][0] {
			return false;
		}
	}
	for &[start, end] in ranges {
		if start > end || end - start >= 100 || start % 100 != 0 {
			return false;
		}
	}
	true
}

fn subscription_span(ranges: &[[usize; 2]]) -> (usize, usize) {
	let start = ranges[0][0];
	let end = ranges[ranges.len() - 1][1];
	(start, end - start + 1)
}

fn subscription_packet(
	guild: Id,
	typing: bool,
	channel: Option<(Id, &[[usize; 2]])>,
	thread: bool,
) -> Frame {
	let channels = match channel {
		Some((channel, ranges)) if !thread => {
			serde_json::json!({ channel.to_string(): ranges })
		}
		_ => serde_json::json!({}),
	};
	// Channel ranges are ignored until a prior frame has subscribed the guild.
	// `typing` is that subscription. It does not send a typing notification.
	let threads: Vec<_> = channel
		.filter(|_| thread)
		.map(|(id, _)| id.to_string())
		.into_iter()
		.collect();
	let mut setting = serde_json::json!({"typing":typing,"threads":false,"activities":true,"member_updates":false,"channels":channels,"thread_member_lists":threads});
	if !typing {
		setting["members"] = serde_json::json!([]);
	}
	Frame::Text(
		serde_json::json!({"op":37,"d":{"subscriptions":{guild.to_string():setting}}})
			.to_string()
			.into(),
	)
}

#[derive(Clone)]
struct ActiveMembers {
	subscription: MemberSubscription,
	start: usize,
	slots: Vec<Option<model::MemberSlot>>,
	missing_identity: BTreeMap<Id, model::MissingMemberFields>,
	global_names: BTreeSet<Id>,
	lazy: bool,
	synced: bool,
	awaiting_sync: bool,
	/// Consecutive stalled-subscription resets; spaces out further resets.
	retries: u32,
	/// Identity Discord actually replied with when it differs from the computed one.
	wire_list: Option<String>,
	total: u64,
	groups: Vec<(String, u64)>,
	pending_presence: BTreeMap<Id, model::MemberPresence>,
	pending_presence_bytes: usize,
	presence_deadline: Option<Instant>,
}

/// Offline debug check of the user subscription packet and its bounded snapshot path.
#[cfg(debug_assertions)]
pub fn debug_thread_member_check(guild: Id, channel: Id, request: u64) -> MemberList {
	let Frame::Text(packet) = subscription_packet(guild, true, Some((channel, &[])), true) else {
		unreachable!()
	};
	let packet: serde_json::Value = serde_json::from_str(&packet).unwrap();
	let subscription = &packet["d"]["subscriptions"][guild.to_string()];
	assert_eq!(packet["op"], 37);
	assert_eq!(subscription["channels"], serde_json::json!({}));
	assert_eq!(
		subscription["thread_member_lists"],
		serde_json::json!([channel.to_string()])
	);
	let mut active = ActiveMembers::new(MemberSubscription {
		guild,
		channel,
		request,
		list_id: String::new(),
		thread: true,
		ranges: vec![],
	});
	let payload = serde_json::json!({"guild_id":guild.to_string(),"thread_id":channel.to_string(),"members":[{"user_id":"987","member":{"user":{"id":"987","username":"Synthetic thread participant"},"nick":"Post reader","roles":[]},"presence":{"status":"online","activities":[]}}]});
	assert!(
		active
			.thread_update(&serde_json::to_vec(&payload).unwrap())
			.unwrap()
	);
	let list = active.snapshot(Freshness::Fresh);
	let model::MemberSlot::Person(first) = list.slots[0].as_ref().unwrap() else {
		panic!("expected person");
	};
	assert_eq!(first.status.as_deref(), Some("online"));
	assert_eq!(first.nick.as_deref(), Some("Post reader"));
	let mut stale = payload.clone();
	stale["thread_id"] = serde_json::json!(if channel == Id(986) { "985" } else { "986" });
	assert!(
		!active
			.thread_update(&serde_json::to_vec(&stale).unwrap())
			.unwrap()
	);
	let mut invalid = payload.clone();
	invalid["members"][0]["user_id"] = serde_json::json!("988");
	assert!(
		active
			.thread_update(&serde_json::to_vec(&invalid).unwrap())
			.is_err()
	);
	let mut large = payload.clone();
	large["members"] = serde_json::Value::Array((1..=101).map(|id| serde_json::json!({"user_id":id.to_string(),"member":{"user":{"id":id.to_string(),"username":"Synthetic"}},"presence":{"status":"offline","activities":[]}})).collect());
	assert!(
		active
			.thread_update(&serde_json::to_vec(&large).unwrap())
			.unwrap()
	);
	assert_eq!(active.slots.len(), 100);
	assert_eq!(active.total, 101);
	let empty = serde_json::json!({"guild_id":guild.to_string(),"thread_id":channel.to_string(),"members":[]});
	assert!(
		active
			.thread_update(&serde_json::to_vec(&empty).unwrap())
			.unwrap()
	);
	assert!(active.synced && active.slots.is_empty());
	let Frame::Text(packet) = subscription_packet(guild, false, None, true) else {
		unreachable!()
	};
	let packet: serde_json::Value = serde_json::from_str(&packet).unwrap();
	assert_eq!(
		packet["d"]["subscriptions"][guild.to_string()]["thread_member_lists"],
		serde_json::json!([])
	);
	list
}

/// Synthetic member stream used by the offline example; never opens a connection.
#[cfg(debug_assertions)]
pub fn debug_member_list_check() {
	use serde_json::json;
	let mut active = ActiveMembers::new(MemberSubscription {
		thread: false,
		guild: Id(1),
		channel: Id(2),
		request: 1,
		list_id: "everyone".into(),
		ranges: vec![[0, 99]],
	});
	let read =
		|value: serde_json::Value| decode::<MemberUpdate>(value.to_string().as_bytes()).unwrap();
	active
		.update(read(
			json!({"guild_id":"1","id":"everyone","ops":[{"op":"SYNC","range":[0,99],"items":[]}]}),
		))
		.unwrap();
	assert!(
		!active.synced && active.awaiting_sync,
		"ambiguous empty replies must keep retrying"
	);
	let groups: Vec<_> = (1..=100)
		.map(|id| json!({"id":id.to_string(),"count":1}))
		.collect();
	active.update(read(json!({"guild_id":"1","id":"everyone","member_count":1,"groups":groups,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Synthetic"},"roles":["1"]},"presence":{"status":"online","activities":[{"type":0,"name":"Synthetic game"}]}}]}]}))).unwrap();
	assert!(active.synced && !active.awaiting_sync);
	assert_eq!(
		active.people().next().unwrap().status.as_deref(),
		Some("online")
	);
	assert_eq!(
		active.people().next().unwrap().activities[0].name,
		"Synthetic game"
	);
	active.update(read(json!({"guild_id":"1","id":"everyone","ops":[{"op":"UPDATE","index":0,"item":{"member":{"user":{"id":"3","username":"Renamed"},"roles":["2"]}}}]}))).unwrap();
	assert_eq!(active.total, 200);
	assert_eq!(active.groups.len(), 100);
	assert_eq!(
		active.people().next().unwrap().activities[0].name,
		"Synthetic game"
	);
	assert_eq!(active.people().next().unwrap().roles, vec![Id(2)]);
	assert!(active.update(read(json!({"guild_id":"1","id":"everyone","ops":[{"op":"DELETE","index":0},{"op":"SYNC","range":[9,1],"items":[]}]}))).is_err());
	assert_eq!(
		active.people().next().unwrap().user.name,
		"Renamed",
		"failed operations must be atomic"
	);
	active
		.update(read(
			json!({"guild_id":"1","id":"everyone","ops":[{"op":"INVALIDATE","range":[0,99]}]}),
		))
		.unwrap();
	assert!(active.awaiting_sync && active.synced);
	active.update(read(json!({"guild_id":"1","id":"everyone","ops":[{"op":"UPDATE","index":0,"item":{"member":{"user":{"id":"3","username":"Renamed"}},"presence":{"status":"offline","activities":[]}}}]}))).unwrap();
	assert!(
		active.awaiting_sync,
		"incremental updates cannot cancel recovery"
	);
	assert_eq!(
		active.people().next().unwrap().status.as_deref(),
		Some("offline")
	);
	assert!(active.people().next().unwrap().activities.is_empty());
	active.update(read(json!({"guild_id":"1","id":"everyone","ops":[{"op":"UPDATE","index":0,"item":{"member":{"user":{"id":"3","username":"Renamed"}},"presence":null}}]}))).unwrap();
	assert!(active.people().next().unwrap().status.is_none());
	active.retarget_ranges(vec![[200, 299]]);
	active.update(read(json!({"guild_id":"1","id":"everyone","member_count":0,"groups":[],"ops":[{"op":"SYNC","range":[200,299],"items":[]}]}))).unwrap();
	assert_eq!(active.total, 0);
	assert!(active.synced && !active.awaiting_sync && active.people().next().is_none());
}

impl ActiveMembers {
	fn new(subscription: MemberSubscription) -> Self {
		let (start, slots, lazy) = if subscription.thread {
			(0, Vec::new(), false)
		} else {
			let (start, len) = subscription_span(&subscription.ranges);
			(start, vec![None; len.min(200)], true)
		};
		Self {
			subscription,
			start,
			slots,
			missing_identity: BTreeMap::new(),
			global_names: BTreeSet::new(),
			lazy,
			synced: false,
			awaiting_sync: true,
			retries: 0,
			wire_list: None,
			total: 0,
			groups: vec![],
			pending_presence: BTreeMap::new(),
			pending_presence_bytes: 0,
			presence_deadline: None,
		}
	}
	fn list_id(&self) -> &str {
		self.wire_list
			.as_deref()
			.unwrap_or(&self.subscription.list_id)
	}
	/// Waits 15, 30, 60, then 120 seconds between resets of a subscription that never syncs.
	fn retry_delay(&self) -> Duration {
		Duration::from_secs(15 << self.retries.min(3))
	}
	/// The computed list identity is unofficial and built from locally cached permissions.
	/// Until this subscription first synchronizes, a populated SYNC over its viewport in the
	/// same server, from no list this connection recently left, is the list Discord chose.
	fn adopt_list(&mut self, update: &MemberUpdate, retired: &[RetiredList]) -> bool {
		if self.subscription.thread
			|| self.synced
			|| update.guild_id != self.subscription.guild
			|| update.id == self.list_id()
			|| update.id.is_empty()
			|| update.id.len() > 32
			|| retired.iter().any(|(guild, id, wire)| {
				*guild == update.guild_id
					&& (*id == update.id || wire.as_deref() == Some(update.id.as_str()))
			}) {
			return false;
		}
		let (start, end) = (self.start, self.span_end());
		let covers = update.ops.iter().any(|op| {
			matches!(op, MemberOp::Sync { range: [from, to], items }
				if !items.is_empty() && *from <= end && *to >= start)
		});
		if covers {
			self.wire_list = Some(update.id.clone());
		}
		covers
	}
	/// Sheds rich activity details from the far end first, then far rows, so an unusually
	/// heavy page still shows its people instead of failing the byte budget.
	fn fit_budget(&mut self) {
		let mut bytes = self.slot_bytes();
		for slot in self.slots.iter_mut().rev() {
			if bytes <= MEMBER_LIST_BYTES {
				return;
			}
			if let Some(model::MemberSlot::Person(member)) = slot
				&& !member.activities.is_empty()
			{
				let before = member.bytes();
				member.activities = Vec::new();
				bytes -= before - member.bytes();
			}
		}
		for slot in self.slots.iter_mut().rev() {
			if bytes <= MEMBER_LIST_BYTES {
				return;
			}
			if let Some(slot) = slot.take() {
				bytes -= slot.bytes();
			}
		}
	}
	fn note_identity_fields(&mut self, item: &discord_protocol::MemberItem) {
		if let discord_protocol::MemberItem::Member { member, .. } = item {
			let user = member.user.id;
			match &member.user.global_name {
				model::Patch::Absent => {}
				model::Patch::Null => {
					self.global_names.remove(&user);
				}
				model::Patch::Value(_) => {
					self.global_names.insert(user);
				}
			}
			let fields = model::MissingMemberFields {
				user,
				global_name: model::GlobalNameUpdate::from_patch(&member.user.global_name),
				nick: matches!(member.nick, model::Patch::Absent),
				roles: matches!(member.roles, model::Patch::Absent),
			};
			if fields.needs_metadata() {
				self.missing_identity.insert(user, fields);
			} else {
				self.missing_identity.remove(&user);
			}
		}
	}
	fn prune_identity_fields(&mut self) {
		let visible: BTreeSet<_> = self.people().map(|member| member.user.id).collect();
		self.missing_identity
			.retain(|user, _| visible.contains(user));
		self.global_names.retain(|user| visible.contains(user));
	}
	fn prune_global_names(&mut self) {
		if !self.global_names.is_empty() {
			let visible: BTreeSet<_> = self.people().map(|member| member.user.id).collect();
			self.global_names.retain(|user| visible.contains(user));
		}
	}
	fn span_end(&self) -> usize {
		self.start
			.saturating_add(self.slots.len().saturating_sub(1))
	}
	fn slot_bytes(&self) -> usize {
		self.slots
			.iter()
			.flatten()
			.map(model::MemberSlot::bytes)
			.sum::<usize>()
			+ self.missing_identity.len()
				* (size_of::<Id>() + size_of::<model::MissingMemberFields>() + 48)
			+ self.global_names.len() * (size_of::<Id>() + 48)
	}
	fn people(&self) -> impl Iterator<Item = &Member> {
		self.slots.iter().filter_map(|slot| match slot {
			Some(model::MemberSlot::Person(member)) => Some(member),
			_ => None,
		})
	}
	fn people_mut(&mut self) -> impl Iterator<Item = &mut Member> {
		self.slots.iter_mut().filter_map(|slot| match slot {
			Some(model::MemberSlot::Person(member)) => Some(member),
			_ => None,
		})
	}
	fn retarget_ranges(&mut self, ranges: Vec<[usize; 2]>) {
		let (new_start, new_len) = subscription_span(&ranges);
		let new_len = new_len.min(200);
		let new_end = new_start + new_len.saturating_sub(1);
		let mut next = vec![None; new_len];
		for (offset, slot) in self.slots.iter().enumerate() {
			let absolute = self.start + offset;
			if absolute >= new_start && absolute <= new_end {
				next[absolute - new_start] = slot.clone();
			}
		}
		let has_people = next
			.iter()
			.any(|slot| matches!(slot, Some(model::MemberSlot::Person(_))));
		self.start = new_start;
		self.slots = next;
		self.prune_identity_fields();
		self.subscription.ranges = ranges;
		if !has_people {
			self.synced = false;
		}
		self.awaiting_sync = true;
	}
	fn parse_groups(
		groups: Vec<discord_protocol::MemberGroupCount>,
	) -> Result<Vec<(String, u64)>, Failure> {
		if groups.len() > model::permissions::MAX_ROLES + 2 {
			return Err(Failure::Protocol);
		}
		let mut out = Vec::with_capacity(groups.len());
		for group in groups {
			if group.id.is_empty() || group.id.len() > 32 {
				return Err(Failure::Protocol);
			}
			out.push((group.id, group.count));
		}
		Ok(out)
	}
	/// Unreadable rows become holes at their position, keeping later indices aligned.
	fn write_slot(&mut self, absolute: usize, item: discord_protocol::MemberItem) {
		if absolute >= self.start && absolute <= self.span_end() {
			self.slots[absolute - self.start] = item.into_guild_slot(self.subscription.guild);
		}
	}
	fn snapshot(&self, freshness: Freshness) -> MemberList {
		MemberList {
			missing_identity: self.missing_identity.values().copied().collect(),
			guild: Some(self.subscription.guild),
			channel: self.subscription.channel,
			request: self.subscription.request,
			start: self.start,
			slots: self.slots.clone(),
			total: self.total,
			lazy: self.lazy,
			freshness,
			groups: self.groups.clone(),
			ranges: self.subscription.ranges.clone(),
		}
	}
	fn identity(&mut self, update: &model::MemberIdentity) {
		if update.guild != self.subscription.guild || !update.valid() {
			return;
		}
		let fallback_name = update.profile.as_ref().filter(|_| {
			update.global_name == model::GlobalNameUpdate::Unchanged
				&& !self.global_names.contains(&update.user)
		});
		if update.profile.is_some() && self.people().any(|member| member.user.id == update.user) {
			match update.global_name {
				model::GlobalNameUpdate::Unchanged => {}
				model::GlobalNameUpdate::Cleared => {
					self.global_names.remove(&update.user);
				}
				model::GlobalNameUpdate::Set => {
					self.global_names.insert(update.user);
				}
			}
		}
		for slot in &mut self.slots {
			if let Some(model::MemberSlot::Person(member)) = slot
				&& member.user.id == update.user
			{
				if update.departed {
					*slot = None;
				} else {
					update.apply(member);
					if let Some(profile) = fallback_name {
						member.user.name.clone_from(&profile.name);
					}
				}
			}
		}
		if update.departed {
			if let Some(presence) = self.pending_presence.remove(&update.user) {
				self.pending_presence_bytes -= presence.heap_bytes();
			}
			self.missing_identity.remove(&update.user);
			self.global_names.remove(&update.user);
		} else if let Some(fields) = self.missing_identity.get_mut(&update.user) {
			if update.profile.is_some() && update.global_name != model::GlobalNameUpdate::Unchanged
			{
				fields.global_name = update.global_name;
			}
			fields.nick &= matches!(update.nick, model::Patch::Absent);
			fields.roles &= matches!(update.roles, model::Patch::Absent);
			if !fields.needs_metadata() {
				self.missing_identity.remove(&update.user);
			}
		}
		self.fit_budget();
		self.prune_identity_fields();
	}
	fn update(&mut self, update: MemberUpdate) -> Result<bool, Failure> {
		if self.subscription.thread
			|| update.guild_id != self.subscription.guild
			|| update.id != self.list_id()
		{
			return Ok(false);
		}
		// Stage the bounded mirror so a bad item cannot partially erase a valid list.
		let mut next = self.clone();
		next.apply_update(update)?;
		*self = next;
		Ok(true)
	}
	fn apply_update(&mut self, update: MemberUpdate) -> Result<(), Failure> {
		// The emitted full snapshot includes the mirror's latest statuses, so a
		// separate queued delta must not race it or reference a removed row.
		self.clear_presence();
		if update.ops.len() > MAX_MEMBER_OPS {
			return Err(Failure::Capacity);
		}
		if let Some(groups) = update.groups {
			self.groups = Self::parse_groups(groups)?;
			// List positions include group headers and only the members visible in this list.
			self.total = self.groups.iter().fold(0u64, |total, (_, count)| {
				total.saturating_add(count.saturating_add(1))
			});
		} else if let Some(total) = update.member_count {
			self.total = total;
		}
		let span_start = self.start;
		let span_end = self.span_end();
		for op in update.ops {
			match op {
				MemberOp::Sync {
					range: [start, end],
					mut items,
				} => {
					if start > end
						|| items.len() > end.saturating_sub(start).saturating_add(1)
						|| items.len() > 100
					{
						return Err(Failure::Protocol);
					}
					if end < span_start || start > span_end {
						continue;
					}
					if items.is_empty() && update.member_count != Some(0) {
						continue;
					}
					if !items.is_empty() {
						self.total = self.total.max(start.saturating_add(items.len()) as u64);
					}
					for item in &items {
						self.note_identity_fields(item);
					}
					// SYNC may reorder rows and omit unchanged member fields. Match by
					// identity before clearing the range, not by the old list position.
					let previous: BTreeMap<_, _> = self
						.people()
						.map(|member| (member.user.id, member))
						.collect();
					for item in &mut items {
						if let discord_protocol::MemberItem::Member { member, .. } = item
							&& let Some(known) = previous.get(&member.user.id)
						{
							let global_name_is_set = self.global_names.contains(&member.user.id);
							item.preserve_missing(known, global_name_is_set);
						}
					}
					let clear_from = start.max(span_start);
					let clear_to = end.min(span_end);
					for absolute in clear_from..=clear_to {
						self.slots[absolute - span_start] = None;
					}
					for (index, item) in items.into_iter().enumerate() {
						self.write_slot(start + index, item);
					}
					self.prune_global_names();
					self.synced = true;
					self.awaiting_sync = false;
					self.retries = 0;
				}
				MemberOp::Invalidate {
					range: [start, end],
				} => {
					if start > end {
						return Err(Failure::Protocol);
					}
					if start <= span_end && end >= span_start {
						self.awaiting_sync = true;
					}
					// The range is stale, not permission to blank the sidebar. The next SYNC
					// replaces these slots. Clearing them here is what made the pane go empty.
				}
				// An unreadable replacement keeps the previous row rather than blanking it.
				MemberOp::Update {
					item: discord_protocol::MemberItem::Unreadable,
					..
				}
				| MemberOp::Unknown => {}
				MemberOp::Update { index, mut item } => {
					if self.synced && index >= span_start && index <= span_end {
						let old_user = match &self.slots[index - span_start] {
							Some(model::MemberSlot::Person(member)) => Some(member.user.id),
							_ => None,
						};
						self.note_identity_fields(&item);
						if let Some(model::MemberSlot::Person(previous)) =
							&self.slots[index - span_start]
						{
							item.preserve_missing(
								previous,
								self.global_names.contains(&previous.user.id),
							);
						}
						self.write_slot(index, item);
						let new_user = match &self.slots[index - span_start] {
							Some(model::MemberSlot::Person(member)) => Some(member.user.id),
							_ => None,
						};
						if old_user != new_user {
							self.prune_global_names();
						}
					}
				}
				MemberOp::Insert { index, item } => {
					if !self.synced || self.slots.is_empty() {
						continue;
					}
					if index > span_end {
						continue;
					}
					if index < span_start {
						self.slots.pop();
						self.slots.insert(0, None);
						self.prune_global_names();
						continue;
					}
					let relative = index - span_start;
					self.note_identity_fields(&item);
					self.slots
						.insert(relative, item.into_guild_slot(self.subscription.guild));
					if self.slots.len() > span_end - span_start + 1 {
						self.slots.pop();
					}
					self.prune_global_names();
				}
				MemberOp::Delete { index } => {
					if !self.synced || self.slots.is_empty() {
						continue;
					}
					if index > span_end {
						continue;
					}
					if index < span_start {
						self.slots.remove(0);
						self.slots.push(None);
						self.prune_global_names();
						continue;
					}
					let relative = index - span_start;
					self.slots.remove(relative);
					self.slots.push(None);
					self.prune_global_names();
				}
			}
		}
		if self.slots.len() > 200 {
			return Err(Failure::Capacity);
		}
		self.fit_budget();
		self.prune_identity_fields();
		Ok(())
	}
	fn thread_update(&mut self, bytes: &[u8]) -> Result<bool, Failure> {
		if !self.subscription.thread {
			return Ok(false);
		}
		#[derive(serde::Deserialize)]
		struct Scope {
			guild_id: Id,
			thread_id: Id,
		}
		let scope: Scope = decode(bytes).map_err(|_| Failure::Protocol)?;
		if scope.guild_id != self.subscription.guild || scope.thread_id != self.subscription.channel
		{
			return Ok(false);
		}
		let list = discord_protocol::thread_members::members(
			bytes,
			scope.guild_id,
			scope.thread_id,
			self.subscription.request,
		)
		.map_err(|_| Failure::Protocol)?;
		self.clear_presence();
		self.start = 0;
		self.global_names.clear();
		self.missing_identity = list
			.missing_identity
			.iter()
			.map(|fields| (fields.user, *fields))
			.collect();
		self.slots = list.slots;
		self.lazy = false;
		self.groups = vec![];
		self.total = list.total;
		self.synced = true;
		self.awaiting_sync = false;
		Ok(true)
	}
	fn clear_presence(&mut self) {
		self.pending_presence.clear();
		self.pending_presence_bytes = 0;
		self.presence_deadline = None;
	}
	#[cfg(test)]
	fn presence(&mut self, update: discord_protocol::presence::PresenceUpdate, now: Instant) {
		self.push_presence(update, now, &|_| Ok(())).unwrap();
	}
	fn push_presence(
		&mut self,
		update: discord_protocol::presence::PresenceUpdate,
		now: Instant,
		emit: &impl Fn(Event) -> Result<(), Failure>,
	) -> Result<(), Failure> {
		if !self.synced || update.guild != Some(self.subscription.guild) {
			return Ok(());
		}
		let Some(previous) = self.people().find(|row| row.user.id == update.user) else {
			return Ok(());
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
		let clients = match update.clients {
			model::Patch::Absent => previous.clients,
			model::Patch::Null => model::ClientPlatforms::default(),
			model::Patch::Value(clients) => clients,
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
			clients: if explicit_unknown || status.as_deref() == Some("offline") {
				model::ClientPlatforms::default()
			} else {
				clients
			},
			status,
			custom_status,
		};
		if !resolved.valid() {
			return Ok(());
		}
		let mut changed = false;
		let projected = self
			.slots
			.iter()
			.flatten()
			.map(|slot| match slot {
				model::MemberSlot::Person(row) if row.user.id == update.user => {
					changed |= row.status != resolved.status
						|| row.custom_status != resolved.custom_status
						|| row.activities != resolved.activities
						|| row.clients != resolved.clients;
					client_core::presence::projected_row_bytes(row, &resolved)
				}
				model::MemberSlot::Person(row) => row.bytes(),
				model::MemberSlot::Group(id) => id.capacity(),
			})
			.sum::<usize>();
		if projected > MEMBER_LIST_BYTES {
			return Ok(());
		}
		if !changed {
			return Ok(());
		}
		let old_bytes = self
			.pending_presence
			.get(&update.user)
			.map_or(0, model::MemberPresence::heap_bytes);
		let new_bytes = resolved.heap_bytes();
		let projected = self.pending_presence_bytes - old_bytes
			+ new_bytes
			+ 100 * size_of::<model::MemberPresence>()
			+ size_of::<Event>();
		if ((self.pending_presence.len() >= 100
			&& !self.pending_presence.contains_key(&update.user))
			|| projected > client_core::MAX_MEMBER_PRESENCE_BYTES)
			&& let Some(event) = self.take_presence()
		{
			emit(event)?;
		}
		if new_bytes + 100 * size_of::<model::MemberPresence>() + size_of::<Event>()
			> client_core::MAX_MEMBER_PRESENCE_BYTES
		{
			return Ok(());
		}
		for row in self.people_mut().filter(|row| row.user.id == update.user) {
			if row.status != resolved.status
				|| row.custom_status != resolved.custom_status
				|| row.activities != resolved.activities
				|| row.clients != resolved.clients
			{
				row.status = resolved.status.clone();
				row.custom_status = resolved.custom_status.clone();
				row.activities = resolved.activities.clone();
				row.clients = resolved.clients;
			}
		}
		if let Some(old) = self.pending_presence.insert(update.user, resolved) {
			self.pending_presence_bytes -= old.heap_bytes();
		}
		self.pending_presence_bytes += new_bytes;
		self.presence_deadline
			.get_or_insert(now + Duration::from_millis(100));
		Ok(())
	}
	fn take_presence(&mut self) -> Option<Event> {
		self.presence_deadline = None;
		self.pending_presence_bytes = 0;
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
#[allow(clippy::type_complexity)]
pub async fn run_with_activity(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	controls: mpsc::Receiver<client_core::voice::Command>,
	activity: (
		watch::Receiver<Option<discord_protocol::rpc::Activity>>,
		watch::Receiver<model::OwnPresence>,
		watch::Receiver<[Option<client_core::member_search::Request>; 2]>,
		watch::Receiver<Option<discord_protocol::spotify::Activity>>,
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
			member_queries: activity.2,
			member_watches: None,
			roster: None,
			spotify: activity.3,
			observe: &observe,
		}),
		emit,
		#[cfg(test)]
		None,
	)
	.await
}
/// Runs the gateway with immediate recovery of an established session. The notification
/// stores one pending request and preserves RESUME; writes and voice joins are never replayed.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub async fn run_with_activity_recovery(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	controls: mpsc::Receiver<client_core::voice::Command>,
	reconnect: Arc<Notify>,
	activity: (
		watch::Receiver<Option<discord_protocol::rpc::Activity>>,
		watch::Receiver<model::OwnPresence>,
		watch::Receiver<[Option<client_core::member_search::Request>; 2]>,
		watch::Receiver<Option<discord_protocol::spotify::Activity>>,
	),
	observe: impl Fn(ActivityObservation) -> Result<(), Failure> + Sync,
	emit: impl Fn(Event) -> Result<(), Failure>,
) -> Result<(), Failure> {
	run_recoverable(
		secret,
		initial_url,
		subscriptions,
		controls,
		Some(ActivityInput {
			receiver: activity.0,
			own_presence: activity.1,
			member_queries: activity.2,
			member_watches: None,
			roster: None,
			spotify: activity.3,
			observe: &observe,
		}),
		Some(&reconnect),
		emit,
		#[cfg(test)]
		None,
	)
	.await
}
/// Sparse display-identity observations for the current accessible conversation.
/// IDs are bounded to 128 and never authorize membership or permissions.
pub type MemberWatch = (Id, Id, Vec<Id>);

/// As recovery, with one account-owned bounded diagnostic history.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub async fn run_with_activity_recovery_diagnostics(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	controls: mpsc::Receiver<client_core::voice::Command>,
	reconnect: Arc<Notify>,
	activity: (
		watch::Receiver<Option<discord_protocol::rpc::Activity>>,
		watch::Receiver<model::OwnPresence>,
		watch::Receiver<[Option<client_core::member_search::Request>; 2]>,
		watch::Receiver<Option<discord_protocol::spotify::Activity>>,
	),
	delivery: watch::Receiver<DeliveryState>,
	diagnostics: Arc<Mutex<ConnectionDiagnostics>>,
	observe: impl Fn(ActivityObservation) -> Result<(), Failure> + Sync,
	emit: impl Fn(Event, u64) -> Result<(), Failure>,
) -> Result<(), Failure> {
	run_with_activity_recovery_diagnostics_members(
		secret,
		initial_url,
		subscriptions,
		controls,
		reconnect,
		activity,
		None,
		delivery,
		diagnostics,
		observe,
		emit,
	)
	.await
}
/// As diagnostics, with independent watches for already-cached member identities.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub async fn run_with_activity_recovery_diagnostics_members(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	controls: mpsc::Receiver<client_core::voice::Command>,
	reconnect: Arc<Notify>,
	activity: (
		watch::Receiver<Option<discord_protocol::rpc::Activity>>,
		watch::Receiver<model::OwnPresence>,
		watch::Receiver<[Option<client_core::member_search::Request>; 2]>,
		watch::Receiver<Option<discord_protocol::spotify::Activity>>,
	),
	member_watches: Option<watch::Receiver<Option<MemberWatch>>>,
	delivery: watch::Receiver<DeliveryState>,
	diagnostics: Arc<Mutex<ConnectionDiagnostics>>,
	observe: impl Fn(ActivityObservation) -> Result<(), Failure> + Sync,
	emit: impl Fn(Event, u64) -> Result<(), Failure>,
) -> Result<(), Failure> {
	run_with_activity_recovery_diagnostics_sync(
		secret,
		initial_url,
		subscriptions,
		controls,
		reconnect,
		activity,
		member_watches,
		None,
		delivery,
		diagnostics,
		observe,
		emit,
	)
	.await
}
/// As member diagnostics, with a separate bounded, explicitly authorized roster refresh lane.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub async fn run_with_activity_recovery_diagnostics_sync(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	controls: mpsc::Receiver<client_core::voice::Command>,
	reconnect: Arc<Notify>,
	activity: (
		watch::Receiver<Option<discord_protocol::rpc::Activity>>,
		watch::Receiver<model::OwnPresence>,
		watch::Receiver<[Option<client_core::member_search::Request>; 2]>,
		watch::Receiver<Option<discord_protocol::spotify::Activity>>,
	),
	member_watches: Option<watch::Receiver<Option<MemberWatch>>>,
	roster: Option<roster::RosterInput>,
	delivery: watch::Receiver<DeliveryState>,
	diagnostics: Arc<Mutex<ConnectionDiagnostics>>,
	observe: impl Fn(ActivityObservation) -> Result<(), Failure> + Sync,
	emit: impl Fn(Event, u64) -> Result<(), Failure>,
) -> Result<(), Failure> {
	run_observed(
		secret,
		initial_url,
		subscriptions,
		controls,
		Some(ActivityInput {
			receiver: activity.0,
			own_presence: activity.1,
			member_queries: activity.2,
			member_watches,
			roster,
			spotify: activity.3,
			observe: &observe,
		}),
		Some(&reconnect),
		Some(delivery),
		diagnostics,
		emit,
		#[cfg(test)]
		None,
	)
	.await
}
#[derive(Clone)]
struct ActivityInput<'a> {
	spotify: watch::Receiver<Option<discord_protocol::spotify::Activity>>,
	member_queries: watch::Receiver<[Option<client_core::member_search::Request>; 2]>,
	member_watches: Option<watch::Receiver<Option<MemberWatch>>>,
	roster: Option<roster::RosterInput>,
	receiver: watch::Receiver<Option<discord_protocol::rpc::Activity>>,
	own_presence: watch::Receiver<model::OwnPresence>,
	observe: &'a (dyn Fn(ActivityObservation) -> Result<(), Failure> + Sync),
}
async fn run_inner(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	voice_controls: mpsc::Receiver<client_core::voice::Command>,
	activity: Option<ActivityInput<'_>>,
	emit: impl Fn(Event) -> Result<(), Failure>,
	#[cfg(test)] test_endpoint: Option<&str>,
) -> Result<(), Failure> {
	run_recoverable(
		secret,
		initial_url,
		subscriptions,
		voice_controls,
		activity,
		None,
		emit,
		#[cfg(test)]
		test_endpoint.map(|endpoint| (endpoint, endpoint)),
	)
	.await
}
#[allow(clippy::too_many_arguments)]
async fn run_recoverable(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	voice_controls: mpsc::Receiver<client_core::voice::Command>,
	activity: Option<ActivityInput<'_>>,
	reconnect: Option<&Notify>,
	emit: impl Fn(Event) -> Result<(), Failure>,
	#[cfg(test)] test_endpoints: Option<(&str, &str)>,
) -> Result<(), Failure> {
	run_observed(
		secret,
		initial_url,
		subscriptions,
		voice_controls,
		activity,
		reconnect,
		None,
		Arc::new(Mutex::new(ConnectionDiagnostics::default())),
		|event, _| emit(event),
		#[cfg(test)]
		test_endpoints,
	)
	.await
}
#[allow(clippy::too_many_arguments)]
async fn run_observed(
	secret: Arc<SessionSecret>,
	initial_url: String,
	subscriptions: watch::Receiver<Option<MemberSubscription>>,
	mut voice_controls: mpsc::Receiver<client_core::voice::Command>,
	activity: Option<ActivityInput<'_>>,
	reconnect: Option<&Notify>,
	delivery: Option<watch::Receiver<DeliveryState>>,
	diagnostics: Arc<Mutex<ConnectionDiagnostics>>,
	emit: impl Fn(Event, u64) -> Result<(), Failure>,
	#[cfg(test)] test_endpoints: Option<(&str, &str)>,
) -> Result<(), Failure> {
	let limits = Arc::new(Mutex::new(outgoing::Cooldowns::default()));
	let mut recovering = false;
	let mut recovery_attempt = 0;
	let result = loop {
		let started = Instant::now();
		let epoch = delivery.as_ref().map_or(0, |d| d.borrow().generation);
		let result = run_observed_inner(
			secret.clone(),
			initial_url.clone(),
			subscriptions.clone(),
			&mut voice_controls,
			activity.clone(),
			reconnect,
			delivery.clone(),
			diagnostics.clone(),
			limits.clone(),
			recovering,
			|event| emit(event, epoch),
			#[cfg(test)]
			test_endpoints,
		)
		.await;
		let permission_refresh = result == Err(Failure::ProtocolAt(PERMISSION_REFRESH));
		if result != Err(Failure::Busy) && !permission_refresh {
			break result;
		}
		if let Ok(mut history) = diagnostics.lock() {
			history.record(
				if permission_refresh {
					ConnectionReason::PermissionRefresh
				} else {
					ConnectionReason::DeliveryCongestion
				},
				None,
				0,
				(0, 0),
			);
		}
		// Once any part of a dispatch was admitted, replaying it could duplicate
		// mutations. Revoke stale permissions and obtain a fresh snapshot instead.
		loop {
			let epoch = delivery.as_ref().map_or(0, |d| d.borrow().generation);
			match emit(Event::Disconnected, epoch).and_then(|()| emit(Event::Resync, epoch)) {
				Ok(()) => break,
				Err(Failure::Busy) => sleep(Duration::from_millis(250)).await,
				Err(failure) => return Err(failure),
			}
		}
		if recovery_attempt > 0 {
			sleep(Duration::from_millis(
				(1000_u64 << recovery_attempt.min(5)) + jitter_ms(1000),
			))
			.await;
		}
		recovery_attempt = next_attempt(recovery_attempt, Some(started.elapsed()));
		recovering = true;
	};
	if let Err(failure) = result
		&& let Ok(mut history) = diagnostics.lock()
	{
		// An explicit close has already retained its numeric code. Other terminal
		// errors can originate from any bounded decoder or downstream emitter.
		if !history.ended_with_close() {
			let reason = match failure {
				Failure::Capacity | Failure::CapacityAt(_) => ConnectionReason::LocalCapacity,
				Failure::Expired | Failure::InvalidCredential | Failure::Challenged => {
					ConnectionReason::SessionEnded
				}
				Failure::Network => history
					.latest_reason()
					.filter(|reason| {
						!matches!(
							reason,
							ConnectionReason::Connecting
								| ConnectionReason::Connected
								| ConnectionReason::Resumed
						)
					})
					.unwrap_or(ConnectionReason::SocketRead),
				_ => ConnectionReason::Protocol,
			};
			history.record(reason, None, 0, (0, 0));
		}
	}
	result
}
#[allow(clippy::too_many_arguments)]
async fn run_observed_inner(
	secret: Arc<SessionSecret>,
	initial_url: String,
	mut subscriptions: watch::Receiver<Option<MemberSubscription>>,
	voice_controls: &mut mpsc::Receiver<client_core::voice::Command>,
	activity: Option<ActivityInput<'_>>,
	reconnect: Option<&Notify>,
	mut delivery: Option<watch::Receiver<DeliveryState>>,
	diagnostics: Arc<Mutex<ConnectionDiagnostics>>,
	limits: Arc<Mutex<outgoing::Cooldowns>>,
	recovering: bool,
	emit: impl Fn(Event) -> Result<(), Failure>,
	#[cfg(test)] test_endpoints: Option<(&str, &str)>,
) -> Result<(), Failure> {
	let record = |reason, code, attempt, queue| {
		if let Ok(mut history) = diagnostics.lock() {
			history.record(reason, code, attempt, queue);
		}
	};
	if recovering {
		while voice_controls.try_recv().is_ok() {}
	}
	let delivery_fence = delivery.clone();
	let delivery_epoch = delivery.as_ref().map_or(0, |d| d.borrow().generation);
	let emit = |event| {
		if delivery_fence.as_ref().is_some_and(|d| {
			let state = *d.borrow();
			state.blocked || state.generation != delivery_epoch
		}) {
			return Err(Failure::Busy);
		}
		emit(event)
	};
	let mut delivery_open = delivery.is_some();
	let initial_url = validated_url(&initial_url)?;
	let activity_enabled = activity.is_some();
	let mut activity_open = activity_enabled;
	let mut presence_open = activity_enabled;
	let mut spotify_open = activity_enabled;
	let ignore_observation = |_| Ok(());
	let ActivityInput {
		receiver: mut activity,
		mut spotify,
		mut member_queries,
		member_watches,
		roster,
		mut own_presence,
		observe,
	} = activity.unwrap_or_else(|| ActivityInput {
		receiver: watch::channel(None).1,
		spotify: watch::channel(None).1,
		member_queries: watch::channel(Default::default()).1,
		member_watches: None,
		roster: None,
		own_presence: watch::channel(model::OwnPresence::default()).1,
		observe: &ignore_observation,
	});
	let mut roster_open = roster.is_some();
	let mut roster_input = roster.clone();
	let mut roster_worker = roster::Worker::new(roster);
	let mut watches_open = member_watches.is_some();
	let mut member_watches = member_watches.unwrap_or_else(|| watch::channel(None).1);
	let mut last_observation = None;
	let mut outgoing_activity = activity::Pending::default();
	outgoing_activity.update(&activity.borrow_and_update())?;
	outgoing_activity.update_spotify(&spotify.borrow_and_update())?;
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
	let mut was_ready = recovering;
	let mut owner_id = None;
	let mut attempt = 0;
	let mut calls = voice::Calls::default();
	// Survives RESUME: the service keeps delivering to lists subscribed before the drop.
	let mut retired_lists: Vec<RetiredList> = Vec::new();
	let mut inbox = channel_events::Inbox::default();
	let mut known_guilds = std::collections::BTreeSet::new();
	let mut voice_open = true;
	let mut skip_backoff = false;
	let mut immediate_reconnect_used = false;
	let mut queries = member_search::Search::default();
	// Initial login is bounded, but an established session must survive long outages.
	while was_ready || attempt < 6 {
		if attempt == 0 {
			let penalty = limits
				.lock()
				.map_err(|_| Failure::Network)?
				.connection_until
				.take();
			if let Some(until) = penalty {
				tokio::time::sleep_until(until).await;
			}
		}
		if attempt > 0 {
			roster_worker.fail(Failure::Network);
			calls.disconnected();
			while voice_controls.try_recv().is_ok() {}
			emit(Event::Disconnected)?;
			let penalty = limits
				.lock()
				.map_err(|_| Failure::Network)?
				.connection_until
				.take();
			if let Some(until) = penalty {
				tokio::time::sleep_until(until).await;
				skip_backoff = true;
			}
			if !std::mem::take(&mut skip_backoff) {
				let _ = recoverable_wait(
					sleep(Duration::from_millis(
						(1000_u64 << attempt.min(5)) + jitter_ms(1000),
					)),
					if was_ready { reconnect } else { None },
				)
				.await;
			}
		}
		// The imminent fresh dial already satisfies a queued recovery request.
		// In particular, Disconnected can synchronously queue one while an
		// immediate server-directed reconnect deliberately skips the backoff.
		let _ = recovery_signal(reconnect).now_or_never();
		record(ConnectionReason::Connecting, None, attempt, (0, 0));
		let url = state.url.as_deref().unwrap_or(&initial_url);
		// Compiled out of shipped builds. Tests replace only dialing, never URL validation.
		#[cfg(test)]
		let url = test_endpoints.map_or(
			url,
			|(initial, resume)| {
				if url == initial_url { initial } else { resume }
			},
		);
		// Application admission already coalesces optional messages; TCP_NODELAY
		// keeps small heartbeat and Stop packets out of Nagle/delayed-ACK waits.
		let config = WebSocketConfig::default()
			.max_message_size(Some(MAX_GATEWAY_WIRE))
			.max_frame_size(Some(MAX_GATEWAY_WIRE))
			.write_buffer_size(0)
			.max_write_buffer_size(64 * 1024);
		let Some(connection) = recoverable_wait(
			timeout(
				Duration::from_secs(15),
				connect_async_with_config(url, Some(config), true),
			),
			if was_ready { reconnect } else { None },
		)
		.await
		else {
			skip_backoff = true;
			continue;
		};
		let Ok(Ok((mut socket, _))) = connection else {
			record(
				if connection.is_err() {
					ConnectionReason::DialTimeout
				} else {
					ConnectionReason::DialFailed
				},
				None,
				attempt,
				(0, 0),
			);
			state.failed();
			attempt = next_attempt(attempt, None);
			continue;
		};
		let mut compression = compression::Decoder::default();
		let Some(hello) = recoverable_wait(
			timeout(Duration::from_secs(10), async {
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
			}),
			if was_ready { reconnect } else { None },
		)
		.await
		else {
			skip_backoff = true;
			continue;
		};
		if let Ok(Err(failure)) = hello
			&& failure.ends_session()
		{
			return Err(failure);
		}
		if let Ok(Ok(Frame::Close(Some(close)))) = &hello {
			let code = u16::from(close.code);
			record(
				if code == 4008 {
					ConnectionReason::RateLimited
				} else {
					ConnectionReason::ServerClose
				},
				Some(code),
				attempt,
				(0, 0),
			);
			if code == 4008 {
				limits
					.lock()
					.map_err(|_| Failure::Network)?
					.connection_until = Some(Instant::now() + Duration::from_secs(60));
			}
			state.close(code)?;
		}
		let Ok(Ok(Frame::Text(text))) = hello else {
			if !matches!(hello, Ok(Ok(Frame::Close(_)))) {
				record(
					match hello {
						Err(_) => ConnectionReason::HelloTimeout,
						Ok(Err(Failure::Network)) => ConnectionReason::SocketRead,
						_ => ConnectionReason::Protocol,
					},
					None,
					attempt,
					(0, 0),
				);
			}
			state.failed();
			attempt = next_attempt(attempt, None);
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
		let handshake_write = timeout(
			Duration::from_secs(5),
			socket.send(Frame::Text(encoded.as_str().to_owned().into())),
		)
		.await;
		if !matches!(handshake_write, Ok(Ok(()))) {
			record(
				if handshake_write.is_err() {
					ConnectionReason::HandshakeTimeout
				} else {
					ConnectionReason::SocketWrite
				},
				None,
				attempt,
				(0, 0),
			);
			state.failed();
			attempt = next_attempt(attempt, None);
			continue;
		}
		let mut heartbeat = Heartbeat::default();
		let interval = Duration::from_millis(hello.heartbeat_interval);
		let mut timer = interval_at(
			Instant::now() + Duration::from_millis(jitter_ms(hello.heartbeat_interval)),
			interval,
		);
		let (sink, mut incoming) = socket.split();
		let mut writer = outgoing::Outgoing::with_limits(sink, interval, limits.clone());
		let mut written_subscription = None;
		let mut probe_requested: Option<u64> = None;
		let mut overdue_started: Option<Instant> = None;
		let mut pending_frame = None;
		let mut priority_reads = 0usize;
		timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		let mut ready_at: Option<Instant> = None;
		let ready_deadline = Instant::now() + Duration::from_secs(30);

		let mut active_members: Option<ActiveMembers> = None;
		let mut direct_presence = presence::Pending::default();
		// The reducer ignores direct presence until RESUMED. If replay exceeds
		// these bounded batches, request a fresh sync rather than silently dropping it.
		let mut replayed_presence = Vec::with_capacity(REPLAYED_PRESENCE_BATCHES);
		let mut sent_members = false;
		let mut members_deadline: Option<Instant> = None;
		let mut subscriptions_open = true;
		let mut queries_open = true;
		queries.reconnect(state.session.is_some());
		if watches_open {
			writer.clear_queries(queries.watch(&member_watches.borrow_and_update()));
		}
		queries.update(&member_queries.borrow_and_update());
		let mut member_ticket = 0;
		outgoing_activity.reconnect();
		loop {
			if delivery.as_ref().is_some_and(|d| {
				let state = *d.borrow();
				state.blocked || state.generation != delivery_epoch
			}) {
				return Err(Failure::Busy);
			}
			if watches_open && member_watches.has_changed().unwrap_or(false) {
				writer.clear_queries(queries.watch(&member_watches.borrow_and_update()));
			}
			if !heartbeat.collect(&writer, &mut probe_requested) {
				record(
					ConnectionReason::LocalCapacity,
					None,
					attempt,
					writer.occupancy(),
				);
				break;
			}
			let probe_deadline = heartbeat.probe_deadline(probe_requested);
			if writer.is_finished() {
				record(
					ConnectionReason::SocketWrite,
					None,
					attempt,
					writer.occupancy(),
				);
				break;
			}
			// A paused task can wake with both its timeout and a buffered ACK ready.
			// Drain local buffered traffic for at most100ms, yielding every32 frames.
			// This allowance starts once and never extends the network deadlines.
			let now = Instant::now();
			let expiring =
				heartbeat.expired(now, interval) || probe_deadline.is_some_and(|at| now >= at);
			if !expiring {
				priority_reads = 0;
				overdue_started = None;
			}
			if expiring {
				let first_overdue = overdue_started.is_none();
				let started = *overdue_started.get_or_insert(now);
				if first_overdue {
					// Yield to the I/O driver before inspecting cached readiness.
					// The finite32 polls also bound this path under a paused test clock.
					for _ in 0..32 {
						pending_frame = incoming.next().now_or_never();
						if pending_frame.is_some() {
							break;
						}
						tokio::task::yield_now().await;
					}
				}
				if now.saturating_duration_since(started) < Duration::from_millis(100)
					&& pending_frame.is_none()
				{
					if priority_reads >= 32 {
						tokio::task::yield_now().await;
						priority_reads = 0;
					}
					pending_frame = incoming.next().now_or_never();
					if pending_frame.is_some() {
						priority_reads += 1;
					}
				}
				if pending_frame.is_none() {
					let probe = probe_deadline.is_some_and(|at| now >= at);
					record(
						if probe {
							ConnectionReason::ProbeTimeout
						} else {
							ConnectionReason::HeartbeatTimeout
						},
						None,
						attempt,
						writer.occupancy(),
					);
					skip_backoff = probe;
					break;
				}
			}
			if activity_enabled && last_observation != Some(outgoing_activity.observation) {
				observe(outgoing_activity.observation)?;
				last_observation = Some(outgoing_activity.observation);
			}
			if ready_at.is_some()
				&& writer.optional_capacity(1)
				&& let Some((guild, users, frame)) = queries.pending_subscription()
			{
				match writer.send(frame).await {
					Ok(()) => queries.subscribed(guild, users),
					Err(Failure::Busy) => {}
					Err(_) => break,
				}
			}
			if ready_at.is_some() && !sent_members && writer.optional_capacity(2) {
				let subscription = subscriptions.borrow_and_update().clone();
				if let Some(subscription) = subscription {
					if subscription.list_id.len() > 32 {
						return Err(Failure::Protocol);
					}
					if !subscription.thread && !validate_member_ranges(&subscription.ranges) {
						return Err(Failure::Protocol);
					}
					let ranges = subscription_packet(
						subscription.guild,
						true,
						Some((subscription.channel, subscription.ranges.as_slice())),
						subscription.thread,
					);
					// The empty-channels frame is only the first guild subscribe. Sending it
					// again replaces the open channel and drops the history stream.
					let guild_open = active_members.as_ref().is_some_and(|active| {
						active.subscription.guild == subscription.guild
							&& !active.subscription.thread
							&& !subscription.thread
					});
					if !guild_open {
						let typing = subscription_packet(subscription.guild, true, None, false);
						if !matches!(
							timeout(Duration::from_secs(5), writer.send(typing)).await,
							Ok(Ok(()))
						) {
							break;
						}
					}
					if !matches!(
						timeout(Duration::from_secs(5), writer.send(ranges)).await,
						Ok(Ok(()))
					) {
						break;
					}
					member_ticket = writer.subscription_ticket();
					members_deadline = None;
					member_diagnostics.record("subscription queued: member ranges");
					if let Some(active) = &mut active_members {
						// Channels with the same list identity share the existing SYNC.
						// A new viewport still has to move the stored span, or the next
						// snapshot keeps the previous hundred slots under the new ranges.
						if !subscription.thread
							&& !active.subscription.thread
							&& active.subscription.ranges != subscription.ranges
						{
							active.retarget_ranges(subscription.ranges.clone());
						}
						active.subscription = subscription;
						active.clear_presence();
						let freshness = if active.synced && !active.awaiting_sync {
							Freshness::Fresh
						} else {
							Freshness::Loading
						};
						if active.awaiting_sync {
							members_deadline = None;
						}
						emit(Event::Members(active.snapshot(freshness)))?;
					} else {
						let mut active = ActiveMembers::new(subscription);
						if !active.subscription.thread
							&& let Some(index) = retired_lists.iter().position(|(guild, id, _)| {
								*guild == active.subscription.guild
									&& *id == active.subscription.list_id
							}) {
							active.wire_list = retired_lists.remove(index).2;
						}
						members_deadline = None;
						active_members = Some(active);
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
				changed=async {match delivery.as_mut() {Some(d)=>d.changed().await,None=>std::future::pending().await}}, if delivery_open => {
					if changed.is_err(){delivery_open=false;}else if delivery.as_ref().is_some_and(|d| {let state=*d.borrow();state.blocked || state.generation!=delivery_epoch}){return Err(Failure::Busy);}
				}
				changed=writer.progress.changed() => {
					if changed.is_err() {record(ConnectionReason::SocketWrite,None,attempt,writer.occupancy());break;}
					let progress=*writer.progress.borrow_and_update();
					if progress.failed {break;}
					if progress.subscription!=written_subscription {
						written_subscription=progress.subscription;
						if let (Some((ticket,sent)),Some(active))=(progress.subscription,active_members.as_ref()) && ticket==member_ticket && active.awaiting_sync {members_deadline=Some(sent+active.retry_delay());}
					}
					for (guild,nonce,sent) in progress.query.into_iter().flatten() {queries.sent(guild,nonce,sent);}
					if let Some((guild,job,sent))=progress.roster {roster_worker.sent(guild,job,sent);}
				}
				() = recovery_signal(reconnect), if was_ready => {
					// READY/RESUMED can reach the transport before the UI drains its events.
					// Consume a late recovery request without dropping that healthy socket.
					if ready_at.is_none() {
						skip_backoff = true;
						break;
					}
					// A network change can leave a ready socket silently dead until the next two
					// heartbeats. Confirm it now with a heartbeat; a healthy socket acknowledges it.
					if probe_deadline.is_none() && probe_requested.is_none() {
						let packet = serde_json::json!({"op":1,"d":state.sequence}).to_string();
						if !matches!(timeout(Duration::from_secs(5), writer.send(Frame::Text(packet.into()))).await, Ok(Ok(()))) { skip_backoff = true; break; }
						probe_requested=Some(writer.heartbeat_ticket());
					}
				}
				_ = tokio::time::sleep_until(probe_deadline.unwrap_or(ready_deadline)), if probe_deadline.is_some() && pending_frame.is_none() => {
					// Re-enter the bounded buffered-frame check before retiring this socket.
					continue;
				}
				changed = own_presence.changed(), if presence_open => {
					presence_open = changed.is_ok();
					outgoing_activity.update_presence(&own_presence.borrow_and_update())?;
				}
				changed = spotify.changed(), if spotify_open => {
					spotify_open = changed.is_ok();
					outgoing_activity.update_spotify(&spotify.borrow_and_update())?;
				}
				changed = activity.changed(), if activity_open => {
					activity_open = changed.is_ok();
					outgoing_activity.update(&activity.borrow_and_update())?;
				}
				_ = tokio::time::sleep_until(activity_deadline.unwrap_or(ready_deadline)), if activity_deadline.is_some() && writer.optional_capacity(1) => {
					// Read the latest value even if a watch notification races the timer.
					outgoing_activity.update_spotify(&spotify.borrow_and_update())?;
					outgoing_activity.update(&activity.borrow_and_update())?;
					outgoing_activity.update_presence(&own_presence.borrow_and_update())?;
					if let Some(packet) = outgoing_activity.packet(Instant::now())
						&& !matches!(timeout(Duration::from_secs(5), writer.send(packet)).await, Ok(Ok(()))) { break; }
				}
				_=tokio::time::sleep_until(direct_presence.deadline.unwrap_or(ready_deadline)), if direct_presence.deadline.is_some() && ready_at.is_some() => {
					if let Some(event)=direct_presence.take() { emit(event)?; }
				}
				command=voice_controls.recv(), if voice_open && ready_at.is_some() && writer.control_capacity() => {
					let Some(command)=command else {voice_open=false;continue;};
					if let client_core::voice::Command::ConfirmSession {channel,request,revision}=command {
						if let Some(event)=calls.confirm_session(channel,request,revision) {emit(event)?;}
						continue;
					}
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
					if let Some(event)=calls.absent_stream_stop(command) {emit(event)?;}
					if let client_core::voice::Command::Leave { channel, request } | client_core::voice::Command::AbandonSession { channel, request } = command
						&& packet.is_none() && !calls.has_call() {
						emit(Event::Voice(client_core::voice::Event::Departed { channel, request }))?;
					}
					if let Some(channel)=connect && let Some(packet)=calls.packet(client_core::voice::Command::Sync { channel })?
						&& !matches!(timeout(Duration::from_secs(5),writer.send(packet)).await,Ok(Ok(()))) {break;}
					if let Some(packet)=packet && !matches!(timeout(Duration::from_secs(5),writer.send(packet)).await,Ok(Ok(()))) {
						if matches!(command,client_core::voice::Command::SetMute{..}|client_core::voice::Command::SetCamera{..}) {
							calls.state_send_failed();
						}
						break;
					}
					if let Some(channel)=connect && let Some(packet)=calls.channel_info_packet(channel)
						&& !matches!(timeout(Duration::from_secs(5),writer.send(packet)).await,Ok(Ok(()))) {break;}
				}

				_=tokio::time::sleep_until(calls.state_deadline.unwrap_or(ready_deadline)), if calls.state_deadline.is_some() && ready_at.is_some() && writer.control_capacity() => {
					if let Some(packet)=calls.flush_state() && !matches!(timeout(Duration::from_secs(5),writer.send(packet)).await,Ok(Ok(()))) {
						calls.state_send_failed();
						break;
					}
				}
				_=tokio::time::sleep_until(calls.departure_deadline.unwrap_or(ready_deadline)), if calls.departure_deadline.is_some() => {
					if let Some(event)=calls.departure_expired() {emit(event)?;}
				}
				changed = async { match roster_input.as_mut() { Some(input) => input.receive.changed().await, None => std::future::pending().await } }, if roster_open && ready_at.is_some() => {
					roster_open = changed.is_ok();
					writer.clear_roster();
					let request = if roster_open { roster_input.as_mut().and_then(|input| *input.receive.borrow_and_update()) } else { None };
					roster_worker.update(request);
				}
				_ = async {}, if roster_worker.pending() && ready_at.is_some() && writer.optional_capacity(1) => {
					if let Some(frame) = roster_worker.frame()
						&& let Some(job)=roster_input.as_ref().and_then(|input| input.receive.borrow().as_ref().map(|request| request.job)) {
							match writer.send_roster(frame,job).await {
								Ok(()) => roster_worker.queued(),
								Err(Failure::Busy) => {},
								Err(failure) => roster_worker.fail(failure),
							}
						}
				}
				_ = tokio::time::sleep_until(roster_worker.deadline().unwrap_or(ready_deadline)), if roster_worker.deadline().is_some() => { roster_worker.expire(); }

				changed = member_watches.changed(), if watches_open && ready_at.is_some() => {
					watches_open = changed.is_ok();
					if watches_open { writer.clear_queries(queries.watch(&member_watches.borrow_and_update())); }
					else { writer.clear_queries(queries.watch(&None)); }
				}

				changed = member_queries.changed(), if queries_open && ready_at.is_some() => {
					queries_open = changed.is_ok();
					let changed_queries=queries.update(&member_queries.borrow_and_update());
					writer.clear_queries(changed_queries);
					queries.report_cooldowns(&emit)?;
				}
				_ = tokio::time::sleep_until(queries.deadline().unwrap_or(ready_deadline)), if queries.deadline().is_some() && ready_at.is_some() && writer.optional_capacity(1) => {
					if let Some(query) = queries.tick(&emit)?
						&& !matches!(timeout(Duration::from_secs(5), writer.send_query(query)).await, Ok(Ok(()))) { break; }
				}
				changed=subscriptions.changed(), if subscriptions_open && ready_at.is_some() && writer.optional_capacity(2) => {
					subscriptions_open=changed.is_ok();
					let next = subscriptions.borrow().clone();
					let mut range_only = false;
					let same_list = subscriptions_open && (active_members.as_ref().is_some_and(|active|active.awaiting_sync || active.synced)) && active_members.as_ref().is_some_and(|active| {
						next.as_ref().is_some_and(|next| {
							!next.thread && !active.subscription.thread && next.guild == active.subscription.guild && next.list_id == active.subscription.list_id
						})
					});
					if same_list
						&& let (Some(active), Some(next)) = (active_members.as_mut(), next.as_ref())
					{
						let same_identity = next.channel == active.subscription.channel
							&& next.request == active.subscription.request;
						if same_identity && next.ranges == active.subscription.ranges {continue;}
						writer.clear_subscriptions();
						if same_identity && next.ranges != active.subscription.ranges {
							active.retarget_ranges(next.ranges.clone());
							active.subscription.channel = next.channel;
							active.subscription.request = next.request;
							active.subscription.list_id = next.list_id.clone();
							active.clear_presence();
							if active.awaiting_sync {
								members_deadline = None;
							}
							range_only = true;
							sent_members = false;
						}
					}
					if !same_list {
						writer.clear_subscriptions();
						if let Some(old)=active_members.take() {
							if !old.subscription.thread {
								retired_lists.retain(|(guild, id, _)| *guild != old.subscription.guild || *id != old.subscription.list_id);
								if retired_lists.len() == RETIRED_LISTS { retired_lists.remove(0); }
								retired_lists.push((old.subscription.guild, old.subscription.list_id.clone(), old.wire_list.clone()));
							}
							if !matches!(timeout(Duration::from_secs(5),writer.send(subscription_packet(old.subscription.guild,false,None,old.subscription.thread))).await,Ok(Ok(()))) {break;}
							queries.canceled_subscription(old.subscription.guild);
						}
						members_deadline=None;
					}
					if !range_only {
						member_diagnostics.record("subscription replaced or canceled");
						sent_members = !subscriptions_open;
					}
				}
				_=tokio::time::sleep_until(members_deadline.unwrap_or(ready_deadline)), if members_deadline.is_some() && writer.optional_capacity(3) => {
					member_diagnostics.record("timeout: resetting stalled member subscription");
					if let Some(active) = &active_members
						&& !matches!(timeout(Duration::from_secs(5), writer.send(subscription_packet(active.subscription.guild, true, None, active.subscription.thread))).await, Ok(Ok(()))) { break; }
					if let Some(active)=active_members.as_mut() {
						active.awaiting_sync = true;
						active.retries = active.retries.saturating_add(1);
						}

					sent_members = false;
					members_deadline=None;
				}
				_=tokio::time::sleep_until(presence_deadline.unwrap_or(ready_deadline)), if presence_deadline.is_some() => {
					if let Some(active)=&mut active_members {
						if subscriptions.has_changed().unwrap_or(true) {active.clear_presence();}
						else if let Some(event)=active.take_presence() {emit(event)?;}
					}
				}
				_ = tokio::time::sleep_until(ready_deadline), if ready_at.is_none() => { record(ConnectionReason::ReadyTimeout,None,attempt,writer.occupancy()); break; },
				_ = timer.tick(), if pending_frame.is_none() => {
					if heartbeat.expired(Instant::now(), interval) {continue;}
					let packet = serde_json::json!({"op":1,"d":state.sequence}).to_string();
					if !matches!(timeout(Duration::from_secs(5), writer.send(Frame::Text(packet.into()))).await, Ok(Ok(()))) { break; }
				}
				frame = async {match pending_frame.take() {Some(frame)=>frame,None=>incoming.next().await}} => {
					let frame = match frame {
						Some(Ok(frame)) => match compression.frame(frame)? {
							Some(frame) => Some(Ok(frame)),
							None => continue,
						},
						Some(Err(error)) => {
							record(ConnectionReason::SocketRead,None,attempt,writer.occupancy());
							let failure = socket_failure(error);
							if failure.ends_session() { return Err(failure); }
							break;
						},
						other => other,
					};
					match frame {
						Some(Ok(Frame::Text(text))) => {
							let packet: GatewayPacket = decode_gateway(text.as_bytes()).map_err(|_| diagnosed(emit, Failure::Protocol, diagnose_packet(text.as_bytes())))?;
							// Every dispatch shares one monotonic replay cursor. READY starts a new session.
							if packet.op == 0 && packet.t.as_deref()!=Some("READY")
								&& packet.s.zip(state.sequence).is_some_and(|(next,last)|next<=last) {continue;}
							let admitted_sequence=(packet.op==0).then_some(packet.s).flatten();
							match packet.op {
								11 => {
									// Drain receipts first; the writer reconciles ACKs that precede flush completion.
									if !heartbeat.collect(&writer, &mut probe_requested) {record(ConnectionReason::LocalCapacity,None,attempt,writer.occupancy());break;}
									if heartbeat.acknowledge(&writer).zip(probe_requested).is_some_and(|(acked,probe)|acked==probe) {probe_requested=None;}
									priority_reads=0;overdue_started=None;
								}
								1 => {
									let packet = serde_json::json!({"op":1,"d":state.sequence}).to_string();
									if !matches!(timeout(Duration::from_secs(5), writer.send(Frame::Text(packet.into()))).await, Ok(Ok(()))) { break; }
								}
								7 => {
									record(ConnectionReason::ServerReconnect, None, attempt, writer.occupancy());
									if ready_at.is_some_and(|ready| ready.elapsed() >= Duration::from_secs(60)) {
										immediate_reconnect_used = false;
									}
									skip_backoff = !immediate_reconnect_used;
									immediate_reconnect_used = true;
									break;
								},
								9 => {
									record(ConnectionReason::InvalidSession,None,attempt,writer.occupancy());
									let resumable: bool = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?;
									if !resumable {
										state = ResumeState::default();
									} else if ready_at.is_none() {
										// The required delay replaces backoff, not the failed RESUME budget.
										state.failed();
									}
									emit(Event::Disconnected)?;
									sleep(Duration::from_millis(1000 + jitter_ms(4000))).await; skip_backoff=true;
									break;
								}
								0 => match packet.t.as_deref().unwrap_or("") {
									"READY" => {
										direct_presence=presence::Pending::default();
										replayed_presence.clear();
										active_members=None;members_deadline=None;sent_members = !subscriptions_open;
										let envelope = ready::decode(packet.d.get().as_bytes()).map_err(|_| diagnosed(emit, Failure::ProtocolAt("Gateway login: invalid READY identity or relationships"), ready::diagnose(packet.d.get().as_bytes())))?;
										if envelope.user.bot { return Err(Failure::InvalidCredential); }
										let permissions = envelope.permissions().map_err(|_| diagnosed(emit, Failure::ProtocolAt("Gateway login: invalid permission metadata"), ready::diagnose(packet.d.get().as_bytes())))?;
										let gates = envelope.onboarding();
										let (mut ready, mut warnings) = envelope.navigation().map_err(|_| diagnosed(emit, Failure::ProtocolAt("Gateway login: invalid READY guild or channel metadata"), ready::diagnose(packet.d.get().as_bytes())))?;
										owner_id=Some(ready.user.id);
										if ready.user.username.is_empty() || ready.user.username.len() > 128 || ready.user.username.chars().any(char::is_control) || ready.session_id.is_empty() || ready.session_id.chars().any(char::is_control) {
											return Err(Failure::ProtocolAt("Gateway login: invalid account identity or session ID"));
										}
										if ready.session_id.len() > 2048 { return Err(Failure::CapacityAt("Gateway session ID exceeds 2 KiB; connection stopped")); }
										state.url = Some(validated_url(&ready.resume_gateway_url).map_err(|f|f.protocol_at("Gateway login: resume address rejected"))?);
										state.session = Some(Zeroizing::new(std::mem::take(&mut ready.session_id)));
										let friends = ready.relationships.as_ref().map(|s| s.friends(&ready.users)).transpose().map_err(|_| Failure::ProtocolAt("Invalid friend metadata"))?;
										let requests = ready.relationships.as_ref().map(|s| s.requests(&ready.users)).transpose().map_err(|_| Failure::ProtocolAt("Invalid friend request metadata"))?;
										let restricted = ready.relationships.as_ref().map(|s| s.restricted(&ready.users)).transpose().map_err(|_| Failure::ProtocolAt("Invalid blocked or ignored user metadata"))?;
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
										let mut message_requests = Vec::new();
										let mut message_spams = Vec::new();
										inbox.reset();
										for channel in &ready.private_channels {
											inbox.observe(channel);
											if channel.is_obfuscated() {
												continue;
											}
											if channel.pending_spam_direct() {
												message_spams.push(channel.id);
											} else if channel.pending_message_request() {
												message_requests.push(channel.id);
											}
										}
										// First occurrence wins, matching READY navigation admission. Rows omitted by
										// the bounds are unknown and must not hydrate from stale display hints.
										let mut display_seen = std::collections::BTreeSet::new();
										let display_capacity = ready.guilds.len().min(4096);
										let mut display_bytes = display_capacity * std::mem::size_of::<model::GuildPatch>();
										let mut display_metadata = Vec::with_capacity(display_capacity);
										for guild in &ready.guilds {
											if display_metadata.len() >= 4096 { break; }
											if guild.id.0 == 0 || !display_seen.insert(guild.id) { continue; }
											let patch = guild.display_patch();
											let field_bytes = |field: &model::Patch<String>| match field { model::Patch::Value(value) => value.capacity(), _ => 0 };
									let bytes = field_bytes(&patch.name) + field_bytes(&patch.icon);
											if display_bytes.saturating_add(bytes) > 2 * 1024 * 1024 { break; }
											display_bytes += bytes;
											display_metadata.push(patch);
										}
										let incident_now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as i128;
										let incident_updates:Vec<_>=ready.guilds.iter().filter_map(|guild|match guild.incident_patch(){model::Patch::Value(incident) if incident.active(incident_now)=>Some((guild.id,incident)),_=>None}).take(256).collect();
										let (guilds, channels) = ready.navigation().map_err(|_| Failure::ProtocolAt("Gateway login: invalid or oversized channel/thread navigation"))?;
										warnings.entries |= ready.skipped;
										let (read_entries,read_version,partial)=ready.read_state.take().map_or((None,None,false),|snapshot|(Some(snapshot.entries.into_iter().filter(|e|e.kind==0).map(|e|(e.id,e.last_message_id,e.mention_count)).collect()),snapshot.version,snapshot.partial));
										if guilds.len() + channels.len() > MAX_NAV { return Err(Failure::CapacityAt("Account navigation exceeds 131,072 entries; connection stopped")); }
										direct_presence.bootstrap_users=friends.as_ref().into_iter().flatten().map(|(u,_)|u.id).chain(channels.iter().filter(|c|c.guild.is_none() && matches!(c.kind,1|3)).flat_map(|c|c.recipients.iter().map(|u|u.id))).take(client_core::presence::MAX_DIRECT_PRESENCES).collect();
										calls.allowed=channels.iter().filter(|c|(c.guild.is_none() && channel_events::private_call(c.kind,c.recipients.len())) || (c.guild.is_some() && matches!(c.kind,2|13))).map(|c|(c.id,c.guild)).collect();
										if was_ready { emit(Event::Resync)?; }
										emit(Event::Interaction(client_core::interactions::Event::Session(state.session.clone().ok_or(Failure::Protocol)?)))?;
										let notifications = ready.user_guild_settings.take().map(|snapshot| {
											let (entries, replace) = snapshot.entries();
											notification_preferences(entries, replace)
										});
										emit(Event::Startup(Box::new(client_core::Startup {
											premium_type: match ready.user.premium_type { model::Patch::Value(kind) => kind, _ => 0 },
											user: ready.user.into_model(), guilds, channels, permissions,
											read_state: client_core::read_state::Event::Snapshot {entries:read_entries,version:read_version,partial},
											notifications, session_dnd: ready.sessions.as_ref().and_then(|s| s.dnd()), warnings,
										}.prepare()?)))?;
										emit(Event::GuildDisplayMetadata(display_metadata))?;
										for(guild,incident)in incident_updates{emit(Event::ServerIncident{guild,incident:Some(incident)})?;}
										// A new session does not replay settings changed while disconnected.
										if was_ready { emit(Event::AccountSettings { status: true, folders: true })?; }
										was_ready = true;
										immediate_reconnect_used=false;
										emit(Event::Onboarding(client_core::onboarding::Event::Gates { snapshot: true, gates }))?;

										let nicknames = ready.relationships.as_ref().map(|s| s.nicknames());
										let spam_requests = ready.relationships.as_ref().map(|s| s.spam_incoming_ids());
										let friends_since = ready.relationships.as_ref().map(|s| s.friends_since()).unwrap_or_default();
										emit(Event::UserAction(client_core::user_actions::Event::Relationships(ready.relationships.take().map(|s| s.entries()))))?;
										emit(Event::UserAction(client_core::user_actions::Event::Friends(friends)))?;
										emit(Event::UserAction(client_core::user_actions::Event::FriendsSince { entries: friends_since, replace: true }))?;
										emit(Event::UserAction(client_core::user_actions::Event::Restrictions(restricted)))?;
										emit(Event::UserAction(client_core::user_actions::Event::Requests(requests)))?;
										emit(Event::UserAction(client_core::user_actions::Event::RequestSpams(spam_requests)))?;
										emit(Event::UserAction(client_core::user_actions::Event::MessageRequests(Some(message_requests))))?;
										emit(Event::UserAction(client_core::user_actions::Event::MessageSpams(Some(message_spams))))?;
										if let Some(nicknames) = nicknames { emit(Event::UserAction(client_core::user_actions::Event::Nicknames(nicknames)))?; }
										if let Some(friends) = ready.merged_presences.as_ref().and_then(|m| m.friends.as_deref()).or(ready.presences.as_deref()) {
											direct_presence.friends(friends, Instant::now(), &emit)?;
										}
										if !participants.is_empty() { emit(Event::Voice(client_core::voice::Event::Snapshot { partial: false, guild: None, participants }))?; }
										state.failures = 0;
										record(ConnectionReason::Connected,None,attempt,writer.occupancy());
										ready_at = Some(Instant::now());
									}
									"RATE_LIMITED" => {
										#[derive(serde::Deserialize,Default)]
										struct Meta { guild_id: Option<Id>, nonce: Option<String> }
										#[derive(serde::Deserialize)]
										struct Limited { opcode: u8, retry_after: f64, #[serde(default)] meta: Meta }
										if packet.d.get().len()<=4096 && let Ok(limited)=decode::<Limited>(packet.d.get().as_bytes()) && limited.retry_after.is_finite() && (0.0..=86400.0).contains(&limited.retry_after) {
											let delay=Duration::from_secs_f64(limited.retry_after);
											writer.cooldown(limited.opcode,limited.meta.guild_id,delay);
											if limited.opcode==8 {queries.limited(limited.meta.guild_id,limited.meta.nonce.as_deref(),delay,&emit)?;roster_worker.limited(limited.meta.guild_id,limited.meta.nonce.as_deref());}
											record(ConnectionReason::RateLimited,None,attempt,writer.occupancy());
										}
									}
									"GUILD_MEMBERS_CHUNK" => {
										if !roster_worker.chunk(packet.d.get().as_bytes()) && let Some(event) = queries.chunk(packet.d.get().as_bytes()) { emit(event)?; }
									}
									"READY_SUPPLEMENTAL" => {
										let (mut extra, warnings) = ready::supplemental(packet.d.get().as_bytes()).map_err(|_| diagnosed(emit, Failure::ProtocolAt("Gateway login: invalid supplemental guild or voice metadata"), ready::diagnose_supplemental(packet.d.get().as_bytes())))?;
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
									"RESUMED" => {
										record(ConnectionReason::Resumed,None,attempt,writer.occupancy()); emit(Event::Interaction(client_core::interactions::Event::Session(state.session.clone().ok_or(Failure::Protocol)?)))?; emit(Event::Resumed)?; state.failures = 0;
										for event in replayed_presence.drain(..) { emit(event)?; }
										ready_at = Some(Instant::now()); },
									"GUILD_SCHEDULED_EVENT_EXCEPTION_CREATE" | "GUILD_SCHEDULED_EVENT_EXCEPTION_DELETE" | "GUILD_SCHEDULED_EVENT_EXCEPTION_UPDATE" | "GUILD_SCHEDULED_EVENT_CREATE" | "GUILD_SCHEDULED_EVENT_UPDATE" | "GUILD_SCHEDULED_EVENT_DELETE" | "GUILD_SCHEDULED_EVENT_USER_ADD" | "GUILD_SCHEDULED_EVENT_USER_REMOVE" | "STAGE_INSTANCE_CREATE" | "STAGE_INSTANCE_UPDATE" | "STAGE_INSTANCE_DELETE" => {
										#[derive(serde::Deserialize)] struct Notice { guild_id: Id, #[serde(default)] channel_id: Option<Id> }
										if let Ok(notice)=decode::<Notice>(packet.d.get().as_bytes()) { emit(Event::CommunityChanged {guild:notice.guild_id, channel:notice.channel_id, stage_ended:packet.t.as_deref()==Some("STAGE_INSTANCE_DELETE")})?; }
									}
									"CALL_CREATE" | "CALL_UPDATE" | "CALL_DELETE" | "CHANNEL_INFO" | "VOICE_CHANNEL_START_TIME_UPDATE" | "VOICE_CHANNEL_STATUS_UPDATE" | "VOICE_STATE_UPDATE" | "VOICE_SERVER_UPDATE" | "STREAM_CREATE" | "STREAM_SERVER_UPDATE" | "STREAM_DELETE" => calls.dispatch(packet.t.as_deref().unwrap_or(""),packet.d.get().as_bytes(),owner_id,&emit)?,
									"THREAD_MEMBER_LIST_UPDATE" => {
										if let Some(active) = &mut active_members {
											match active.thread_update(packet.d.get().as_bytes()) {
												Ok(true) => { emit(Event::Members(active.snapshot(Freshness::Fresh)))?; members_deadline = None; }
												Ok(false) => {}
												Err(_) => { active.clear_presence(); active.slots.clear(); active.synced = false; emit(Event::Members(active.snapshot(Freshness::Unavailable)))?; members_deadline = None; }
											}
										}
									}
									"GUILD_MEMBER_LIST_UPDATE" => {
										if let Some(active)=&mut active_members && !active.subscription.thread {
											let decoded = decode::<MemberUpdate>(packet.d.get().as_bytes());
											if decoded.is_err() {
												#[derive(serde::Deserialize)]
												struct Scope { guild_id: Id, id: String }
												if let Ok(scope) = decode::<Scope>(packet.d.get().as_bytes())
													&& (scope.guild_id != active.subscription.guild || scope.id != active.list_id()) { continue; }
											}
											if let Ok(update) = &decoded && active.adopt_list(update, &retired_lists) {
												member_diagnostics.record("reply list identity differs from the computed one; following the service");
											}
											if let Ok(update) = &decoded && update.ops.iter().any(|op| match op {
												MemberOp::Sync { items, .. } => items.iter().any(|item| matches!(item, discord_protocol::MemberItem::Unreadable)),
												MemberOp::Update { item, .. } | MemberOp::Insert { item, .. } => matches!(item, discord_protocol::MemberItem::Unreadable),
												_ => false,
											}) {
												member_diagnostics.record("reply contained unreadable member rows; kept their positions");
											}
											match &decoded {
												Err(_) => member_diagnostics.record("reply decode failed: unsupported member payload"),
												Ok(update) if update.guild_id != active.subscription.guild || update.id != active.list_id() => member_diagnostics.record("reply ignored: different guild or list identity"),
												Ok(update) if update.ops.iter().any(|op| matches!(op, MemberOp::Sync {items, ..} if !items.is_empty())) => member_diagnostics.record("reply: populated SYNC received"),
												Ok(update) if update.ops.iter().any(|op| matches!(op, MemberOp::Sync {items, ..} if items.is_empty())) => member_diagnostics.record("reply: empty SYNC received"),
												Ok(_) => member_diagnostics.record("reply: incremental operations only; no SYNC"),
											}
											match decoded.map_err(|_|Failure::Protocol).and_then(|update|active.update(update)) {
												Ok(true)=>{member_diagnostics.record(if active.synced {"snapshot synchronized"} else {"snapshot still awaiting populated SYNC"});let freshness=if active.synced && !active.awaiting_sync {Freshness::Fresh}else{Freshness::Loading};emit(Event::Members(active.snapshot(freshness)))?;if active.awaiting_sync {members_deadline.get_or_insert(Instant::now()+Duration::from_secs(15));} else {members_deadline=None;}},
												Ok(false)=>{},
												Err(_)=>{member_diagnostics.record("member update rejected; retaining last valid snapshot and scheduling retry");active.awaiting_sync=true;let freshness=if active.synced {Freshness::Stale}else{Freshness::Loading};emit(Event::Members(active.snapshot(freshness)))?;members_deadline.get_or_insert(Instant::now()+Duration::from_secs(15));}
											}
										}
									}
									"PRESENCE_UPDATE" => {
										if let Ok(update)=discord_protocol::presence::decode(packet.d.get().as_bytes()) {
											if update.guild.is_none() {
													if ready_at.is_some() { direct_presence.push(update,Instant::now(),emit)?; }
												else { direct_presence.push(update,Instant::now(),|event| defer_replayed_presence(&mut replayed_presence,event))?; }
											}
											else if let Some(active)=&mut active_members && active.synced { active.push_presence(update,Instant::now(),&emit)?; }
										}
									}
									"TYPING_START" => {
										// Malformed ephemeral signals must not interrupt message delivery.
										if let Ok(typing)=discord_protocol::typing::decode(packet.d.get().as_bytes()) {
											emit(Event::Typing(client_core::typing::Signal { channel:typing.channel_id,user:typing.user_id,timestamp:typing.timestamp, member:typing.member }))?;
										}
									}
									"CHANNEL_RECIPIENT_ADD" => {let d:RecipientAdded=decode(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;emit(Event::RecipientAdded {channel:d.channel_id,user:d.user.into_model()})?;}
									"CHANNEL_RECIPIENT_REMOVE" => {let d:RecipientRemoved=decode(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;if owner_id==Some(d.user.id) {calls.allowed.remove(&d.channel_id);}emit(Event::RecipientRemoved {channel:d.channel_id,user:d.user.id})?;}
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
										let ignored = (packet.t.as_deref() != Some("RELATIONSHIP_REMOVE") && (relationship.kind == 2 || relationship.user_ignored)).then_some(relationship.user_ignored && relationship.kind != 2);
										let incoming = (packet.t.as_deref() != Some("RELATIONSHIP_REMOVE") && matches!(relationship.kind,3|4)).then_some(relationship.kind==3);
										emit(Event::UserAction(client_core::user_actions::Event::Friend { user: relationship.id, friend, profile: profile.clone() }))?;
										if friend && let Some(since) = relationship.since { emit(Event::UserAction(client_core::user_actions::Event::FriendsSince { entries: vec![(relationship.id, since)], replace: false }))?; }
										emit(Event::UserAction(client_core::user_actions::Event::Restriction { user: relationship.id, ignored, profile: profile.clone() }))?;
										emit(Event::UserAction(client_core::user_actions::Event::Request { user: relationship.id, incoming, profile }))?;
										emit(Event::UserAction(client_core::user_actions::Event::RequestSpam { user: relationship.id, spam: packet.t.as_deref() != Some("RELATIONSHIP_REMOVE") && relationship.kind == 3 && relationship.is_spam_request }))?;
										if friend { match relationship.nickname {
											model::Patch::Absent => {},
											model::Patch::Null => emit(Event::UserAction(client_core::user_actions::Event::Nickname { user: relationship.id, text: String::new() }))?,
											model::Patch::Value(text) => emit(Event::UserAction(client_core::user_actions::Event::Nickname { user: relationship.id, text }))?,
										} }
									}
									"USER_UPDATE" => {
										let user: discord_protocol::UserDto = decode(packet.d.get().as_bytes()).map_err(|_| Failure::ProtocolAt("Invalid user update"))?;
										emit(Event::StickerEntitlement { user: user.id, premium_type: user.premium_type.clone() })?;
										let profile = discord_protocol::relationships::friend(user).map_err(|_| Failure::ProtocolAt("Invalid user update"))?;
										emit(Event::UserAction(client_core::user_actions::Event::FriendProfile(profile)))?;
									}
									"USER_GUILD_SETTINGS_UPDATE" => {
										if let Ok(setting)=decode::<discord_protocol::notifications::Setting>(packet.d.get().as_bytes()) {emit(notification_settings(vec![setting],false))?;}
										else {emit(Event::NotificationPreferences(client_core::notifications::Event::Invalidate))?;gateway_diagnostics.record("notification metadata unavailable; refresh required");}
									}
									"SESSIONS_REPLACE" => {
										if activity_enabled { outgoing_activity.observe(packet.d.get().as_bytes(), state.session.as_deref().map(String::as_str).unwrap_or_default()); }
										let sessions=decode::<discord_protocol::notifications::Sessions>(packet.d.get().as_bytes()).ok();
										emit(Event::NotificationPreferences(client_core::notifications::Event::Presence(sessions.and_then(|s|s.dnd()))))?;
									}
									// Status, appearance and server folders live here. Channel and guild
									// mutes live on user guild settings, so this event must not clear them.
									"USER_SETTINGS_PROTO_UPDATE" => {
										let frecency = discord_protocol::settings_update::decode_frecency(packet.d.get().as_bytes());
										if frecency.as_ref().map_or(true, |touched| touched.reactions) {
											emit(Event::ReactionPreferencesChanged)?;
										}
										if frecency.is_ok_and(|touched| touched.favorites) {
											emit(Event::GifFavoritesChanged)?;
										}
										if let Some(touched) = discord_protocol::settings_update::decode(packet.d.get().as_bytes()).unwrap_or(Some(discord_protocol::settings_update::Touched { status: true, folders: true }))
											&& (touched.status || touched.folders)
										{
											emit(Event::AccountSettings { status: touched.status, folders: touched.folders })?;
										}
									}
									"INTERACTION_SUCCESS" | "INTERACTION_FAILURE" | "INTERACTION_MODAL_CREATE" | "APPLICATION_COMMAND_AUTOCOMPLETE_RESPONSE" => { if let Some(event) = interactions::event(packet.t.as_deref().unwrap_or_default(),packet.d.get().as_bytes())? { emit(event)?; } },
									"MESSAGE_CREATE" => {
										let message = decode::<MessageDto>(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?.into_model();
										if message.ephemeral { emit(Event::Interaction(client_core::interactions::Event::Ephemeral(Box::new(message))))?; }
										else { emit(Event::Message(message))?; }
									},
									"MESSAGE_ACK" => {
										let ack=decode::<read_state::Ack>(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										emit(Event::ReadState(client_core::read_state::Event::Ack{channel:ack.channel_id,message:ack.message_id,manual:ack.manual,mention_count:ack.mention_count,version:ack.version}))?;
									}
									"PASSIVE_UPDATE_V2" => {
										let envelope = ready::passive(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										if let Some(owner)=owner_id && let Some((guild,roles,timeout_until))=envelope.permissions(owner).map_err(|_|Failure::ProtocolAt(PERMISSION_REFRESH))? {
											emit(Event::Permissions(client_core::permissions::Event::Member {guild,roles,timeout_until}))?;
										}
										for update in envelope.member_identities() {
											if let Some(active) = &mut active_members { active.identity(&update); }
											emit(Event::MemberIdentity(Box::new(update)))?;
										}
										let mut update = envelope.voice().map_err(|_|Failure::Protocol)?;
										let latest = std::mem::take(&mut update.updated_channels);
										calls.passive(update,owner_id,&emit)?;
										emit(Event::ReadState(client_core::read_state::Event::Latest(latest.into_iter().map(|c|(c.id,c.last_message_id)).collect())))?;
									}
									"MESSAGE_POLL_VOTE_ADD" | "MESSAGE_POLL_VOTE_REMOVE" => {
										let vote: discord_protocol::polls::Vote = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?;
										if vote.channel_id.0 == 0 || vote.message_id.0 == 0 || vote.user_id.0 == 0 || vote.answer_id == 0 { return Err(Failure::Protocol); }
										if packet.s.is_some() { emit(Event::Polls(client_core::polls::Event::Vote { channel: vote.channel_id, message: vote.message_id,
											user: vote.user_id, answer: vote.answer_id, add: packet.t.as_deref() == Some("MESSAGE_POLL_VOTE_ADD") }))?; }
									}
									"MESSAGE_UPDATE" => emit(Event::Patch(decode::<PatchDto>(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?.into_model()))?,
									"MESSAGE_REACTION_ADD" | "MESSAGE_REACTION_REMOVE" | "MESSAGE_REACTION_REMOVE_ALL" | "MESSAGE_REACTION_REMOVE_EMOJI" => {
										emit(Event::Reactions(reaction_event(packet.t.as_deref().unwrap_or(""), packet.d.get().as_bytes(), packet.s.is_some())?))?;
									}
									"RECENT_MENTION_DELETE" | "SAVED_MESSAGE_CREATE" | "SAVED_MESSAGE_UPDATE" | "SAVED_MESSAGE_DELETE" => { emit(Event::InboxInvalidated)?; }
									"MESSAGE_DELETE" => { let d: Deleted = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?; emit(Event::Delete { channel:d.channel_id, id:d.id })?; }
									"MESSAGE_DELETE_BULK" => { let d: BulkDeleted = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?; if d.ids.len() > 100 { return Err(Failure::Capacity); } emit(Event::DeleteBulk { channel:d.channel_id, ids: d.ids })?; }
									// Only the auth-session hash changed; revocation arrives as HTTP 401 or close 4004.
									"AUTH_SESSION_CHANGE" => {}
									"CHANNEL_DELETE" => { let c: ChannelDto = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?; inbox.forget(c.id); calls.invalidate(c.id); emit(Event::Unavailable(c.id))?; }
									"CHANNEL_CREATE" => {
										let permissions=owner_id.map(|owner|channel_events::permission_metadata(packet.d.get().as_bytes(),owner)).transpose()?.flatten();
										let (event, request, spam)=channel_events::create(packet.d.get().as_bytes(),&mut inbox)?;
										match &event {
											Event::ChannelCreated(c) => channel_events::admit_call(c,&known_guilds,&mut calls),
											Event::Unavailable(id) => calls.invalidate(*id),
											_=>{}
										}
										emit(event)?;
										if let Some(channel) = request {
											emit(Event::UserAction(client_core::user_actions::Event::MessageRequest { channel, pending: true }))?;
										}
										if let Some(channel) = spam {
											emit(Event::UserAction(client_core::user_actions::Event::MessageSpam { channel, spam: true }))?;
										}
										if let Some(permissions)=permissions {emit(permissions)?;}
									}
									"CHANNEL_UPDATE" => {
										let permissions=owner_id.map(|owner|channel_events::permission_metadata(packet.d.get().as_bytes(),owner)).transpose()?.flatten();
										let update=channel_events::update(packet.d.get().as_bytes(),&mut inbox)?;
										if let Some(channel)=update.restored {channel_events::admit_call(&channel,&known_guilds,&mut calls);emit(Event::ChannelRestored(channel))?;}
										if let Event::Unavailable(id)=&update.event {calls.invalidate(*id);}
										if let Event::ChannelChanged(patch)=&update.event && let model::Patch::Value(kind)=patch.kind && !matches!(kind,1..=3) {calls.invalidate(patch.id);}
										emit(update.event)?;
										if let Some((channel, pending)) = update.message_request {
											emit(Event::UserAction(client_core::user_actions::Event::MessageRequest { channel, pending }))?;
										}
										if let Some((channel, spam)) = update.spam_direct {
											emit(Event::UserAction(client_core::user_actions::Event::MessageSpam { channel, spam }))?;
										}
										if let Some(permissions)=permissions {emit(permissions)?;}
									}
									"THREAD_CREATE" | "THREAD_UPDATE" | "THREAD_DELETE" | "THREAD_LIST_SYNC" | "THREAD_MEMBERS_UPDATE" => {
										if let Some(event) = thread_events::decode_event(packet.t.as_deref().unwrap_or(""), packet.d.get().as_bytes(), owner_id)? { emit(event)?; }
									}
									"GUILD_STICKERS_UPDATE" => {
										let update=decode::<discord_protocol::stickers::GuildStickersUpdate>(packet.d.get().as_bytes()).and_then(|u|discord_protocol::stickers::guild_catalog(u.stickers.0,u.guild_id).map(|stickers|(u.guild_id,stickers)));
										if let Ok((guild,stickers))=update {emit(Event::GuildStickers{guild,stickers})?;}
										else {if let Some(guild)=catalog_scope(packet.d.get().as_bytes()){emit(Event::GuildStickers{guild,stickers:Vec::new()})?;}gateway_diagnostics.record("sticker catalog unavailable; invalidated");}
									}
									"GUILD_EMOJIS_UPDATE" => {
										if let Ok(update)=decode::<GuildEmojisUpdate>(packet.d.get().as_bytes()) {emit(Event::GuildEmojis { guild:update.guild_id,emojis:update.emojis.0})?;}
										else {if let Some(guild)=catalog_scope(packet.d.get().as_bytes()){emit(Event::GuildEmojis{guild,emojis:Vec::new()})?;}gateway_diagnostics.record("emoji catalog unavailable; invalidated");}
									}
									// Best-effort playback metadata: an unsupported effect never ends the session.
									"VOICE_CHANNEL_EFFECT_SEND" => {
										if let Ok(Some(effect)) = discord_protocol::soundboard::effect(packet.d.get().as_bytes()) {
											emit(Event::Soundboard(client_core::soundboard::Event::Effect { channel: effect.channel, user: effect.user, sound: effect.sound, volume: effect.volume }))?;
										}
									}
									"GUILD_SOUNDBOARD_SOUND_CREATE" | "GUILD_SOUNDBOARD_SOUND_UPDATE" | "GUILD_SOUNDBOARD_SOUND_DELETE" | "GUILD_SOUNDBOARD_SOUNDS_UPDATE" => {
										if let Ok(guild) = discord_protocol::soundboard::changed_guild(packet.d.get().as_bytes()) {
											emit(Event::Soundboard(client_core::soundboard::Event::Changed(guild)))?;
										}
									}
									"GUILD_CREATE" => {
										member_diagnostics.record("guild refresh received");
										let permissions=owner_id.map(|owner|permissions::guild(packet.d.get().as_bytes(),owner)).transpose().map_err(|_|Failure::ProtocolAt(PERMISSION_REFRESH))?;
										let mut guild: GuildDto = decode(packet.d.get().as_bytes()).map_err(|_| Failure::ProtocolAt("Gateway guild refresh: unsupported guild payload"))?;
										let display = guild.display_patch();
										if guild.channels.len() + calls.allowed.len() > MAX_NAV { return Err(Failure::Capacity); }
										if !known_guilds.contains(&guild.id) {
											if known_guilds.len() >= MAX_NAV { return Err(Failure::Capacity); }
											known_guilds.insert(guild.id);
											let name = match &display.name { model::Patch::Value(name) => name.chars().take(128).collect(), _ => String::new() };
											let icon = match &display.icon { model::Patch::Value(icon) if model::valid_avatar_hash(icon) => Some(icon.clone()), _ => None };
											emit(Event::GuildJoined(model::Guild { default_message_notifications: guild.default_notification_level(), id: guild.id, name, icon, stickers: None, emojis: None }))?;
										}
										emit(Event::GuildChanged(display))?;
										if let model::Patch::Value(incident)=guild.incident_patch(){emit(Event::ServerIncident{guild:guild.id,incident:Some(incident)})?;}

										if let Some(permissions)=permissions {emit(Event::Permissions(client_core::permissions::Event::Snapshot(permissions)))?;}
										if let Some(gate)=owner_id.and_then(|owner|onboarding::guild(packet.d.get().as_bytes(),owner)) {emit(Event::Onboarding(client_core::onboarding::Event::Gates { snapshot: false, gates: vec![gate] }))?;}
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
										if let Some(stickers) = guild.stickers { emit(Event::GuildStickers { guild: guild.id, stickers: discord_protocol::stickers::guild_catalog(stickers.0, guild.id).map_err(|_| Failure::Protocol)? })?; }
										if let Some(emojis) = guild.emojis { emit(Event::GuildEmojis { guild: guild.id, emojis: emojis.0 })?; }
									}
									"GUILD_JOIN_REQUEST_CREATE"|"GUILD_JOIN_REQUEST_UPDATE"|"GUILD_JOIN_REQUEST_DELETE"=>{
										let deleted=packet.t.as_deref()==Some("GUILD_JOIN_REQUEST_DELETE");
										if let Ok(Some((guild,request)))=discord_protocol::server_access::application_update(packet.d.get().as_bytes(),deleted){emit(Event::GuildApplicationChanged{guild,request,deleted})?;}
									}
									"GUILD_UPDATE" => {
										if let Ok(Some((guild,incident)))=discord_protocol::server_access::incident_update(packet.d.get().as_bytes()){emit(Event::ServerIncident{guild,incident})?;}
										let owner=permissions::owner(packet.d.get().as_bytes()).map_err(|_|Failure::ProtocolAt(PERMISSION_REFRESH))?;
										emit(Event::GuildChanged(decode::<GuildPatchDto>(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?.into_model()))?;
										if let Some((guild,owner))=owner {emit(Event::Permissions(client_core::permissions::Event::Owner {guild,owner}))?;}
										if let Some(gate)=owner_id.and_then(|owner|onboarding::guild(packet.d.get().as_bytes(),owner)) {emit(Event::Onboarding(client_core::onboarding::Event::Gates { snapshot: false, gates: vec![gate] }))?;}
									}
									"GUILD_BULK_BAN_UPDATE" => {
										let (guild,report)=discord_protocol::moderation::bulk_update(packet.d.get().as_bytes()).map_err(|_|Failure::Protocol)?;
										emit(Event::BulkModeration {guild,report})?;
									}
									"GUILD_MEMBER_UPDATE" => {
										if let Ok(update) = discord_protocol::member_identity::update(packet.d.get().as_bytes()) {
											if let Some(active) = &mut active_members { active.identity(&update); }
											emit(Event::MemberIdentity(Box::new(update)))?;
										}
										if let Some(owner)=owner_id && let Some((guild,roles,timeout_until))=permissions::member(packet.d.get().as_bytes(),owner).map_err(|_|Failure::ProtocolAt(PERMISSION_REFRESH))? {
											emit(Event::Permissions(client_core::permissions::Event::Member {guild,roles,timeout_until}))?;
										}
										if let Some(gate)=owner_id.and_then(|owner|onboarding::member(packet.d.get().as_bytes(),owner)) {emit(Event::Onboarding(client_core::onboarding::Event::Gates { snapshot: false, gates: vec![gate] }))?;}
									}
									"GUILD_MEMBER_REMOVE" => {
										if let Ok(update) = discord_protocol::member_identity::removed(packet.d.get().as_bytes()) {
											if let Some(active) = &mut active_members { active.identity(&update); }
											emit(Event::MemberIdentity(Box::new(update)))?;
										}
									}
									"GUILD_DELETE" => {
										#[derive(serde::Deserialize)]
										struct DeletedGuild {
											id: Id,
											#[serde(default)]
											unavailable: Option<bool>,
										}
										let guild: DeletedGuild = decode(packet.d.get().as_bytes()).map_err(|_| Failure::Protocol)?;
										let removed: Vec<_> = calls.allowed.iter().filter_map(|(channel, id)| (*id == Some(guild.id)).then_some(*channel)).collect();
										for channel in removed { calls.invalidate(channel); emit(Event::Unavailable(channel))?; }
										emit(Event::GuildStickers { guild: guild.id, stickers: Vec::new() })?;
										emit(Event::GuildEmojis { guild: guild.id, emojis: Vec::new() })?;
										emit(Event::Permissions(client_core::permissions::Event::UnavailableGuild(guild.id)))?;
										if guild.unavailable != Some(true) {
											known_guilds.remove(&guild.id);
											emit(Event::GuildRemoved(guild.id))?;
										}
									}
									"GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
										let (guild,role)=permissions::role(packet.d.get().as_bytes()).map_err(|_|Failure::ProtocolAt(PERMISSION_REFRESH))?;
										emit(Event::Permissions(client_core::permissions::Event::Role {guild,role}))?;
									}
									"GUILD_ROLE_DELETE" => {
										let (guild,id)=permissions::role_removed(packet.d.get().as_bytes()).map_err(|_|Failure::ProtocolAt(PERMISSION_REFRESH))?;
										emit(Event::Permissions(client_core::permissions::Event::RoleRemoved {guild,id}))?;
									}
									_ => gateway_diagnostics.record(ignored_dispatch_label(packet.t.as_deref())),
								},
								_ => {
									gateway_diagnostics.record("unsupported opcode; connection stopped");
									return Err(Failure::Protocol);
								},
							}
							if let Some(sequence)=admitted_sequence {state.sequence=Some(sequence); }
						}
						Some(Ok(Frame::Close(close))) => {
							let code = close.map_or(1006, |f| u16::from(f.code));
							record(if code==4008 {ConnectionReason::RateLimited}else{ConnectionReason::ServerClose},Some(code),attempt,writer.occupancy());
							if code==4008 {limits.lock().map_err(|_|Failure::Network)?.connection_until=Some(Instant::now()+Duration::from_secs(60));}
							state.close(code)?;
							break;
						}
						Some(Ok(Frame::Ping(_))) => { /* Tungstenite queues and flushes its automatic Pong on the next read. */ }
						Some(Ok(Frame::Pong(_))) => {},
						Some(Ok(Frame::Binary(_))) => return Err(Failure::Protocol),
						_ => {record(ConnectionReason::SocketRead,None,attempt,writer.occupancy());break;},
					}
				}
			}
		}
		let progress = *writer.progress.borrow();
		if progress.failed {
			record(
				if progress.capacity {
					ConnectionReason::LocalCapacity
				} else {
					ConnectionReason::SocketWrite
				},
				None,
				attempt,
				writer.occupancy(),
			);
		}
		if ready_at.is_none() && !skip_backoff {
			state.failed();
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
	client_core::notifications::Event::Updates {
		entries: entries
			.into_iter()
			.map(|setting| setting.into_update(true))
			.collect(),
		replace,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn replayed_presence_buffer_requests_recovery_at_its_bound() {
		let mut pending = Vec::new();
		for _ in 0..REPLAYED_PRESENCE_BATCHES {
			defer_replayed_presence(&mut pending, Event::DirectPresence(vec![])).unwrap();
		}
		assert_eq!(
			defer_replayed_presence(&mut pending, Event::DirectPresence(vec![])),
			Err(Failure::Busy)
		);
		assert_eq!(pending.len(), REPLAYED_PRESENCE_BATCHES);
	}
	#[test]
	fn notification_settings_forward_explicit_mention_suppression() {
		use model::Patch;
		let setting=decode::<discord_protocol::notifications::Setting>(br#"{"guild_id":"1","muted":false,"message_notifications":1,"channel_overrides":[{"channel_id":"3","muted":true,"mute_config":{"end_time":"2020-01-01T00:00:00Z"}}],"suppress_everyone":false,"suppress_roles":true}"#).unwrap();
		let Event::NotificationPreferences(client_core::notifications::Event::Updates {
			entries,
			replace,
		}) = notification_settings(vec![setting], false)
		else {
			panic!()
		};
		assert!(!replace);
		assert_eq!(entries.len(), 1);
		assert!(entries[0].full);
		assert_eq!(
			(
				entries[0].fields.suppress_everyone.clone(),
				entries[0].fields.suppress_roles.clone()
			),
			(Patch::Value(false), Patch::Value(true))
		);
		assert_eq!(entries[0].guild, Some(model::Id(1)));
		let row = &entries[0].channels.as_ref().unwrap()[0];
		assert_eq!(row.channel, model::Id(3));
		assert_eq!(row.fields.mute_until, Patch::Value(1577836800));
	}
	#[test]
	fn guild_mute_without_channel_overrides_and_full_defaults_are_retained() {
		use model::Patch;
		let setting = decode::<discord_protocol::notifications::Setting>(
			br#"{"guild_id":"1","muted":true,"mute_config":{"end_time":"2020-01-01T00:00:00Z"}}"#,
		)
		.unwrap();
		let client_core::notifications::Event::Updates { entries, .. } =
			notification_preferences(vec![setting], false)
		else {
			panic!()
		};
		assert_eq!(entries[0].fields.muted, Patch::Value(true));
		assert_eq!(entries[0].fields.mute_until, Patch::Value(1577836800));
		assert_eq!(entries[0].fields.level, Patch::Value(3));
		assert_eq!(entries[0].fields.suppress_roles, Patch::Value(false));
		assert!(entries[0].channels.as_ref().unwrap().is_empty());
	}
	#[test]
	fn full_channel_overrides_default_omissions_but_keep_null_unknown() {
		use model::Patch;
		let setting=decode::<discord_protocol::notifications::Setting>(br#"{"guild_id":"1","message_notifications":1,"channel_overrides":[{"channel_id":"3","message_notifications":0},{"channel_id":"4","muted":null,"message_notifications":0}]}"#).unwrap();
		let client_core::notifications::Event::Updates { entries, .. } =
			notification_preferences(vec![setting], false)
		else {
			panic!()
		};
		let rows = entries[0].channels.as_ref().unwrap();
		assert_eq!(rows.len(), 2);
		assert_eq!(rows[0].fields.muted, Patch::Value(false));
		assert_eq!(rows[1].fields.muted, Patch::Null);
		assert_eq!(rows[0].fields.level, Patch::Value(0));
		assert_eq!(rows[1].fields.level, Patch::Value(0));
	}

	#[test]
	fn decoded_preferences_reach_core_and_a_sparse_account_command() {
		use model::{Id, Patch};
		let mut state = client_core::State {
			demo: true,
			guilds: vec![model::Guild {
				id: Id(2),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
				stickers: None,
				default_message_notifications: Some(0),
			}],
			..Default::default()
		};
		let record=decode::<discord_protocol::notifications::Setting>(br#"{"guild_id":"2","flags":36028797018980352,"hide_muted_channels":true,"channel_overrides":[]}"#).unwrap();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: notification_settings(vec![record], false),
		});
		assert!(state.channel_preferences_known(Id(2)));
		assert!(!state.personally_shows_all_channels(Id(2)));
		assert_eq!(state.hides_muted_channels(Id(2)), Some(true));
		state.update_server_notifications(
			Id(2),
			client_core::server_actions::NotificationOptions {
				level: Some(1),
				..Default::default()
			},
		);
		state.flush_channel_preferences();
		let Some(client_core::Command::ChannelPreferences(request)) =
			state.poll_channel_preferences(std::time::Instant::now())
		else {
			panic!()
		};
		assert_eq!(request.patch.fields.level, Patch::Value(1));
		assert_eq!(request.patch.fields.flags, Patch::Absent);
		assert_eq!(request.patch.fields.hide_muted, Patch::Absent);
		let result = state.demo_channel_preferences(&request);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: Event::ChannelPreferences(result),
		});
		assert_eq!(state.server_notification_settings(Id(2)).level, Some(1));
		assert_eq!(state.hides_muted_channels(Id(2)), Some(true));
	}

	use serde_json::{Value, json};
	use tokio::net::{TcpListener, TcpStream};
	use tokio_tungstenite::{
		WebSocketStream, accept_async,
		tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
	};

	pub(super) async fn packet(socket: &mut WebSocketStream<TcpStream>) -> Value {
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
	pub(super) async fn send(socket: &mut WebSocketStream<TcpStream>, value: Value) {
		socket
			.send(Frame::Text(value.to_string().into()))
			.await
			.unwrap();
	}
	pub(super) async fn acknowledge(socket: &mut WebSocketStream<TcpStream>, sequence: u64) {
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
	pub(super) fn ready(sequence: u64, session: &str) -> Value {
		json!({"op":0,"t":"READY","s":sequence,"d":{
			"user":{"id":"1","username":"synthetic"}, "session_id":session,
			"resume_gateway_url":"wss://gateway.discord.gg/", "guilds":[], "private_channels":[]
		}})
	}

	#[tokio::test]
	async fn favorite_settings_notifications_reach_core_without_invalidating_unrelated_updates() {
		timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let (client_finished, terminal_observed) = tokio::sync::oneshot::channel();
			let state = Mutex::new(client_core::State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				..Default::default()
			});
			{
				let mut state = state.lock().unwrap();
				let Some(client_core::Command::GifFavorites { request, .. }) =
					state.request_gif_favorites_if_needed()
				else {
					panic!("initial favorites read");
				};
				state.apply_gif_favorites(request, Ok(vec![]));
			}
			let favorites_changed = Mutex::new(0);
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(
					&mut socket,
					json!({"op":10,"d":{"heartbeat_interval":1000}}),
				)
				.await;
				assert_eq!(packet(&mut socket).await["op"], 2);
				send(&mut socket, ready(1, "synthetic-favorite-settings")).await;
				for (sequence, update) in (2_u64..).zip([
					json!({"settings":{"type":2,"proto":"agA="},"partial":true}),
					json!({"settings":{"type":1,"proto":"EgA="},"partial":true}),
					json!({"settings":{"type":2,"proto":"!"},"partial":false}),
					json!({"settings":{"type":2,"proto":"EgA="},"partial":true}),
					json!({"settings":{"type":2,"proto":""},"partial":false}),
				]) {
					send(
						&mut socket,
						json!({"op":0,"t":"USER_SETTINGS_PROTO_UPDATE","s":sequence,"d":update}),
					)
					.await;
				}
				socket
					.send(Frame::Close(Some(CloseFrame {
						code: CloseCode::Library(4004),
						reason: "synthetic stop".into(),
					})))
					.await
					.unwrap();
				terminal_observed.await.unwrap();
			};
			let client = async {
				let result = run_inner(
					Arc::new(
						SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap(),
					),
					"wss://gateway.discord.gg/".into(),
					watch::channel(None).1,
					mpsc::channel(1).1,
					None,
					|event| {
						if !matches!(&event, Event::GifFavoritesChanged) {
							return Ok(());
						}
						let mut state = state.lock().unwrap();
						let generation = state.generation;
						state.apply(client_core::Envelope { generation, event });
						assert!(state.gifs.sync_ready);
						let Some(client_core::Command::GifFavorites {
							request,
							change: None,
						}) = state.request_gif_favorites_if_needed()
						else {
							panic!("remote update must refresh an already loaded list");
						};
						state.apply_gif_favorites(request, Ok(vec![]));
						assert!(state.gifs.sync_ready);
						assert!(state.request_gif_favorites_if_needed().is_none());
						*favorites_changed.lock().unwrap() += 1;
						Ok(())
					},
					Some(&endpoint),
				)
				.await;
				let _ = client_finished.send(());
				result
			};
			let ((), result) = tokio::join!(server, client);
			assert_eq!(result, Err(Failure::Expired));
			assert_eq!(*favorites_changed.lock().unwrap(), 2);
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn replayed_presence_bursts_wait_for_resumed_before_delivery() {
		timeout(Duration::from_secs(15), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let (completed, done) = tokio::sync::oneshot::channel();
			let (client_finished, terminal_observed) = tokio::sync::oneshot::channel();
			let observed = std::sync::Mutex::new((false, BTreeSet::new(), Some(completed)));
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
				assert_eq!(packet(&mut socket).await["op"], 2);
				send(&mut socket, ready(1, "synthetic-replayed-presence")).await;
				send(&mut socket, json!({"op":7,"d":null})).await;
				drop(socket);
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
				let resume = packet(&mut socket).await;
				assert_eq!(resume["op"], 6);
				assert_eq!(resume["d"]["seq"], 1);
				for (sequence, user) in (2_u64..).zip(10..160) {
					send(&mut socket, json!({"op":0,"t":"PRESENCE_UPDATE","s":sequence,"d":{"user":{"id":user.to_string()},"status":"online"}})).await;
				}
				send(&mut socket, json!({"op":0,"t":"RESUMED","s":152,"d":{}})).await;
				done.await.unwrap();
				socket.send(Frame::Close(Some(CloseFrame { code: CloseCode::Library(4004), reason: "synthetic stop".into() }))).await.unwrap();
				terminal_observed.await.unwrap();
			};
			let client = run_inner(
				Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),
				"wss://gateway.discord.gg/".into(), watch::channel(None).1, mpsc::channel(1).1, None,
				|event| {
					let mut seen = observed.lock().unwrap();
					match event {
						Event::Resumed => seen.0 = true,
						Event::DirectPresence(updates) => {
							assert!(seen.0, "direct presence must not reach the disconnected reducer");
							assert!(updates.len() <= 100);
							seen.1.extend(updates.into_iter().map(|u| u.user));
						}
						_ => {}
					}
					if seen.1.len() == 150 && let Some(done) = seen.2.take() { let _ = done.send(()); }
					Ok(())
				}, Some(&endpoint),
			);
			let client = async { let result = client.await; let _ = client_finished.send(()); result };
			let ((), result) = tokio::join!(server, client);
			assert_eq!(result, Err(Failure::Expired));
			let seen = observed.into_inner().unwrap();
			assert!(seen.0);
			assert_eq!(seen.1, (10..160).map(Id).collect());
		}).await.unwrap();
	}

	#[tokio::test]
	async fn permanent_guild_departure_allows_same_guild_to_join_again() {
		timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let joined = std::sync::Mutex::new(Vec::new());
			let removed = std::sync::Mutex::new(Vec::new());
			let changed = std::sync::Mutex::new(Vec::new());
			let startup_display = std::sync::Mutex::new(Vec::new());
			let (client_finished, terminal_observed) = tokio::sync::oneshot::channel();
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(
					&mut socket,
					json!({"op":10,"d":{"heartbeat_interval":1000}}),
				)
				.await;
				assert_eq!(packet(&mut socket).await["op"], 2);
				let mut initial = ready(1, "synthetic-guild-rejoin");
				initial["d"]["guilds"] = json!([{"id":"2","name":"Original"},{"id":"2","name":"Duplicate"},{"id":"0","name":"Invalid"}]);
				send(&mut socket, initial).await;
				for (sequence, name, data) in [
					(2, "GUILD_DELETE", json!({"id":"2","unavailable":true})),
					(
						3,
						"GUILD_CREATE",
						json!({"id":"2","name":"Available again"}),
					),
					(4, "GUILD_DELETE", json!({"id":"2"})),
					(5, "GUILD_CREATE", json!({"id":"2","name":"Rejoined","icon":"0123456789abcdef0123456789abcdef","properties":{"icon":null}})),
					(6, "GUILD_CREATE", json!({"id":"2"})),
				] {
					send(&mut socket, json!({"op":0,"t":name,"s":sequence,"d":data})).await;
				}
				acknowledge(&mut socket, 6).await;
				socket
					.send(Frame::Close(Some(CloseFrame {
						code: CloseCode::from(4004),
						reason: "synthetic expiration".into(),
					})))
					.await
					.unwrap();
				terminal_observed.await.unwrap();
			};
			let client = run_inner(
				Arc::new(SessionSecret::from_owner_input("SYNTHETIC_GUILD_TOKEN".into()).unwrap()),
				"wss://gateway.discord.gg/".into(),
				watch::channel(None).1,
				mpsc::channel(1).1,
				None,
				|event| {
					match event {
						Event::GuildJoined(guild) => {
							joined.lock().unwrap().push((guild.id, guild.name, guild.icon))
						}
						Event::GuildRemoved(guild) => removed.lock().unwrap().push(guild),
						Event::GuildChanged(patch) => changed.lock().unwrap().push((patch.name,patch.icon)),
						Event::GuildDisplayMetadata(patches) => startup_display.lock().unwrap().extend(patches.into_iter().map(|patch| (patch.id,patch.name,patch.icon))),
						_ => {}
					}
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
			assert_eq!(
				joined.into_inner().unwrap(),
				vec![(Id(2), "Rejoined".into(), None)]
			);
			assert_eq!(removed.into_inner().unwrap(), vec![Id(2)]);
			assert_eq!(startup_display.into_inner().unwrap(), vec![(Id(2), model::Patch::Value("Original".into()), model::Patch::Absent)]);
			assert_eq!(changed.into_inner().unwrap(), vec![
				(model::Patch::Value("Available again".into()), model::Patch::Absent),
				(model::Patch::Value("Rejoined".into()), model::Patch::Null),
				(model::Patch::Absent, model::Patch::Absent),
			]);
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn late_recovery_preserves_ready_and_resumed_sockets() {
		timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let reconnect = Notify::new();
			let events = std::sync::Mutex::new(Vec::new());
			let (client_finished, terminal_observed) = tokio::sync::oneshot::channel();
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(
					&mut socket,
					json!({"op":10,"d":{"heartbeat_interval":1000}}),
				)
				.await;
				assert_eq!(packet(&mut socket).await["op"], 2);
				send(&mut socket, ready(41, "synthetic-recovery-session")).await;
				acknowledge(&mut socket, 41).await;
				// The READY callback queues recovery while the UI would still be disconnected.
				// Keep the established socket alive while that notification is consumed.
				assert!(
					timeout(Duration::from_millis(100), listener.accept())
						.await
						.is_err()
				);
				acknowledge(&mut socket, 41).await;
				send(&mut socket, json!({"op":7,"d":null})).await;
				// Disconnected queues another recovery request. It must still skip the >=2s backoff.
				let (stream, _) = timeout(Duration::from_millis(1500), listener.accept())
					.await
					.unwrap()
					.unwrap();
				drop(socket);
				let mut socket = accept_async(stream).await.unwrap();
				send(
					&mut socket,
					json!({"op":10,"d":{"heartbeat_interval":1000}}),
				)
				.await;
				let handshake = packet(&mut socket).await;
				assert_eq!(handshake["op"], 6);
				assert_eq!(handshake["d"]["session_id"], "synthetic-recovery-session");
				assert_eq!(handshake["d"]["seq"], 41);
				send(&mut socket, json!({"op":0,"t":"RESUMED","s":42,"d":{}})).await;
				acknowledge(&mut socket, 42).await;
				// A late send/Refresh pulse after RESUMED must also preserve the healthy socket.
				assert!(
					timeout(Duration::from_millis(100), listener.accept())
						.await
						.is_err()
				);
				acknowledge(&mut socket, 42).await;
				socket
					.send(Frame::Close(Some(CloseFrame {
						code: CloseCode::from(4004),
						reason: "synthetic expiration".into(),
					})))
					.await
					.unwrap();
				// Retain TCP until the terminal close is consumed, even if a heartbeat races it.
				terminal_observed.await.unwrap();
			};
			let client = async {
				let result = run_recoverable(
					Arc::new(
						SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap(),
					),
					"wss://gateway.discord.gg/".into(),
					watch::channel(None).1,
					mpsc::channel(1).1,
					None,
					Some(&reconnect),
					|event| {
						let label = match event {
							Event::Startup(_) => "ready",
							Event::Disconnected => "disconnected",
							Event::Resumed => "resumed",
							_ => return Ok(()),
						};
						let mut events = events.lock().unwrap();
						assert!(events.len() < 4, "recovery must not cycle a healthy socket");
						events.push(label);
						reconnect.notify_one();
						Ok(())
					},
					Some((&endpoint, &endpoint)),
				)
				.await;
				let _ = client_finished.send(());
				result
			};
			let ((), result) = tokio::join!(server, client);
			assert_eq!(result, Err(Failure::Expired));
			assert_eq!(
				events.into_inner().unwrap(),
				vec!["ready", "disconnected", "resumed"]
			);
		})
		.await
		.unwrap();
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
		{
			timeout(Duration::from_secs(25), async {
	            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	            let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
	            let game = |name: &str| discord_protocol::rpc::ActivityFields::default().into_activity(Id(42), name.into()).unwrap();
	            let (activity, receiver) = watch::channel(Some(game("osu!")));
				let (own_presence, presence_receiver) = watch::channel(model::OwnPresence {
					status: model::PresenceStatus::DoNotDisturb, custom_status: "Synthetic focus".into(), expires_at_ms: None,
				});
	            let (observations, mut observed) = watch::channel(ActivityObservation::Unconfirmed);
	            let (finished, done) = tokio::sync::oneshot::channel();
	            let server = async {
	                let mut previous = None;
				let mut sequence=1;
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
	                    if connection!=0 {sequence+=1;}
						send(&mut socket, if connection == 0 {
	                        ready(1, "synthetic-own-activity")
	                    } else { json!({"op":0,"t":"RESUMED","s":sequence,"d":{}}) }).await;
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
	                        // Receipt times include scheduling delay before the previous read,
	                        // which can only shorten the observed gap below the 5 s client pacing.
	                        if let Some(previous) = previous {
	                            assert!(now.duration_since(previous) >= Duration::from_millis(4500));
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
									sequence+=1;
	                                    send(&mut socket, json!({"op":0,"t":"SESSIONS_REPLACE","s":sequence,"d":[{
	                                        "session_id":"all","status":"online",
	                                        "activities":if public {json!([game])} else {json!([])},
	                                        "hidden_activities":if hidden {json!([game])} else {json!([])}
	                                    }]})).await;
	                                    observed.wait_for(|value| *value == expected).await.unwrap();
	                                }
	                                activity.send(Some(game("Skipped intermediate"))).unwrap();
									own_presence.send_replace(model::OwnPresence {status:model::PresenceStatus::DoNotDisturb,custom_status:"On a break".into(),expires_at_ms:None});
	                                activity.send(Some(game("Minecraft"))).unwrap();
	                                observed.wait_for(|value| *value == ActivityObservation::Unconfirmed).await.unwrap();
	                            }
	                            Some(_) if connection == 1 => activity.send(None).unwrap(),
	                            Some(_) => {
								sequence+=1;
	                                send(&mut socket, json!({"op":0,"t":"SESSIONS_REPLACE","s":sequence,"d":[{
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
	                    mpsc::channel(1).1, Some(ActivityInput { member_watches: None, roster: None, spotify: watch::channel(None).1, member_queries: watch::channel(Default::default()).1, receiver, own_presence: presence_receiver, observe: &|value| { observations.send_replace(value); Ok(()) } }), |_| Ok(()), Some(&endpoint),
	                ).await;
	                finished.send(()).unwrap();
	                result
	            };
	            let ((), result) = tokio::join!(server, client);
	            assert_eq!(result, Err(Failure::Expired));
	        }).await.expect("synthetic activity lifecycle timed out");
		}
		{
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
                for (sequence, name) in [(2, Some("SYNTHETIC_PRIVATE_EVENT_NAME")), (3, None), (4, Some("")), (5, Some("AUTH_SESSION_CHANGE"))] {
                    send(&mut socket, json!({"op":0,"t":name,"s":sequence,"d":{
                        "token":"SYNTHETIC_PRIVATE_PAYLOAD", "permissions":"8", "content":"not a message"
                    }})).await;
                }
                for (sequence, data) in [
                    (6, json!({"channel_id":"2","user_id":"3","timestamp":1700000000,"member":{"user":{"username":"discarded"}}})),
                    (7, json!({"channel_id":"2","user_id":"3","timestamp":"invalid"})),
                    (8, json!({"channel_id":"2","user_id":"3","timestamp":1700000000,"member":{"padding":"x".repeat(discord_protocol::typing::MAX_WIRE)}})),
                ] {
                    send(&mut socket, json!({"op":0,"t":"TYPING_START","s":sequence,"d":data})).await;
                }
                for (sequence,name,data) in [
                    (9,"SESSIONS_REPLACE",json!([{"status":42}])),
                    (10,"USER_GUILD_SETTINGS_UPDATE",json!({"guild_id":"2","muted":42})),
                    (11,"GUILD_STICKERS_UPDATE",json!({"guild_id":"2","stickers":"unsupported"})),
                    (12,"GUILD_EMOJIS_UPDATE",json!({"guild_id":"2","emojis":"unsupported"})),
                ] {send(&mut socket,json!({"op":0,"t":name,"s":sequence,"d":data})).await;}
                send(&mut socket, json!({"op":0,"t":"MESSAGE_CREATE","s":13,"d":{"id":"4","channel_id":"2","author":{"id":"3","username":"Synthetic"},"content":"Message after invalid typing"}})).await;
                acknowledge(&mut socket, 13).await;
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
	async fn local_member_identity_dispatches_update_names_roles_and_keep_delivery() {
		for cached_watch in [false, true] {
			timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let (finished, observed) = tokio::sync::oneshot::channel();
			let finished = std::sync::Mutex::new(Some(finished));
			let (reconnect_consumed, reconnect_observed) = tokio::sync::oneshot::channel();
			let reconnect_consumed = std::sync::Mutex::new(Some(reconnect_consumed));
			let (queries, member_queries) = watch::channel(Default::default());
			let (watch_send, watch_receive) = watch::channel(None);
			let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
				assert_eq!(packet(&mut socket).await["op"], 2);
				let mut initial = ready(1, "synthetic-member-session");
				initial["d"]["guilds"] = json!([{"id":"10","name":"Synthetic guild","owner_id":"99","roles":[{"id":"10","permissions":"68608"},{"id":"11","permissions":"0","name":"Colored role","color":65280}],"members":[{"user":{"id":"1","username":"synthetic"},"roles":[]}],"channels":[{"id":"20","type":0,"name":"general","permission_overwrites":[]}]}]);
				send(&mut socket, initial).await;
				loop {
					let subscribed = packet(&mut socket).await;
					if subscribed["op"] == 1 {
						send(&mut socket, json!({"op":11,"d":null})).await;
						continue;
					}
					if subscribed["op"] == 37 {
						assert_eq!(subscribed["d"]["subscriptions"]["10"]["typing"], true);
						assert_eq!(subscribed["d"]["subscriptions"]["10"]["members"], json!(["7"]));
						break;
					}
					assert!(subscribed["op"] == 3 || (!cached_watch && subscribed["op"] == 8));
				}
				for (sequence, name, data) in [
					(2,"MESSAGE_CREATE", json!({"id":"100","channel_id":"20","guild_id":"10","author":{"id":"7","username":"account"},"member":{"nick":"Old nickname","roles":["11"]},"content":"Before change"})),
					(3,"GUILD_MEMBER_UPDATE", json!({"guild_id":"10","user":{"id":"7","username":"account"},"nick":"New nickname","roles":["11"]})),
					(4,"GUILD_MEMBER_UPDATE", json!({"guild_id":"10","user":{"id":"7"},"nick":null,"roles":[]})),
					// Invalid optional fields on someone else must not invalidate the owner's permissions.
					(5,"GUILD_MEMBER_UPDATE", json!({"guild_id":"10","user":{"id":"7"},"roles":"unsupported","nick":false})),
					(6,"TYPING_START", json!({"guild_id":"10","channel_id":"20","user_id":"7","timestamp":stamp,"member":{"user":{"id":"7","username":"account"},"nick":"Typing nickname","roles":[]}})),
					(7,"PASSIVE_UPDATE_V2", json!({"guild_id":"10","updated_members":[{"user":{"id":"7","username":"account"},"nick":"Passive nickname","roles":["11"]}]})),
					(8,"GUILD_MEMBER_REMOVE", json!({"guild_id":"10","user":{"id":"7"}})),
				] { send(&mut socket, json!({"op":0,"t":name,"s":sequence,"d":data})).await; }
				// Prove all changes were admitted before requesting reconnection. A timer
				// heartbeat or the initial subscriptions can already be queued.
				send(&mut socket, json!({"op":1,"d":null})).await;
				let mut admitted = false;
				for _ in 0..16 {
					let outgoing = packet(&mut socket).await;
					match outgoing["op"].as_u64() {
						Some(1) => {
							send(&mut socket, json!({"op":11,"d":null})).await;
							if outgoing["d"] == 8 {
								admitted = true;
								break;
							}
						}
						Some(3 | 8 | 37) => {}
						_ => panic!("unexpected synthetic gateway command"),
					}
				}
				assert!(admitted, "member changes must precede reconnection");
				send(&mut socket, json!({"op":7,"d":null})).await;
				// An unread client heartbeat can reset TCP on drop and discard op 7.
				// Keep the socket alive until the client observes the reconnect.
				reconnect_observed.await.unwrap();
				drop(socket);
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
				let resume = packet(&mut socket).await;
				assert_eq!(resume["op"], 6);
				assert_eq!(resume["d"]["seq"], 8);
				// Replayed member changes arrive before RESUMED; the reducer must keep them.
				send(&mut socket, json!({"op":0,"t":"GUILD_MEMBER_UPDATE","s":9,"d":{"guild_id":"10","user":{"id":"7","username":"account"},"nick":"Resumed nickname","roles":[]}})).await;
				send(&mut socket, json!({"op":0,"t":"RESUMED","s":10,"d":{}})).await;
				send(&mut socket, json!({"op":0,"t":"MESSAGE_CREATE","s":11,"d":{"id":"101","channel_id":"20","author":{"id":"8","username":"Other account"},"content":"After identity changes"}})).await;
				observed.await.unwrap();
			};
			let state = std::sync::Mutex::new(client_core::State::default());
			let names = std::sync::Mutex::new(Vec::new());
			let client = run_inner(
				Arc::new(SessionSecret::from_owner_input("SYNTHETIC_MEMBER_SESSION".into()).unwrap()),
				"wss://gateway.discord.gg/".into(), watch::channel(None).1, mpsc::channel(1).1,
				Some(ActivityInput { member_watches: cached_watch.then_some(watch_receive), roster: None, spotify: watch::channel(None).1, member_queries, receiver: watch::channel(None).1, own_presence: watch::channel(Default::default()).1, observe: &|_| Ok(()) }),
				|event| {
					let startup = matches!(&event, Event::Startup(_) | Event::Ready { .. });
					let changed = matches!(&event, Event::MemberIdentity(_) | Event::Typing(_));
					let disconnected = matches!(&event, Event::Disconnected);
					let last = matches!(&event, Event::Message(message) if message.id == Id(101));
					let mut state = state.lock().unwrap();
					let generation = state.generation;
					state.apply(client_core::Envelope { generation, event });
					if disconnected && let Some(consumed) = reconnect_consumed.lock().unwrap().take() {
						consumed.send(()).unwrap();
					}
					if startup {
						let Some(client_core::Command::History { request, .. }) = state.select(Id(20)) else { panic!("synthetic guild accessible") };
						state.apply(client_core::Envelope { generation, event: Event::History { channel: Id(20), request, older: false, messages: vec![] } });
						if cached_watch {
							let revision=state.member_display_revision();
							state.hydrate_member_displays(Id(10),revision,vec![model::MemberDisplay{guild:Id(10),user:Id(7),name:"Cached account".into(),username:Some("account".into()),nick:Some("Cached nickname".into())}]);
							watch_send.send_replace(Some((Id(10),Id(20),vec![Id(7)])));
						} else {
						queries.send_replace([None, Some(client_core::member_search::Request {guild: Id(10), channel: Id(20), nonce: 1, slot: 1, query: String::new(), users: vec![Id(7)]})]);
						}
					}
					if changed {
						names.lock().unwrap().push(state.member_nickname(Id(20), Id(7)).flatten().map(str::to_owned));
						assert!(state.can_send(Id(20)));
					}
					if last {
						assert_eq!(state.member_nickname(Id(20), Id(7)), Some(Some("Resumed nickname")));
						assert!(state.timeline.get(Id(101)).is_some());
						finished.lock().unwrap().take().unwrap().send(()).unwrap();
						return Err(Failure::Expired);
					}
					Ok(())
				}, Some(&endpoint));
			let ((), result) = tokio::join!(server, client);
			assert_eq!(result, Err(Failure::Expired));
			assert_eq!(*names.lock().unwrap(), vec![Some("New nickname".into()), None, None, Some("Typing nickname".into()), Some("Passive nickname".into()), None, Some("Resumed nickname".into())]);
		}).await.expect("member changes interrupted message delivery");
		}
	}

	#[test]
	fn incremental_member_rows_preserve_omitted_identity_fields() {
		let mut active = ActiveMembers::new(MemberSubscription {
			guild: Id(10),
			channel: Id(20),
			request: 1,
			thread: false,
			list_id: "everyone".into(),
			ranges: vec![[0, 99]],
		});
		active.update(decode::<MemberUpdate>(br#"{"guild_id":"10","id":"everyone","ops":[{"op":"SYNC","range":[0,0],"items":[{"member":{"user":{"id":"7","username":"account"},"nick":"Guild nickname","roles":["11"]},"presence":{"status":"online","activities":[]}}]}]}"#).unwrap()).unwrap();
		active.update(decode::<MemberUpdate>(br#"{"guild_id":"10","id":"everyone","ops":[{"op":"UPDATE","index":0,"item":{"member":{"user":{"id":"7","username":"account"}},"presence":{"status":"idle","activities":[]}}}]}"#).unwrap()).unwrap();
		let row = active.people().next().unwrap();
		assert_eq!(row.nick.as_deref(), Some("Guild nickname"));
		assert_eq!(row.roles, vec![Id(11)]);
		assert_eq!(row.status.as_deref(), Some("idle"));
		let update = discord_protocol::member_identity::update(
			br#"{"guild_id":"10","user":{"id":"7"},"nick":"Changed nickname","roles":[]}"#,
		)
		.unwrap();
		active.identity(&update);
		let list = active.snapshot(Freshness::Fresh);
		let Some(model::MemberSlot::Person(row)) = &list.slots[0] else {
			panic!()
		};
		assert_eq!(row.nick.as_deref(), Some("Changed nickname"));
		assert!(row.roles.is_empty());
		active.update(decode::<MemberUpdate>(br#"{"guild_id":"10","id":"everyone","ops":[{"op":"UPDATE","index":0,"item":{"member":{"user":{"id":"7","username":"account"},"nick":null,"roles":[]}}}]}"#).unwrap()).unwrap();
		assert!(active.people().next().unwrap().nick.is_none());
		assert!(active.people().next().unwrap().roles.is_empty());
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
                    // A temporary outage preserves the acknowledged read marker on restoration.
                    (32,"GUILD_DELETE",json!({"id":"2","unavailable":true})),
                    (33,"GUILD_CREATE",json!({"id":"2","owner_id":"7","roles":[{"id":"2","permissions":"68608"}],"members":[{"user":{"id":"1","username":"Synthetic"},"roles":[]}],"channels":[{"id":"4","type":0,"name":"Synthetic channel","position":0,"parent_id":null,"last_message_id":"9","permission_overwrites":[]}]})),
                    (34,"VOICE_CHANNEL_EFFECT_SEND",json!({"channel_id":"4","guild_id":"2","user_id":"8","sound_id":"40","sound_volume":0.5,"emoji":{"id":null,"name":"x"}})),
                    // Emoji-only and malformed effects are dropped without ending the session.
                    (35,"VOICE_CHANNEL_EFFECT_SEND",json!({"channel_id":"4","guild_id":"2","user_id":"8","emoji":{"id":null,"name":"x"}})),
                    (36,"VOICE_CHANNEL_EFFECT_SEND",json!({"channel_id":"not-an-id","sound_id":[]})),
                    (37,"GUILD_SOUNDBOARD_SOUND_DELETE",json!({"guild_id":"2","sound_id":"40"})),
                ] {send(&mut socket,json!({"op":0,"t":name,"s":sequence,"d":data})).await;}
                // A stale replay must neither change counts nor regress the heartbeat cursor.
                send(&mut socket,json!({"op":0,"t":"MESSAGE_REACTION_ADD","s":7,"d":{"channel_id":"4","message_id":"9","user_id":"1","emoji":{"id":null,"name":"x"}}})).await;
                acknowledge(&mut socket,37).await;
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
            let sound_events=std::sync::atomic::AtomicUsize::new(0);
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
                    if let Event::Soundboard(sound)=&event {
                        match (sound_events.fetch_add(1,std::sync::atomic::Ordering::Relaxed),sound) {
                            (0,client_core::soundboard::Event::Effect{channel,user,sound,volume}) => assert_eq!((*channel,*user,*sound,*volume),(Id(4),Id(8),Id(40),0.5)),
                            (1,client_core::soundboard::Event::Changed(guild)) => assert_eq!(*guild,Id(2)),
                            _ => panic!("unexpected soundboard event"),
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
            assert_eq!(sound_events.load(std::sync::atomic::Ordering::Relaxed),2);
            assert_eq!(reaction_changes.load(std::sync::atomic::Ordering::Relaxed),4);
            assert_eq!(thread_events.load(std::sync::atomic::Ordering::Relaxed),8);
        }).await.unwrap();
	}

	#[tokio::test]
	async fn local_socket_identify_ack_drop_resume_and_invalid_session() {
		{
			timeout(Duration::from_secs(45), async {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
				let (client_finished, mut terminal_observed) = tokio::sync::oneshot::channel();
				let (invalid_session_consumed, mut invalid_session_observed) =
					tokio::sync::oneshot::channel();
				let invalid_session_consumed =
					std::sync::Mutex::new(Some(invalid_session_consumed));
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
				let sessions = std::sync::Mutex::new(Vec::new());
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
							Event::Interaction(client_core::interactions::Event::Session(
								session,
							)) => {
								sessions.lock().unwrap().push(session.to_string());
								return Ok(());
							}
							Event::Startup(_) => "ready",
							Event::GuildDisplayMetadata(patches) => {
								assert!(patches.is_empty());
								return Ok(());
							}
							Event::Onboarding(client_core::onboarding::Event::Gates {
								snapshot,
								gates,
							}) => {
								assert!(snapshot);
								assert!(gates.is_empty());
								"onboarding"
							}
							Event::UserAction(client_core::user_actions::Event::FriendsSince {
								entries,
								replace,
							}) => {
								assert!(replace);
								assert!(entries.is_empty());
								return Ok(());
							}
							Event::Resumed => "resumed",
							Event::DirectPresence(_) => "presence",
							Event::Resync => "resync",
							Event::Disconnected => "disconnected",
							// A fresh session after resume failure must refetch account settings.
							Event::AccountSettings {
								status: true,
								folders: true,
							} => "settings",
							Event::ReadState(client_core::read_state::Event::Snapshot {
								..
							}) => {
								return Ok(());
							}
							Event::UserAction(
								client_core::user_actions::Event::Relationships(None)
								| client_core::user_actions::Event::Requests(None)
								| client_core::user_actions::Event::Friends(None)
								| client_core::user_actions::Event::Restrictions(None)
								| client_core::user_actions::Event::MessageRequests(_)
								| client_core::user_actions::Event::MessageSpams(_)
								| client_core::user_actions::Event::RequestSpams(_),
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
				assert_eq!(
					sessions.into_inner().unwrap(),
					[
						"synthetic-first-session",
						"synthetic-first-session",
						"synthetic-new-session"
					]
				);
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
					[
						"ready",
						"onboarding",
						"resumed",
						"presence",
						"resync",
						"ready",
						"settings",
						"onboarding"
					]
				);
			})
			.await
			.expect("local lifecycle exceeded its bounded deadline");
		}
		{
			let mut heartbeat = Heartbeat::default();
			let now = Instant::now();
			let interval = Duration::from_secs(10);
			heartbeat.pending.push_back((1, now));
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
			assert_eq!(next_attempt(6, None), 6);
			assert_eq!(next_attempt(u32::MAX, None), 6);
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

	#[tokio::test]
	async fn unjoined_dm_call_discovery_and_lifecycle_over_local_gateway() {
		use client_core::voice::{Command as V, Event as E};
		timeout(Duration::from_secs(45), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let (controls, receive) = mpsc::channel(8);
			let (deleted, mut deletion) = watch::channel(false);
			let observed = std::sync::Mutex::new(Vec::new());
			let (client_finished, terminal_observed) = tokio::sync::oneshot::channel();
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
				socket.send(Frame::Close(Some(CloseFrame { code: CloseCode::Library(4004), reason: "synthetic stop".into() }))).await.unwrap();
				// Keep TCP alive until the terminal frame is consumed: unread heartbeats can
				// otherwise reset the socket on drop and discard Close on Windows ARM.
				terminal_observed.await.unwrap();
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
			let client = async { let result = client.await; let _ = client_finished.send(()); result };
			let ((), result) = tokio::join!(server, client);
			assert_eq!(result, Err(Failure::Expired));
			assert_eq!(*observed.lock().unwrap(), ["create", "update", "state", "delete"]);
		}).await.unwrap();
	}
	#[tokio::test]
	async fn explicit_dm_join_negotiation_and_leave_over_local_gateway() {
		use client_core::voice::{Command as V, Event as E};
		timeout(Duration::from_secs(45),async {
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let endpoint=format!("ws://{}/",listener.local_addr().unwrap());
            let (controls,receive)=mpsc::channel(8);
            let (client_finished, terminal_observed) = tokio::sync::oneshot::channel();
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
                socket.send(Frame::Close(Some(CloseFrame{code:CloseCode::Library(4004),reason:"synthetic stop".into()}))).await.unwrap();
                // Keep TCP alive until the client consumes Close; unread heartbeats can
                // otherwise reset the socket on Windows before the terminal frame arrives.
                terminal_observed.await.unwrap();
            };
            let client=run_inner(Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),"wss://gateway.discord.gg/".into(),watch::channel(None).1,receive,None,|event| {
                match event {
                    Event::Startup(_)=>controls.try_send(V::Join{channel:Id(2),request:7,ring:false,mute:false,deaf:false}).unwrap(),
                    Event::Voice(E::State{request,session,..})=>{assert_eq!(request,Some(7));assert_eq!(session.unwrap().expose(),"synthetic-call-session");},
                    Event::Voice(E::Server{request,token,..})=>{assert_eq!(request,7);assert_eq!(token.unwrap().expose(),"synthetic-call-token");controls.try_send(V::Leave{channel:Id(2),request}).unwrap();},
                    _=>{},
                }
                Ok(())
            },Some(&endpoint));
            let client = async { let result = client.await; let _ = client_finished.send(()); result };
            let ((),result)=tokio::join!(server,client);assert_eq!(result,Err(Failure::Expired));
        }).await.unwrap();
	}
}

#[cfg(test)]
mod member_tests {
	use super::*;
	use serde_json::json;

	fn person(list: &ActiveMembers, index: usize) -> &Member {
		match list.slots[index].as_ref().unwrap() {
			model::MemberSlot::Person(member) => member,
			_ => panic!("expected person at {index}"),
		}
	}
	fn person_mut(list: &mut ActiveMembers, index: usize) -> &mut Member {
		match list.slots[index].as_mut().unwrap() {
			model::MemberSlot::Person(member) => member,
			_ => panic!("expected person at {index}"),
		}
	}
	fn person_snap(list: &MemberList, index: usize) -> &Member {
		match list.slots[index].as_ref().unwrap() {
			model::MemberSlot::Person(member) => member,
			_ => panic!("expected person at {index}"),
		}
	}
	#[test]
	fn member_presence_burst_delivers_every_user() {
		let mut list = ActiveMembers::new(MemberSubscription {
			thread: false,
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
			ranges: vec![[0, 99], [100, 199]],
		});
		let rows = |start, end| {
			(start..end)
				.map(
					|id: u64| json!({"member":{"user":{"id":id.to_string(),"username":"Synthetic"}}}),
				)
				.collect::<Vec<_>>()
		};
		list.update(decode(&serde_json::to_vec(&json!({"guild_id":"1","id":"everyone","member_count":150,"ops":[{"op":"SYNC","range":[0,99],"items":rows(10,110)},{"op":"SYNC","range":[100,199],"items":rows(110,160)}]})).unwrap()).unwrap()).unwrap();
		let now = Instant::now();
		let received = std::cell::RefCell::new(Vec::new());
		let emit = |event: Event| {
			assert!(event.bytes() <= client_core::MAX_MEMBER_PRESENCE_BYTES);
			let Event::MemberPresence { updates, .. } = event else {
				panic!("expected member presence")
			};
			assert!(updates.len() <= 100);
			received
				.borrow_mut()
				.extend(updates.into_iter().map(|u| u.user));
			Ok(())
		};
		for user in 10..160 {
			let wire = format!(r#"{{"guild_id":"1","user":{{"id":"{user}"}},"status":"online"}}"#);
			list.push_presence(
				discord_protocol::presence::decode(wire.as_bytes()).unwrap(),
				now,
				&emit,
			)
			.unwrap();
		}
		emit(list.take_presence().unwrap()).unwrap();
		assert_eq!(*received.borrow(), (10..160).map(Id).collect::<Vec<_>>());
	}
	#[test]
	fn member_presence_flushes_at_the_byte_limit() {
		let mut list = ActiveMembers::new(MemberSubscription {
			thread: false,
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
			ranges: vec![[0, 99]],
		});
		let rows: Vec<_> = (1..=25)
			.map(|id: u64| json!({"member":{"user":{"id":id.to_string(),"username":"Synthetic"}}}))
			.collect();
		let snapshot = json!({"guild_id":"1","id":"everyone","member_count":25,"ops":[{"op":"SYNC","range":[0,99],"items":rows}]});
		list.update(decode(&serde_json::to_vec(&snapshot).unwrap()).unwrap())
			.unwrap();
		let received = std::cell::RefCell::new(Vec::new());
		let emit = |event: Event| {
			assert!(event.bytes() <= client_core::MAX_MEMBER_PRESENCE_BYTES);
			let Event::MemberPresence { updates, .. } = event else {
				panic!()
			};
			received
				.borrow_mut()
				.extend(updates.into_iter().map(|u| u.user));
			Ok(())
		};
		let text = "🚀".repeat(128);
		let activities: Vec<_> = (0..4)
			.map(|kind| json!({"type":kind,"name":text,"details":text,"state":text}))
			.collect();
		let now = Instant::now();
		for user in 1..=25 {
			let wire = json!({"guild_id":"1","user":{"id":user.to_string()},"status":"online","activities":activities});
			list.push_presence(
				discord_protocol::presence::decode(&serde_json::to_vec(&wire).unwrap()).unwrap(),
				now,
				&emit,
			)
			.unwrap();
		}
		assert!(
			!received.borrow().is_empty(),
			"the byte limit should flush before 100 users"
		);
		emit(list.take_presence().unwrap()).unwrap();
		assert_eq!(*received.borrow(), (1..=25).map(Id).collect::<Vec<_>>());
		assert_eq!(list.pending_presence_bytes, 0);
	}
	#[tokio::test]
	async fn local_gateway_delivers_bursts_for_direct_and_server_presence() {
		use super::tests::{packet, ready, send};
		use tokio::net::TcpListener;
		use tokio_tungstenite::{
			accept_async,
			tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
		};

		timeout(Duration::from_secs(15), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let (_selection, receive) = watch::channel(Some(MemberSubscription {
				thread: false, guild: Id(1), channel: Id(2), request: 7,
				list_id: "everyone".into(), ranges: vec![[0, 99], [100, 199]],
			}));
			let (completed, done) = tokio::sync::oneshot::channel();
			let (client_finished, terminal_observed) = tokio::sync::oneshot::channel();
			let observed = std::sync::Mutex::new((BTreeSet::new(), BTreeSet::new(), Some(completed)));
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(&mut socket, json!({"op":10,"d":{"heartbeat_interval":1000}})).await;
				assert_eq!(packet(&mut socket).await["op"], 2);
				send(&mut socket, ready(1, "synthetic-presence-burst")).await;
				loop {
					let outbound = packet(&mut socket).await;
					if outbound["op"] == 1 {
						send(&mut socket, json!({"op":11,"d":null})).await;
					} else if outbound["op"] == 37
						&& outbound["d"]["subscriptions"]["1"]["channels"] == json!({"2":[[0,99],[100,199]]})
					{
						break;
					}
				}
				let rows = |start, end| (start..end).map(|id: u64| json!({"member":{"user":{"id":id.to_string(),"username":"Synthetic"}}})).collect::<Vec<_>>();
				send(&mut socket, json!({"op":0,"t":"GUILD_MEMBER_LIST_UPDATE","s":2,"d":{"guild_id":"1","id":"everyone","member_count":150,"ops":[{"op":"SYNC","range":[0,99],"items":rows(10,110)},{"op":"SYNC","range":[100,199],"items":rows(110,160)}]}})).await;
				for (sequence, user) in (3_u64..).zip(10..160) {
					send(&mut socket, json!({"op":0,"t":"PRESENCE_UPDATE","s":sequence,"d":{"guild_id":"1","user":{"id":user.to_string()},"status":"online"}})).await;
				}
				for (sequence, user) in (153_u64..).zip(10..160) {
					send(&mut socket, json!({"op":0,"t":"PRESENCE_UPDATE","s":sequence,"d":{"user":{"id":user.to_string()},"status":"idle"}})).await;
				}
				done.await.unwrap();
				socket.send(Frame::Close(Some(CloseFrame { code: CloseCode::Library(4004), reason: "synthetic stop".into() }))).await.unwrap();
				terminal_observed.await.unwrap();
			};
			let client = run_inner(
				Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),
				"wss://gateway.discord.gg/".into(), receive, mpsc::channel(1).1, None,
				|event| {
					let bytes = event.bytes();
					let mut seen = observed.lock().unwrap();
					match event {
						Event::DirectPresence(updates) => {
							assert!(updates.len() <= 100 && bytes <= client_core::MAX_MEMBER_PRESENCE_BYTES);
							seen.0.extend(updates.into_iter().map(|u| u.user));
						}
						Event::MemberPresence { guild, channel, request, updates } => {
							assert_eq!((guild, channel, request), (Id(1), Id(2), 7));
							assert!(updates.len() <= 100 && bytes <= client_core::MAX_MEMBER_PRESENCE_BYTES);
							seen.1.extend(updates.into_iter().map(|u| u.user));
						}
						_ => {}
					}
					if seen.0.len() == 150 && seen.1.len() == 150 && let Some(done) = seen.2.take() { let _ = done.send(()); }
					Ok(())
				}, Some(&endpoint),
			);
			let client = async { let result = client.await; let _ = client_finished.send(()); result };
			let ((), result) = tokio::join!(server, client);
			assert_eq!(result, Err(Failure::Expired));
			let seen = observed.into_inner().unwrap();
			assert_eq!(seen.0, (10..160).map(Id).collect());
			assert_eq!(seen.1, (10..160).map(Id).collect());
		}).await.unwrap();
	}
	#[test]
	fn rich_activity_patches_preserve_absence_and_clear_on_status_loss() {
		let mut list = ActiveMembers::new(MemberSubscription {
			thread: false,
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
			ranges: vec![[0, 99]],
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
			assert_eq!(!person(&list, 0).activities.is_empty(), active);
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
			clients: model::ClientPlatforms::default(),
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
			thread: false,
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
			ranges: vec![[0, 99]],
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
			let row = person(&list, 0);
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
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 7,
				list_id: "everyone".into(),
				ranges: vec![[0, 99]],
			});
			list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Synthetic"},"roles":["12","11"]}}]}]}"#).unwrap()).unwrap();
			assert_eq!(person(&list, 0).roles, vec![Id(11), Id(12)]);
			list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"UPDATE","index":0,"item":{"member":{"user":{"id":"3","username":"Synthetic"},"roles":["13"]}}}]}"#).unwrap()).unwrap();
			assert_eq!(
				person_snap(&list.snapshot(Freshness::Fresh), 0).roles,
				vec![Id(13)]
			);
		}
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 7,
				list_id: "everyone".into(),
				ranges: vec![[0, 99]],
			});
			let update = decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"groups":[{"id":"online","count":1}],"ops":[{"op":"SYNC","range":[0,99],"items":[{"group":{"id":"online"}},{"member":{"user":{"id":"3","username":"Synthetic"}}}]}]}"#).unwrap();
			assert!(list.update(update).unwrap());
			assert!(list.synced);
			assert!(
				matches!(list.slots[0].as_ref(), Some(model::MemberSlot::Group(id)) if id == "online")
			);
			assert_eq!(person(&list, 1).user.id, Id(3));
			assert_eq!(list.groups, vec![("online".into(), 1)]);
			assert_eq!(list.snapshot(Freshness::Fresh).total, 2);
		}
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 3,
				list_id: "everyone".into(),
				ranges: vec![[0, 99]],
			});
			let mut apply = |value: serde_json::Value| {
				list.update(decode::<MemberUpdate>(value.to_string().as_bytes()).unwrap())
			};
			assert!(
				!apply(json!({"guild_id":"9","id":"everyone","member_count":2,"ops":[]})).unwrap()
			);
			assert!(apply(json!({"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"SYNC","range":[0,99],"items":[{"group":{"id":"online","count":2}},{"member":{"user":{"id":"4","username":"First"}}},{"member":{"user":{"id":"5","username":"Second"}}}]}]})).unwrap());
			assert!(list.synced);
			assert!(
				matches!(list.slots[0].as_ref(), Some(model::MemberSlot::Group(id)) if id == "online")
			);
			assert_eq!(person(&list, 2).user.id, Id(5));
			list.update(decode(json!({"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"DELETE","index":1},{"op":"INSERT","index":2,"item":{"member":{"user":{"id":"6","username":"Third"}}}},{"op":"UPDATE","index":1,"item":{"member":{"user":{"id":"5","username":"Updated"}}}}]}).to_string().as_bytes()).unwrap()).unwrap();
			assert_eq!(person(&list, 1).user.name, "Updated");
			assert_eq!(person(&list, 2).user.id, Id(6));
			assert_eq!(list.slots.len(), 100);
			list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"INVALIDATE","range":[0,99]}]}"#).unwrap()).unwrap();
			assert!(list.synced);
			assert_eq!(person(&list, 1).user.name, "Updated");
			assert!(list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"SYNC","range":[9,1],"items":[]}]}"#).unwrap()).is_err());
			list.retarget_ranges(vec![[100, 199]]);
			assert_eq!(list.start, 100);
			assert_eq!(list.slots.len(), 100);
			assert!(!list.synced);
			assert!(
				list.update(
					decode::<MemberUpdate>(
						json!({"guild_id":"1","id":"everyone","member_count":150,"ops":[{"op":"SYNC","range":[100,199],"items":[{"member":{"user":{"id":"9","username":"Later"}}}]}]})
							.to_string()
							.as_bytes(),
					)
					.unwrap(),
				)
				.unwrap()
			);
			assert!(list.synced);
			assert_eq!(person(&list, 0).user.id, Id(9));
			assert_eq!(list.subscription.ranges, vec![[100, 199]]);
		}
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 3,
				list_id: "everyone".into(),
				ranges: vec![[0, 99]],
			});
			let apply = |list: &mut ActiveMembers, value: serde_json::Value| {
				list.update(decode::<MemberUpdate>(value.to_string().as_bytes()).unwrap())
			};
			// Before, one undecodable row rejected every SYNC and the pane loaded forever.
			assert!(
				apply(
					&mut list,
					json!({"guild_id":"1","id":"everyone","ops":[{"op":"SYNC","range":[0,99],"items":[
						{"member":{"user":{"id":"4","username":"First"}}},
						{"member":{"user":{"id":"5"}}},
						{"member":{"user":{"id":"6","username":"Third"}}}
					]}]})
				)
				.unwrap()
			);
			assert!(
				apply(
					&mut list,
					json!({"guild_id":"1","id":"everyone","ops":[
						{"op":"INSERT","index":0,"item":{"member":{"user":{"id":"0","username":"Unreadable"}}}},
						{"op":"UPDATE","index":1,"item":{"group":{}}}
					]})
				)
				.unwrap()
			);
			assert!(list.synced && !list.awaiting_sync);
			assert!(
				list.slots[0].is_none(),
				"inserted placeholder keeps later indices"
			);
			assert_eq!(
				person(&list, 1).user.id,
				Id(4),
				"unreadable UPDATE keeps the row"
			);
			assert!(list.slots[2].is_none());
			assert_eq!(person(&list, 3).user.id, Id(6));
			// A heavy page sheds activity details, far rows first, instead of failing.
			let activity = |n: usize| json!({"type":0,"name":format!("Game {n}"),"details":"\u{1d54f}".repeat(128),"state":"\u{1d54f}".repeat(128)});
			let items: Vec<_> = (1..=100).map(|id| json!({"member":{"user":{"id":id.to_string(),"username":"u".repeat(32),"global_name":"g".repeat(32)},"nick":"n".repeat(32)},"presence":{"status":"online","activities":(0..4).map(activity).collect::<Vec<_>>()}})).collect();
			let ops: Vec<_> =
				std::iter::once(json!({"op":"SYNC","range":[0,99],"items":items}))
					.chain((0..300).map(
						|_| json!({"op":"UPDATE","index":200,"item":{"group":{"id":"online"}}}),
					))
					.collect();
			assert!(apply(&mut list, json!({"guild_id":"1","id":"everyone","ops":ops})).unwrap());
			assert!(list.slot_bytes() <= MEMBER_LIST_BYTES);
			assert!(list.people().count() == 100);
			assert_eq!(person(&list, 0).activities.len(), 4);
			assert!(person(&list, 99).activities.is_empty());
			assert_eq!(person(&list, 99).status.as_deref(), Some("online"));
		}
	}
	#[test]
	fn member_sync_preserves_omitted_identity_by_user_across_reordering() {
		let mut list = ActiveMembers::new(MemberSubscription {
			thread: false,
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
			ranges: vec![[0, 99]],
		});
		let send = |list: &mut ActiveMembers, items: serde_json::Value| {
			let wire = json!({"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"SYNC","range":[0,99],"items":items}]});
			list.update(decode::<MemberUpdate>(&serde_json::to_vec(&wire).unwrap()).unwrap())
				.unwrap();
		};
		send(
			&mut list,
			json!([
				{"member":{"user":{"id":"3","username":"one","global_name":"First Display"},"nick":"First nickname","roles":["11"]}},
				{"member":{"user":{"id":"4","username":"two","global_name":"Second Display"},"nick":"Second nickname","roles":["12"]}}
			]),
		);
		send(
			&mut list,
			json!([
				{"member":{"user":{"id":"4","username":"two"}}},
				{"member":{"user":{"id":"3","username":"one"}}}
			]),
		);
		assert_eq!(person(&list, 0).user.id, Id(4));
		assert_eq!(person(&list, 0).user.name, "Second Display");
		assert_eq!(person(&list, 0).nick.as_deref(), Some("Second nickname"));
		assert_eq!(person(&list, 0).roles, vec![Id(12)]);
		assert_eq!(person(&list, 1).user.id, Id(3));
		assert_eq!(person(&list, 1).user.name, "First Display");
		assert_eq!(person(&list, 1).nick.as_deref(), Some("First nickname"));
		assert_eq!(person(&list, 1).roles, vec![Id(11)]);
		let fields = list.snapshot(Freshness::Fresh).missing_identity;
		assert!(fields.iter().any(|fields| fields.user == Id(3)
			&& fields.global_name == model::GlobalNameUpdate::Unchanged
			&& fields.nick
			&& fields.roles));
		assert!(fields.iter().any(|fields| fields.user == Id(4)
			&& fields.global_name == model::GlobalNameUpdate::Unchanged
			&& fields.nick
			&& fields.roles));
		send(
			&mut list,
			json!([
				{"member":{"user":{"id":"4","username":"two","global_name":null},"nick":null,"roles":[]}},
				{"member":{"user":{"id":"3","username":"one"}}}
			]),
		);
		assert_eq!(person(&list, 0).nick, None);
		assert_eq!(person(&list, 0).user.name, "two");
		assert!(person(&list, 0).roles.is_empty());
		assert!(
			list.snapshot(Freshness::Fresh)
				.missing_identity
				.iter()
				.any(|fields| {
					fields.user == Id(4)
						&& fields.global_name == model::GlobalNameUpdate::Cleared
						&& !fields.nick && !fields.roles
				})
		);
		assert_eq!(person(&list, 1).nick.as_deref(), Some("First nickname"));
		assert_eq!(person(&list, 1).roles, vec![Id(11)]);
		send(
			&mut list,
			json!([
				{"member":{"user":{"id":"4","username":"two","global_name":"Updated Display"}}},
				{"member":{"user":{"id":"3","username":"one"}}}
			]),
		);
		assert_eq!(person(&list, 0).user.name, "Updated Display");
		assert_eq!(person(&list, 1).user.name, "First Display");
		send(
			&mut list,
			json!([
				{"member":{"user":{"id":"4","username":"two"}}},
				{"member":{"user":{"id":"3","username":"same","global_name":"same"}}}
			]),
		);
		send(
			&mut list,
			json!([
				{"member":{"user":{"id":"4","username":"two"}}},
				{"member":{"user":{"id":"3","username":"renamed"}}}
			]),
		);
		assert_eq!(person(&list, 1).user.name, "same");
		assert_eq!(person(&list, 1).user.username.as_deref(), Some("renamed"));
		let mut profile = person(&list, 1).user.clone();
		profile.name = "renamed".into();
		list.identity(&model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Cleared,
			guild: Id(1),
			user: Id(3),
			profile: Some(profile),
			avatar: model::Patch::Absent,
			nick: model::Patch::Absent,
			roles: model::Patch::Absent,
			departed: false,
		});
		let mut profile = person(&list, 1).user.clone();
		profile.name = "later".into();
		profile.username = Some("later".into());
		list.identity(&model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			guild: Id(1),
			user: Id(3),
			profile: Some(profile),
			avatar: model::Patch::Absent,
			nick: model::Patch::Absent,
			roles: model::Patch::Absent,
			departed: false,
		});
		assert_eq!(person(&list, 1).user.name, "later");
		send(
			&mut list,
			json!([
				{"member":{"user":{"id":"4","username":"two"}}},
				{"member":{"user":{"id":"3","username":"newer"}}}
			]),
		);
		assert_eq!(person(&list, 1).user.name, "newer");
		assert!(list.global_names.len() <= list.people().count());
	}

	#[test]
	fn global_name_tracking_follows_the_bounded_member_window() {
		let mut list = ActiveMembers::new(MemberSubscription {
			thread: false,
			guild: Id(1),
			channel: Id(2),
			request: 7,
			list_id: "everyone".into(),
			ranges: vec![[0, 99]],
		});
		let initial: Vec<_> = (1..=100)
			.map(|id| json!({"member":{"user":{"id":id.to_string(),"username":"account","global_name":"Display"}}}))
			.collect();
		let wire = json!({"guild_id":"1","id":"everyone","member_count":100,"ops":[{"op":"SYNC","range":[0,99],"items":initial}]});
		list.update(decode::<MemberUpdate>(&serde_json::to_vec(&wire).unwrap()).unwrap())
			.unwrap();
		let ops: Vec<_> = (0..300)
			.map(|index| json!({"op":"UPDATE","index":index % 100,"item":{"member":{"user":{"id":(index+1000).to_string(),"username":"account","global_name":"Display"}}}}))
			.collect();
		let wire = json!({"guild_id":"1","id":"everyone","member_count":100,"ops":ops});
		list.update(decode::<MemberUpdate>(&serde_json::to_vec(&wire).unwrap()).unwrap())
			.unwrap();
		assert_eq!(list.global_names.len(), 100);
		assert!(
			list.global_names
				.iter()
				.all(|id| list.people().any(|member| member.user.id == *id))
		);
		assert!(list.slot_bytes() <= MEMBER_LIST_BYTES);
	}

	#[test]
	fn presence_coalesces_loaded_rows_at_a_fixed_deadline_and_snapshots_supersede_it() {
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 7,
				list_id: "everyone".into(),
				ranges: vec![[0, 99]],
			});
			let now = Instant::now();
			let presence = |guild, user, status| discord_protocol::presence::PresenceUpdate {
				guild,
				user,
				status,
				custom_status: model::Patch::Absent,
				activities: model::Patch::Absent,
				clients: model::Patch::Absent,
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
			assert_eq!(person(&list, 0).status.as_deref(), Some("dnd"));
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
				person_snap(&list.snapshot(Freshness::Fresh), 0)
					.status
					.as_deref(),
				Some("offline")
			);
			list.presence(
				presence(Some(Id(1)), Id(3), model::Patch::Value("idle".into())),
				now,
			);
			list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"INVALIDATE","range":[3,99]}]}"#).unwrap()).unwrap();
			assert!(list.synced);
			assert_eq!(person(&list, 0).user.name, "First");
			assert_eq!(person(&list, 1).user.name, "Second");
			assert!(list.slots[2].is_none());
			list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"INVALIDATE","range":[0,99]}]}"#).unwrap()).unwrap();
			assert!(
				list.synced && list.take_presence().is_none() && list.presence_deadline.is_none()
			);
		}
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 7,
				list_id: "everyone".into(),
				ranges: vec![[0, 99]],
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
						clients: model::Patch::Absent,
					},
					now,
				);
			}
			assert_eq!(list.pending_presence.len(), 100);
			assert_eq!(list.slots.len(), 100);
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
						&& update.custom_status.as_deref()
							== Some("\u{1f680}".repeat(128).as_str()))
			);
			assert!(list.presence_deadline.is_none());
		}
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 7,
				list_id: "everyone".into(),
				ranges: vec![[0, 99]],
			});
			list.update(decode(br#"{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Synthetic"}}}]}]}"#).unwrap()).unwrap();
			// Fill the synthetic mirror to three bytes below its admitted budget.
			let row = person_mut(&mut list, 0);
			row.nick = Some("n".repeat(256 * 1024 - row.bytes() - 3));
			assert_eq!(row.bytes(), 256 * 1024 - 3);
			let now = Instant::now();
			let update = |status: &str| discord_protocol::presence::PresenceUpdate {
				guild: Some(Id(1)),
				user: Id(3),
				status: model::Patch::Value(status.into()),
				custom_status: model::Patch::Absent,
				activities: model::Patch::Absent,
				clients: model::Patch::Absent,
			};
			list.presence(update("idle"), now);
			assert!(person(&list, 0).status.is_none());
			assert!(list.pending_presence.is_empty() && list.presence_deadline.is_none());
			list.presence(update("dnd"), now);
			assert_eq!(person(&list, 0).bytes(), 256 * 1024);
			list.presence(update("offline"), now + Duration::from_millis(50));
			assert_eq!(person(&list, 0).status.as_deref(), Some("dnd"));
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
					clients: model::Patch::Absent,
				},
				now,
			);
			assert_eq!(person(&list, 0).bytes(), 256 * 1024 - 3);
		}
	}
	#[tokio::test]
	async fn replies_follow_the_service_list_identity_until_synchronized() {
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 3,
				list_id: "computed".into(),
				ranges: vec![[0, 99]],
			});
			let sync = |id: &str| {
				decode::<MemberUpdate>(json!({"guild_id":"1","id":id,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"4","username":"First"}}}]}]}).to_string().as_bytes()).unwrap()
			};
			let retired = [
				(Id(1), "left".to_owned(), None),
				(Id(1), "other".to_owned(), Some("other-service".to_owned())),
			];
			assert!(
				!list.adopt_list(&sync("left"), &retired),
				"a late reply from a left list"
			);
			assert!(!list.adopt_list(&sync("other-service"), &retired));
			let other_guild = decode::<MemberUpdate>(json!({"guild_id":"9","id":"service","ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"4","username":"First"}}}]}]}).to_string().as_bytes()).unwrap();
			assert!(!list.adopt_list(&other_guild, &retired));
			let incremental = decode::<MemberUpdate>(
				br#"{"guild_id":"1","id":"service","ops":[{"op":"DELETE","index":0}]}"#,
			)
			.unwrap();
			assert!(!list.adopt_list(&incremental, &retired));
			assert!(list.adopt_list(&sync("service"), &retired));
			assert!(list.update(sync("service")).unwrap());
			assert!(list.synced && list.list_id() == "service");
			assert!(
				!list.adopt_list(&sync("another"), &retired),
				"never re-pointed once synced"
			);
			assert!(!list.update(sync("computed")).unwrap());
		}
		{
			let mut list = ActiveMembers::new(MemberSubscription {
				thread: false,
				guild: Id(1),
				channel: Id(2),
				request: 3,
				list_id: "everyone".into(),
				ranges: vec![[0, 99]],
			});
			let delays: Vec<_> = (0..6)
				.map(|retries| {
					list.retries = retries;
					list.retry_delay().as_secs()
				})
				.collect();
			assert_eq!(delays, [15, 30, 60, 120, 120, 120]);
		}
		{
			use tokio::net::TcpListener;
			use tokio_tungstenite::{
				accept_async,
				tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
			};
			timeout(Duration::from_secs(10),async {
	            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let endpoint=format!("ws://{}/",listener.local_addr().unwrap());
	            let (selection,receive)=watch::channel(Some(MemberSubscription {thread:false,guild:Id(1),channel:Id(2),request:7,list_id:"everyone".into(),
				ranges: vec![[0, 99]],
			}));
	            let server=async {
	                let (stream,_)=listener.accept().await.unwrap();let mut socket=accept_async(stream).await.unwrap();
	                socket.send(Frame::Text(json!({"op":10,"d":{"heartbeat_interval":1000}}).to_string().into())).await.unwrap();
	                assert!(matches!(socket.next().await,Some(Ok(Frame::Text(_)))));
	                socket.send(Frame::Text(json!({"op":0,"t":"READY","s":1,"d":{"user":{"id":"1","username":"Owner"},"session_id":"synthetic-members","resume_gateway_url":"wss://gateway.discord.gg/","guilds":[],"private_channels":[]}}).to_string().into())).await.unwrap();
	                let mut typing_ready=false;
	                let mut subscribed=false;
	                let mut switched=false;
	                let mut scrolled=false;
	                while let Some(Ok(Frame::Text(text)))=socket.next().await {
	                    let packet:serde_json::Value=serde_json::from_str(&text).unwrap();
	                    if packet["op"]==1 {socket.send(Frame::Text(json!({"op":11,"d":null}).to_string().into())).await.unwrap();continue;}
	                    assert_eq!(packet["op"],37);
	                    assert_eq!(packet["d"]["subscriptions"].as_object().unwrap().len(),1);
	                    let subscription=&packet["d"]["subscriptions"]["1"];
	                    assert_eq!(subscription["threads"],false);
	                    assert_eq!(subscription["activities"],true);
	                    assert_eq!(subscription["member_updates"],false);
	                    if subscription["typing"]==false {
	                        assert_eq!(subscription["members"],json!([]));
	                        assert!(subscribed && switched && scrolled, "unsubscribe before the scrolled range");
	                        assert_eq!(subscription["channels"],json!({}));
	                        break;
	                    }
	                    assert!(subscription.get("members").is_none(), "range updates must preserve separately subscribed members");
	                    if subscription["channels"]==json!({}) && subscription["thread_member_lists"]==json!([]) {
	                        assert!(!subscribed, "cleared the open channel subscription while scrolling");
	                        typing_ready=true;
	                        continue;
	                    }
	                    assert!(typing_ready, "channel ranges before the guild typing subscription");
	                    if !subscribed {
	                        assert_eq!(subscription["typing"],true);assert_eq!(subscription["channels"],json!({"2":[[0,99]]}));subscribed=true;
	                        socket.send(Frame::Text(json!({"op":0,"t":"GUILD_MEMBER_LIST_UPDATE","s":2,"d":{"guild_id":"1","id":"everyone","member_count":1,"ops":[{"op":"SYNC","range":[0,99],"items":[{"member":{"user":{"id":"3","username":"Visible","avatar":"0123456789abcdef0123456789abcdef"}}}]}]}}).to_string().into())).await.unwrap();
	                        for (sequence,data) in [
	                            (3,json!({"guild_id":"9","user":{"id":"3"},"status":"dnd"})),
	                            (4,json!({"guild_id":"1","user":{"id":"4"},"status":"online"})),
	                            (5,json!({"guild_id":"1","user":{"id":"3"},"status":"idle","activities":[{"type":4,"state":"Synthetic live update"}]})),
	                            (6,json!({"guild_id":"1","user":{"id":"3"}})),
	                        ] {socket.send(Frame::Text(json!({"op":0,"t":"PRESENCE_UPDATE","s":sequence,"d":data}).to_string().into())).await.unwrap();}
	                    } else if !scrolled {
                            assert_eq!(subscription["typing"],true);
                            if subscription["channels"]==json!({"4":[[0,99]]}) && !switched {
                                // The latest viewport may replace this intermediate queued
                                // switch. Either way the final100..199 request and matching
                                // service SYNC remain mandatory below.
                                switched=true;continue;
                            }
                            assert_eq!(subscription["channels"],json!({"4":[[100,199]]}));switched=true;scrolled=true;
	                        socket.send(Frame::Text(json!({"op":0,"t":"GUILD_MEMBER_LIST_UPDATE","s":7,"d":{"guild_id":"1","id":"everyone","member_count":150,"ops":[{"op":"SYNC","range":[100,199],"items":[{"member":{"user":{"id":"9","username":"Later"}}}]}]}}).to_string().into())).await.unwrap();
	                    } else {panic!("unexpected member subscription after the scrolled range");}
	                }
	                socket.close(Some(CloseFrame{code:CloseCode::Library(4004),reason:"synthetic stop".into()})).await.unwrap();
	            };
	            let client=run_inner(Arc::new(SessionSecret::from_owner_input("synthetic-owner-session".into()).unwrap()),"wss://gateway.discord.gg/".into(),receive,mpsc::channel(1).1,None,|event| {
	                if let Event::Members(list)=&event {
	                    if list.ranges==vec![[100,199]] {
	                        assert_eq!(list.start,100);
	                        assert_eq!(list.channel,Id(4));
	                        assert_eq!(list.request,8);
	                        if list.freshness==Freshness::Fresh {
	                            assert_eq!(person_snap(list, 0).user.id,Id(9));
	                            selection.send(None).unwrap();
	                        }
	                    } else if list.request==8 {
	                        assert_eq!(list.channel,Id(4));
	                        assert_eq!(list.ranges,vec![[0,99]]);
	                        assert_eq!(list.freshness,Freshness::Fresh);
	                        assert_eq!(person_snap(list, 0).user.id,Id(3));
	                        assert_eq!(person_snap(list, 0).status.as_deref(),Some("idle"));
	                        selection.send(Some(MemberSubscription {thread:false,guild:Id(1),channel:Id(4),request:8,list_id:"everyone".into(),ranges:vec![[100,199]]})).unwrap();
	                    } else {
	                        assert_eq!(person_snap(list, 0).user.id,Id(3));
	                        assert_eq!(list.freshness,Freshness::Fresh);
	                        assert_eq!(list.request,7);assert_eq!(list.channel,Id(2));
	                    }
	                }
	                if let Event::MemberPresence {guild,channel,request,updates}=event {
	                    assert_eq!((guild,channel,request),(Id(1),Id(2),7));
	                    assert_eq!(updates,vec![record(3,Some("idle"),Some("Synthetic live update"))]);
	                    selection.send(Some(MemberSubscription {thread:false,guild:Id(1),channel:Id(4),request:8,list_id:"everyone".into(),
				ranges: vec![[0, 99]],
			})).unwrap();
	                }
	                Ok(())
	            },Some(&endpoint));
	            let ((),result)=tokio::join!(server,client);assert_eq!(result,Err(Failure::Expired));
	        }).await.unwrap();
		}
	}
}

#[cfg(debug_assertions)]
pub use member_search::debug_check as debug_member_search_check;

#[cfg(debug_assertions)]
pub use voice::debug_voice_state_retry_check;
