//! User settings modal in the main app's layout: a searchable sidebar of pages on the left,
//! the selected page on the right, and the round close control with its Escape hint.
//! Page names, order, sections and search keywords match `crates/ui/src/settings.rs`.
//! Device choices are process-wide or live on [`Settings`]; `persist` saves them to the
//! experiment's own local store.
mod account;
mod appearance;
mod chat;
mod general;
mod keybinds;
pub mod kit;
mod notifications;
mod permissions;
mod profile;
mod storage;

pub(crate) use profile::demo_profile_edit;

use crate::theme::{self, Appearance, Icon, color, icon, palette, solid};
use crate::{Serein, input};
use gpui::{prelude::*, *};
use model::ReadingPreferences;
use ui::design::Variant;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Page {
	Account,
	Profile,
	General,
	#[default]
	Appearance,
	Chat,
	MessagingPermissions,
	Notifications,
	Activity,
	Voice,
	Keybinds,
	Storage,
	Updates,
	Extensions,
	Themes,
}
impl Page {
	/// Every page in sidebar order.
	pub const ALL: [Self; 14] = [
		Self::Account,
		Self::Profile,
		Self::MessagingPermissions,
		Self::Storage,
		Self::Appearance,
		Self::Chat,
		Self::Notifications,
		Self::Voice,
		Self::Keybinds,
		Self::Activity,
		Self::General,
		Self::Updates,
		Self::Themes,
		Self::Extensions,
	];
	const SECTIONS: [(&'static str, &'static [Self]); 3] = [
		(
			"User settings",
			&[
				Self::Account,
				Self::Profile,
				Self::MessagingPermissions,
				Self::Storage,
			],
		),
		(
			"App settings",
			&[
				Self::Appearance,
				Self::Chat,
				Self::Notifications,
				Self::Voice,
				Self::Keybinds,
				Self::Activity,
				Self::General,
				Self::Updates,
			],
		),
		("Customization", &[Self::Themes, Self::Extensions]),
	];
	pub fn label(self) -> &'static str {
		match self {
			Self::Account => "My Account",
			Self::Profile => "Profile",
			Self::General => "General",
			Self::Appearance => "Appearance",
			Self::Chat => "Chat",
			Self::MessagingPermissions => "Messaging Permissions",
			Self::Notifications => "Notifications",
			Self::Activity => "Game Activity",
			Self::Voice => "Voice & Video",
			Self::Keybinds => "Keybinds",
			Self::Storage => "Data & Privacy",
			Self::Updates => "Updates",
			Self::Extensions => "Extensions",
			Self::Themes => "Themes",
		}
	}
	fn description(self) -> &'static str {
		match self {
			Self::Account => "The Discord account signed in on this device.",
			Self::Profile => "Choose how you appear across Discord.",
			Self::General => "Startup, window and graphics behavior on this device.",
			Self::Appearance => "Theme, colours, window effects and layout.",
			Self::Chat => "How messages, media, links and scrolling behave.",
			Self::MessagingPermissions => {
				"Control who can contact you and how messages are filtered."
			}
			Self::Notifications => "Choose which notifications you receive and how they appear.",
			Self::Activity => "Show others what you are playing.",
			Self::Voice => "Microphone, speakers, camera and voice processing.",
			Self::Keybinds => "Keyboard shortcuts for Serein.",
			Self::Storage => "What Serein keeps on this device.",
			Self::Updates => "Keep Serein up to date on this device.",
			Self::Extensions => "Manage community plugins.",
			Self::Themes => "Choose a community theme.",
		}
	}
	fn matches(self, query: &str) -> bool {
		let keywords = match self {
			Self::Account => "my account profile logout",
			Self::Profile => "profile edit display name about me bio pronouns color colour",
			Self::General => {
				"general windows macos login menu bar startup autostart automatically open minimized minimize close tray background title bar caption window buttons graphics gpu adapter render discrete integrated hardware acceleration performance battery"
			}
			Self::Appearance => {
				"appearance customization primary accent hex window effects transparency blur theme dark light system mode zoom scale layout sidebar width people members member list reset colour color preset"
			}
			Self::Chat => {
				"chat messages media reading animate animated gifs autoplay hide image links confirm confirmation external browser smooth scrolling scroll speed motion trackpad wheel hidden channels channel list reset"
			}
			Self::MessagingPermissions => {
				"messaging permissions spam filters direct messages dm friend requests personalized connected games"
			}
			Self::Notifications => {
				"notifications desktop system alerts overview sounds badges message ring"
			}
			Self::Activity => "game activity playing osu status presence sharing",
			Self::Voice => {
				"voice video camera preview audio microphone speakers devices volume gain noise suppression push to talk"
			}
			Self::Storage => "data privacy local storage clear cache drafts credentials",
			Self::Updates => {
				"updates auto update release channel production stable nightly download restart version check diagnostics issue bug system info debug"
			}
			Self::Keybinds => {
				"system keybinds keyboard shortcuts custom default formatting navigation"
			}
			Self::Extensions => "extensions plugins shop store catalog import community tools",
			Self::Themes => "themes shop store catalog import community appearance colors",
		};
		keywords.contains(query)
	}
	/// `--demo-settings=NAME` picks the first page whose label contains `NAME`.
	pub fn find(name: &str) -> Option<Self> {
		let name = name.to_lowercase();
		Self::ALL
			.into_iter()
			.find(|page| page.label().to_lowercase().contains(&name))
	}
}

