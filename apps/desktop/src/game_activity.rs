//! Local Rich Presence: Discord-compatible IPC and WebSocket transports plus opt-in
//! detection of running games. Everything here is off until activity sharing is enabled.
use discord_api::{
	detectable::{Game, Index},
	external_assets::external_image_url,
	rpc::Metadata,
};
use discord_protocol::rpc::{self, Activity, Request};
use eframe::egui;
use futures_util::{SinkExt, StreamExt};
use model::{Id, User};
use std::{future::Future, io, net::Ipv4Addr, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
	io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
	net::{TcpListener, TcpStream},
	sync::{mpsc, watch},
	task::JoinSet,
	time::{Instant, timeout},
};
use tokio_tungstenite::{
	WebSocketStream,
	tungstenite::{
		Message,
		handshake::server::{ErrorResponse, Request as Handshake, Response},
		protocol::WebSocketConfig,
	},
};

pub type Detection = Result<Option<model::RichActivity>, &'static str>;
const MAX_CLIENTS: usize = 8;
/// Two images per activity, and a client normally repeats the same pair.
const MAX_EXTERNAL_CACHE: usize = 8;
/// Artwork uploaded while a game runs is picked up, without polling Discord per update.
const ASSET_REFRESH: Duration = Duration::from_secs(60);
const SCAN_INTERVAL: Duration = Duration::from_secs(10);
/// Origins Discord's own RPC WebSocket accepts. A native client sends no origin at all.
const WEB_ORIGINS: [&str; 6] = [
	"https://discord.com",
	"https://ptb.discord.com",
	"https://canary.discord.com",
	"https://discordapp.com",
	"https://ptb.discordapp.com",
	"https://canary.discordapp.com",
];
const INVALID: &str = "A game sent activity that could not be read.";

type Update = (usize, Option<Activity>);

/// The credential-free and account lookups a session may perform, so tests never reach Discord.
trait Applications: Clone + Send + Sync + 'static {
	fn metadata(&self, id: Id) -> impl Future<Output = Result<Metadata, &'static str>> + Send;
	/// Registered artwork only; used again when a key was uploaded after the game connected.
	fn assets(
		&self,
		id: Id,
	) -> impl Future<Output = Result<Vec<discord_api::rpc::Asset>, &'static str>> + Send;
	/// Ask Discord to proxy caller-supplied image URLs. Serein never fetches them itself.
	fn external(
		&self,
		id: Id,
		urls: &[String],
	) -> impl Future<Output = Result<Vec<String>, &'static str>> + Send;
	fn detectable(&self) -> impl Future<Output = Result<Vec<Game>, &'static str>> + Send;
}

/// One HTTP client and one shared rate-limit cooldown for a whole sharing session.
#[derive(Clone)]
struct Service {
	client: reqwest::Client,
	cooldown: Arc<tokio::sync::Mutex<Instant>>,
	api: Arc<discord_api::DiscordApi>,
}
impl Applications for Service {
	async fn metadata(&self, id: Id) -> Result<Metadata, &'static str> {
		discord_api::rpc::metadata(&self.client, &mut *self.cooldown.lock().await, id).await
	}
	async fn assets(&self, id: Id) -> Result<Vec<discord_api::rpc::Asset>, &'static str> {
		discord_api::rpc::assets(&self.client, &mut *self.cooldown.lock().await, id).await
	}
	async fn external(&self, id: Id, urls: &[String]) -> Result<Vec<String>, &'static str> {
		self.api
			.external_assets(id, urls)
			.await
			.map_err(|_| "Discord could not prepare the game's artwork.")
	}
	async fn detectable(&self) -> Result<Vec<Game>, &'static str> {
		let bytes =
			discord_api::detectable::download_list(&self.client, &mut *self.cooldown.lock().await)
				.await?;
		// Several megabytes of JSON must never be parsed on a runtime worker.
		tokio::task::spawn_blocking(move || discord_api::detectable::decode(&bytes))
			.await
			.map_err(|_| "The detectable game list is unavailable.")?
	}
}

/// Sharing owns the listeners and all clients. Dropping it cancels every pending operation.
pub async fn run(
	enabled: watch::Receiver<bool>,
	activity: watch::Sender<Option<Activity>>,
	report: watch::Sender<Detection>,
	invites: watch::Sender<Option<(u64, String)>>,
	ctx: egui::Context,
	user: User,
	api: Arc<discord_api::DiscordApi>,
) {
	run_enabled(enabled, &activity, &report, &ctx, || async {
		let service = Service {
			client: discord_api::rpc::client()?,
			cooldown: Arc::new(tokio::sync::Mutex::new(Instant::now())),
			api: api.clone(),
		};
		listen(&activity, &report, &invites, &ctx, &user, &service).await
	})
	.await;
}
async fn run_enabled<F, Fut>(
	mut enabled: watch::Receiver<bool>,
	activity: &watch::Sender<Option<Activity>>,
	report: &watch::Sender<Detection>,
	ctx: &egui::Context,
	start: F,
) where
	F: Fn() -> Fut,
	Fut: std::future::Future<Output = Result<(), &'static str>>,
{
	loop {
		activity.send_replace(None);
		let _ = report.send_replace(Ok(None));
		ctx.request_repaint();
		while !*enabled.borrow_and_update() {
			if enabled.changed().await.is_err() {
				return;
			}
		}
		tokio::select! {
			biased;
			_ = enabled.changed() => {},
			result = start() => {
				if let Err(error) = result {
					activity.send_replace(None);
					let _ = report.send_replace(Err(error));
					ctx.request_repaint();
				}
				// Do not repeatedly bind or retry failed metadata/service operations.
				if enabled.changed().await.is_err() { return; }
			}
		}
		if enabled.has_changed().is_err() {
			activity.send_replace(None);
			let _ = report.send_replace(Ok(None));
			return;
		}
	}
}

/// Aborts the scanner when the sharing session ends or the toggle is turned off.
struct Abort(tokio::task::JoinHandle<()>);
impl Drop for Abort {
	fn drop(&mut self) {
		self.0.abort();
	}
}

