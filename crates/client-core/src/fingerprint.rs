//! Client identity presented to Discord over REST and the Gateway.
//!
//! Observed (unofficial): Discord's desktop client sends a super-properties payload, its
//! Electron user agent, locale and timezone on REST and the Gateway; a 0.0.423 session was
//! recorded on 2026-11-19 (see the compatibility notes). How the service weighs any of
//! them is unverified; an earlier Serein identity without them coincided with account
//! restrictions (spam flag, attachment limits), which is not proof of cause. Every transport
//! uses the desktop metadata below. Embedded verification keeps its engine-specific identity.
//! Locale and timezone come from the operating system. Host and engine versions are those of
//! the recorded client, not evidence of an Electron runtime.

/// Chrome version advertised on Windows, where verification runs in Chromium WebView2.
const CHROME_MAJOR: &str = "149";
/// Safari version advertised on macOS and Linux, where verification runs in WebKit.
const SAFARI_VERSION: &str = "26.4";
/// The web build reported until this launch reads the current one from Discord's `/app` page.
/// Observed there and in a recorded desktop 0.0.423 session on 2026-11-20; it rose by about
/// 1,300 in the day before. The remote web build is separate from the native host version.
const FALLBACK_BUILD_NUMBER: u64 = 677_695;
static BUILD_NUMBER: std::sync::atomic::AtomicU64 =
	std::sync::atomic::AtomicU64::new(FALLBACK_BUILD_NUMBER);

/// The web build this launch reports as `client_build_number`.
pub fn client_build_number() -> u64 {
	BUILD_NUMBER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Adopts the `BUILD_NUMBER` in the text of Discord's `/app` page, the document the desktop
/// client loads at every start and takes its web build from (unofficial, observed
/// 2026-11-20). Builds only count up: an older, missing or implausible number is refused
/// and the previous one stays.
pub fn adopt_build_number(page: &str) -> bool {
	let number = page
		.split_once("\"BUILD_NUMBER\":\"")
		.and_then(|(_, rest)| rest.split_once('"'))
		.map(|(digits, _)| digits)
		.filter(|digits| digits.len() <= 8 && digits.bytes().all(|b| b.is_ascii_digit()))
		.and_then(|digits| digits.parse::<u64>().ok())
		.filter(|number| *number >= FALLBACK_BUILD_NUMBER);
	if let Some(number) = number {
		BUILD_NUMBER.fetch_max(number, std::sync::atomic::Ordering::Relaxed);
	}
	number.is_some()
}
/// The macOS updater's metadata version in the same recording. Discord's update manifests
/// do not expose it, and no Windows or Linux value was observed, so only macOS sends one.
const MACOS_NATIVE_BUILD_NUMBER: u64 = 98_981;
mod locale;
mod os;
pub use locale::{accept_language, discord_locale, system_locale, timezone};
/// Stable host versions from Discord's platform update manifests on 2026-11-19.
/// Keep the platform mapping explicit: a macOS host version is not a Linux/Windows one.
fn desktop_version() -> &'static str {
	match std::env::consts::OS {
		"windows" => "1.0.9269",
		"macos" => "0.0.423",
		_ => "1.0.170",
	}
}
/// The engine of the recorded desktop 0.0.423 client (2026-11-19), paired with wreq-util's
/// Chromium 152 handshake. Not a claim that Serein runs on Electron.
const ELECTRON_VERSION: &str = "44.10.2";
const DESKTOP_CHROMIUM_VERSION: &str = "152.0.7977.169";
const WEBKIT: bool = !cfg!(windows);

/// Discord's spelling of the host operating system.
pub fn os_name() -> &'static str {
	match std::env::consts::OS {
		"macos" => "Mac OS X",
		"windows" => "Windows",
		_ => "Linux",
	}
}

/// Platform token used inside the user agent string.
fn platform_token() -> &'static str {
	match std::env::consts::OS {
		"macos" => "Macintosh; Intel Mac OS X 10_15_7",
		"windows" => "Windows NT 10.0; Win64; x64",
		_ => "X11; Linux x86_64",
	}
}

/// Desktop application name used in super-properties (unofficial).
pub fn browser() -> &'static str {
	"Discord Client"
}

/// Electron version paired with [`browser`].
pub fn browser_version() -> String {
	ELECTRON_VERSION.to_owned()
}

