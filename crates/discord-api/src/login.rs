//! Native sign-in (unofficial, live-unverified): the unauthenticated REST calls Discord's own
//! login page makes, sent with the client identity every later request uses, so the session
//! is issued to the client that then presents it. Wire shapes follow SakuraCord's
//! `DiscordSessionAuthenticator` (commit aa8a3da8) and discord-userdoccers.
//!
//! Every call is one attempt: nothing is retried, and a challenge is only ever answered by
//! the user. Passwords, codes, tickets and tokens are zeroized where this crate owns them
//! and never logged; failures are fixed labels, never remote text.
use super::*;
use client_core::captcha::{Challenge, Solved};
use std::fmt;
use zeroize::Zeroizing;

/// Login answers are small; the cap also bounds a hostile or broken response.
const MAX_BYTES: usize = 64 * 1024;
const LOGIN_PAGE: &str = "https://discord.com/login";
/// A ticket or encrypted token longer than this is not one Discord issued.
const MAX_TICKET: usize = 4096;
const ROUTE_WAIT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginError {
	InvalidLogin,
	/// Discord emailed a link that approves this location; signing in works after it is opened.
	NewLocation,
	PasswordReset,
	InvalidCode,
	/// The MFA ticket ran out; the password step has to be repeated.
	TicketExpired,
	/// A second challenge followed a solved one. It is not solved again.
	CaptchaRejected,
	UnsupportedMfa,
	/// Discord signed the account in but wants a step first, such as a new password.
	RequiredAction,
	UnsupportedCaptcha,
	Restricted,
	Blocked,
	/// Seconds Discord asked to wait.
	RateLimited(u32),
	Network,
	/// The configured proxy route was not ready in time.
	NoRoute,
	Rejected(u16),
	Protocol,
}
impl fmt::Display for LoginError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::InvalidLogin => "Login or password is invalid.",
			Self::NewLocation => {
				"New login location detected. Open the link Discord emailed you, then log in again."
			}
			Self::PasswordReset => {
				"Discord requires a password reset for this account. Reset it at discord.com, then log in again."
			}
			Self::InvalidCode => "That code was not accepted. Check it and try again.",
			Self::TicketExpired => "This sign-in took too long. Enter your password again.",
			Self::CaptchaRejected => {
				"Discord asked for another verification after the first one. Wait a moment and try again."
			}
			Self::UnsupportedMfa => {
				"This account only allows a security key or passkey here. Use the QR code or Discord's web page instead."
			}
			Self::RequiredAction => {
				"Discord wants an account step first, such as a new password. Finish it on Discord's web page, then log in again."
			}
			Self::UnsupportedCaptcha => {
				"Discord asked for a verification Serein cannot show. Use Discord's web page instead."
			}
			Self::Restricted => {
				"Discord stopped this sign-in for account reasons. Check the official client."
			}
			Self::Blocked => "Discord blocked this request. Wait before trying again.",
			Self::RateLimited(seconds) => {
				return write!(
					f,
					"Discord is rate limiting sign-in. Wait {seconds} seconds before trying again."
				);
			}
			Self::Network => "Could not reach Discord. Check your connection and try again.",
			Self::NoRoute => "Serein's network route is not ready. Try again in a moment.",
			Self::Rejected(status) => {
				return write!(f, "Discord answered sign-in with HTTP {status}.");
			}
			Self::Protocol => "Discord sent a sign-in response Serein does not understand.",
		})
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MfaMethod {
	Totp,
	Sms,
	Backup,
}
impl MfaMethod {
	fn route(self) -> &'static str {
		match self {
			Self::Totp => "/auth/mfa/totp",
			Self::Sms => "/auth/mfa/sms",
			Self::Backup => "/auth/mfa/backup",
		}
	}
	/// The code as Discord's own form submits it; `None` when it cannot be a valid one.
	fn normalize(self, code: &str) -> Option<Zeroizing<String>> {
		let code = Zeroizing::new(match self {
			Self::Totp | Self::Sms => code
				.chars()
				.filter(char::is_ascii_digit)
				.collect::<String>(),
			Self::Backup => code
				.chars()
				.filter(char::is_ascii_alphanumeric)
				.map(|c| c.to_ascii_lowercase())
				.collect(),
		});
		let length = if self == Self::Backup { 8 } else { 6 };
		(code.len() == length).then_some(code)
	}
}

