//! Provider results relayed by `/gifs/search`, `/gifs/trending` and `/gifs/trending-gifs`.
//! Unofficial: the official developer reference does not document these normal-client routes.
//! `/gifs/trending` carries the categories and only a sample GIF for the home tile; the
//! trending grid itself is the plain GIF list from `/gifs/trending-gifs`.
use model::{GIF_CATEGORIES, GIF_PAGE_SIZE, Gif, GifCategory, GifPage};
use serde::{
	Deserialize, Deserializer,
	de::{IgnoredAny, SeqAccess, Visitor},
};

pub const MAX_WIRE: usize = 256 * 1024;

/// Provider IDs arrive as strings; accept a bare number too rather than reject the page.
#[derive(Deserialize)]
#[serde(untagged)]
enum WireId {
	Text(String),
	Number(u64),
}
impl From<WireId> for String {
	fn from(id: WireId) -> Self {
		match id {
			WireId::Text(id) => id,
			WireId::Number(id) => id.to_string(),
		}
	}
}

#[derive(Deserialize)]
pub struct GifDto {
	id: WireId,
	#[serde(default)]
	title: String,
	url: String,
	#[serde(default)]
	gif_src: Option<String>,
	#[serde(default)]
	preview: Option<String>,
	#[serde(default)]
	width: u32,
	#[serde(default)]
	height: u32,
}
#[derive(Deserialize)]
struct CategoryDto {
	name: String,
	#[serde(default)]
	src: Option<String>,
}
#[derive(Deserialize)]
pub struct SearchReply(#[serde(deserialize_with = "truncated::<_, _, GIF_PAGE_SIZE>")] Vec<GifDto>);
#[derive(Deserialize)]
pub struct TrendingReply {
	#[serde(default, deserialize_with = "truncated::<_, _, GIF_CATEGORIES>")]
	categories: Vec<CategoryDto>,
	#[serde(default, deserialize_with = "truncated::<_, _, GIF_PAGE_SIZE>")]
	gifs: Vec<GifDto>,
}

/// Keep the first `N` entries and skip the rest; a longer list is not a protocol failure.
fn truncated<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
	d: D,
) -> Result<Vec<T>, D::Error> {
	struct Truncated<T, const N: usize>(std::marker::PhantomData<T>);
	impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Truncated<T, N> {
		type Value = Vec<T>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			write!(f, "a list; the first {N} entries are kept")
		}
		fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
			let mut items = Vec::new();
			while items.len() < N {
				match seq.next_element()? {
					Some(item) => items.push(item),
					None => return Ok(items),
				}
			}
			while seq.next_element::<IgnoredAny>()?.is_some() {}
			Ok(items)
		}
	}
	d.deserialize_seq(Truncated::<T, N>(std::marker::PhantomData))
}

fn into_gifs(gifs: Vec<GifDto>) -> Vec<Gif> {
	let mut out: Vec<Gif> = Vec::with_capacity(gifs.len());
	for gif in gifs {
		if !model::valid_gif_url(&gif.url) {
			continue;
		}
		// KLIPY's `preview` GIF is often several megabytes; its `gif_src` WebP is the same
		// clip at a fraction of the size, so the picker shows that instead.
		let webp = gif
			.gif_src
			.as_ref()
			.filter(|src| src.ends_with(".webp") && model::valid_gif_preview(src))
			.cloned();
		let Some(preview) = webp.or(gif.preview) else {
			continue;
		};
		let gif = Gif {
			id: gif.id.into(),
			title: gif.title.trim().chars().take(256).collect(),
			// Sharing the actual GIF lets the timeline play it without a provider video player.
			url: gif
				.gif_src
				.filter(|url| model::valid_gif_url(url) && url.ends_with(".gif"))
				.unwrap_or(gif.url),
			preview,
			width: gif.width,
			height: gif.height,
		};
		// Rejected entries are skipped rather than failing the whole page.
		if gif.valid() && !out.iter().any(|known| known.id == gif.id) {
			out.push(gif);
		}
	}
	out
}