async fn listen<A: Applications>(
	activity: &watch::Sender<Option<Activity>>,
	report: &watch::Sender<Detection>,
	invites: &watch::Sender<Option<(u64, String)>>,
	ctx: &egui::Context,
	user: &User,
	service: &A,
) -> Result<(), &'static str> {
	let mut listener = platform::game_activity::Listener::bind().map_err(
		|_| "Game activity is unavailable. Close other Discord clients, then turn sharing off and on.",
	)?;
	// Browser and Electron clients speak the same RPC over localhost. Its absence is not fatal.
	let web = bind_web().await;
	let (send, mut receive) = mpsc::channel::<Update>(16);
	let (invite_send, mut invite_receive) = mpsc::channel::<String>(4);
	let (scan_send, mut scan_receive) = mpsc::channel::<Option<Activity>>(4);
	let mut workers = JoinSet::new();
	let mut slots = [false; MAX_CLIENTS];
	let mut values: [Option<(Instant, Activity)>; MAX_CLIENTS] = std::array::from_fn(|_| None);
	let mut scanned = None;
	let mut next_accept_ipc = Instant::now();
	let mut next_accept_web = Instant::now();
	let mut invite_count = 0;
	let _scanner = Abort(tokio::spawn(scan(service.clone(), scan_send)));
	loop {
		tokio::select! {
			// An admission interval also bounds credential-free metadata requests (two per client).
			accepted = async {
				tokio::time::sleep_until(next_accept_ipc).await;
				listener.accept().await
			}, if workers.len() < MAX_CLIENTS => {
				let stream = accepted.map_err(|_| "Game activity stopped. Turn sharing off and on to retry.")?;
				next_accept_ipc = Instant::now() + Duration::from_secs(5);
				let slot = claim(&mut slots);
				let (send, user, service, invites) = (send.clone(), user.clone(), service.clone(), invite_send.clone());
				workers.spawn(async move {
					(slot, serve_ipc(stream, &user, send, slot, invites, &service).await)
				});
			},
			accepted = async {
				match &web {
					Some(listener) => {
						tokio::time::sleep_until(next_accept_web).await;
						listener.accept().await.map(Some)
					}
					// Never resolves: an unavailable WebSocket port must not spin this loop.
					None => std::future::pending().await,
				}
			}, if workers.len() < MAX_CLIENTS => {
				next_accept_web = Instant::now() + Duration::from_secs(5);
				let Ok(Some((stream, peer))) = accepted else { continue };
				if !peer.ip().is_loopback() { continue; }
				let slot = claim(&mut slots);
				let (send, user, service, invites) = (send.clone(), user.clone(), service.clone(), invite_send.clone());
				workers.spawn(async move {
					(slot, serve_web(stream, &user, send, slot, invites, &service).await)
				});
			},
			Some((slot, value)) = receive.recv() => {
				values[slot] = value.map(|value| (Instant::now(), value));
				publish(&values, &scanned, activity, report, ctx);
			},
			Some(value) = scan_receive.recv() => {
				scanned = value;
				publish(&values, &scanned, activity, report, ctx);
			},
			Some(code) = invite_receive.recv() => {
				invite_count += 1;
				let _ = invites.send_replace(Some((invite_count, code)));
				ctx.request_repaint();
			},
			Some(result) = workers.join_next(), if !workers.is_empty() => {
				let (slot, result) = result.map_err(|_| "Game activity stopped. Turn sharing off and on to retry.")?;
				// Drain accepted updates before retiring the slot so an old update cannot revive it.
				while let Ok((index, value)) = receive.try_recv() {
					values[index] = value.map(|value| (Instant::now(), value));
				}
				slots[slot] = false;
				values[slot] = None;
				publish(&values, &scanned, activity, report, ctx);
				if let Err(error) = result && activity.borrow().is_none() {
					let _ = report.send_replace(Err(error)); ctx.request_repaint();
				}
			}
		}
	}
}

fn claim(slots: &mut [bool; MAX_CLIENTS]) -> usize {
	let slot = slots
		.iter()
		.position(|used| !used)
		.expect("bounded client slots");
	slots[slot] = true;
	slot
}

/// Discord's RPC WebSocket range. Every slot being taken only disables this transport.
async fn bind_web() -> Option<TcpListener> {
	for port in 6463..=6472 {
		if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
			return Some(listener);
		}
	}
	None
}

/// A connected client always outranks detection: it knows what the game is actually doing.
fn publish(
	values: &[Option<(Instant, Activity)>; MAX_CLIENTS],
	scanned: &Option<Activity>,
	activity: &watch::Sender<Option<Activity>>,
	report: &watch::Sender<Detection>,
	ctx: &egui::Context,
) {
	let latest = values
		.iter()
		.flatten()
		.max_by_key(|(at, _)| *at)
		.map(|(_, value)| value.clone())
		.or_else(|| scanned.clone());
	let display = latest.as_ref().map(display_activity);
	activity.send_if_modified(|current| {
		if *current == latest {
			return false;
		}
		*current = latest;
		true
	});
	if report.send_if_modified(|current| {
		if *current == Ok(display.clone()) {
			return false;
		}
		*current = Ok(display);
		true
	}) {
		ctx.request_repaint();
	}
}

fn display_activity(activity: &Activity) -> model::RichActivity {
	let text = |value: &Option<String>| {
		value
			.as_deref()
			.map(str::trim)
			.filter(|text| !text.is_empty())
			.map(str::to_owned)
	};
	let image = |key: &String| match key.strip_prefix("mp:") {
		Some(path) => Some(model::ActivityImage::Proxy(path.to_owned())),
		None => key.parse().ok().map(|asset| model::ActivityImage::Asset {
			application: activity.application_id,
			asset,
		}),
	};
	let assets = activity.assets.as_ref();
	let large = assets
		.and_then(|assets| assets.large_image.as_ref())
		.and_then(image);
	let small = assets
		.and_then(|assets| assets.small_image.as_ref())
		.and_then(image);
	// Discord shows the application icon in the large slot when no large image resolves;
	// the small image stays the corner badge and never becomes the artwork.
	let primary = large
		.clone()
		.unwrap_or(model::ActivityImage::Application(activity.application_id));
	let small_image = small
		.or_else(|| {
			(large.is_some() && assets.is_some_and(|assets| assets.small_image.is_none()))
				.then_some(model::ActivityImage::Application(activity.application_id))
		})
		.filter(|small| *small != primary);
	model::RichActivity {
		kind: activity.kind,
		name: activity.name.trim().to_owned(),
		details: text(&activity.details),
		state: text(&activity.state),
		image: Some(primary),
		small_image,
		started_at: activity
			.timestamps
			.as_ref()
			.and_then(|timestamps| timestamps.start),
	}
}