/// The second step Discord asked for after a correct password.
pub struct MfaChallenge {
	ticket: Zeroizing<String>,
	instance: Option<String>,
	pub totp: bool,
	pub sms: bool,
	pub backup: bool,
}
impl MfaChallenge {
	/// A challenge no service issued, for offline previews of the code step.
	pub fn fixture() -> Self {
		Self {
			ticket: Zeroizing::new("synthetic-fixture-ticket".to_owned()),
			instance: None,
			totp: true,
			sms: true,
			backup: true,
		}
	}
	pub fn allows(&self, method: MfaMethod) -> bool {
		match method {
			MfaMethod::Totp => self.totp,
			MfaMethod::Sms => self.sms,
			MfaMethod::Backup => self.backup,
		}
	}
}
impl fmt::Debug for MfaChallenge {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("MfaChallenge([REDACTED])")
	}
}

#[derive(Debug)]
pub enum Step {
	Token(SessionSecret),
	Mfa(MfaChallenge),
	Captcha(Challenge),
}

/// A QR ticket exchange: the token still encrypted to the QR session's key, or a challenge.
pub enum Exchange {
	Encrypted(Zeroizing<String>),
	Captcha(Challenge),
}

struct Answer {
	status: StatusCode,
	bytes: Zeroizing<Vec<u8>>,
	retry_after: Option<f64>,
	/// Cloudflare answers a blocked client with a page, not the API's JSON.
	page: bool,
}

