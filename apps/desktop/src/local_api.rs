//! Opt-in local API for other programs on this computer. Protocol: `docs/local-api.md`.
//!
//! A program that holds the owner's access key opens a pipe or socket only this OS user can
//! reach, proves the key without sending it, and is then pushed session and call state.
//! Changing voice needs a second opt-in. There is no TCP port, so a web page or another
//! computer cannot connect. State stays in memory, and nothing reaches Discord except the
//! mute and deafen an active call already reports.

use client_core::State;
use client_core::voice::{Participant, Phase};
use diagnostics::{Category, Level};
use hmac::{Hmac, Mac};
use model::Id;
use platform::game_activity::{Listener, Stream, private_endpoint};
use serde::{Serialize, Serializer};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
	io,
	sync::{Arc, Mutex, PoisonError},
	time::Duration,
};
use tokio::{
	io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
	sync::{Notify, mpsc, watch},
	task::JoinSet,
};
use ui::{LocalApiKey, LocalApiState, LocalApiStatus};
use zeroize::Zeroizing;

/// `\\.\pipe\serein-api` on Windows, `serein-api.sock` in the runtime directory elsewhere.
const ENDPOINT: &str = "serein-api";
const VERSION: u32 = 1;
const MAX_CLIENTS: usize = 8;
/// Requests are a few short fields; anything longer is not this protocol.
const MAX_REQUEST: usize = 4 * 1024;
const MAX_MEMBERS: usize = 64;
const NAME_CHARS: usize = 64;
const CLIENT_NAME_CHARS: usize = 32;
const NONCE_CHARS: usize = 64;
const HANDSHAKE: Duration = Duration::from_secs(5);
const WRITE: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_secs(1);

type Key = Arc<Zeroizing<String>>;
type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Identity {
	#[serde(serialize_with = "id_text")]
	id: u64,
	name: String,
	avatar: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Member {
	#[serde(serialize_with = "id_text")]
	user_id: u64,
	name: String,
	avatar: Option<String>,
	/// Playback percent on this device, 0–200.
	volume: u16,
	/// Silenced on this device only.
	muted: bool,
	self_mute: bool,
	self_deaf: bool,
	speaking: bool,
}

impl Member {
	/// Everything but `speaking`, which changes far more often and has its own event.
	fn same_settings(&self, other: &Self) -> bool {
		self.name == other.name
			&& self.avatar == other.avatar
			&& self.volume == other.volume
			&& self.muted == other.muted
			&& self.self_mute == other.self_mute
			&& self.self_deaf == other.self_deaf
	}
}

/// Snowflakes exceed what JavaScript numbers hold, so every ID is sent as text.
fn id_text<S: Serializer>(id: &u64, serializer: S) -> Result<S::Ok, S::Error> {
	serializer.collect_str(id)
}

fn optional_id(id: u64) -> Option<String> {
	(id != 0).then(|| id.to_string())
}

/// Everything the API may show. Nothing else in the session can be read through it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
	user: Option<Identity>,
	/// The owner's second opt-in: clients may change voice.
	control: bool,
	mute: bool,
	deaf: bool,
	/// Zero while not in a call.
	channel_id: u64,
	guild_id: u64,
	/// Sorted by user ID, own account included.
	members: Vec<Member>,
}

impl Snapshot {
	/// Builds the view from the signed-in session. A failed call and a signed-out window both
	/// read as not in a call, so a client drops its room instead of keeping a stale one.
	pub fn capture(state: &State, ui: &ui::MessagingUi) -> Self {
		let signed_in = state.user.is_some()
			&& (state.demo || state.auth == client_core::auth::AuthState::Authenticated);
		let Some(own) = state.user.as_ref().filter(|_| signed_in) else {
			return Self::default();
		};
		let call = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.phase != Phase::Failed);
		// Discord's indicator, and what the call sends: deafening always reads as muted.
		let (mute, deaf) = call.map_or(
			(ui.voice_muted || ui.voice_deafened, ui.voice_deafened),
			|call| (call.self_muted(), call.deafened),
		);
		let mut members = Vec::new();
		if let Some(call) = call {
			for participant in &call.participants {
				if participant.user.0 == 0 {
					continue;
				}
				if members.len() == MAX_MEMBERS {
					break;
				}
				members.push(member_of(state, ui, own.id, (mute, deaf), participant));
			}
			members.sort_by_key(|member| member.user_id);
			members.dedup_by_key(|member| member.user_id);
		}
		Self {
			user: Some(Identity {
				id: own.id.0,
				name: bounded_name(&own.name),
				avatar: avatar_hash(own),
			}),
			control: ui.local_api_control,
			mute,
			deaf,
			channel_id: call.map_or(0, |call| call.channel.0),
			guild_id: call.and_then(|call| call.guild).map_or(0, |id| id.0),
			members,
		}
	}

	fn session(&self) -> Value {
		json!({ "signed_in": self.user.is_some(), "user": self.user, "control": self.control })
	}

	fn voice(&self) -> Value {
		json!({
			"connected": self.channel_id != 0,
			"channel_id": optional_id(self.channel_id),
			"guild_id": optional_id(self.guild_id),
			"mute": self.mute,
			"deaf": self.deaf,
			"members": self.members,
		})
	}

	fn topic(&self, topic: Topic) -> Value {
		match topic {
			Topic::Session => self.session(),
			Topic::Voice => self.voice(),
		}
	}
}

fn member_of(
	state: &State,
	ui: &ui::MessagingUi,
	own: Id,
	(own_mute, own_deaf): (bool, bool),
	participant: &Participant,
) -> Member {
	let (name, avatar) = lookup(state, participant.user);
	let is_own = participant.user == own;
	Member {
		user_id: participant.user.0,
		name,
		avatar,
		volume: if is_own {
			100
		} else {
			ui.voice_user_volume_percent(participant.user)
		},
		muted: !is_own && ui.voice_user_mutes().contains(&participant.user.0),
		self_mute: if is_own { own_mute } else { participant.muted },
		self_deaf: if is_own {
			own_deaf
		} else {
			participant.deafened
		},
		speaking: ui.voice_speaking.contains(&participant.user),
	}
}

