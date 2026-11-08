//! Bounded, offline GIF descriptions for accessible labels and tooltips.
//!
//! This ports the sanitizing half of Equicord's BetterGifAltText: derive a short
//! name from a GIF URL or filename, never fetch or resolve anything.

/// URLs longer than this are left undescribed rather than parsed.
const MAX_SOURCE_BYTES: usize = 2048;
/// Upstream appends `...` once the joined name exceeds 300 characters.
const MAX_LABEL_CHARS: usize = 300;
/// Upstream keeps at most 20 delimiter-separated words.
const MAX_WORDS: usize = 20;

/// Upstream `altify` leaves non-GIF content types alone even when the filename
/// looks like a GIF. Only absent metadata falls back to the extension.
pub(crate) fn is_gif_media(content_type: Option<&str>, filename: &str) -> bool {
	content_type.map_or_else(
		|| filename.to_ascii_lowercase().ends_with(".gif"),
		|mime| {
			mime.split(';')
				.next()
				.unwrap_or(mime)
				.trim()
				.eq_ignore_ascii_case("image/gif")
		},
	)
}

/// Describe a GIF from its source URL or filename. Returns `None` when the
/// basename has no usable word characters. The caller decides whether the media
/// is actually a GIF.
pub(crate) fn gif_alt_text(source: &str) -> Option<String> {
	if source.is_empty() || source.len() > MAX_SOURCE_BYTES {
		return None;
	}
	let decoded = decode_uri(source);
	// Upstream also uses the last path component, including any query string.
	let basename = decoded.rsplit('/').next().unwrap_or(decoded.as_str());
	let without_digits: String = basename
		.chars()
		.filter(|character| !character.is_ascii_digit())
		.collect();
	let without_extension = without_digits
		.strip_suffix(".gif")
		.unwrap_or(&without_digits);
	let words = without_extension
		.split([',', '-', '_', ' '])
		.filter(|word| !word.is_empty())
		.take(MAX_WORDS);
	let name = words.collect::<Vec<_>>().join(" ");
	if name.is_empty() {
		return None;
	}
	let name = if name.chars().count() > MAX_LABEL_CHARS {
		format!(
			"{}...",
			name.chars().take(MAX_LABEL_CHARS).collect::<String>()
		)
	} else {
		name
	};
	Some(format!("GIF - {name}"))
}

/// Decode percent-escaped bytes the way JavaScript `decodeURI` does for this
/// use: reserved URI characters stay escaped, and invalid UTF-8 falls back to
/// the original source instead of producing replacement characters.
fn decode_uri(source: &str) -> String {
	fn hex(byte: u8) -> Option<u8> {
		match byte {
			b'0'..=b'9' => Some(byte - b'0'),
			b'a'..=b'f' => Some(byte - b'a' + 10),
			b'A'..=b'F' => Some(byte - b'A' + 10),
			_ => None,
		}
	}
	fn reserved(byte: u8) -> bool {
		matches!(
			byte,
			b';' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',' | b'#'
		)
	}

	let bytes = source.as_bytes();
	let mut decoded = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index] == b'%'
			&& let (Some(high), Some(low)) = (
				bytes.get(index + 1).copied().and_then(hex),
				bytes.get(index + 2).copied().and_then(hex),
			) {
			let value = high * 16 + low;
			if reserved(value) {
				decoded.extend_from_slice(&bytes[index..index + 3]);
			} else {
				decoded.push(value);
			}
			index += 3;
			continue;
		}
		decoded.push(bytes[index]);
		index += 1;
	}
	String::from_utf8(decoded).unwrap_or_else(|_| source.to_owned())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn strips_digits_extension_and_delimiters_like_upstream() {
		assert_eq!(
			gif_alt_text("https://cdn.discordapp.com/attachments/1/2/Funny_Cat-01.gif").as_deref(),
			Some("GIF - Funny Cat")
		);
		// decodeURI turns %20 into a delimiter and keeps %2F escaped; the following
		// upstream digit-removal step then removes the escape's `2` as well.
		assert_eq!(
			gif_alt_text("https://cdn.discordapp.com/a/My%20Clip%2Fx.gif").as_deref(),
			Some("GIF - My Clip%Fx")
		);
		// Unescaped lowercase .gif is removed; other suffixes are not.
		assert_eq!(
			gif_alt_text("https://x/y.CAT.gif").as_deref(),
			Some("GIF - y.CAT")
		);
	}

	#[test]
	fn caps_words_and_label_length() {
		let words = (0..30)
			.map(|index| format!("w{}", (b'a' + index % 26) as char))
			.collect::<Vec<_>>()
			.join("-");
		let label = gif_alt_text(&format!("https://x/{words}.gif")).expect("describable");
		assert!(label.starts_with("GIF - wa wb wc"));
		assert_eq!(label.split(' ').count(), 2 + MAX_WORDS);

		let long = "a".repeat(MAX_LABEL_CHARS + 50);
		let label = gif_alt_text(&format!("https://x/{long}.gif")).expect("describable");
		assert_eq!(
			label.chars().count(),
			"GIF - ".chars().count() + MAX_LABEL_CHARS + 3
		);
		assert!(label.ends_with("..."));
	}

	#[test]
	fn content_type_wins_over_a_gif_suffix() {
		assert!(is_gif_media(Some("image/gif; charset=binary"), "clip"));
		assert!(is_gif_media(None, "clip.GIF"));
		assert!(!is_gif_media(Some("image/png"), "clip.gif"));
		assert!(!is_gif_media(None, "clip.png"));
	}

	#[test]
	fn leaves_invalid_or_empty_sources_undescribed() {
		assert_eq!(gif_alt_text("https://x/").as_deref(), None);
		assert_eq!(gif_alt_text("https://x/12345.gif"), None);
		assert_eq!(gif_alt_text(""), None);
		assert_eq!(gif_alt_text(&"a".repeat(MAX_SOURCE_BYTES + 1)), None);
		// An invalid UTF-8 escape keeps the original text rather than dropping bytes.
		assert_eq!(
			gif_alt_text("https://x/%FF.gif").as_deref(),
			Some("GIF - %FF")
		);
	}
}