/// Modal state plus the device choices its pages edit.
pub struct Settings {
	pub open: bool,
	pub page: Page,
	query: Entity<input::Input>,
	/// Primary colour as `#RRGGBB`, applied on Enter.
	pub(crate) hex: Entity<input::Input>,
	focus: FocusHandle,
	/// Focus before opening, restored on close.
	restore: Option<FocusHandle>,
	scroll: ScrollHandle,
	/// OS alerts for mentions and DMs; off until opted in.
	pub notifications: bool,
	/// The main app's reading choices; the ones this frontend can honour are applied.
	pub reading: ReadingPreferences,
	pub show_hidden_channels: bool,
	/// Profile page draft and inputs.
	profile: profile::ProfileEditor,
	/// Messaging Permissions page state.
	messaging: permissions::Messaging,
}

impl Settings {
	pub fn new(window: &mut Window, cx: &mut Context<Serein>) -> Self {
		theme::set_system_appearance(window.appearance());
		cx.observe_window_appearance(window, |_, window, _| {
			theme::set_system_appearance(window.appearance());
			window.refresh();
		})
		.detach();
		let query = cx.new(input::Input::new);
		query.update(cx, |input, cx| input.set_placeholder("Search".into(), cx));
		cx.subscribe_in(
			&query,
			window,
			|this, _, event: &input::Event, window, cx| match event {
				input::Event::Changed => this.settings_query_changed(cx),
				input::Event::Cancel => this.close_settings(window, cx),
				_ => {}
			},
		)
		.detach();
		let hex = cx.new(input::Input::new);
		hex.update(cx, |input, cx| input.set_placeholder("#1A72E8".into(), cx));
		cx.subscribe(&hex, |this, _, _: &input::Submit, cx| this.apply_hex(cx))
			.detach();
		let profile = profile::ProfileEditor::new(window, cx);
		Self {
			open: false,
			page: Page::default(),
			query,
			hex,
			focus: cx.focus_handle(),
			restore: None,
			scroll: ScrollHandle::new(),
			notifications: false,
			reading: ReadingPreferences::default(),
			show_hidden_channels: false,
			profile,
			messaging: permissions::Messaging::default(),
		}
	}

	/// With `demo`: `--demo-settings[=PAGE]` opens the modal, `--demo-light`/`--demo-dark`
	/// pick a mode and `--demo-theme=KEY` a colour preset.
	pub fn apply_demo_flags(&mut self) {
		let args = std::env::args().collect::<Vec<_>>();
		let flag = |name: &str| args.iter().any(|arg| arg == name);
		if flag("--demo-light") {
			theme::set_appearance(Appearance::Light);
		} else if flag("--demo-dark") {
			theme::set_appearance(Appearance::Dark);
		}
		if let Some(variant) = args
			.iter()
			.find_map(|arg| arg.strip_prefix("--demo-theme="))
			.and_then(Variant::from_key)
		{
			ui::design::set_variant(variant);
		}
		if let Some(page) = args.iter().find_map(|arg| {
			arg.strip_prefix("--demo-settings")
				.map(|rest| rest.trim_start_matches('='))
		}) {
			self.open = true;
			self.page = Page::find(page).unwrap_or_default();
		}
	}
}

impl Serein {
	pub(crate) fn open_settings(
		&mut self,
		page: Option<Page>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		if !self.settings.open {
			self.settings.restore = window.focused(cx);
		}
		self.settings.open = true;
		if let Some(page) = page {
			self.show_settings_page(page);
		}
		self.close_switcher(window, cx);
		window.focus(&self.settings.focus, cx);
		cx.notify();
	}