/// Desktop transport identity; this does not change the native UI into Electron.
pub fn user_agent() -> String {
	format!(
		"Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) discord/{} Chrome/{DESKTOP_CHROMIUM_VERSION} Electron/{ELECTRON_VERSION} Safari/537.36",
		platform_token(),
		desktop_version()
	)
}

/// Identity for the actual embedded verification engine, independent of REST.
pub fn verification_user_agent() -> String {
	if WEBKIT {
		format!(
			"Mozilla/5.0 ({}) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{SAFARI_VERSION} Safari/605.1.15",
			platform_token()
		)
	} else {
		format!(
			"Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{CHROME_MAJOR}.0.0.0 Safari/537.36",
			platform_token()
		)
	}
}

#[cfg(feature = "transport")]
mod transport;
#[cfg(feature = "transport")]
pub use transport::{discord_request, emulation, websocket};

/// Chromium client hints paired with the desktop transport baseline.
pub fn client_hints() -> Option<[(&'static str, String); 3]> {
	let platform = match std::env::consts::OS {
		"macos" => "macOS",
		"windows" => "Windows",
		_ => "Linux",
	};
	Some([
		(
			"sec-ch-ua",
			// Electron's brands: no "Google Chrome", unlike the browser profile's own hints.
			r#""Not?A_Brand";v="24", "Chromium";v="152""#.to_owned(),
		),
		("sec-ch-ua-mobile", "?0".to_owned()),
		("sec-ch-ua-platform", format!("\"{platform}\"")),
	])
}

/// Encodings supported by the desktop transport.
pub fn accept_encoding() -> &'static str {
	"gzip, deflate, br, zstd"
}

/// A JSON string literal for `value`.
fn json(value: &str) -> String {
	let mut out = String::with_capacity(value.len() + 2);
	out.push('"');
	for c in value.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// A random version-4 UUID keeping only the bits set in `mask`; empty without system randomness.
fn uuid(mask: [u8; 16]) -> String {
	let mut bytes = [0_u8; 16];
	if getrandom::fill(&mut bytes).is_err() {
		return String::new();
	}
	bytes[6] = (bytes[6] & 0x0f) | 0x40;
	bytes[8] = (bytes[8] & 0x3f) | 0x80;
	let hex: String = bytes
		.iter()
		.zip(mask)
		.map(|(byte, mask)| format!("{:02x}", byte & mask))
		.collect();
	format!(
		"{}-{}-{}-{}-{}",
		&hex[..8],
		&hex[8..12],
		&hex[12..16],
		&hex[16..20],
		&hex[20..]
	)
}

/// A random version-4 UUID for this launch, as the desktop client makes one per start.
pub fn launch_id() -> &'static str {
	static ID: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| uuid([0xff; 16]));
	&ID
}

/// The launch's second UUID. The official client sets some of its bits when it detects a
/// client mod; these are the ones it leaves clear otherwise (unofficial: the mask is
/// Acheron's, and the signatures recorded from the desktop client fit it).
fn launch_signature() -> &'static str {
	const UNMODIFIED: [u8; 16] = [
		0xff, 0x7f, 0xef, 0xef, 0xf7, 0xef, 0xf7, 0xff, 0xdf, 0x7e, 0xff, 0xbf, 0xfe, 0xff, 0xf7,
		0xff,
	];
	static SIGNATURE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| uuid(UNMODIFIED));
	&SIGNATURE
}

/// Whether the window has focus and whether a voice connection is live, as last reported.
static FOCUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static RTC_CONNECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Records what the interface knows about its own use. The identity reports it as
/// `client_app_state`, and the Gateway beside each heartbeat.
pub fn set_activity(focused: bool, rtc_connected: bool) {
	FOCUSED.store(focused, std::sync::atomic::Ordering::Relaxed);
	RTC_CONNECTED.store(rtc_connected, std::sync::atomic::Ordering::Relaxed);
}

/// `(focused, rtc_connected)` as last given to [`set_activity`].
pub fn activity() -> (bool, bool) {
	(
		FOCUSED.load(std::sync::atomic::Ordering::Relaxed),
		RTC_CONNECTED.load(std::sync::atomic::Ordering::Relaxed),
	)
}

