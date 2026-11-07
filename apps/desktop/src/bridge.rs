//! Local WebSocket bridge that forwards incoming chat to the paired phone app.
//!
//! The bridge is opt-in. It listens on the LAN so an iPhone on the same network can reach
//! it, requires a shared token, and never exposes anything but channel names, author
//! display names and bounded message text. It never sends: the phone app is read-only.

use eframe::egui;
use model::{Channel, Guild, Id, Message};
use std::{
	collections::{BTreeMap, VecDeque},
	net::{Ipv4Addr, SocketAddr},
	sync::{Arc, Mutex},
	time::Duration,
};
use tokio::{
	net::TcpListener,
	sync::{broadcast, watch},
};
use tokio_tungstenite::tungstenite::handshake::server::{Request as HandshakeRequest, Response};

/// Bounded fan-out: a slow phone drops its oldest frames instead of growing memory.
const FRAME_BACKLOG: usize = 128;
/// Concurrent phones; the owner has one, and this bounds a misconfigured LAN.
const MAX_CLIENTS: usize = 4;
/// One frame stays small enough for a phone notification payload.
const MAX_CONTENT: usize = 512;
const MAX_NAME: usize = 96;
/// Channels announced in one snapshot; the sidebar never holds more.
const MAX_CHANNELS: usize = 200;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Cached messages replayed per channel, and across all channels, when a phone connects.
const REPLAY_PER_CHANNEL: usize = 20;
const REPLAY_TOTAL: usize = 400;
/// Phone requests waiting for the client to act on them, and the longest message it may send.
const MAX_REQUESTS: usize = 32;
const MAX_SEND: usize = 2000;

/// State every connection shares.
#[derive(Clone)]
struct Shared {
	token: Arc<str>,
	frames: broadcast::Sender<Arc<str>>,
	channels: Arc<Mutex<BTreeMap<u64, String>>>,
	account: Arc<Mutex<Option<u64>>>,
	requests: Arc<Mutex<VecDeque<Request>>>,
	wake: Arc<Mutex<Option<egui::Context>>>,
}

/// One thing the phone asked the client to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
	/// Load and forward a channel's recent history.
	Open { channel: u64 },
	/// Post `content` into `channel`, acknowledged with `nonce`.
	Send {
		channel: u64,
		nonce: String,
		content: String,
	},
}

#[derive(Default)]
pub struct Bridge {
	handle: Option<Handle>,
	/// Requests the phone sent, drained by the client each frame.
	requests: Arc<Mutex<VecDeque<Request>>>,
	/// Wakes the interface so a request is drained without waiting for the next repaint.
	wake: Arc<Mutex<Option<egui::Context>>>,
	/// Client nonce -> phone nonce, so an acknowledgement names the phone's own request.
	expected: Arc<Mutex<BTreeMap<String, String>>>,
	/// Preferences the running listener was started with.
	running: Option<Config>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
	pub enabled: bool,
	pub port: u16,
	pub token: String,
}

struct Handle {
	address: SocketAddr,
	frames: broadcast::Sender<Arc<str>>,
	stop: watch::Sender<bool>,
	/// Latest channel row per channel, replayed to each phone that connects.
	channels: Arc<Mutex<BTreeMap<u64, String>>>,
	/// Signed-in account whose cached history the replay reads.
	account: Arc<Mutex<Option<u64>>>,
}

impl Bridge {
	/// Applies preferences, starting or stopping the listener when they changed.
	pub fn configure(&mut self, runtime: &tokio::runtime::Handle, config: Config) {
		if self.running.as_ref() == Some(&config) {
			return;
		}
		self.stop();
		self.running = Some(config.clone());
		if !config.enabled || config.token.is_empty() || config.port < 1024 {
			return;
		}
		let (frames, _) = broadcast::channel(FRAME_BACKLOG);
		let (stop, stopped) = watch::channel(false);
		let token = Arc::<str>::from(config.token.as_str());
		let address = SocketAddr::from((Ipv4Addr::UNSPECIFIED, config.port));
		let sender = frames.clone();
		let channels = Arc::new(Mutex::new(BTreeMap::new()));
		let known = channels.clone();
		let account = Arc::new(Mutex::new(None));
		let signed_in = account.clone();
		let requests = self.requests.clone();
		let queue = requests.clone();
		let wake = self.wake.clone();
		runtime.spawn(async move {
			let shared = Shared {
				token,
				frames: sender,
				channels: known,
				account: signed_in,
				requests: queue,
				wake,
			};
			if let Err(error) = serve(address, shared, stopped).await {
				diagnostics::record_fields(
					diagnostics::Category::Session,
					diagnostics::Level::Error,
					"phone bridge stopped",
					&[("reason", error.into())],
				);
			}
		});
		self.handle = Some(Handle {
			address,
			frames,
			stop,
			channels,
			account,
		});
	}

