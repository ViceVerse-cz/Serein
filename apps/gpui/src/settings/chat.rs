//! Chat page and the Appearance page's Layout group (reading preferences), in the main app's
//! order and wording. Choices this frontend cannot honour yet are shown dimmed with the
//! behaviour it actually has, and never change anything.
use super::kit;
use crate::Serein;
use gpui::{prelude::*, *};
use model::ReadingPreferences;

const NOT_YET: &str = "Not available in this preview yet.";

impl Serein {
	/// Applies reading preferences to the live view; invalid values are ignored.
	pub(crate) fn apply_reading(&mut self, value: ReadingPreferences) {
		if !value.is_valid() {
			return;
		}
		self.settings.reading = value;
		self.members_open = value.show_members;
		crate::chat::set_confirm_links(value.confirm_external_links);
	}

	fn update_reading(&mut self, change: impl FnOnce(&mut ReadingPreferences)) {
		let mut value = self.settings.reading;
		// The header's People button toggles the same preference, as in the main app.
		value.show_members = self.members_open;
		change(&mut value);
		self.apply_reading(value);
	}

	pub(super) fn set_show_hidden_channels(&mut self, on: bool) {
		self.settings.show_hidden_channels = on;
		self.sync_channels();
	}

	pub(super) fn settings_chat(&mut self, cx: &mut Context<Self>) -> Div {
		let reading = self.settings.reading;
		div()
			.flex()
			.flex_col()
			.gap_3()
			.child(kit::header_with_reset(
				"reset-chat",
				"Messages and media",
				"Reset chat",
				cx,
				|this, _| {
					let defaults = ReadingPreferences::default();
					this.update_reading(|value| {
						value.animate_gifs = defaults.animate_gifs;
						value.hide_media_links = defaults.hide_media_links;
						value.confirm_external_links = defaults.confirm_external_links;
						value.smooth_scrolling = defaults.smooth_scrolling;
						value.scroll_speed_percent = defaults.scroll_speed_percent;
					});
				},
			))
			.child(
				kit::card()
					.child(kit::switch(
						"animate-gifs",
						"Animate GIFs",
						Some("GIFs show as still images in this preview."),
						false,
						false,
						cx,
						|_, _, _, _| {},
					))
					.child(kit::divider())
					.child(kit::switch(
						"hide-media-links",
						"Hide image and GIF links",
						Some("Hide standalone links when their image or GIF preview is shown."),
						reading.hide_media_links,
						true,
						cx,
						|this, on, _, _| this.update_reading(|value| value.hide_media_links = on),
					)),
			)
			.child(kit::group(
				"Links",
				kit::card().child(kit::switch(
					"confirm-links",
					"Confirm before opening links",
					Some("Ask before opening external links. Discord links always open directly."),
					reading.confirm_external_links,
					true,
					cx,
					|this, on, _, _| this.update_reading(|value| value.confirm_external_links = on),
				)),
			))
			.child(kit::group(
				"Scrolling",
				kit::card()
					.child(kit::switch(
						"smooth-scrolling",
						"Smooth scrolling",
						Some("Scrolling follows your system settings in this preview."),
						false,
						false,
						cx,
						|_, _, _, _| {},
					))
					.child(div().h(px(10.)))
					.child(unavailable(kit::slider_row(
						"scroll-speed",
						"Scrolling speed",
						Some("Mouse wheel and trackpad movement. 100% is the default."),
						100,
						25..=300,
						"%",
						cx,
						|_, _, _| {},
					)))
					.child(kit::hint(NOT_YET)),
			))
			.child(kit::group(
				"Channel list",
				kit::card().child(kit::switch(
					"show-hidden-channels",
					"Show hidden channels",
					Some("Show channels you cannot currently access."),
					self.settings.show_hidden_channels,
					true,
					cx,
					|this, on, _, _| this.set_show_hidden_channels(on),
				)),
			))
	}

	pub(super) fn settings_layout(&mut self, cx: &mut Context<Self>) -> Div {
		let reading = self.settings.reading;
		div()
			.flex()
			.flex_col()
			.gap_2()
			.child(kit::header_with_reset(
				"reset-layout",
				"Layout",
				"Reset layout",
				cx,
				|this, _| {
					let defaults = ReadingPreferences::default();
					this.update_reading(|value| {
						value.zoom_percent = defaults.zoom_percent;
						value.sidebar_width = defaults.sidebar_width;
						value.show_members = defaults.show_members;
					});
				},
			))
			.child(
				kit::card()
					.child(unavailable(kit::slider_row(
						"zoom",
						"Zoom",
						Some("Scales text and controls across the app."),
						100,
						80..=150,
						"%",
						cx,
						|_, _, _| {},
					)))
					.child(kit::hint(NOT_YET))
					.child(div().h(px(10.)))
					.child(kit::slider_row(
						"sidebar-width",
						"Sidebar width",
						Some("Channel and conversation list width in wide windows."),
						reading.sidebar_width,
						190..=360,
						" px",
						cx,
						|this, width, _| this.update_reading(|value| value.sidebar_width = width),
					))
					.child(kit::divider())
					.child(kit::switch(
						"show-members",
						"Show People in wide windows",
						Some("Keep the member list open whenever the window is wide enough."),
						self.members_open,
						true,
						cx,
						|this, on, _, _| this.update_reading(|value| value.show_members = on),
					)),
			)
	}
}

/// Dims a control and covers it so pointer input never reaches it; its setter is a no-op.
fn unavailable(control: Div) -> Div {
	div()
		.relative()
		.child(control.opacity(0.6))
		.child(div().absolute().inset_0().occlude())
}
