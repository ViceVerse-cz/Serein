//! Loopback bridge for [JokelMixer](https://github.com/Comulative/JokelMixer).
//!
//! Speaks the same localhost HTTP API as the Legcord volume-mixer bridge, on
//! `127.0.0.1:6890`, so a Stream Deck can change per-person playback, mute and deafen.
//! Nothing here is sent to Discord except the mute and deafen the call already reports.
//! The event ring and the command queue stay in memory and are dropped on sign-out.

use client_core::State;
use client_core::voice::{Participant, Phase};
use diagnostics::{Category, Level};
use model::Id;
use serde_json::{Value, json};
use std::{collections::VecDeque, future::pending, io, net::Ipv4Addr, sync::Arc, time::Duration};
use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	net::{TcpListener, TcpStream},
	sync::{Semaphore, mpsc, oneshot, watch},
};

pub const PORT: u16 = 6890;
const MAX_EVENTS: usize = 250;
const MAX_BODY: usize = 16 * 1024;
const MAX_HEADERS: usize = 8 * 1024;
const MAX_CONNECTIONS: usize = 4;
const NAME_CHARS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Member {
	user_id: u64,
	name: String,
	avatar: Option<String>,
	discriminator: u16,
	volume: u16,
	muted: bool,
	self_mute: bool,
	self_deaf: bool,
}

/// What the bridge is allowed to show. `channel_id == 0` means not in a call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
	ready: bool,
	user_id: u64,
	name: String,
	avatar: Option<String>,
	discriminator: u16,
	mute: bool,
	deaf: bool,
	channel_id: u64,
	guild_id: u64,
	members: Vec<Member>,
	speaking: Vec<u64>,
}

impl Snapshot {
	/// Builds the bridge view from the signed-in session. A failed call and a signed-out
	/// window both report not ready / not in a channel, so the deck disconnects instead of
	/// keeping a stale room.
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
		let (mute, deaf) = call.map_or((ui.voice_muted, ui.voice_deafened), |call| {
			(call.muted, call.deafened)
		});
		let mut members = Vec::new();
		if let Some(call) = call {
			for participant in &call.participants {
				if participant.user.0 == 0 {
					continue;
				}
				if members.len() == 64 {
					break;
				}
				members.push(member_of(
					state,
					ui,
					own.id,
					call.muted,
					call.deafened,
					participant,
				));
			}
			members.sort_by_key(|member| member.user_id);
		}
		let mut speaking = Vec::new();
		if call.is_some() {
			speaking.extend(
				ui.voice_speaking
					.iter()
					.map(|id| id.0)
					.filter(|id| *id != 0),
			);
			speaking.sort_unstable();
			speaking.dedup();
			speaking.truncate(64);
		}
		Self {
			ready: true,
			user_id: own.id.0,
			name: bounded_name(&own.name),
			avatar: avatar_hash(own),
			discriminator: own.discriminator,
			mute,
			deaf,
			channel_id: call.map_or(0, |call| call.channel.0),
			guild_id: call.and_then(|call| call.guild).map_or(0, |id| id.0),
			members,
			speaking,
		}
	}
}

fn member_of(
	state: &State,
	ui: &ui::MessagingUi,
	own: Id,
	own_mute: bool,
	own_deaf: bool,
	participant: &Participant,
) -> Member {
	let (name, avatar, discriminator) = lookup(state, participant.user);
	let is_own = participant.user == own;
	Member {
		user_id: participant.user.0,
		name,
		avatar,
		discriminator,
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
	}
}

fn lookup(state: &State, user: Id) -> (String, Option<String>, u16) {
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
		return (
			bounded_name(name),
			avatar_hash(&entry.user),
			entry.user.discriminator,
		);
	}
	if let Some(channel) = state.voice.active.as_ref().map(|call| call.channel)
		&& let Some(person) = state
			.channels
			.iter()
			.find(|channel_row| channel_row.id == channel)
			.and_then(|channel_row| {
				channel_row
					.recipients
					.iter()
					.find(|person| person.id == user)
			}) {
		return (
			bounded_name(&person.name),
			avatar_hash(person),
			person.discriminator,
		);
	}
	if let Some(own) = state.user.as_ref().filter(|own| own.id == user) {
		return (bounded_name(&own.name), avatar_hash(own), own.discriminator);
	}
	("Participant".into(), None, 0)
}

