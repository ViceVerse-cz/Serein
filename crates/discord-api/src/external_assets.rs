//! Turn the image URLs a Rich Presence client supplies into Discord media-proxy paths.
//! Serein never fetches the URL itself; Discord returns the proxy path presences may carry.
use crate::{DiscordApi, Failure};
use discord_protocol::rpc::MAX_ASSET_KEY;
use model::Id;
use reqwest::Method;
use serde::Deserialize;

/// Two images per activity, each a bounded proxy path.
const MAX_RESPONSE: usize = 8 * 1024;
pub const MAX_EXTERNAL_URL: usize = 1024;

#[derive(Deserialize)]
struct Resolved {
	external_asset_path: String,
}

/// An external image must be an ordinary absolute HTTPS URL, never a local or credentialed one.
pub fn external_image_url(value: &str) -> Option<&str> {
	let rest = value.strip_prefix("https://")?;
	(!rest.is_empty()
		&& value.len() <= MAX_EXTERNAL_URL
		&& !value.chars().any(|c| c.is_control() || c.is_whitespace())
		&& !rest.contains('@')
		&& !rest.starts_with('/'))
	.then_some(value)
}

impl DiscordApi {
	/// Resolves in request order; a shorter reply is an error rather than a silent mismatch.
	pub async fn external_assets(
		&self,
		application: Id,
		urls: &[String],
	) -> Result<Vec<String>, Failure> {
		if urls.is_empty() || urls.len() > 2 || application.0 == 0 {
			return Err(Failure::Protocol);
		}
		if urls.iter().any(|url| external_image_url(url).is_none()) {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::POST,
				&format!("/applications/{application}/external-assets"),
				Some(serde_json::json!({ "urls": urls })),
				MAX_RESPONSE,
			)
			.await?;
		decode(&bytes, urls.len()).ok_or(Failure::ProtocolAt(
			"Discord could not prepare the game's artwork",
		))
	}
}

fn decode(bytes: &[u8], expected: usize) -> Option<Vec<String>> {
	let resolved: Vec<Resolved> = serde_json::from_slice(bytes).ok()?;
	(resolved.len() == expected).then_some(())?;
	resolved
		.into_iter()
		.map(|item| {
			let key = format!("mp:{}", item.external_asset_path);
			// The path is rendered as a media-proxy URL, so reuse the presence image checks.
			(key.len() <= MAX_ASSET_KEY
				&& model::ActivityImage::Proxy(item.external_asset_path).valid())
			.then_some(key)
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_absolute_https_urls_resolve_into_bounded_proxy_paths() {
		assert!(external_image_url("https://example.com/art.png").is_some());
		for rejected in [
			"http://example.com/art.png",
			"https://",
			"https:///etc/passwd",
			"https://user:pass@example.com/a.png",
			"https://example.com/a b.png",
			"mp:external/hash/https/example.com/a.png",
			"map",
		] {
			assert!(external_image_url(rejected).is_none());
		}
		assert!(external_image_url(&format!("https://e.com/{}", "x".repeat(2048))).is_none());
		assert_eq!(
			decode(
				br#"[{"external_asset_path":"external/hash-01/https/example.com/art.png"}]"#,
				1
			),
			Some(vec!["mp:external/hash-01/https/example.com/art.png".into()])
		);
		assert!(decode(br#"[{"external_asset_path":"external/../secret"}]"#, 1).is_none());
		assert!(decode(br#"[{"external_asset_path":"a"}]"#, 2).is_none());
		assert!(decode(b"{}", 1).is_none());
	}
}
