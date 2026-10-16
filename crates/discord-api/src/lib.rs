// Origin-fixed REST adapter. No cookies, redirects, logging, persistence or bot SDK.
mod activity_sharing;
mod archives;
mod badges;
mod channel_actions;
mod collectibles;
pub mod detectable;
pub mod external_assets;
mod forum;
mod gif_favorites;
mod group_actions;
mod guild_folders;
mod interactions;
mod message_options;
mod messaging_permissions;
mod onboarding;
mod profile_edit;
#[cfg(debug_assertions)]
pub use profile_edit::debug_profile_board_request_check;
pub mod proxy;
#[cfg(test)]
mod proxy_tests;
pub mod rpc;
mod server_actions;
mod server_admin;
mod server_audit_log;
mod server_integrations;
mod server_invites;
mod server_roles;
mod server_settings;
pub mod spotify;
pub mod upload;
mod user_actions;
use client_core::{
	Command, Event, Reply,
	auth::{AuthProvider, Failure, SessionSecret},
};
use discord_protocol::*;
use model::{Id, User};
use reqwest::{
	Client, Method, StatusCode,
	header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue},
};
use std::{
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	time::Duration,
};
use tokio::{
	sync::{Mutex, Semaphore},
	time::{Instant, sleep_until},
};

tokio::task_local! {
	/// Set while a hover preload runs.
	static SPECULATIVE: ();
}
/// REST permits a preload must leave free for requests the user is waiting for.
const FOREGROUND_PERMITS: usize = 2;
const READ_RATE_LIMIT_RETRIES: u8 = 2;
/// Longer JSON limits fail the read at once, so sign-in never stalls silently.
const MAX_READ_RATE_LIMIT_WAIT: Duration = Duration::from_secs(10);
const API_ORIGINS: [&str; 2] = [
	"https://discord.com/api/v10",
	"https://ptb.discord.com/api/v10",
];
static API_ON_TEST_HOST: AtomicBool = AtomicBool::new(false);

/// Later lookups in this process follow a Cloudflare block of discord.com.
pub fn api_v10_base() -> &'static str {
	API_ORIGINS[usize::from(API_ON_TEST_HOST.load(Ordering::Acquire))]
}

pub struct DiscordApi {
	interaction_session: std::sync::Mutex<Option<zeroize::Zeroizing<String>>>,
	ack_token: Mutex<zeroize::Zeroizing<Option<String>>>,
	client: Client,
	proxy: Mutex<ProxyState>,
	upload_client: tokio::sync::OnceCell<Client>,
	secret: Arc<SessionSecret>,
	cooldown: Mutex<Instant>,
	requests: Semaphore,
	stopped: AtomicBool,
	catalog: std::sync::Mutex<collectibles::Catalog>,
	#[cfg(test)]
	base: String,
	/// Stands in for the public test host; tests keep the process-wide switch untouched.
	#[cfg(test)]
	test_host: Option<(String, AtomicBool)>,
	#[cfg(test)]
	upload_origin: Option<std::net::SocketAddr>,
}
struct ProxyState {
	receiver: tokio::sync::watch::Receiver<Option<proxy::ApiProxy>>,
	cached: Option<(proxy::ApiProxy, Client)>,
}

enum RequestContent {
	Json(serde_json::Value),
	Multipart { content_type: String, body: Vec<u8> },
}

