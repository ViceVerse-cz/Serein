//! Bounded reactions to documented account-restriction error codes
//! (<https://docs.discord.com/developers/topics/opcodes-and-status-codes#json>). They hold
//! back DM creation or one conversation. Account-wide security refusals stop authenticated
//! traffic until explicit reconnection; nothing here retries a request.
use client_core::auth::Failure;
use std::{
	collections::VecDeque,
	hash::{Hash, Hasher},
	time::Duration,
};
use tokio::time::Instant;
use wreq::Method;

pub(crate) const OPENING_TOO_FAST: &str =
	"Discord says you are opening DMs too fast; wait a while before opening another";
pub(crate) const NOT_MESSAGEABLE: &str =
	"This person is not accepting messages from you; nothing was sent";

/// 40003 without a usable `retry_after`. A local, conservative choice; Discord documents none.
const OPENING_HOLD: Duration = Duration::from_secs(10 * 60);
/// The longest hold a service-supplied `retry_after` may start.
const MAX_OPENING_HOLD: Duration = Duration::from_secs(60 * 60);
const MAX_HELD_ROUTES: usize = 16;
const MAX_UNREACHABLE: usize = 64;
const OPEN_DM: &str = "/users/@me/channels";

#[derive(Default)]
pub(crate) struct Restrictions {
	account: Option<Failure>,
	/// Route class, the instant it frees, and the reason shown meanwhile.
	held: Vec<(u64, Instant, &'static str)>,
	/// Conversations that answered 50007, oldest first.
	unreachable: VecDeque<model::Id>,
}

/// One class of write: the method and the route with every id replaced.
fn route(method: &Method, path: &str) -> u64 {
	let mut hasher = std::collections::hash_map::DefaultHasher::new();
	method.hash(&mut hasher);
	let path = path.split_once('?').map_or(path, |(path, _)| path);
	for part in path.split('/') {
		if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
			part.hash(&mut hasher);
		}
	}
	hasher.finish()
}

/// The conversation a message create addresses.
fn message_channel(method: &Method, path: &str) -> Option<model::Id> {
	(method == Method::POST)
		.then(|| {
			path.strip_prefix("/channels/")?
				.strip_suffix("/messages")?
				.parse()
				.ok()
		})
		.flatten()
		.map(model::Id)
}

impl Restrictions {
	/// Refuses a write that an earlier answer put on hold, before any request is made.
	pub(crate) fn check(
		&mut self,
		method: &Method,
		path: &str,
		now: Instant,
	) -> Result<(), Failure> {
		if let Some(failure) = self.account {
			return Err(failure);
		}
		if method == Method::GET {
			return Ok(());
		}
		self.held.retain(|(_, until, _)| *until > now);
		let class = route(method, path);
		if let Some((_, _, reason)) = self.held.iter().find(|(held, ..)| *held == class) {
			return Err(Failure::ProtocolAt(reason));
		}
		if message_channel(method, path).is_some_and(|channel| self.unreachable.contains(&channel))
		{
			return Err(Failure::ProtocolAt(NOT_MESSAGEABLE));
		}
		Ok(())
	}

	fn hold(&mut self, class: u64, until: Instant, reason: &'static str) {
		self.held.retain(|(held, ..)| *held != class);
		if self.held.len() == MAX_HELD_ROUTES {
			self.held.remove(0);
		}
		self.held.push((class, until, reason));
	}

	/// Records a rejected write's error code and returns the failure to report for it.
	pub(crate) fn rejected(
		&mut self,
		method: &Method,
		path: &str,
		code: Option<u64>,
		retry_after: Option<f64>,
		now: Instant,
	) -> Option<Failure> {
		// Account-wide security refusals apply to reads and writes, without a timed resume.
		if matches!(code, Some(40002 | 40004)) {
			self.account = Some(Failure::AccountRestricted);
			return self.account;
		}
		if method == Method::GET {
			return None;
		}
		let reason = match code? {
			40003 => {
				let wait = retry_after
					.filter(|seconds| seconds.is_finite() && *seconds > 0.0)
					.map_or(OPENING_HOLD, |seconds| {
						Duration::from_secs_f64(seconds.min(MAX_OPENING_HOLD.as_secs_f64()))
					});
				self.hold(route(&Method::POST, OPEN_DM), now + wait, OPENING_TOO_FAST);
				OPENING_TOO_FAST
			}
			50007 => {
				if let Some(channel) = message_channel(method, path)
					&& !self.unreachable.contains(&channel)
				{
					if self.unreachable.len() == MAX_UNREACHABLE {
						self.unreachable.pop_front();
					}
					self.unreachable.push_back(channel);
				}
				NOT_MESSAGEABLE
			}
			_ => return None,
		};
		Some(Failure::ProtocolAt(reason))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::DiscordApi;
	use client_core::auth::SessionSecret;
	use std::sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	/// Answers each request with the next body as HTTP 403 and reports how many arrived.
	async fn serve(bodies: &'static [&'static str]) -> (DiscordApi, Arc<AtomicUsize>) {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_SECRET_MARKER".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let served = Arc::new(AtomicUsize::new(0));
		let count = served.clone();
		tokio::spawn(async move {
			for body in bodies {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut buffer = [0; 4096];
				let _ = socket.read(&mut buffer).await.unwrap();
				count.fetch_add(1, Ordering::SeqCst);
				socket
					.write_all(
						format!(
							"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
							body.len()
						)
						.as_bytes(),
					)
					.await
					.unwrap();
			}
		});
		(api, served)
	}
	async fn send(api: &DiscordApi, channel: u64) -> Option<Failure> {
		api.send_message(model::Id(channel), "synthetic", "n", None, None, None)
			.await
			.err()
	}