struct HeartbeatSession {
	id: String,
	started_ms: u64,
	used: std::time::Instant,
}
static HEARTBEAT_SESSION: std::sync::Mutex<Option<HeartbeatSession>> = std::sync::Mutex::new(None);
/// How long a heartbeat session outlives the last moment the app was in use.
const HEARTBEAT_SESSION_IDLE: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// The official client's "time spent" session: its ID and start in Unix milliseconds
/// (unofficial; recorded as `client_heartbeat_session_id` on REST and in Gateway opcode 41).
/// One starts while the window is focused or a call is connected and ends after 30 idle
/// minutes, as Acheron models it. It lives in memory only and no analytics event carries it.
pub fn heartbeat_session() -> Option<(String, u64)> {
	let (focused, rtc_connected) = activity();
	let in_use = focused || rtc_connected;
	let now = std::time::Instant::now();
	let mut slot = HEARTBEAT_SESSION.lock().ok()?;
	if slot
		.as_ref()
		.is_some_and(|session| now.duration_since(session.used) >= HEARTBEAT_SESSION_IDLE)
	{
		*slot = None;
	}
	match &mut *slot {
		Some(session) if in_use => session.used = now,
		None if in_use => {
			let id = uuid([0xff; 16]);
			if id.is_empty() {
				return None;
			}
			*slot = Some(HeartbeatSession {
				id,
				started_ms: std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.map_or(0, |elapsed| {
						elapsed.as_millis().min(u64::MAX as u128) as u64
					}),
				used: now,
			});
		}
		_ => {}
	}
	slot.as_ref()
		.map(|session| (session.id.clone(), session.started_ms))
}

/// The desktop client's own fields, in its order (observed in recorded 0.0.423 sessions).
/// The web client's device and referrer fields are not part of it.
fn fields() -> Vec<(&'static str, String)> {
	let os = &*os::OS;
	let mut fields: Vec<(&str, String)> = vec![
		("os", json(os_name())),
		("browser", json(browser())),
		("release_channel", json("stable")),
		("client_version", json(desktop_version())),
		("os_version", json(&os.version)),
		("os_version_is_release", "true".into()),
	];
	if let Some(sdk) = &os.sdk {
		fields.push(("os_sdk_version", json(sdk)));
	}
	fields.extend([
		("os_arch", json(os::arch())),
		("app_arch", json(os::arch())),
		("system_locale", json(system_locale())),
		("has_client_mods", "false".into()),
		("client_launch_id", json(launch_id())),
		("browser_user_agent", json(&user_agent())),
		("browser_version", json(&browser_version())),
	]);
	if let Some(linux) = &os.linux {
		fields.push(("window_manager", json(&linux.window_manager)));
		if let Some(distro) = &linux.distro {
			fields.push(("distro", json(distro)));
		}
		fields.push(("runtime_environment", json(linux.runtime_environment)));
		fields.push(("display_server", json(&linux.display_server)));
	}
	if let Some(revision) = &os.build_revision {
		fields.push(("os_build_revision", json(revision)));
	}
	fields.push(("client_build_number", client_build_number().to_string()));
	if cfg!(target_os = "macos") {
		fields.push(("native_build_number", MACOS_NATIVE_BUILD_NUMBER.to_string()));
	}
	fields.push(("client_event_source", "null".into()));
	fields.push(("launch_signature", json(launch_signature())));
	fields
}

fn object(fields: Vec<(&'static str, String)>) -> String {
	let body: Vec<String> = fields
		.into_iter()
		.map(|(key, value)| format!("\"{key}\":{value}"))
		.collect();
	format!("{{{}}}", body.join(","))
}

fn app_state() -> String {
	json(if activity().0 { "focused" } else { "unfocused" })
}

/// The decoded `X-Super-Properties` payload of a REST call. The heartbeat session is named
/// only while there is one.
pub fn properties() -> String {
	let mut fields = fields();
	if let Some((session, _)) = heartbeat_session() {
		fields.push(("client_heartbeat_session_id", json(&session)));
	}
	fields.push(("client_app_state", app_state()));
	object(fields)
}

/// Gateway `identify.d.properties` as the desktop client sends them when it connects after
/// its first screen rather than from its early fast-connect script (unofficial, recorded
/// 2026-11-19). Both recorded identifies named no heartbeat session, though one existed.
pub fn identify_properties() -> String {
	let mut fields = fields();
	fields.extend([
		("client_heartbeat_session_id", "null".into()),
		("client_app_state", app_state()),
		("is_fast_connect", "false".into()),
		("gateway_connect_reasons", json("AppSkeleton")),
	]);
	if let Some(installation) = installation_id() {
		fields.push(("installation_id", json(&installation)));
	}
	object(fields)
}

/// The installation ID Discord issued this device, once known.
static INSTALLATION: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Discord's installation ID for this device, sent as `X-Installation-ID` on REST and as
/// `installation_id` in the Gateway identify (unofficial, observed in SakuraCord's desktop
/// metadata). Serein never makes one up; without one, both are left out.
pub fn installation_id() -> Option<String> {
	INSTALLATION.read().ok()?.clone()
}

/// Keeps `id` as this device's installation ID when it is a plausible header value.
pub fn set_installation_id(id: &str) -> bool {
	let valid = (1..=256).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_graphic());
	if valid && let Ok(mut slot) = INSTALLATION.write() {
		*slot = Some(id.to_owned());
	}
	valid
}