impl SearchReply {
	pub fn into_page(self) -> Result<GifPage, &'static str> {
		let page = GifPage {
			gifs: into_gifs(self.0),
			categories: Vec::new(),
		};
		page.valid().then_some(page).ok_or("GIF page rejected")
	}
}
impl TrendingReply {
	/// Categories from `/gifs/trending` with the full `/gifs/trending-gifs` list; the sample
	/// GIF is kept only when the full list has nothing usable.
	pub fn into_page_with(self, trending: SearchReply) -> Result<GifPage, &'static str> {
		let mut page = self.into_page()?;
		let gifs = into_gifs(trending.0);
		if !gifs.is_empty() {
			page.gifs = gifs;
		}
		page.valid().then_some(page).ok_or("GIF page rejected")
	}
	pub fn into_page(self) -> Result<GifPage, &'static str> {
		let mut categories: Vec<GifCategory> = Vec::with_capacity(self.categories.len());
		for category in self.categories {
			let category = GifCategory {
				name: category.name.trim().chars().take(64).collect(),
				// Unusable artwork leaves the flat tile rather than dropping the category.
				preview: category.src.filter(|src| model::valid_gif_preview(src)),
			};
			if category.valid() && !categories.iter().any(|known| known.name == category.name) {
				categories.push(category);
			}
		}
		let page = GifPage {
			gifs: into_gifs(self.gifs),
			categories,
		};
		page.valid().then_some(page).ok_or("GIF page rejected")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn replies_skip_rejected_entries_and_truncate_long_lists() {
		let entry = |id: usize, host: &str| {
			format!(
				r#"{{"id":"{id}","title":"  Wave  ","url":"https://{host}/view/wave-gif-{id}","src":"https://media.tenor.com/x/tenor.mp4","gif_src":"https://media.tenor.com/x/tenor.gif","width":498,"height":280,"preview":"https://media.tenor.com/x{id}/tenor.png"}}"#
			)
		};
		let mut body = String::from("[");
		body.push_str(&entry(0, "tenor.com"));
		body.push(',');
		body.push_str(&entry(1, "example.com"));
		body.push(',');
		body.push_str(r#"{"id":"2","url":"https://tenor.com/view/x","width":10,"height":10}"#);
		for id in 3..70 {
			body.push(',');
			body.push_str(&entry(id, "tenor.com"));
		}
		body.push(']');
		let page = crate::decode::<SearchReply>(body.as_bytes())
			.unwrap()
			.into_page()
			.unwrap();
		assert_eq!(page.gifs.len(), GIF_PAGE_SIZE - 2);
		assert_eq!(page.gifs[0].title, "Wave");
		assert!(page.gifs.iter().all(|gif| gif.id != "1" && gif.id != "2"));
		assert!(
			page.gifs
				.iter()
				.all(|gif| gif.id.parse::<usize>().unwrap() < GIF_PAGE_SIZE)
		);

		let trending = format!(
			r#"{{"categories":[{{"name":"happy","src":"https://media.tenor.com/c.gif"}},{{"name":" "}},{{"name":"happy"}}],"gifs":[{}]}}"#,
			entry(9, "tenor.com")
		);
		let page = crate::decode::<TrendingReply>(trending.as_bytes())
			.unwrap()
			.into_page()
			.unwrap();
		assert_eq!(
			page.categories,
			vec![GifCategory {
				name: "happy".into(),
				preview: Some("https://media.tenor.com/c.gif".into()),
			}]
		);
		assert_eq!(page.gifs.len(), 1);
		assert!(
			crate::decode::<TrendingReply>(b"{}")
				.unwrap()
				.into_page()
				.is_ok()
		);
		assert!(crate::decode::<SearchReply>(b"{}").is_err());
	}

	/// Synthetic replies shaped like the normal client's trending routes: `/gifs/trending`
	/// sends categories plus one sample GIF, `/gifs/trending-gifs` the grid itself.
	#[test]
	fn trending_grid_comes_from_trending_gifs_and_keeps_categories() {
		let klipy = |id: &str| {
			let name = id.trim_matches('"');
			format!(
				r#"{{"id":{id},"title":"Synthetic {name}","url":"https://klipy.com/gifs/synthetic-{name}","src":"https://static.klipy.com/s/{name}.mp4","gif_src":"https://static.klipy.com/s/{name}.webp","width":320,"height":240,"preview":"https://static.klipy.com/s/{name}.gif"}}"#
			)
		};
		let categories = format!(
			r#"{{"categories":[{{"name":"happy","src":"https://static.klipy.com/c/happy.mp4"}},{{"name":"dance","src":"https://static.klipy.com/c/dance.gif"}}],"gifs":[{}]}}"#,
			klipy(r#""sample""#)
		);
		let grid = format!(
			"[{},{},{}]",
			klipy(r#""a1""#),
			klipy("42"),
			klipy(r#""a1""#)
		);
		let decode_categories = || crate::decode::<TrendingReply>(categories.as_bytes()).unwrap();
		let page = decode_categories()
			.into_page_with(crate::decode::<SearchReply>(grid.as_bytes()).unwrap())
			.unwrap();
		assert_eq!(
			page.gifs
				.iter()
				.map(|gif| gif.id.as_str())
				.collect::<Vec<_>>(),
			["a1", "42"]
		);
		assert_eq!(page.gifs[0].preview, "https://static.klipy.com/s/a1.webp");
		assert_eq!(
			page.categories
				.iter()
				.map(|category| (category.name.as_str(), category.preview.is_some()))
				.collect::<Vec<_>>(),
			[("happy", false), ("dance", true)]
		);
		// An empty grid falls back to the sample rather than an empty Trending view.
		let page = decode_categories()
			.into_page_with(crate::decode::<SearchReply>(b"[]").unwrap())
			.unwrap();
		assert_eq!(page.gifs.len(), 1);
		assert_eq!(page.gifs[0].id, "sample");
		assert_eq!(page.categories.len(), 2);
	}
}
