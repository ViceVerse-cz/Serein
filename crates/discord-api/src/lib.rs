// Direct, origin-fixed REST adapter. No cookies, redirects, logging, persistence or bot SDK.
mod activity_sharing;
mod archives;
mod channel_actions;
mod forum;
mod group_actions;
mod guild_folders;
mod messaging_permissions;
mod notification_settings;
mod profile_edit;
pub mod rpc;
mod server_actions;
mod server_admin;
mod server_audit_log;
mod server_integrations;
mod server_invites;
mod server_roles;
mod server_settings;
pub mod upload;
mod user_actions;
use client_core::{
	Command, Event,
	auth::{AuthProvider, Failure, SessionSecret},
};
use discord_protocol::*;
use model::User;
use reqwest::{
	Client, Method, StatusCode,
	header::{AUTHORIZATION, HeaderValue},
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

pub struct DiscordApi {
	ack_token: Mutex<zeroize::Zeroizing<Option<String>>>,
	client: Client,
	upload_client: tokio::sync::OnceCell<Client>,
	secret: Arc<SessionSecret>,
	cooldown: Mutex<Instant>,
	requests: Semaphore,
	stopped: AtomicBool,
	#[cfg(test)]
	base: String,
	#[cfg(test)]
	upload_origin: Option<std::net::SocketAddr>,
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
impl DiscordApi {
	pub fn new(secret: Arc<SessionSecret>) -> Result<Self, Failure> {
		let client = Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			// Keep writes single-attempt, including attachment slot allocation.
			.retry(reqwest::retry::never())
			.no_proxy()
			.timeout(Duration::from_secs(20))
			.connect_timeout(Duration::from_secs(10))
			.user_agent(client_core::fingerprint::user_agent())
			.default_headers(fingerprint_headers()?)
			.build()
			.map_err(|_| Failure::Network)?;
		Ok(Self {
			upload_client: tokio::sync::OnceCell::new(),
			ack_token: Mutex::new(zeroize::Zeroizing::new(None)),
			client,
			secret,
			cooldown: Mutex::new(Instant::now()),
			requests: Semaphore::new(4),
			stopped: AtomicBool::new(false),
			#[cfg(test)]
			base: "https://discord.com/api/v10".into(),
			#[cfg(test)]
			upload_origin: None,
		})
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
		self.request_with_captcha(method, path, body, max_bytes, None, None)
			.await
	}
	async fn request_with_captcha(
		&self,
		method: Method,
		path: &str,
		body: Option<serde_json::Value>,
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
		let _permit = self
			.requests
			.acquire()
			.await
			.map_err(|_| Failure::Network)?;
		// Four permits bound concurrent REST work. A slow profile body must not hold the
		// cooldown mutex and delay a message write; only service rate admission is shared.
		loop {
			let next = *self.cooldown.lock().await;
			sleep_until(next).await;
			if Instant::now() >= *self.cooldown.lock().await {
				break;
			}
		}
		if self.stopped() {
			return Err(Failure::Expired);
		}
		let mut authorization =
			HeaderValue::from_str(self.secret.expose()).map_err(|_| Failure::InvalidCredential)?;
		authorization.set_sensitive(true);
		#[cfg(not(test))]
		let base = "https://discord.com/api/v10";
		#[cfg(test)]
		let base = &self.base;
		let write = method != Method::GET;
		let mut request = self
			.client
			.request(method, format!("{base}{path}"))
			.header(AUTHORIZATION, authorization);
		if let Some(retry) = retry {
			if retry.expired() {
				return Err(Failure::ProtocolAt("Verification expired; join again"));
			}
			for (name, value) in [
				("x-captcha-key", Some(retry.passcode())),
				("x-captcha-rqtoken", retry.rqtoken()),
				("x-captcha-session-id", retry.session_id()),
			] {
				if let Some(value) = value {
					let mut value = HeaderValue::from_str(value).map_err(|_| Failure::Protocol)?;
					value.set_sensitive(true);
					request = request.header(name, value);
				}
			}
		}
		if let Some(body) = body {
			request = request.json(&body);
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
		if exhausted || status == StatusCode::TOO_MANY_REQUESTS {
			let mut next = self.cooldown.lock().await;
			*next = (*next).max(Instant::now() + safe_delay(reset.or(retry_header))?);
		}
		if status == StatusCode::UNAUTHORIZED {
			self.stop();
			return Err(Failure::Expired);
		}
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
				if matches!(status, StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN)
					&& error.captcha_key.is_some()
					&& !matches!(error.code, Some(60003 | 50014))
					&& let Some(output) = challenge.as_mut()
				{
					if let Some(parsed) = invite_captcha(&bytes) {
						**output = Some(parsed);
						return Err(Failure::Challenged);
					}
					return Err(Failure::ProtocolAt(
						"This invite's verification is unavailable; try joining in Discord",
					));
				}

				self.stop();
				return Err(Failure::Challenged);
			}
			if status == StatusCode::TOO_MANY_REQUESTS {
				let mut next = self.cooldown.lock().await;
				*next =
					(*next).max(Instant::now() + safe_delay(error.retry_after.or(retry_header))?);
				return Err(Failure::RateLimited);
			}
			// A missing private note is empty, not a missing user profile. No other 404 is converted.
			if !write && status == StatusCode::NOT_FOUND && path.starts_with("/users/@me/notes/") {
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
		Ok(std::mem::take(&mut *bytes))
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
	async fn join_invite(
		&self,
		code: &str,
		request: u64,
		captcha: Option<Box<client_core::captcha::Retry>>,
	) -> Event {
		let mut challenge = None;
		let result = if client_core::invites::valid_code(code)
			&& captcha.as_ref().is_none_or(|c| c.matches(code, request))
		{
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
	pub async fn execute(&self, command: Command) -> Event {
		match command {
			Command::MessagingPermissions { request, change } => Event::MessagingPermissions {
				request,
				result: self.account_messaging_permissions(change).await,
			},
			Command::AccountNotificationSettings {
				request,
				section,
				change,
			} => Event::AccountNotificationSettings {
				request,
				result: self.account_notification_settings(section, change).await,
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
			Command::GuildFolders(settings) => Event::GuildFolders(match settings {
				Some(settings) => self.save_guild_folders(settings).await,
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
			Command::UserAction { action, request } => {
				if let client_core::user_actions::Action::LoadNote(user) = action {
					return Event::UserAction(client_core::user_actions::Event::NoteLoaded {
						user,
						request,
						result: self.user_note(user).await,
					});
				}
				let result = self.user_action(&action).await;
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
				request,
			} => {
				let result = self.create_post(parent, guild, &title, &content).await;
				Event::PostCreated {
					parent,
					request,
					result,
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
				request,
			} => {
				let result = self.search(channel, guild, &query, before).await;
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
			Command::MarkRead {
				channel,
				message,
				request,
			} => {
				let result = self.mark_read(channel, message).await;
				Event::ReadState(client_core::read_state::Event::Result {
					channel,
					message,
					request,
					result,
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
				})
			}
			Command::Profile {
				user,
				guild,
				request,
			} => {
				let mut path = format!(
					"/users/{user}/profile?with_mutual_guilds=true&with_mutual_friends=false&with_mutual_friends_count=false"
				);
				if let Some(guild) = guild {
					path.push_str(&format!("&guild_id={guild}"));
				}
				let result = self
					.request_limited(Method::GET, &path, None, profile::MAX_PROFILE_WIRE)
					.await
					.and_then(|bytes| {
						let profile = profile::decode_profile(&bytes, guild)
							.map_err(|_| Failure::Protocol)?;
						if profile.user.id != user {
							return Err(Failure::Protocol);
						}
						Ok(Box::new(profile))
					});
				Event::Profile {
					user,
					guild,
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
			Command::Voice(_) | Command::Members { .. } => Event::Failure(Failure::Protocol),
			Command::History {
				channel,
				before,
				after,
				request,
			} => {
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
					.and_then(|bytes| {
						decode::<Vec<MessageDto>>(&bytes).map_err(|_| Failure::Protocol)
					}) {
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
			Command::Send {
				channel,
				content,
				nonce,
				reply,
			} => {
				let result = self
					.send_message(channel, &content, &nonce, reply, None)
					.await;
				Event::SendResult { nonce, result }
			}
			Command::Edit {
				request,
				channel,
				message,
				content,
			} => {
				if content.trim().is_empty() || content.chars().count() > client_core::MAX_CONTENT {
					return Event::Edited {
						request,
						channel,
						message,
						result: Err(Failure::Capacity),
					};
				}
				let body = serde_json::json!({"content": content, "allowed_mentions": allowed_mentions(&content)});
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
	async fn mark_read(&self, channel: model::Id, message: model::Id) -> Result<(), Failure> {
		#[derive(serde::Deserialize)]
		struct Reply {
			#[serde(default)]
			token: Option<String>,
		}
		// Legacy acknowledgement tokens are session-only, redacted by ownership, and never cached.
		let mut token = self.ack_token.lock().await;
		let body = serde_json::json!({"token":token.as_deref(),"manual":false});
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
	) -> Result<client_core::search::Outcome, Failure> {
		if !model::valid_search_query(query) {
			return Err(Failure::Protocol);
		}
		let (content, filters) = model::search_terms(query).map_err(|_| Failure::Protocol)?;
		// Encode values separately; user input cannot add arbitrary query parameters.
		let encoded: String = content.bytes().map(|b| format!("%{b:02X}")).collect();
		let mut path = match guild {
			Some(guild) => format!("/guilds/{guild}/messages/search?channel_id={channel}&"),
			None => format!("/channels/{channel}/messages/search?"),
		};
		path.push_str(&format!(
			"content={encoded}&limit=25&sort_by=timestamp&sort_order=desc"
		));
		let mut maximum = before.map(|id| id.0);
		for (key, value) in filters {
			if key == "max_id" {
				let id = value.parse::<u64>().map_err(|_| Failure::Protocol)?;
				maximum = Some(maximum.map_or(id, |current| current.min(id)));
			} else {
				let encoded: String = value.bytes().map(|b| format!("%{b:02X}")).collect();
				path.push_str(&format!("&{key}={encoded}"));
			}
		}
		if let Some(before) = maximum {
			path.push_str(&format!("&max_id={before}"));
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
			.into_page(channel, before)
			.map(client_core::search::Outcome::Page)
			.map_err(|_| Failure::Protocol)
	}
	/// Unofficial normal-client relay of KLIPY search/trending. Only the query text is encoded
	/// into a fixed route; previews stay static and are loaded by the credential-free worker.
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
				let bytes = self
					.request_limited(
						Method::GET,
						&format!("/gifs/trending?limit={}&{OPTIONS}", model::GIF_PAGE_SIZE),
						None,
						gifs::MAX_WIRE,
					)
					.await?;
				decode::<gifs::TrendingReply>(&bytes)
					.map_err(|_| Failure::Protocol)?
					.into_page()
					.map_err(|_| Failure::Protocol)
			}
		}
	}
	async fn send_message(
		&self,
		channel: model::Id,
		content: &str,
		nonce: &str,
		reply: Option<model::Id>,
		attachment: Option<Vec<serde_json::Value>>,
	) -> Result<model::Message, Failure> {
		if (content.trim().is_empty() && attachment.is_none())
			|| content.chars().count() > client_core::MAX_CONTENT
		{
			return Err(Failure::Capacity);
		}
		let mut body = serde_json::json!({"content":content,"nonce":nonce,"allowed_mentions":allowed_mentions(content)});
		if let Some(reply) = reply {
			body["message_reference"] =
				serde_json::json!({"message_id":reply,"channel_id":channel});
		}
		if let Some(attachment) = attachment {
			body["attachments"] = serde_json::json!(attachment);
		}
		// No enforce_nonce claim until normal-user semantics are live verified. Never auto-retry writes.
		self.request(
			Method::POST,
			&format!("/channels/{channel}/messages"),
			Some(body),
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
	async fn create_post(
		&self,
		parent: model::Id,
		guild: model::Id,
		title: &str,
		content: &str,
	) -> Result<model::Channel, Failure> {
		let title = title.trim();
		if title.is_empty()
			|| title.chars().count() > client_core::forum::MAX_TITLE
			|| content.trim().is_empty()
			|| content.chars().count() > client_core::MAX_CONTENT
		{
			return Err(Failure::Capacity);
		}
		let body = serde_json::json!({
			"name": title,
			"auto_archive_duration": 4320,
			"message": {"content": content, "allowed_mentions": allowed_mentions(content)},
		});
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
	async fn invite_captcha_preserves_fatal_auth_and_malformed_challenges() {
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
			invite_captcha(br#"{"captcha_service":"hcaptcha","captcha_sitekey":"bad/sitekey"}"#)
				.is_none()
		);
	}
	#[tokio::test]
	async fn invite_captcha_only_resumes_explicit_matching_write() {
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
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};
	#[tokio::test]
	async fn single_message_delete_confirms_only_success_and_never_retries_ambiguity() {
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
					assert!(request.starts_with("DELETE /channels/20/messages/100 HTTP/1.1\r\n"));
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
	#[tokio::test]
	async fn history_after_includes_zero_and_rejects_combined_cursors() {
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
                    ("/guilds/2/messages/search?channel_id=1&content=%78%26%23%3F%2E%2E&limit=25&sort_by=timestamp&sort_order=desc","200 OK",r#"{"messages":[[{"id":"9","channel_id":"1","author":{"id":"3","username":"Synthetic"},"content":"match"}]],"total_results":1}"#),
                    ("/channels/1/messages/search?content=%78&limit=25&sort_by=timestamp&sort_order=desc&max_id=9","200 OK",r#"{"messages":[],"total_results":0}"#),
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
            let Event::Search {channel:Id(1),request:8,result:Ok(Outcome::Page(page))}=api.execute(Command::Search {channel:Id(1),guild:Some(Id(2)),query:"x&#?..".into(),before:None,request:8}).await else {panic!()};
            assert_eq!(page.hits[0].id,Id(9));
            assert!(matches!(api.search(Id(1),None,"x",Some(Id(9))).await,Ok(Outcome::Page(p)) if p.hits.is_empty()));
            assert!(matches!(api.search(Id(1),None,"x",None).await,Err(Failure::Forbidden)));
            let Event::Search { channel:Id(1),request:9,result:Ok(Outcome::Pins(page)) } = api.execute(Command::Pins { channel:Id(1),before:None,request:9 }).await else {panic!()};
            assert!(page.hits[0].id == Id(9) && page.partial);
            let cursor = page.pin_cursor.unwrap();
            assert!(matches!(api.pins(Id(1),Some(cursor)).await,Ok(Outcome::Pins(p)) if p.hits[0].id == Id(99) && p.pin_cursor.is_none() && !p.partial));
            assert!(matches!(api.pins(Id(1),Some(cursor)).await,Err(Failure::Protocol)));
            assert!(matches!(api.pins(Id(1),Some(i128::MAX)).await,Err(Failure::Protocol)));
            assert!(matches!(api.pins(Id(1),None).await,Err(Failure::Forbidden)));
            assert!(matches!(api.search(Id(1),None,"x",None).await,Ok(Outcome::Indexing)));
            assert!(*api.cooldown.lock().await>Instant::now());
            server.await.unwrap();
        }).await.unwrap();
	}
	#[tokio::test]
	async fn read_ack_is_explicit_scoped_and_chains_only_session_tokens() {
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
								header.starts_with("POST /channels/1/messages/2/ack HTTP/1.1\r\n")
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
			for (request, expected) in [(1, Ok(())), (2, Ok(())), (3, Err(Failure::Ambiguous))] {
				let Event::ReadState(client_core::read_state::Event::Result {
					channel,
					message,
					request: actual,
					result,
				}) = api.execute(Command::MarkRead {
					channel: Id(1),
					message: Id(2),
					request,
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
	#[tokio::test]
	async fn reaction_routes_encode_one_component_and_read_back_scoped_counts() {
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
	#[tokio::test]
	async fn send_response_must_belong_to_the_requested_channel() {
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
						channel: model::Id(2),
						content: "Synthetic reply".into(),
						nonce: "local".into(),
						reply: Some(model::Id(50)),
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
			assert!(request.starts_with("GET /users/5/profile?with_mutual_guilds=true&with_mutual_friends=false&with_mutual_friends_count=false&guild_id=2 HTTP/1.1"));
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
				})
				.await
		});
		tokio::time::timeout(Duration::from_secs(2), started.notified())
			.await
			.unwrap();
		let sent = tokio::time::timeout(
			Duration::from_secs(2),
			api.execute(Command::Send {
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
		assert!(matches!(
			pending.await.unwrap(),
			Event::Profile {
				user: model::Id(5),
				guild: Some(model::Id(2)),
				request: 9,
				result: Ok(_)
			}
		));
		assert!(matches!(
			api.execute(Command::Profile {
				user: model::Id(5),
				guild: None,
				request: 10
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
				request: 11
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
				"429 Too Many Requests",
				"{\"retry_after\":0.1,\"global\":true}",
				Failure::RateLimited,
			),
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
		assert!(safe_delay(Some(f64::NAN)).is_err());
		assert!(safe_delay(Some(-1.0)).is_err());
	}
}

fn allowed_mentions(content: &str) -> serde_json::Value {
	let everyone: &[&str] = if model::has_mass_mention(content) {
		&["everyone"]
	} else {
		&[]
	};
	serde_json::json!({"parse":everyone,"users":model::mentioned_user_ids(content),"replied_user":false})
}
#[cfg(test)]
mod mention_tests {
	#[test]
	fn mass_mentions_and_explicit_users_are_allowed() {
		assert_eq!(
			super::allowed_mentions("hello test"),
			serde_json::json!({"parse":[],"users":[],"replied_user":false})
		);
		assert_eq!(
			super::allowed_mentions("@everyone <@&4> <@7> <@!7> <@9>"),
			serde_json::json!({"parse":["everyone"],"users":["7","9"],"replied_user":false})
		);
		assert_eq!(
			super::allowed_mentions("@here"),
			serde_json::json!({"parse":["everyone"],"users":[],"replied_user":false})
		);
	}
}
