//! Sign-in card and the hosted Discord login surface.
use crate::theme::{Icon, color, icon, palette, tint};
use crate::{Serein, backend};
use gpui::{prelude::*, *};

impl Serein {
	/// Hosted Discord login fills the window below a native header.
	#[cfg(not(target_os = "linux"))]
	pub(crate) fn render_login(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		if let Some(login) = &self.login {
			let size = window.viewport_size();
			login.resize_native(f32::from(size.width), f32::from(size.height));
		}
		div()
			.size_full()
			.bg(color(p.base))
			.text_color(color(p.text))
			.child(
				div()
					.h(px(platform::LOGIN_HEADER_HEIGHT))
					.px_4()
					.flex()
					.items_center()
					.gap_3()
					.child(icon(Icon::Serein, px(20.), color(p.accent)))
					.child(
						div()
							.flex_1()
							.flex()
							.flex_col()
							.child(
								div()
									.font_weight(FontWeight::SEMIBOLD)
									.text_color(color(p.text_strong))
									.child("Sign in with Discord"),
							)
							.child(
								div()
									.text_xs()
									.text_color(color(p.muted))
									.child("discord.com · temporary, private sign-in window"),
							),
					)
					.child(
						self.button("cancel-login", "Cancel", false)
							.on_click(cx.listener(|this, _, _, cx| {
								this.login = None;
								this.status = "Sign-in cancelled";
								cx.notify();
							})),
					),
			)
			.into_any_element()
	}

	pub(crate) fn render_sign_in(&self, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		// A pending keychain lookup does not block signing in; opening login stops it.
		let busy = self.backend_status.starts_with("Authenticating")
			|| self.status.starts_with("Verifying");
		let consent = self.authorized;
		div()
			.size_full()
			.bg(color(p.base))
			.text_color(color(p.text))
			.flex()
			.items_center()
			.justify_center()
			.child(
				div()
					.w(px(420.))
					.p_8()
					.rounded(px(16.))
					.bg(color(p.sidebar))
					.border_1()
					.border_color(color(p.border))
					.shadow_lg()
					.flex()
					.flex_col()
					.gap_4()
					.child(
						div()
							.flex()
							.flex_col()
							.items_center()
							.gap_3()
							.child(
								div()
									.size(px(56.))
									.rounded(px(17.))
									.bg(tint(p.accent, 0.16))
									.flex()
									.items_center()
									.justify_center()
									.child(
										div()
											.size(px(44.))
											.rounded(px(13.))
											.bg(color(p.accent))
											.flex()
											.items_center()
											.justify_center()
											.child(icon(Icon::Serein, px(28.), color(p.accent_text))),
									),
							)
							.child(
								div()
									.text_2xl()
									.font_weight(FontWeight::SEMIBOLD)
									.text_color(color(p.text_strong))
									.child("Welcome to Serein"),
							)
							.child(
								div()
									.text_sm()
									.text_color(color(p.muted))
									.child("Sign in with your Discord account to get started."),
							),
					)
					.child(
						self.button(
							"login",
							if busy {
								"Waiting for Discord…"
							} else {
								"Continue with Discord"
							},
							true,
						)
						.justify_center()
						.when(!consent || busy, |d| d.opacity(0.5).cursor_default())
						.on_click(cx.listener(|this, _, window, cx| this.open_login(window, cx))),
					)
					.child(
						div()
							.id("authorize")
							.focusable()
							.tab_stop(true)
							.focus(|d| d.border_color(color(p.accent)))
							.flex()
							.gap_3()
							.p_2()
							.rounded(px(8.))
							.border_1()
							.border_color(gpui::transparent_black())
							.cursor_pointer()
							.hover(|d| d.bg(color(p.hover)))
							.on_click(cx.listener(|this, _, _, cx| {
								this.authorized = !this.authorized;
								cx.notify();
							}))
							.child(
								div()
									.mt(px(2.))
									.size(px(18.))
									.flex_none()
									.rounded(px(5.))
									.border_2()
									.border_color(color(if consent { p.accent } else { p.muted }))
									.bg(color(if consent {
										p.accent
									} else {
										egui::Color32::TRANSPARENT
									}))
									.flex()
									.items_center()
									.justify_center()
									.text_xs()
									.text_color(color(p.accent_text))
									.when(consent, |d| d.child("✓")),
							)
							.child(
								div()
									.flex_1()
									.min_w_0()
									.flex()
									.flex_col()
									.gap_1()
									.child(
										div()
											.text_sm()
											.font_weight(FontWeight::MEDIUM)
											.text_color(color(p.text_strong))
											.child("I own this account and authorize this session."),
									)
									.child(div().text_xs().text_color(color(p.muted)).child(
										"Passwords and 2FA stay on Discord's own login page; only the session token is kept, in your OS credential store.",
									)),
							),
					)
					.child(
						div()
							.text_sm()
							.text_color(color(p.muted))
							.child(self.status),
					)
					.child(div().h(px(1.)).bg(color(p.border)))
					.child(
						div()
							.flex()
							.items_center()
							.justify_between()
							.child(
								div()
									.text_xs()
									.text_color(color(p.muted))
									.child("Unofficial client · GPUI experiment"),
							)
							.child(
								self.button("retry", "Retry saved login", false)
									.on_click(cx.listener(|this, _, _, cx| {
										this.backend = backend::Backend::start(false);
										this.backend_status = "";
										cx.notify();
									})),
							),
					),
			)
			.into_any_element()
	}

	fn open_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		if !self.authorized {
			self.status = "Confirm that you own this account before signing in.";
			cx.notify();
			return;
		}
		#[cfg(not(target_os = "linux"))]
		{
			let size = window.viewport_size();
			match platform::LoginView::open_native(
				window,
				f32::from(size.width),
				f32::from(size.height),
				|| backend::WAKE.notify_one(),
			) {
				Ok(login) => {
					// Stop any saved-login lookup; the hosted page supplies the session.
					self.backend = backend::Backend::idle();
					self.backend_status = "";
					self.login = Some(login);
					self.status = "Waiting for Discord login…";
				}
				Err(_) => self.status = "The platform login window could not be opened.",
			}
		}
		#[cfg(target_os = "linux")]
		{
			let _ = window;
			self.status = "Sign in using the main Serein app, then retry saved login.";
		}
		cx.notify();
	}
}
