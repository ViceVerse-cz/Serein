//! Client identity presented to Discord over REST and the Gateway.
//!
//! Discord's anti-spam classifies normal-user sessions by their client fingerprint. A custom
//! user agent with no super-properties header reads as an automated tool, which quarantines
//! the account (spam restriction, attachment upload limits). Presenting one consistent,
//! browser-shaped identity on every transport keeps the session classified as an ordinary
//! web client. Nothing here carries account data; it is the same for every user.

/// Chrome version advertised on Windows and Linux; bump together when refreshing.
const CHROME_MAJOR: &str = "128";
/// Safari version advertised on macOS, where the verification widget runs in WebKit.
const SAFARI_VERSION: &str = "18.5";
/// Build number of the official web client this fingerprint mirrors.
pub const CLIENT_BUILD_NUMBER: u64 = 328_246;
pub const LOCALE: &str = "en-US";

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

/// Browser name Discord expects in super-properties, matched to the engine the embedded
/// hCaptcha widget actually runs in. A passcode solved in WebKit but submitted with a Chrome
/// fingerprint reads as a relayed captcha and triggers phone verification.
pub fn browser() -> &'static str {
	if cfg!(target_os = "macos") {
		"Safari"
	} else {
		"Chrome"
	}
}

/// Browser version paired with [`browser`].
pub fn browser_version() -> String {
	if cfg!(target_os = "macos") {
		SAFARI_VERSION.to_owned()
	} else {
		format!("{CHROME_MAJOR}.0.0.0")
	}
}

/// `User-Agent` header value shared by REST, uploads, the Gateway identify and the
/// embedded verification webview.
pub fn user_agent() -> String {
	if cfg!(target_os = "macos") {
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

/// Gateway `identify.d.properties` and the decoded `X-Super-Properties` payload.
pub fn properties() -> String {
	// Hand-built JSON keeps this crate free of a serde dependency; every value is a fixed
	// literal or a controlled ASCII string, so no escaping is required.
	format!(
		concat!(
			"{{\"os\":\"{os}\",\"browser\":\"{browser}\",\"device\":\"\",\"system_locale\":\"{locale}\",",
			"\"browser_user_agent\":\"{ua}\",\"browser_version\":\"{version}\",\"os_version\":\"\",",
			"\"referrer\":\"\",\"referring_domain\":\"\",\"referrer_current\":\"\",\"referring_domain_current\":\"\",",
			"\"release_channel\":\"stable\",\"client_build_number\":{build},\"client_event_source\":null}}"
		),
		os = os_name(),
		locale = LOCALE,
		browser = browser(),
		ua = user_agent(),
		version = browser_version(),
		build = CLIENT_BUILD_NUMBER,
	)
}

/// `X-Super-Properties` header value: standard base64 of [`properties`].
pub fn super_properties() -> String {
	base64(properties().as_bytes())
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
	#[test]
	fn properties_are_valid_json_and_consistent_with_user_agent() {
		{
			let text = properties();
			assert!(text.starts_with('{') && text.ends_with('}'));
			assert!(text.contains(&format!("\"browser_user_agent\":\"{}\"", user_agent())));
			assert!(text.contains(&format!("\"browser\":\"{}\"", browser())));
			assert!(text.contains(&format!("\"browser_version\":\"{}\"", browser_version())));
			assert!(user_agent().starts_with("Mozilla/5.0 ("));
			assert_eq!(user_agent().contains("Chrome/"), browser() == "Chrome");
			assert_eq!(user_agent().contains("Version/"), browser() == "Safari");
			assert!(
				super_properties()
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
			);
		}
		{
			assert_eq!(base64(b""), "");
			assert_eq!(base64(b"f"), "Zg==");
			assert_eq!(base64(b"fo"), "Zm8=");
			assert_eq!(base64(b"foo"), "Zm9v");
			assert_eq!(base64(b"foobar"), "Zm9vYmFy");
		}
	}
	#[test]
	fn nonce_is_a_decimal_snowflake() {
		let a = nonce(1_700_000_000_000, 1);
		let b = nonce(1_700_000_000_000, 2);
		assert_ne!(a, b);
		assert!(a.bytes().all(|b| b.is_ascii_digit()));
		assert_eq!(
			a.parse::<u64>().unwrap() >> 22,
			(1_700_000_000_000 - 1_420_070_400_000)
		);
		assert_eq!(nonce(0, 7), "7");
	}
}
