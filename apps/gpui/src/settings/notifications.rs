//! Notifications: the desktop alert opt-in; sounds and badges are explained, not offered.
use super::kit;
use crate::Serein;
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_notifications(&mut self, cx: &mut Context<Self>) -> Div {
		let demo = self.state.demo;
		div()
			.flex()
			.flex_col()
			.gap_3()
			.child(kit::group(
				"Overview",
				kit::card()
					.child(kit::switch(
						"desktop-notifications",
						"Enable Desktop Notifications",
						Some(
							"Mentions and direct messages while Serein is in the background. For per-channel or per-server notifications, right-click the channel or server and select Notification Settings.",
						),
						self.settings.notifications,
						true,
						cx,
						move |this, on, _, _| {
							if demo {
								// The preview never asks the OS for permission.
								this.settings.notifications = on;
							} else {
								this.set_notifications(on);
							}
						},
					))
					.when(demo, |d| d.child(kit::hint("Offline preview · no alerts are shown."))),
			))
			.child(kit::group(
				"Sounds",
				kit::card().child(kit::notice(
					kit::Level::Info,
					"Notification sounds and ringtones play in the main Serein app. This preview is silent.",
				)),
			))
			.child(kit::group(
				"Badges",
				kit::card().child(kit::switch(
					"unread-badge",
					"Enable Unread Message Badge",
					Some("App icon badges are not available in this preview yet."),
					false,
					false,
					cx,
					|_, _, _, _| {},
				)),
			))
	}
}