#[cfg(feature = "demo")]
pub fn demo_activity() -> model::RichActivity {
	model::RichActivity {
		kind: 0,
		name: "osu!".into(),
		details: Some("Playing a synthetic beatmap".into()),
		state: Some("Solo".into()),
		image: None,
		small_image: None,
		started_at: None,
	}
}

/// One connected client: its application, the metadata it needs and the artwork it reused.
struct Session<'a, A> {
	application: Id,
	service: &'a A,
	metadata: Option<Metadata>,
	refreshed: Option<Instant>,
	external: Vec<(String, String)>,
}
impl<'a, A: Applications> Session<'a, A> {
	fn new(application: Id, service: &'a A) -> Self {
		Self {
			application,
			service,
			metadata: None,
			refreshed: None,
			external: Vec::new(),
		}
	}
	async fn activity(&mut self, fields: rpc::ActivityFields) -> Result<Activity, &'static str> {
		if self.metadata.is_none() {
			self.metadata = Some(self.service.metadata(self.application).await?);
		}
		let name = self
			.metadata
			.as_ref()
			.expect("resolved metadata")
			.name
			.clone();
		let mut value = fields
			.into_activity(self.application, name)
			.map_err(|_| INVALID)?;
		if let Some(assets) = value.assets.take() {
			value.assets = Some(self.resolve(assets).await);
		}
		value.validate().map_err(|_| INVALID)?;
		Ok(value)
	}
	/// Registered keys, already-proxied `mp:` keys and caller-supplied URLs all end up as
	/// something Discord will render, or as nothing at all. A URL is never forwarded raw.
	async fn resolve(&mut self, mut assets: rpc::Assets) -> rpc::Assets {
		let mut pending: Vec<String> = [&assets.large_image, &assets.small_image]
			.into_iter()
			.flatten()
			.filter(|value| external_image_url(value).is_some() && self.remembered(value).is_none())
			.cloned()
			.collect();
		pending.dedup();
		if !pending.is_empty()
			&& let Ok(paths) = self.service.external(self.application, &pending).await
		{
			for (url, path) in pending.into_iter().zip(paths) {
				if self.external.len() >= MAX_EXTERNAL_CACHE {
					self.external.remove(0);
				}
				self.external.push((url, path));
			}
		}
		for image in [&mut assets.large_image, &mut assets.small_image] {
			*image = match image.take() {
				Some(key) => self.key(&key).await,
				None => None,
			};
		}
		if assets.large_image.is_none() {
			assets.large_text = None;
		}
		if assets.small_image.is_none() {
			assets.small_text = None;
		}
		assets
	}
	fn remembered(&self, url: &str) -> Option<String> {
		self.external
			.iter()
			.find(|(key, _)| key == url)
			.map(|(_, path)| path.clone())
	}
	async fn key(&mut self, value: &str) -> Option<String> {
		if let Some(path) = value.strip_prefix("mp:") {
			return model::ActivityImage::Proxy(path.to_owned())
				.valid()
				.then(|| value.to_owned());
		}
		if external_image_url(value).is_some() {
			return self.remembered(value);
		}
		if let Some(id) = self.metadata.as_ref().and_then(|data| data.asset(value)) {
			return Some(id);
		}
		// A launcher may upload artwork seconds after it connects; refetch, but not per update.
		if self
			.refreshed
			.is_none_or(|at| at.elapsed() >= ASSET_REFRESH)
		{
			self.refreshed = Some(Instant::now());
			if let Ok(assets) = self.service.assets(self.application).await
				&& let Some(metadata) = self.metadata.as_mut()
			{
				metadata.assets = assets;
			}
		}
		self.metadata.as_ref().and_then(|data| data.asset(value))
	}
}

/// A framed local transport reduced to what a session needs.
enum Incoming {
	Command(Vec<u8>),
	Close,
}
trait Channel {
	fn receive(&mut self) -> impl Future<Output = io::Result<Incoming>> + Send;
	fn reply(&mut self, bytes: &[u8]) -> impl Future<Output = io::Result<()>> + Send;
}

struct Ipc<S>(S);
impl<S: AsyncRead + AsyncWrite + Unpin + Send> Channel for Ipc<S> {
	async fn receive(&mut self) -> io::Result<Incoming> {
		loop {
			let (opcode, bytes) = read_frame(&mut self.0).await?;
			match opcode {
				1 => return Ok(Incoming::Command(bytes)),
				2 => return Ok(Incoming::Close),
				3 => write_frame(&mut self.0, 4, &bytes).await?,
				4 => {}
				_ => return Err(io::Error::from(io::ErrorKind::InvalidData)),
			}
		}
	}
	async fn reply(&mut self, bytes: &[u8]) -> io::Result<()> {
		write_frame(&mut self.0, 1, bytes).await
	}
}

struct Web(WebSocketStream<TcpStream>);
impl Channel for Web {
	async fn receive(&mut self) -> io::Result<Incoming> {
		loop {
			let Some(message) = self.0.next().await else {
				return Ok(Incoming::Close);
			};
			// Ping/Pong are answered by the protocol layer; only payloads reach a session.
			match message.map_err(io::Error::other)? {
				Message::Text(text) => return Ok(Incoming::Command(text.as_bytes().to_vec())),
				Message::Binary(bytes) => return Ok(Incoming::Command(bytes.to_vec())),
				Message::Close(_) => return Ok(Incoming::Close),
				_ => {}
			}
		}
	}
	async fn reply(&mut self, bytes: &[u8]) -> io::Result<()> {
		let text = String::from_utf8(bytes.to_vec()).map_err(io::Error::other)?;
		self.0
			.send(Message::Text(text.into()))
			.await
			.map_err(io::Error::other)
	}
}

async fn serve_ipc<S, A>(
	mut stream: S,
	user: &User,
	send: mpsc::Sender<Update>,
	slot: usize,
	invites: mpsc::Sender<String>,
	service: &A,
) -> Result<(), &'static str>
where
	S: AsyncRead + AsyncWrite + Unpin + Send,
	A: Applications,
{
	let (opcode, bytes) = timeout(Duration::from_secs(10), read_frame(&mut stream))
		.await
		.map_err(|_| INVALID)?
		.map_err(|_| INVALID)?;
	if opcode != 0 {
		return Err(INVALID);
	}
	let application = rpc::decode_handshake(&bytes).map_err(|_| INVALID)?;
	let mut channel = Ipc(stream);
	channel
		.reply(&rpc::ready(user.id, &user.name))
		.await
		.map_err(|_| INVALID)?;
	serve(channel, application, send, slot, invites, service).await
}