pub struct LoginApi {
	client: Client,
	proxy: Mutex<ProxyState>,
	/// Discord's pre-login fingerprint, asked for once per sign-in screen. `Some(None)` after
	/// an attempt that got none, so a failure is not asked for again on every request.
	fingerprint: Mutex<Option<Option<HeaderValue>>>,
	#[cfg(test)]
	base: String,
}
impl LoginApi {
	/// None waits for extension configuration, as authenticated REST does.
	pub fn new(
		receiver: tokio::sync::watch::Receiver<Option<proxy::ApiProxy>>,
	) -> Result<Self, Failure> {
		Ok(Self {
			client: api_client(&proxy::ApiProxy::Direct)?,
			proxy: Mutex::new(ProxyState {
				receiver,
				cached: None,
			}),
			fingerprint: Mutex::new(None),
			#[cfg(test)]
			base: API_ORIGIN.into(),
		})
	}
	fn base(&self) -> &str {
		#[cfg(not(test))]
		return API_ORIGIN;
		#[cfg(test)]
		return self.base.as_str();
	}
	async fn client(&self) -> Result<Client, LoginError> {
		// A proxy extension that never reports a route must not leave the form waiting.
		let route = async { self.proxy.lock().await.client(&self.client).await };
		tokio::time::timeout(ROUTE_WAIT, route)
			.await
			.map_err(|_| LoginError::NoRoute)?
			.map_err(|_| LoginError::NoRoute)
	}
	/// What the login page asks before its first submit (observed in the desktop client on
	/// 2026-11-19): the experiments, whose answer carries the fingerprint later requests send,
	/// then the location metadata. The fingerprint is issued by Discord and never made up;
	/// sign-in proceeds without it. The page asks for no installation ID.
	async fn prepare(&self, client: &Client) -> Option<HeaderValue> {
		let mut slot = self.fingerprint.lock().await;
		if let Some(known) = &*slot {
			return known.clone();
		}
		// The page itself loads first, and names the web build these requests report.
		let route = self.proxy.lock().await.receiver.borrow().clone();
		if let Some(route) = route {
			crate::read_build_number(&route).await;
		}
		#[derive(serde::Deserialize)]
		struct Experiments {
			fingerprint: String,
		}
		let fetch = async {
			let request = self
				.request(
					client,
					Method::GET,
					"/experiments?with_guild_experiments=true",
					None,
				)
				.header(
					"x-context-properties",
					// base64 of {"location":"/login"}: the page the request came from.
					HeaderValue::from_static("eyJsb2NhdGlvbiI6Ii9sb2dpbiJ9"),
				);
			let response = crate::current_identity(request).send().await.ok()?;
			if !response.status().is_success() {
				return None;
			}
			// The body lists every experiment; only the fingerprint is kept.
			let bytes = read(response, MAX_WIRE).await?;
			let fingerprint = decode::<Experiments>(&bytes).ok()?.fingerprint;
			let valid = (1..=256).contains(&fingerprint.len())
				&& fingerprint.bytes().all(|b| b.is_ascii_graphic());
			valid.then(|| HeaderValue::from_str(&fingerprint).ok())?
		};
		let fingerprint = tokio::time::timeout(Duration::from_secs(10), fetch)
			.await
			.ok()
			.flatten();
		// Read and dropped: the page uses it for its consent notices, which Serein has none of.
		let metadata = async {
			let request = self.request(
				client,
				Method::GET,
				"/auth/location-metadata",
				fingerprint.as_ref(),
			);
			let response = crate::current_identity(request).send().await.ok()?;
			read(response, MAX_BYTES).await
		};
		let _ = tokio::time::timeout(Duration::from_secs(5), metadata).await;
		*slot = Some(fingerprint.clone());
		fingerprint
	}
	/// Makes the login page's opening requests now, as the page does when it loads, so the
	/// first submit or QR exchange does not wait on them.
	pub async fn open_page(&self) {
		if let Ok(client) = self.client().await {
			self.prepare(&client).await;
		}
	}
	fn request(
		&self,
		client: &Client,
		method: Method,
		path: &str,
		fingerprint: Option<&HeaderValue>,
	) -> wreq::RequestBuilder {
		let write = method != Method::GET;
		let mut request = client
			.request(method, format!("{}{path}", self.base()))
			.header(wreq::header::REFERER, LOGIN_PAGE);
		if write {
			request = request.header(wreq::header::ORIGIN, "https://discord.com");
		}
		if let Some(fingerprint) = fingerprint {
			request = request.header("x-fingerprint", fingerprint.clone());
		}
		if let Some(installation) = client_core::fingerprint::installation_id()
			&& let Ok(value) = HeaderValue::from_str(&installation)
		{
			request = request.header("x-installation-id", value);
		}
		request
	}
	async fn post(
		&self,
		path: &str,
		body: &impl serde::Serialize,
		solved: Option<&Solved>,
	) -> Result<Answer, LoginError> {
		if solved.is_some_and(Solved::expired) {
			return Err(LoginError::CaptchaRejected);
		}
		let client = self.client().await?;
		let fingerprint = self.prepare(&client).await;
		let body = Zeroizing::new(serde_json::to_vec(body).map_err(|_| LoginError::Protocol)?);
		let mut request = self
			.request(&client, Method::POST, path, fingerprint.as_ref())
			.header(CONTENT_TYPE, "application/json")
			// The transport's copy is wiped with the request, not just freed.
			.body(bytes::Bytes::from_owner(body));
		if let Some(solved) = solved {
			for (name, value) in [
				("x-captcha-key", Some(solved.passcode())),
				("x-captcha-rqtoken", solved.rqtoken()),
				("x-captcha-session-id", solved.session_id()),
			] {
				if let Some(value) = value {
					let mut value =
						HeaderValue::from_str(value).map_err(|_| LoginError::Protocol)?;
					value.set_sensitive(true);
					request = request.header(name, value);
				}
			}
		}
		let response = crate::current_identity(request)
			.send()
			.await
			.map_err(|_| LoginError::Network)?;
		let status = response.status();
		let header = |name: &str| {
			response
				.headers()
				.get(name)
				.and_then(|v| v.to_str().ok())
				.and_then(|s| s.parse::<f64>().ok())
		};
		let retry_after = header("retry-after").or_else(|| header("x-ratelimit-reset-after"));
		let page = response
			.headers()
			.get(CONTENT_TYPE)
			.and_then(|value| value.to_str().ok())
			.is_some_and(|value| value.starts_with("text/html") || value.starts_with("text/plain"));
		let bytes = match read(response, MAX_BYTES).await {
			Some(bytes) => Zeroizing::new(bytes),
			// The wait a rate limit asks for is in its status and headers, body or not.
			None if status == StatusCode::TOO_MANY_REQUESTS => Zeroizing::default(),
			None => return Err(LoginError::Protocol),
		};
		Ok(Answer {
			status,
			bytes,
			retry_after,
			page,
		})
	}
	/// Submits the login form once. `solved` replays it with the challenge the user completed.
	pub async fn login(
		&self,
		login: &str,
		password: &str,
		solved: Option<&Solved>,
	) -> Result<Step, LoginError> {
		#[derive(serde::Serialize)]
		struct Form<'a> {
			login: &'a str,
			password: &'a str,
			undelete: bool,
			login_source: Option<()>,
			gift_code_sku_id: Option<()>,
		}
		let answer = self
			.post(
				"/auth/login",
				&Form {
					login,
					password,
					undelete: false,
					login_source: None,
					gift_code_sku_id: None,
				},
				solved,
			)
			.await?;
		if let Some(challenge) = captcha(&answer) {
			return if solved.is_some() {
				Err(LoginError::CaptchaRejected)
			} else {
				Ok(Step::Captcha(challenge?))
			};
		}
		granted(&answer)
	}
	/// Submits one MFA code for the ticket the password step returned.
	pub async fn mfa(
		&self,
		challenge: &MfaChallenge,
		method: MfaMethod,
		code: &str,
	) -> Result<SessionSecret, LoginError> {
		#[derive(serde::Serialize)]
		struct Form<'a> {
			code: &'a str,
			ticket: &'a str,
			#[serde(skip_serializing_if = "Option::is_none")]
			login_instance_id: Option<&'a str>,
			login_source: Option<()>,
			gift_code_sku_id: Option<()>,
		}
		if !challenge.allows(method) {
			return Err(LoginError::UnsupportedMfa);
		}
		let code = method.normalize(code).ok_or(LoginError::InvalidCode)?;
		let answer = self
			.post(
				method.route(),
				&Form {
					code: &code,
					ticket: &challenge.ticket,
					login_instance_id: challenge.instance.as_deref(),
					login_source: None,
					gift_code_sku_id: None,
				},
				None,
			)
			.await?;
		match granted(&answer)? {
			Step::Token(secret) => Ok(secret),
			Step::Mfa(_) | Step::Captcha(_) => Err(LoginError::Protocol),
		}
	}
	/// Asks Discord to text the account's phone a code for this ticket.
	pub async fn send_sms(&self, challenge: &MfaChallenge) -> Result<(), LoginError> {
		#[derive(serde::Serialize)]
		struct Form<'a> {
			ticket: &'a str,
		}
		if !challenge.sms {
			return Err(LoginError::UnsupportedMfa);
		}
		let answer = self
			.post(
				"/auth/mfa/sms/send",
				&Form {
					ticket: &challenge.ticket,
				},
				None,
			)
			.await?;
		rejected(&answer)
	}
	/// Trades a ticket the phone approved for the token, still encrypted to the QR session's key.
	pub async fn remote_auth(
		&self,
		ticket: &str,
		solved: Option<&Solved>,
	) -> Result<Exchange, LoginError> {
		#[derive(serde::Serialize)]
		struct Form<'a> {
			ticket: &'a str,
		}
		#[derive(serde::Deserialize)]
		struct Granted {
			encrypted_token: String,
		}
		let answer = self
			.post("/users/@me/remote-auth/login", &Form { ticket }, solved)
			.await?;
		if let Some(challenge) = captcha(&answer) {
			return if solved.is_some() {
				Err(LoginError::CaptchaRejected)
			} else {
				Ok(Exchange::Captcha(challenge?))
			};
		}
		rejected(&answer)?;
		let token = Zeroizing::new(
			decode::<Granted>(&answer.bytes)
				.map_err(|_| LoginError::Protocol)?
				.encrypted_token,
		);
		if token.is_empty() || token.len() > MAX_TICKET {
			return Err(LoginError::Protocol);
		}
		Ok(Exchange::Encrypted(token))
	}
}

