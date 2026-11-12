//! Browser TLS/HTTP2 handshake and WebSocket dialing for the advertised identity.
//! Gated so offline consumers such as the fuzz targets do not build BoringSSL.

/// Chromium profile paired with the desktop user agent baseline.
fn profile() -> (wreq_util::Profile, wreq_util::Platform) {
	use wreq_util::{Platform, Profile};
	let platform = match std::env::consts::OS {
		"macos" => Platform::MacOS,
		"windows" => Platform::Windows,
		_ => Platform::Linux,
	};
	let profile = Profile::Chrome148;
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

/// Domains the official web client reaches with the page's own handshake.
const DISCORD_DOMAINS: [&str; 5] = [
	"discord.com",
	"discord.gg",
	"discord.media",
	"discordapp.com",
	"discordapp.net",
];

fn discord_host(host: &str) -> bool {
	DISCORD_DOMAINS.iter().any(|domain| {
		host.strip_suffix(domain)
			.is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
	})
}

/// Gives a request bound for a Discord host, such as the CDN, the browser handshake and user
/// agent. Requests to other hosts keep the client's own identity.
pub fn discord_request(request: wreq::RequestBuilder, url: &str) -> wreq::RequestBuilder {
	let discord = url
		.parse::<wreq::Uri>()
		.ok()
		.and_then(|uri| uri.host().map(discord_host))
		.unwrap_or(false);
	if !discord {
		return request;
	}
	request
		.emulation(emulation())
		.header(wreq::header::USER_AGENT, super::user_agent())
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
		.header(wreq::header::ACCEPT_LANGUAGE, super::accept_language())
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
	/// Look-alike hosts must keep the plain identity; Discord's subdomains must not.
	#[test]
	fn only_discord_hosts_take_the_browser_identity() {
		for host in ["cdn.discordapp.com", "media.discordapp.net", "discord.com"] {
			assert!(discord_host(host), "{host}");
		}
		for host in [
			"evildiscord.com",
			"discord.com.example",
			"discordapp.net.evil",
			"tenor.com",
		] {
			assert!(!discord_host(host), "{host}");
		}
	}
}
