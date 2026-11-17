//! REST admission, response limits, cooldown and protected-request handling.
use super::{
	ADMIN_READ, AUTHORIZATION, CONTENT_TYPE, Client, DiscordApi, FOREGROUND_PERMITS, Failure,
	HeaderValue, Instant, MAX_RATE_LIMIT_WAIT, Method, Ordering, READ_RATE_LIMIT_RETRIES,
	RequestContent, ResponsePolicy, SPECULATIVE, StatusCode, api_client, invite_captcha, mfa,
	proxy, safe_delay, sleep_until,
};
#[cfg(not(test))]
use super::{API_ON_TEST_HOST, API_ORIGINS, api_v10_base};
use discord_protocol::{ErrorBody, MAX_WIRE, decode};

impl DiscordApi {
	async fn request_permit<'a>(
		&self,
		semaphore: &'a tokio::sync::Semaphore,
		deadline: Option<Instant>,
	) -> Result<tokio::sync::SemaphorePermit<'a>, Failure> {
		let acquire = async {
			semaphore.acquire().await.map_err(|_| {
				if self.stopped() {
					Failure::Expired
				} else {
					Failure::Network
				}
			})
		};
		match deadline {
			Some(deadline) => tokio::time::timeout_at(deadline, acquire)
				.await
				.map_err(|_| {
					Failure::ProtocolAt("Verification expired before the request could resume")
				})?,
			None => acquire.await,
		}
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
	pub(super) async fn request(
		&self,
		method: Method,
		path: &str,
		body: Option<serde_json::Value>,
	) -> Result<Vec<u8>, Failure> {
		self.request_limited(method, path, body, MAX_WIRE).await
	}
	pub(super) async fn request_limited(
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
			ResponsePolicy::required(max_bytes),
			None,
			None,
		)
		.await
	}
	/// Keep a missing optional GET distinct from malformed responses and other HTTP failures.
	pub(super) async fn request_optional_limited(
		&self,
		path: &str,
		max_bytes: usize,
	) -> Result<Option<Vec<u8>>, Failure> {
		let mut missing = false;
		let bytes = self
			.request_with_content(
				Method::GET,
				path,
				None,
				ResponsePolicy {
					max_bytes,
					missing: Some(&mut missing),
				},
				None,
				None,
			)
			.await?;
		Ok((!missing).then_some(bytes))
	}
	pub(super) async fn request_multipart_limited(
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
			ResponsePolicy::required(max_bytes),
			None,
			None,
		)
		.await
	}
	/// One typed request with an optional captcha retry and challenge output slot.
	pub(super) async fn request_with_captcha(
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
			ResponsePolicy::required(max_bytes),
			retry,
			challenge,
		)
		.await
	}
	/// The REST origin for the next attempt and whether it is still discord.com.
	#[cfg(not(test))]
	pub(super) fn api_origin(&self) -> (&'static str, bool) {
		let base = api_v10_base();
		(base, base == API_ORIGINS[0])
	}
	/// Without a stand-in test host, a test behaves as if the process already moved.
	#[cfg(test)]
	pub(super) fn api_origin(&self) -> (&str, bool) {
		match &self.test_host {
			Some((origin, moved)) if moved.load(Ordering::Acquire) => (origin, false),
			Some(_) => (&self.base, true),
			None => (&self.base, false),
		}
	}
	pub(super) fn move_to_test_host(&self) {
		#[cfg(not(test))]
		API_ON_TEST_HOST.store(true, Ordering::Release);
		#[cfg(test)]
		if let Some((_, moved)) = &self.test_host {
			moved.store(true, Ordering::Release);
		}
	}
	pub(super) async fn request_with_content(
		&self,
		method: Method,
		path: &str,
		body: Option<RequestContent>,
		response: ResponsePolicy<'_>,
		retry: Option<&client_core::captcha::Retry>,
		challenge: Option<&mut Option<client_core::captcha::Challenge>>,
	) -> Result<Vec<u8>, Failure> {
		self.request_with_reason(method, path, body, response, (retry, challenge), None)
			.await
	}
	pub(super) async fn request_reason_limited(
		&self,
		method: Method,
		path: &str,
		body: Option<serde_json::Value>,
		max_bytes: usize,
		reason: &str,
	) -> Result<Vec<u8>, Failure> {
		if !model::automod::valid_reason(reason) {
			return Err(Failure::Protocol);
		}
		self.request_with_reason(
			method,
			path,
			body.map(RequestContent::Json),
			ResponsePolicy::required(max_bytes),
			(None, None),
			Some(reason),
		)
		.await
	}
	pub(super) async fn request_with_reason(
		&self,
		method: Method,
		path: &str,
		body: Option<RequestContent>,
		response: ResponsePolicy<'_>,
		captcha: (
			Option<&client_core::captcha::Retry>,
			Option<&mut Option<client_core::captcha::Challenge>>,
		),
		reason: Option<&str>,
	) -> Result<Vec<u8>, Failure> {
		let (retry, mut challenge) = captcha;
		let ResponsePolicy {
			max_bytes,
			mut missing,
		} = response;
		// Only typed adapter methods construct paths. Never accept a URL or route from UI/content.
		if !path.starts_with('/')
			|| path.contains("://")
			|| path.contains('\\')
			|| path.contains("..")
		{
			return Err(Failure::Protocol);
		}
		let speculative = SPECULATIVE.try_with(|()| ()).is_ok();
		let idempotent = method == Method::GET && body.is_none() && !speculative;
		let mut body = body;
		let route = super::rate_limits::Route::new(&method, path);
		let mut rate_limits = 0u8;
		let mut admission_deadline = None;
		let mut mfa_replay: Option<reqwest::RequestBuilder> = None;
		let mut mfa_resumed = false;
		let mut mfa_deadline: Option<Instant> = None;
		loop {
			// Proxy setup may pause: resolve it before checking cooldown and session admission.
			let client = if let Some(deadline) = mfa_deadline {
				tokio::time::timeout_at(deadline, self.rest_client())
					.await
					.map_err(|_| {
						Failure::ProtocolAt("Verification expired before the request could resume")
					})??
			} else {
				self.rest_client().await?
			};
			let rate_deadline =
				*admission_deadline.get_or_insert_with(|| Instant::now() + MAX_RATE_LIMIT_WAIT);
			loop {
				let stopped = self.stop_signal.notified();
				tokio::pin!(stopped);
				stopped.as_mut().enable();

				if self.stopped() {
					return Err(Failure::Expired);
				}
				let next = self.cooldown.lock().await.deadline(&route)?;
				if Instant::now() >= next {
					break;
				}
				if speculative {
					return Err(Failure::Capacity);
				}
				if next > rate_deadline {
					return Err(Failure::RateLimited);
				}
				if let Some(deadline) = mfa_deadline {
					tokio::select! { _=&mut stopped=>return Err(Failure::Expired), _=sleep_until(deadline)=>return Err(Failure::ProtocolAt("Verification expired before the request could resume")), _=sleep_until(next)=>{} }
				} else {
					tokio::select! { _=&mut stopped=>return Err(Failure::Expired), _=sleep_until(next)=>{} }
				}
			}
			// Acquire at the HTTP boundary only: nested adapter methods cannot hold a permit
			// while awaiting another permit. Review section futures share this two-read ceiling.
			let _admin_read = if ADMIN_READ.try_with(|()| ()).is_ok() {
				Some(self.request_permit(&self.admin_reads, mfa_deadline).await?)
			} else {
				None
			};
			let _permit = if speculative {
				// A hover preload takes only spare capacity, so it never delays a click.
				if self.requests.available_permits() <= FOREGROUND_PERMITS {
					return Err(Failure::Capacity);
				}
				self.requests.try_acquire().map_err(|_| Failure::Capacity)?
			} else {
				self.request_permit(&self.requests, mfa_deadline).await?
			};
			// Four permits bound concurrent REST work. A slow profile body must not hold the
			// cooldown mutex and delay a message write; only service rate admission is shared.

			if Instant::now() < self.cooldown.lock().await.deadline(&route)? {
				continue;
			}
			if self.stopped() {
				return Err(Failure::Expired);
			}
			if mfa_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
				return Err(Failure::ProtocolAt(
					"Verification expired before the request could resume",
				));
			}
			admission_deadline = None;
			let mut authorization = HeaderValue::from_str(self.secret.expose())
				.map_err(|_| Failure::InvalidCredential)?;
			authorization.set_sensitive(true);
			let (base, on_stable) = self.api_origin();
			let write = method != Method::GET;
			let replaying = mfa_replay.is_some();
			let mut request = mfa_replay.take().unwrap_or_else(|| {
				client
					.request(method.clone(), format!("{base}{path}"))
					.header(AUTHORIZATION, authorization)
			});
			if !replaying && let Some(reason) = reason.filter(|reason| !reason.is_empty()) {
				let encoded: String = reason.bytes().map(|byte| format!("%{byte:02X}")).collect();
				let mut value = HeaderValue::from_str(&encoded).map_err(|_| Failure::Protocol)?;
				value.set_sensitive(true);
				request = request.header("x-audit-log-reason", value);
			}
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
			if !replaying && let Some(content) = body.take() {
				request = match content {
					RequestContent::Json(body) => request.json(&body),
					RequestContent::SecretJson(body) => request
						.header(CONTENT_TYPE, "application/json")
						.body(body.to_vec()),
					RequestContent::Multipart { content_type, body } => request
						.header(
							CONTENT_TYPE,
							HeaderValue::from_str(&content_type).map_err(|_| Failure::Protocol)?,
						)
						.body(body),
				};
			}
			let mfa_target = write
				.then(|| mfa::OPERATION.try_with(Clone::clone).ok())
				.flatten();
			let exact_retry = (!mfa_resumed && mfa_target.is_some())
				.then(|| request.try_clone())
				.flatten();
			// A suspended builder retains its original client. Execute its immutable
			// request through the currently approved route after proxy reconfiguration.
			let result = match request.build() {
				Ok(request) => client.execute(request).await,
				Err(error) => Err(error),
			};
			let mut response = result.map_err(|_| {
				if write {
					Failure::Ambiguous
				} else {
					Failure::Network
				}
			})?;
			let status = response.status();
			let body_limit = if status.is_success() {
				max_bytes
			} else {
				mfa::MAX_WIRE
			};
			let uncertain_write = write && (status.is_success() || status.is_server_error());
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
			let bucket = response
				.headers()
				.get("x-ratelimit-bucket")
				.and_then(|v| v.to_str().ok())
				.filter(|v| v.len() <= 128)
				.map(str::to_owned);
			let global_header = response
				.headers()
				.get("x-ratelimit-global")
				.is_some_and(|v| v == "true")
				|| response
					.headers()
					.get("x-ratelimit-scope")
					.is_some_and(|v| v == "global");
			let retry_header = response
				.headers()
				.get("retry-after")
				.and_then(|v| v.to_str().ok())
				.and_then(|s| s.parse::<f64>().ok());
			let cloudflare_block = response
				.headers()
				.get(CONTENT_TYPE)
				.and_then(|value| value.to_str().ok())
				.is_some_and(|value| {
					value.starts_with("text/html") || value.starts_with("text/plain")
				});
			// Accepted or server-failed writes may already have taken effect.
			let overflow = if uncertain_write {
				Failure::Ambiguous
			} else {
				Failure::ResponseLimit
			};
			if response
				.content_length()
				.is_some_and(|n| n > body_limit as u64)
			{
				if status == StatusCode::UNAUTHORIZED {
					self.stop();
					return Err(Failure::Expired);
				}
				return Err(overflow);
			}
			let declared_overflow = response
				.content_length()
				.is_some_and(|n| n > max_bytes as u64);
			let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(
				response
					.content_length()
					.unwrap_or(0)
					.min(body_limit as u64) as usize,
			));
			while let Some(chunk) = response.chunk().await.map_err(|_| {
				if status == StatusCode::UNAUTHORIZED {
					self.stop();
					return Failure::Expired;
				}
				if declared_overflow {
					overflow
				} else if write {
					Failure::Ambiguous
				} else {
					Failure::Network
				}
			})? {
				if bytes.len() + chunk.len() > body_limit {
					if status == StatusCode::UNAUTHORIZED {
						self.stop();
						return Err(Failure::Expired);
					}
					return Err(overflow);
				}
				bytes.extend_from_slice(&chunk);
			}
			let error =
				(!status.is_success()).then(|| decode::<ErrorBody>(&bytes).unwrap_or_default());
			if let Some(error) = &error {
				if error.code == Some(60003)
					&& !mfa_resumed && let (Some(target), Some(replay), Some(challenge)) =
					(mfa_target, exact_retry, mfa::challenge(&bytes))
				{
					drop(_permit);
					drop(_admin_read);
					let proof = self.wait_for_mfa(target, challenge).await?;
					if self.stopped() {
						return Err(Failure::Expired);
					}
					mfa_deadline = Some(Instant::now() + proof.remaining());
					mfa_replay =
						Some(replay.header("x-discord-mfa-authorization", proof.header()?));
					mfa_resumed = true;
					continue;
				}
				if error.code == Some(60003) && mfa_resumed {
					return Err(Failure::ProtocolAt(
						"Discord did not accept verification; review the operation before trying again",
					));
				}
				// A well-formed action challenge is not evidence that the account was revoked.
				// Unsupported command families keep the session usable and hand off explicitly.
				if error.code == Some(60003) && mfa::challenge(&bytes).is_some() {
					return Err(Failure::ProtocolAt(
						"This action requires verification in the official Discord client",
					));
				}
				if status == StatusCode::UNAUTHORIZED {
					if matches!(path, "/mfa/finish" | "/auth/mfa/sms/send")
						&& matches!(error.code, Some(60006 | 60008 | 60009))
					{
						return Err(Failure::ProtocolAt(
							"Verification was rejected; check the code and try again",
						));
					}
					self.stop();
					return Err(Failure::Expired);
				}
			}
			// Error bodies are read up to the small MFA envelope limit only to classify a
			// genuine authentication challenge. Other replies retain the caller's limit.
			if bytes.len() > max_bytes {
				return Err(overflow);
			}
			if status != StatusCode::TOO_MANY_REQUESTS {
				let delay = if exhausted {
					Some(safe_delay(reset.or(retry_header)).map_err(|f| {
						if uncertain_write {
							Failure::Ambiguous
						} else {
							f
						}
					})?)
				} else {
					None
				};
				// Metadata capacity cannot turn an accepted write into a definite failure.
				self.cooldown
					.lock()
					.await
					.observe(&route, bucket.as_deref(), delay, false)
					.map_err(|f| {
						if uncertain_write {
							Failure::Ambiguous
						} else {
							f
						}
					})?;
			}
			if let Some(error) = error {
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
					self.cooldown
						.lock()
						.await
						.observe(&route, None, Some(delay), true)?;
					return Err(Failure::RateLimited);
				}
				if status == StatusCode::TOO_MANY_REQUESTS {
					// The body is the penalty Discord documents. A missing or shorter reset
					// header must not send the retry early.
					let delay = safe_delay(error.retry_after.or(retry_header).or(reset))?;
					{
						self.cooldown.lock().await.observe(
							&route,
							bucket.as_deref(),
							Some(delay),
							error.global || global_header,
						)?;
					}
					if idempotent
						&& delay <= MAX_RATE_LIMIT_WAIT
						&& rate_limits < READ_RATE_LIMIT_RETRIES
					{
						rate_limits += 1;
						continue;
					}
					return Err(Failure::RateLimited);
				}
				if !write
					&& status == StatusCode::NOT_FOUND
					&& let Some(missing) = missing.as_deref_mut()
				{
					*missing = true;
					return Ok(Vec::new());
				}
				// Ordinary requests convert only missing private notes.
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
				} else if status.is_server_error() {
					Failure::Network
				} else {
					Failure::Protocol
				});
			}
			return Ok(std::mem::take(&mut *bytes));
		}
	}
}
