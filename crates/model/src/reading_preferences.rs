/// Fixed-size, application-wide reading settings; independent of Discord accounts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadingPreferences {
	pub zoom_percent: u16,
	pub sidebar_width: u16,
	pub show_members: bool,
	pub animate_gifs: bool,
	pub smooth_scrolling: bool,
	pub scroll_speed_percent: u16,
	pub hide_media_links: bool,
	pub confirm_external_links: bool,
}
impl Default for ReadingPreferences {
	fn default() -> Self {
		Self {
			zoom_percent: 100,
			sidebar_width: 236,
			show_members: true,
			animate_gifs: true,
			smooth_scrolling: true,
			scroll_speed_percent: 100,
			hide_media_links: true,
			confirm_external_links: true,
		}
	}
}
impl ReadingPreferences {
	pub fn is_valid(self) -> bool {
		(80..=150).contains(&self.zoom_percent)
			&& (190..=360).contains(&self.sidebar_width)
			&& (25..=300).contains(&self.scroll_speed_percent)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_and_inclusive_bounds() {
		let defaults = ReadingPreferences::default();
		assert_eq!(defaults.zoom_percent, 100);
		assert_eq!(defaults.sidebar_width, 236);
		assert!(defaults.show_members && defaults.smooth_scrolling && defaults.is_valid());
		for zoom_percent in [0, 79, 80, 150, 151, u16::MAX] {
			for sidebar_width in [0, 189, 190, 360, 361, u16::MAX] {
				for show_members in [false, true] {
					let preferences = ReadingPreferences {
						zoom_percent,
						sidebar_width,
						show_members,
						animate_gifs: false,
						smooth_scrolling: true,
						scroll_speed_percent: 100,
						hide_media_links: true,
						confirm_external_links: true,
					};
					assert_eq!(
						preferences.is_valid(),
						matches!(zoom_percent, 80 | 150) && matches!(sidebar_width, 190 | 360)
					);
				}
			}
		}
	}
}