fn lookup(state: &State, user: Id) -> (String, Option<String>) {
	if let Some(entry) = state
		.voice
		.roster
		.iter()
		.find(|entry| entry.participant.user == user)
		.and_then(|entry| entry.member.as_ref())
	{
		let name = entry
			.nick
			.as_deref()
			.filter(|nick| !nick.is_empty())
			.unwrap_or(&entry.user.name);
		return (bounded_name(name), avatar_hash(&entry.user));
	}
	if let Some(channel) = state.voice.active.as_ref().map(|call| call.channel)
		&& let Some(person) = state
			.channels
			.iter()
			.find(|row| row.id == channel)
			.and_then(|row| row.recipients.iter().find(|person| person.id == user))
	{
		return (bounded_name(&person.name), avatar_hash(person));
	}
	if let Some(own) = state.user.as_ref().filter(|own| own.id == user) {
		return (bounded_name(&own.name), avatar_hash(own));
	}
	(String::new(), None)
}

fn avatar_hash(user: &model::User) -> Option<String> {
	user.avatar
		.as_deref()
		.filter(|hash| model::valid_avatar_hash(hash))
		.map(str::to_owned)
}

fn bounded_name(value: &str) -> String {
	value.chars().take(NAME_CHARS).collect()
}

/// A voice change a client asked for; the UI thread applies it. Volume is a percent, 0–200.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
	SelfVoice {
		mute: Option<bool>,
		deaf: Option<bool>,
	},
	User {
		id: u64,
		volume: Option<u16>,
		mute: Option<bool>,
	},
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Topic {
	Session = 1,
	Voice = 2,
}

impl Topic {
	const ALL: [Self; 2] = [Self::Session, Self::Voice];

	fn name(self) -> &'static str {
		match self {
			Self::Session => "session",
			Self::Voice => "voice",
		}
	}
}

/// Appends one frame: a little-endian `u32` byte length, then that much JSON.
fn push(out: &mut Vec<u8>, message: &Value) {
	let start = out.len();
	out.extend_from_slice(&[0; 4]);
	// Writing an owned JSON value into memory cannot fail.
	let _ = serde_json::to_writer(&mut *out, message);
	let length = (out.len() - start - 4) as u32;
	out[start..start + 4].copy_from_slice(&length.to_le_bytes());
}

fn event(topic: Topic, kind: &str, data: Value) -> Value {
	json!({ "op": "event", "topic": topic.name(), "type": kind, "data": data })
}

/// Frames that take a subscriber of `topics` from `old` to `new`. A client that fell behind
/// gets the difference to the newest state, so nothing queues up behind a slow reader.
fn diff(old: &Snapshot, new: &Snapshot, topics: u8, out: &mut Vec<u8>) {
	if topics & Topic::Session as u8 != 0 && (old.user != new.user || old.control != new.control) {
		push(out, &event(Topic::Session, "state", new.session()));
	}
	if topics & Topic::Voice as u8 == 0 {
		return;
	}
	let voice = |kind: &str, data: Value| event(Topic::Voice, kind, data);
	if (old.channel_id, old.guild_id) != (new.channel_id, new.guild_id) {
		push(out, &voice("state", new.voice()));
		return;
	}
	if (old.mute, old.deaf) != (new.mute, new.deaf) {
		push(
			out,
			&voice("self", json!({ "mute": new.mute, "deaf": new.deaf })),
		);
	}
	let find = |members: &[Member], id: u64| {
		members
			.binary_search_by_key(&id, |member| member.user_id)
			.ok()
	};
	for before in &old.members {
		if find(&new.members, before.user_id).is_none() {
			let user = before.user_id.to_string();
			push(out, &voice("member_leave", json!({ "user_id": user })));
		}
	}
	for member in &new.members {
		let Some(before) = find(&old.members, member.user_id).map(|at| &old.members[at]) else {
			push(out, &voice("member_join", json!(member)));
			continue;
		};
		if !before.same_settings(member) {
			push(out, &voice("member_update", json!(member)));
		} else if before.speaking != member.speaking {
			let user = member.user_id.to_string();
			push(
				out,
				&voice(
					"speaking",
					json!({ "user_id": user, "speaking": member.speaking }),
				),
			);
		}
	}
}

fn hex(bytes: &[u8]) -> String {
	use std::fmt::Write;
	bytes
		.iter()
		.fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
			let _ = write!(text, "{byte:02x}");
			text
		})
}

fn unhex(text: &str) -> Option<[u8; 32]> {
	let text = text.as_bytes();
	if text.len() != 64 {
		return None;
	}
	let digit = |byte: u8| (byte as char).to_digit(16).map(|value| value as u8);
	let mut out = [0; 32];
	for (slot, [high, low]) in out.iter_mut().zip(text.as_chunks::<2>().0) {
		*slot = digit(*high)? << 4 | digit(*low)?;
	}
	Some(out)
}

fn random_hex() -> String {
	let mut bytes = Zeroizing::new([0u8; 32]);
	getrandom::fill(&mut bytes[..]).expect("the OS provides randomness");
	hex(&bytes[..])
}

fn fresh_key() -> Key {
	Arc::new(Zeroizing::new(random_hex()))
}

/// HMAC-SHA256 over both nonces. `role` keeps a client's proof from being replayed as the
/// server's, and the fresh server nonce keeps an old proof from opening a new connection.
fn mac(key: &str, role: &str, server: &str, client: &str) -> Hmac<Sha256> {
	let mut mac =
		Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
	mac.update(format!("serein-local-api/{VERSION} {role} {server} {client}").as_bytes());
	mac
}

/// Length-prefixed request frames. A read cancelled part-way keeps its bytes here.
#[derive(Default)]
struct Input(Vec<u8>);