/// Browser clients hand the application id to the upgrade request instead of a handshake frame.
// tungstenite fixes the rejection type of an upgrade callback; it cannot be boxed.
#[allow(clippy::result_large_err)]
async fn serve_web<A: Applications>(
	stream: TcpStream,
	user: &User,
	send: mpsc::Sender<Update>,
	slot: usize,
	invites: mpsc::Sender<String>,
	service: &A,
) -> Result<(), &'static str> {
	let accepted = Arc::new(std::sync::Mutex::new(None));
	let captured = accepted.clone();
	let config = WebSocketConfig::default()
		.max_message_size(Some(rpc::MAX_FRAME_BYTES))
		.max_frame_size(Some(rpc::MAX_FRAME_BYTES));
	// Browsers, extensions and port scanners all reach these ports. A refused upgrade is
	// ordinary traffic, not a game failing, so it never becomes a reported error.
	let Ok(Ok(socket)) = timeout(
		Duration::from_secs(10),
		tokio_tungstenite::accept_hdr_async_with_config(
			stream,
			move |request: &Handshake, response: Response| match upgrade_application(request) {
				Some(id) => {
					if let Ok(mut slot) = captured.lock() {
						*slot = Some(id);
					}
					Ok(response)
				}
				None => Err(ErrorResponse::new(None)),
			},
			Some(config),
		),
	)
	.await
	else {
		return Ok(());
	};
	let Some(application) = accepted.lock().ok().and_then(|id| *id) else {
		return Ok(());
	};
	let mut channel = Web(socket);
	channel
		.reply(&rpc::ready(user.id, &user.name))
		.await
		.map_err(|_| INVALID)?;
	serve(channel, application, send, slot, invites, service).await
}

/// Only Discord's own web origins, or a native client that sends no origin, may connect.
fn upgrade_application(request: &Handshake) -> Option<Id> {
	let origin = request.headers().get("origin");
	if let Some(origin) = origin {
		let origin = origin.to_str().ok()?;
		WEB_ORIGINS.contains(&origin).then_some(())?;
	}
	let query = request.uri().query()?;
	let mut version = None;
	let mut client = None;
	for pair in query.split('&').take(16) {
		match pair.split_once('=') {
			Some(("v", value)) => version = Some(value),
			Some(("client_id", value)) => client = Some(value),
			_ => {}
		}
	}
	(version == Some("1")).then_some(())?;
	client?.parse().ok()
}

async fn serve<C: Channel, A: Applications>(
	mut channel: C,
	application: Id,
	send: mpsc::Sender<Update>,
	slot: usize,
	invites: mpsc::Sender<String>,
	service: &A,
) -> Result<(), &'static str> {
	let mut session = Session::new(application, service);
	loop {
		let bytes = match channel.receive().await {
			Ok(Incoming::Command(bytes)) => bytes,
			Ok(Incoming::Close) => return Ok(()),
			Err(error)
				if matches!(
					error.kind(),
					io::ErrorKind::UnexpectedEof
						| io::ErrorKind::BrokenPipe
						| io::ErrorKind::ConnectionReset
				) =>
			{
				return Ok(());
			}
			Err(_) => return Err(INVALID),
		};
		match rpc::decode_request(&bytes) {
			Ok(Request::SetActivity(command)) => {
				let ack = rpc::acknowledge(&command);
				let value = match command.activity {
					Some(fields) => Some(session.activity(fields).await?),
					None => None,
				};
				// A client that disconnected during a lookup must not publish stale activity.
				channel.reply(&ack).await.map_err(|_| INVALID)?;
				send.send((slot, value)).await.map_err(|_| INVALID)?;
			}
			Ok(Request::Invite { nonce, code }) => {
				// Handing an invite to the client only opens a dialog; joining stays confirmed.
				let _ = invites.try_send(code.clone());
				channel
					.reply(&rpc::invite_acknowledge(&nonce, &code))
					.await
					.map_err(|_| INVALID)?;
			}
			Err(_) => channel
				.reply(&rpc::error_for_payload(&bytes))
				.await
				.map_err(|_| INVALID)?,
		}
		// Backpressure on chatty local senders; no polling when clients are idle.
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
}

/// Detection for games that never speak RPC. Publishes only the public application identity.
async fn scan<A: Applications>(service: A, send: mpsc::Sender<Option<Activity>>) {
	let Some(games) = detectable(&service).await else {
		return;
	};
	let games = Index::new(&games);
	let mut current: Option<(Id, u64)> = None;
	loop {
		let paths = tokio::task::spawn_blocking(platform::processes::running).await;
		let found = paths
			.ok()
			.and_then(Result::ok)
			.and_then(|paths| choose(&games, &paths, current.map(|(id, _)| id)));
		let started = match (found.as_ref(), current) {
			(Some((id, _)), Some((previous, at))) if *id == previous => at,
			(Some(_), _) => std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap_or_default()
				.as_millis()
				.min(u64::MAX as u128) as u64,
			(None, _) => 0,
		};
		let next = found.as_ref().map(|(id, _)| (*id, started));
		if next != current {
			current = next;
			let value = found.and_then(|(id, name)| {
				rpc::ActivityFields {
					timestamps: Some(rpc::Timestamps {
						start: Some(started),
						end: None,
					}),
					..Default::default()
				}
				.into_activity(id, name)
				.ok()
			});
			if send.send(value).await.is_err() {
				return;
			}
		}
		tokio::time::sleep(SCAN_INTERVAL).await;
	}
}

/// Keeping the running match stable avoids flapping between two matching processes.
fn choose(games: &Index, paths: &[String], previous: Option<Id>) -> Option<(Id, String)> {
	let mut first = None;
	for path in paths {
		let Some(found) = games.find(path) else {
			continue;
		};
		if Some(found.0) == previous {
			return Some(found);
		}
		first.get_or_insert(found);
	}
	first
}

/// The published list changes slowly and is large, so a refresh is a daily cost at most.
const LIST_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_CACHED_LIST: u64 = 8 * 1024 * 1024;