/// `X-Super-Properties` header value: standard base64 of [`properties`].
pub fn super_properties() -> String {
	base64(properties().as_bytes())
}

/// Where in the interface a message create started. Unofficial: the web client's request
/// helper sends its `context` option as `X-Context-Properties`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageContext {
	ChatInput,
	Forwarding,
}

/// `X-Context-Properties` header value: standard base64 of `{"location":"…"}`.
pub fn context_properties(context: MessageContext) -> String {
	let location = match context {
		MessageContext::ChatInput => "chat_input",
		MessageContext::Forwarding => "forwarding",
	};
	base64(format!("{{\"location\":\"{location}\"}}").as_bytes())
}

fn base64(input: &[u8]) -> String {
	const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
	for chunk in input.chunks(3) {
		let bytes = [
			chunk[0],
			*chunk.get(1).unwrap_or(&0),
			*chunk.get(2).unwrap_or(&0),
		];
		let word = u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]);
		for i in 0..4 {
			if i <= chunk.len() {
				out.push(TABLE[((word >> (18 - 6 * i)) & 63) as usize] as char);
			} else {
				out.push('=');
			}
		}
	}
	out
}

/// Discord snowflake epoch in Unix milliseconds.
const DISCORD_EPOCH_MS: u128 = 1_420_070_400_000;

/// Snowflake-shaped message nonce like the official client's, unique per `sequence`.
pub fn nonce(unix_ms: u128, sequence: u64) -> String {
	let elapsed = unix_ms
		.saturating_sub(DISCORD_EPOCH_MS)
		.min(u64::MAX as u128) as u64;
	((elapsed << 22) | (sequence & 0x3F_FFFF)).to_string()
}

#[cfg(test)]
mod tests {
	use super::*;
	/// Free-text OS fields (a distribution name) must not break the JSON both transports send.
	#[test]
	fn properties_are_valid_json_with_escaped_text() {
		assert_eq!(json("a\"b\\c\u{1}"), r#""a\"b\\c\u0001""#);
	}
	/// The page is remote text: only a newer, plain build number may change the identity.
	#[test]
	fn build_number_is_adopted_only_from_a_plausible_page() {
		let page = |value: &str| {
			format!(r#"<script>window.GLOBAL_ENV = {{"BUILD_NUMBER":{value},"X":1}}"#)
		};
		let before = client_build_number();
		for refused in [
			String::new(),
			page("677696"),
			page("\"\""),
			page("\"12\""),
			page("\"-677696\""),
			page("\"677696e9\""),
			page("\"999999999999999999\""),
			page("\"677696"),
		] {
			assert!(!adopt_build_number(&refused), "{refused}");
		}
		assert_eq!(client_build_number(), before);
		let newer = before + 7;
		assert!(adopt_build_number(&page(&format!("\"{newer}\""))));
		assert_eq!(client_build_number(), newer);
		// An older page answered later does not move the identity back.
		assert!(adopt_build_number(&page(&format!("\"{}\"", before + 1))));
		assert_eq!(client_build_number(), newer);
	}
	#[test]
	fn base64_padding() {
		{
			assert_eq!(base64(b""), "");
			assert_eq!(base64(b"f"), "Zg==");
			assert_eq!(base64(b"fo"), "Zm8=");
			assert_eq!(base64(b"foo"), "Zm9v");
			assert_eq!(base64(b"foobar"), "Zm9vYmFy");
		}
	}
}