/// Reads a body of at most `max` bytes; `None` when it is longer or the read fails.
async fn read(response: wreq::Response, max: usize) -> Option<Vec<u8>> {
	if response.content_length().is_some_and(|n| n > max as u64) {
		return None;
	}
	let mut bytes = Zeroizing::new(Vec::new());
	let mut body = std::pin::pin!(response.bytes_stream());
	while let Some(chunk) = futures_util::StreamExt::next(&mut body).await {
		let chunk = chunk.ok()?;
		if bytes.len() + chunk.len() > max {
			return None;
		}
		bytes.extend_from_slice(&chunk);
	}
	Some(std::mem::take(&mut *bytes))
}

/// A challenge the user can solve, when the answer asks for one. `Some(Err)` is a challenge
/// this client cannot show. A rate limit is never answered with a challenge.
fn captcha(answer: &Answer) -> Option<Result<Challenge, LoginError>> {
	#[derive(serde::Deserialize)]
	struct Asked {
		captcha_key: Option<serde::de::IgnoredAny>,
		captcha_sitekey: Option<serde::de::IgnoredAny>,
	}
	if answer.status.is_success() || answer.page || answer.status == StatusCode::TOO_MANY_REQUESTS {
		return None;
	}
	if let Some(challenge) = invite_captcha(&answer.bytes) {
		return Some(Ok(challenge));
	}
	let asked = decode::<Asked>(&answer.bytes).ok()?;
	(asked.captcha_key.is_some() || asked.captcha_sitekey.is_some())
		.then_some(Err(LoginError::UnsupportedCaptcha))
}