	fn stop(&mut self) {
		if let Some(handle) = self.handle.take() {
			let _ = handle.stop.send(true);
		}
	}

	/// The port the listener bound, once it is running.
	pub fn port(&self) -> Option<u16> {
		self.handle.as_ref().map(|handle| handle.address.port())
	}

	/// Gives the bridge a way to wake the interface when the phone asks for something.
	pub fn set_wake(&mut self, ctx: egui::Context) {
		if let Ok(mut wake) = self.wake.lock() {
			*wake = Some(ctx);
		}
	}

	/// Takes the requests the phone queued since the last frame.
	pub fn take_requests(&self) -> Vec<Request> {
		self.requests
			.lock()
			.map(|mut requests| requests.drain(..).collect())
			.unwrap_or_default()
	}

	/// Remembers which phone request a queued send belongs to.
	pub fn expect(&self, client_nonce: &str, phone_nonce: &str) {
		if let Ok(mut expected) = self.expected.lock() {
			if expected.len() >= MAX_REQUESTS {
				expected.clear();
			}
			expected.insert(client_nonce.to_owned(), phone_nonce.to_owned());
		}
	}

	/// Acknowledges the phone request that produced `client_nonce`.
	pub fn acknowledge_client(&self, client_nonce: &str, error: Option<&str>) {
		let phone_nonce = self
			.expected
			.lock()
			.ok()
			.and_then(|mut expected| expected.remove(client_nonce));
		if let Some(phone_nonce) = phone_nonce {
			self.acknowledge(&phone_nonce, error);
		}
	}

	/// Tells the phone whether its message was accepted.
	pub fn acknowledge(&self, nonce: &str, error: Option<&str>) {
		let Some(handle) = &self.handle else {
			return;
		};
		let frame = match error {
			None => format!(
				r#"{{"type":"sent","nonce":{}}}"#,
				json_string(&bounded(nonce, MAX_NAME))
			),
			Some(error) => format!(
				r#"{{"type":"send_failed","nonce":{},"error":{}}}"#,
				json_string(&bounded(nonce, MAX_NAME)),
				json_string(&bounded(error, MAX_NAME))
			),
		};
		let _ = handle.frames.send(Arc::from(frame.as_str()));
	}

	/// Announces a channel so the phone can group messages by server or friend.
	pub fn announce(&self, state: &client_core::State, channel: Id) {
		let Some(handle) = &self.handle else {
			return;
		};
		let Some(channel) = state.channel(channel) else {
			return;
		};
		let row = channel_row(state, channel);
		let frame = channel_frame(&row);
		if let Ok(mut channels) = handle.channels.lock() {
			channels.insert(channel.id.0, row);
		}
		let _ = handle.frames.send(Arc::from(frame.as_str()));
	}

	/// Announces every channel the phone groups by, after a login or resync.
	pub fn announce_all(&self, state: &client_core::State) {
		let Some(handle) = &self.handle else {
			return;
		};
		if let Ok(mut account) = handle.account.lock() {
			*account = state.user.as_ref().map(|user| user.id.0);
		}
		for id in grouped_channels(state) {
			self.announce(state, id);
		}
	}

	/// Forwards one received message. Called after the client applied it to state.
	pub fn publish(&self, state: &client_core::State, message: &Message) {
		let Some(handle) = &self.handle else {
			return;
		};
		let channel = state
			.channel(message.channel)
			.map(|channel| channel_row(state, channel));
		let frame = message_frame(channel.as_deref(), message);
		let _ = handle.frames.send(Arc::from(frame.as_str()));
	}
}

