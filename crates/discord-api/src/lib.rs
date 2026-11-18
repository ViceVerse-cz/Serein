mod commands;
mod force_sync;
mod history;
mod http;
mod messages;
mod rate_limits;
mod read_state;
// Origin-fixed REST adapter. No cookies, redirects, logging, persistence or bot SDK.
mod account_settings;
mod activity_sharing;
mod archives;
mod badges;
mod channel_actions;
mod collectibles;
mod community_admin;
mod preferences_sync;

mod community;
pub mod detectable;
pub mod external_assets;
mod forum;
mod gif_favorites;
mod group_actions;
mod guild_folders;
mod interactions;
mod member_guide;
mod message_options;
mod messaging_permissions;
mod mfa;
mod onboarding;
mod owned_cosmetics;
mod profile_edit;
mod server_profile;
#[cfg(debug_assertions)]
pub use profile_edit::debug_profile_board_request_check;
mod command_permissions;
mod inbox;
pub mod proxy;
#[cfg(test)]
mod proxy_tests;
mod reporting;
pub mod rpc;
mod server_access;
mod server_actions;
mod server_admin;
mod server_audit_log;
mod server_integrations;
mod server_invites;
mod server_moderation;
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
	static ADMIN_READ: ();
}
/// REST permits a preload must leave free for requests the user is waiting for.
const FOREGROUND_PERMITS: usize = 2;
const READ_RATE_LIMIT_RETRIES: u8 = 2;
/// Bound the time a cooled-down route can hold a foreground operation.
const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(10);
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
	cooldown: Mutex<rate_limits::Limits>,
	stop_signal: tokio::sync::Notify,
	requests: Semaphore,
	admin_reads: Semaphore,
	review_progress: std::sync::Mutex<Option<server_moderation::ReviewNotify>>,
	stopped: AtomicBool,
	catalog: std::sync::Mutex<collectibles::Catalog>,
	mfa: mfa::Broker,
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
	SecretJson(zeroize::Zeroizing<Vec<u8>>),
	Multipart { content_type: String, body: Vec<u8> },
}