/// Maps a refused answer to its fixed label; `Ok` for a 2xx. Only the fields that decide the
/// label are read, so a refusal's other contents are not copied anywhere.
fn rejected(answer: &Answer) -> Result<(), LoginError> {
	#[derive(serde::Deserialize, Default)]
	struct Refusal {
		code: Option<u64>,
		retry_after: Option<f64>,
		suspended_user_token: Option<serde::de::IgnoredAny>,
		required_actions: Option<serde::de::IgnoredAny>,
		#[serde(default)]
		errors: serde_json::Value,
	}
	if answer.status.is_success() {
		return Ok(());
	}
	let body = if answer.page {
		Refusal::default()
	} else {
		decode::<Refusal>(&answer.bytes).unwrap_or_default()
	};
	// The wait comes first, whatever else the answer carries or however it is formatted.
	if answer.status == StatusCode::TOO_MANY_REQUESTS {
		let seconds = body
			.retry_after
			.into_iter()
			.chain(answer.retry_after)
			.filter(|seconds| seconds.is_finite())
			.fold(1.0_f64, f64::max);
		return Err(LoginError::RateLimited(seconds.ceil().min(86_400.0) as u32));
	}
	if answer.page {
		return Err(LoginError::Blocked);
	}
	match body.code {
		Some(60008) => return Err(LoginError::InvalidCode),
		// Unofficial: an MFA ticket or session Discord no longer accepts.
		Some(60006 | 60009) => return Err(LoginError::TicketExpired),
		_ => {}
	}
	if body.suspended_user_token.is_some()
		|| body.required_actions.is_some()
		|| matches!(body.code, Some(20013 | 40002 | 40007))
		|| matches!(
			answer.status,
			StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
		) {
		return Err(LoginError::Restricted);
	}
	if answer.status != StatusCode::BAD_REQUEST {
		return Err(LoginError::Rejected(answer.status.as_u16()));
	}
	// Unofficial: the form's field errors carry a code beside the localized text.
	Err(match field_code(&body.errors, 0) {
		Some("ACCOUNT_LOGIN_VERIFICATION_EMAIL") => LoginError::NewLocation,
		Some("ACCOUNT_COMPROMISED_RESET_PASSWORD") => LoginError::PasswordReset,
		Some("INVALID_TOTP_CODE" | "INVALID_CODE") => LoginError::InvalidCode,
		_ => LoginError::InvalidLogin,
	})
}

