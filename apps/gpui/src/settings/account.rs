//! My Account: the profile card with its banner, and the session group with Log out.
use super::{Page, kit};
use crate::Serein;
use crate::sidebar::avatar;
use crate::theme::{color, palette};
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_account(&mut self, cx: &mut Context<Self>) -> Div {
		let p = palette();
		let demo = self.state.demo;
		let user = self.state.user.clone();
		let name = user
			.as_ref()
			.map_or("Your account", |user| self.state.user_display_name(user))
			.to_owned();
		let username = user
			.as_ref()
			.map(|user| user.name.clone())
			.unwrap_or_default();
		let field = |label: &str, value: String| {
			div()
				.flex()
				.flex_col()
				.gap(px(2.))
				.child(kit::eyebrow(label))
				.child(
					div()
						.text_size(px(15.))
						.text_color(color(p.text_strong))
						.overflow_hidden()
						.whitespace_nowrap()
						.text_ellipsis()
						.child(value),
				)
		};
		let card = div()
			.w_full()
			.relative()
			.rounded(px(8.))
			.bg(color(p.raised))
			.border_1()
			.border_color(color(p.border))
			.overflow_hidden()
			.flex()
			.flex_col()
			.child(div().h(px(96.)).bg(color(p.accent)))
			.child(
				div()
					.absolute()
					.left(px(12.))
					.top(px(52.))
					.p(px(4.))
					.rounded_full()
					.bg(color(p.raised))
					.child(avatar(&name, 80., user.as_ref())),
			)
			.child(
				div()
					.px_4()
					.pt_3()
					.pb_4()
					.flex()
					.flex_col()
					.gap_4()
					.child(
						div()
							.pl(px(96.))
							.flex()
							.flex_col()
							.gap(px(2.))
							.child(
								div()
									.text_size(px(20.))
									.font_weight(FontWeight::SEMIBOLD)
									.text_color(color(p.text_strong))
									.overflow_hidden()
									.whitespace_nowrap()
									.text_ellipsis()
									.child(name.clone()),
							)
							.child(div().text_size(px(13.)).text_color(color(p.muted)).child(
								if demo {
									"Offline preview · synthetic account"
								} else {
									"Signed in with your Discord account"
								},
							)),
					)
					.child(
						div()
							.px_4()
							.py_3()
							.rounded(px(8.))
							.bg(color(p.chat))
							.flex()
							.flex_col()
							.gap(px(10.))
							.child(field("Display name", name))
							.child(div().h(px(1.)).bg(color(p.border)))
							.child(field("Username", username))
							.child(div().h(px(1.)).bg(color(p.border)))
							.child(field(
								"Email, password and security",
								"Managed in Discord".into(),
							)),
					)
					.child(
						div().flex().justify_end().child(
							kit::button(
								"edit-profile",
								"Edit profile",
								kit::ButtonKind::Outline,
								true,
							)
							.on_click(cx.listener(|this, _, _, cx| {
								this.show_settings_page(Page::Profile);
								cx.notify();
							})),
						),
					),
			);
		let label = if demo { "Exit preview" } else { "Log out" };
		div()
			.flex()
			.flex_col()
			.gap_3()
			.child(card)
			.child(kit::group(
				"Session",
				kit::card().child(kit::row(
					label,
					Some(if demo {
						"Closes the offline fixture. Nothing is stored for the preview."
					} else {
						"Removes the saved login from this device's credential store."
					}),
					kit::button("account-log-out", label, kit::ButtonKind::Danger, true).on_click(
						cx.listener(|this, _, window, cx| this.settings_log_out(window, cx)),
					),
				)),
			))
	}
}
