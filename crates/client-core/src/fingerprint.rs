//! Client identity presented to Discord over REST and the Gateway.
//!
//! Observed (unofficial): Discord's desktop client sends a super-properties payload, its
//! Electron user agent, locale and timezone on REST and the Gateway; a 0.0.423 session was
//! recorded on 2026-11-19 (see the compatibility notes). How the service weighs any of
//! them is unverified; an earlier Serein identity without them coincided with account
//! restrictions (spam flag, attachment limits), which is not proof of cause. Every transport
//! uses the desktop metadata below. Embedded verification keeps its engine-specific identity.
//! Locale and timezone come from the operating system. Host versions are observed
//! platform metadata; the older paired engine/profile baseline remains
//! an unofficial reference, not evidence of an Electron runtime.

/// Chrome version advertised on Windows, where verification runs in Chromium WebView2.
const CHROME_MAJOR: &str = "149";
/// Safari version advertised on macOS and Linux, where verification runs in WebKit.
const SAFARI_VERSION: &str = "26.4";
/// Observed in the public `/app` bundle and a recorded desktop 0.0.423 session on
/// 2026-11-19. The remote web build is separate from the native host version.
pub const CLIENT_BUILD_NUMBER: u64 = 676_376;
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
/// Retained paired engine baseline: the pinned wreq-util has no Chromium 152 profile.
/// The downloaded Linux core declares Electron 44.9.3, so this is not a current runtime claim.
const ELECTRON_VERSION: &str = "42.7.1";
const DESKTOP_CHROMIUM_VERSION: &str = "148.0.7778.280";
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
			r#""Not/A)Brand";v="99", "Chromium";v="148""#.to_owned(),
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

/// A random version-4 UUID for this launch, as the desktop client makes one per start.
fn launch_id() -> &'static str {
	static ID: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
		let mut bytes = [0_u8; 16];
		if getrandom::fill(&mut bytes).is_err() {
			return String::new();
		}
		bytes[6] = (bytes[6] & 0x0f) | 0x40;
		bytes[8] = (bytes[8] & 0x3f) | 0x80;
		let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
		format!(
			"{}-{}-{}-{}-{}",
			&hex[..8],
			&hex[8..12],
			&hex[12..16],
			&hex[16..20],
			&hex[20..]
		)
	});
	&ID
}

/// Gateway `identify.d.properties` and the decoded `X-Super-Properties` payload: the
/// desktop client's fields, in its order (observed in a recorded 0.0.423 session). The web
/// client's device and referrer fields are not part of it. `launch_signature`, the
/// analytics heartbeat session and the focus state are left out.
pub fn properties() -> String {
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
	fields.push(("client_build_number", CLIENT_BUILD_NUMBER.to_string()));
	if cfg!(target_os = "macos") {
		fields.push(("native_build_number", MACOS_NATIVE_BUILD_NUMBER.to_string()));
	}
	fields.push(("client_event_source", "null".into()));
	let body: Vec<String> = fields
		.into_iter()
		.map(|(key, value)| format!("\"{key}\":{value}"))
		.collect();
	format!("{{{}}}", body.join(","))
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