fn avatar_hash(user: &model::User) -> Option<String> {
	user.avatar
		.as_deref()
		.filter(|hash| model::valid_avatar_hash(hash))
		.map(str::to_owned)
}

fn bounded_name(value: &str) -> String {
	let name: String = value.chars().take(NAME_CHARS).collect();
	if name.is_empty() {
		"Participant".into()
	} else {
		name
	}
}

/// A write the UI thread applies. Volumes are percents, 0–200.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixerCommand {
	SelfVoice {
		mute: Option<bool>,
		deaf: Option<bool>,
	},
	User {
		id: u64,
		volume: Option<u16>,
		muted: Option<bool>,
	},
}

enum Incoming {
	Http {
		request: HttpRequest,
		reply: oneshot::Sender<Vec<u8>>,
	},
}

struct HttpRequest {
	method: String,
	path: String,
	query: Vec<(String, String)>,
	body: Vec<u8>,
}

#[derive(Default)]
struct Service {
	current: Snapshot,
	previous: Snapshot,
	events: VecDeque<Queued>,
	last_event_id: i64,
}

struct Queued {
	id: i64,
	message: Value,
}

impl Service {
	fn reset(&mut self) {
		*self = Self::default();
	}

	fn apply_snapshot(&mut self, next: Snapshot) {
		if !next.ready {
			self.current.ready = false;
			return;
		}
		for message in diff(&self.previous, &next) {
			self.push(message);
		}
		self.previous = next.clone();
		self.current = next;
	}

	fn push(&mut self, message: Value) {
		self.last_event_id += 1;
		self.events.push_back(Queued {
			id: self.last_event_id,
			message,
		});
		while self.events.len() > MAX_EVENTS {
			self.events.pop_front();
		}
	}