	pub(crate) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		if !self.settings.open {
			return;
		}
		self.settings.open = false;
		// The main app asks for messaging permissions again on the next open.
		self.settings.messaging.requested = false;
		self.settings
			.query
			.update(cx, |input, cx| input.set_value(String::new(), cx));
		if let Some(previous) = self.settings.restore.take() {
			window.focus(&previous, cx);
		}
		cx.notify();
	}

	fn show_settings_page(&mut self, page: Page) {
		if self.settings.page != page {
			if page == Page::MessagingPermissions {
				self.settings.messaging.requested = false;
			}
			self.settings.page = page;
			self.settings.scroll.set_offset(point(px(0.), px(0.)));
		}
	}

	fn settings_query_changed(&mut self, cx: &mut Context<Self>) {
		let query = self.settings.query.read(cx).value().trim().to_lowercase();
		if !self.settings.page.matches(&query)
			&& let Some(page) = Page::ALL.into_iter().find(|page| page.matches(&query))
		{
			self.show_settings_page(page);
		}
		cx.notify();
	}

	/// Toggles desktop alerts; `persist` saves the opt-in.
	pub(crate) fn set_notifications(&mut self, on: bool) {
		self.settings.notifications = on;
		self.alerts.set_enabled(on);
	}

	pub(crate) fn render_settings(
		&mut self,
		window: &mut Window,
		cx: &mut Context<Self>,
	) -> Option<impl IntoElement + use<>> {
		if !self.settings.open {
			return None;
		}
		let p = palette();
		let viewport = window.viewport_size();
		let width = (viewport.width - px(32.)).clamp(px(280.), px(1100.));
		let height = (viewport.height - px(40.)).clamp(px(240.), px(820.));
		let page = self.settings.page;
		let query = self.settings.query.read(cx).value().trim().to_lowercase();
		let found = Page::ALL.into_iter().any(|page| page.matches(&query));
		let content = if found {
			self.settings_page(page, window, cx)
		} else {
			div()
				.flex()
				.flex_col()
				.gap_1()
				.child(
					div()
						.text_size(px(16.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(color(p.text_strong))
						.child("No settings found"),
				)
				.child(
					div()
						.text_size(px(14.))
						.text_color(color(p.muted))
						.child("Try theme, notifications, voice, or cache."),
				)
		};
		let demo = self.state.demo;
		Some(deferred(
			div()
				.id("settings-backdrop")
				.occlude()
				.absolute()
				.inset_0()
				.bg(hsla(0., 0., 0., 0.7))
				.flex()
				.items_center()
				.justify_center()
				.font_family(theme::FONT)
				.on_mouse_down(
					MouseButton::Left,
					cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
				)
				.child(
					div()
						.id("settings")
						.track_focus(&self.settings.focus)
						.key_context("SereinSettings")
						.w(width)
						.h(height)
						.rounded(px(12.))
						.bg(solid(p.chat))
						.border_1()
						.border_color(color(p.border))
						.shadow_lg()
						.overflow_hidden()
						.flex()
						.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
						.on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
							if event.keystroke.key == "escape" {
								this.close_settings(window, cx);
								cx.stop_propagation();
							}
						}))
						.child(self.settings_navigation(&query, cx))
						.child(
							div()
								.flex_1()
								.min_w_0()
								.h_full()
								.pl_10()
								.pr_5()
								.pt_6()
								.flex()
								.flex_col()
								.child(
									div()
										.flex()
										.items_start()
										.gap_4()
										.child(
											div()
												.flex_1()
												.min_w_0()
												.flex()
												.flex_col()
												.gap(px(2.))
												.child(
													div()
														.text_size(px(20.))
														.font_weight(FontWeight::SEMIBOLD)
														.text_color(color(p.text_strong))
														.child(page.label()),
												)
												.child(
													div()
														.text_size(px(13.))
														.text_color(color(p.muted))
														.child(page.description()),
												),
										)
										.child(close_control(cx)),
								)
								.child(
									div()
										.id(("settings-content", page as usize))
										.track_scroll(&self.settings.scroll)
										.mt_4()
										.flex_1()
										.min_h_0()
										.overflow_y_scroll()
										.child(
											div()
												.w_full()
												.max_w(px(720.))
												.pr_2()
												.pb_6()
												.flex()
												.flex_col()
												.gap_3()
												.child(content)
												.when(demo, |d| {
													d.child(div().pt_4().child(kit::hint(
														"Offline preview · changes stay in this session and are never sent.",
													)))
												}),
										),
								),
						),
				),
		))
	}

	fn settings_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) -> Div {
		match page {
			Page::Account => self.settings_account(cx),
			Page::Profile => self.settings_profile(window, cx),
			Page::MessagingPermissions => self.settings_permissions(window, cx),
			Page::Storage => self.settings_storage(window, cx),
			Page::Appearance => self.settings_appearance(window, cx),
			Page::Chat => self.settings_chat(cx),
			Page::Notifications => self.settings_notifications(cx),
			Page::Keybinds => self.settings_keybinds(cx),
			Page::General => self.settings_general(window, cx),
			Page::Voice => kit::empty_state(
				Icon::Speaker,
				"Voice & Video stay in the main Serein app",
				"This preview does not join calls, so it has no microphone, speaker or camera settings.",
			),
			Page::Activity => kit::empty_state(
				Icon::Users,
				"Game activity stays in the main Serein app",
				"This preview never scans running programs or shares what you are playing.",
			),
			Page::Updates => kit::empty_state(
				Icon::Download,
				"Updates are managed by the main Serein app",
				"This GPUI preview is built from source and does not download updates.",
			),
			Page::Themes | Page::Extensions => kit::empty_state(
				Icon::Gear,
				"Community add-ons stay in the main Serein app",
				"The GPUI preview does not run themes or plugins. Built-in colour presets are under Appearance.",
			),
		}
	}

	fn settings_navigation(&self, query: &str, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let current = self.settings.page;
		let sections = Page::SECTIONS
			.iter()
			.filter_map(|(heading, pages)| {
				let visible = pages
					.iter()
					.copied()
					.filter(|page| page.matches(query))
					.collect::<Vec<_>>();
				(!visible.is_empty()).then(|| {
					div()
						.pt(px(6.))
						.flex()
						.flex_col()
						.gap(px(2.))
						.child(div().pb(px(2.)).child(kit::eyebrow(heading)))
						.children(visible.into_iter().map(|page| {
							nav_item(page, page == current).on_click(cx.listener(
								move |this, _, _, cx| {
									this.show_settings_page(page);
									cx.notify();
								},
							))
						}))
				})
			})
			.collect::<Vec<_>>();
		let exit = if self.state.demo {
			"Exit preview"
		} else {
			"Log out"
		};
		div()
			.id("settings-navigation")
			.w(px(232.))
			.h_full()
			.flex_none()
			.bg(solid(p.sidebar))
			.pl_3()
			.pr_2()
			.pt_5()
			.pb_4()
			.overflow_y_scroll()
			.flex()
			.flex_col()
			.gap(px(2.))
			.child(
				div()
					.h(px(36.))
					.px_2()
					.rounded(px(6.))
					.bg(color(p.raised))
					.flex()
					.items_center()
					.gap(px(6.))
					.text_size(px(14.))
					.child(div().flex_1().min_w_0().child(self.settings.query.clone()))
					.child(icon(Icon::Search, px(16.), color(p.muted))),
			)
			.child(div().h_3())
			.children(sections)
			.child(div().my_2().h(px(1.)).bg(color(p.border)))
			.child(
				div()
					.id("settings-log-out")
					.h(px(32.))
					.px(px(10.))
					.rounded(px(4.))
					.flex()
					.items_center()
					.justify_between()
					.cursor_pointer()
					.focusable()
					.tab_stop(true)
					.hover(|d| d.bg(color(p.hover)))
					.text_size(px(15.))
					.font_weight(FontWeight::MEDIUM)
					.text_color(color(p.danger))
					.on_click(cx.listener(|this, _, window, cx| this.settings_log_out(window, cx)))
					.child(exit),
			)
			.child(
				div()
					.pt_3()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child(format!(
						"Serein {} · GPUI preview",
						env!("CARGO_PKG_VERSION")
					)),
			)
			.child(
				div()
					.text_size(px(11.))
					.text_color(color(p.muted))
					.child("Unofficial · not endorsed by Discord"),
			)
	}

	/// Log out (or leave the offline preview) from the sidebar or the account page.
	fn settings_log_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.close_settings(window, cx);
		if self.state.demo {
			cx.quit();
		} else {
			self.confirm_log_out(window, cx);
		}
	}

	fn apply_hex(&mut self, cx: &mut Context<Self>) {
		let text = self.settings.hex.read(cx).value().trim().to_owned();
		match parse_hex(&text) {
			Some(rgb) => {
				ui::design::set_primary_color(Some(rgb));
				self.settings
					.hex
					.update(cx, |input, cx| input.set_value(format_hex(rgb), cx));
			}
			None => self.notify_user("Enter a colour like #1A72E8."),
		}
		cx.notify();
	}
}