impl Input {
	fn take(&mut self) -> io::Result<Option<Vec<u8>>> {
		let Some(header) = self.0.first_chunk::<4>() else {
			return Ok(None);
		};
		let length = u32::from_le_bytes(*header) as usize;
		if length > MAX_REQUEST {
			return Err(io::ErrorKind::InvalidData.into());
		}
		if self.0.len() < 4 + length {
			return Ok(None);
		}
		let frame = self.0[4..4 + length].to_vec();
		self.0.drain(..4 + length);
		Ok(Some(frame))
	}

	/// The buffer never holds more than one unfinished frame plus one read.
	async fn next(&mut self, reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
		let mut chunk = [0u8; 1024];
		loop {
			if let Some(frame) = self.take()? {
				return Ok(frame);
			}
			let read = reader.read(&mut chunk).await?;
			if read == 0 {
				return Err(io::ErrorKind::UnexpectedEof.into());
			}
			self.0.extend_from_slice(&chunk[..read]);
		}
	}
}

/// What each connection shares with the UI thread.
#[derive(Clone)]
struct Shared {
	enabled: watch::Receiver<bool>,
	snapshots: watch::Receiver<Arc<Snapshot>>,
	commands: mpsc::Sender<Command>,
	wake: Wake,
}

async fn send(writer: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> bool {
	matches!(
		tokio::time::timeout(WRITE, writer.write_all(bytes)).await,
		Ok(Ok(()))
	)
}

/// Hello, then the client's proof, then ours. Returns the name the client gave itself, or
/// the error code it is refused with. Nothing about the session is sent before the proof.
async fn handshake(
	reader: &mut (impl AsyncRead + Unpin),
	writer: &mut (impl AsyncWrite + Unpin),
	input: &mut Input,
	key: &str,
) -> Result<String, &'static str> {
	let server = random_hex();
	let mut out = Vec::new();
	push(
		&mut out,
		&json!({ "op": "hello", "v": VERSION, "nonce": server }),
	);
	if !send(writer, &out).await {
		return Err("timeout");
	}
	let frame = input.next(reader).await.map_err(|_| "bad_frame")?;
	let auth: Value = serde_json::from_slice(&frame).map_err(|_| "bad_frame")?;
	let text = |field: &str| auth.get(field).and_then(Value::as_str).unwrap_or("");
	let client = text("nonce");
	if text("op") != "auth"
		|| client.len() != NONCE_CHARS
		|| !client.bytes().all(|byte| byte.is_ascii_hexdigit())
	{
		return Err("bad_request");
	}
	// `verify_slice` compares in constant time.
	let proven = unhex(text("proof")).is_some_and(|proof| {
		mac(key, "client", &server, client)
			.verify_slice(&proof)
			.is_ok()
	});
	if !proven {
		return Err("unauthorized");
	}
	let proof = hex(&mac(key, "server", &server, client).finalize().into_bytes());
	let topics = Topic::ALL.map(Topic::name);
	out.clear();
	push(
		&mut out,
		&json!({ "op": "ready", "v": VERSION, "proof": proof, "topics": topics }),
	);
	if !send(writer, &out).await {
		return Err("timeout");
	}
	Ok(text("name")
		.chars()
		.filter(|character| !character.is_control())
		.take(CLIENT_NAME_CHARS)
		.collect::<String>()
		.trim()
		.to_owned())
}

/// Tells the peer why, then waits briefly for it to hang up: closing a pipe at once can
/// discard the reason before the peer reads it.
async fn refuse(
	reader: &mut (impl AsyncRead + Unpin),
	writer: &mut (impl AsyncWrite + Unpin),
	code: &str,
) {
	let mut out = Vec::new();
	push(&mut out, &json!({ "op": "error", "code": code }));
	if send(writer, &out).await {
		let _ = tokio::time::timeout(Duration::from_secs(1), reader.read(&mut [0u8; 64])).await;
	}
}

/// One client from hello to hang-up. `seat` runs once the key is proven; what it returns is
/// held until the client leaves.
async fn serve<S, T>(stream: S, key: Key, mut shared: Shared, seat: impl FnOnce(String) -> T)
where
	S: AsyncRead + AsyncWrite + Unpin,
{
	let (mut reader, mut writer) = tokio::io::split(stream);
	let mut input = Input::default();
	let welcome = tokio::time::timeout(
		HANDSHAKE,
		handshake(&mut reader, &mut writer, &mut input, &key),
	)
	.await;
	let _seat = match welcome.unwrap_or(Err("timeout")) {
		Ok(name) => seat(name),
		Err(code) => {
			if code == "unauthorized" {
				diagnostics::record(Category::App, Level::Warn, "local api client refused");
			}
			return refuse(&mut reader, &mut writer, code).await;
		}
	};
	drop(key);
	// The state this client has been told about, and the topics it follows.
	let mut seen = Arc::new(Snapshot::default());
	let mut topics = 0u8;
	let mut out = Vec::new();
	loop {
		out.clear();
		let request = tokio::select! {
			frame = input.next(&mut reader) => match frame {
				Ok(frame) => Some(frame),
				Err(error) if error.kind() == io::ErrorKind::InvalidData => {
					return refuse(&mut reader, &mut writer, "bad_frame").await;
				}
				Err(_) => return,
			},
			changed = shared.snapshots.changed() => {
				// Switching the API off empties the view; that is a hang-up, not a sign-out.
				if changed.is_err() || !*shared.enabled.borrow() {
					return;
				}
				None
			}
		};
		// Events first, so an answer never describes a state older than one already sent.
		let latest = shared.snapshots.borrow_and_update().clone();
		diff(&seen, &latest, topics, &mut out);
		seen = latest;
		if let Some(frame) = request {
			let Ok(request) = serde_json::from_slice::<Value>(&frame) else {
				return refuse(&mut reader, &mut writer, "bad_frame").await;
			};
			respond(&request, &seen, &mut topics, &shared, &mut out);
		}
		if !out.is_empty() && !send(&mut writer, &out).await {
			return;
		}
	}
}