	fn respond(
		&mut self,
		request: &HttpRequest,
		commands: &mpsc::Sender<MixerCommand>,
		wake: &dyn Fn(),
	) -> Vec<u8> {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/health") => json_response(200, "OK", &self.health()),
			("GET", "/events") => {
				let after = query_i64(&request.query, "after");
				let events: Vec<_> = self
					.events
					.iter()
					.filter(|event| event.id > after)
					.map(|event| json!({ "id": event.id, "message": event.message }))
					.collect();
				json_response(
					200,
					"OK",
					&json!({ "ok": true, "last_event_id": self.last_event_id, "events": events }),
				)
			}
			("POST", "/rpc") => match serde_json::from_slice::<Value>(&request.body) {
				Ok(packet) => json_response(200, "OK", &self.rpc(&packet, commands, wake)),
				Err(_) => json_response(
					400,
					"Bad Request",
					&json!({ "ok": false, "error": "invalid json" }),
				),
			},
			_ => json_response(
				404,
				"Not Found",
				&json!({ "ok": false, "error": "not found" }),
			),
		}
	}

	fn health(&self) -> Value {
		json!({
			"ok": true,
			"name": "serein-jokelmixer",
			"version": "1.0.0",
			"ready": self.current.ready && self.current.user_id != 0,
			"user_id": id_str(self.current.user_id),
			"last_event_id": self.last_event_id,
			"last_error": "",
		})
	}

	fn rpc(
		&mut self,
		packet: &Value,
		commands: &mpsc::Sender<MixerCommand>,
		wake: &dyn Fn(),
	) -> Value {
		let cmd = packet
			.get("cmd")
			.and_then(Value::as_str)
			.unwrap_or("")
			.to_ascii_uppercase();
		let args = packet.get("args").cloned().unwrap_or_else(|| json!({}));
		if !self.current.ready || self.current.user_id == 0 {
			return rpc_error(packet, "not signed in");
		}
		match cmd.as_str() {
			"AUTHORIZE" => rpc_ok(packet, json!({ "code": "serein-jokelmixer" })),
			"AUTHENTICATE" => rpc_ok(packet, json!({ "user": self.user_json() })),
			"SUBSCRIBE" | "UNSUBSCRIBE" => rpc_ok(packet, Value::Null),
			"GET_VOICE_SETTINGS" => rpc_ok(
				packet,
				json!({ "mute": self.current.mute, "deaf": self.current.deaf }),
			),
			"SET_VOICE_SETTINGS" => {
				let mute = args.get("mute").and_then(Value::as_bool);
				let deaf = args.get("deaf").and_then(Value::as_bool);
				if mute.is_none() && deaf.is_none() {
					return rpc_error(packet, "mute or deaf is required");
				}
				self.enqueue(MixerCommand::SelfVoice { mute, deaf }, commands, wake);
				rpc_ok(
					packet,
					json!({
						"mute": mute.unwrap_or(self.current.mute),
						"deaf": deaf.unwrap_or(self.current.deaf),
					}),
				)
			}
			"GET_SELECTED_VOICE_CHANNEL" => rpc_ok(packet, self.channel_json()),
			"SET_USER_VOICE_SETTINGS" => {
				let Some(id) = user_arg(&args) else {
					return rpc_error(packet, "user_id is invalid");
				};
				let volume = args.get("volume").and_then(percent);
				let muted = args.get("mute").and_then(Value::as_bool);
				if volume.is_none() && muted.is_none() {
					return rpc_error(packet, "volume or mute is required");
				}
				self.enqueue(MixerCommand::User { id, volume, muted }, commands, wake);
				let state = self
					.current
					.members
					.iter()
					.find(|member| member.user_id == id)
					.map(|member| {
						let mut copy = member.clone();
						if let Some(volume) = volume {
							copy.volume = volume;
						}
						if let Some(muted) = muted {
							copy.muted = muted;
						}
						member_json(&copy, self.current.channel_id, self.current.guild_id)
					});
				rpc_ok(packet, state.unwrap_or(Value::Null))
			}
			_ => rpc_error(packet, "unsupported command"),
		}
	}

	fn enqueue(
		&self,
		command: MixerCommand,
		commands: &mpsc::Sender<MixerCommand>,
		wake: &dyn Fn(),
	) {
		match commands.try_send(command) {
			Ok(()) => wake(),
			Err(mpsc::error::TrySendError::Full(_)) => {
				diagnostics::record(Category::Voice, Level::Warn, "jokelmixer command dropped");
			}
			Err(mpsc::error::TrySendError::Closed(_)) => {}
		}
	}

	fn user_json(&self) -> Value {
		json!({
			"id": id_str(self.current.user_id),
			"username": self.current.name,
			"global_name": Value::Null,
			"avatar": self.current.avatar,
			"discriminator": self.current.discriminator.to_string(),
		})
	}

	fn channel_json(&self) -> Value {
		json!({
			"id": id_str(self.current.channel_id),
			"guild_id": id_str(self.current.guild_id),
			"voice_states": self.current.members.iter().map(|member| {
				member_json(member, self.current.channel_id, self.current.guild_id)
			}).collect::<Vec<_>>(),
		})
	}
}

fn diff(previous: &Snapshot, next: &Snapshot) -> Vec<Value> {
	if !next.ready {
		return Vec::new();
	}
	let mut out = Vec::new();
	if !previous.ready || previous.channel_id != next.channel_id {
		out.push(dispatch(
			"VOICE_CHANNEL_SELECT",
			json!({ "channel_id": id_str(next.channel_id) }),
		));
	}
	for member in &next.members {
		match previous
			.members
			.iter()
			.find(|old| old.user_id == member.user_id)
		{
			None => out.push(dispatch(
				"VOICE_STATE_CREATE",
				member_json(member, next.channel_id, next.guild_id),
			)),
			Some(old) if member_changed(old, member) => out.push(dispatch(
				"VOICE_STATE_UPDATE",
				member_json(member, next.channel_id, next.guild_id),
			)),
			Some(_) => {}
		}
	}
	for old in &previous.members {
		if !next
			.members
			.iter()
			.any(|member| member.user_id == old.user_id)
		{
			out.push(dispatch(
				"VOICE_STATE_DELETE",
				member_json(old, previous.channel_id, previous.guild_id),
			));
		}
	}
	for id in &next.speaking {
		if !previous.speaking.contains(id) {
			out.push(dispatch(
				"SPEAKING_START",
				json!({ "user_id": id.to_string() }),
			));
		}
	}
	for id in &previous.speaking {
		if !next.speaking.contains(id) {
			out.push(dispatch(
				"SPEAKING_STOP",
				json!({ "user_id": id.to_string() }),
			));
		}
	}
	if !previous.ready || previous.mute != next.mute || previous.deaf != next.deaf {
		out.push(dispatch(
			"VOICE_SETTINGS_UPDATE",
			json!({ "mute": next.mute, "deaf": next.deaf }),
		));
	}
	out
}