fn stream_preview_url(bytes: &[u8], key: &str) -> Result<String, Failure> {
	#[derive(serde::Deserialize)]
	struct Preview {
		url: String,
	}
	let url = decode::<Preview>(bytes).map_err(|_| Failure::Protocol)?.url;
	let path = [
		format!("https://cdn.discordapp.com/streams/{key}/"),
		format!("https://media.discordapp.net/streams/{key}/"),
	]
	.into_iter()
	.find_map(|prefix| url.strip_prefix(&prefix));
	let Some(path) = path else {
		return Err(Failure::Protocol);
	};
	let hash = path.split_once('?').map_or(path, |(path, _)| path);
	let hash = hash.strip_suffix(".png").unwrap_or(hash);
	if url.len() > 2048
		|| url
			.bytes()
			.any(|byte| byte.is_ascii_control() || byte == b'\\')
		|| !(16..=128).contains(&hash.len())
		|| !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
	{
		return Err(Failure::Protocol);
	}
	Ok(url)
}
// Unofficial wire fields: discord.py-self errors.py CaptchaRequired and http.py request.
// Borrow ordinary fields; escaped JSON strings are bounded by the 64 KiB wire cap.
fn invite_captcha(bytes: &[u8]) -> Option<client_core::captcha::Challenge> {
	use std::borrow::Cow;
	#[derive(serde::Deserialize)]
	struct Captcha<'a> {
		#[serde(borrow)]
		captcha_service: Cow<'a, str>,
		#[serde(borrow)]
		captcha_sitekey: Cow<'a, str>,
		#[serde(borrow)]
		captcha_rqdata: Option<Cow<'a, str>>,
		#[serde(borrow)]
		captcha_rqtoken: Option<Cow<'a, str>>,
		#[serde(borrow)]
		captcha_session_id: Option<Cow<'a, str>>,
		#[serde(default)]
		should_serve_invisible: bool,
	}
	impl Drop for Captcha<'_> {
		fn drop(&mut self) {
			for value in [
				&mut self.captcha_rqdata,
				&mut self.captcha_rqtoken,
				&mut self.captcha_session_id,
			] {
				if let Some(Cow::Owned(value)) = value {
					zeroize::Zeroize::zeroize(value);
				}
			}
		}
	}
	let mut c: Captcha<'_> = serde_json::from_slice(bytes).ok()?;
	(c.captcha_service == "hcaptcha").then_some(())?;
	client_core::captcha::Challenge::new(
		c.captcha_sitekey.to_string(),
		c.captcha_rqdata.take().map(Cow::into_owned),
		c.captcha_rqtoken.take().map(Cow::into_owned),
		c.captcha_session_id.take().map(Cow::into_owned),
		c.should_serve_invisible,
	)
}
/// Headers Discord's web client sends on every REST call; without them a normal-user
/// session is classified as automated and quarantined (spam flag, attachment limits).
fn fingerprint_headers() -> Result<reqwest::header::HeaderMap, Failure> {
	let mut headers = reqwest::header::HeaderMap::new();
	headers.insert(
		"x-super-properties",
		HeaderValue::from_str(&client_core::fingerprint::super_properties())
			.map_err(|_| Failure::Network)?,
	);
	headers.insert(
		"x-discord-locale",
		HeaderValue::from_static(client_core::fingerprint::LOCALE),
	);
	headers.insert("x-discord-timezone", HeaderValue::from_static("UTC"));
	headers.insert(
		reqwest::header::ACCEPT_LANGUAGE,
		HeaderValue::from_static("en-US,en;q=0.9"),
	);
	headers.insert(
		reqwest::header::ORIGIN,
		HeaderValue::from_static("https://discord.com"),
	);
	headers.insert(
		reqwest::header::REFERER,
		HeaderValue::from_static("https://discord.com/channels/@me"),
	);
	Ok(headers)
}
fn api_client(route: &proxy::ApiProxy) -> Result<Client, Failure> {
	let builder = Client::builder()
		.redirect(reqwest::redirect::Policy::none())
		// Keep writes single-attempt, including attachment slot allocation.
		.retry(reqwest::retry::never())
		.timeout(Duration::from_secs(20))
		.connect_timeout(Duration::from_secs(10))
		.user_agent(client_core::fingerprint::user_agent())
		.default_headers(fingerprint_headers()?);
	route
		.client_builder(builder)
		.map_err(Failure::ProtocolAt)?
		.build()
		.map_err(|_| Failure::ProtocolAt("API proxy connection unavailable"))
}
/// How long a profile card waits for its optional board-game names and collectible artwork;
/// a card without them is still complete, so it opens rather than waiting on a slow read.
const PROFILE_EXTRAS: Duration = Duration::from_millis(600);
impl DiscordApi {
	pub fn new(secret: Arc<SessionSecret>) -> Result<Self, Failure> {
		Self::with_proxy(
			secret,
			tokio::sync::watch::channel(Some(proxy::ApiProxy::Direct)).1,
		)
	}
	/// None waits for extension configuration; requests never race ahead using a direct route.
	pub fn with_proxy(
		secret: Arc<SessionSecret>,
		receiver: tokio::sync::watch::Receiver<Option<proxy::ApiProxy>>,
	) -> Result<Self, Failure> {
		let client = api_client(&proxy::ApiProxy::Direct)?;
		Ok(Self {
			interaction_session: std::sync::Mutex::new(None),
			upload_client: tokio::sync::OnceCell::new(),
			ack_token: Mutex::new(zeroize::Zeroizing::new(None)),
			client,
			proxy: Mutex::new(ProxyState {
				receiver,
				cached: None,
			}),
			secret,
			cooldown: Mutex::new(Instant::now()),
			requests: Semaphore::new(4),
			stopped: AtomicBool::new(false),
			catalog: std::sync::Mutex::default(),
			#[cfg(test)]
			base: "https://discord.com/api/v10".into(),
			#[cfg(test)]
			test_host: None,
			#[cfg(test)]
			upload_origin: None,
		})
	}
	/// Credential-free client snapshot for Discord REST; authorization is added per request.
	pub async fn rest_client(&self) -> Result<Client, Failure> {
		let mut state = self.proxy.lock().await;
		let route = loop {
			let current = state.receiver.borrow_and_update().clone();
			if let Some(route) = current {
				break route;
			}
			state.cached = None;
			state
				.receiver
				.changed()
				.await
				.map_err(|_| Failure::ProtocolAt("API proxy configuration unavailable"))?;
		};
		if let Some((previous, client)) = &state.cached
			&& previous == &route
		{
			return Ok(client.clone());
		}
		// Drop the old route first: invalid configuration must never reuse a direct connection.
		state.cached = None;
		let client = if route == proxy::ApiProxy::Direct {
			self.client.clone()
		} else {
			api_client(&route)?
		};
		state.cached = Some((route, client.clone()));
		Ok(client)
	}
	pub fn stop(&self) {
		self.stopped.store(true, Ordering::Release);
	}
	pub fn stopped(&self) -> bool {
		self.stopped.load(Ordering::Acquire)
	}
	async fn request(
		&self,
		method: Method,
		path: &str,
		body: Option<serde_json::Value>,
	) -> Result<Vec<u8>, Failure> {
		self.request_limited(method, path, body, MAX_WIRE).await
	}
	async fn request_limited(
		&self,
		method: Method,
		path: &str,
		body: Option<serde_json::Value>,
		max_bytes: usize,
	) -> Result<Vec<u8>, Failure> {
		self.request_with_content(
			method,
			path,
			body.map(RequestContent::Json),
			max_bytes,
			None,
			None,
		)
		.await
	}
	async fn request_multipart_limited(
		&self,
		path: &str,
		content_type: String,
		body: Vec<u8>,
		max_bytes: usize,
	) -> Result<Vec<u8>, Failure> {
		self.request_with_content(
			Method::POST,
			path,
			Some(RequestContent::Multipart { content_type, body }),
			max_bytes,
			None,
			None,
		)
		.await
	}
	/// One typed request with an optional captcha retry and challenge output slot.
	async fn request_with_captcha(
		&self,
		method: Method,
		path: &str,
		body: Option<serde_json::Value>,
		max_bytes: usize,
		retry: Option<&client_core::captcha::Retry>,
		challenge: Option<&mut Option<client_core::captcha::Challenge>>,
	) -> Result<Vec<u8>, Failure> {
		self.request_with_content(
			method,
			path,
			body.map(RequestContent::Json),
			max_bytes,
			retry,
			challenge,
		)
		.await
	}
	/// The REST origin for the next attempt and whether it is still discord.com.
	#[cfg(not(test))]
	fn api_origin(&self) -> (&'static str, bool) {
		let base = api_v10_base();
		(base, base == API_ORIGINS[0])
	}
	/// Without a stand-in test host, a test behaves as if the process already moved.
	#[cfg(test)]
	fn api_origin(&self) -> (&str, bool) {
		match &self.test_host {
			Some((origin, moved)) if moved.load(Ordering::Acquire) => (origin, false),
			Some(_) => (&self.base, true),
			None => (&self.base, false),
		}
	}
	fn move_to_test_host(&self) {
		#[cfg(not(test))]
		API_ON_TEST_HOST.store(true, Ordering::Release);
		#[cfg(test)]
		if let Some((_, moved)) = &self.test_host {
			moved.store(true, Ordering::Release);
		}
	}
	async fn request_with_content(
		&self,
		method: Method,
		path: &str,
		body: Option<RequestContent>,
		max_bytes: usize,
		retry: Option<&client_core::captcha::Retry>,
		mut challenge: Option<&mut Option<client_core::captcha::Challenge>>,
	) -> Result<Vec<u8>, Failure> {
		// Only typed adapter methods construct paths. Never accept a URL or route from UI/content.
		if !path.starts_with('/')
			|| path.contains("://")
			|| path.contains('\\')
			|| path.contains("..")
		{
			return Err(Failure::Protocol);
		}
		let speculative = SPECULATIVE.try_with(|()| ()).is_ok();
		let _permit = if speculative {
			// A hover preload takes only spare capacity, so it never delays a click.
			if self.requests.available_permits() <= FOREGROUND_PERMITS {
				return Err(Failure::Capacity);
			}
			self.requests.try_acquire().map_err(|_| Failure::Capacity)?
		} else {
			self.requests
				.acquire()
				.await
				.map_err(|_| Failure::Network)?
		};
		// Four permits bound concurrent REST work. A slow profile body must not hold the
		// cooldown mutex and delay a message write; only service rate admission is shared.
		let idempotent = method == Method::GET && body.is_none() && !speculative;
		let mut body = body;
		let mut rate_limits = 0u8;
		loop {
			loop {
				let next = *self.cooldown.lock().await;
				if speculative && Instant::now() < next {
					return Err(Failure::Capacity);
				}
				sleep_until(next).await;
				if Instant::now() >= *self.cooldown.lock().await {
					break;
				}
			}
			if self.stopped() {
				return Err(Failure::Expired);
			}
			let mut authorization = HeaderValue::from_str(self.secret.expose())
				.map_err(|_| Failure::InvalidCredential)?;
			authorization.set_sensitive(true);
			let (base, on_stable) = self.api_origin();
			let write = method != Method::GET;
			let mut request = self
				.rest_client()
				.await?
				.request(method.clone(), format!("{base}{path}"))
				.header(AUTHORIZATION, authorization);
			if let Some(retry) = retry {
				if retry.expired() {
					return Err(Failure::ProtocolAt(
						"Verification expired; start the check again",
					));
				}
				for (name, value) in [
					("x-captcha-key", Some(retry.passcode())),
					("x-captcha-rqtoken", retry.rqtoken()),
					("x-captcha-session-id", retry.session_id()),
				] {
					if let Some(value) = value {
						let mut value =
							HeaderValue::from_str(value).map_err(|_| Failure::Protocol)?;
						value.set_sensitive(true);
						request = request.header(name, value);
					}
				}
			}
			if let Some(content) = body.take() {
				request = match content {
					RequestContent::Json(body) => request.json(&body),
					RequestContent::Multipart { content_type, body } => request
						.header(
							CONTENT_TYPE,
							HeaderValue::from_str(&content_type).map_err(|_| Failure::Protocol)?,
						)
						.body(body),
				};
			}
			let mut response = request.send().await.map_err(|_| {
				if write {
					Failure::Ambiguous
				} else {
					Failure::Network
				}
			})?;
			let status = response.status();
			let exhausted = response
				.headers()
				.get("x-ratelimit-remaining")
				.and_then(|v| v.to_str().ok())
				== Some("0");
			let reset = response
				.headers()
				.get("x-ratelimit-reset-after")
				.and_then(|v| v.to_str().ok())
				.and_then(|s| s.parse::<f64>().ok());
			let retry_header = response
				.headers()
				.get("retry-after")
				.and_then(|v| v.to_str().ok())
				.and_then(|s| s.parse::<f64>().ok());
			if exhausted && status != StatusCode::TOO_MANY_REQUESTS {
				let mut next = self.cooldown.lock().await;
				*next = (*next).max(Instant::now() + safe_delay(reset)?);
			}
			if status == StatusCode::UNAUTHORIZED {
				self.stop();
				return Err(Failure::Expired);
			}
			let cloudflare_block = response
				.headers()
				.get(CONTENT_TYPE)
				.and_then(|value| value.to_str().ok())
				.is_some_and(|value| {
					value.starts_with("text/html") || value.starts_with("text/plain")
				});
			if response
				.content_length()
				.is_some_and(|n| n > max_bytes as u64)
			{
				return Err(Failure::Capacity);
			}
			let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(
				response.content_length().unwrap_or(0).min(max_bytes as u64) as usize,
			));
			while let Some(chunk) = response.chunk().await.map_err(|_| {
				if write {
					Failure::Ambiguous
				} else {
					Failure::Network
				}
			})? {
				if bytes.len() + chunk.len() > max_bytes {
					return Err(Failure::Capacity);
				}
				bytes.extend_from_slice(&chunk);
			}
			if !status.is_success() {
				let error = decode::<ErrorBody>(&bytes).unwrap_or_default();
				if error.captcha_key.is_some() || matches!(error.code, Some(60003 | 50014)) {
					let auth_challenge = matches!(error.code, Some(60003 | 50014));
					if !auth_challenge
						&& error.captcha_key.is_some()
						&& matches!(status, StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN)
						&& let Some(output) = challenge.as_mut()
					{
						if let Some(parsed) = invite_captcha(&bytes) {
							**output = Some(parsed);
							return Err(Failure::Challenged);
						}
						return Err(Failure::ProtocolAt(
							"This verification is unavailable; complete the action in the official client",
						));
					}
					if !auth_challenge && write && challenge.is_none() {
						// The service can require a captcha for one write (for example a friend
						// request). No solver is wired for this action, but it is not a session
						// challenge: keep the connection and report a bounded local reason.
						return Err(Failure::ProtocolAt(
							"Discord requires verification for this action; complete it in the official client",
						));
					}

					self.stop();
					return Err(Failure::Challenged);
				}
				if status == StatusCode::TOO_MANY_REQUESTS && cloudflare_block {
					if on_stable {
						self.move_to_test_host();
						if idempotent {
							continue;
						}
						return Err(Failure::RateLimited);
					}
					// A block page has no JSON penalty. The short read retry must not run.
					let delay = safe_delay(retry_header.or(reset).or(Some(60.0)))?;
					let mut next = self.cooldown.lock().await;
					*next = (*next).max(Instant::now() + delay);
					return Err(Failure::RateLimited);
				}
				if status == StatusCode::TOO_MANY_REQUESTS {
					// The body is the penalty Discord documents. A missing or shorter reset
					// header must not send the retry early.
					let delay = safe_delay(error.retry_after.or(retry_header).or(reset))?;
					{
						let mut next = self.cooldown.lock().await;
						*next = (*next).max(Instant::now() + delay);
					}
					if idempotent
						&& delay <= MAX_READ_RATE_LIMIT_WAIT
						&& rate_limits < READ_RATE_LIMIT_RETRIES
					{
						rate_limits += 1;
						continue;
					}
					return Err(Failure::RateLimited);
				}
				// A missing private note is empty, not a missing user profile. No other 404 is converted.
				if !write
					&& status == StatusCode::NOT_FOUND
					&& path.starts_with("/users/@me/notes/")
				{
					return Ok(br#"{"note":""}"#.to_vec());
				}
				return Err(if status == StatusCode::FORBIDDEN {
					Failure::Forbidden
				} else if status.is_server_error() && write {
					Failure::Ambiguous
				} else {
					Failure::Protocol
				});
			}
			return Ok(std::mem::take(&mut *bytes));
		}
	}
	pub async fn gateway_url(&self) -> Result<String, Failure> {
		let bytes = self
			.request(Method::GET, "/gateway", None)
			.await
			.map_err(|f| f.protocol_at("Gateway discovery: HTTP response rejected"))?;
		decode::<GatewayLocation>(&bytes)
			.map(|g| g.url)
			.map_err(|_| Failure::ProtocolAt("Gateway discovery: response format unsupported"))
	}
	pub async fn current_user(&self) -> Result<User, Failure> {
		let bytes = self
			.request(Method::GET, "/users/@me", None)
			.await
			.map_err(|f| f.protocol_at("Account verification: HTTP response rejected"))?;
		let user = decode::<UserDto>(&bytes).map_err(|_| {
			Failure::ProtocolAt("Account verification: user response format unsupported")
		})?;
		if user.bot {
			self.stop();
			return Err(Failure::InvalidCredential);
		}
		Ok(user.into_model())
	}
	/// Unofficial normal-user DM endpoint; no retry of ambiguous ringing writes.
	pub async fn ring_call(
		&self,
		channel: model::Id,
		recipient: Option<model::Id>,
		stop: bool,
	) -> Result<(), Failure> {
		let route = if stop { "stop-ringing" } else { "ring" };
		let body = match recipient {
			Some(recipient) => serde_json::json!({"recipients":[recipient]}),
			None if stop => serde_json::json!({}),
			None => serde_json::json!({"recipients":null}),
		};
		self.request(
			Method::POST,
			&format!("/channels/{channel}/call/{route}"),
			Some(body),
		)
		.await
		.map(|_| ())
	}
	// https://docs.discord.com/developers/resources/invite#get-invite
	async fn invite(&self, code: &str) -> Result<model::InvitePreview, Failure> {
		if !client_core::invites::valid_code(code) {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/invites/{code}?with_counts=true"),
				None,
				64 * 1024,
			)
			.await?;
		discord_protocol::invites::decode(&bytes).map_err(|_| Failure::Protocol)
	}
	// Unofficial user endpoint; observed in discord.py-self/http.py accept_invite (2026-09-11).
	// One explicit human solution may resume this specific write; never loop/retry automatically.
	/// Accepts one invite, optionally resuming a single user-solved challenge.
	async fn join_invite(
		&self,
		code: &str,
		request: u64,
		captcha: Option<Box<client_core::captcha::Retry>>,
	) -> Event {
		let mut challenge = None;
		let result = if client_core::invites::valid_code(code)
			&& captcha.as_ref().is_none_or(|c| {
				c.matches(
					&client_core::captcha::Target::Invite {
						code: code.to_owned(),
					},
					request,
				)
			}) {
			self.request_with_captcha(
				Method::POST,
				&format!("/invites/{code}"),
				Some(serde_json::json!({})),
				64 * 1024,
				captcha.as_deref(),
				Some(&mut challenge),
			)
			.await
			.and_then(|bytes| {
				discord_protocol::invites::decode(&bytes)
					.map(|p| p.guild)
					.map_err(|_| Failure::Ambiguous)
			})
		} else {
			Err(Failure::Protocol)
		};
		if let Some(challenge) = challenge {
			Event::InviteChallenge {
				request,
				challenge: Box::new(challenge),
			}
		} else {
			Event::JoinInvite {
				request,
				result: result.map_err(|f| {
					f.protocol_at(
						"Invite rejected - it may be expired, invalid, or require joining in Discord",
					)
				}),
			}
		}
	}
	// Unofficial normal-user endpoint; observed in discord.py-self/http.py create_guild
	// (2026-09-23). This non-idempotent write is deliberately attempted only once.
	async fn create_guild(
		&self,
		request: &client_core::guild_creation::Request,
	) -> Result<model::Id, Failure> {
		#[derive(serde::Deserialize)]
		struct CreatedGuild {
			id: model::Id,
		}
		if !request.valid() {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::POST,
				"/guilds",
				Some(serde_json::json!({
					"name": request.name(),
					"icon": request.icon(),
					"system_channel_id": null,
					"channels": [],
					"guild_template_code": "2TffvPucqHkN",
				})),
				64 * 1024,
			)
			.await?;
		let guild = decode::<CreatedGuild>(&bytes).map_err(|_| Failure::Ambiguous)?;
		(guild.id.0 != 0)
			.then_some(guild.id)
			.ok_or(Failure::Ambiguous)
	}
	/// One bounded 50-message history page, before or after a cursor or at the latest message.
	async fn history(
		&self,
		channel: Id,
		before: Option<Id>,
		after: Option<Id>,
		request: u64,
	) -> Event {
		if before.is_some() && after.is_some() {
			return Event::Failure(Failure::Protocol);
		}
		let mut path = format!("/channels/{channel}/messages?limit=50");
		if let Some(before) = before {
			path.push_str(&format!("&before={before}"));
		}
		if let Some(after) = after {
			path.push_str(&format!("&after={after}"));
		}
		match self
			.request(Method::GET, &path, None)
			.await
			.and_then(|bytes| decode::<Vec<MessageDto>>(&bytes).map_err(|_| Failure::Protocol))
		{
			Ok(messages) if messages.len() <= 50 => Event::History {
				channel,
				request,
				older: before.is_some(),
				messages: messages.into_iter().map(MessageDto::into_model).collect(),
			},
			Ok(_) => Event::Failure(Failure::Capacity),
			Err(Failure::Forbidden) => Event::Unavailable(channel),
			Err(f) => Event::Failure(f),
		}
	}
	/// Runs one typed command and returns its typed event.
	/// [`Self::execute`] for the write queue. Like the official client, a message Discord
	/// rate-limits (documented: a 429 request is not processed, so nothing was created) stays
	/// pending and is resent with the same nonce once the service's wait has passed; `execute`
	/// sleeps out that shared cooldown first. `throttled` hears each wait. Ambiguous outcomes are
	/// never resent.
	pub async fn execute_write(&self, command: Command, mut throttled: impl FnMut(&str)) -> Event {
		/// Resends of one message, each after the wait Discord asked for.
		const MAX_SEND_RETRIES: usize = 30;
		let resend = match &command {
			Command::Send {
				sticker,
				channel,
				content,
				nonce,
				reply,
			} => Some((*sticker, *channel, content.clone(), nonce.clone(), *reply)),
			_ => None,
		};
		let mut event = self.execute(command).await;
		let Some((sticker, channel, content, nonce, reply)) = resend else {
			return event;
		};
		for _ in 0..MAX_SEND_RETRIES {
			if !matches!(
				event,
				Event::SendResult {
					result: Err(Failure::RateLimited),
					..
				}
			) || self.stopped()
			{
				break;
			}
			throttled(&nonce);
			event = self
				.execute(Command::Send {
					sticker,
					channel,
					content: content.clone(),
					nonce: nonce.clone(),
					reply,
				})
				.await;
		}
		event
	}
	pub async fn execute(&self, command: Command) -> Event {
		match command {
			Command::ApplicationCommands {
				channel,
				guild,
				request,
			} => Event::ApplicationCommands {
				channel,
				request,
				result: self.application_commands(channel, guild).await,
			},
			Command::Interaction(request) => {
				Event::Interaction(client_core::interactions::Event::Submitted {
					result: self.interaction(&request, None).await,
					nonce: request.nonce,
				})
			}
			Command::MessagingPermissions { request, change } => Event::MessagingPermissions {
				request,
				result: self.account_messaging_permissions(change).await,
			},
			Command::ServerAdmin {
				guild,
				request,
				action,
			} => Event::ServerAdmin(self.server_admin(guild, request, &action).await),
			Command::ServerSettings {
				guild,
				request,
				edit,
			} => Event::ServerSettings(self.server_settings(guild, request, edit).await),
			Command::Onboarding {
				guild,
				request,
				action,
			} => Event::Onboarding(self.onboarding(guild, request, action).await),
			Command::SendServerInvite {
				guild,
				user,
				code,
				nonce,
				request,
			} => Event::ServerAction(client_core::server_actions::Event::InviteSent {
				guild,
				user,
				request,
				result: self
					.send_server_invite(user, &code, &nonce)
					.await
					.map(Box::new),
			}),
			Command::GroupAction { action, request } => {
				Event::GroupAction(client_core::group_actions::Event::Written {
					channel: action.channel(),
					request,
					result: self.group_action(action).await,
				})
			}
			Command::JoinInvite {
				code,
				request,
				captcha,
			} => self.join_invite(&code, request, captcha).await,
			Command::CreateGuild { request, sequence } => Event::GuildCreated {
				sequence,
				result: self.create_guild(&request).await,
			},
			Command::GuildFolders(settings) => Event::GuildFolders(match settings {
				Some((base, settings)) => self.save_guild_folders(base, settings).await,
				None => self.guild_folders().await,
			}),
			Command::ChannelAction {
				guild,
				channel,
				request,
				action,
			} => Event::ChannelAction(client_core::channel_actions::Event::Finished {
				guild,
				channel,
				request,
				result: self.channel_action(guild, channel, &action).await,
			}),
			Command::ServerAction { action, request } => {
				Event::ServerAction(client_core::server_actions::Event::Written {
					action,
					request,
					result: self.server_action(action).await,
				})
			}
			Command::UserAction {
				action,
				request,
				captcha,
			} => {
				if let client_core::user_actions::Action::OpenDm(user) = action {
					let mut challenge = None;
					let result = self
						.open_dm(user, captcha.as_deref(), Some(&mut challenge))
						.await;
					if let Some(challenge) = challenge {
						return Event::UserAction(client_core::user_actions::Event::Challenge {
							action,
							request,
							challenge: Box::new(challenge),
						});
					}
					return Event::UserAction(client_core::user_actions::Event::DmOpened {
						user,
						request,
						result: result.map(Box::new),
					});
				}
				if let client_core::user_actions::Action::LoadNote(user) = action {
					return Event::UserAction(client_core::user_actions::Event::NoteLoaded {
						user,
						request,
						result: self.user_note(user).await,
					});
				}
				let mut challenge = None;
				let slot =
					client_core::user_actions::challengeable(&action).then_some(&mut challenge);
				let result = self.user_action(&action, captcha.as_deref(), slot).await;
				if let Some(challenge) = challenge {
					return Event::UserAction(client_core::user_actions::Event::Challenge {
						action,
						request,
						challenge: Box::new(challenge),
					});
				}
				Event::UserAction(client_core::user_actions::Event::Written {
					action,
					request,
					result,
				})
			}
			Command::Invite { code } => {
				let result = self.invite(&code).await.map(Box::new);
				Event::Invite { code, result }
			}
			Command::CreatePost {
				parent,
				guild,
				title,
				content,
				attachments,
				tags,
				request,
			} => {
				// Files are staged by the upload worker, which owns the whole post request.
				let result = if attachments.is_empty() {
					self.create_post(parent, guild, (&title, &tags), &content, None)
						.await
				} else {
					Err(Failure::ProtocolAt(
						"Upload unavailable; reselect the file to retry",
					))
				};
				Event::PostCreated {
					parent,
					request,
					result,
				}
			}
			Command::ForumSummaries { channels, request } => {
				Event::ForumSummaries {
					request,
					results: futures_util::future::join_all(channels.into_iter().map(
						|channel| async move { (channel, self.forum_summary(channel).await) },
					))
					.await,
				}
			}
			Command::ForumPosts {
				parent,
				guild,
				offset,
				request,
			} => {
				let result = self.forum_posts(parent, guild, offset).await;
				Event::ForumPosts {
					parent,
					request,
					result,
				}
			}
			Command::Archives {
				parent,
				guild,
				kind,
				before,
				request,
			} => {
				let result = self.archives(parent, guild, kind, before).await;
				Event::Archives {
					parent,
					request,
					result,
				}
			}
			Command::ThreadStarter {
				thread,
				parent,
				request,
			} => {
				// Documented single-message read; a thread shares its id with its starter.
				let result = self
					.request(
						Method::GET,
						&format!("/channels/{parent}/messages/{thread}"),
						None,
					)
					.await
					.and_then(|bytes| {
						decode::<MessageDto>(&bytes)
							.map(MessageDto::into_model)
							.map_err(|_| Failure::Protocol)
					})
					.and_then(|message| {
						if message.id == thread && message.channel == parent {
							Ok(message)
						} else {
							Err(Failure::Protocol)
						}
					});
				Event::ThreadStarter {
					thread,
					request,
					result,
				}
			}
			Command::Pins {
				channel,
				before,
				request,
			} => {
				let result = self.pins(channel, before).await;
				Event::Search {
					channel,
					request,
					result,
				}
			}
			Command::Search {
				channel,
				guild,
				query,
				before,
				offset,
				oldest,
				request,
			} => {
				let result = self
					.search(channel, guild, &query, before, offset, oldest)
					.await;
				Event::Search {
					channel,
					request,
					result,
				}
			}
			Command::CancelSearch => Event::Failure(Failure::Protocol),
			Command::Gifs { query, request } => Event::Gifs {
				request,
				result: self.gifs(query.as_deref()).await,
			},
			Command::CancelGifs => Event::Failure(Failure::Protocol),
			Command::GifFavorites { request, change } => Event::GifFavorites {
				request,
				result: self.gif_favorites(change).await,
			},
			Command::MarkRead {
				channel,
				message,
				request,
				manual,
				mention_count,
			} => {
				let result = self
					.mark_read(channel, message, manual, mention_count)
					.await;
				Event::ReadState(client_core::read_state::Event::Result {
					channel,
					message,
					request,
					result,
				})
			}
			Command::MarkGuildRead { guild, request } => {
				let result = self.mark_guild_read(guild).await;
				Event::ReadState(client_core::read_state::Event::GuildAck {
					guild,
					request,
					result,
				})
			}
			Command::MarkAllRead { request, channels } => {
				let (acked, result) = self.mark_all_read(&channels).await;
				Event::ReadState(client_core::read_state::Event::AllAck {
					request,
					acked,
					result,
				})
			}
			Command::Polls(command) => {
				use client_core::polls::{Action, Event as E};
				let channel = command.channel;
				let message = command.message;
				let result = async {
					if channel == Id(0) {
						return Err(Failure::Protocol);
					}
					let bytes = match command.action {
						Action::Create(create) => {
							if message.is_some() || !create.valid() {
								return Err(Failure::Protocol);
							}
							let answers: Vec<_> = create
								.answers
								.iter()
								.map(|a| {
									let emoji = a.emoji.as_ref().map(|e| match e.id {
										Some(id) => serde_json::json!({"id": id.to_string()}),
										None => serde_json::json!({"name": e.name}),
									});
									let mut media = serde_json::json!({"text":a.text});
									if let Some(emoji) = emoji {
										media["emoji"] = emoji;
									}
									serde_json::json!({"poll_media":media})
								})
								.collect();
							self.request(
								Method::POST,
								&format!("/channels/{channel}/messages"),
								Some(serde_json::json!({
									"poll":{"question":{"text":create.question},"answers":answers,"duration":create.duration,
									"allow_multiselect":create.multiselect,"layout_type":1}, "nonce":command.nonce,
									"allowed_mentions":{"parse":[],"replied_user":false}
								})),
							)
							.await?
						}
						action => {
							let id = message.filter(|id| *id != Id(0)).ok_or(Failure::Protocol)?;
							match action {
								Action::End => {
									return self
										.request(
											Method::POST,
											&format!("/channels/{channel}/polls/{id}/expire"),
											None,
										)
										.await
										.and_then(|bytes| {
											decode::<MessageDto>(&bytes)
												.map(MessageDto::into_model)
												.map_err(|_| Failure::Protocol)
										});
								}
								Action::Vote(answers) => {
									if answers.len() > model::polls::MAX_ANSWERS
										|| answers
											.iter()
											.enumerate()
											.any(|(i, a)| *a == 0 || answers[..i].contains(a))
									{
										return Err(Failure::Protocol);
									}
									self.request(
										Method::PUT,
										&format!("/channels/{channel}/polls/{id}/answers/@me"),
										Some(serde_json::json!({"answer_ids":answers})),
									)
									.await?;
								}
								Action::Read => {}
								Action::Create(_) => unreachable!(),
							}
							let bytes = self
								.request(
									Method::GET,
									&format!("/channels/{channel}/messages?limit=1&around={id}"),
									None,
								)
								.await?;
							let [dto] =
								decode::<[MessageDto; 1]>(&bytes).map_err(|_| Failure::Protocol)?;
							if dto.id != id || dto.channel_id != channel {
								return Err(Failure::Protocol);
							}
							return Ok(dto.into_model());
						}
					};
					decode::<MessageDto>(&bytes)
						.map(MessageDto::into_model)
						.map_err(|_| Failure::Protocol)
				}
				.await;
				Event::Polls(E::Result {
					channel,
					message,
					request: command.request,
					result: result.map(Box::new),
				})
			}
			Command::Reactions(command) => {
				use client_core::reactions::{Command as R, Event as E};
				Event::Reactions(match command {
					R::Read {
						channel,
						message,
						request,
					} => {
						let result = self
							.request(
								Method::GET,
								&format!("/channels/{channel}/messages?limit=1&around={message}"),
								None,
							)
							.await
							.and_then(|bytes| {
								// Normal-user sessions read a message through history, not
								// the bot-only single-message endpoint. Never use a neighbor.
								let [dto] = decode::<[MessageDto; 1]>(&bytes)
									.map_err(|_| Failure::Protocol)?;
								if dto.id != message || dto.channel_id != channel {
									return Err(Failure::Protocol);
								}
								Ok(dto.into_model().reactions.unwrap_or_default())
							});
						E::Read {
							channel,
							message,
							request,
							result,
						}
					}
					R::Set {
						channel,
						message,
						emoji,
						add,
						request,
					} => {
						let result = match reaction_path(channel, message, &emoji) {
							Some(path) => self
								.request(
									if add { Method::PUT } else { Method::DELETE },
									&path,
									None,
								)
								.await
								.map(|_| ()),
							None => Err(Failure::Protocol),
						};
						E::Written {
							channel,
							message,
							request,
							result,
						}
					}
					R::Users {
						channel,
						message,
						emoji,
						after,
						request,
					} => {
						let result = match reaction_users_path(channel, message, &emoji, after) {
							Some(path) => {
								self.request(Method::GET, &path, None)
									.await
									.and_then(|bytes| {
										let users = decode::<Vec<UserDto>>(&bytes)
											.map_err(|_| Failure::Protocol)?;
										if users.len() > client_core::reactions::REACTION_USER_PAGE
										{
											return Err(Failure::Protocol);
										}
										Ok(users.into_iter().map(UserDto::into_model).collect())
									})
							}
							None => Err(Failure::Protocol),
						};
						E::Users {
							channel,
							message,
							emoji,
							request,
							result,
						}
					}
				})
			}
			Command::Profile {
				user,
				guild,
				request,
				with_mutuals,
				cosmetics,
			} => {
				let mut path = format!(
					"/users/{user}/profile?with_mutual_guilds={with_mutuals}&with_mutual_friends={with_mutuals}&with_mutual_friends_count=false"
				);
				if let Some(guild) = guild {
					path.push_str(&format!("&guild_id={guild}"));
				}
				let result = async {
					let bytes = self
						.request_limited(Method::GET, &path, None, profile::MAX_PROFILE_WIRE)
						.await?;
					let mut profile = profile::decode_profile(&bytes, guild, with_mutuals)
						.map_err(|_| Failure::Protocol)?;
					if profile.user.id != user {
						return Err(Failure::Protocol);
					}
					// Optional names/artwork must not hold the entire profile behind slow reads
					// or a shared service cooldown. Dropping these reads is safe: they never write.
					let effect = profile.effect_sku.filter(|_| cosmetics.effects);
					let frame = profile.frame_sku.filter(|_| cosmetics.frames);
					let (board, catalog, badges) = tokio::join!(
						tokio::time::timeout(
							PROFILE_EXTRAS,
							self.load_profile_board_games(&mut profile),
						),
						tokio::time::timeout(
							PROFILE_EXTRAS,
							self.load_profile_collectibles(effect, frame),
						),
						// Only the account's own profile is requested without mutuals.
						tokio::time::timeout(
							Duration::from_secs(2),
							self.load_badge_catalog(user, !with_mutuals)
						),
					);
					if let Ok(result) = board {
						result?;
					}
					if let Ok(result) = badges {
						profile.badge_catalog = result?.map(Box::new);
						if !profile.valid() {
							profile.badge_catalog = None;
						}
					}
					if let Ok(result) = catalog {
						let (effect, frame) = result?;
						profile.effect = effect.map(Box::new);
						profile.frame = frame.map(Box::new);
						// Artwork that would exceed the profile bound is dropped, not the profile.
						if !profile.valid() {
							profile.effect = None;
							profile.frame = None;
						}
					}
					Ok(Box::new(profile))
				}
				.await;
				Event::Profile {
					user,
					guild,
					request,
					result,
				}
			}
			Command::BadgeSettings { request, settings } => Event::BadgeSettings {
				request,
				result: self.save_badge_settings(settings).await,
			},
			Command::StreamPreview {
				guild,
				channel,
				user,
				request,
			} => {
				let key = format!("guild:{guild}:{channel}:{user}");
				let result = self
					.request_limited(Method::GET, &format!("/streams/{key}/preview"), None, 4096)
					.await
					.and_then(|bytes| stream_preview_url(&bytes, &key));
				Event::StreamPreview {
					guild,
					channel,
					user,
					request,
					result,
				}
			}
			Command::EditProfile {
				user,
				request,
				changes,
			} => Event::ProfileEdited {
				user,
				request,
				result: self.edit_profile(user, changes).await,
			},
			Command::CancelProfile => Event::Failure(Failure::Protocol),
			Command::MemberSearch(_) | Command::Voice(_) | Command::Members { .. } => {
				Event::Failure(Failure::Protocol)
			}
			Command::History {
				channel,
				before,
				after,
				request,
			} => self.history(channel, before, after, request).await,
			// The same request a click sends, but it gives way: without spare capacity or during
			// a cooldown it fails with Capacity at once, and a click that adopted it sends its own.
			Command::Prefetch(command) => {
				Box::pin(SPECULATIVE.scope((), self.execute(*command))).await
			}
			Command::StickerPacks => Event::StickerPacks(
				self.request_limited(
					Method::GET,
					"/sticker-packs",
					None,
					client_core::stickers::MAX_PACK_BYTES,
				)
				.await
				.and_then(|b| {
					discord_protocol::stickers::sticker_packs(&b).map_err(|_| Failure::Protocol)
				}),
			),
			Command::Sticker(id) => Event::Sticker {
				id,
				result: if id.0 == 0 {
					Err(Failure::Protocol)
				} else {
					self.request_limited(Method::GET, &format!("/stickers/{id}"), None, 16 * 1024)
						.await
						.and_then(|b| {
							discord_protocol::stickers::sticker(&b).map_err(|_| Failure::Protocol)
						})
				},
			},
			Command::Forward {
				source,
				message,
				guild,
				channel,
				nonce,
			} => {
				let result = if source.0 == 0
					|| message.0 == 0
					|| channel.0 == 0
					|| guild.is_some_and(|id| id.0 == 0)
				{
					Err(Failure::Protocol)
				} else {
					let mut reference =
						serde_json::json!({"type":1,"channel_id":source,"message_id":message});
					if let Some(guild) = guild {
						reference["guild_id"] = serde_json::json!(guild);
					}
					self.request(Method::POST, &format!("/channels/{channel}/messages"), Some(serde_json::json!({
						"message_reference":reference,"nonce":nonce,"allowed_mentions":{"parse":[],"replied_user":false}
					}))).await.and_then(|bytes| {
						let message = decode::<MessageDto>(&bytes).map_err(|_| Failure::Ambiguous)?;
						if message.channel_id != channel { return Err(Failure::Ambiguous); }
						Ok(message.into_model())
					})
				};
				Event::SendResult { nonce, result }
			}
			Command::Send {
				sticker,
				channel,
				content,
				nonce,
				reply,
			} => {
				self.send_text(channel, &content, nonce, reply, sticker, None)
					.await
			}
			Command::VerifiedSend {
				sticker,
				channel,
				content,
				nonce,
				reply,
				captcha,
			} => {
				let target = client_core::captcha::Target::Message {
					channel,
					nonce: nonce.clone(),
				};
				if captcha.matches_target(&target) {
					self.send_text(channel, &content, nonce, reply, sticker, Some(&captcha))
						.await
				} else {
					Event::SendResult {
						nonce,
						result: Err(Failure::ProtocolAt(
							"Verification expired; retry the message",
						)),
					}
				}
			}
			Command::Edit {
				request,
				channel,
				message,
				content,
			} => {
				if content.trim().is_empty()
					|| content.chars().count() > client_core::MAX_PREMIUM_CONTENT
				{
					return Event::Edited {
						request,
						channel,
						message,
						result: Err(Failure::Capacity),
					};
				}
				let body = serde_json::json!({"content": content, "allowed_mentions": allowed_mentions(&content, None)});
				let result = self
					.request(
						Method::PATCH,
						&format!("/channels/{channel}/messages/{message}"),
						Some(body),
					)
					.await
					.and_then(|bytes| {
						decode::<MessageDto>(&bytes)
							.map(MessageDto::into_model)
							.map_err(|_| Failure::Ambiguous)
					});
				Event::Edited {
					request,
					channel,
					message,
					result,
				}
			}
			Command::Delete { channel, message } => {
				match self
					.request(
						Method::DELETE,
						&format!("/channels/{channel}/messages/{message}"),
						None,
					)
					.await
				{
					Ok(_) => Event::Delete {
						channel,
						id: message,
					},
					Err(Failure::Forbidden) => Event::Unavailable(channel),
					Err(f) => Event::Failure(f),
				}
			}
			Command::Pin {
				request,
				channel,
				message,
				pinned,
			} => {
				// Documented message pin routes; a failed pin never affects channel access.
				let result = self
					.request(
						if pinned { Method::PUT } else { Method::DELETE },
						&format!("/channels/{channel}/messages/pins/{message}"),
						None,
					)
					.await
					.map(|_| ());
				Event::Pinned {
					request,
					channel,
					message,
					pinned,
					result,
				}
			}
		}
	}
}
impl DiscordApi {
	async fn mark_read(
		&self,
		channel: model::Id,
		message: model::Id,
		manual: bool,
		mention_count: Option<u32>,
	) -> Result<(), Failure> {
		#[derive(serde::Deserialize)]
		struct Reply {
			#[serde(default)]
			token: Option<String>,
		}
		// Legacy acknowledgement tokens are session-only, redacted by ownership, and never cached.
		// Manual mark-unread omits the token, matching the unofficial normal-user ack body.
		let mut token = self.ack_token.lock().await;
		let body = if manual {
			let mut body = serde_json::json!({"manual": true});
			if let Some(count) = mention_count {
				body["mention_count"] = count.into();
			}
			body
		} else {
			serde_json::json!({"token":token.as_deref(),"manual":false})
		};
		let bytes = zeroize::Zeroizing::new(
			self.request_limited(
				Method::POST,
				&format!("/channels/{channel}/messages/{message}/ack"),
				Some(body),
				4096,
			)
			.await?,
		);
		let next = zeroize::Zeroizing::new(if bytes.is_empty() {
			None
		} else {
			decode::<Reply>(&bytes)
				.map_err(|_| Failure::Ambiguous)?
				.token
		});
		if next
			.as_ref()
			.is_some_and(|t| t.len() > 2048 || t.chars().any(char::is_control))
		{
			return Err(Failure::Ambiguous);
		}
		*token = next;
		Ok(())
	}
	async fn mark_guild_read(&self, guild: model::Id) -> Result<(), Failure> {
		self.request_limited(Method::POST, &format!("/guilds/{guild}/ack"), None, 4096)
			.await
			.map(|_| ())
	}
	/// Unofficial normal-user bulk acknowledgement in 100-channel requests (unverified limit).
	/// Returns how many leading channels were confirmed, even when a later request fails.
	async fn mark_all_read(
		&self,
		channels: &[(model::Id, model::Id)],
	) -> (usize, Result<(), Failure>) {
		if channels.len() > client_core::MAX_BULK_ACK {
			return (0, Err(Failure::Protocol));
		}
		let mut acked = 0;
		for chunk in channels.chunks(100) {
			let read_states = chunk
				.iter()
				.map(|(channel, message)| {
					serde_json::json!({
						"channel_id": channel.to_string(),
						"message_id": message.to_string(),
						"read_state_type": 0,
					})
				})
				.collect::<Vec<_>>();
			if let Err(failure) = self
				.request_limited(
					Method::POST,
					"/read-states/ack-bulk",
					Some(serde_json::json!({ "read_states": read_states })),
					4096,
				)
				.await
			{
				return (acked, Err(failure));
			}
			acked += chunk.len();
		}
		(acked, Ok(()))
	}
}
impl DiscordApi {
	async fn pins(
		&self,
		channel: model::Id,
		before: Option<i128>,
	) -> Result<client_core::search::Outcome, Failure> {
		let mut path = format!("/channels/{channel}/messages/pins?limit=25");
		if let Some(cursor) = before {
			let timestamp = pins::format_cursor(cursor).map_err(|_| Failure::Protocol)?;
			let encoded: String = timestamp
				.bytes()
				.map(|byte| format!("%{byte:02X}"))
				.collect();
			path.push_str(&format!("&before={encoded}"));
		}
		let bytes = self
			.request_limited(Method::GET, &path, None, search::MAX_WIRE)
			.await?;
		decode::<discord_protocol::pins::Reply>(&bytes)
			.map_err(|_| Failure::Protocol)?
			.into_page(channel, before)
			.map(client_core::search::Outcome::Pins)
			.map_err(|_| Failure::Protocol)
	}
	async fn search(
		&self,
		channel: model::Id,
		guild: Option<model::Id>,
		query: &str,
		before: Option<model::Id>,
		offset: u32,
		oldest: bool,
	) -> Result<client_core::search::Outcome, Failure> {
		if !model::valid_search_query(query) || offset > model::MAX_SEARCH_OFFSET {
			return Err(Failure::Protocol);
		}
		let (content, filters) = model::search_terms(query).map_err(|_| Failure::Protocol)?;
		// Encode values separately; user input cannot add arbitrary query parameters.
		let encoded: String = content.bytes().map(|b| format!("%{b:02X}")).collect();
		// Server searches span every readable channel unless `in:` narrows them.
		let mut path = match guild {
			Some(guild) => format!("/guilds/{guild}/messages/search?"),
			None => format!("/channels/{channel}/messages/search?"),
		};
		// Discord sorts the whole result set; ascending puts the earliest match on page one.
		path.push_str(&format!(
			"content={encoded}&limit=25&sort_by=timestamp&sort_order={}",
			if oldest { "asc" } else { "desc" }
		));
		// The cursor continues past the previous page's last hit: older for newest-first,
		// newer for oldest-first.
		let (mut maximum, mut minimum) = if oldest {
			(None, before.map(|id| id.0))
		} else {
			(before.map(|id| id.0), None)
		};
		for (key, value) in filters {
			if key == "channel_id" && guild.is_none() {
				return Err(Failure::Protocol);
			}
			if key == "max_id" {
				let id = value.parse::<u64>().map_err(|_| Failure::Protocol)?;
				maximum = Some(maximum.map_or(id, |current| current.min(id)));
			} else if key == "min_id" {
				let id = value.parse::<u64>().map_err(|_| Failure::Protocol)?;
				minimum = Some(minimum.map_or(id, |current| current.max(id)));
			} else {
				let encoded: String = value.bytes().map(|b| format!("%{b:02X}")).collect();
				path.push_str(&format!("&{key}={encoded}"));
			}
		}
		if let Some(before) = maximum {
			path.push_str(&format!("&max_id={before}"));
		}
		if let Some(after) = minimum {
			path.push_str(&format!("&min_id={after}"));
		}
		if offset != 0 {
			path.push_str(&format!("&offset={offset}"));
		}
		let bytes = self
			.request_limited(Method::GET, &path, None, search::MAX_WIRE)
			.await?;
		let reply = decode::<search::Reply>(&bytes).map_err(|_| Failure::Protocol)?;
		if reply
			.code
			.is_some_and(|code| (110000..119999).contains(&code))
		{
			let mut next = self.cooldown.lock().await;
			*next = (*next).max(Instant::now() + safe_delay(reply.retry_after)?);
			return Ok(client_core::search::Outcome::Indexing);
		}
		reply
			.into_page(
				guild.is_none().then_some(channel),
				if oldest { minimum } else { maximum }.map(model::Id),
				oldest,
			)
			.map(client_core::search::Outcome::Page)
			.map_err(|_| Failure::Protocol)
	}
	/// Unofficial normal-client relay of KLIPY search/trending. Only the query text is encoded
	/// into a fixed route; previews stay static and are loaded by the credential-free worker.
	/// Trending needs two routes: `/gifs/trending` returns the categories with a single sample
	/// GIF, while the Trending GIFs grid is `/gifs/trending-gifs`.
	async fn gifs(&self, query: Option<&str>) -> Result<model::GifPage, Failure> {
		const OPTIONS: &str = "media_format=tinygif&provider=klipy&locale=en-US";
		match query {
			Some(query) => {
				if !model::valid_search_query(query) {
					return Err(Failure::Protocol);
				}
				let encoded: String = query.bytes().map(|b| format!("%{b:02X}")).collect();
				let bytes = self
					.request_limited(
						Method::GET,
						&format!(
							"/gifs/search?q={encoded}&limit={}&{OPTIONS}",
							model::GIF_PAGE_SIZE
						),
						None,
						gifs::MAX_WIRE,
					)
					.await?;
				decode::<gifs::SearchReply>(&bytes)
					.map_err(|_| Failure::Protocol)?
					.into_page()
					.map_err(|_| Failure::Protocol)
			}
			None => {
				let categories_path = format!("/gifs/trending?{OPTIONS}");
				let grid_path = format!(
					"/gifs/trending-gifs?limit={}&{OPTIONS}",
					model::GIF_PAGE_SIZE
				);
				let (categories, grid) = tokio::join!(
					self.request_limited(Method::GET, &categories_path, None, gifs::MAX_WIRE),
					self.request_limited(Method::GET, &grid_path, None, gifs::MAX_WIRE),
				);
				let categories =
					decode::<gifs::TrendingReply>(&categories?).map_err(|_| Failure::Protocol)?;
				// A failed grid keeps the categories and their sample GIF usable.
				match grid
					.ok()
					.and_then(|bytes| decode::<gifs::SearchReply>(&bytes).ok())
				{
					Some(grid) => categories.into_page_with(grid),
					None => categories.into_page(),
				}
				.map_err(|_| Failure::Protocol)
			}
		}
	}
	/// One text send that reports a service captcha instead of a terminal rejection.
	async fn send_text(
		&self,
		channel: model::Id,
		content: &str,
		nonce: String,
		reply: Option<Reply>,
		sticker: Option<model::Id>,
		retry: Option<&client_core::captcha::Retry>,
	) -> Event {
		let mut challenge = None;
		let result = self
			.send_message_with_captcha(
				channel,
				content,
				&nonce,
				(reply, None, sticker),
				retry,
				Some(&mut challenge),
			)
			.await;
		match challenge {
			Some(challenge) => Event::SendChallenge {
				nonce,
				reply,
				challenge: Box::new(challenge),
			},
			None => Event::SendResult { nonce, result },
		}
	}
	async fn send_message(
		&self,
		channel: model::Id,
		content: &str,
		nonce: &str,
		reply: Option<Reply>,
		attachment: Option<Vec<serde_json::Value>>,
		sticker: Option<model::Id>,
	) -> Result<model::Message, Failure> {
		self.send_message_with_captcha(
			channel,
			content,
			nonce,
			(reply, attachment, sticker),
			None,
			None,
		)
		.await
	}
	async fn send_message_with_captcha(
		&self,
		channel: model::Id,
		content: &str,
		nonce: &str,
		(reply, attachment, sticker): (
			Option<Reply>,
			Option<Vec<serde_json::Value>>,
			Option<model::Id>,
		),
		retry: Option<&client_core::captcha::Retry>,
		challenge: Option<&mut Option<client_core::captcha::Challenge>>,
	) -> Result<model::Message, Failure> {
		if !message_options::valid(
			content,
			client_core::MAX_PREMIUM_CONTENT,
			attachment.as_ref().is_some_and(|items| !items.is_empty()) || sticker.is_some(),
		) {
			return Err(Failure::Capacity);
		}
		let (content, silent) = message_options::content(content);
		if sticker.is_some_and(|id| id.0 == 0) {
			return Err(Failure::Protocol);
		}
		let mut body = serde_json::json!({"content":content,"nonce":nonce,"allowed_mentions":allowed_mentions(content, reply)});
		if silent {
			body["flags"] = serde_json::json!(message_options::SUPPRESS_NOTIFICATIONS);
		}
		if let Some(sticker) = sticker {
			body["sticker_ids"] = serde_json::json!([sticker]);
		}
		if let Some(reply) = reply {
			body["message_reference"] =
				serde_json::json!({"message_id":reply.target(),"channel_id":channel});
		}
		if let Some(attachment) = attachment {
			body["attachments"] = serde_json::json!(attachment);
		}
		// No enforce_nonce claim until normal-user semantics are live verified. Never auto-retry writes.
		self.request_with_captcha(
			Method::POST,
			&format!("/channels/{channel}/messages"),
			Some(body),
			MAX_WIRE,
			retry,
			challenge,
		)
		.await
		.and_then(|bytes| {
			let message = decode::<MessageDto>(&bytes).map_err(|_| Failure::Ambiguous)?;
			if message.channel_id != channel {
				return Err(Failure::Ambiguous);
			}
			Ok(message.into_model())
		})
	}
}
impl DiscordApi {
	/// Documented forum post creation: one thread with its starter message. Never auto-retried.
	pub(crate) async fn create_post(
		&self,
		parent: model::Id,
		guild: model::Id,
		(title, tags): (&str, &[model::Id]),
		content: &str,
		attachments: Option<Vec<serde_json::Value>>,
	) -> Result<model::Channel, Failure> {
		let title = title.trim();
		if title.is_empty()
			|| title.chars().count() > client_core::forum::MAX_TITLE
			|| tags.len() > model::forum::MAX_APPLIED_TAGS
			|| !message_options::valid(
				content,
				client_core::MAX_PREMIUM_CONTENT,
				attachments.as_ref().is_some_and(|items| !items.is_empty()),
			) {
			return Err(Failure::Capacity);
		}
		let (content, silent) = message_options::content(content);
		let mut message = serde_json::json!({
			"content": content,
			"allowed_mentions": allowed_mentions(content, None),
		});
		if silent {
			message["flags"] = serde_json::json!(message_options::SUPPRESS_NOTIFICATIONS);
		}
		if let Some(attachments) = attachments {
			message["attachments"] = serde_json::json!(attachments);
		}
		let mut body = serde_json::json!({
			"name": title,
			"auto_archive_duration": 4320,
			"message": message,
		});
		if !tags.is_empty() {
			body["applied_tags"] = serde_json::json!(tags);
		}
		self.request(
			Method::POST,
			&format!("/channels/{parent}/threads"),
			Some(body),
		)
		.await
		.and_then(|bytes| {
			let post = decode::<ChannelDto>(&bytes).map_err(|_| Failure::Ambiguous)?;
			if post.parent_id != Some(parent) || post.guild_id.is_some_and(|id| id != guild) {
				return Err(Failure::Ambiguous);
			}
			let mut post = post.into_model();
			post.guild = Some(guild);
			post.name = post.name.chars().take(128).collect();
			Ok(post)
		})
	}
}
impl AuthProvider for DiscordApi {
	async fn authenticate(&mut self) -> Result<User, Failure> {
		self.current_user().await
	}
}
fn reaction_path(
	channel: model::Id,
	message: model::Id,
	emoji: &model::ReactionEmoji,
) -> Option<String> {
	if !emoji.valid() {
		return None;
	}
	let name = emoji.name.as_ref()?;
	let value = emoji
		.id
		.map_or_else(|| name.clone(), |id| format!("{name}:{id}"));
	// Encode the complete emoji as one path component, including custom-name separators.
	let encoded: String = value.bytes().map(|byte| format!("%{byte:02X}")).collect();
	Some(format!(
		"/channels/{channel}/messages/{message}/reactions/{encoded}/@me"
	))
}
fn reaction_users_path(
	channel: model::Id,
	message: model::Id,
	emoji: &model::ReactionEmoji,
	after: Option<model::Id>,
) -> Option<String> {
	let mut path = reaction_path(channel, message, emoji)?;
	path.truncate(path.len() - "/@me".len());
	path.push_str("?limit=100");
	if let Some(after) = after {
		path.push_str(&format!("&after={after}"));
	}
	Some(path)
}
fn safe_delay(seconds: Option<f64>) -> Result<Duration, Failure> {
	let seconds = seconds.unwrap_or(1.0);
	if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
		return Err(Failure::Protocol);
	}
	Ok(Duration::from_secs_f64(seconds.max(0.05)))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn stream_preview_accepts_only_the_requested_discord_cdn_path() {
		let key = "guild:1:2:3";
		let good =
			br#"{"url":"https://cdn.discordapp.com/streams/guild:1:2:3/0123456789abcdef.png"}"#;
		assert!(stream_preview_url(good, key).is_ok());
		for bad in [
			br#"{"url":"https://evil.example/streams/guild:1:2:3/0123456789abcdef.png"}"#
				.as_slice(),
			br#"{"url":"https://cdn.discordapp.com/streams/guild:1:2:4/0123456789abcdef.png"}"#,
			br#"{"url":"https://cdn.discordapp.com/streams/guild:1:2:3/../token.png"}"#,
		] {
			assert_eq!(stream_preview_url(bad, key), Err(Failure::Protocol));
		}
	}

	#[tokio::test]
	async fn preloads_give_way_to_clicks_instead_of_waiting() {
		let api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_PRELOAD_TOKEN".into()).unwrap(),
		))
		.unwrap();
		let preload = || {
			Command::Prefetch(Box::new(Command::History {
				channel: model::Id(21),
				before: None,
				after: None,
				request: 1 << 63,
			}))
		};
		let gave_way = |event: Event| {
			matches!(
				event,
				Event::Failure(Failure::Capacity)
					| Event::HistoryFailed {
						failure: Failure::Capacity,
						..
					}
			)
		};
		// During a service cooldown a preload fails at once instead of waiting it out.
		*api.cooldown.lock().await = Instant::now() + Duration::from_secs(30);
		let event = tokio::time::timeout(Duration::from_secs(1), api.execute(preload()))
			.await
			.expect("a preload never waits for the cooldown");
		assert!(gave_way(event));
		// Without spare permits it leaves the remaining ones to requests the user waits for.
		*api.cooldown.lock().await = Instant::now();
		let _held = api.requests.acquire_many(2).await.unwrap();
		let event = tokio::time::timeout(Duration::from_secs(1), api.execute(preload()))
			.await
			.expect("a preload never waits for a permit");
		assert!(gave_way(event));
		assert_eq!(api.requests.available_permits(), FOREGROUND_PERMITS);
	}
	#[tokio::test]
	async fn guild_creation_posts_once_and_waits_for_gateway_state() {
		use tokio::{
			io::{AsyncReadExt, AsyncWriteExt},
			net::TcpListener,
		};
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_GUILD_CREATE_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			let (mut stream, _) = listener.accept().await.unwrap();
			let mut bytes = Vec::new();
			loop {
				let mut chunk = [0; 1024];
				let count = stream.read(&mut chunk).await.unwrap();
				assert!(count > 0);
				bytes.extend_from_slice(&chunk[..count]);
				let Some(headers_end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
					continue;
				};
				let headers = std::str::from_utf8(&bytes[..headers_end]).unwrap();
				let length = headers
					.lines()
					.find_map(|line| {
						line.to_ascii_lowercase()
							.strip_prefix("content-length: ")
							.and_then(|value| value.parse::<usize>().ok())
					})
					.unwrap();
				if bytes.len() >= headers_end + 4 + length {
					assert!(headers.starts_with("POST /guilds HTTP/1.1"));
					let body: serde_json::Value =
						serde_json::from_slice(&bytes[headers_end + 4..headers_end + 4 + length])
							.unwrap();
					assert_eq!(
						body,
						serde_json::json!({
							"name": "Synthetic server",
							"icon": null,
							"system_channel_id": null,
							"channels": [],
							"guild_template_code": "2TffvPucqHkN",
						})
					);
					break;
				}
			}
			let body = r#"{"id":"42","name":"Synthetic server"}"#;
			stream
				.write_all(
					format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
						body.len()
					)
					.as_bytes(),
				)
				.await
				.unwrap();
		});
		let mut state = client_core::State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			..Default::default()
		};
		let command = state
			.create_guild("  Synthetic server  ".into(), None)
			.unwrap();
		let event = api.execute(command).await;
		assert!(matches!(
			event,
			Event::GuildCreated {
				result: Ok(model::Id(42)),
				..
			}
		));
		state.apply(client_core::Envelope {
			generation: state.generation,
			event,
		});
		assert_eq!(state.guild_creation.result, Some(Ok(model::Id(42))));
		assert!(state.guild(model::Id(42)).is_none());
		server.await.unwrap();
	}
	#[tokio::test]
	async fn invite_captcha_preserves_fatal_auth_and_malformed_challenges() {
		{
			assert_eq!(invite_captcha(br#"{"captcha_service":"hcaptcha","captcha_sitekey":"synthetic-sitekey","captcha_rqdata":"escaped\/data"}"#).unwrap().rqdata(), Some("escaped/data"));
			for (status, code, service) in [
				("403 Forbidden", 0, "hcaptcha"),
				("400 Bad Request", 60003, "hcaptcha"),
				("400 Bad Request", 50014, "hcaptcha"),
				("400 Bad Request", 0, "unsupported"),
			] {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_INVITE_OWNER_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let server = tokio::spawn(async move {
					let (mut stream, _) = listener.accept().await.unwrap();
					let mut buffer = [0; 4096];
					assert!(stream.read(&mut buffer).await.unwrap() > 0);
					let body = serde_json::json!({"code":code,"captcha_key":["required"],"captcha_service":service,"captcha_sitekey":"synthetic-sitekey"}).to_string();
					stream
						.write_all(
							format!(
								"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
								body.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
				});
				let event = api
					.execute(Command::JoinInvite {
						code: "synthetic".into(),
						request: 1,
						captcha: None,
					})
					.await;
				if matches!(code, 60003 | 50014) {
					assert!(matches!(
						event,
						Event::JoinInvite {
							result: Err(Failure::Challenged),
							..
						}
					));
					assert!(api.stopped());
				} else {
					if service == "hcaptcha" {
						assert!(matches!(event, Event::InviteChallenge { .. }));
					} else {
						assert!(matches!(
							event,
							Event::JoinInvite {
								result: Err(Failure::ProtocolAt(_)),
								..
							}
						));
					}
					assert!(!api.stopped());
				}
				server.await.unwrap();
			}
			assert!(
				invite_captcha(
					br#"{"captcha_service":"hcaptcha","captcha_sitekey":"bad/sitekey"}"#
				)
				.is_none()
			);
		}
		{
			let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_INVITE_OWNER_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				for attempt in 0..3 {
					let (mut stream, _) = listener.accept().await.unwrap();
					let mut bytes = Vec::new();
					while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
						let mut buf = [0; 1024];
						let n = stream.read(&mut buf).await.unwrap();
						assert!(n > 0);
						bytes.extend_from_slice(&buf[..n]);
						assert!(bytes.len() < 16384);
					}
					let request = std::str::from_utf8(&bytes).unwrap();
					assert!(request.starts_with("POST /invites/synthetic HTTP/1.1"));
					assert_eq!(
						request.contains("x-captcha-key: synthetic-solution"),
						attempt == 1
					);
					assert_eq!(
						request.contains("x-captcha-rqtoken: synthetic-rqtoken"),
						attempt == 1
					);
					assert_eq!(
						request.contains("x-captcha-session-id: synthetic-session"),
						attempt == 1
					);
					let (status, body) = match attempt {
						0 => (
							"400 Bad Request",
							r#"{"captcha_key":["required"],"captcha_service":"hcaptcha","captcha_sitekey":"synthetic-sitekey","captcha_rqdata":"synthetic-rqdata","captcha_rqtoken":"synthetic-rqtoken","captcha_session_id":"synthetic-session"}"#,
						),
						1 => ("200 OK", r#"{"guild":{"id":"2","name":"Synthetic"}}"#),
						_ => (
							"400 Bad Request",
							r#"{"captcha_key":["required"],"captcha_service":"unsupported"}"#,
						),
					};
					stream
						.write_all(
							format!(
								"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
								body.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
				}
			});
			let mut state = client_core::State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				..Default::default()
			};
			state.invites.insert(
				"synthetic".into(),
				(
					std::time::Instant::now(),
					Some(Ok(model::InvitePreview {
						guild: model::Id(2),
						embed: Default::default(),
					})),
				),
			);
			let event = api
				.execute(state.join_invite("synthetic".into()).unwrap())
				.await;
			assert!(matches!(event, Event::InviteChallenge { .. }));
			assert!(!api.stopped());
			state.apply(client_core::Envelope {
				generation: state.generation,
				event,
			});
			let request = state.invite_challenge().unwrap().0;
			let command = state
				.resume_invite_challenge(
					request,
					client_core::captcha::Solution::new("synthetic-solution".into()).unwrap(),
				)
				.unwrap();
			assert!(matches!(
				api.execute(command).await,
				Event::JoinInvite {
					result: Ok(model::Id(2)),
					..
				}
			));
			assert!(!api.stopped());
			assert!(matches!(
				api.execute(Command::JoinInvite {
					code: "synthetic".into(),
					request: 99,
					captcha: None
				})
				.await,
				Event::JoinInvite {
					result: Err(Failure::ProtocolAt(_)),
					..
				}
			));
			assert!(!api.stopped());
			server.await.unwrap();
		}
	}
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};
	/// Trending merges both unofficial routes; synthetic replies only, no live service.
	#[tokio::test]
	async fn trending_gifs_load_categories_and_the_full_grid() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_GIF_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let gif = |id: &str| {
				format!(
					r#"{{"id":"{id}","title":"Synthetic","url":"https://klipy.com/gifs/synthetic-{id}","gif_src":"https://static.klipy.com/s/{id}.webp","width":320,"height":240,"preview":"https://static.klipy.com/s/{id}.gif"}}"#
				)
			};
			let categories = format!(
				r#"{{"categories":[{{"name":"happy","src":"https://static.klipy.com/c/happy.gif"}}],"gifs":[{}]}}"#,
				gif("sample")
			);
			let grid = format!("[{},{},{}]", gif("t1"), gif("t2"), gif("t3"));
			let server = tokio::spawn(async move {
				let mut paths = Vec::new();
				for _ in 0..2 {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = Vec::new();
					while !request.windows(4).any(|w| w == b"\r\n\r\n") {
						let mut bytes = [0; 1024];
						let n = socket.read(&mut bytes).await.unwrap();
						assert!(n > 0 && request.len() < 4096);
						request.extend_from_slice(&bytes[..n]);
					}
					let request = String::from_utf8(request).unwrap();
					let path = request.split(' ').nth(1).unwrap().to_owned();
					assert!(
						path.contains("provider=klipy")
							&& path.contains("locale=en-US")
							&& path.contains("media_format=tinygif"),
						"{path}"
					);
					let body = if path.starts_with("/gifs/trending-gifs?") {
						&grid
					} else {
						assert!(path.starts_with("/gifs/trending?"), "{path}");
						&categories
					};
					socket
						.write_all(
							format!(
								"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
								body.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
					paths.push(path);
				}
				paths
			});
			let page = api.gifs(None).await.unwrap();
			assert_eq!(
				page.gifs
					.iter()
					.map(|gif| gif.id.as_str())
					.collect::<Vec<_>>(),
				["t1", "t2", "t3"]
			);
			assert_eq!(page.categories.len(), 1);
			let mut paths = server.await.unwrap();
			paths.sort();
			assert!(paths[0].starts_with("/gifs/trending-gifs?limit="));
			assert!(paths[1].starts_with("/gifs/trending?"));
		})
		.await
		.unwrap();
	}
	#[tokio::test]
	async fn single_message_delete_confirms_only_success_and_never_retries_ambiguity() {
		{
			use model::Id;
			tokio::time::timeout(Duration::from_secs(10), async {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_DELETE_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let server = tokio::spawn(async move {
					for status in [
						"204 No Content",
						"403 Forbidden",
						"500 Internal Server Error",
					] {
						let (mut socket, _) = listener.accept().await.unwrap();
						let mut request = Vec::new();
						loop {
							let mut bytes = [0; 1024];
							let n = socket.read(&mut bytes).await.unwrap();
							assert!(n > 0);
							request.extend_from_slice(&bytes[..n]);
							assert!(request.len() < 4096);
							if request.windows(4).any(|w| w == b"\r\n\r\n") {
								break;
							}
						}
						let request = std::str::from_utf8(&request).unwrap();
						assert!(
							request.starts_with("DELETE /channels/20/messages/100 HTTP/1.1\r\n")
						);
						assert!(request.contains("SYNTHETIC_DELETE_TOKEN"));
						socket
							.write_all(
								format!(
									"HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
								)
								.as_bytes(),
							)
							.await
							.unwrap();
					}
				});
				let command = || Command::Delete {
					channel: Id(20),
					message: Id(100),
				};
				assert!(matches!(
					api.execute(command()).await,
					Event::Delete {
						channel: Id(20),
						id: Id(100)
					}
				));
				assert!(matches!(
					api.execute(command()).await,
					Event::Unavailable(Id(20))
				));
				assert!(matches!(
					api.execute(command()).await,
					Event::Failure(Failure::Ambiguous)
				));
				server.await.unwrap();
			})
			.await
			.unwrap();
		}
		{
			tokio::time::timeout(Duration::from_secs(5), async {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_SEND_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let server = tokio::spawn(async move {
					for channel in [2, 9] {
						let (mut socket, _) = listener.accept().await.unwrap();
						let mut request = Vec::new();
						loop {
							let mut bytes = [0; 1024];
							let count = socket.read(&mut bytes).await.unwrap();
							assert!(count > 0);
							request.extend_from_slice(&bytes[..count]);
							assert!(request.len() < 4096);
							if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
								break;
							}
						}
						assert!(request.starts_with(b"POST /channels/2/messages HTTP/1.1\r\n"));
						let body = serde_json::json!({
							"id":"100", "channel_id":channel.to_string(),
							"author":{"id":"1","username":"Synthetic"},
							"type":19, "content":"Synthetic reply", "nonce":"local",
							"message_reference":{"type":0,"channel_id":channel.to_string(),"message_id":"50"},
							"referenced_message":null,
						})
						.to_string();
						socket
							.write_all(
								format!(
									"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
									body.len(),
								)
								.as_bytes(),
							)
							.await
							.unwrap();
					}
				});
				for accepted in [true, false] {
					let Event::SendResult { nonce, result } = api
						.execute(Command::Send {
							sticker: None,
							channel: model::Id(2),
							content: "Synthetic reply".into(),
							nonce: "local".into(),
							reply: Some(Reply::to(model::Id(50))),
						})
						.await
					else {
						panic!("send response");
					};
					assert_eq!(nonce, "local");
					if accepted {
						let message = result.unwrap();
						assert_eq!(message.channel, model::Id(2));
						assert!(message.reply_deleted);
					} else {
						assert!(matches!(result, Err(Failure::Ambiguous)));
					}
				}
				server.await.unwrap();
				assert!(!api.stopped());
			})
			.await
			.unwrap();
		}
	}
	#[tokio::test]
	async fn history_after_includes_zero_and_rejects_combined_cursors() {
		{
			use model::Id;
			tokio::time::timeout(Duration::from_secs(10), async {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_HISTORY_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let server = tokio::spawn(async move {
					for (cursor, ids) in [
						("after=0", vec![2, 1]),
						("after=9", vec![11, 10]),
						("before=9", vec![8, 7]),
						("after=99", (100..151).collect()),
					] {
						let (mut socket, _) = listener.accept().await.unwrap();
						let mut request = Vec::new();
						loop {
							let mut buffer = [0; 1024];
							let n = socket.read(&mut buffer).await.unwrap();
							assert!(n > 0);
							request.extend_from_slice(&buffer[..n]);
							assert!(request.len() < 4096);
							if request.windows(4).any(|w| w == b"\r\n\r\n") {
								break;
							}
						}
						assert!(std::str::from_utf8(&request).unwrap().starts_with(&format!(
							"GET /channels/1/messages?limit=50&{cursor} HTTP/1.1\r\n"
						),));
						let body = serde_json::to_string(
							&ids.into_iter()
								.map(|id| {
									serde_json::json!({
										"id": id.to_string(), "channel_id": "1", "author": {"id":"3","username":"Synthetic"},
										"content":"Synthetic history",
									})
								})
								.collect::<Vec<_>>(),
						)
						.unwrap();
						socket
							.write_all(
								format!(
									"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
									body.len()
								)
								.as_bytes(),
							)
							.await
							.unwrap();
					}
				});
				assert!(matches!(
					api.execute(Command::History {
						channel: Id(1),
						before: Some(Id(9)),
						after: Some(Id(0)),
						request: 1,
					})
					.await,
					Event::Failure(Failure::Protocol)
				));
				for (before, after, first) in [
					(None, Some(Id(0)), Id(2)),
					(None, Some(Id(9)), Id(11)),
					(Some(Id(9)), None, Id(8)),
				] {
					let Event::History {
						channel,
						request,
						older,
						messages,
					} = api.execute(Command::History {
						channel: Id(1),
						before,
						after,
						request: 7,
					})
					.await
					else {
						panic!("expected bounded history page");
					};
					assert_eq!((channel, request, older), (Id(1), 7, before.is_some()));
					assert_eq!(messages.len(), 2);
					assert_eq!(messages[0].id, first);
				}
				assert!(matches!(
					api.execute(Command::History {
						channel: Id(1),
						before: None,
						after: Some(Id(99)),
						request: 8,
					})
					.await,
					Event::Failure(Failure::Capacity)
				));
				server.await.unwrap();
			})
			.await
			.unwrap();
		}
		{
			use model::Id;
			tokio::time::timeout(Duration::from_secs(10), async {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_READ_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let server = tokio::spawn(async move {
					for (expected, body) in [
						(None, r#"{"token":"synthetic-ack"}"#),
						(Some("synthetic-ack"), r#"{"token":null}"#),
						(None, "invalid"),
					] {
						let (mut socket, _) = listener.accept().await.unwrap();
						let mut request = Vec::new();
						let payload = loop {
							let mut bytes = [0; 1024];
							let n = socket.read(&mut bytes).await.unwrap();
							assert!(n > 0);
							request.extend_from_slice(&bytes[..n]);
							assert!(request.len() < 4096);
							if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
								let header = std::str::from_utf8(&request[..end]).unwrap();
								assert!(
									header.starts_with(
										"POST /channels/1/messages/2/ack HTTP/1.1\r\n"
									)
								);
								let length: usize = header
									.lines()
									.find_map(|line| {
										line.to_ascii_lowercase()
											.strip_prefix("content-length: ")
											.map(str::to_owned)
									})
									.unwrap()
									.parse()
									.unwrap();
								if request.len() >= end + 4 + length {
									break serde_json::from_slice::<serde_json::Value>(
										&request[end + 4..end + 4 + length],
									)
									.unwrap();
								}
							}
						};
						assert_eq!(
							payload,
							serde_json::json!({"manual":false,"token":expected})
						);
						socket
							.write_all(
								format!(
									"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
									body.len()
								)
								.as_bytes(),
							)
							.await
							.unwrap();
					}
				});
				for (request, expected) in [(1, Ok(())), (2, Ok(())), (3, Err(Failure::Ambiguous))]
				{
					let Event::ReadState(client_core::read_state::Event::Result {
						channel,
						message,
						request: actual,
						result,
					}) = api.execute(Command::MarkRead {
						channel: Id(1),
						message: Id(2),
						request,
						manual: false,
						mention_count: None,
					})
					.await
					else {
						panic!()
					};
					assert_eq!((channel, message, actual), (Id(1), Id(2), request));
					assert_eq!(result, expected);
				}
				server.await.unwrap();
			})
			.await
			.unwrap();
		}
	}
	#[tokio::test]
	async fn search_routes_are_encoded_scoped_and_indexing_never_auto_retries() {
		use client_core::search::Outcome;
		use model::Id;
		tokio::time::timeout(Duration::from_secs(10),async {
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut api=DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_SEARCH_TOKEN".into()).unwrap())).unwrap();
            api.base=format!("http://{}",listener.local_addr().unwrap());
            let server=tokio::spawn(async move {
                for (route,status,body) in [
                    ("/guilds/2/messages/search?content=%78%26%23%3F%2E%2E&limit=25&sort_by=timestamp&sort_order=desc","200 OK",r#"{"messages":[[{"id":"9","channel_id":"1","author":{"id":"3","username":"Synthetic"},"content":"match"}]],"total_results":1}"#),
                    ("/channels/1/messages/search?content=%78&limit=25&sort_by=timestamp&sort_order=asc&min_id=9","200 OK",r#"{"messages":[[{"id":"10","channel_id":"1","author":{"id":"3","username":"Synthetic"},"content":"next"}]],"total_results":2}"#),
                    ("/guilds/2/messages/search?content=%78&limit=25&sort_by=timestamp&sort_order=desc&max_id=8&offset=25","200 OK",r#"{"messages":[],"total_results":50}"#),
                    ("/channels/1/messages/search?content=%78&limit=25&sort_by=timestamp&sort_order=desc&offset=9975","200 OK",r#"{"messages":[],"total_results":10000}"#),
                    ("/channels/1/messages/search?content=%78&limit=25&sort_by=timestamp&sort_order=desc&max_id=9&offset=25","200 OK",r#"{"messages":[[{"id":"9","channel_id":"1","author":{"id":"3","username":"Synthetic"},"content":"outside filter"}]],"total_results":50}"#),
                    ("/channels/1/messages/search?content=%78&limit=25&sort_by=timestamp&sort_order=desc","403 Forbidden",r#"{"code":50001}"#),
                    ("/channels/1/messages/pins?limit=25","200 OK",r#"{"items":[{"pinned_at":"2026-09-10T12:00:00Z","message":{"id":"9","channel_id":"1","author":{"id":"3","username":"Synthetic"},"content":"pin"}}],"has_more":true}"#),
                    ("/channels/1/messages/pins?limit=25&before=%32%30%32%36%2D%30%39%2D%31%30%54%31%32%3A%30%30%3A%30%30%5A","200 OK",r#"{"items":[{"pinned_at":"2026-09-10T11:00:00Z","message":{"id":"99","channel_id":"1","author":{"id":"3","username":"Synthetic"},"content":"older pin, newer message"}}],"has_more":false}"#),
                    ("/channels/1/messages/pins?limit=25&before=%32%30%32%36%2D%30%39%2D%31%30%54%31%32%3A%30%30%3A%30%30%5A","200 OK",r#"{"items":[{"pinned_at":"2026-09-10T12:00:00Z","message":{"id":"99","channel_id":"1","author":{"id":"3","username":"Synthetic"}}}],"has_more":true}"#),
                    ("/channels/1/messages/pins?limit=25","403 Forbidden",r#"{"code":50001}"#),
                    ("/channels/1/messages/search?content=%78&limit=25&sort_by=timestamp&sort_order=desc","202 Accepted",r#"{"code":110000,"retry_after":1}"#),
                ] {
                    let (mut socket,_)=listener.accept().await.unwrap();let mut request=Vec::new();
                    loop {let mut buffer=[0;1024];let n=socket.read(&mut buffer).await.unwrap();assert!(n>0);request.extend_from_slice(&buffer[..n]);assert!(request.len()<4096);if request.windows(4).any(|w|w==b"\r\n\r\n") {break;}}
                    let text=std::str::from_utf8(&request).unwrap();
                    assert!(text.starts_with(&format!("GET {route} HTTP/1.1\r\n")));
                    assert!(text.contains("SYNTHETIC_SEARCH_TOKEN"));
                    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                }
            });
            let Event::Search {channel:Id(1),request:8,result:Ok(Outcome::Page(page))}=api.execute(Command::Search {channel:Id(1),guild:Some(Id(2)),query:"x&#?..".into(),before:None,offset:0,oldest:false,request:8}).await else {panic!()};
            assert_eq!(page.hits[0].id,Id(9));
            // Oldest first, the cursor is a lower bound: the next page holds newer IDs.
            assert!(matches!(api.search(Id(1),None,"x",Some(Id(9)),0,true).await,Ok(Outcome::Page(p)) if p.hits.len()==1 && p.hits[0].id==Id(10)));
            assert!(matches!(api.search(Id(1),Some(Id(2)),"x before_id:8",Some(Id(9)),25,false).await,Ok(Outcome::Page(p)) if p.total == 50));
            assert!(matches!(api.search(Id(1),None,"x",None,9975,false).await,Ok(Outcome::Page(p)) if p.total == 10000));
            assert!(matches!(api.search(Id(1),None,"x before_id:9",None,25,false).await,Err(Failure::Protocol)));
            assert!(matches!(api.search(Id(1),None,"x",None,9976,false).await,Err(Failure::Protocol)));
            assert!(matches!(api.search(Id(1),None,"x",None,0,false).await,Err(Failure::Forbidden)));
            let Event::Search { channel:Id(1),request:9,result:Ok(Outcome::Pins(page)) } = api.execute(Command::Pins { channel:Id(1),before:None,request:9 }).await else {panic!()};
            assert!(page.hits[0].id == Id(9) && page.partial);
            let cursor = page.pin_cursor.unwrap();
            assert!(matches!(api.pins(Id(1),Some(cursor)).await,Ok(Outcome::Pins(p)) if p.hits[0].id == Id(99) && p.pin_cursor.is_none() && !p.partial));
            assert!(matches!(api.pins(Id(1),Some(cursor)).await,Err(Failure::Protocol)));
            assert!(matches!(api.pins(Id(1),Some(i128::MAX)).await,Err(Failure::Protocol)));
            assert!(matches!(api.pins(Id(1),None).await,Err(Failure::Forbidden)));
            assert!(matches!(api.search(Id(1),None,"x",None,0,false).await,Ok(Outcome::Indexing)));
            assert!(*api.cooldown.lock().await>Instant::now());
            server.await.unwrap();
        }).await.unwrap();
	}
	#[tokio::test]
	async fn reaction_routes_encode_one_component_and_read_back_scoped_counts() {
		{
			use client_core::reactions::{Command as R, Event as E};
			use model::{Id, ReactionEmoji};
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_REACTION_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				for (expected, body) in [
					(
						"PUT /channels/1/messages/2/reactions/%F0%9F%91%8D/@me",
						None,
					),
					(
						"DELETE /channels/1/messages/2/reactions/%61%2F%62%3A%33/@me",
						None,
					),
					(
						"GET /channels/1/messages?limit=1&around=2",
						Some(
							r#"[{"id":"2","channel_id":"1","author":{"id":"4","username":"Synthetic"},"reactions":[{"emoji":{"id":null,"name":"x"},"count":2,"me":true}]}]"#,
						),
					),
					(
						"GET /channels/1/messages?limit=1&around=2",
						Some(
							r#"[{"id":"9","channel_id":"1","author":{"id":"4","username":"Synthetic"}}]"#,
						),
					),
					(
						"GET /channels/1/messages?limit=1&around=2",
						Some(
							r#"[{"id":"2","channel_id":"9","author":{"id":"4","username":"Synthetic"}}]"#,
						),
					),
					("GET /channels/1/messages?limit=1&around=2", Some("[]")),
					(
						"GET /channels/1/messages?limit=1&around=2",
						Some(
							r#"[{"id":"2","channel_id":"1","author":{"id":"4","username":"Synthetic"}},{"id":"3","channel_id":"1","author":{"id":"4","username":"Synthetic"}}]"#,
						),
					),
					(
						"GET /channels/1/messages?limit=1&around=2",
						Some(
							r#"{"id":"2","channel_id":"1","author":{"id":"4","username":"Synthetic"}}"#,
						),
					),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = vec![];
					loop {
						let mut bytes = [0; 1024];
						let n = socket.read(&mut bytes).await.unwrap();
						assert!(n > 0);
						request.extend_from_slice(&bytes[..n]);
						assert!(request.len() < 4096);
						if request.windows(4).any(|w| w == b"\r\n\r\n") {
							break;
						}
					}
					let request = std::str::from_utf8(&request).unwrap();
					assert!(
						request.starts_with(&format!("{expected} HTTP/1.1\r\n")),
						"{request}"
					);
					assert!(request.contains("SYNTHETIC_REACTION_TOKEN"));
					let response = match body {
						Some(body) => format!(
							"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
							body.len()
						),
						None => "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".into(),
					};
					socket.write_all(response.as_bytes()).await.unwrap();
				}
			});
			for (emoji, add) in [
				(
					ReactionEmoji {
						id: None,
						name: Some("👍".into()),
					},
					true,
				),
				(
					ReactionEmoji {
						id: Some(Id(3)),
						name: Some("a/b".into()),
					},
					false,
				),
			] {
				assert!(matches!(
					api.execute(Command::Reactions(R::Set {
						channel: Id(1),
						message: Id(2),
						emoji,
						add,
						request: 1
					}))
					.await,
					Event::Reactions(E::Written { result: Ok(()), .. })
				));
			}
			let read = || {
				Command::Reactions(R::Read {
					channel: Id(1),
					message: Id(2),
					request: 2,
				})
			};
			assert!(
				matches!(api.execute(read()).await,Event::Reactions(E::Read{result:Ok(r),..}) if r.len()==1 && r[0].count==2 && r[0].me)
			);
			// Missing/deleted targets and neighbors cannot overwrite the selected message.
			for _ in 0..5 {
				assert!(matches!(
					api.execute(read()).await,
					Event::Reactions(E::Read {
						result: Err(Failure::Protocol),
						..
					})
				));
			}
			server.await.unwrap();
			assert!(
				reaction_path(
					Id(1),
					Id(2),
					&ReactionEmoji {
						id: None,
						name: None
					}
				)
				.is_none()
			);
		}
		{
			assert_eq!(
				reaction_users_path(
					model::Id(1),
					model::Id(2),
					&model::ReactionEmoji {
						id: Some(model::Id(3)),
						name: Some("a/b".into()),
					},
					Some(model::Id(4)),
				)
				.as_deref(),
				Some("/channels/1/messages/2/reactions/%61%2F%62%3A%33?limit=100&after=4")
			);
		}
	}

	#[tokio::test]
	async fn sticker_send_writes_one_id_and_preserves_reply() {
		tokio::time::timeout(Duration::from_secs(5), async {
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut api=DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_STICKER_TOKEN".into()).unwrap())).unwrap();
            api.base=format!("http://{}",listener.local_addr().unwrap());
            let server=tokio::spawn(async move {
                let (mut socket,_)=listener.accept().await.unwrap();
                let mut request=Vec::new();
                loop {
                    let mut bytes=[0;1024];let n=socket.read(&mut bytes).await.unwrap();assert!(n>0);request.extend_from_slice(&bytes[..n]);assert!(request.len()<4096);
                    if let Some(end)=request.windows(4).position(|w|w==b"\r\n\r\n") {
                        let headers=String::from_utf8_lossy(&request[..end]);
                        let length:usize=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned)).unwrap().parse().unwrap();
                        if request.len()>=end+4+length {
                            assert!(headers.starts_with("POST /channels/2/messages HTTP/1.1"));
                            let body:serde_json::Value=serde_json::from_slice(&request[end+4..]).unwrap();
                            assert_eq!(body["sticker_ids"],serde_json::json!(["9"]));assert_eq!(body["content"],"");assert_eq!(body["message_reference"]["message_id"],"50");break;
                        }
                    }
                }
                let body=r#"{"id":"100","channel_id":"2","author":{"id":"1","username":"Synthetic"},"content":"","nonce":"local","sticker_items":[{"id":"9","name":"Wave","format_type":1}]}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            });
            let result=api.execute(Command::Send{channel:model::Id(2),content:String::new(),nonce:"local".into(),reply:Some(Reply::to(model::Id(50))),sticker:Some(model::Id(9))}).await;
            assert!(matches!(result,Event::SendResult{result:Ok(message),..} if message.sticker_items.len()==1));
            server.await.unwrap();
        }).await.unwrap();
	}
	#[tokio::test]
	async fn optional_badge_capacity_preserves_profiles_but_expiry_ends_session() {
		tokio::time::timeout(Duration::from_secs(5), async {
			for case in 0..3 {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_BADGE_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let server = tokio::spawn(async move {
					let (mut stream, _) = listener.accept().await.unwrap();
					let mut buffer = [0; 4096];
					let n = stream.read(&mut buffer).await.unwrap();
					assert!(std::str::from_utf8(&buffer[..n]).unwrap().starts_with("GET /users/5/profile?"));
					let body = r#"{"user":{"id":"5","username":"Synthetic"},"badges":[{"id":"premium","description":"Subscriber"}]}"#;
					stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
					let (mut stream, _) = listener.accept().await.unwrap();
					let n = stream.read(&mut buffer).await.unwrap();
					assert!(std::str::from_utf8(&buffer[..n]).unwrap().starts_with("GET /users/5/badges HTTP/1.1"));
					let header = match case {
						0 => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", discord_protocol::badges::MAX_CATALOG_WIRE + 1),
						1 => "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".into(),
						_ => "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
					};
					stream.write_all(header.as_bytes()).await.unwrap();
					if case == 1 {
						// The reader may close the socket as soon as the streaming bound is hit.
						let _ = stream.write_all(&vec![b' '; discord_protocol::badges::MAX_CATALOG_WIRE + 1]).await;
					}
				});
				let event = api.execute(Command::Profile {
					user: model::Id(5),
					guild: None,
					request: 1,
					with_mutuals: true,
					cosmetics: Default::default(),
				}).await;
				if case == 2 {
					assert!(matches!(event, Event::Profile { result: Err(Failure::Expired), .. }));
					assert!(api.stopped());
				} else {
					let Event::Profile { result: Ok(profile), .. } = event else {
						panic!("oversized optional catalog must preserve the profile");
					};
					assert!(profile.badge_catalog.is_none());
					assert_eq!(profile.badges.len(), 1);
					assert_eq!(profile.badges[0].id, "premium");
					assert!(!api.stopped());
				}
				server.await.unwrap();
			}
		}).await.unwrap();
	}

	#[tokio::test]
	async fn profiles_are_scoped_capped_and_do_not_block_message_writes() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_PROFILE_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let api = Arc::new(api);
		let started = Arc::new(tokio::sync::Notify::new());
		let release = Arc::new(tokio::sync::Notify::new());
		let server_started = started.clone();
		let server_release = release.clone();
		let server = tokio::spawn(async move {
			let (mut profile, _) = listener.accept().await.unwrap();
			let mut buffer = [0; 4096];
			let n = profile.read(&mut buffer).await.unwrap();
			let request = std::str::from_utf8(&buffer[..n]).unwrap();
			assert!(request.starts_with("GET /users/5/profile?with_mutual_guilds=true&with_mutual_friends=true&with_mutual_friends_count=false&guild_id=2 HTTP/1.1"));
			assert!(request.contains("SYNTHETIC_PROFILE_TOKEN"));
			let body = r#"{"user":{"id":"5","username":"Synthetic"},"user_profile":{"bio":"About","banner":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#;
			profile
				.write_all(
					format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
						body.len()
					)
					.as_bytes(),
				)
				.await
				.unwrap();
			server_started.notify_one();
			let (mut write, _) = listener.accept().await.unwrap();
			let n = write.read(&mut buffer).await.unwrap();
			assert!(
				std::str::from_utf8(&buffer[..n])
					.unwrap()
					.starts_with("POST /channels/2/messages HTTP/1.1")
			);
			let sent = r#"{"id":"6","channel_id":"2","author":{"id":"1","username":"Synthetic"},"content":"Synthetic local test"}"#;
			write
				.write_all(
					format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sent}",
						sent.len()
					)
					.as_bytes(),
				)
				.await
				.unwrap();
			server_release.notified().await;
			profile.write_all(body.as_bytes()).await.unwrap();
			// The optional badge catalog follows the profile body.
			let (mut catalog, _) = listener.accept().await.unwrap();
			let n = catalog.read(&mut buffer).await.unwrap();
			assert!(
				std::str::from_utf8(&buffer[..n])
					.unwrap()
					.starts_with("GET /users/5/badges HTTP/1.1")
			);
			let badges = r#"{"badges":[{"badge_id":18,"name":"Account Age","owned":true}]}"#;
			catalog
				.write_all(
					format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{badges}",
						badges.len()
					)
					.as_bytes(),
				)
				.await
				.unwrap();
			for response in [
				format!(
					"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
					profile::MAX_PROFILE_WIRE + 1
				),
				{
					let body = r#"{"user":{"id":"7","username":"Wrong identity"}}"#;
					format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
						body.len()
					)
				},
			] {
				let (mut stream, _) = listener.accept().await.unwrap();
				let _ = stream.read(&mut buffer).await.unwrap();
				stream.write_all(response.as_bytes()).await.unwrap();
			}
		});
		let profile_api = api.clone();
		let pending = tokio::spawn(async move {
			profile_api
				.execute(Command::Profile {
					user: model::Id(5),
					guild: Some(model::Id(2)),
					request: 9,
					with_mutuals: true,
					cosmetics: Default::default(),
				})
				.await
		});
		tokio::time::timeout(Duration::from_secs(2), started.notified())
			.await
			.unwrap();
		let sent = tokio::time::timeout(
			Duration::from_secs(2),
			api.execute(Command::Send {
				sticker: None,
				channel: model::Id(2),
				content: "Synthetic local test".into(),
				nonce: "local".into(),
				reply: None,
			}),
		)
		.await
		.unwrap();
		assert!(matches!(sent, Event::SendResult { result: Ok(_), .. }));
		release.notify_one();
		let Event::Profile {
			user: model::Id(5),
			guild: Some(model::Id(2)),
			request: 9,
			result: Ok(profile),
		} = pending.await.unwrap()
		else {
			panic!("profile expected");
		};
		assert_eq!(
			profile
				.badge_catalog
				.as_ref()
				.map(|catalog| catalog.badges.len()),
			Some(1)
		);
		assert!(matches!(
			api.execute(Command::Profile {
				user: model::Id(5),
				guild: None,
				request: 10,
				with_mutuals: true,
				cosmetics: Default::default(),
			})
			.await,
			Event::Profile {
				result: Err(Failure::Capacity),
				request: 10,
				..
			}
		));
		assert!(!api.stopped());
		assert!(matches!(
			api.execute(Command::Profile {
				user: model::Id(5),
				guild: None,
				request: 11,
				with_mutuals: true,
				cosmetics: Default::default(),
			})
			.await,
			Event::Profile {
				result: Err(Failure::Protocol),
				request: 11,
				..
			}
		));
		server.await.unwrap();
		assert_eq!(api.requests.available_permits(), 4);
	}
	#[tokio::test]
	async fn explicit_dm_ring_and_decline_use_only_scoped_routes() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_OWNER_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			for (route, body) in [
				("ring", serde_json::json!({"recipients":null})),
				("stop-ringing", serde_json::json!({"recipients":["1"]})),
				("stop-ringing", serde_json::json!({})),
			] {
				let (mut stream, _) = listener.accept().await.unwrap();
				let mut bytes = vec![0; 4096];
				let n = stream.read(&mut bytes).await.unwrap();
				let text = std::str::from_utf8(&bytes[..n]).unwrap();
				assert!(text.starts_with(&format!("POST /channels/2/call/{route} HTTP/1.1")));
				let actual: serde_json::Value =
					serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap();
				assert_eq!(actual, body);
				stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
			}
		});
		api.ring_call(model::Id(2), None, false).await.unwrap();
		api.ring_call(model::Id(2), Some(model::Id(1)), true)
			.await
			.unwrap();
		api.ring_call(model::Id(2), None, true).await.unwrap();
		server.await.unwrap();
	}
	/// A message Discord rate-limits is resent with its nonce after the wait, not failed.
	#[tokio::test]
	async fn rate_limited_sends_wait_and_resend_with_the_same_nonce() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		let server = tokio::spawn(async move {
			let mut nonces = Vec::new();
			for response in [
				(
					"429 Too Many Requests",
					r#"{"message":"You are being rate limited.","retry_after":0.05,"global":false}"#,
				),
				(
					"200 OK",
					r#"{"id":"6","channel_id":"2","author":{"id":"1","username":"Synthetic"},"content":"e","nonce":"spam"}"#,
				),
			] {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut request = Vec::new();
				let payload = loop {
					let mut bytes = [0; 1024];
					let n = socket.read(&mut bytes).await.unwrap();
					assert!(n > 0);
					request.extend_from_slice(&bytes[..n]);
					assert!(request.len() < 8192);
					if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
						let header = std::str::from_utf8(&request[..end]).unwrap();
						assert!(header.starts_with("POST /channels/2/messages HTTP/1.1\r\n"));
						let length: usize = header
							.lines()
							.find_map(|line| {
								line.to_ascii_lowercase()
									.strip_prefix("content-length: ")
									.map(str::to_owned)
							})
							.unwrap()
							.parse()
							.unwrap();
						if request.len() >= end + 4 + length {
							break serde_json::from_slice::<serde_json::Value>(
								&request[end + 4..end + 4 + length],
							)
							.unwrap();
						}
					}
				};
				nonces.push(payload["nonce"] == "spam");
				let (status, body) = response;
				// An exhausted bucket without wait headers still waits only the body's delay.
				let exhausted = if status.starts_with("429") {
					"X-RateLimit-Remaining: 0\r\n"
				} else {
					""
				};
				socket
					.write_all(
						format!(
							"HTTP/1.1 {status}\r\n{exhausted}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
							body.len()
						)
						.as_bytes(),
					)
					.await
					.unwrap();
			}
			nonces
		});
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_SECRET_MARKER".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{address}");
		let mut throttled = Vec::new();
		let started = Instant::now();
		let event = tokio::time::timeout(
			Duration::from_secs(5),
			api.execute_write(
				Command::Send {
					sticker: None,
					channel: model::Id(2),
					content: "e".into(),
					nonce: "spam".into(),
					reply: None,
				},
				|nonce| throttled.push(nonce.to_owned()),
			),
		)
		.await
		.unwrap();
		assert!(matches!(event, Event::SendResult { result: Ok(_), .. }));
		assert!(started.elapsed() < Duration::from_millis(900));
		assert_eq!(throttled, ["spam"]);
		assert_eq!(server.await.unwrap(), [true, true]);
	}
	#[tokio::test]
	async fn local_http_checks_redirect_expiry_rate_limits_and_response_cap() {
		for (status, body, expected) in [
			(
				"302 Found",
				"",
				Failure::ProtocolAt("Account verification: HTTP response rejected"),
			),
			(
				"200 OK",
				"{\"synthetic_private_field\":\"must-not-appear-in-error\"}",
				Failure::ProtocolAt("Account verification: user response format unsupported"),
			),
			("401 Unauthorized", "{}", Failure::Expired),
			(
				"403 Forbidden",
				"{\"captcha_key\":[\"challenge\"]}",
				Failure::Challenged,
			),
		] {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let address = listener.local_addr().unwrap();
			let task = tokio::spawn(async move {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut buffer = [0; 4096];
				let n = socket.read(&mut buffer).await.unwrap();
				let request = String::from_utf8_lossy(&buffer[..n]);
				assert!(request.contains("SYNTHETIC_SECRET_MARKER"));
				let response = format!(
					"HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/never\r\nConnection: close\r\n\r\n{body}",
					body.len()
				);
				socket.write_all(response.as_bytes()).await.unwrap();
			});
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_SECRET_MARKER".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{address}");
			assert_eq!(api.current_user().await.map(|_| ()).unwrap_err(), expected);
			if expected.ends_session() {
				assert!(api.stopped());
			}
			task.await.unwrap();
		}
		fn limited(status: &str, body: &str) -> String {
			format!(
				"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
				body.len()
			)
		}
		{
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let address = listener.local_addr().unwrap();
			let attempts = usize::from(READ_RATE_LIMIT_RETRIES) + 1;
			let task = tokio::spawn(async move {
				for _ in 0..attempts {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut buffer = [0; 4096];
					let _ = socket.read(&mut buffer).await.unwrap();
					let body = r#"{"retry_after":0.05,"global":true}"#;
					socket
						.write_all(limited("429 Too Many Requests", body).as_bytes())
						.await
						.unwrap();
				}
			});
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_SECRET_MARKER".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{address}");
			assert_eq!(
				api.current_user().await.map(|_| ()).unwrap_err(),
				Failure::RateLimited
			);
			assert!(!api.stopped());
			task.await.unwrap();
		}
		{
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let address = listener.local_addr().unwrap();
			let task = tokio::spawn(async move {
				for (status, body) in [
					(
						"429 Too Many Requests",
						r#"{"retry_after":0.05,"global":true}"#,
					),
					("200 OK", r#"{"id":"42","username":"synthetic"}"#),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut buffer = [0; 4096];
					let _ = socket.read(&mut buffer).await.unwrap();
					socket
						.write_all(limited(status, body).as_bytes())
						.await
						.unwrap();
				}
			});
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_SECRET_MARKER".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{address}");
			assert_eq!(api.current_user().await.unwrap().id, Id(42));
			task.await.unwrap();
		}
		{
			// A Cloudflare block page moves reads to the test host for the rest of the process,
			// and a block page there fails with a cooldown instead of another retry.
			fn page(status: &str, content_type: &str, body: &str) -> String {
				format!(
					"HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				)
			}
			let blocked = page("429 Too Many Requests", "text/html", "<html>blocked</html>");
			let user = page(
				"200 OK",
				"application/json",
				r#"{"id":"42","username":"synthetic"}"#,
			);
			let serve = |listener: TcpListener, responses: Vec<String>| {
				tokio::spawn(async move {
					for response in responses {
						let (mut socket, _) = listener.accept().await.unwrap();
						let mut request = Vec::new();
						while !request.windows(4).any(|w| w == b"\r\n\r\n") {
							let mut bytes = [0; 1024];
							let n = socket.read(&mut bytes).await.unwrap();
							assert!(n > 0 && request.len() < 4096);
							request.extend_from_slice(&bytes[..n]);
						}
						assert!(request.starts_with(b"GET /users/@me HTTP/1.1\r\n"));
						socket.write_all(response.as_bytes()).await.unwrap();
					}
				})
			};
			let stable = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let test = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_SECRET_MARKER".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", stable.local_addr().unwrap());
			api.test_host = Some((
				format!("http://{}", test.local_addr().unwrap()),
				AtomicBool::new(false),
			));
			let stable = serve(stable, vec![blocked.clone()]);
			let test = serve(test, vec![user.clone(), user, blocked]);
			assert_eq!(api.current_user().await.unwrap().id, Id(42));
			stable.await.unwrap();
			assert_eq!(api.current_user().await.unwrap().id, Id(42));
			assert_eq!(
				api.current_user().await.map(|_| ()).unwrap_err(),
				Failure::RateLimited
			);
			assert!(*api.cooldown.lock().await > Instant::now() + Duration::from_secs(30));
			assert!(!api.stopped());
			test.await.unwrap();
		}
		assert!(safe_delay(Some(f64::NAN)).is_err());
		assert!(safe_delay(Some(-1.0)).is_err());
	}
}

fn allowed_mentions(content: &str, reply: Option<client_core::Reply>) -> serde_json::Value {
	let everyone: &[&str] = if model::has_mass_mention(content) {
		&["everyone"]
	} else {
		&[]
	};
	serde_json::json!({"parse":everyone,"users":model::mentioned_user_ids(content),"roles":model::mentioned_role_ids(content),"replied_user":reply.is_some_and(|r| r.mention)})
}
#[cfg(test)]
mod mention_tests {
	#[test]
	fn mass_mentions_and_explicit_users_are_allowed() {
		assert_eq!(
			super::allowed_mentions("hello test", None),
			serde_json::json!({"parse":[],"users":[],"roles":[],"replied_user":false})
		);
		assert_eq!(
			super::allowed_mentions("@everyone <@&4> <@7> <@!7> <@9>", None),
			serde_json::json!({"parse":["everyone"],"users":["7","9"],"roles":["4"],"replied_user":false})
		);
		assert_eq!(
			super::allowed_mentions("@here", None),
			serde_json::json!({"parse":["everyone"],"users":[],"roles":[],"replied_user":false})
		);
	}
}