/// The first `_errors[].code` under a form-error object, at most four fields deep.
fn field_code(errors: &serde_json::Value, depth: u8) -> Option<&str> {
	let fields = errors.as_object()?;
	if let Some(code) = fields
		.get("_errors")
		.and_then(|list| list.as_array())
		.and_then(|list| list.iter().find_map(|error| error["code"].as_str()))
	{
		return Some(code);
	}
	if depth == 4 {
		return None;
	}
	fields
		.values()
		.take(16)
		.find_map(|field| field_code(field, depth + 1))
}

/// Reads a 2xx login or MFA answer: the token, or the second step it asks for.
fn granted(answer: &Answer) -> Result<Step, LoginError> {
	#[derive(serde::Deserialize)]
	struct Granted {
		token: Option<String>,
		ticket: Option<String>,
		#[serde(default)]
		mfa: bool,
		#[serde(default)]
		totp: Option<bool>,
		#[serde(default)]
		sms: Option<bool>,
		#[serde(default)]
		backup: Option<bool>,
		login_instance_id: Option<String>,
		required_actions: Option<Vec<serde::de::IgnoredAny>>,
	}
	impl Drop for Granted {
		fn drop(&mut self) {
			zeroize::Zeroize::zeroize(&mut self.token);
			zeroize::Zeroize::zeroize(&mut self.ticket);
		}
	}
	rejected(answer)?;
	let mut body = decode::<Granted>(&answer.bytes).map_err(|_| LoginError::Protocol)?;
	// Unofficial: a step such as `update_password` that only Discord's own page can finish.
	// The session it came with is not used.
	if body
		.required_actions
		.as_ref()
		.is_some_and(|actions| !actions.is_empty())
	{
		return Err(LoginError::RequiredAction);
	}
	if let Some(token) = body.token.take() {
		return SessionSecret::from_owner_input(token)
			.map(Step::Token)
			.map_err(|_| LoginError::Protocol);
	}
	let ticket = Zeroizing::new(body.ticket.take().ok_or(LoginError::Protocol)?);
	let valid = |value: &str| {
		(1..=MAX_TICKET).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_graphic())
	};
	let instance = body.login_instance_id.take();
	if !body.mfa || !valid(&ticket) || instance.as_deref().is_some_and(|id| !valid(id)) {
		return Err(LoginError::Protocol);
	}
	let challenge = MfaChallenge {
		ticket,
		instance,
		totp: body.totp == Some(true),
		sms: body.sms == Some(true),
		backup: body.backup == Some(true),
	};
	if !(challenge.totp || challenge.sms || challenge.backup) {
		return Err(LoginError::UnsupportedMfa);
	}
	Ok(Step::Mfa(challenge))
}