fn respond(request: &Value, state: &Snapshot, topics: &mut u8, shared: &Shared, out: &mut Vec<u8>) {
	let id = match request.get("id") {
		Some(id @ (Value::Number(_) | Value::String(_))) => id.clone(),
		_ => Value::Null,
	};
	let states = |mask: u8| -> Value {
		Topic::ALL
			.into_iter()
			.filter(|topic| mask & *topic as u8 != 0)
			.map(|topic| (topic.name().to_owned(), state.topic(topic)))
			.collect::<serde_json::Map<_, _>>()
			.into()
	};
	let result = match request.get("op").and_then(Value::as_str).unwrap_or("") {
		"subscribe" => requested_topics(request).map(|mask| {
			*topics |= mask;
			states(mask)
		}),
		"unsubscribe" => requested_topics(request).map(|mask| {
			*topics &= !mask;
			Value::Null
		}),
		"get" => requested_topics(request).map(states),
		"call" => call(request, state, shared).map(|()| Value::Null),
		_ => Err("unknown_op"),
	};
	push(
		out,
		&match result {
			Ok(data) => json!({ "op": "result", "id": id, "ok": true, "data": data }),
			Err(code) => json!({ "op": "result", "id": id, "ok": false, "error": code }),
		},
	);
}

fn requested_topics(request: &Value) -> Result<u8, &'static str> {
	let names = request
		.get("topics")
		.and_then(Value::as_array)
		.filter(|names| !names.is_empty())
		.ok_or("bad_request")?;
	names.iter().try_fold(0, |mask, name| {
		Topic::ALL
			.into_iter()
			.find(|topic| name.as_str() == Some(topic.name()))
			.map(|topic| mask | topic as u8)
			.ok_or("unknown_topic")
	})
}

/// Queues one voice change for the UI thread. The result only says it was accepted; the
/// change itself arrives as events.
fn call(request: &Value, state: &Snapshot, shared: &Shared) -> Result<(), &'static str> {
	let method = request.get("method").and_then(Value::as_str).unwrap_or("");
	if !matches!(method, "voice.set_self" | "voice.set_user") {
		return Err("unknown_method");
	}
	if !state.control {
		return Err("forbidden");
	}
	let Some(own) = &state.user else {
		return Err("unavailable");
	};
	let params = request.get("params").unwrap_or(&Value::Null);
	let flag = |name: &str| match params.get(name) {
		None | Some(Value::Null) => Ok(None),
		Some(Value::Bool(value)) => Ok(Some(*value)),
		Some(_) => Err("bad_request"),
	};
	let mute = flag("mute")?;
	let command = if method == "voice.set_self" {
		let deaf = flag("deaf")?;
		if mute.is_none() && deaf.is_none() {
			return Err("bad_request");
		}
		Command::SelfVoice { mute, deaf }
	} else {
		let id = match params.get("user_id") {
			Some(Value::String(text)) => text.parse().ok(),
			Some(Value::Number(number)) => number.as_u64(),
			_ => None,
		}
		.ok_or("bad_request")?;
		let volume = match params.get("volume") {
			None | Some(Value::Null) => None,
			Some(value) => Some(percent(value).ok_or("bad_request")?),
		};
		if volume.is_none() && mute.is_none() {
			return Err("bad_request");
		}
		// Only someone else in the current call: a client cannot fill the saved volume table
		// with people the owner never heard.
		if id == own.id || !state.members.iter().any(|member| member.user_id == id) {
			return Err("unknown_user");
		}
		Command::User { id, volume, mute }
	};
	match shared.commands.try_send(command) {
		Ok(()) => {
			(shared.wake)();
			Ok(())
		}
		Err(mpsc::error::TrySendError::Full(_)) => Err("busy"),
		Err(mpsc::error::TrySendError::Closed(_)) => Err("unavailable"),
	}
}

/// A dial may overshoot, so a finite number is clamped to the slider's range.
fn percent(value: &Value) -> Option<u16> {
	let number = value.as_f64().filter(|number| number.is_finite())?;
	Some(number.round().clamp(0.0, 200.0) as u16)
}

/// Names of the apps that proved the key, for the settings page.
#[derive(Default)]
struct Roster {
	names: Mutex<Vec<(u64, String)>>,
	changed: Notify,
}

impl Roster {
	fn names(&self) -> Vec<String> {
		let names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
		names.iter().map(|(_, name)| name.clone()).collect()
	}

	fn seat(self: Arc<Self>, id: u64, name: String) -> Seat {
		self.names
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.push((id, name));
		self.changed.notify_one();
		Seat(self, id)
	}
}

/// Removes its name when the client's task ends, however it ends.
struct Seat(Arc<Roster>, u64);

impl Drop for Seat {
	fn drop(&mut self) {
		self.0
			.names
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.retain(|(id, _)| *id != self.1);
		self.0.changed.notify_one();
	}
}

/// The saved key, or a new one. `false` means it could not be saved and lasts for this run.
async fn stored_key(store: bool) -> (Key, bool) {
	if !store {
		return (fresh_key(), false);
	}
	tokio::task::spawn_blocking(|| match platform::local_api_key::load() {
		Ok(Some(key)) => (Arc::new(key), true),
		Ok(None) => {
			let key = fresh_key();
			let saved = platform::local_api_key::save(&key).is_ok();
			(key, saved)
		}
		// A store that cannot be read is left as it is.
		Err(_) => (fresh_key(), false),
	})
	.await
	.unwrap_or_else(|_| (fresh_key(), false))
}

async fn replaced_key(store: bool) -> (Key, bool) {
	let key = fresh_key();
	if !store {
		return (key, false);
	}
	let saving = key.clone();
	let saved = tokio::task::spawn_blocking(move || {
		let saved = platform::local_api_key::save(&saving).is_ok();
		if !saved {
			// The key being replaced must not come back on the next launch.
			let _ = platform::local_api_key::forget();
		}
		saved
	})
	.await
	.unwrap_or(false);
	(key, saved)
}