fn bounded(text: &str, limit: usize) -> String {
	let mut out = String::with_capacity(text.len().min(limit));
	for character in text.chars() {
		if out.len() + character.len_utf8() > limit {
			break;
		}
		out.push(character);
	}
	out
}

/// A channel row: server channels carry their server, DMs carry the other person.
fn channel_row(state: &client_core::State, channel: &Channel) -> String {
	let guild = channel
		.guild
		.and_then(|id| state.guild(id))
		.map(|guild: &Guild| bounded(&guild.name, MAX_NAME));
	let name = if channel.kind == 1 || channel.kind == 3 {
		channel
			.recipients
			.first()
			.map(|user| bounded(&user.name, MAX_NAME))
			.unwrap_or_else(|| bounded(&channel.name, MAX_NAME))
	} else {
		bounded(&channel.name, MAX_NAME)
	};
	format!(
		r#"{{"id":"{}","kind":{},"name":{},"guild":{},"voice":{}}}"#,
		channel.id.0,
		channel.kind,
		json_string(&name),
		guild.map_or_else(|| "null".into(), |guild| json_string(&guild)),
		channel.kind == 2
	)
}

fn channel_frame(row: &str) -> String {
	format!(r#"{{"type":"channel","channel":{row}}}"#)
}

fn message_frame(channel_row: Option<&str>, message: &Message) -> String {
	let author = message
		.author_nick
		.as_deref()
		.filter(|nick| !nick.is_empty())
		.map(str::to_owned)
		.unwrap_or_else(|| message.author.name.clone());
	format!(
		r#"{{"type":"message","channel":{channel},"message":{{"id":"{}","author":{},"content":{},"edited":{}}}}}"#,
		message.id.0,
		json_string(&bounded(&author, MAX_NAME)),
		json_string(&bounded(&message.content, MAX_CONTENT)),
		message.edited,
		channel = channel_row.unwrap_or("null"),
	)
}

/// Minimal JSON string escaping; the frame is hand-built to keep the payload small.
fn json_string(value: &str) -> String {
	let mut out = String::with_capacity(value.len() + 2);
	out.push('"');
	for character in value.chars() {
		match character {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			character if (character as u32) < 0x20 => {
				out.push_str(&format!("\\u{:04x}", character as u32))
			}
			character => out.push(character),
		}
	}
	out.push('"');
	out
}

async fn serve(
	address: SocketAddr,
	shared: Shared,
	stop: watch::Receiver<bool>,
) -> Result<&'static str, &'static str> {
	let listener = TcpListener::bind(address)
		.await
		.map_err(|_| "could not listen")?;
	serve_on(listener, shared, stop).await
}

async fn serve_on(
	listener: TcpListener,
	shared: Shared,
	mut stop: watch::Receiver<bool>,
) -> Result<&'static str, &'static str> {
	let clients = Arc::new(tokio::sync::Semaphore::new(MAX_CLIENTS));
	loop {
		tokio::select! {
			_ = stop.changed() => return Ok("stopped"),
			accepted = listener.accept() => {
				let Ok((stream, _)) = accepted else { continue };
				let Ok(permit) = clients.clone().try_acquire_owned() else { continue };
				let shared = shared.clone();
				let stop = stop.clone();
				tokio::spawn(async move {
					let _permit = permit;
					let _ = client(stream, shared, stop).await;
				});
			}
		}
	}
}

// The handshake callback's error type is fixed by the WebSocket library.
#[allow(clippy::result_large_err)]
async fn client(
	stream: tokio::net::TcpStream,
	shared: Shared,
	mut stop: watch::Receiver<bool>,
) -> Result<(), &'static str> {
	let _ = stream.set_nodelay(true);
	let token_for_check = shared.token.clone();
	let handshake = tokio::time::timeout(
		HANDSHAKE_TIMEOUT,
		tokio_tungstenite::accept_hdr_async(
			stream,
			move |request: &HandshakeRequest, response: Response| {
				if authorized(request.uri().query(), &token_for_check) {
					Ok(response)
				} else {
					Err(
						tokio_tungstenite::tungstenite::handshake::server::ErrorResponse::new(
							Some("unauthorized".into()),
						),
					)
				}
			},
		),
	)
	.await
	.map_err(|_| "handshake timed out")?
	.map_err(|_| "handshake rejected")?;
	let (mut sink, mut source) = handshake.split();
	use futures_util::{SinkExt, StreamExt};
	// Replay the channels and cached messages the phone missed before it connected.
	let known: Vec<(u64, String)> = shared
		.channels
		.lock()
		.map(|channels| {
			channels
				.iter()
				.map(|(id, row)| (*id, row.clone()))
				.collect()
		})
		.unwrap_or_default();
	for (_, row) in &known {
		let frame = channel_frame(row);
		if sink
			.send(tokio_tungstenite::tungstenite::Message::Text(frame.into()))
			.await
			.is_err()
		{
			return Ok(());
		}
	}
	let signed_in = shared.account.lock().ok().and_then(|account| *account);
	if let Some(account) = signed_in
		&& let Ok(replay) = replay_frames(account, &known).await
	{
		for frame in replay {
			if sink
				.send(tokio_tungstenite::tungstenite::Message::Text(frame.into()))
				.await
				.is_err()
			{
				return Ok(());
			}
		}
	}
	let mut frames = shared.frames.subscribe();
	loop {
		tokio::select! {
			_ = stop.changed() => return Ok(()),
			incoming = source.next() => match incoming {
				None | Some(Err(_)) | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => return Ok(()),
				Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
					if let Some(request) = parse_request(&text)
						&& let Ok(mut queue) = shared.requests.lock()
						&& queue.len() < MAX_REQUESTS
					{
						queue.push_back(request);
						drop(queue);
						if let Some(wake) = shared
							.wake
							.lock()
							.ok()
							.and_then(|wake| wake.clone())
						{
							wake.request_repaint();
						}
					}
				}
				Some(Ok(_)) => {}
			},
			frame = frames.recv() => match frame {
				Ok(frame) => {
					if sink
						.send(tokio_tungstenite::tungstenite::Message::Text(frame.as_ref().into()))
						.await
						.is_err()
					{
						return Ok(());
					}
				}
				Err(broadcast::error::RecvError::Lagged(_)) => {}
				Err(broadcast::error::RecvError::Closed) => return Ok(()),
			},
		}
	}
}