fn member_changed(old: &Member, new: &Member) -> bool {
	old.name != new.name
		|| old.avatar != new.avatar
		|| old.volume != new.volume
		|| old.muted != new.muted
		|| old.self_mute != new.self_mute
		|| old.self_deaf != new.self_deaf
}

fn member_json(member: &Member, channel_id: u64, guild_id: u64) -> Value {
	json!({
		"nick": member.name,
		"user": {
			"id": member.user_id.to_string(),
			"username": member.name,
			"global_name": Value::Null,
			"avatar": member.avatar,
			"discriminator": member.discriminator.to_string(),
		},
		"volume": member.volume,
		"mute": member.muted,
		"channel_id": id_str(channel_id),
		"guild_id": id_str(guild_id),
		"voice_state": {
			"self_mute": member.self_mute,
			"self_deaf": member.self_deaf,
			"mute": false,
			"deaf": false,
		},
	})
}

fn dispatch(evt: &str, data: Value) -> Value {
	json!({ "cmd": "DISPATCH", "data": data, "evt": evt, "nonce": Value::Null })
}

fn id_str(id: u64) -> String {
	if id == 0 {
		String::new()
	} else {
		id.to_string()
	}
}

fn user_arg(args: &Value) -> Option<u64> {
	let raw = args.get("user_id")?;
	let text = raw
		.as_str()
		.map(str::to_owned)
		.or_else(|| raw.as_u64().map(|id| id.to_string()))?;
	text.parse::<Id>().ok().map(|id| id.0)
}

fn percent(value: &Value) -> Option<u16> {
	let number = value
		.as_f64()
		.or_else(|| value.as_i64().map(|value| value as f64))?;
	if !number.is_finite() {
		return None;
	}
	Some(number.round().clamp(0.0, 200.0) as u16)
}

fn query_i64(query: &[(String, String)], key: &str) -> i64 {
	query
		.iter()
		.find(|(name, _)| name == key)
		.and_then(|(_, value)| value.parse().ok())
		.unwrap_or(0)
		.max(0)
}

fn rpc_ok(packet: &Value, data: Value) -> Value {
	json!({
		"cmd": packet.get("cmd").and_then(Value::as_str).unwrap_or(""),
		"data": data,
		"evt": packet.get("evt").filter(|value| !value.is_null()).cloned().unwrap_or(Value::Null),
		"nonce": packet.get("nonce").cloned().unwrap_or(Value::Null),
	})
}

fn rpc_error(packet: &Value, message: &str) -> Value {
	json!({
		"cmd": packet.get("cmd").and_then(Value::as_str).unwrap_or(""),
		"data": { "message": message },
		"evt": "ERROR",
		"nonce": packet.get("nonce").cloned().unwrap_or(Value::Null),
	})
}

fn json_response(status: u16, reason: &str, body: &Value) -> Vec<u8> {
	let payload = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
	let head = format!(
		"HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
		payload.len()
	);
	let mut out = head.into_bytes();
	out.extend_from_slice(&payload);
	out
}

/// Host, Origin and the bridge header. A browser page sends Origin and cannot set
/// `X-DVM-Bridge` on a simple request; anything else is refused before it can change a call.
fn authorized(host: &str, headers: &[(String, String)], port: u16) -> bool {
	let host = host.trim().to_ascii_lowercase();
	let host_ok = host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}");
	host_ok
		&& !headers.iter().any(|(name, _)| name == "origin")
		&& headers
			.iter()
			.any(|(name, value)| name == "x-dvm-bridge" && value.trim() == "1")
}