/// The listener's side of the channels [`LocalApi`] holds.
struct Link {
	enabled: watch::Receiver<bool>,
	reset: mpsc::Receiver<()>,
	status: watch::Sender<LocalApiStatus>,
	key: watch::Sender<Option<Key>>,
	shared: Shared,
}

impl Link {
	fn report(&self, next: LocalApiStatus) {
		let changed = self.status.send_if_modified(|current| {
			let changed = *current != next;
			if changed {
				*current = next;
			}
			changed
		});
		if changed {
			(self.shared.wake)();
		}
	}
}

async fn supervise(endpoint: String, store: bool, mut link: Link) {
	// Off: nothing is bound and no key is held.
	while link.enabled.wait_for(|enabled| *enabled).await.is_ok() {
		let running = listen(&endpoint, store, &mut link).await;
		link.key.send_replace(None);
		link.report(LocalApiStatus::default());
		if !running {
			return;
		}
	}
}

async fn accept(listener: &mut Option<Listener>) -> io::Result<Stream> {
	match listener {
		Some(listener) => listener.accept().await,
		None => std::future::pending().await,
	}
}

/// Serves until the owner turns the API off. Returns false once the app itself is closing.
async fn listen(endpoint: &str, store: bool, link: &mut Link) -> bool {
	let address = private_endpoint(endpoint).unwrap_or_default();
	link.report(LocalApiStatus {
		state: LocalApiState::Starting,
		endpoint: address.clone(),
		..LocalApiStatus::default()
	});
	let (mut key, mut saved) = stored_key(store).await;
	link.key.send_replace(Some(key.clone()));
	let roster = Arc::new(Roster::default());
	let mut clients = JoinSet::new();
	let mut listener = None;
	let mut next_client = 0u64;
	let mut retry = tokio::time::interval(RETRY);
	retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	loop {
		// Binding fails while another program owns the name; it is never joined or replaced.
		if listener.is_none()
			&& let Ok(bound) = Listener::bind_private(endpoint)
		{
			diagnostics::record(Category::App, Level::Info, "local api listening");
			listener = Some(bound);
		}
		link.report(LocalApiStatus {
			state: if listener.is_some() {
				LocalApiState::Listening
			} else {
				LocalApiState::Unavailable
			},
			endpoint: address.clone(),
			clients: roster.names(),
			key: if saved {
				LocalApiKey::Saved
			} else {
				LocalApiKey::SessionOnly
			},
		});
		tokio::select! {
			changed = link.enabled.changed() => {
				if changed.is_err() {
					return false;
				}
				if !*link.enabled.borrow() {
					break;
				}
			}
			Some(()) = link.reset.recv() => {
				(key, saved) = replaced_key(store).await;
				link.key.send_replace(Some(key.clone()));
				// Every connected app proved the old key; each must return with the new one.
				clients.shutdown().await;
			}
			accepted = accept(&mut listener), if clients.len() < MAX_CLIENTS => match accepted {
				Ok(stream) => {
					next_client += 1;
					let (roster, id) = (roster.clone(), next_client);
					clients.spawn(serve(stream, key.clone(), link.shared.clone(), move |name| {
						roster.seat(id, name)
					}));
				}
				Err(_) => listener = None,
			},
			Some(_) = clients.join_next(), if !clients.is_empty() => {}
			() = roster.changed.notified() => {}
			_ = retry.tick(), if listener.is_none() => {}
		}
	}
	clients.shutdown().await;
	diagnostics::record(Category::App, Level::Info, "local api stopped");
	true
}

/// The UI thread's handle. Dropping it closes the endpoint and every connection.
pub struct LocalApi {
	enabled: watch::Sender<bool>,
	snapshot: watch::Sender<Arc<Snapshot>>,
	commands: mpsc::Receiver<Command>,
	reset: mpsc::Sender<()>,
	status: watch::Receiver<LocalApiStatus>,
	key: watch::Receiver<Option<Key>>,
	task: tokio::task::JoinHandle<()>,
}

