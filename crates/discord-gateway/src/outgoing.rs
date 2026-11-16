//! One bounded socket writer. Admission never awaits the network on the reader's task.
use crate::{Failure, Frame};
use futures_util::{Sink, SinkExt};
use model::Id;
use std::{
	collections::{BTreeMap, VecDeque},
	future::poll_fn,
	sync::{Arc, Mutex},
	time::Duration,
};
use tokio::{
	sync::{Notify, watch},
	task::JoinHandle,
	time::{Instant, sleep_until, timeout},
};

pub(super) const PACKETS: usize = 16;
pub(super) const BYTES: usize = 64 * 1024;
const WINDOW: Duration = Duration::from_secs(60);
const EVENTS: usize = 120;
const COOL_DOWN_KEYS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
	Presence,
	Subscription(Id, bool),
	CancelSubscription(Id),
	MemberIdentities(Id),
	Query(crate::member_search::Purpose),
	Roster(u64),
}
#[derive(Clone, Copy, Debug)]
struct Meta {
	opcode: Option<u8>,
	guild: Option<Id>,
	nonce: Option<u64>,
	key: Option<Key>,
	heartbeat: bool,
	critical: bool,
}
impl Meta {
	fn of(frame: &Frame) -> Self {
		let mut meta = Self {
			opcode: None,
			guild: None,
			nonce: None,
			key: None,
			heartbeat: false,
			critical: true,
		};
		let Frame::Text(text) = frame else {
			return meta;
		};
		let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
			return meta;
		};
		meta.opcode = value["op"].as_u64().and_then(|v| u8::try_from(v).ok());
		match meta.opcode {
			Some(1) => meta.heartbeat = true,
			Some(3) => {
				meta.key = Some(Key::Presence);
				meta.critical = false;
			}
			Some(8) => {
				meta.guild = value["d"]["guild_id"]
					.as_str()
					.and_then(|v| v.parse::<u64>().ok())
					.map(Id);
				meta.nonce = value["d"]["nonce"]
					.as_str()
					.and_then(|v| v.parse::<u64>().ok());
				meta.critical = false;
			}
			Some(37) => {
				if let Some((guild, setting)) = value["d"]["subscriptions"]
					.as_object()
					.and_then(|v| v.iter().next())
				{
					meta.guild = guild.parse::<u64>().ok().map(Id);
					let empty = setting["channels"]
						.as_object()
						.is_none_or(serde_json::Map::is_empty)
						&& setting["thread_member_lists"]
							.as_array()
							.is_none_or(Vec::is_empty);
					// Keep the initial guild subscription before its ranges; only viewport replacements coalesce.
					if setting.get("members").is_some()
						&& setting.get("channels").is_none()
						&& setting.get("thread_member_lists").is_none()
					{
						meta.key = meta.guild.map(Key::MemberIdentities);
						meta.critical = false;
					} else if setting["typing"] == true {
						meta.key = meta.guild.map(|id| Key::Subscription(id, empty));
						meta.critical = false;
					} else {
						meta.key = meta.guild.map(Key::CancelSubscription);
					}
				}
			}
			_ => {}
		}
		meta
	}
}
struct Packet {
	frame: Frame,
	meta: Meta,
	bytes: usize,
	serial: u64,
}
#[derive(Default)]
pub(super) struct Cooldowns {
	pub connection_until: Option<Instant>,
	deadlines: BTreeMap<(u8, Option<Id>), Instant>,
	blocked_until: Option<Instant>,
}