	#[tokio::test]
	async fn scoped_refusals_stay_local_and_account_verification_stops_all_routes() {
		// 40003: the second DM open is refused locally; only one request reaches the service.
		let (api, served) = serve(&[
			r#"{"message":"synthetic","code":40003,"retry_after":120.0}"#,
			r#"{"code":0}"#,
		])
		.await;
		for _ in 0..2 {
			assert_eq!(
				api.open_dm(model::Id(7), None, None).await.err(),
				Some(Failure::ProtocolAt(OPENING_TOO_FAST))
			);
		}
		assert_eq!(served.load(Ordering::SeqCst), 1);
		assert!(!api.stopped());

		// 50007 closes that conversation only; another conversation still sends.
		let (api, served) = serve(&[r#"{"code":50007}"#, r#"{"code":0}"#, r#"{"code":0}"#]).await;
		for _ in 0..2 {
			assert_eq!(
				send(&api, 5).await,
				Some(Failure::ProtocolAt(NOT_MESSAGEABLE))
			);
		}
		assert_eq!(send(&api, 6).await, Some(Failure::Forbidden));
		assert_eq!(served.load(Ordering::SeqCst), 2);

		// A malformed body is local; a real verification refusal latches the whole account.
		let (api, served) = serve(&[
			r#"{"code":"40002","retry_after":[]}"#,
			"not json",
			r#"{"code":40002}"#,
			r#"{"code":0}"#,
		])
		.await;
		for expected in [
			Failure::Forbidden,
			Failure::Forbidden,
			Failure::AccountRestricted,
			Failure::AccountRestricted,
		] {
			assert_eq!(send(&api, 5).await, Some(expected));
		}
		assert_eq!(served.load(Ordering::SeqCst), 3);
		assert!(api.stopped());
		assert_eq!(api.stop_reason(), Some(Failure::AccountRestricted));
		assert_eq!(api.terminal_failure().await, Failure::AccountRestricted);
		api.stop(); // Later generic teardown must not turn this into credential expiry.
		assert_eq!(api.stop_reason(), Some(Failure::AccountRestricted));
		assert_eq!(
			api.request(Method::GET, "/users/@me", None).await,
			Err(Failure::AccountRestricted)
		);
		assert_eq!(send(&api, 99).await, Some(Failure::AccountRestricted));
		assert_eq!(served.load(Ordering::SeqCst), 3);
	}

	#[tokio::test]
	async fn account_refusal_on_read_stops_writes_but_guild_ban_is_local() {
		let (api, served) =
			serve(&[r#"{"code":40007}"#, r#"{"code":40004}"#, r#"{"code":0}"#]).await;
		assert_eq!(send(&api, 5).await, Some(Failure::Forbidden));
		assert!(!api.stopped());
		let (terminal, read) = tokio::join!(
			api.terminal_failure(),
			api.request(Method::GET, "/users/@me", None)
		);
		assert_eq!(terminal, Failure::AccountRestricted);
		assert_eq!(read, Err(Failure::AccountRestricted));
		assert_eq!(send(&api, 6).await, Some(Failure::AccountRestricted));
		assert_eq!(served.load(Ordering::SeqCst), 2);
	}

	#[tokio::test]
	async fn revoked_link_does_not_expire_the_discord_session() {
		let (api, served) = serve(&[r#"{"code":40012}"#, r#"{"code":0}"#]).await;
		let result = api
			.request(
				Method::GET,
				"/users/@me/connections/spotify/7/access-token",
				None,
			)
			.await;
		assert!(matches!(result, Err(Failure::ProtocolAt(_))));
		assert!(!api.stopped());
		assert_eq!(send(&api, 6).await, Some(Failure::Forbidden));
		assert_eq!(served.load(Ordering::SeqCst), 2);
	}

	#[test]
	fn holds_are_bounded_and_expire() {
		let mut restrictions = Restrictions::default();
		let now = Instant::now();
		for channel in 0..200u64 {
			let path = format!("/channels/{channel}/messages");
			restrictions.rejected(&Method::POST, &path, Some(50007), None, now);
			restrictions.rejected(
				&Method::PUT,
				&format!("/synthetic{channel}/1"),
				Some(40003),
				None,
				now,
			);
		}
		assert_eq!(restrictions.unreachable.len(), MAX_UNREACHABLE);
		assert_eq!(restrictions.held.len(), 1);
		// A hostile wait is capped, and a hold frees its route once it has passed.
		restrictions.rejected(&Method::POST, OPEN_DM, Some(40003), Some(f64::MAX), now);
		assert!(
			restrictions
				.check(&Method::POST, OPEN_DM, now + MAX_OPENING_HOLD / 2)
				.is_err()
		);
		assert!(
			restrictions
				.check(&Method::POST, OPEN_DM, now + MAX_OPENING_HOLD)
				.is_ok()
		);
		assert!(restrictions.check(&Method::GET, OPEN_DM, now).is_ok());
	}
}