/// Reads one phone request. Unknown or malformed frames are ignored.
fn parse_request(text: &str) -> Option<Request> {
	let value: serde_json::Value = serde_json::from_str(text).ok()?;
	let channel = value.get("channel")?.as_str()?.parse().ok()?;
	match value.get("type")?.as_str()? {
		"open" => Some(Request::Open { channel }),
		"send" => {
			let content = value.get("content")?.as_str()?.trim();
			if content.is_empty() || content.len() > MAX_SEND {
				return None;
			}
			let nonce = value.get("nonce")?.as_str()?.to_owned();
			if nonce.is_empty() || nonce.len() > MAX_NAME {
				return None;
			}
			Some(Request::Send {
				channel,
				nonce,
				content: content.to_owned(),
			})
		}
		_ => None,
	}
}

/// Cached history for the channels the phone is about to see, oldest first.
async fn replay_frames(account: u64, channels: &[(u64, String)]) -> Result<Vec<String>, ()> {
	let channels = channels.to_vec();
	tokio::task::spawn_blocking(move || {
		let store = local_store::LocalStore::open_default().ok()?;
		let mut frames = Vec::new();
		for (id, row) in channels {
			if frames.len() >= REPLAY_TOTAL {
				break;
			}
			let Ok(messages) = store.load_channel(model::Id(account), model::Id(id)) else {
				continue;
			};
			let start = messages.len().saturating_sub(REPLAY_PER_CHANNEL);
			for message in &messages[start..] {
				frames.push(message_frame(Some(&row), message));
			}
		}
		Some(frames)
	})
	.await
	.map_err(|_| ())?
	.ok_or(())
}