struct ResponsePolicy<'a> {
	max_bytes: usize,
	missing: Option<&'a mut bool>,
}
impl ResponsePolicy<'_> {
	fn required(max_bytes: usize) -> Self {
		Self {
			max_bytes,
			missing: None,
		}
	}
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
			cooldown: Mutex::new(rate_limits::Limits::default()),
			stop_signal: tokio::sync::Notify::new(),
			requests: Semaphore::new(4),
			admin_reads: Semaphore::new(2),
			review_progress: std::sync::Mutex::default(),
			stopped: AtomicBool::new(false),
			catalog: std::sync::Mutex::default(),
			mfa: mfa::Broker::default(),
			#[cfg(test)]
			base: "https://discord.com/api/v10".into(),
			#[cfg(test)]
			test_host: None,
			#[cfg(test)]
			upload_origin: None,
		})
	}
	pub fn stop(&self) {
		self.stopped.store(true, Ordering::Release);
		self.requests.close();
		self.admin_reads.close();
		self.stop_signal.notify_waiters();
		self.mfa.cancel_all();
	}
	pub fn stopped(&self) -> bool {
		self.stopped.load(Ordering::Acquire)
	}
	/// Read-only startup recovery. Rejected credentials and ambiguous writes are never retried.
	pub async fn bootstrap(&self) -> Result<(User, String), Failure> {
		let deadline = Instant::now() + Duration::from_secs(30);
		for attempt in 0..=2 {
			let result = tokio::time::timeout_at(deadline, async {
				let user = self.current_user().await?;
				let gateway = self.gateway_url().await?;
				Ok((user, gateway))
			})
			.await
			.unwrap_or(Err(Failure::Network));
			if !matches!(result, Err(Failure::Network)) || attempt == 2 || self.stopped() {
				return result;
			}
			let jitter = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap_or_default()
				.subsec_millis()
				% 250;
			let retry =
				Instant::now() + Duration::from_millis((1000 << attempt) + u64::from(jitter));
			if retry >= deadline {
				return Err(Failure::Network);
			}
			sleep_until(retry).await;
		}
		Err(Failure::Network)
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
}
impl DiscordApi {
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
	#[tokio::test]
	async fn startup_retries_transient_reads_but_not_revoked_credentials() {
		for revoked in [false, true] {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("synthetic-bootstrap-recovery".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = async {
				let replies = if revoked {
					vec![("401 Unauthorized", "{\"code\":0}")]
				} else {
					vec![
						("503 Service Unavailable", "{}"),
						("200 OK", "{\"id\":\"1\",\"username\":\"Synthetic\"}"),
						("200 OK", "{\"url\":\"wss://gateway.discord.gg/\"}"),
					]
				};
				for (status, body) in replies {
					let (mut stream, _) = listener.accept().await.unwrap();
					let mut headers = Vec::new();
					while !headers.ends_with(b"\r\n\r\n") {
						headers.push(stream.read_u8().await.unwrap());
						assert!(headers.len() < 4096);
					}
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
			};
			let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
				tokio::join!(api.bootstrap(), server)
			})
			.await
			.unwrap();
			if revoked {
				assert!(matches!(result, Err(Failure::Expired)));
				assert!(api.stopped());
			} else {
				let (user, url) = result.unwrap();
				assert_eq!(user.id, Id(1));
				assert_eq!(url, "wss://gateway.discord.gg/");
				assert!(!api.stopped());
			}
		}
	}
	#[tokio::test]
	async fn cooling_route_waiters_leave_permits_for_unrelated_requests() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("synthetic-isolated-cooldown".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		api.cooldown
			.lock()
			.await
			.observe(
				&rate_limits::Route::new(&Method::GET, "/channels/1/messages"),
				Some("messages"),
				Some(Duration::from_secs(5)),
				false,
			)
			.unwrap();
		let server = async {
			let (mut stream, _) = listener.accept().await.unwrap();
			let mut bytes = Vec::new();
			while !bytes.ends_with(b"\r\n\r\n") {
				bytes.push(stream.read_u8().await.unwrap());
				assert!(bytes.len() < 4096);
			}
			assert!(bytes.starts_with(b"GET /channels/2/messages "));
			stream
				.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
				.await
				.unwrap();
		};
		let waiting = async {
			let _ = tokio::join!(
				api.request(Method::GET, "/channels/1/messages", None),
				api.request(Method::GET, "/channels/1/messages", None),
				api.request(Method::GET, "/channels/1/messages", None),
				api.request(Method::GET, "/channels/1/messages", None)
			);
		};
		let foreground = async {
			tokio::task::yield_now().await;
			assert_eq!(
				api.request(Method::GET, "/channels/2/messages", None)
					.await
					.unwrap(),
				b"{}"
			);
		};
		tokio::time::timeout(Duration::from_secs(2),async {tokio::select! {()=waiting=>panic!("cooling route sent early"), _=async {tokio::join!(server,foreground);}=>{}}}).await.unwrap();
		assert_eq!(api.requests.available_permits(), 4);
		assert!(!api.stopped());
	}
	#[tokio::test]
	async fn long_cooldown_rejects_an_unsent_write_without_releasing_the_penalty() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("synthetic-long-cooldown".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let path = "/channels/1/messages";
		let route = rate_limits::Route::new(&Method::POST, path);
		api.cooldown
			.lock()
			.await
			.observe(&route, Some("send"), Some(Duration::from_secs(60)), false)
			.unwrap();
		let result = tokio::time::timeout(
			Duration::from_secs(2),
			api.request(
				Method::POST,
				path,
				Some(serde_json::json!({"content":"fixture"})),
			),
		)
		.await;
		assert!(matches!(result, Ok(Err(Failure::RateLimited))));
		assert!(
			api.cooldown.lock().await.deadline(&route).unwrap()
				> Instant::now() + MAX_RATE_LIMIT_WAIT
		);
		assert!(
			tokio::time::timeout(Duration::from_millis(30), listener.accept())
				.await
				.is_err()
		);
		assert_eq!(api.requests.available_permits(), 4);
		assert!(!api.stopped());
	}
	#[tokio::test]
	async fn stop_wakes_a_rate_limit_wait_without_sending() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("synthetic-stop-cooldown".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		api.cooldown.lock().await.global = Instant::now() + MAX_RATE_LIMIT_WAIT;
		let stop = async {
			tokio::task::yield_now().await;
			api.stop();
		};
		let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
			tokio::join!(api.request(Method::GET, "/users/@me", None), stop)
		})
		.await
		.unwrap();
		assert_eq!(result, Err(Failure::Expired));
		assert!(
			tokio::time::timeout(Duration::from_millis(25), listener.accept())
				.await
				.is_err()
		);
	}
	#[tokio::test]
	async fn optional_read_preserves_missing_result_after_a_rate_limit_retry() {
		tokio::time::timeout(Duration::from_secs(3), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_OPTIONAL_RETRY".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = async {
				for (status, body) in [
					("429 Too Many Requests", r#"{"retry_after":0.05}"#),
					("404 Not Found", "{}"),
				] {
					let (mut stream, _) = listener.accept().await.unwrap();
					let mut headers = Vec::new();
					while !headers.ends_with(b"\r\n\r\n") {
						assert!(headers.len() < 4096);
						headers.push(stream.read_u8().await.unwrap());
					}
					assert!(
						std::str::from_utf8(&headers)
							.unwrap()
							.starts_with("GET /synthetic/optional HTTP/1.1")
					);
					stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
				}
			};
			let client = async {
				assert_eq!(
					api.request_optional_limited("/synthetic/optional", 128).await,
					Ok(None)
				);
				assert!(!api.stopped());
			};
			tokio::join!(server, client);
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn response_overflow_preserves_write_ambiguity_and_definite_rejections() {
		for chunked in [false, true] {
			for (method, status, expected) in [
				(Method::POST, "200 OK", Failure::Ambiguous),
				(
					Method::POST,
					"500 Internal Server Error",
					Failure::Ambiguous,
				),
				(Method::POST, "400 Bad Request", Failure::ResponseLimit),
				(Method::POST, "403 Forbidden", Failure::ResponseLimit),
				(
					Method::POST,
					"429 Too Many Requests",
					Failure::ResponseLimit,
				),
				(Method::GET, "200 OK", Failure::ResponseLimit),
			] {
				let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_BODY_LIMIT_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let server = tokio::spawn(async move {
					let (mut stream, _) = listener.accept().await.unwrap();
					let mut headers = Vec::new();
					while !headers.ends_with(b"\r\n\r\n") {
						assert!(headers.len() < 4096);
						headers.push(stream.read_u8().await.unwrap());
					}
					let body = if chunked {
						"Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\n12345\r\n0\r\n\r\n"
					} else {
						"Content-Length: 5\r\nConnection: close\r\n\r\n12345"
					};
					stream
						.write_all(format!("HTTP/1.1 {status}\r\n{body}").as_bytes())
						.await
						.unwrap();
					assert!(
						tokio::time::timeout(Duration::from_millis(25), listener.accept())
							.await
							.is_err(),
						"writes are never replayed"
					);
				});
				assert_eq!(
					api.request_limited(method, "/synthetic", None, 4).await,
					Err(expected)
				);
				server.await.unwrap();
			}
		}
	}

	#[tokio::test]
	async fn accepted_message_with_oversized_response_reports_ambiguous_delivery() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_MESSAGE_LIMIT_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			let (mut stream, _) = listener.accept().await.unwrap();
			let mut buffer = [0; 4096];
			let count = stream.read(&mut buffer).await.unwrap();
			assert!(
				std::str::from_utf8(&buffer[..count])
					.unwrap()
					.starts_with("POST /channels/2/messages HTTP/1.1")
			);
			stream
				.write_all(
					format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
						MAX_WIRE + 1
					)
					.as_bytes(),
				)
				.await
				.unwrap();
			assert!(
				tokio::time::timeout(Duration::from_millis(25), listener.accept())
					.await
					.is_err()
			);
		});
		assert_eq!(
			api.send_message(model::Id(2), "Synthetic message", "7", None, None, None)
				.await
				.map(|_| ()),
			Err(Failure::Ambiguous)
		);
		server.await.unwrap();
	}

	#[tokio::test]
	async fn accepted_message_with_invalid_rate_header_reports_ambiguity_without_retry() {
		for reset in ["NaN", "-1", "86401"] {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_RATE_HEADER_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				let (mut stream, _) = listener.accept().await.unwrap();
				let mut headers = Vec::new();
				while !headers.ends_with(b"\r\n\r\n") {
					assert!(headers.len() < 4096);
					headers.push(stream.read_u8().await.unwrap());
				}
				assert!(
					std::str::from_utf8(&headers)
						.unwrap()
						.starts_with("POST /channels/2/messages HTTP/1.1")
				);
				stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nx-ratelimit-remaining: 0\r\nx-ratelimit-reset-after: {reset}\r\nConnection: close\r\n\r\n{{}}").as_bytes()).await.unwrap();
				assert!(
					tokio::time::timeout(Duration::from_millis(25), listener.accept())
						.await
						.is_err(),
					"an uncertain send must never be replayed"
				);
			});
			assert_eq!(
				api.send_message(model::Id(2), "Synthetic message", "7", None, None, None)
					.await
					.map(|_| ()),
				Err(Failure::Ambiguous)
			);
			server.await.unwrap();
		}
	}

	#[tokio::test]
	async fn unauthorized_response_stops_traffic_before_invalid_rate_header_parsing() {
		for method in [Method::GET, Method::POST] {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_EXPIRED_HEADER_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				let (mut stream, _) = listener.accept().await.unwrap();
				let mut headers = Vec::new();
				while !headers.ends_with(b"\r\n\r\n") {
					assert!(headers.len() < 4096);
					headers.push(stream.read_u8().await.unwrap());
				}
				stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nx-ratelimit-remaining: 0\r\nx-ratelimit-reset-after: NaN\r\nConnection: close\r\n\r\n{}").await.unwrap();
				assert!(
					tokio::time::timeout(Duration::from_millis(25), listener.accept())
						.await
						.is_err(),
					"expired credentials cannot dispatch another request"
				);
			});
			assert_eq!(
				api.request(method, "/synthetic", None).await,
				Err(Failure::Expired)
			);
			assert!(api.stopped());
			assert_eq!(
				api.request(Method::POST, "/synthetic", None).await,
				Err(Failure::Expired)
			);
			server.await.unwrap();
		}
	}

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
		api.cooldown.lock().await.global = Instant::now() + Duration::from_secs(30);
		let event = tokio::time::timeout(Duration::from_secs(1), api.execute(preload()))
			.await
			.expect("a preload never waits for the cooldown");
		assert!(gave_way(event));
		// Without spare permits it leaves the remaining ones to requests the user waits for.
		api.cooldown.lock().await.global = Instant::now();
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
						("after=150", vec![152, 151]),
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
				let oversized = api
					.execute(Command::History {
						channel: Id(1),
						before: None,
						after: Some(Id(99)),
						request: 8,
					})
					.await;
				assert!(matches!(
					oversized,
					Event::HistoryFailed {
						channel: Id(1),
						request: 8,
						failure: Failure::ResponseLimit,
					}
				));
				let mut state = client_core::State::default();
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: Event::Ready {
						user: decode::<UserDto>(br#"{"id":"3","username":"Synthetic"}"#)
							.unwrap()
							.into_model(),
						guilds: vec![],
						channels: vec![],
						permissions: Default::default(),
					},
				});
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: oversized,
				});
				assert_eq!(state.auth, client_core::auth::AuthState::Authenticated);
				assert!(state.gateway_connected);
				assert!(!api.stopped());
				assert!(matches!(
					api.execute(Command::History {
						channel: Id(1), before: None, after: Some(Id(150)), request: 9,
					}).await,
					Event::History { messages, .. } if messages.len() == 2
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
            assert!(api.cooldown.lock().await.global<=Instant::now());
            server.await.unwrap();
        }).await.unwrap();
	}
	#[tokio::test]
	async fn reply_preview_reads_exact_rows_and_around_history_preserves_context() {
		tokio::time::timeout(Duration::from_secs(10),async {
			let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api=DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_REPLY_PREVIEW_TOKEN".into()).unwrap())).unwrap();
			api.base=format!("http://{}",listener.local_addr().unwrap());
			let server=tokio::spawn(async move {
				for (route,body) in [
					("GET /channels/1/messages?limit=1&around=2",r#"[{"id":"2","channel_id":"1","author":{"id":"4","username":"Original"},"content":"Original body"}]"#),
					("GET /channels/1/messages?limit=1&around=2",r#"[{"id":"3","channel_id":"1","author":{"id":"4","username":"Neighbour"}}]"#),
					("GET /channels/1/messages?limit=1&around=2",r#"[{"id":"2","channel_id":"9","author":{"id":"4","username":"Other channel"}}]"#),
					("GET /channels/1/messages?limit=1&around=2","[]"),
					("GET /channels/1/messages?limit=50&around=2",r#"[{"id":"1","channel_id":"1","author":{"id":"4","username":"Context"}},{"id":"2","channel_id":"1","author":{"id":"4","username":"Original"}},{"id":"3","channel_id":"1","author":{"id":"4","username":"Context"}}]"#),
				] {
					let (mut stream,_)=listener.accept().await.unwrap();let mut request=Vec::new();
					while !request.ends_with(b"\r\n\r\n") { assert!(request.len()<4096);request.push(stream.read_u8().await.unwrap()); }
					assert!(std::str::from_utf8(&request).unwrap().starts_with(&format!("{route} HTTP/1.1\r\n")));
					stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
				}
			});
			let read=|request|Command::ReplyPreview {channel:Id(1),message:Id(2),request};
			let Event::ReplyPreview(result)=api.execute(read(1)).await else {panic!("preview result")};
			assert_eq!(result.result.unwrap().unwrap().message().content,"Original body");
			for request in 2..=4 {
				let Event::ReplyPreview(result)=api.execute(read(request)).await else {panic!("preview result")};
				assert!(result.result.unwrap().is_none(),"never render a neighbour or other channel");
			}
			let Event::History {messages,older:false,request:5,..}=api.execute(Command::HistoryAround {channel:Id(1),target:Id(2),request:5}).await else {panic!("around history")};
			assert_eq!(messages.iter().map(|m|m.id).collect::<Vec<_>>(),vec![Id(1),Id(2),Id(3)]);
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
	async fn soundboard_reads_bounded_catalogs_and_sends_one_documented_request() {
		use client_core::soundboard::{Event as E, Request as R};
		tokio::time::timeout(Duration::from_secs(5), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_SOUNDBOARD_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				for (line, expected, reply) in [
					(
						"GET /soundboard-default-sounds HTTP/1.1",
						None,
						r#"[{"name":"quack","sound_id":"1","volume":1.0,"guild_id":"0"}]"#,
					),
					(
						"GET /guilds/9/soundboard-sounds HTTP/1.1",
						None,
						r#"{"items":[{"name":"Yay","sound_id":"30","volume":0.5,"guild_id":"9"}]}"#,
					),
					(
						"POST /channels/2/send-soundboard-sound HTTP/1.1",
						Some(serde_json::json!({"sound_id":"30","source_guild_id":"8"})),
						"",
					),
					(
						"POST /channels/2/send-soundboard-sound HTTP/1.1",
						Some(serde_json::json!({"sound_id":"31"})),
						"DROP_RESPONSE",
					),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = Vec::new();
					loop {
						let mut bytes = [0; 1024];
						let n = socket.read(&mut bytes).await.unwrap();
						assert!(n > 0);
						request.extend_from_slice(&bytes[..n]);
						assert!(request.len() < 4096);
						let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
							continue;
						};
						let headers = String::from_utf8_lossy(&request[..end]).into_owned();
						assert!(headers.starts_with(line), "{headers}");
						let Some(expected) = &expected else {
							break;
						};
						let length: usize = headers
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
							let body: serde_json::Value =
								serde_json::from_slice(&request[end + 4..]).unwrap();
							assert_eq!(&body, expected);
							break;
						}
					}
					if reply == "DROP_RESPONSE" {
						drop(socket);
						continue;
					}
					let status = if reply.is_empty() {
						"204 No Content"
					} else {
						"200 OK"
					};
					socket
						.write_all(
							format!(
								"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
								reply.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
				}
				assert!(
					tokio::time::timeout(Duration::from_millis(150), listener.accept())
						.await
						.is_err(),
					"an uncertain sound must not be repeated"
				);
			});
			let Event::Soundboard(E::Default {
				request: 1,
				result: Ok(sounds),
			}) = api.execute(Command::Soundboard(R::Default { request: 1 }))
				.await
			else {
				panic!("default sounds must decode");
			};
			assert_eq!((sounds[0].id, sounds[0].guild), (model::Id(1), None));
			let Event::Soundboard(E::Guild {
				request: 2,
				guild,
				result: Ok(sounds),
			}) = api.execute(Command::Soundboard(R::Guild {
				guild: model::Id(9),
				request: 2,
			}))
			.await
			else {
				panic!("server sounds must decode");
			};
			assert_eq!(guild, model::Id(9));
			assert_eq!(
				(sounds[0].id, sounds[0].guild, sounds[0].volume),
				(model::Id(30), Some(model::Id(9)), 0.5)
			);
			assert!(matches!(
				api.execute(Command::Soundboard(R::Send {
					call: 7,
					request: 3,
					channel: model::Id(2),
					sound: model::Id(30),
					source_guild: Some(model::Id(8)),
				}))
				.await,
				Event::Soundboard(E::Sent {
					request: 3,
					channel: model::Id(2),
					sound: model::Id(30),
					result: Ok(()),
				})
			));
			assert!(matches!(
				api.execute(Command::Soundboard(R::Send {
					call: 7,
					request: 5,
					channel: model::Id(2),
					sound: model::Id(31),
					source_guild: None,
				}))
				.await,
				Event::Soundboard(E::Sent {
					request: 5,
					result: Err(Failure::Ambiguous),
					..
				})
			));
			server.await.unwrap();
			// Zero identifiers never reach the network.
			assert!(matches!(
				api.execute(Command::Soundboard(R::Send {
					call: 7,
					request: 3,
					channel: model::Id(0),
					sound: model::Id(30),
					source_guild: None,
				}))
				.await,
				Event::Soundboard(E::Sent {
					result: Err(Failure::Protocol),
					..
				})
			));
			assert!(matches!(
				api.execute(Command::Soundboard(R::Guild {
					guild: model::Id(0),
					request: 4
				}))
				.await,
				Event::Soundboard(E::Guild {
					result: Err(Failure::Protocol),
					..
				})
			));
		})
		.await
		.unwrap();
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
				result: Err(Failure::ResponseLimit),
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
			assert!(api.cooldown.lock().await.global > Instant::now() + Duration::from_secs(30));
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
