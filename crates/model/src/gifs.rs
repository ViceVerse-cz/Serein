//! Provider GIF results relayed by the service: bounded strings, fixed hosts, static previews.

pub const GIF_PAGE_SIZE: usize = 50;
pub const GIF_CATEGORIES: usize = 32;
pub const MAX_GIF_BYTES: usize = 96 * 1024;
pub const MAX_GIF_FAVORITES: usize = 100;
const MAX_URL: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gif {
	pub id: String,
	pub title: String,
	/// Provider page address; this is the text sent when the GIF is chosen.
	pub url: String,
	/// Preview on an allowed media host; only its first frame is displayed.
	pub preview: String,
	pub width: u32,
	pub height: u32,
}
impl Gif {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.id.capacity()
			+ self.title.capacity()
			+ self.url.capacity()
			+ self.preview.capacity()
	}
	pub fn valid(&self) -> bool {
		(1..=64).contains(&self.id.len())
			&& self
				.id
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
			&& self.title.len() <= 256
			&& !self.title.chars().any(char::is_control)
			&& valid_gif_url(&self.url)
			&& valid_gif_preview(&self.preview)
			&& (1..=4096).contains(&self.width)
			&& (1..=4096).contains(&self.height)
	}
}

fn plain_https_path(url: &str, hosts: &[&str]) -> bool {
	url.len() <= MAX_URL
		&& url
			.bytes()
			.all(|b| b.is_ascii_graphic() && b != b'\\' && b != b'?' && b != b'#')
		&& !url.contains("..")
		&& hosts.iter().any(|host| {
			url.strip_prefix("https://")
				.and_then(|rest| rest.strip_prefix(host))
				.and_then(|rest| rest.strip_prefix('/'))
				.is_some_and(|path| !path.is_empty() && !path.starts_with('/'))
		})
}

/// KLIPY results and previously saved Tenor favorites may be shared.
pub fn valid_gif_url(url: &str) -> bool {
	plain_https_path(
		url,
		&[
			"klipy.com",
			"static.klipy.com",
			"static1.klipy.com",
			"static2.klipy.com",
			"tenor.com",
			"media.tenor.com",
		],
	)
}

/// Allow only provider media hosts and supported image formats.
pub fn valid_gif_preview(url: &str) -> bool {
	(plain_https_path(url, &["media.tenor.com", "c.tenor.com"])
		&& (url.ends_with(".png") || url.ends_with(".gif")))
		|| (plain_https_path(
			url,
			&["static.klipy.com", "static1.klipy.com", "static2.klipy.com"],
		) && [".png", ".gif", ".jpg", ".jpeg", ".webp"]
			.iter()
			.any(|extension| url.ends_with(extension)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GifCategory {
	/// Opens a search for this name.
	pub name: String,
	/// Artwork on an allowed media host, shown behind the name.
	pub preview: Option<String>,
}
impl GifCategory {
	pub fn valid(&self) -> bool {
		(1..=64).contains(&self.name.len())
			&& !self.name.trim().is_empty()
			&& !self.name.chars().any(char::is_control)
			&& self.preview.as_deref().is_none_or(valid_gif_preview)
	}
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GifPage {
	pub gifs: Vec<Gif>,
	/// Trending categories, shown as tiles on the picker home.
	pub categories: Vec<GifCategory>,
}
impl GifPage {
	pub fn bytes(&self) -> usize {
		self.gifs.capacity() * size_of::<Gif>()
			+ self
				.gifs
				.iter()
				.map(|gif| gif.bytes() - size_of::<Gif>())
				.sum::<usize>()
			+ self.categories.capacity() * size_of::<GifCategory>()
			+ self
				.categories
				.iter()
				.map(|category| {
					category.name.capacity() + category.preview.as_ref().map_or(0, String::capacity)
				})
				.sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		self.gifs.len() <= GIF_PAGE_SIZE
			&& self.categories.len() <= GIF_CATEGORIES
			&& self.bytes() <= MAX_GIF_BYTES
			&& self.gifs.iter().all(Gif::valid)
			&& self
				.gifs
				.iter()
				.enumerate()
				.all(|(i, gif)| self.gifs[..i].iter().all(|other| other.id != gif.id))
			&& self.categories.iter().all(GifCategory::valid)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn gif(id: &str) -> Gif {
		Gif {
			id: id.into(),
			title: "Synthetic wave".into(),
			url: "https://tenor.com/view/synthetic-wave-gif-1".into(),
			preview: "https://media.tenor.com/synthetic/AAAAe/wave.png".into(),
			width: 498,
			height: 280,
		}
	}

	#[test]
	fn gif_hosts_and_pages_are_bounded() {
		assert!(gif("a1").valid());
		let mut klipy = gif("klipy-1");
		klipy.url = "https://klipy.com/gifs/synthetic-wave".into();
		for host in ["static.klipy.com", "static1.klipy.com", "static2.klipy.com"] {
			for extension in ["png", "gif", "jpg", "jpeg", "webp"] {
				klipy.preview = format!("https://{host}/synthetic/wave.{extension}");
				assert!(klipy.valid());
			}
		}
		assert!(!valid_gif_preview(
			"https://static.klipy.com.evil.test/x.gif"
		));
		assert!(!valid_gif_preview("https://static.klipy.com/x.mp4"));
		assert!(!valid_gif_url("https://klipy.com.evil.test/gifs/x"));
		assert!(valid_gif_url("https://media.tenor.com/x/tenor.gif"));
		for url in [
			"http://tenor.com/view/x",
			"https://tenor.com.evil.example/view/x",
			"https://tenor.com//x",
			"https://tenor.com/view/x?y=1",
			"https://tenor.com/view/x#frag",
			"https://tenor.com/../x",
			"https://tenor.com/",
			"https://example.com/tenor.com/x",
			"https://tenor.com/view/x y",
		] {
			assert!(!valid_gif_url(url), "{url}");
		}
		assert!(valid_gif_preview("https://media.tenor.com/x/tenor.gif"));
		assert!(!valid_gif_preview("https://tenor.com/view/x.png"));
		let mut wrong = gif("a1");
		wrong.width = 0;
		assert!(!wrong.valid());
		let mut wrong = gif("a1");
		wrong.title = "bad\u{7}".into();
		assert!(!wrong.valid());
		assert!(!gif("").valid());
		assert!(!gif("a/1").valid());

		let page = GifPage {
			gifs: (0..GIF_PAGE_SIZE).map(|i| gif(&format!("g{i}"))).collect(),
			categories: ["happy", "dance"]
				.map(|name| GifCategory {
					name: name.into(),
					preview: Some(format!("https://static.klipy.com/synthetic/{name}.gif")),
				})
				.into(),
		};
		assert!(page.valid());
		assert!(page.bytes() <= MAX_GIF_BYTES);
		let mut duplicate = page.clone();
		duplicate.gifs[1].id = "g0".into();
		assert!(!duplicate.valid());
		let mut oversized = page.clone();
		oversized.gifs.push(gif("extra"));
		assert!(!oversized.valid());
		let mut blank = page.clone();
		blank.categories.push(GifCategory {
			name: "   ".into(),
			preview: None,
		});
		assert!(!blank.valid());
		let mut foreign = page.clone();
		foreign.categories[0].preview = Some("https://example.com/x.gif".into());
		assert!(!foreign.valid());
	}
}