impl LocalApi {
	/// Starts idle: nothing is bound until [`Self::publish`] is told the owner enabled it.
	/// `store` keeps the key in the OS credential store; without it the key lasts one run.
	pub fn start(
		handle: &tokio::runtime::Handle,
		store: bool,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Self {
		Self::spawn(handle, ENDPOINT.into(), store, wake)
	}

	fn spawn(
		handle: &tokio::runtime::Handle,
		endpoint: String,
		store: bool,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Self {
		let (enabled, enabled_rx) = watch::channel(false);
		let (snapshot, snapshots) = watch::channel(Arc::new(Snapshot::default()));
		let (commands_tx, commands) = mpsc::channel(32);
		let (reset, reset_rx) = mpsc::channel(1);
		let (status_tx, status) = watch::channel(LocalApiStatus::default());
		let (key_tx, key) = watch::channel(None);
		let link = Link {
			enabled: enabled_rx.clone(),
			reset: reset_rx,
			status: status_tx,
			key: key_tx,
			shared: Shared {
				enabled: enabled_rx,
				snapshots,
				commands: commands_tx,
				wake: Arc::new(wake),
			},
		};
		Self {
			enabled,
			snapshot,
			commands,
			reset,
			status,
			key,
			task: handle.spawn(supervise(endpoint, store, link)),
		}
	}

	/// Follows the owner's switch. While it is on, `capture` supplies the newest view and
	/// clients hear about it only if it changed; while it is off, nothing is captured.
	pub fn publish(&self, enabled: bool, capture: impl FnOnce() -> Snapshot) {
		let switched = self
			.enabled
			.send_if_modified(|current| std::mem::replace(current, enabled) != enabled);
		if !enabled {
			if switched {
				self.snapshot.send_replace(Arc::default());
			}
			return;
		}
		let next = capture();
		self.snapshot.send_if_modified(|current| {
			let changed = **current != next;
			if changed {
				*current = Arc::new(next);
			}
			changed
		});
	}

	/// The next voice change a client asked for.
	pub fn poll(&mut self) -> Option<Command> {
		self.commands.try_recv().ok()
	}

	pub fn status(&self) -> watch::Ref<'_, LocalApiStatus> {
		self.status.borrow()
	}

	/// The access key, for the owner's Copy button. `None` until the listener has loaded it.
	pub fn key(&self) -> Option<Zeroizing<String>> {
		self.key.borrow().as_deref().cloned()
	}

	/// Replaces the key and disconnects every app. A reset already queued covers this one.
	pub fn reset_key(&self) {
		let _ = self.reset.try_send(());
	}
}

impl Drop for LocalApi {
	fn drop(&mut self) {
		self.task.abort();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::voice::{Call, RosterEntry};
	use model::{AccountKind, ClientPlatforms, Member as RosterMember, User};
	use std::time::Instant;
	use tokio::time::timeout;

	const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
	const WAIT: Duration = Duration::from_secs(5);

	fn person(id: u64, name: &str) -> User {
		User {
			kind: AccountKind::Human,
			webhook: false,
			id: Id(id),
			name: name.into(),
			avatar: None,
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
		}
	}

	fn participant(user: u64, muted: bool) -> Participant {
		Participant {
			user: Id(user),
			muted,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			video: false,
			streaming: false,
		}
	}

	// `voice::State` keeps call bookkeeping private, so the fixture fills the public fields
	// after `Default` instead of building the struct in one literal.
	#[allow(clippy::field_reassign_with_default)]
	fn signed_in() -> (State, ui::MessagingUi) {
		let mut state = State::default();
		state.demo = true;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.user = Some(person(10, "Mina"));
		state.voice.active = Some(Call {
			channel: Id(50),
			guild: None,
			connected_at: Some(Instant::now()),
			channel_started_at: None,
			server_muted: false,
			server_deafened: false,
			request: 1,
			phase: Phase::Connected,
			muted: false,
			deafened: false,
			participants: vec![participant(10, false), participant(20, true)],
			camera: false,
			watching: None,
			error: None,
			reconnecting: None,
		});
		state.voice.roster.push(RosterEntry {
			guild: Id(7),
			channel: Id(50),
			participant: participant(20, true),
			member: Some(RosterMember {
				roles: Vec::new(),
				user: person(20, "ada"),
				nick: Some("Ada".into()),
				status: None,
				custom_status: None,
				activities: Vec::new(),
				clients: ClientPlatforms::default(),
			}),
		});
		(state, ui::MessagingUi::default())
	}

	fn member(user_id: u64, name: &str) -> Member {
		Member {
			user_id,
			name: name.into(),
			avatar: None,
			volume: 100,
			muted: false,
			self_mute: false,
			self_deaf: false,
			speaking: false,
		}
	}

	/// Mina (10) in channel 50 with Ada (20).
	fn room() -> Snapshot {
		Snapshot {
			user: Some(Identity {
				id: 10,
				name: "Mina".into(),
				avatar: None,
			}),
			channel_id: 50,
			members: vec![member(10, "Mina"), member(20, "Ada")],
			..Snapshot::default()
		}
	}

	async fn read(stream: &mut (impl AsyncRead + Unpin)) -> Value {
		let mut header = [0u8; 4];
		timeout(WAIT, stream.read_exact(&mut header))
			.await
			.expect("a frame arrives")
			.unwrap();
		let mut body = vec![0; u32::from_le_bytes(header) as usize];
		stream.read_exact(&mut body).await.unwrap();
		serde_json::from_slice(&body).unwrap()
	}

	async fn write(stream: &mut (impl AsyncWrite + Unpin), message: Value) {
		let mut out = Vec::new();
		push(&mut out, &message);
		stream.write_all(&out).await.unwrap();
	}

	/// The client's half of the handshake. Returns the server's answer after checking that a
	/// `ready` proves the key back.
	async fn authenticate(stream: &mut (impl AsyncRead + AsyncWrite + Unpin), key: &str) -> Value {
		let hello = read(stream).await;
		assert_eq!(hello["op"], "hello");
		let server = hello["nonce"].as_str().unwrap().to_owned();
		let client = random_hex();
		let proof = hex(&mac(key, "client", &server, &client).finalize().into_bytes());
		let auth = json!({ "op": "auth", "name": "Deck", "nonce": client, "proof": proof });
		write(stream, auth).await;
		let reply = read(stream).await;
		if reply["op"] == "ready" {
			let theirs = unhex(reply["proof"].as_str().unwrap()).unwrap();
			mac(key, "server", &server, &client)
				.verify_slice(&theirs)
				.expect("the server proves the key too");
		}
		reply
	}

	struct Rig {
		snapshots: watch::Sender<Arc<Snapshot>>,
		commands: mpsc::Receiver<Command>,
		shared: Shared,
	}

	impl Rig {
		fn new(snapshot: Snapshot) -> Self {
			let (snapshots, receiver) = watch::channel(Arc::new(snapshot));
			let (sender, commands) = mpsc::channel(4);
			Self {
				snapshots,
				commands,
				shared: Shared {
					enabled: watch::channel(true).1,
					snapshots: receiver,
					commands: sender,
					wake: Arc::new(|| {}),
				},
			}
		}

		/// A new connection to a server holding [`KEY`].
		fn connect(&self) -> tokio::io::DuplexStream {
			let (client, server) = tokio::io::duplex(64 * 1024);
			let key = Arc::new(Zeroizing::new(KEY.to_owned()));
			tokio::spawn(serve(server, key, self.shared.clone(), |_| ()));
			client
		}

		async fn signed_in(&self) -> tokio::io::DuplexStream {
			let mut client = self.connect();
			assert_eq!(authenticate(&mut client, KEY).await["op"], "ready");
			client
		}
	}

	#[test]
	fn proof_matches_the_documented_construction() {
		let (server, client) = ("11".repeat(32), "22".repeat(32));
		let proof = |role| hex(&mac(KEY, role, &server, &client).finalize().into_bytes());
		// Computed independently: HMAC-SHA256(key text, "serein-local-api/1 <role> <s> <c>").
		assert_eq!(
			proof("client"),
			"72021e203224508142d4cf926521ca6b9c904aae47ff176774cf3045d0fa769a"
		);
		assert_eq!(
			proof("server"),
			"2e51534838c33f5e4a9ed4878544f57fc5c78e7f5c56d2e2836dc083ab1172a6"
		);
	}

	#[tokio::test]
	async fn nothing_is_readable_without_proving_the_key() {
		let rig = Rig::new(room());

		let mut wrong = rig.connect();
		let refused = authenticate(&mut wrong, &"f".repeat(64)).await;
		assert_eq!(refused, json!({ "op": "error", "code": "unauthorized" }));

		// A request in place of the proof is refused, not answered.
		let mut eager = rig.connect();
		read(&mut eager).await;
		write(
			&mut eager,
			json!({ "op": "subscribe", "topics": ["voice"] }),
		)
		.await;
		assert_eq!(read(&mut eager).await["op"], "error");

		// A client proof replayed as the server's role does not open the door either.
		let mut replay = rig.connect();
		let server = read(&mut replay).await["nonce"]
			.as_str()
			.unwrap()
			.to_owned();
		let client = random_hex();
		let proof = hex(&mac(KEY, "server", &server, &client).finalize().into_bytes());
		write(
			&mut replay,
			json!({ "op": "auth", "nonce": client, "proof": proof }),
		)
		.await;
		assert_eq!(read(&mut replay).await["code"], "unauthorized");

		let mut client = rig.signed_in().await;
		write(
			&mut client,
			json!({ "op": "get", "id": 1, "topics": ["voice"] }),
		)
		.await;
		let answer = read(&mut client).await;
		assert_eq!(answer["id"], 1);
		assert_eq!(answer["data"]["voice"]["channel_id"], "50");
		assert!(answer["data"].get("session").is_none());
	}

	#[tokio::test]
	async fn a_subscriber_gets_changes_for_its_topics_only() {
		let rig = Rig::new(room());
		let mut client = rig.signed_in().await;
		write(
			&mut client,
			json!({ "op": "subscribe", "id": "a", "topics": ["voice"] }),
		)
		.await;
		let answer = read(&mut client).await;
		assert_eq!(answer["data"]["voice"]["members"][1]["user_id"], "20");

		// Session changed too, but this client follows voice: only the speaker is reported.
		let mut next = room();
		next.control = true;
		next.members[1].speaking = true;
		rig.snapshots.send_replace(Arc::new(next.clone()));
		let speaking = read(&mut client).await;
		assert_eq!(speaking["type"], "speaking");
		assert_eq!(
			speaking["data"],
			json!({ "user_id": "20", "speaking": true })
		);

		next.members[1].volume = 40;
		rig.snapshots.send_replace(Arc::new(next.clone()));
		let update = read(&mut client).await;
		assert_eq!(update["type"], "member_update");
		assert_eq!(update["data"]["volume"], 40);

		next.members.pop();
		rig.snapshots.send_replace(Arc::new(next.clone()));
		let leave = read(&mut client).await;
		assert_eq!(leave["type"], "member_leave");
		assert_eq!(leave["data"]["user_id"], "20");

		// Leaving the call replaces the room instead of listing each departure.
		next.channel_id = 0;
		next.members.clear();
		rig.snapshots.send_replace(Arc::new(next));
		let left = read(&mut client).await;
		assert_eq!(left["type"], "state");
		assert_eq!(left["data"]["connected"], false);
		assert_eq!(left["data"]["channel_id"], Value::Null);
	}

	#[tokio::test]
	async fn control_is_a_second_opt_in_and_reaches_only_call_members() {
		let mut rig = Rig::new(room());
		let mut client = rig.signed_in().await;
		let mut ask = async |method: &str, params: Value| {
			write(
				&mut client,
				json!({ "op": "call", "method": method, "params": params }),
			)
			.await;
			let answer = read(&mut client).await;
			answer["error"].as_str().unwrap_or("ok").to_owned()
		};

		assert_eq!(
			ask("voice.set_self", json!({ "mute": true })).await,
			"forbidden"
		);
		assert!(rig.commands.try_recv().is_err());

		let mut allowed = room();
		allowed.control = true;
		rig.snapshots.send_replace(Arc::new(allowed));
		assert_eq!(ask("voice.set_self", json!({ "mute": true })).await, "ok");
		assert_eq!(
			rig.commands.try_recv().unwrap(),
			Command::SelfVoice {
				mute: Some(true),
				deaf: None
			}
		);
		// A dial past the end of the slider is clamped rather than refused.
		let loud = json!({ "user_id": "20", "volume": 250.4 });
		assert_eq!(ask("voice.set_user", loud).await, "ok");
		assert_eq!(
			rig.commands.try_recv().unwrap(),
			Command::User {
				id: 20,
				volume: Some(200),
				mute: None
			}
		);
		for (params, error) in [
			(json!({ "user_id": "99", "mute": true }), "unknown_user"),
			(json!({ "user_id": "10", "mute": true }), "unknown_user"),
			(json!({ "user_id": "20", "mute": "yes" }), "bad_request"),
			(json!({ "user_id": "20" }), "bad_request"),
		] {
			assert_eq!(ask("voice.set_user", params).await, error);
		}
		assert_eq!(ask("voice.kick", Value::Null).await, "unknown_method");
		assert!(rig.commands.try_recv().is_err());
	}

	#[tokio::test]
	async fn an_oversized_or_malformed_frame_ends_the_connection() {
		let rig = Rig::new(room());
		for frame in [
			(MAX_REQUEST as u32 + 1).to_le_bytes().to_vec(),
			[&2u32.to_le_bytes()[..], &b"{["[..]].concat(),
		] {
			let mut client = rig.signed_in().await;
			client.write_all(&frame).await.unwrap();
			let refused = read(&mut client).await;
			assert_eq!(refused, json!({ "op": "error", "code": "bad_frame" }));
			// Hanging up ends the server's short wait; it then closes without another frame.
			client.shutdown().await.unwrap();
			let mut rest = Vec::new();
			timeout(WAIT, client.read_to_end(&mut rest))
				.await
				.expect("the server closes")
				.unwrap();
			assert!(rest.is_empty());
		}
	}

	#[test]
	fn capture_shows_the_call_and_nothing_once_it_fails_or_the_owner_signs_out() {
		let (mut state, mut ui) = signed_in();
		ui.apply_local_api_user(Id(20), Some(150), Some(true));
		ui.voice_speaking.push(Id(20));
		let snapshot = Snapshot::capture(&state, &ui);
		assert_eq!(snapshot.channel_id, 50);
		let [own, ada] = &snapshot.members[..] else {
			panic!("both people are in the room");
		};
		assert_eq!((own.user_id, own.volume, own.muted), (10, 100, false));
		// A local mute keeps the slider's percent, and the nickname wins over the username.
		assert_eq!(
			(ada.name.as_str(), ada.volume, ada.muted),
			("Ada", 150, true)
		);
		assert!(ada.self_mute && ada.speaking);

		state.voice.active.as_mut().unwrap().phase = Phase::Failed;
		let failed = Snapshot::capture(&state, &ui);
		assert_eq!(failed.channel_id, 0);
		assert!(failed.members.is_empty() && failed.user.is_some());

		state.user = None;
		assert_eq!(Snapshot::capture(&state, &ui), Snapshot::default());
	}

	#[test]
	fn deafened_reads_as_muted_and_unmuting_undeafens() {
		let (mut state, mut ui) = signed_in();
		state.voice.active.as_mut().unwrap().deafened = true;
		let snapshot = Snapshot::capture(&state, &ui);
		assert!(snapshot.mute && snapshot.deaf);
		assert!(snapshot.members[0].self_mute);

		ui.voice_deafened = true;
		assert_eq!(
			ui.apply_local_api_self_voice(Some(false), None),
			(false, false)
		);
		// Undeafening alone restores the microphone choice kept underneath.
		ui.voice_muted = true;
		ui.voice_deafened = true;
		assert_eq!(
			ui.apply_local_api_self_voice(None, Some(false)),
			(true, false)
		);
	}

	#[cfg(windows)]
	async fn open(endpoint: &str) -> io::Result<impl AsyncRead + AsyncWrite + Unpin> {
		tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)
	}