struct State {
	queue: VecDeque<Packet>,
	bytes: usize,
	in_flight: bool,
	serial: u64,
	subscription_serial: u64,
	heartbeat_serial: u64,
	heartbeat_in_flight: Option<(u64, bool)>,
	heartbeats: VecDeque<HeartbeatReceipt>,
}
pub(super) struct HeartbeatReceipt {
	pub ticket: u64,
	pub sent: Instant,
	pub acknowledged: bool,
}
impl Default for State {
	fn default() -> Self {
		Self {
			queue: VecDeque::with_capacity(PACKETS),
			bytes: 0,
			in_flight: false,
			serial: 0,
			subscription_serial: 0,
			heartbeat_serial: 0,
			heartbeat_in_flight: None,
			heartbeats: VecDeque::with_capacity(PACKETS),
		}
	}
}
#[derive(Clone, Copy, Default)]
pub(super) struct Progress {
	pub heartbeat: Option<(u64, Instant)>,
	pub query: [Option<(Id, u64, Instant)>; 2],
	pub roster: Option<(Id, u64, Instant)>,
	pub subscription: Option<(u64, Instant)>,
	pub failed: bool,
	pub capacity: bool,
}
struct FailureOnExit(watch::Sender<Progress>);
impl Drop for FailureOnExit {
	fn drop(&mut self) {
		self.0.send_modify(|progress| progress.failed = true);
	}
}
pub(super) struct Outgoing {
	state: Arc<Mutex<State>>,
	limits: Arc<Mutex<Cooldowns>>,
	wake: Arc<Notify>,
	pub progress: watch::Receiver<Progress>,
	task: JoinHandle<()>,
}
impl Drop for Outgoing {
	fn drop(&mut self) {
		self.task.abort();
	}
}
struct Budget {
	sent: VecDeque<Instant>,
	reserved: usize,
}
impl Budget {
	fn new(interval: Duration) -> Self {
		let heartbeats = WINDOW.as_millis().div_ceil(interval.as_millis().max(1)) as usize;
		Self {
			sent: VecDeque::with_capacity(EVENTS),
			reserved: (heartbeats + 12).min(EVENTS - 1),
		}
	}
	fn earliest(&mut self, now: Instant, meta: Meta) -> Instant {
		while self
			.sent
			.front()
			.is_some_and(|at| now.saturating_duration_since(*at) >= WINDOW)
		{
			self.sent.pop_front();
		}
		if meta.opcode.is_none() {
			return now;
		} // WebSocket Pong is not a Gateway event.
		let limit = if meta.heartbeat {
			EVENTS
		} else if meta.critical {
			EVENTS - self.reserved.saturating_sub(8)
		} else {
			EVENTS - self.reserved
		};
		if self.sent.len() < limit {
			now
		} else {
			self.sent[self.sent.len() - limit] + WINDOW
		}
	}
}
impl Outgoing {
	#[cfg(test)]
	pub fn start<S>(sink: S, interval: Duration) -> Self
	where
		S: Sink<Frame> + Unpin + Send + 'static,
		S::Error: Send,
	{
		Self::with_limits(sink, interval, Arc::new(Mutex::new(Cooldowns::default())))
	}
	pub fn with_limits<S>(mut sink: S, interval: Duration, limits: Arc<Mutex<Cooldowns>>) -> Self
	where
		S: Sink<Frame> + Unpin + Send + 'static,
		S::Error: Send,
	{
		let state = Arc::new(Mutex::new(State::default()));
		let wake = Arc::new(Notify::new());
		let (report, progress) = watch::channel(Progress::default());
		let worker_report = report.clone();
		let worker_state = state.clone();
		let worker_wake = wake.clone();
		let worker_limits = limits.clone();
		let task = tokio::spawn(async move {
			let report = worker_report;
			let _failure_on_exit = FailureOnExit(report.clone());
			let mut budget = Budget::new(interval);
			// Identify/Resume was sent before the writer was installed.
			budget.sent.push_back(Instant::now());
			loop {
				let notified = worker_wake.notified();
				let now = Instant::now();
				let selected = {
					let Ok(mut state) = worker_state.lock() else {
						report.send_modify(|p| p.failed = true);
						return;
					};
					let Ok(mut limits) = worker_limits.lock() else {
						return;
					};
					limits.deadlines.retain(|_, until| *until > now);
					let mut next = None;
					let mut deadline = None;
					for (index, packet) in state.queue.iter().enumerate() {
						let mut at = budget.earliest(now, packet.meta);
						if !packet.meta.critical
							&& let Some(until) = limits.blocked_until
						{
							at = at.max(until);
						}
						if let Some(opcode) = packet.meta.opcode {
							let until = limits
								.deadlines
								.get(&(opcode, packet.meta.guild))
								.copied()
								.into_iter()
								.chain(limits.deadlines.get(&(opcode, None)).copied())
								.chain(limits.deadlines.iter().filter_map(
									|((limited, _), until)| {
										(packet.meta.guild.is_none() && *limited == opcode)
											.then_some(*until)
									},
								))
								.max();
							if let Some(until) = until {
								at = at.max(until);
							}
						}
						let foreground_ahead = next.is_some_and(|selected: usize| {
							matches!(state.queue[selected].meta.key, Some(Key::Roster(_)))
								&& !matches!(packet.meta.key, Some(Key::Roster(_)))
						});
						if at <= now
							&& (next.is_none() || packet.meta.heartbeat || foreground_ahead)
						{
							next = Some(index);
							if packet.meta.heartbeat {
								break;
							}
						}
						deadline = Some(deadline.map_or(at, |previous: Instant| previous.min(at)));
					}
					match next {
						Some(index) => {
							state.in_flight = true;
							Some(Ok(state.queue.remove(index).unwrap()))
						}
						None => deadline.map(Err),
					}
				};
				let packet = match selected {
					Some(Ok(packet)) => packet,
					Some(Err(at)) => {
						tokio::select! { _=notified => {}, _=sleep_until(at)=>{} };
						continue;
					}
					None => {
						notified.await;
						continue;
					}
				};
				let transmit = async {
					if poll_fn(|cx| sink.poll_ready_unpin(cx)).await.is_err() {
						return false;
					}
					let started = {
						let Ok(mut state) = worker_state.lock() else {
							return false;
						};
						// start_send can expose bytes before returning; ACK handling shares this lock.
						if packet.meta.heartbeat {
							state.heartbeat_in_flight = Some((packet.serial, false));
						}
						sink.start_send_unpin(packet.frame).is_ok()
					};
					started && sink.flush().await.is_ok()
				};
				if !matches!(timeout(Duration::from_secs(5), transmit).await, Ok(true)) {
					report.send_modify(|p| p.failed = true);
					return;
				}
				let sent = Instant::now();
				if packet.meta.opcode.is_some() {
					budget.sent.push_back(sent);
				}
				if let Ok(mut state) = worker_state.lock() {
					state.bytes = state.bytes.saturating_sub(packet.bytes);
					state.in_flight = false;
					if packet.meta.heartbeat {
						if state.heartbeats.len() == PACKETS {
							report.send_modify(|p| {
								p.failed = true;
								p.capacity = true;
							});
							return;
						}
						let acknowledged = state.heartbeat_in_flight.take().is_some_and(
							|(ticket, acknowledged)| ticket == packet.serial && acknowledged,
						);
						state.heartbeats.push_back(HeartbeatReceipt {
							ticket: packet.serial,
							sent,
							acknowledged,
						});
					}
				}
				if let Ok(mut limits) = worker_limits.lock()
					&& packet.meta.opcode == Some(8)
				{
					let until = sent + Duration::from_secs(30);
					if limits.deadlines.len() < COOL_DOWN_KEYS
						|| limits.deadlines.contains_key(&(8, packet.meta.guild))
					{
						limits
							.deadlines
							.entry((8, packet.meta.guild))
							.and_modify(|at| *at = (*at).max(until))
							.or_insert(until);
					} else {
						limits.blocked_until =
							Some(limits.blocked_until.map_or(until, |at| at.max(until)));
					}
				}
				report.send_modify(|p| {
					if packet.meta.heartbeat {
						p.heartbeat = Some((packet.serial, sent));
					}
					if packet.meta.opcode == Some(37)
						&& !matches!(packet.meta.key, Some(Key::MemberIdentities(_)))
					{
						p.subscription = Some((packet.serial, sent));
					}
					if let (Some(Key::Query(purpose)), Some(guild), Some(nonce)) =
						(packet.meta.key, packet.meta.guild, packet.meta.nonce)
					{
						// Watch updates may coalesce before the reader observes them.
						// One purpose must not erase the other purpose's send receipt.
						p.query[purpose.slot()] = Some((guild, nonce, sent));
					}
					if let (Some(Key::Roster(job)), Some(guild)) =
						(packet.meta.key, packet.meta.guild)
					{
						p.roster = Some((guild, job, sent));
					}
				});
			}
		});
		Self {
			state,
			limits,
			wake,
			progress,
			task,
		}
	}
	pub fn is_finished(&self) -> bool {
		self.task.is_finished()
	}
	pub fn take_heartbeat(&self) -> Option<HeartbeatReceipt> {
		self.state.lock().ok()?.heartbeats.pop_front()
	}
	pub fn acknowledge_heartbeat(&self) -> Option<u64> {
		let mut state = self.state.lock().ok()?;
		if let Some(receipt) = state
			.heartbeats
			.iter_mut()
			.find(|receipt| !receipt.acknowledged)
		{
			receipt.acknowledged = true;
			return Some(receipt.ticket);
		}
		// An early ACK belongs only to this started send, never a queued heartbeat.
		if let Some((_, acknowledged)) = &mut state.heartbeat_in_flight {
			*acknowledged = true;
		}
		None
	}
	pub fn heartbeat_ticket(&self) -> u64 {
		self.state.lock().map(|s| s.heartbeat_serial).unwrap_or(0)
	}
	pub fn occupancy(&self) -> (usize, usize) {
		self.state
			.lock()
			.map(|s| (s.queue.len() + usize::from(s.in_flight), s.bytes))
			.unwrap_or((PACKETS, BYTES))
	}
	pub fn optional_capacity(&self, count: usize) -> bool {
		let (items, bytes) = self.occupancy();
		items + count <= PACKETS - 4 && bytes <= BYTES - 24 * 1024
	}
	pub fn control_capacity(&self) -> bool {
		let (items, bytes) = self.occupancy();
		// A Join can enqueue Sync, Join and channel-info. Keep its complete batch
		// below the critical limit, with the final slot reserved for a heartbeat.
		items <= PACKETS - 4 && bytes <= BYTES - 12 * 1024
	}
	pub fn subscription_ticket(&self) -> u64 {
		self.state
			.lock()
			.map(|s| s.subscription_serial)
			.unwrap_or(0)
	}
	pub fn clear_queries(&self, changed: u8) {
		if let Ok(mut state) = self.state.lock() {
			let mut removed = 0;
			state.queue.retain(|packet| {
				if matches!(packet.meta.key,Some(Key::Query(purpose)) if changed&(1<<purpose.slot())!=0)
				{
					removed += packet.bytes;
					false
				} else {
					true
				}
			});
			state.bytes -= removed;
		}
		self.wake.notify_one();
	}
	pub fn clear_subscriptions(&self) {
		if let Ok(mut state) = self.state.lock() {
			let mut removed = 0;
			state.queue.retain(|p| {
				if matches!(p.meta.key, Some(Key::Subscription(_, false))) {
					removed += p.bytes;
					false
				} else {
					true
				}
			});
			state.bytes = state.bytes.saturating_sub(removed);
		}
		self.wake.notify_one();
	}
	/// Queue admission only; success never claims that Discord accepted the action.
	pub async fn send(&self, frame: Frame) -> Result<(), Failure> {
		self.enqueue(frame, None)
	}
	pub async fn send_query(&self, query: crate::member_search::Query) -> Result<(), Failure> {
		self.enqueue(query.frame, Some(Key::Query(query.purpose)))
	}
	pub async fn send_roster(&self, frame: Frame, job: u64) -> Result<(), Failure> {
		self.enqueue(frame, Some(Key::Roster(job)))
	}
	pub fn clear_roster(&self) {
		if let Ok(mut state) = self.state.lock() {
			let mut removed = 0;
			state.queue.retain(|packet| {
				if matches!(packet.meta.key, Some(Key::Roster(_))) {
					removed += packet.bytes;
					false
				} else {
					true
				}
			});
			state.bytes = state.bytes.saturating_sub(removed);
		}
		self.wake.notify_one();
	}
	fn enqueue(&self, frame: Frame, key: Option<Key>) -> Result<(), Failure> {
		if self.progress.borrow().failed {
			return Err(Failure::Network);
		}
		let mut meta = Meta::of(&frame);
		if key.is_some() {
			meta.key = key;
		}
		let bytes = frame.len() + std::mem::size_of::<Packet>();
		let mut state = self.state.lock().map_err(|_| Failure::Network)?;
		let superseded = |packet: &Packet| {
			meta.key.is_some_and(|key| {
				packet.meta.key == Some(key)
					|| matches!((key, packet.meta.key),
						(Key::CancelSubscription(guild), Some(Key::Subscription(id, _) | Key::MemberIdentities(id))) if guild == id)
			})
		};
		let (removed_count, removed_bytes) = state
			.queue
			.iter()
			.filter(|packet| superseded(packet))
			.fold((0, 0), |(count, bytes), packet| {
				(count + 1, bytes + packet.bytes)
			});

		let limit = if meta.heartbeat {
			PACKETS
		} else if meta.critical {
			PACKETS - 1
		} else {
			PACKETS - 4
		};
		let reserve = if meta.critical { 0 } else { 16 * 1024 };
		if state.queue.len() - removed_count + usize::from(state.in_flight) >= limit
			|| bytes > BYTES.saturating_sub(state.bytes - removed_bytes + reserve)
		{
			return Err(if bytes > BYTES {
				Failure::ResponseLimit
			} else {
				Failure::Busy
			});
		}
		state.queue.retain(|packet| !superseded(packet));
		state.bytes -= removed_bytes;

		state.serial = state.serial.wrapping_add(1);
		let serial = state.serial;
		if meta.heartbeat {
			state.heartbeat_serial = serial;
		}
		if meta.opcode == Some(37) && !matches!(meta.key, Some(Key::MemberIdentities(_))) {
			state.subscription_serial = serial;
		}
		state.bytes += bytes;
		state.queue.push_back(Packet {
			frame,
			meta,
			bytes,
			serial,
		});
		drop(state);
		self.wake.notify_one();
		Ok(())
	}
	pub fn cooldown(&self, opcode: u8, guild: Option<Id>, duration: Duration) {
		if let Ok(mut limits) = self.limits.lock() {
			let now = Instant::now();
			let until = now + duration;
			limits.deadlines.retain(|_, deadline| *deadline > now);
			if limits.deadlines.len() < COOL_DOWN_KEYS
				|| limits.deadlines.contains_key(&(opcode, guild))
			{
				limits
					.deadlines
					.entry((opcode, guild))
					.and_modify(|at| *at = (*at).max(until))
					.or_insert(until);
			} else {
				limits.blocked_until = Some(limits.blocked_until.map_or(until, |at| at.max(until)));
			}
		}
		self.wake.notify_one();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{
		pin::Pin,
		sync::atomic::{AtomicBool, Ordering},
		task::{Context, Poll, Waker},
	};
	type Wake = Arc<Mutex<Option<Waker>>>;
	type Frames = Arc<Mutex<Vec<Frame>>>;
	struct Controlled {
		ready: Arc<AtomicBool>,
		wake: Wake,
		frames: Frames,
	}
	impl Sink<Frame> for Controlled {
		type Error = ();
		fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
			if self.ready.load(Ordering::Acquire) {
				Poll::Ready(Ok(()))
			} else {
				*self.wake.lock().unwrap() = Some(cx.waker().clone());
				Poll::Pending
			}
		}
		fn start_send(self: Pin<&mut Self>, frame: Frame) -> Result<(), ()> {
			self.frames.lock().unwrap().push(frame);
			Ok(())
		}
		fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
			Poll::Ready(Ok(()))
		}
		fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
			Poll::Ready(Ok(()))
		}
	}
	fn fixture() -> (Controlled, Arc<AtomicBool>, Wake, Frames) {
		let ready = Arc::new(AtomicBool::new(false));
		let wake = Arc::new(Mutex::new(None));
		let frames = Arc::new(Mutex::new(Vec::new()));
		(
			Controlled {
				ready: ready.clone(),
				wake: wake.clone(),
				frames: frames.clone(),
			},
			ready,
			wake,
			frames,
		)
	}
	fn frame(value: serde_json::Value) -> Frame {
		Frame::Text(value.to_string().into())
	}
	struct AckBeforeFlush {
		complete: Arc<AtomicBool>,
		wake: Wake,
		ack: tokio::sync::mpsc::Sender<Frame>,
		pending_ack: bool,
	}
	impl Sink<Frame> for AckBeforeFlush {
		type Error = ();
		fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
			Poll::Ready(Ok(()))
		}
		fn start_send(mut self: Pin<&mut Self>, frame: Frame) -> Result<(), ()> {
			self.pending_ack = Meta::of(&frame).heartbeat;
			Ok(())
		}
		fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
			if self.pending_ack {
				self.pending_ack = false;
				self.ack
					.try_send(frame(serde_json::json!({"op":11,"d":null})))
					.unwrap();
			}
			if self.complete.load(Ordering::Acquire) {
				Poll::Ready(Ok(()))
			} else {
				*self.wake.lock().unwrap() = Some(cx.waker().clone());
				Poll::Pending
			}
		}
		fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), ()>> {
			Poll::Ready(Ok(()))
		}
	}
	struct FlushControl {
		complete: Arc<AtomicBool>,
		wake: Wake,
		incoming: tokio::sync::mpsc::Receiver<Frame>,
	}
	impl FlushControl {
		async fn ack(&mut self) {
			let ack = timeout(Duration::from_secs(1), self.incoming.recv())
				.await
				.unwrap()
				.unwrap();
			assert_eq!(
				serde_json::from_str::<serde_json::Value>(ack.to_text().unwrap()).unwrap()["op"],
				11
			);
		}
		fn finish(&self) {
			self.complete.store(true, Ordering::Release);
			if let Some(wake) = self.wake.lock().unwrap().take() {
				wake.wake();
			}
		}
	}
	fn flush_fixture() -> (AckBeforeFlush, FlushControl) {
		let (ack, incoming) = tokio::sync::mpsc::channel(PACKETS);
		let complete = Arc::new(AtomicBool::new(false));
		let wake = Arc::new(Mutex::new(None));
		(
			AckBeforeFlush {
				complete: complete.clone(),
				wake: wake.clone(),
				ack,
				pending_ack: false,
			},
			FlushControl {
				complete,
				wake,
				incoming,
			},
		)
	}
	async fn heartbeat_sent(writer: &mut Outgoing) -> (u64, Instant) {
		let ticket = writer.heartbeat_ticket();
		timeout(Duration::from_secs(1), async {
			loop {
				if let Some(receipt) = writer.progress.borrow_and_update().heartbeat
					&& receipt.0 == ticket
				{
					return receipt;
				}
				writer.progress.changed().await.unwrap();
			}
		})
		.await
		.unwrap()
	}
	#[tokio::test]
	async fn heartbeat_ack_before_flush_completion_does_not_leave_a_phantom_timeout() {
		let (sink, mut control) = flush_fixture();
		let interval = Duration::from_secs(45);
		let mut writer = Outgoing::start(sink, interval);
		let mut heartbeat = crate::Heartbeat::default();
		writer
			.send(frame(serde_json::json!({"op":1,"d":42})))
			.await
			.unwrap();
		let mut probe_requested = Some(writer.heartbeat_ticket());
		control.ack().await;
		assert!(writer.progress.borrow().heartbeat.is_none());
		assert!(heartbeat.collect(&writer, &mut probe_requested));
		assert!(heartbeat.probe_deadline(probe_requested).is_none());
		assert_eq!(heartbeat.acknowledge(&writer), None);
		assert!(probe_requested.is_some());
		control.finish();
		heartbeat_sent(&mut writer).await;
		assert!(heartbeat.collect(&writer, &mut probe_requested));
		assert_eq!(probe_requested, None);
		assert!(
			!heartbeat.expired(Instant::now() + interval, interval),
			"the ACK received during flush must retire that heartbeat"
		);
	}
	#[tokio::test]
	async fn delayed_ack_for_an_uncollected_receipt_keeps_the_in_flight_probe_outstanding() {
		let (sink, mut control) = flush_fixture();
		let interval = Duration::from_secs(45);
		let mut writer = Outgoing::start(sink, interval);
		let mut heartbeat = crate::Heartbeat::default();
		control.finish();
		writer
			.send(frame(serde_json::json!({"op":1,"d":41})))
			.await
			.unwrap();
		control.ack().await;
		let (older, _) = heartbeat_sent(&mut writer).await;
		control.complete.store(false, Ordering::Release);
		writer
			.send(frame(serde_json::json!({"op":1,"d":42})))
			.await
			.unwrap();
		let probe = writer.heartbeat_ticket();
		let mut probe_requested = Some(probe);
		control.ack().await;
		assert_eq!(heartbeat.acknowledge(&writer), Some(older));
		assert!(heartbeat.collect(&writer, &mut probe_requested));
		assert_eq!(probe_requested, Some(probe));
		assert!(heartbeat.probe_deadline(probe_requested).is_none());
		control.finish();
		let (_, sent) = heartbeat_sent(&mut writer).await;
		assert!(heartbeat.collect(&writer, &mut probe_requested));
		assert_eq!(
			heartbeat.probe_deadline(probe_requested),
			Some(sent + crate::PROBE_TIMEOUT)
		);
		assert_eq!(heartbeat.acknowledge(&writer), Some(probe));
	}
	#[tokio::test]
	async fn repeated_early_acks_do_not_credit_a_queued_heartbeat() {
		let (sink, mut control) = flush_fixture();
		let interval = Duration::from_secs(45);
		let mut writer = Outgoing::start(sink, interval);
		let mut heartbeat = crate::Heartbeat::default();
		writer
			.send(frame(serde_json::json!({"op":1,"d":41})))
			.await
			.unwrap();
		control.ack().await;
		writer
			.send(frame(serde_json::json!({"op":1,"d":42})))
			.await
			.unwrap();
		let probe = writer.heartbeat_ticket();
		let mut probe_requested = Some(probe);
		for _ in 0..3 {
			assert_eq!(heartbeat.acknowledge(&writer), None);
		}
		control.finish();
		control.ack().await;
		let (_, sent) = heartbeat_sent(&mut writer).await;
		assert!(heartbeat.collect(&writer, &mut probe_requested));
		assert_eq!(probe_requested, Some(probe));
		assert_eq!(heartbeat.pending.front().unwrap().0, probe);
		assert_eq!(heartbeat.pending.len(), 1);
		assert!(heartbeat.expired(sent + interval, interval));
	}
	#[tokio::test]
	async fn unsolicited_acks_cannot_credit_a_heartbeat_waiting_for_sink_readiness() {
		let (sink, ready, wake, _) = fixture();
		let interval = Duration::from_secs(45);
		let mut writer = Outgoing::start(sink, interval);
		let mut heartbeat = crate::Heartbeat::default();
		assert_eq!(heartbeat.acknowledge(&writer), None);
		writer
			.send(frame(serde_json::json!({"op":1,"d":42})))
			.await
			.unwrap();
		let probe = writer.heartbeat_ticket();
		let mut probe_requested = Some(probe);
		tokio::task::yield_now().await;
		assert!(wake.lock().unwrap().is_some());
		assert_eq!(heartbeat.acknowledge(&writer), None);
		assert!(heartbeat.collect(&writer, &mut probe_requested));
		assert!(heartbeat.probe_deadline(probe_requested).is_none());
		ready.store(true, Ordering::Release);
		wake.lock().unwrap().take().unwrap().wake();
		let (_, sent) = heartbeat_sent(&mut writer).await;
		assert!(heartbeat.collect(&writer, &mut probe_requested));
		assert_eq!(probe_requested, Some(probe));
		assert_eq!(
			heartbeat.probe_deadline(probe_requested),
			Some(sent + crate::PROBE_TIMEOUT)
		);
		assert!(!heartbeat.expired(sent + interval - Duration::from_millis(1), interval));
		assert!(heartbeat.expired(sent + interval, interval));
	}
	#[tokio::test]
	async fn roster_reads_yield_to_foreground_and_have_independent_send_receipts() {
		let (sink, ready, wake, frames) = fixture();
		let mut writer = Outgoing::start(sink, Duration::from_secs(45));
		writer
			.send_roster(
				frame(
					serde_json::json!({"op":8,"d":{"guild_id":"9","query":"","limit":0,"nonce":"serein-sync-42"}}),
				),
				42,
			)
			.await
			.unwrap();
		writer
			.send(frame(serde_json::json!({"op":3,"d":{"status":"online"}})))
			.await
			.unwrap();
		ready.store(true, Ordering::Release);
		if let Some(wake) = wake.lock().unwrap().take() {
			wake.wake();
		}
		timeout(Duration::from_secs(1), async {
			loop {
				writer.progress.changed().await.unwrap();
				if writer.progress.borrow().roster.is_some() {
					break;
				}
			}
		})
		.await
		.unwrap();
		let frames = frames.lock().unwrap();
		let Frame::Text(first) = &frames[0] else {
			panic!()
		};
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(first).unwrap()["op"],
			3
		);
		assert_eq!(writer.progress.borrow().roster.unwrap().1, 42);
		assert!(writer.progress.borrow().query.iter().all(Option::is_none));
	}
	#[tokio::test]
	async fn cancelling_queued_roster_preserves_foreground_and_heartbeat() {
		let (sink, ready, wake, frames) = fixture();
		let mut writer = Outgoing::start(sink, Duration::from_secs(45));
		writer
			.send_roster(
				frame(
					serde_json::json!({"op":8,"d":{"guild_id":"9","query":"","limit":0,"nonce":"serein-sync-42"}}),
				),
				42,
			)
			.await
			.unwrap();
		writer.clear_roster();
		writer
			.send(frame(serde_json::json!({"op":1,"d":2})))
			.await
			.unwrap();
		ready.store(true, Ordering::Release);
		if let Some(wake) = wake.lock().unwrap().take() {
			wake.wake();
		}
		timeout(Duration::from_secs(1), async {
			loop {
				writer.progress.changed().await.unwrap();
				if writer.progress.borrow().heartbeat.is_some() {
					break;
				}
			}
		})
		.await
		.unwrap();
		assert!(writer.progress.borrow().roster.is_none());
		assert_eq!(frames.lock().unwrap().len(), 1);
	}

	#[test]
	fn rolling_budget_reserves_heartbeat_and_control_headroom() {
		let now = Instant::now();
		let mut budget = Budget::new(Duration::from_secs(1));
		let normal = Meta::of(&frame(serde_json::json!({"op":3,"d":{}})));
		for _ in 0..(EVENTS - budget.reserved) {
			assert_eq!(budget.earliest(now, normal), now);
			budget.sent.push_back(now);
		}
		assert_eq!(budget.earliest(now, normal), now + WINDOW);
		let heartbeat = Meta::of(&frame(serde_json::json!({"op":1,"d":null})));
		for _ in budget.sent.len()..EVENTS {
			assert_eq!(budget.earliest(now, heartbeat), now);
			budget.sent.push_back(now);
		}
		assert_eq!(budget.earliest(now, heartbeat), now + WINDOW);
		assert_eq!(budget.earliest(now + WINDOW, normal), now + WINDOW);
	}
	#[tokio::test]
	async fn blocked_writer_does_not_block_owner_and_only_reports_actual_heartbeat_sends() {
		let (sink, ready, wake, frames) = fixture();
		let mut writer = Outgoing::start(sink, Duration::from_secs(45));
		writer
			.send(frame(serde_json::json!({"op":3,"d":{"status":"online"}})))
			.await
			.unwrap();
		tokio::task::yield_now().await;
		writer
			.send(frame(serde_json::json!({"op":1,"d":42})))
			.await
			.unwrap();
		assert!(writer.progress.borrow().heartbeat.is_none());
		// The owner's task remains usable while the socket write is deliberately stuck.
		for _ in 0..5 {
			tokio::task::yield_now().await;
		}
		assert!(frames.lock().unwrap().is_empty());
		ready.store(true, Ordering::Release);
		if let Some(wake) = wake.lock().unwrap().take() {
			wake.wake();
		}
		timeout(Duration::from_secs(1), async {
			loop {
				writer.progress.changed().await.unwrap();
				if writer.progress.borrow().heartbeat.is_some() {
					break;
				}
			}
		})
		.await
		.unwrap();
		assert!(writer.progress.borrow().heartbeat.unwrap().1 <= Instant::now());
		assert_eq!(frames.lock().unwrap().len(), 2);
	}
	#[tokio::test]
	async fn rejected_replacements_preserve_queued_updates() {
		let (sink, _, _, _) = fixture();
		let writer = Outgoing::start(sink, Duration::from_secs(45));
		let presence = frame(serde_json::json!({"op":3,"d":{"status":"online"}}));
		writer.send(presence.clone()).await.unwrap();
		let original = writer.occupancy();
		for size in [55 * 1024, BYTES + 1] {
			let replacement = frame(serde_json::json!({"op":3,"d":{"status":"x".repeat(size)}}));
			assert!(writer.send(replacement).await.is_err());
			assert_eq!(writer.occupancy(), original);
			assert_eq!(writer.state.lock().unwrap().queue[0].frame, presence);
		}

		let subscription = crate::subscription_packet(Id(2), true, None, false);
		writer.send(subscription.clone()).await.unwrap();
		let original = writer.occupancy();
		let cancellation = frame(serde_json::json!({
			"op":37,"d":{"subscriptions":{"2":{"typing":false,"padding":"x".repeat(BYTES)}}}
		}));
		assert!(writer.send(cancellation).await.is_err());
		assert_eq!(writer.occupancy(), original);
		assert_eq!(writer.state.lock().unwrap().queue[1].frame, subscription);
	}

	#[tokio::test]
	async fn pending_presence_and_viewports_coalesce_without_losing_initial_subscription() {
		let (sink, ready, wake, frames) = fixture();
		let mut writer = Outgoing::start(sink, Duration::from_secs(45));
		for value in 0..20 {
			writer
				.send(frame(serde_json::json!({"op":3,"d":{"value":value}})))
				.await
				.unwrap();
		}
		assert_eq!(writer.occupancy().0, 1);
		writer
			.send(crate::subscription_packet(Id(2), true, None, false))
			.await
			.unwrap();
		writer
			.send(crate::subscription_packet(
				Id(2),
				true,
				Some((Id(3), &[[0, 99]])),
				false,
			))
			.await
			.unwrap();
		writer.clear_subscriptions();
		writer
			.send(crate::subscription_packet(
				Id(2),
				true,
				Some((Id(3), &[[100, 199]])),
				false,
			))
			.await
			.unwrap();
		assert_eq!(writer.occupancy().0, 3);
		ready.store(true, Ordering::Release);
		if let Some(wake) = wake.lock().unwrap().take() {
			wake.wake();
		}
		timeout(Duration::from_secs(1), async {
			while writer.occupancy().0 > 0 {
				writer.progress.changed().await.unwrap();
			}
		})
		.await
		.unwrap();
		let frames = frames.lock().unwrap();
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(frames[0].to_text().unwrap()).unwrap()["d"]["value"],
			19
		);
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(frames[1].to_text().unwrap()).unwrap()["d"]["subscriptions"]
				["2"]["channels"],
			serde_json::json!({})
		);
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(frames[2].to_text().unwrap()).unwrap()["d"]["subscriptions"]
				["2"]["channels"],
			serde_json::json!({"3":[[100,199]]})
		);
	}
	#[tokio::test]
	async fn member_identity_updates_coalesce_without_resetting_range_progress() {
		let (sink, ready, wake, frames) = fixture();
		let mut writer = Outgoing::start(sink, Duration::from_secs(45));
		for user in 1..=5 {
			writer
				.send(frame(
					serde_json::json!({"op":37,"d":{"subscriptions":{"2":{"typing":true,"members":[user.to_string()]}}}}),
				))
				.await
				.unwrap();
		}
		assert_eq!(writer.occupancy().0, 1);
		assert_eq!(writer.subscription_ticket(), 0);
		writer
			.send(crate::subscription_packet(
				Id(2),
				true,
				Some((Id(3), &[[0, 99]])),
				false,
			))
			.await
			.unwrap();
		let ticket = writer.subscription_ticket();
		assert_ne!(ticket, 0);
		ready.store(true, Ordering::Release);
		if let Some(wake) = wake.lock().unwrap().take() {
			wake.wake();
		}
		timeout(Duration::from_secs(1), async {
			while writer.progress.borrow().subscription.is_none() {
				writer.progress.changed().await.unwrap();
			}
		})
		.await
		.unwrap();
		assert_eq!(writer.progress.borrow().subscription.unwrap().0, ticket);
		let frames = frames.lock().unwrap();
		assert_eq!(frames.len(), 2);
		let identity: serde_json::Value =
			serde_json::from_str(frames[0].to_text().unwrap()).unwrap();
		assert_eq!(
			identity["d"]["subscriptions"]["2"]["members"],
			serde_json::json!(["5"])
		);
	}
	#[tokio::test]
	async fn queue_limits_preserve_critical_reserve_and_never_discard_a_stop() {
		let (sink, _, _, _) = fixture();
		let writer = Outgoing::start(sink, Duration::from_secs(45));
		for guild in 1..=12 {
			writer
				.send(crate::subscription_packet(Id(guild), true, None, false))
				.await
				.unwrap();
		}
		let stop = frame(serde_json::json!({"op":19,"d":{"stream_key":"synthetic-only"}}));
		writer.send(stop).await.unwrap();
		writer
			.send(frame(serde_json::json!({"op":1,"d":1})))
			.await
			.unwrap();
		{
			let state = writer.state.lock().unwrap();
			assert!(state.queue.iter().any(|p| p.meta.opcode == Some(19)));
			assert_eq!(state.queue.len(), 14);
			assert!(state.bytes <= BYTES);
		}
		assert!(
			writer
				.send(Frame::Text("x".repeat(BYTES + 1).into()))
				.await
				.is_err()
		);
		assert!(!writer.progress.borrow().failed);
		assert!(writer.occupancy().0 <= PACKETS && writer.occupancy().1 <= BYTES);
	}
	#[tokio::test]
	async fn a_join_batch_keeps_heartbeat_headroom_when_the_writer_is_blocked() {
		for occupied in [12, 13] {
			let (sink, _, _, _) = fixture();
			let writer = Outgoing::start(sink, Duration::from_secs(45));
			for _ in 0..occupied {
				writer
					.send(frame(serde_json::json!({"op":4,"d":{}})))
					.await
					.unwrap();
			}
			tokio::task::yield_now().await;
			assert_eq!(writer.occupancy().0, occupied);
			assert_eq!(writer.control_capacity(), occupied == 12);
			if writer.control_capacity() {
				// The same admission guard protects the production Sync/Join/info batch.
				for opcode in [12, 4, 17] {
					writer
						.send(frame(serde_json::json!({"op":opcode,"d":{}})))
						.await
						.unwrap();
				}
				writer
					.send(frame(serde_json::json!({"op":1,"d":null})))
					.await
					.unwrap();
				assert_eq!(writer.occupancy().0, PACKETS);
			}
			assert!(!writer.progress.borrow().failed);
		}
	}
	#[tokio::test(start_paused = true)]
	async fn delayed_numeric_search_and_author_lookup_keep_their_explicit_slots() {
		use crate::member_search::{Purpose, Query};
		let (sink, ready, _, frames) = fixture();
		ready.store(true, Ordering::Release);
		let mut writer = Outgoing::start(sink, Duration::from_secs(45));
		writer.cooldown(8, None, Duration::from_secs(60));
		let query = |purpose, guild, nonce| Query {
			purpose,
			// Numeric composer searches and author lookups both encode user_ids.
			frame: frame(serde_json::json!({"op":8,"d":{
				"guild_id":guild,"nonce":nonce,"user_ids":["123456789"]
			}})),
		};
		writer
			.send_query(query(Purpose::Composer, "2", "1"))
			.await
			.unwrap();
		writer
			.send_query(query(Purpose::Authors, "3", "2"))
			.await
			.unwrap();
		writer.clear_queries(1 << Purpose::Composer.slot());
		assert_eq!(writer.occupancy().0, 1);
		writer
			.send_query(query(Purpose::Composer, "2", "3"))
			.await
			.unwrap();
		assert_eq!(writer.occupancy().0, 2);
		tokio::task::yield_now().await;
		assert!(frames.lock().unwrap().is_empty());
		tokio::time::advance(Duration::from_secs(60)).await;
		timeout(Duration::from_secs(1), async {
			while writer.occupancy().0 > 0 {
				writer.progress.changed().await.unwrap();
			}
		})
		.await
		.unwrap();
		let nonces: Vec<_> = frames
			.lock()
			.unwrap()
			.iter()
			.map(|frame| {
				serde_json::from_str::<serde_json::Value>(frame.to_text().unwrap()).unwrap()["d"]
					["nonce"]
					.as_str()
					.unwrap()
					.to_owned()
			})
			.collect();
		assert_eq!(nonces, ["2", "3"]);
		assert!(!writer.progress.borrow().failed);
	}
	#[tokio::test]
	async fn unobserved_author_receipt_survives_multiple_composer_completions() {
		use crate::member_search::{Purpose, Query};
		let (sink, ready, _, frames) = fixture();
		ready.store(true, Ordering::Release);
		let writer = Outgoing::start(sink, Duration::from_secs(45));
		for (completed, purpose, guild, nonce) in [
			(1, Purpose::Authors, 2, 10),
			(2, Purpose::Composer, 3, 11),
			(3, Purpose::Composer, 4, 12),
		] {
			writer
				.send_query(Query {
					purpose,
					frame: frame(serde_json::json!({"op":8,"d":{
						"guild_id":guild.to_string(),"nonce":nonce.to_string(),"user_ids":["123456789"]
					}})),
				})
				.await
				.unwrap();
			// Deliberately do not read/acknowledge the progress watch until all
			// three actual sends complete. Different guilds have independent limits.
			timeout(Duration::from_secs(1), async {
				while frames.lock().unwrap().len() < completed {
					tokio::task::yield_now().await;
				}
			})
			.await
			.unwrap();
		}
		let progress = *writer.progress.borrow();
		assert_eq!(
			progress.query[Purpose::Authors.slot()].map(|(guild, nonce, _)| (guild, nonce)),
			Some((Id(2), 10))
		);
		assert_eq!(
			progress.query[Purpose::Composer.slot()].map(|(guild, nonce, _)| (guild, nonce)),
			Some((Id(4), 12))
		);
		let limits = writer.limits.lock().unwrap();
		for guild in [2, 3, 4] {
			assert!(limits.deadlines[&(8, Some(Id(guild)))] > Instant::now());
		}
	}
	#[tokio::test]
	async fn each_requested_heartbeat_has_its_own_send_ticket() {
		let (sink, ready, wake, frames) = fixture();
		let mut writer = Outgoing::start(sink, Duration::from_secs(45));
		writer
			.send(frame(serde_json::json!({"op":1,"d":1})))
			.await
			.unwrap();
		let first = writer.heartbeat_ticket();
		writer
			.send(frame(serde_json::json!({"op":1,"d":2})))
			.await
			.unwrap();
		let second = writer.heartbeat_ticket();
		assert!(
			second > first,
			"server requests and recovery probes cannot stand in for each other"
		);
		assert_eq!(writer.occupancy().0, 2);
		ready.store(true, Ordering::Release);
		if let Some(wake) = wake.lock().unwrap().take() {
			wake.wake();
		}
		timeout(Duration::from_secs(1), async {
			loop {
				writer.progress.changed().await.unwrap();
				if writer
					.progress
					.borrow()
					.heartbeat
					.is_some_and(|(ticket, _)| ticket == second)
				{
					break;
				}
			}
		})
		.await
		.unwrap();
		assert_eq!(frames.lock().unwrap().len(), 2);
		assert_eq!(writer.take_heartbeat().unwrap().ticket, first);
		assert_eq!(writer.take_heartbeat().unwrap().ticket, second);
		assert!(writer.take_heartbeat().is_none());
	}
	#[tokio::test]
	async fn cooldown_overflow_defers_optional_work_without_holding_an_unrelated_stop() {
		let (sink, ready, _, frames) = fixture();
		ready.store(true, Ordering::Release);
		let mut writer = Outgoing::start(sink, Duration::from_secs(45));
		{
			let mut limits = writer.limits.lock().unwrap();
			limits.blocked_until = Some(Instant::now() + Duration::from_secs(60));
		}
		writer
			.send(frame(
				serde_json::json!({"op":8,"d":{"guild_id":"2","nonce":"1","query":"synthetic"}}),
			))
			.await
			.unwrap();
		writer
			.send(frame(
				serde_json::json!({"op":19,"d":{"stream_key":"synthetic-only"}}),
			))
			.await
			.unwrap();
		timeout(Duration::from_secs(1), async {
			while frames.lock().unwrap().is_empty() {
				writer.progress.changed().await.unwrap();
			}
		})
		.await
		.unwrap();
		let sent = frames.lock().unwrap();
		assert_eq!(sent.len(), 1);
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(sent[0].to_text().unwrap()).unwrap()["op"],
			19
		);
		assert_eq!(writer.occupancy().0, 1);
	}
}
