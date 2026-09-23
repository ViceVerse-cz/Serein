//! General: startup, window and graphics behaviour on this device. Startup and the tray are
//! main-app features, shown disabled with the reason; the window size can be remembered.
use super::kit;
use crate::Serein;
use crate::theme::{color, palette};
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_general(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
		let saves = self.persist.saves_window();
		let remember = self.persist.remember_window();
		let renderer = if cfg!(target_os = "macos") {
			"GPUI · Metal"
		} else if cfg!(target_os = "windows") {
			"GPUI · DirectX"
		} else {
			"GPUI · wgpu"
		};
		// GPUI names the adapter only where the platform reports it (not on macOS today).
		let adapter = window.gpu_specs().map(|specs| {
			format!(
				"Currently drawing with {}{}.",
				specs.device_name,
				if specs.is_software_emulated {
					" (software)"
				} else {
					""
				}
			)
		});
		let choice = if cfg!(target_os = "macos") {
			"GPUI prefers the built-in, low-power GPU; Apple silicon Macs have one."
		} else {
			"GPUI picks the graphics adapter itself."
		};
		let graphics_detail = match &adapter {
			Some(adapter) => format!("{adapter} {choice}"),
			None => choice.to_owned(),
		};
		let tray = if cfg!(target_os = "macos") {
			"Keep Serein in the menu bar"
		} else {
			"Keep Serein in the system tray"
		};
		div()
			.flex()
			.flex_col()
			.gap_3()
			.child(kit::group(
				"Startup",
				kit::card()
					.child(kit::switch(
						"startup-open",
						"Open Serein when your computer starts",
						Some("Serein signs in and connects in the background."),
						false,
						false,
						cx,
						|_, _, _, _| {},
					))
					.child(kit::divider())
					.child(kit::switch(
						"startup-minimized",
						"Start minimized",
						Some("Start in the background, out of your way."),
						false,
						false,
						cx,
						|_, _, _, _| {},
					))
					.child(kit::hint(
						"Automatic startup is set in the main Serein app. This preview never registers itself to open with your computer.",
					)),
			))
			.child(kit::group(
				"Window",
				kit::card()
					.child(kit::switch(
						"remember-window",
						"Remember window size",
						Some("Reopen at the size and position you left, kept on a connected display."),
						remember,
						true,
						cx,
						|this, on, window, cx| this.set_remember_window(on, window, cx),
					))
					.when(!saves, |d| {
						d.child(kit::hint(if self.state.demo {
							"Offline preview · the window size is never saved."
						} else {
							"Local settings are unavailable, so the window size lasts until you quit."
						}))
					})
					.child(kit::divider())
					.child(kit::switch(
						"keep-in-tray",
						tray,
						Some("Closing the window quits this preview."),
						false,
						false,
						cx,
						|_, _, _, _| {},
					))
					.child(kit::hint(
						"The tray icon is part of the main Serein app; this preview has none.",
					)),
			))
			.child(kit::group(
				"Graphics",
				kit::card().child(kit::row(
					"Render with",
					Some(&graphics_detail),
					div()
						.h(px(32.))
						.px_3()
						.rounded(px(6.))
						.bg(color(palette().base))
						.border_1()
						.border_color(color(palette().border))
						.flex()
						.items_center()
						.text_size(px(14.))
						.font_weight(FontWeight::MEDIUM)
						.text_color(color(palette().text_strong))
						.child(renderer),
				))
				.child(kit::hint(
					"Choosing a GPU is a main-app setting; this preview does not read it.",
				)),
			))
	}
}