	#[cfg(unix)]
	async fn open(endpoint: &str) -> io::Result<impl AsyncRead + AsyncWrite + Unpin> {
		tokio::net::UnixStream::connect(endpoint).await
	}

	async fn closed(stream: &mut (impl AsyncRead + Unpin)) {
		let mut rest = Vec::new();
		// A torn-down pipe reports an error where a socket reports end of file.
		let _ = timeout(WAIT, stream.read_to_end(&mut rest))
			.await
			.expect("the connection is closed");
		assert!(rest.is_empty());
	}

	#[tokio::test]
	async fn the_endpoint_follows_the_switch_and_a_key_reset_disconnects_everyone() {
		let name = format!(
			"serein-api-test-{}-{}",
			std::process::id(),
			getrandom::u64().unwrap()
		);
		let endpoint = private_endpoint(&name).unwrap();
		let api = LocalApi::spawn(&tokio::runtime::Handle::current(), name, false, || {});
		let mut status = api.status.clone();
		let mut keys = api.key.clone();
		// Off by default: nothing is bound until the owner enables it.
		assert!(open(&endpoint).await.is_err());

		api.publish(true, room);
		timeout(
			WAIT,
			status.wait_for(|status| status.state == LocalApiState::Listening),
		)
		.await
		.expect("the endpoint binds")
		.unwrap();
		let first = api.key().unwrap();
		let mut client = open(&endpoint).await.unwrap();
		assert_eq!(authenticate(&mut client, &first).await["op"], "ready");
		timeout(WAIT, status.wait_for(|status| status.clients == ["Deck"]))
			.await
			.expect("the app is listed")
			.unwrap();

		api.reset_key();
		closed(&mut client).await;
		let second = timeout(
			WAIT,
			keys.wait_for(|key| key.as_ref().is_some_and(|key| **key != first)),
		)
		.await
		.expect("the key is replaced")
		.unwrap()
		.as_deref()
		.cloned()
		.unwrap();
		let mut stale = open(&endpoint).await.unwrap();
		assert_eq!(
			authenticate(&mut stale, &first).await["code"],
			"unauthorized"
		);
		let mut client = open(&endpoint).await.unwrap();
		assert_eq!(authenticate(&mut client, &second).await["op"], "ready");
		write(
			&mut client,
			json!({ "op": "subscribe", "topics": ["session"] }),
		)
		.await;
		assert_eq!(
			read(&mut client).await["data"]["session"]["signed_in"],
			true
		);

		// Switching off hangs up; a subscriber is not told the owner signed out first.
		api.publish(false, room);
		closed(&mut client).await;
		timeout(
			WAIT,
			status.wait_for(|status| *status == LocalApiStatus::default()),
		)
		.await
		.expect("the listener stops")
		.unwrap();
		assert!(api.key().is_none());
		timeout(WAIT, async {
			// The OS releases the name once the cancelled accept completes.
			while open(&endpoint).await.is_ok() {
				tokio::task::yield_now().await;
			}
		})
		.await
		.expect("the endpoint is gone");
	}
}