/// `#RRGGBB` or `RRGGBB`, case-insensitive.
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
	let digits = text.strip_prefix('#').unwrap_or(text);
	if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
		return None;
	}
	let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
	Some([channel(0)?, channel(2)?, channel(4)?])
}

pub fn format_hex([r, g, b]: [u8; 3]) -> String {
	format!("#{r:02X}{g:02X}{b:02X}")
}

/// Sidebar entry; the open page gets the selected surface and an accent rail at the left.
fn nav_item(page: Page, selected: bool) -> Stateful<Div> {
	let p = palette();
	div()
		.id(page.label())
		.h(px(34.))
		.px_3()
		.relative()
		.flex_none()
		.rounded(px(8.))
		.flex()
		.items_center()
		.cursor_pointer()
		.focusable()
		.tab_stop(true)
		.text_size(px(15.))
		.font_weight(FontWeight::MEDIUM)
		.when(selected, |d| {
			d.bg(color(p.selected))
				.text_color(color(p.text_strong))
				.child(
					div()
						.absolute()
						.left_0()
						.top(px(9.))
						.w(px(3.))
						.h(px(16.))
						.rounded(px(2.))
						.bg(color(p.accent)),
				)
		})
		.when(!selected, |d| {
			d.text_color(color(p.muted))
				.hover(|d| d.bg(color(p.hover)).text_color(color(p.text)))
		})
		.focus(|d| d.border_1().border_color(color(p.accent)))
		.child(page.label())
}