fn list_path() -> Option<PathBuf> {
	dirs::data_local_dir().map(|root| root.join("serein").join("detectable.json"))
}

async fn detectable<A: Applications>(service: &A) -> Option<Vec<Game>> {
	if let Some(games) = tokio::task::spawn_blocking(|| read_list(list_path()?))
		.await
		.ok()
		.flatten()
	{
		return Some(games);
	}
	let games = service.detectable().await.ok()?;
	let stored = games.clone();
	let _ = tokio::task::spawn_blocking(move || write_list(list_path(), &stored)).await;
	Some(games)
}

fn read_list(path: PathBuf) -> Option<Vec<Game>> {
	let metadata = std::fs::metadata(&path).ok()?;
	(metadata.len() <= MAX_CACHED_LIST).then_some(())?;
	(metadata.modified().ok()?.elapsed().ok()? < LIST_TTL).then_some(())?;
	discord_api::detectable::decode_cached(&std::fs::read(&path).ok()?).ok()
}

fn write_list(path: Option<PathBuf>, games: &[Game]) -> Option<()> {
	let path = path?;
	std::fs::create_dir_all(path.parent()?).ok()?;
	let bytes = serde_json::to_vec(games).ok()?;
	(bytes.len() as u64 <= MAX_CACHED_LIST).then_some(())?;
	std::fs::write(&path, bytes).ok()
}

async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<(u32, Vec<u8>)> {
	let mut header = [0; 8];
	// Idle clients may publish only once. Once a frame begins it must finish promptly.
	stream.read_exact(&mut header[..1]).await?;
	timeout(Duration::from_secs(5), async {
		stream.read_exact(&mut header[1..]).await?;
		let opcode = u32::from_le_bytes(header[..4].try_into().expect("opcode"));
		let size = u32::from_le_bytes(header[4..].try_into().expect("size")) as usize;
		if size > rpc::MAX_FRAME_BYTES {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"IPC frame limit",
			));
		}
		let mut bytes = vec![0; size];
		stream.read_exact(&mut bytes).await?;
		Ok((opcode, bytes))
	})
	.await
	.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}
