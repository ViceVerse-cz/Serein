use crate::Id;

/// Public artwork selected for an ordinary image attachment or a named Markdown link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageShare {
	Emoji { id: Id, animated: bool },
	Sticker { id: Id, format_type: u8 },
}

impl ImageShare {
	/// Only public Discord artwork URLs; no user-supplied host or path.
	pub fn url(self) -> Option<String> {
		let (id, host, kind, extension, query) = match self {
			Self::Emoji { id, animated } => (
				id,
				"cdn.discordapp.com",
				"emojis",
				if animated { "gif" } else { "png" },
				"?size=64",
			),
			Self::Sticker {
				id,
				format_type: 1 | 2,
			} => (id, "cdn.discordapp.com", "stickers", "png", ""),
			Self::Sticker { id, format_type: 3 } => (
				id,
				"media.discordapp.net",
				"stickers",
				"png",
				"?passthrough=false",
			),
			Self::Sticker { id, format_type: 4 } => {
				(id, "media.discordapp.net", "stickers", "gif", "")
			}
			_ => return None,
		};
		(id.0 != 0).then(|| format!("https://{host}/{kind}/{id}.{extension}{query}"))
	}

	/// Recognize exact artwork routes, rather than treating arbitrary links as uploads.
	pub fn from_url(url: &str) -> Option<Self> {
		if url.len() > 160 {
			return None;
		}
		let asset = if let Some(path) = url.strip_prefix("https://cdn.discordapp.com/emojis/") {
			let (id, extension) = path.split_once('.')?;
			Self::Emoji {
				id: Id(id.parse().ok()?),
				animated: match extension {
					"png?size=64" => false,
					"gif?size=64" => true,
					_ => return None,
				},
			}
		} else if let Some(path) = url.strip_prefix("https://cdn.discordapp.com/stickers/") {
			Self::Sticker {
				id: Id(path.strip_suffix(".png")?.parse().ok()?),
				format_type: 1,
			}
		} else {
			let path = url.strip_prefix("https://media.discordapp.net/stickers/")?;
			let (id, extension) = path.split_once('.')?;
			Self::Sticker {
				id: Id(id.parse().ok()?),
				format_type: match extension {
					"png?passthrough=false" => 3,
					"gif" => 4,
					_ => return None,
				},
			}
		};
		(asset.url().as_deref() == Some(url)).then_some(asset)
	}

	pub fn markdown(self, name: &str) -> Option<String> {
		let mut label = String::new();
		for character in name.chars().filter(|c| !c.is_control()).take(100) {
			if "\\[]()*_~`".contains(character) {
				label.push('\\');
			}
			label.push(character);
		}
		if label.trim().is_empty() {
			label.push_str("emoji");
		}
		Some(format!("[{label}]({})", self.url()?))
	}

	/// One bounded named link at the start of a draft; escapes stay part of native undo/copy.
	pub fn markdown_prefix(source: &str) -> Option<(Self, String, usize)> {
		let mut name = String::new();
		let mut escaped = false;
		for (byte, character) in source.strip_prefix('[')?.char_indices() {
			if byte > 400 || character.is_control() {
				return None;
			}
			if !escaped && character == ']' {
				let tail = source.get(byte + 2..)?.strip_prefix('(')?;
				let end = tail
					.as_bytes()
					.iter()
					.take(161)
					.position(|byte| *byte == b')')?;
				let asset = Self::from_url(tail.get(..end)?)?;
				return (!name.trim().is_empty()).then_some((asset, name, byte + end + 4));
			}
			if !escaped && character == '\\' {
				escaped = true;
				continue;
			}
			name.push(character);
			escaped = false;
		}
		None
	}

	/// Up to ten artwork links without any ordinary text; mixed drafts remain Markdown.
	pub fn markdown_only(source: &str) -> Option<Vec<Self>> {
		let mut remaining = source.trim();
		let mut assets = Vec::new();
		while !remaining.is_empty() {
			if assets.len() == 10 {
				return None;
			}
			let (asset, _, bytes) = Self::markdown_prefix(remaining)?;
			assets.push(asset);
			remaining = remaining[bytes..].trim_start();
		}
		(!assets.is_empty()).then_some(assets)
	}
}