fn parse_request(head: &str, body: Vec<u8>, port: u16) -> Result<HttpRequest, u16> {
	let mut lines = head.split("\r\n");
	let Some(start) = lines.next() else {
		return Err(400);
	};
	let mut parts = start.split_whitespace();
	let (Some(method), Some(target), Some(_)) = (parts.next(), parts.next(), parts.next()) else {
		return Err(400);
	};
	if method != "GET" && method != "POST" {
		return Err(404);
	}
	let (path, query_text) = target.split_once('?').unwrap_or((target, ""));
	if !path.starts_with('/') || path.contains("://") {
		return Err(400);
	}
	let mut headers = Vec::new();
	let mut host = None;
	let mut declared = None;
	for line in lines {
		if line.is_empty() {
			continue;
		}
		let Some((name, value)) = line.split_once(':') else {
			return Err(400);
		};
		let name = name.trim().to_ascii_lowercase();
		let value = value.trim();
		if name == "host" {
			host = Some(value.to_owned());
		}
		if name == "content-length" {
			let length: usize = value.parse().map_err(|_| 400u16)?;
			if length > MAX_BODY {
				return Err(413);
			}
			declared = Some(length);
		}
		if name == "transfer-encoding" {
			return Err(400);
		}
		headers.push((name, value.to_owned()));
	}
	let Some(host) = host else {
		return Err(403);
	};
	if !authorized(&host, &headers, port) {
		return Err(403);
	}
	if declared.unwrap_or(0) != body.len() {
		return Err(400);
	}
	let query = if query_text.is_empty() {
		Vec::new()
	} else {
		query_text
			.split('&')
			.filter_map(|pair| {
				let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
				if key.is_empty() {
					None
				} else {
					Some((key.to_owned(), value.to_owned()))
				}
			})
			.collect()
	};
	Ok(HttpRequest {
		method: method.to_owned(),
		path: path.to_owned(),
		query,
		body,
	})
}

async fn read_http(stream: &mut TcpStream, port: u16) -> Result<HttpRequest, u16> {
	let mut buf = Vec::with_capacity(512);
	let mut tmp = [0u8; 1024];
	let header_at = loop {
		if buf.len() > MAX_HEADERS {
			return Err(431);
		}
		let read = stream.read(&mut tmp).await.map_err(|_| 400u16)?;
		if read == 0 {
			return Err(400);
		}
		buf.extend_from_slice(&tmp[..read]);
		if let Some(at) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
			break at;
		}
	};
	let head = std::str::from_utf8(&buf[..header_at]).map_err(|_| 400u16)?;
	let mut body = buf[header_at + 4..].to_vec();
	let length = content_length(head).ok_or(400u16)?;
	if length > MAX_BODY {
		return Err(413);
	}
	while body.len() < length {
		let read = stream.read(&mut tmp).await.map_err(|_| 400u16)?;
		if read == 0 {
			return Err(400);
		}
		body.extend_from_slice(&tmp[..read]);
		if body.len() > MAX_BODY {
			return Err(413);
		}
	}
	body.truncate(length);
	parse_request(head, body, port)
}

fn content_length(head: &str) -> Option<usize> {
	let mut length = 0usize;
	let mut seen = false;
	for line in head.split("\r\n").skip(1) {
		let (name, value) = line.split_once(':')?;
		if name.trim().eq_ignore_ascii_case("content-length") {
			length = value.trim().parse().ok()?;
			seen = true;
		}
		if name.trim().eq_ignore_ascii_case("transfer-encoding") {
			return None;
		}
	}
	seen.then_some(length).or(Some(0))
}

async fn write_status(stream: &mut TcpStream, status: u16) -> io::Result<()> {
	let reason = match status {
		400 => "Bad Request",
		403 => "Forbidden",
		404 => "Not Found",
		413 => "Payload Too Large",
		431 => "Request Header Fields Too Large",
		503 => "Service Unavailable",
		_ => "Error",
	};
	let bytes = json_response(status, reason, &json!({ "ok": false, "error": reason }));
	stream.write_all(&bytes).await?;
	stream.shutdown().await
}

pub struct Bridge {
	snapshot: watch::Sender<Snapshot>,
	commands: mpsc::Receiver<MixerCommand>,
	task: tokio::task::JoinHandle<()>,
	#[cfg(test)]
	port: watch::Receiver<u16>,
	#[cfg(not(test))]
	_port: watch::Receiver<u16>,
}

impl Bridge {
	pub fn start(handle: &tokio::runtime::Handle, wake: impl Fn() + Send + Sync + 'static) -> Self {
		Self::spawn(handle, PORT, wake)
	}

