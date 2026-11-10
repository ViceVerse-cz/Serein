//! Browser TLS/HTTP2 handshake and WebSocket dialing for the advertised identity.
//! Gated so offline consumers such as the fuzz targets do not build BoringSSL.

/// wreq-util profile of [`super::browser`] at `CHROME_MAJOR`/`SAFARI_VERSION`; bump them
/// together.
fn profile() -> (wreq_util::Profile, wreq_util::Platform) {
	use wreq_util::{Platform, Profile};
	let platform = match std::env::consts::OS {
		"macos" => Platform::MacOS,
		"windows" => Platform::Windows,
		_ => Platform::Linux,
	};
	let profile = if super::WEBKIT {
		Profile::Safari26_4
	} else {
		Profile::Chrome149
	};
	(profile, platform)
}

/// TLS and HTTP/2 handshake of [`super::browser`], so every transport agrees with
/// [`super::user_agent`]. Every Discord connection must take its handshake from here.
pub fn emulation() -> wreq_util::Emulation {
	let (profile, platform) = profile();
	wreq_util::Emulation::builder()
		.profile(profile)
		.platform(platform)
		// Its headers describe a page navigation; each transport sends its own.
		.headers(false)
		.build()
}

/// Header names, in order, as a browser writes its HTTP/1.1 WebSocket upgrade. Cloudflare
/// answers the Gateway upgrade with 403 when `Origin` arrives lowercased, as hyper sends it.
const WEBSOCKET_HEADER_ORDER: [&str; 12] = [
	"Host",
	"Connection",
	"Pragma",
	"Cache-Control",
	"User-Agent",
	"Upgrade",
	"Origin",
	"Sec-WebSocket-Version",
	"Accept-Encoding",
	"Accept-Language",
	"Sec-WebSocket-Key",
	"Sec-WebSocket-Extensions",
];

/// Opens a WebSocket from discord.com's page origin with the browser TLS handshake.
/// Proxies are not used, matching the previous direct Gateway and voice dialing.
pub async fn websocket(
	url: &str,
	config: tokio_tungstenite::tungstenite::protocol::WebSocketConfig,
) -> Result<tokio_tungstenite::WebSocketStream<wreq::Upgraded>, &'static str> {
	let mut casing = wreq::header::OrigHeaderMap::new();
	for name in WEBSOCKET_HEADER_ORDER {
		casing.insert(name);
	}
	let socket = wreq::Client::builder()
		.emulation(emulation())
		.no_proxy()
		.redirect(wreq::redirect::Policy::none())
		.retry(wreq::retry::Policy::never())
		.build()
		.map_err(|_| "WebSocket client unavailable")?
		.websocket(url)
		.header(wreq::header::USER_AGENT, super::user_agent())
		.header(wreq::header::ORIGIN, "https://discord.com")
		.header(wreq::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
		.header(wreq::header::CACHE_CONTROL, "no-cache")
		.header(wreq::header::PRAGMA, "no-cache")
		.orig_headers(casing)
		.send()
		.await
		.map_err(|_| "WebSocket connection failed")?
		.into_websocket()
		.await
		.map_err(|_| "WebSocket handshake rejected")?;
	// Hand the upgraded stream to the caller's tungstenite with its own frame limits.
	Ok(tokio_tungstenite::WebSocketStream::from_raw_socket(
		socket.into_inner(),
		tokio_tungstenite::tungstenite::protocol::Role::Client,
		Some(config),
	)
	.await)
}

#[cfg(test)]
mod tests {
	use super::*;
	/// The handshake profile and the advertised identity are bumped by hand in separate
	/// places; a profile that names another browser version is the mismatch this guards.
	#[test]
	fn handshake_profile_names_the_advertised_browser() {
		let (profile, platform) = profile();
		let emulation = wreq::IntoEmulation::into_emulation(
			wreq_util::Emulation::builder()
				.profile(profile)
				.platform(platform)
				.build(),
		);
		let header = |name: &str| {
			emulation
				.headers
				.get(name)
				.and_then(|value| value.to_str().ok())
				.map(str::to_owned)
		};
		let agent = header("user-agent").unwrap();
		// Safari profiles carry only the macOS user agent, so compare from the engine on.
		let engine = |agent: &str| agent[agent.find("AppleWebKit").unwrap()..].to_owned();
		assert_eq!(engine(&agent), engine(&crate::fingerprint::user_agent()));
		assert_eq!(
			header("sec-ch-ua"),
			crate::fingerprint::client_hints().map(|[(_, brands), ..]| brands)
		);
	}
}