/// `?token=<value>` must match exactly.
fn authorized(query: Option<&str>, token: &str) -> bool {
	let Some(query) = query else {
		return false;
	};
	let mut presented = None;
	for pair in query.split('&') {
		if let Some(value) = pair.strip_prefix("token=") {
			presented = Some(value);
		}
	}
	let Some(presented) = presented else {
		return false;
	};
	// Compare without an early exit so the token length stays unobservable.
	let mut difference = presented.len() ^ token.len();
	for (left, right) in presented.bytes().zip(token.bytes()) {
		difference |= usize::from(left ^ right);
	}
	difference == 0
}

/// Channels the phone groups by, newest activity first.
pub fn grouped_channels(state: &client_core::State) -> Vec<Id> {
	let mut channels: Vec<_> = state
		.channels
		.iter()
		.filter(|channel| matches!(channel.kind, 0..=3))
		.map(|channel| (channel.last_message.unwrap_or(Id(0)), channel.id))
		.collect();
	channels.sort_by(|left, right| right.cmp(left));
	channels
		.into_iter()
		.map(|(_, id)| id)
		.take(MAX_CHANNELS)
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	use futures_util::StreamExt as _;

	async fn pair() -> (
		SocketAddr,
		Arc<str>,
		broadcast::Sender<Arc<str>>,
		watch::Sender<bool>,
	) {
		let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
		let address = listener.local_addr().unwrap();
		let token: Arc<str> = Arc::from("abc123");
		let (frames, _) = broadcast::channel(FRAME_BACKLOG);
		let (stop, stopped) = watch::channel(false);
		let channels = Arc::new(Mutex::new(BTreeMap::new()));
		tokio::spawn(serve_on(
			listener,
			Shared {
				token: token.clone(),
				frames: frames.clone(),
				channels,
				account: Arc::new(Mutex::new(None)),
				requests: Arc::new(Mutex::new(VecDeque::new())),
				wake: Arc::new(Mutex::new(None)),
			},
			stopped,
		));
		(address, token, frames, stop)
	}

	#[tokio::test]
	async fn authorized_client_receives_frames() {
		let (address, token, frames, stop) = pair().await;
		let url = format!("ws://{address}/?token={token}");
		let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
		// The subscription exists once the handshake finished; publish after a short yield.
		tokio::task::yield_now().await;
		frames.send(Arc::from(r#"{"type":"message"}"#)).unwrap();
		let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
			.await
			.unwrap()
			.unwrap()
			.unwrap();
		assert_eq!(message.into_text().unwrap(), r#"{"type":"message"}"#);
		let _ = stop.send(true);
	}

	#[tokio::test]
	async fn wrong_token_is_rejected() {
		let (address, _, _, stop) = pair().await;
		let url = format!("ws://{address}/?token=wrong");
		assert!(tokio_tungstenite::connect_async(url).await.is_err());
		let _ = stop.send(true);
	}

	#[test]
	fn phone_requests_are_parsed_and_bounded() {
		assert_eq!(
			parse_request(r#"{"type":"open","channel":"42"}"#),
			Some(Request::Open { channel: 42 })
		);
		assert_eq!(
			parse_request(r#"{"type":"send","channel":"42","nonce":"n1","content":" hi "}"#),
			Some(Request::Send {
				channel: 42,
				nonce: "n1".into(),
				content: "hi".into()
			})
		);
		assert_eq!(
			parse_request(r#"{"type":"send","channel":"42","nonce":"n","content":"  "}"#),
			None
		);
		assert_eq!(
			parse_request(&format!(
				r#"{{"type":"send","channel":"42","nonce":"n","content":"{}"}}"#,
				"x".repeat(MAX_SEND + 1)
			)),
			None
		);
		assert_eq!(parse_request(r#"{"type":"delete","channel":"42"}"#), None);
		assert_eq!(parse_request("not json"), None);
	}

	#[test]
	fn token_must_match_exactly() {
		assert!(authorized(Some("token=abc"), "abc"));
		assert!(authorized(Some("a=1&token=abc"), "abc"));
		assert!(!authorized(Some("token=abcd"), "abc"));
		assert!(!authorized(Some("token=ab"), "abc"));
		assert!(!authorized(Some("token="), "abc"));
		assert!(!authorized(Some("abc"), "abc"));
		assert!(!authorized(None, "abc"));
	}

	#[test]
	fn json_strings_escape_control_characters() {
		assert_eq!(json_string("a\"b\\c"), r#""a\"b\\c""#);
		assert_eq!(json_string("line\nbreak"), r#""line\nbreak""#);
		assert_eq!(json_string("\u{1}"), r#""\u0001""#);
	}

	#[tokio::test]
	async fn publish_sends_a_message_frame_with_its_channel() {
		let state = test_support::demo_state();
		let channel = state
			.channels
			.iter()
			.find(|channel| channel.kind == 0)
			.expect("fixture has a text channel");
		let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
		let address = listener.local_addr().unwrap();
		let token: Arc<str> = Arc::from("abc123");
		let (frames, _) = broadcast::channel(FRAME_BACKLOG);
		let (stop, stopped) = watch::channel(false);
		let channels = Arc::new(Mutex::new(BTreeMap::new()));
		tokio::spawn(serve_on(
			listener,
			Shared {
				token: token.clone(),
				frames: frames.clone(),
				channels: channels.clone(),
				account: Arc::new(Mutex::new(None)),
				requests: Arc::new(Mutex::new(VecDeque::new())),
				wake: Arc::new(Mutex::new(None)),
			},
			stopped,
		));
		let bridge = Bridge {
			handle: Some(Handle {
				address,
				frames,
				stop,
				channels,
				account: Arc::new(Mutex::new(None)),
			}),
			requests: Arc::new(Mutex::new(VecDeque::new())),
			wake: Arc::new(Mutex::new(None)),
			expected: Arc::new(Mutex::new(BTreeMap::new())),
			running: None,
		};
		let (mut socket, _) =
			tokio_tungstenite::connect_async(format!("ws://{address}/?token={token}"))
				.await
				.unwrap();
		tokio::task::yield_now().await;
		bridge.publish(&state, &test_support::message(9, channel.id));
		let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
			.await
			.unwrap()
			.unwrap()
			.unwrap()
			.into_text()
			.unwrap();
		assert!(
			frame.starts_with(r#"{"type":"message","channel":{"id":"#),
			"{frame}"
		);
		assert!(
			frame.contains(&format!(r#""id":"{}""#, channel.id.0)),
			"{frame}"
		);
	}

	#[test]
	fn frames_carry_grouping_names_and_bounds() {
		let state = test_support::demo_state();
		let channel = state
			.channels
			.iter()
			.find(|channel| channel.kind == 0)
			.expect("fixture has a text channel");
		let frame = channel_frame(&channel_row(&state, channel));
		assert!(
			frame.starts_with(r#"{"type":"channel","channel":{"id":""#),
			"{frame}"
		);
		assert!(frame.contains(r#""voice":false"#), "{frame}");
		let guild = state
			.guild(channel.guild.expect("text channel belongs to a server"))
			.expect("fixture guild");
		assert!(frame.contains(&json_string(&guild.name)), "{frame}");

		let mut message = test_support::message(9, channel.id);
		message.content = "x".repeat(MAX_CONTENT * 2);
		let row = channel_row(&state, channel);
		let frame = message_frame(Some(&row), &message);
		assert!(
			frame.starts_with(r#"{"type":"message","channel":{"id":"#),
			"{frame}"
		);
		assert!(frame.contains(r#""author":"#), "{frame}");
		assert!(
			!frame.contains(&"x".repeat(MAX_CONTENT + 1)),
			"content is bounded"
		);
		assert!(
			frame.len() < MAX_CONTENT + 2048,
			"frame stays small: {}",
			frame.len()
		);
	}

	#[test]
	fn bounded_keeps_whole_characters() {
		assert_eq!(bounded("abcdef", 4), "abcd");
		assert_eq!(bounded("aé", 2), "a");
		assert_eq!(bounded("aé", 3), "aé");
	}
}