	fn spawn(
		handle: &tokio::runtime::Handle,
		port: u16,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Self {
		let (snapshot, snapshot_rx) = watch::channel(Snapshot::default());
		let (commands_tx, commands) = mpsc::channel(32);
		let (port_tx, port_rx) = watch::channel(0u16);
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
		let task = handle.spawn(serve(port, snapshot_rx, commands_tx, wake, port_tx));
		Self {
			snapshot,
			commands,
			task,
			#[cfg(test)]
			port: port_rx,
			#[cfg(not(test))]
			_port: port_rx,
		}
	}

	pub fn publish(&self, next: Snapshot) {
		self.snapshot.send_if_modified(|current| {
			if *current == next {
				false
			} else {
				*current = next;
				true
			}
		});
	}

	pub fn poll(&mut self) -> Option<MixerCommand> {
		self.commands.try_recv().ok()
	}
}

impl Drop for Bridge {
	fn drop(&mut self) {
		self.task.abort();
	}
}

async fn serve(
	port: u16,
	mut snapshots: watch::Receiver<Snapshot>,
	commands: mpsc::Sender<MixerCommand>,
	wake: Arc<dyn Fn() + Send + Sync>,
	bound: watch::Sender<u16>,
) {
	let (jobs_tx, mut jobs_rx) = mpsc::channel::<Incoming>(8);
	let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
	let mut listener: Option<TcpListener> = None;
	// Port 0 asks the OS for a free port. Host checks use the port we actually bound.
	let mut advertised = port;
	let mut service = Service::default();
	let mut retry = tokio::time::interval(Duration::from_secs(2));
	retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
	retry.tick().await;
	loop {
		if !service.current.ready {
			if listener.take().is_some() {
				let _ = bound.send(0);
				diagnostics::record(Category::Voice, Level::Info, "jokelmixer bridge stopped");
			}
			service.reset();
		} else if listener.is_none()
			&& let Ok(socket) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await
		{
			if let Ok(addr) = socket.local_addr() {
				advertised = addr.port();
				let _ = bound.send(advertised);
			}
			diagnostics::record(Category::Voice, Level::Info, "jokelmixer bridge listening");
			listener = Some(socket);
		}

		tokio::select! {
			biased;
			changed = snapshots.changed() => {
				if changed.is_err() {
					return;
				}
				let next = snapshots.borrow().clone();
				service.apply_snapshot(next);
			}
			incoming = jobs_rx.recv() => {
				let Some(Incoming::Http { request, reply }) = incoming else {
					return;
				};
				let bytes = service.respond(&request, &commands, wake.as_ref());
				let _ = reply.send(bytes);
			}
			accepted = async {
				match listener.as_mut() {
					Some(socket) => socket.accept().await,
					None => pending().await,
				}
			} => {
				match accepted {
					Ok((stream, peer)) if peer.ip().is_loopback() => {
						let jobs = jobs_tx.clone();
						let slots = slots.clone();
						tokio::spawn(connection(stream, advertised, jobs, slots));
					}
					Ok(_) => {}
					Err(_) => {
						listener = None;
						let _ = bound.send(0);
					}
				}
			}
			_ = retry.tick(), if service.current.ready && listener.is_none() => {}
		}
	}
}

async fn connection(
	mut stream: TcpStream,
	port: u16,
	jobs: mpsc::Sender<Incoming>,
	slots: Arc<Semaphore>,
) {
	let Ok(permit) = slots.try_acquire_owned() else {
		let _ = write_status(&mut stream, 503).await;
		return;
	};
	let _permit = permit;
	let request =
		match tokio::time::timeout(Duration::from_secs(2), read_http(&mut stream, port)).await {
			Ok(Ok(request)) => request,
			Ok(Err(status)) => {
				let _ = write_status(&mut stream, status).await;
				return;
			}
			Err(_) => {
				let _ = write_status(&mut stream, 400).await;
				return;
			}
		};
	let (reply_tx, reply_rx) = oneshot::channel();
	if jobs
		.try_send(Incoming::Http {
			request,
			reply: reply_tx,
		})
		.is_err()
	{
		let _ = write_status(&mut stream, 503).await;
		return;
	}
	if let Ok(bytes) = reply_rx.await {
		let _ = stream.write_all(&bytes).await;
		let _ = stream.shutdown().await;
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::voice::{Call, Participant, Phase, RosterEntry};
	use model::{AccountKind, ClientPlatforms, Member as RosterMember, User};
	use std::time::Instant;

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

	fn call_with(user: u64) -> Call {
		Call {
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
			participants: vec![
				Participant {
					user: Id(10),
					muted: false,
					deafened: false,
					server_muted: false,
					server_deafened: false,
					video: false,
					streaming: false,
				},
				Participant {
					user: Id(user),
					muted: true,
					deafened: false,
					server_muted: false,
					server_deafened: false,
					video: false,
					streaming: false,
				},
			],
			camera: false,
			watching: None,
			error: None,
			reconnecting: None,
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
		state.voice.active = Some(call_with(20));
		state.voice.roster.push(RosterEntry {
			guild: Id(7),
			channel: Id(50),
			participant: Participant {
				user: Id(20),
				muted: true,
				deafened: false,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: false,
			},
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

	#[test]
	fn capture_keeps_the_slider_when_a_person_is_locally_muted() {
		let (mut state, mut ui) = signed_in();
		ui.apply_mixer_user(Id(20), Some(150), Some(true));
		ui.voice_speaking.push(Id(20));
		let snap = Snapshot::capture(&state, &ui);
		let ada = snap
			.members
			.iter()
			.find(|member| member.user_id == 20)
			.unwrap();
		assert_eq!(ada.name, "Ada");
		assert_eq!(ada.volume, 150);
		assert!(ada.muted);
		assert!(ada.self_mute);
		assert_eq!(snap.speaking, vec![20]);
		assert_eq!(snap.channel_id, 50);
		assert!(snap.members.iter().any(|member| member.user_id == 10));

		state.voice.active.as_mut().unwrap().phase = Phase::Failed;
		let failed = Snapshot::capture(&state, &ui);
		assert_eq!(failed.channel_id, 0);
		assert!(failed.members.is_empty());

		state.user = None;
		assert!(!Snapshot::capture(&state, &ui).ready);
	}

	#[test]
	fn bridge_header_rejects_browsers_and_other_hosts() {
		let ok = [("x-dvm-bridge".into(), "1".into())];
		assert!(authorized("127.0.0.1:6890", &ok, 6890));
		assert!(authorized("localhost:6890", &ok, 6890));
		assert!(!authorized("127.0.0.1:6890", &[], 6890));
		assert!(!authorized(
			"127.0.0.1:6890",
			&[
				("x-dvm-bridge".into(), "1".into()),
				("origin".into(), "https://evil.test".into())
			],
			6890,
		));
		assert!(!authorized("example.com:6890", &ok, 6890));
	}

	#[test]
	fn diff_reports_volume_speaking_and_leave_once() {
		let mut previous = Snapshot {
			ready: true,
			user_id: 10,
			channel_id: 50,
			members: vec![Member {
				user_id: 20,
				name: "Ada".into(),
				avatar: None,
				discriminator: 0,
				volume: 100,
				muted: false,
				self_mute: false,
				self_deaf: false,
			}],
			..Snapshot::default()
		};
		let mut next = previous.clone();
		next.members[0].volume = 40;
		next.speaking = vec![20];
		let events = diff(&previous, &next);
		assert_eq!(events.len(), 2);
		assert_eq!(events[0]["evt"], "VOICE_STATE_UPDATE");
		assert_eq!(events[0]["data"]["volume"], 40);
		assert_eq!(events[1]["evt"], "SPEAKING_START");

		previous = next.clone();
		next.members.clear();
		next.speaking.clear();
		next.channel_id = 0;
		let events = diff(&previous, &next);
		assert!(
			events
				.iter()
				.any(|event| event["evt"] == "VOICE_STATE_DELETE")
		);
		assert!(events.iter().any(|event| event["evt"] == "SPEAKING_STOP"));
		assert!(
			events
				.iter()
				.any(|event| event["evt"] == "VOICE_CHANNEL_SELECT")
		);
		assert!(
			!events
				.iter()
				.any(|event| event["evt"] == "VOICE_STATE_CREATE")
		);
	}

	#[test]
	fn event_ring_drops_the_oldest() {
		let mut service = Service::default();
		for index in 0..MAX_EVENTS + 3 {
			service.push(json!(index));
		}
		assert_eq!(service.events.len(), MAX_EVENTS);
		assert_eq!(service.events.front().unwrap().id, 4);
		assert_eq!(service.last_event_id, (MAX_EVENTS + 3) as i64);
	}

	#[test]
	fn volume_above_the_slider_is_clamped_and_a_bad_id_is_refused() {
		let (tx, mut rx) = mpsc::channel(4);
		let mut service = Service {
			current: Snapshot {
				ready: true,
				user_id: 10,
				channel_id: 50,
				members: vec![Member {
					user_id: 20,
					name: "Ada".into(),
					avatar: None,
					discriminator: 0,
					volume: 100,
					muted: false,
					self_mute: false,
					self_deaf: false,
				}],
				..Snapshot::default()
			},
			..Service::default()
		};
		let request = HttpRequest {
			method: "POST".into(),
			path: "/rpc".into(),
			query: Vec::new(),
			body: br#"{"cmd":"SET_USER_VOICE_SETTINGS","nonce":"jm","args":{"user_id":"20","volume":250.4,"mute":false}}"#.to_vec(),
		};
		let response = String::from_utf8(service.respond(&request, &tx, &|| {})).unwrap();
		assert!(response.contains("\"volume\":200"));
		assert!(!response.contains("\"evt\":\"ERROR\""));
		assert_eq!(
			rx.try_recv().unwrap(),
			MixerCommand::User {
				id: 20,
				volume: Some(200),
				muted: Some(false)
			}
		);
		let request = HttpRequest {
			method: "POST".into(),
			path: "/rpc".into(),
			query: Vec::new(),
			body: br#"{"cmd":"SET_USER_VOICE_SETTINGS","args":{"user_id":"nope","mute":true}}"#
				.to_vec(),
		};
		let response = String::from_utf8(service.respond(&request, &tx, &|| {})).unwrap();
		assert!(response.contains("\"evt\":\"ERROR\""));
		assert!(rx.try_recv().is_err());
	}

	#[tokio::test]
	async fn missing_bridge_header_is_forbidden_and_a_change_is_polled() {
		let bridge = Bridge::spawn(&tokio::runtime::Handle::current(), 0, || {});
		let mut port_rx = bridge.port.clone();
		let mut snap = Snapshot {
			ready: true,
			user_id: 10,
			name: "Mina".into(),
			channel_id: 50,
			members: vec![Member {
				user_id: 20,
				name: "Ada".into(),
				avatar: None,
				discriminator: 0,
				volume: 100,
				muted: false,
				self_mute: false,
				self_deaf: false,
			}],
			..Snapshot::default()
		};
		bridge.publish(snap.clone());
		let port = tokio::time::timeout(Duration::from_secs(2), async {
			loop {
				if *port_rx.borrow() != 0 {
					return *port_rx.borrow();
				}
				port_rx.changed().await.unwrap();
			}
		})
		.await
		.unwrap();

		let refused = exchange(
			port,
			"GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n",
		)
		.await;
		assert!(refused.starts_with("HTTP/1.1 403"));

		let health = exchange(
			port,
			&format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-DVM-Bridge: 1\r\n\r\n"),
		)
		.await;
		assert!(health.contains("\"ready\":true"));
		assert!(health.contains("\"user_id\":\"10\""));
		let last = health
			.split("\"last_event_id\":")
			.nth(1)
			.unwrap()
			.split([',', '}'])
			.next()
			.unwrap();

		snap.members[0].volume = 80;
		bridge.publish(snap);
		tokio::time::timeout(Duration::from_secs(2), async {
			loop {
				let body = exchange(
					port,
					&format!(
						"GET /events?after={last} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-DVM-Bridge: 1\r\n\r\n"
					),
				)
				.await;
				if body.contains("VOICE_STATE_UPDATE") {
					assert!(body.contains("\"volume\":80"));
					return;
				}
				tokio::task::yield_now().await;
			}
		})
		.await
		.expect("volume change is visible on /events");
	}

	async fn exchange(port: u16, raw: &str) -> String {
		let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
			.await
			.unwrap();
		stream
			.write_all(raw.replace("{port}", &port.to_string()).as_bytes())
			.await
			.unwrap();
		let mut buf = Vec::new();
		let _ = tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut buf)).await;
		String::from_utf8_lossy(&buf).into_owned()
	}
}