async fn write_frame(
	stream: &mut (impl AsyncWrite + Unpin),
	opcode: u32,
	bytes: &[u8],
) -> io::Result<()> {
	if bytes.len() > rpc::MAX_FRAME_BYTES {
		return Err(io::Error::from(io::ErrorKind::InvalidData));
	}
	// Some game SDKs parse each pipe read as a complete frame; a separate
	// opcode/header write makes them reject READY and disconnect immediately.
	let mut frame = Vec::with_capacity(8 + bytes.len());
	frame.extend_from_slice(&opcode.to_le_bytes());
	frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
	frame.extend_from_slice(bytes);
	timeout(Duration::from_secs(5), async {
		stream.write_all(&frame).await?;
		stream.flush().await
	})
	.await
	.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

#[cfg(test)]
mod tests {
	use super::*;
	use discord_api::rpc::Asset;
	use std::sync::{
		Mutex,
		atomic::{AtomicUsize, Ordering},
	};
	use tokio::sync::Notify;

	fn user() -> User {
		User {
			primary_guild: None,
			id: model::Id(1),
			name: "Synthetic user".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		}
	}

	/// Offline stand-in for Discord's public application lookups and proxy endpoint.
	#[derive(Clone)]
	struct Offline {
		assets: Arc<Mutex<Vec<Asset>>>,
		proxied: Arc<Mutex<Vec<(String, String)>>>,
		asset_calls: Arc<AtomicUsize>,
		proxy_calls: Arc<AtomicUsize>,
		started: Arc<Notify>,
		release: Option<Arc<Notify>>,
		games: Arc<Vec<Game>>,
	}
	impl Default for Offline {
		fn default() -> Self {
			Self {
				assets: Arc::new(Mutex::new(vec![Asset {
					id: model::Id(9),
					name: "map".into(),
				}])),
				proxied: Arc::new(Mutex::new(Vec::new())),
				asset_calls: Arc::new(AtomicUsize::new(0)),
				proxy_calls: Arc::new(AtomicUsize::new(0)),
				started: Arc::new(Notify::new()),
				release: None,
				games: Arc::new(Vec::new()),
			}
		}
	}
	impl Applications for Offline {
		async fn metadata(&self, _: Id) -> Result<Metadata, &'static str> {
			self.started.notify_one();
			if let Some(release) = &self.release {
				release.notified().await;
			}
			Ok(Metadata {
				name: "A game outside the old list".into(),
				assets: self.assets.lock().unwrap().clone(),
			})
		}
		async fn assets(&self, _: Id) -> Result<Vec<Asset>, &'static str> {
			self.asset_calls.fetch_add(1, Ordering::SeqCst);
			Ok(self.assets.lock().unwrap().clone())
		}
		async fn external(&self, _: Id, urls: &[String]) -> Result<Vec<String>, &'static str> {
			self.proxy_calls.fetch_add(1, Ordering::SeqCst);
			let proxied = self.proxied.lock().unwrap();
			urls.iter()
				.map(|url| {
					proxied
						.iter()
						.find(|(key, _)| key == url)
						.map(|(_, path)| path.clone())
						.ok_or("no artwork")
				})
				.collect()
		}
		async fn detectable(&self) -> Result<Vec<Game>, &'static str> {
			Ok(self.games.as_ref().clone())
		}
	}

	async fn handshake(game: &mut tokio::io::DuplexStream) {
		write_frame(game, 0, br#"{"v":1,"client_id":"7"}"#)
			.await
			.unwrap();
		let ready = read_frame(game).await.unwrap();
		assert_eq!(ready.0, 1);
		assert!(String::from_utf8(ready.1).unwrap().contains("READY"));
	}

	#[tokio::test]
	async fn ipc_handshake_rich_update_clear_ping_and_disconnect() {
		let (mut game, server) = tokio::io::duplex(32 * 1024);
		let (send, mut receive) = mpsc::channel(16);
		let (invites, _held) = mpsc::channel(4);
		let service = Offline::default();
		let worker =
			tokio::spawn(
				async move { serve_ipc(server, &user(), send, 0, invites, &service).await },
			);
		handshake(&mut game).await;
		write_frame(
			&mut game,
			1,
			br#"{"cmd":"SUBSCRIBE","evt":"ACTIVITY_JOIN","nonce":"subscription"}"#,
		)
		.await
		.unwrap();
		let error = read_frame(&mut game).await.unwrap();
		assert!(String::from_utf8(error.1).unwrap().contains("subscription"));
		write_frame(&mut game, 3, b"ping").await.unwrap();
		assert_eq!(read_frame(&mut game).await.unwrap(), (4, b"ping".to_vec()));
		write_frame(&mut game, 1, br#"{"cmd":"SET_ACTIVITY","nonce":"one","args":{"pid":123,"activity":{"details":"Level 4","state":"In match","timestamps":{"start":1700000000},"assets":{"large_image":"map","small_image":"https://localhost/private"},"secrets":{"join":"synthetic-not-forwarded"}}}}"#).await.unwrap();
		let ack = read_frame(&mut game).await.unwrap();
		assert!(String::from_utf8(ack.1).unwrap().contains("one"));
		let (_, value) = receive.recv().await.unwrap();
		let value = value.unwrap();
		assert_eq!(value.application_id, model::Id(7));
		assert_eq!(value.name, "A game outside the old list");
		assert_eq!(value.details.as_deref(), Some("Level 4"));
		assert_eq!(value.state.as_deref(), Some("In match"));
		assert_eq!(value.timestamps.unwrap().start, Some(1700000000000));
		let assets = value.assets.unwrap();
		assert_eq!(assets.large_image.as_deref(), Some("9"));
		assert!(assets.small_image.is_none());
		for clear in [
			br#"{"cmd":"SET_ACTIVITY","nonce":"clear","args":{"pid":123,"activity":null}}"#
				.as_slice(),
			br#"{"cmd":"SET_ACTIVITY","nonce":"legacy-clear","args":{"pid":123}}"#,
		] {
			write_frame(&mut game, 1, clear).await.unwrap();
			read_frame(&mut game).await.unwrap();
			assert_eq!(receive.recv().await.unwrap(), (0, None));
		}
		drop(game);
		timeout(Duration::from_secs(2), worker)
			.await
			.unwrap()
			.unwrap()
			.unwrap();
	}

	#[tokio::test]
	async fn artwork_urls_are_proxied_once_and_late_uploads_still_resolve() {
		let service = Offline::default();
		service.assets.lock().unwrap().clear();
		service.proxied.lock().unwrap().push((
			"https://example.com/cover.png".into(),
			"mp:external/synthetic-hash-01/https/example.com/cover.png".into(),
		));
		let mut session = Session::new(model::Id(7), &service);
		let sent = rpc::Assets {
			large_image: Some("https://example.com/cover.png".into()),
			large_text: Some("Cover".into()),
			small_image: Some("map".into()),
			small_text: Some("Rank".into()),
		};
		let activity = session
			.activity(rpc::ActivityFields {
				assets: Some(sent.clone()),
				..Default::default()
			})
			.await
			.unwrap();
		let assets = activity.assets.unwrap();
		// The URL became a media-proxy key; the not-yet-uploaded registered key dropped out.
		assert_eq!(
			assets.large_image.as_deref(),
			Some("mp:external/synthetic-hash-01/https/example.com/cover.png")
		);
		assert_eq!(assets.large_text.as_deref(), Some("Cover"));
		assert!(assets.small_image.is_none() && assets.small_text.is_none());
		assert_eq!(service.proxy_calls.load(Ordering::SeqCst), 1);
		assert_eq!(service.asset_calls.load(Ordering::SeqCst), 1);
		// Repeating the same activity reuses both lookups instead of asking Discord again.
		session
			.activity(rpc::ActivityFields {
				assets: Some(sent.clone()),
				..Default::default()
			})
			.await
			.unwrap();
		assert_eq!(service.proxy_calls.load(Ordering::SeqCst), 1);
		assert_eq!(service.asset_calls.load(Ordering::SeqCst), 1);
		// Artwork uploaded after the game connected is picked up on the next refresh.
		service.assets.lock().unwrap().push(Asset {
			id: model::Id(9),
			name: "map".into(),
		});
		session.refreshed = None;
		let assets = session
			.activity(rpc::ActivityFields {
				assets: Some(sent),
				..Default::default()
			})
			.await
			.unwrap()
			.assets
			.unwrap();
		assert_eq!(assets.small_image.as_deref(), Some("9"));
		assert_eq!(
			assets.large_image.as_deref(),
			Some("mp:external/synthetic-hash-01/https/example.com/cover.png")
		);
		assert_eq!(service.proxy_calls.load(Ordering::SeqCst), 1);
		assert_eq!(service.asset_calls.load(Ordering::SeqCst), 2);
		// An already-proxied key passes through; traversal and unknown URLs never do.
		for (key, expected) in [
			(
				"mp:external/hash-02/https/example.com/a.png",
				Some("mp:external/hash-02/https/example.com/a.png"),
			),
			("mp:external/../secret", None),
			("https://example.com/unknown.png", None),
			("http://example.com/insecure.png", None),
		] {
			let assets = session
				.activity(rpc::ActivityFields {
					assets: Some(rpc::Assets {
						large_image: Some(key.into()),
						..Default::default()
					}),
					..Default::default()
				})
				.await
				.unwrap()
				.assets
				.unwrap();
			assert_eq!(assets.large_image.as_deref(), expected, "for {key}");
		}
	}

	#[tokio::test]
	async fn websocket_requires_a_discord_origin_and_publishes_activity() {
		let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
		let port = listener.local_addr().unwrap().port();
		let (send, mut receive) = mpsc::channel(16);
		let (invites, mut asked) = mpsc::channel(4);
		let service = Offline::default();
		let server = tokio::spawn(async move {
			let mut results = Vec::new();
			for slot in 0..3 {
				let (stream, _) = listener.accept().await.unwrap();
				results.push(
					serve_web(
						stream,
						&user(),
						send.clone(),
						slot,
						invites.clone(),
						&service,
					)
					.await,
				);
			}
			results
		});
		let connect = |origin: Option<&'static str>, query: &str| {
			let url = format!("ws://127.0.0.1:{port}/{query}");
			async move {
				let mut request =
					tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
						url.as_str(),
					)
					.unwrap();
				if let Some(origin) = origin {
					request
						.headers_mut()
						.insert("origin", origin.parse().unwrap());
				}
				tokio_tungstenite::connect_async(request).await
			}
		};
		assert!(
			connect(Some("https://evil.example"), "?v=1&client_id=7")
				.await
				.is_err()
		);
		assert!(connect(None, "?v=2&client_id=7").await.is_err());
		let (mut socket, _) = connect(Some("https://discord.com"), "?v=1&client_id=7")
			.await
			.unwrap();
		let ready = socket.next().await.unwrap().unwrap();
		assert!(ready.to_text().unwrap().contains("READY"));
		socket.send(Message::Text(r#"{"cmd":"SET_ACTIVITY","nonce":"one","args":{"pid":123,"activity":{"details":"From a browser"}}}"#.into())).await.unwrap();
		let ack = socket.next().await.unwrap().unwrap();
		assert!(ack.to_text().unwrap().contains("one"));
		let value = receive.recv().await.unwrap().1.unwrap();
		assert_eq!(value.application_id, model::Id(7));
		assert_eq!(value.details.as_deref(), Some("From a browser"));
		socket
			.send(Message::Text(
				r#"{"cmd":"INVITE_BROWSER","nonce":"two","args":{"code":"hTKzmak"}}"#.into(),
			))
			.await
			.unwrap();
		let ack = socket.next().await.unwrap().unwrap();
		assert!(ack.to_text().unwrap().contains("hTKzmak"));
		assert_eq!(asked.recv().await.unwrap(), "hTKzmak");
		socket.close(None).await.unwrap();
		let results = timeout(Duration::from_secs(5), server)
			.await
			.unwrap()
			.unwrap();
		// A refused upgrade is silence, not a reported game failure.
		assert!(results.iter().all(Result::is_ok));
	}

	#[tokio::test]
	async fn disconnect_during_metadata_lookup_never_queues_activity() {
		let (mut game, server) = tokio::io::duplex(4096);
		let (send, mut receive) = mpsc::channel(16);
		let (invites, _held) = mpsc::channel(4);
		let release = Arc::new(Notify::new());
		let service = Offline {
			release: Some(release.clone()),
			..Offline::default()
		};
		let started = service.started.clone();
		let worker =
			tokio::spawn(
				async move { serve_ipc(server, &user(), send, 0, invites, &service).await },
			);
		write_frame(&mut game, 0, br#"{"v":1,"client_id":"7"}"#)
			.await
			.unwrap();
		read_frame(&mut game).await.unwrap();
		write_frame(
			&mut game,
			1,
			br#"{"cmd":"SET_ACTIVITY","nonce":"one","args":{"pid":123,"activity":{}}}"#,
		)
		.await
		.unwrap();
		started.notified().await;
		drop(game);
		release.notify_one();
		assert!(worker.await.unwrap().is_err());
		assert!(receive.recv().await.is_none());
	}

	#[tokio::test]
	#[cfg(windows)]
	async fn replies_reach_pending_game_reads_as_complete_frames() {
		use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
		let name = format!(
			r"\\.\pipe\serein-test-reply-{}-{}",
			std::process::id(),
			getrandom::u64().unwrap()
		);
		let mut server = ServerOptions::new()
			.first_pipe_instance(true)
			.create(&name)
			.unwrap();
		let mut game = ClientOptions::new().open(&name).unwrap();
		server.connect().await.unwrap();
		let reply = rpc::ready(user().id, &user().name);
		let mut buffer = vec![0; rpc::MAX_FRAME_BYTES + 8];
		let read = game.read(&mut buffer);
		tokio::pin!(read);
		// osu!'s SDK parses each completed pipe read as a whole frame. Start its
		// read first so a separately written header cannot hide in buffered data.
		tokio::select! {
			biased;
			result = &mut read => panic!("unexpected read before reply: {result:?}"),
			_ = tokio::task::yield_now() => {}
		}
		write_frame(&mut server, 1, &reply).await.unwrap();
		let length = timeout(Duration::from_secs(2), read)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(length, reply.len() + 8);
		assert_eq!(&buffer[..4], &1u32.to_le_bytes());
		assert_eq!(&buffer[4..8], &(reply.len() as u32).to_le_bytes());
		assert_eq!(&buffer[8..length], reply);
	}

	#[tokio::test]
	async fn frames_bound_before_allocation_and_handle_partial_reads() {
		let (mut client, mut server) = tokio::io::duplex(64);
		client.write_all(&1u32.to_le_bytes()).await.unwrap();
		client
			.write_all(&(rpc::MAX_FRAME_BYTES as u32 + 1).to_le_bytes())
			.await
			.unwrap();
		assert_eq!(
			read_frame(&mut server).await.unwrap_err().kind(),
			io::ErrorKind::InvalidData
		);
		let worker = tokio::spawn(async move {
			for byte in [1, 0, 0, 0, 2, 0, 0, 0, b'{', b'}'] {
				client.write_all(&[byte]).await.unwrap();
				tokio::task::yield_now().await;
			}
		});
		assert_eq!(read_frame(&mut server).await.unwrap(), (1, b"{}".to_vec()));
		worker.await.unwrap();
	}

	#[test]
	fn detection_fills_in_for_games_that_never_connect() {
		let games = Index::new(
			&discord_api::detectable::decode(
				br#"[{"id":"7","name":"Scanned game","executables":[{"name":"scanned"}]},
				     {"id":"8","name":"Other game","executables":[{"name":"other"}]}]"#,
			)
			.unwrap(),
		);
		let paths = ["/usr/bin/other".to_owned(), "/opt/scanned".to_owned()];
		// Without a previous match the first running process wins.
		assert_eq!(choose(&games, &paths, None).unwrap().0, model::Id(8));
		// With one, the running match is kept so the presence does not flap.
		assert_eq!(
			choose(&games, &paths, Some(model::Id(7))).unwrap().0,
			model::Id(7)
		);
		assert!(choose(&games, &["/usr/bin/none".to_owned()], None).is_none());

		let (activity, _) = watch::channel(None);
		let (report, _) = watch::channel(Ok(None));
		let scanned = rpc::ActivityFields::default()
			.into_activity(model::Id(7), "Scanned game".into())
			.unwrap();
		let mut values = std::array::from_fn(|_| None);
		publish(
			&values,
			&Some(scanned.clone()),
			&activity,
			&report,
			&egui::Context::default(),
		);
		assert_eq!(*activity.borrow(), Some(scanned.clone()));
		// A connected client knows more than a process name and always wins.
		let connected = rpc::ActivityFields {
			details: Some("Ranked match".into()),
			..Default::default()
		}
		.into_activity(model::Id(9), "Connected game".into())
		.unwrap();
		values[0] = Some((Instant::now(), connected.clone()));
		publish(
			&values,
			&Some(scanned.clone()),
			&activity,
			&report,
			&egui::Context::default(),
		);
		assert_eq!(*activity.borrow(), Some(connected));
		values[0] = None;
		publish(
			&values,
			&Some(scanned.clone()),
			&activity,
			&report,
			&egui::Context::default(),
		);
		assert_eq!(*activity.borrow(), Some(scanned));
	}

	#[test]
	fn latest_game_falls_back_and_clears_without_polling() {
		let (activity, _) = watch::channel(None);
		let (report, _) = watch::channel(Ok(None));
		let mut values = std::array::from_fn(|_| None);
		let first = rpc::ActivityFields::default()
			.into_activity(model::Id(7), "First game".into())
			.unwrap();
		let second = rpc::ActivityFields::default()
			.into_activity(model::Id(8), "Second game".into())
			.unwrap();
		values[0] = Some((Instant::now(), first.clone()));
		values[1] = Some((Instant::now() + Duration::from_millis(1), second.clone()));
		publish(
			&values,
			&None,
			&activity,
			&report,
			&egui::Context::default(),
		);
		assert_eq!(*activity.borrow(), Some(second));
		values[1] = None;
		publish(
			&values,
			&None,
			&activity,
			&report,
			&egui::Context::default(),
		);
		assert_eq!(*activity.borrow(), Some(first));
		let first = values[0].as_mut().unwrap();
		first.1.details = Some("  Next beatmap  ".into());
		first.1.state = Some(" ".into());
		first.1.assets = Some(rpc::Assets {
			large_image: Some("99".into()),
			..Default::default()
		});
		publish(
			&values,
			&None,
			&activity,
			&report,
			&egui::Context::default(),
		);
		let display = report.borrow().as_ref().unwrap().clone().unwrap();
		assert!(display.valid());
		assert_eq!(display.summary(), "Playing First game");
		assert_eq!(display.details.as_deref(), Some("Next beatmap"));
		assert!(display.state.is_none());
		assert_eq!(
			display.image,
			Some(model::ActivityImage::Asset {
				application: model::Id(7),
				asset: model::Id(99),
			})
		);
		values[0] = None;
		publish(
			&values,
			&None,
			&activity,
			&report,
			&egui::Context::default(),
		);
		assert!(activity.borrow().is_none());
		assert_eq!(*report.borrow(), Ok(None));
	}

	#[test]
	fn a_small_badge_never_becomes_the_artwork() {
		let activity = |assets: rpc::Assets| {
			rpc::ActivityFields {
				assets: Some(assets),
				..Default::default()
			}
			.into_activity(model::Id(7), "A game".into())
			.unwrap()
		};
		// The reported failure: only the badge resolved, so it was shown as the cover.
		let badge_only = display_activity(&activity(rpc::Assets {
			small_image: Some("42".into()),
			..Default::default()
		}));
		assert_eq!(
			badge_only.image,
			Some(model::ActivityImage::Application(model::Id(7)))
		);
		assert_eq!(
			badge_only.small_image,
			Some(model::ActivityImage::Asset {
				application: model::Id(7),
				asset: model::Id(42),
			})
		);
		let proxied = display_activity(&activity(rpc::Assets {
			large_image: Some("mp:external/hash-01/https/example.com/cover.png".into()),
			..Default::default()
		}));
		assert_eq!(
			proxied.image,
			Some(model::ActivityImage::Proxy(
				"external/hash-01/https/example.com/cover.png".into()
			))
		);
		assert!(proxied.valid());
	}

	#[tokio::test]
	async fn disable_and_sender_close_cancel_the_session_and_clear_latest() {
		use std::sync::atomic::{AtomicUsize, Ordering};
		struct Running(Arc<AtomicUsize>);
		impl Drop for Running {
			fn drop(&mut self) {
				self.0.fetch_sub(1, Ordering::SeqCst);
			}
		}
		let (enabled, receiver) = watch::channel(false);
		let (activity, mut changes) = watch::channel(None);
		let (report, _) = watch::channel(Ok(None));
		let running = Arc::new(AtomicUsize::new(0));
		let count = running.clone();
		let worker = tokio::spawn(async move {
			run_enabled(
				receiver,
				&activity,
				&report,
				&egui::Context::default(),
				|| async {
					count.fetch_add(1, Ordering::SeqCst);
					let _running = Running(count.clone());
					activity.send_replace(Some(
						rpc::ActivityFields::default()
							.into_activity(model::Id(7), "Synthetic game".into())
							.unwrap(),
					));
					std::future::pending().await
				},
			)
			.await;
		});
		tokio::task::yield_now().await;
		assert_eq!(running.load(Ordering::SeqCst), 0);
		for stop_with_drop in [false, true] {
			enabled.send(true).unwrap();
			timeout(Duration::from_secs(2), async {
				loop {
					changes.changed().await.unwrap();
					if changes.borrow_and_update().is_some() {
						break;
					}
				}
			})
			.await
			.unwrap();
			assert_eq!(running.load(Ordering::SeqCst), 1);
			if stop_with_drop {
				break;
			}
			enabled.send(false).unwrap();
			timeout(Duration::from_secs(2), async {
				loop {
					changes.changed().await.unwrap();
					if changes.borrow_and_update().is_none() {
						break;
					}
				}
			})
			.await
			.unwrap();
			assert_eq!(running.load(Ordering::SeqCst), 0);
		}
		drop(enabled);
		timeout(Duration::from_secs(2), worker)
			.await
			.unwrap()
			.unwrap();
		assert!(changes.borrow().is_none());
		assert_eq!(running.load(Ordering::SeqCst), 0);
	}

	#[test]
	fn choice_survives_late_load_and_failed_load_never_enables_sharing() {
		let mut settings = crate::toggle_setting::Settings::default();
		settings.restore(Err(local_store::StoreError::Unavailable));
		assert!(!settings.enabled);
		assert!(settings.failed);
		settings.observe(true);
		settings.restore(Ok(false));
		assert!(settings.enabled && settings.dirty && !settings.failed);
		settings.saving = true;
		settings.dirty = false;
		settings.observe(false);
		assert!(settings.dirty && settings.saving && !settings.enabled);
	}
}
