//! One temporary, user-started provider player (YouTube/Vimeo embeds). Ephemeral storage,
//! no IPC, no downloads; links that leave the player go back to the app's link confirmation.
#[cfg(not(target_os = "linux"))]
use std::sync::{
	Arc,
	mpsc::{self, Receiver},
};

/// Provider hosts a player frame may load. Everything else is cancelled.
pub fn player_navigation(value: &str) -> bool {
	if value == "about:blank" || value == "about:srcdoc" {
		return true;
	}
	url::Url::parse(value).is_ok_and(|url| {
		url.scheme() == "https"
			&& url.username().is_empty()
			&& url.password().is_none()
			&& url.port_or_known_default() == Some(443)
			&& url.host_str().is_some_and(|host| {
				[
					"youtube-nocookie.com",
					"youtube.com",
					"ytimg.com",
					"googlevideo.com",
					"google.com",
					"gstatic.com",
					"googleapis.com",
					"vimeo.com",
					"vimeocdn.com",
				]
				.iter()
				.any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
			})
	})
}

#[cfg(not(target_os = "linux"))]
pub struct WebPlayer {
	view: wry::WebView,
	external: Receiver<String>,
}

#[cfg(not(target_os = "linux"))]
impl WebPlayer {
	/// `url` must already be a validated provider embed URL.
	pub fn open(
		parent: Arc<winit::window::Window>,
		url: &str,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, &'static str> {
		if !player_navigation(url) {
			return Err("This video provider is not supported.");
		}
		let (send, external) = mpsc::sync_channel(4);
		let mut headers = wry::http::HeaderMap::new();
		// Embedded players require a client identity referrer (YouTube error 153 otherwise).
		headers.insert(
			wry::http::header::REFERER,
			wry::http::HeaderValue::from_static("https://cz.viceverse.serein/"),
		);
		let builder = wry::WebViewBuilder::new()
			.with_url_and_headers(url, headers)
			.with_visible(false)
			.with_incognito(true)
			.with_devtools(false)
			.with_autoplay(true)
			.with_back_forward_navigation_gestures(false)
			.with_background_color((0, 0, 0, 255))
			.with_navigation_handler(|url| player_navigation(&url))
			.with_new_window_req_handler(move |url, _| {
				if url.len() <= 2048 && send.try_send(url).is_ok() {
					wake();
				}
				wry::NewWindowResponse::Deny
			})
			.with_download_started_handler(|_, _| false);
		let view = builder
			.build_as_child(parent.as_ref())
			.map_err(|_| "The video player could not open.")?;
		Ok(Self { view, external })
	}
	pub fn set_bounds(&self, x: i32, y: i32, width: u32, height: u32) {
		if self
			.view
			.set_bounds(wry::Rect {
				position: wry::dpi::PhysicalPosition::new(x, y).into(),
				size: wry::dpi::PhysicalSize::new(width, height).into(),
			})
			.is_ok()
		{
			let _ = self.view.set_visible(width > 0 && height > 0);
		}
	}
	/// A link the provider tried to open in a new window.
	pub fn external(&self) -> Option<String> {
		self.external.try_recv().ok()
	}
}

#[cfg(target_os = "linux")]
pub struct WebPlayer;

#[cfg(target_os = "linux")]
impl WebPlayer {
	pub fn open(
		_: std::sync::Arc<winit::window::Window>,
		_: &str,
		_: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, &'static str> {
		Err("In-app provider playback is unavailable on Linux.")
	}
	pub fn set_bounds(&self, _: i32, _: i32, _: u32, _: u32) {}
	pub fn external(&self) -> Option<String> {
		None
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn player_frames_stay_on_provider_hosts() {
		for url in [
			"https://www.youtube-nocookie.com/embed/KwRSAfoW5uo?autoplay=1",
			"https://i.ytimg.com/vi/KwRSAfoW5uo/hqdefault.jpg",
			"https://player.vimeo.com/video/76979871",
			"about:blank",
		] {
			assert!(super::player_navigation(url), "{url}");
		}
		for url in [
			"http://www.youtube.com/embed/x",
			"https://youtube.com.evil.test/",
			"https://evilyoutube.com/",
			"https://user@www.youtube.com/",
			"https://www.youtube.com:444/",
			"https://discord.com/",
			"serein-captcha://verification.invalid/",
		] {
			assert!(!super::player_navigation(url), "{url}");
		}
	}
}