/// The round close button with the "ESC" hint underneath.
fn close_control(cx: &mut Context<Serein>) -> impl IntoElement {
	let p = palette();
	div()
		.id("settings-close")
		.group("settings-close")
		.w(px(40.))
		.flex_none()
		.flex()
		.flex_col()
		.items_center()
		.gap_1()
		.cursor_pointer()
		.tooltip(crate::tooltip("Close settings (Esc)"))
		.on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)))
		.child(
			div()
				.size(px(36.))
				.rounded_full()
				.border_2()
				.border_color(color(p.muted))
				.flex()
				.items_center()
				.justify_center()
				.group_hover("settings-close", |d| {
					d.bg(color(p.hover)).border_color(color(p.text))
				})
				.child(icon(Icon::Close, px(16.), color(p.muted))),
		)
		.child(
			div()
				.text_size(px(11.))
				.font_weight(FontWeight::SEMIBOLD)
				.text_color(color(p.muted))
				.child("ESC"),
		)
}

#[cfg(test)]
mod tests {
	use super::{Page, format_hex, parse_hex};
	use crate::theme::{self, Appearance};
	use gpui::WindowAppearance;
	use ui::design::{Variant, colors};

	#[test]
	fn appearance_and_variant_choose_the_palette() {
		theme::set_system_appearance(WindowAppearance::Light);
		theme::set_appearance(Appearance::System);
		assert!(!theme::dark());
		assert_eq!(theme::palette(), colors(false, ui::design::variant()));
		theme::set_system_appearance(WindowAppearance::VibrantDark);
		assert!(theme::dark());
		theme::set_appearance(Appearance::Light);
		assert!(!theme::dark());
		theme::set_appearance(Appearance::Dark);
		ui::design::set_variant(Variant::Eclipse);
		assert_eq!(theme::palette(), colors(true, Variant::Eclipse));
		ui::design::set_variant(Variant::Standard);
		theme::set_appearance(Appearance::System);
		assert_eq!(theme::appearance(), Appearance::System);
	}

	#[test]
	fn pages_match_the_main_app_and_search_by_keyword() {
		assert_eq!(Page::find("chat"), Some(Page::Chat));
		assert_eq!(Page::find("account"), Some(Page::Account));
		assert_eq!(Page::find("nope"), None);
		assert!(Page::Chat.matches("hidden channels"));
		assert!(Page::Appearance.matches("accent"));
		assert!(!Page::Keybinds.matches("accent"));
		for (_, pages) in Page::SECTIONS {
			for page in pages.iter() {
				assert!(Page::ALL.contains(page));
			}
		}
	}

	#[test]
	fn hex_colours_round_trip() {
		assert_eq!(parse_hex("#1a72e8"), Some([0x1a, 0x72, 0xe8]));
		assert_eq!(parse_hex("1A72E8"), Some([0x1a, 0x72, 0xe8]));
		assert_eq!(parse_hex("#1A72E"), None);
		assert_eq!(parse_hex("#GG72E8"), None);
		assert_eq!(format_hex([0x1a, 0x72, 0xe8]), "#1A72E8");
	}
}
