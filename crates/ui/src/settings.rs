//! User settings modal in Discord's layout: a sidebar of pages on the left, the selected page
//! on the right, and a round close control with its Escape hint.
use crate::{MessagingUi, design, i18n::Language, icons};
use client_core::State;
use egui::RichText;
use std::cell::RefCell;

#[cfg(test)]
mod search_tests {
	use super::*;

	#[test]
	fn translated_control_labels_find_their_real_page_and_target() {
		for (language, query, key, page) in [
			(
				Language::French,
				"synchroniser tout le compte",
				"force-sync-account",
				Page::Storage,
			),
			(
				Language::English,
				"start minimized",
				"general-start-minimized",
				Page::General,
			),
			(
				Language::French,
				"fuseau horaire personnalisé",
				"timezone-custom",
				Page::LanguageTime,
			),
			(
				Language::Spanish,
				"zona horaria del sistema",
				"timezone-system",
				Page::LanguageTime,
			),
			(
				Language::English,
				"noise suppression",
				"voice-voice-processing-controls-noise-suppression",
				Page::Voice,
			),
			(
				Language::Spanish,
				"solicitudes de amistad",
				"messaging-permissions-tab-friend-requests",
				Page::MessagingPermissions,
			),
		] {
			let query = query.to_lowercase();
			let target = search_targets(&query, language)
				.find(|target| target.label == key)
				.unwrap();
			assert_eq!(target.page, page);
			let mut settings = Settings {
				query,
				..Default::default()
			};
			settings.refresh_search(language);
			assert!(settings.search_matches_page(page));
		}
		assert_eq!(search_targets("  ", Language::English).count(), 0);
		for target in SEARCH_TARGETS {
			for key in [target.label, target.anchor] {
				for language in Language::ALL {
					assert!(
						!language.text(key).starts_with("Unknown localization key:"),
						"{language:?}: {key}"
					);
				}
			}
		}
	}

	#[test]
	fn typing_preserves_the_current_page_and_drafts_until_a_result_is_activated() {
		fn text_shapes(shape: &egui::Shape, result: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => {
					result.push((text.galley.job.text.clone(), text.visual_bounding_rect()))
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						text_shapes(shape, result);
					}
				}
				_ => {}
			}
		}
		crate::i18n::set_current(Language::English);
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		view.settings.page = Page::Profile;
		view.settings.timezone_query = "unsaved region".into();
		{
			let mut frame = |events| {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(520.0, 600.0),
						)),
						focused: true,
						events,
						..Default::default()
					},
					|ui| view.settings_search(ui),
				);
				let mut shapes = Vec::new();
				for shape in &output.shapes {
					text_shapes(&shape.shape, &mut shapes);
				}
				output.drop_without_applying_deltas();
				(
					shapes,
					view.settings.page,
					view.settings.timezone_query.clone(),
				)
			};
			frame(vec![]);
			let click = |point, pressed| {
				vec![
					egui::Event::PointerMoved(point),
					egui::Event::PointerButton {
						pos: point,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				]
			};
			let search = egui::pos2(25.0, 18.0);
			frame(click(search, true));
			frame(click(search, false));
			let (labels, page_before_activation, draft_before_activation) =
				frame(vec![egui::Event::Text("custom timezone".into())]);
			assert_eq!(
				page_before_activation,
				Page::Profile,
				"typing must not navigate"
			);
			assert_eq!(draft_before_activation, "unsaved region");
			let result = labels
				.iter()
				.find(|(text, _)| text.starts_with("Custom timezone · "))
				.expect("matching translated control result")
				.1
				.center();
			frame(click(result, true));
			frame(click(result, false));
		}
		assert_eq!(view.settings.query, "custom timezone");
		assert_eq!(view.settings.page, Page::LanguageTime);
		assert_eq!(view.settings.search_jump.unwrap().anchor, "timezone-custom");
		assert_eq!(view.settings.timezone_query, "unsaved region");
	}

	#[test]
	fn search_jump_scrolls_to_a_real_control_once_without_activating_it() {
		let ctx = egui::Context::default();
		crate::i18n::set_current(Language::English);
		let target = search_targets("start minimized", Language::English)
			.next()
			.unwrap();
		let mut selected = false;
		let offset = {
			let mut frame = |jump, time| {
				let mut offset = 0.0;
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(400.0, 180.0),
						)),
						time: Some(time),
						..Default::default()
					},
					|ui| {
						offset = egui::ScrollArea::vertical()
							.id_salt("search-jump-test")
							.max_height(100.0)
							.show(ui, |ui| {
								let _scope = SearchScrollScope::new(jump, Language::English);
								ui.add_space(700.0);
								design::switch(ui, target.anchor, None, &mut selected);
								ui.add_space(300.0);
							})
							.state
							.clamped_offset()
							.y;
					},
				);
				output.drop_without_applying_deltas();
				offset
			};
			frame(None, 0.0);
			frame(Some(target), 0.05);
			let mut offset = 0.0;
			for step in 2..30 {
				offset = frame(None, f64::from(step) * 0.05);
			}
			offset
		};
		assert!(
			offset > 500.0,
			"expected control to be scrolled into view, got {offset}"
		);
		assert!(!selected, "search navigation must not activate settings");
		SEARCH_SCROLL_TARGET.with(|slot| assert!(slot.borrow().is_none()));
	}

	#[test]
	fn spanish_search_navigates_after_preferences_load_without_discarding_the_draft() {
		use client_core::{Command, auth::AuthState};
		use model::messaging_permissions::{Change, Snapshot};
		crate::i18n::set_current(Language::Spanish);
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = AuthState::Authenticated;
		state.gateway_connected = true;
		state.messaging_permissions.draft = Some(Change::Everyone(false));
		view.language = Language::Spanish;
		view.settings.open = true;
		view.settings.page = Page::Profile;
		view.settings.query = "solicitudes de amistad".into();
		let target = search_targets(&view.settings.query, view.language)
			.find(|target| target.label == "messaging-permissions-tab-friend-requests")
			.unwrap();
		view.activate_search_target(target);
		let mut commands = Vec::new();
		let input = |time| egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(900.0, 500.0),
			)),
			focused: true,
			time: Some(time),
			..Default::default()
		};
		ctx.run_ui(input(0.0), |ui| {
			view.show_settings(ui.ctx(), &mut state, &mut commands)
		})
		.drop_without_applying_deltas();
		assert_eq!(view.settings.page, Page::MessagingPermissions);
		assert!(
			view.settings.messaging_permissions.jump
				== Some(crate::messaging_permissions::Tab::FriendRequests)
		);
		assert_eq!(
			state.messaging_permissions.draft,
			Some(Change::Everyone(false))
		);
		let request = commands
			.iter()
			.find_map(|command| match command {
				Command::MessagingPermissions {
					request,
					change: None,
				} => Some(*request),
				_ => None,
			})
			.expect("search navigates through the existing preferences read");
		assert!(commands.iter().all(|command| !matches!(
			command,
			Command::MessagingPermissions {
				change: Some(_),
				..
			}
		)));
		state.apply_messaging_permissions(request, Ok(Snapshot::default()));
		let heading = Language::Spanish
			.text("messaging-permissions-heading-friend-requests")
			.to_uppercase();
		let mut visible = false;
		for step in 1..=30 {
			let output = ctx.run_ui(input(f64::from(step) * 0.05), |ui| {
				view.show_settings(ui.ctx(), &mut state, &mut commands)
			});
			for clipped in &output.shapes {
				if let egui::Shape::Text(text) = &clipped.shape
					&& text.galley.job.text == heading
					&& clipped.clip_rect.intersects(text.visual_bounding_rect())
				{
					visible = true;
				}
			}
			output.drop_without_applying_deltas();
			if visible {
				break;
			}
		}
		assert!(
			visible,
			"the translated target should scroll into the actual settings viewport"
		);
		assert_eq!(
			state.messaging_permissions.draft,
			Some(Change::Everyone(false))
		);
		assert_eq!(
			state
				.messaging_permissions
				.snapshot
				.as_ref()
				.unwrap()
				.friend_source_flags,
			Snapshot::default().friend_source_flags
		);
		assert!(commands.iter().all(|command| !matches!(
			command,
			Command::MessagingPermissions {
				change: Some(_),
				..
			}
		)));
		crate::i18n::set_current(Language::English);
	}

	#[test]
	fn manual_content_interaction_cancels_a_pending_search_jump() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		view.settings.search_jump =
			Some(target(Page::Profile, "profile-edit-form-profile-picture"));
		view.settings.messaging_permissions.jump =
			Some(crate::messaging_permissions::Tab::FriendRequests);
		let point = egui::pos2(50.0, 50.0);
		ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(200.0, 180.0),
				)),
				events: vec![
					egui::Event::PointerMoved(point),
					egui::Event::PointerButton {
						pos: point,
						button: egui::PointerButton::Primary,
						pressed: true,
						modifiers: egui::Modifiers::NONE,
					},
				],
				..Default::default()
			},
			|ui| view.cancel_search_jump_on_content_interaction(ui),
		)
		.drop_without_applying_deltas();
		assert!(view.settings.search_jump.is_none());
		assert!(view.settings.messaging_permissions.jump.is_none());
	}

	#[test]
	fn indexed_settings_anchors_reach_their_production_controls() {
		crate::i18n::set_current(Language::English);
		for &target in SEARCH_TARGETS {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			let mut state = test_support::demo_state();
			state.messaging_permissions.snapshot =
				Some(model::messaging_permissions::Snapshot::default());
			view.settings.open = true;
			view.activate_search_target(target);
			if target.page == Page::Profile {
				state.own_profile.loading = true;
			}
			let mut commands = Vec::new();
			let mut frame = |view: &mut MessagingUi, state: &mut State| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(1000.0, 650.0),
						)),
						..Default::default()
					},
					|ui| view.show_settings(ui.ctx(), state, &mut commands),
				)
				.drop_without_applying_deltas();
			};
			frame(&mut view, &mut state);
			if target.page == Page::Profile {
				assert_eq!(
					view.settings.search_jump,
					Some(target),
					"profile target must survive loading"
				);
				state.own_profile.loading = false;
				state.own_profile.data = Some(crate::profiles::synthetic(
					state.user.as_ref().unwrap(),
					None,
				));
				frame(&mut view, &mut state);
			}
			if target.page == Page::MessagingPermissions {
				assert!(
					view.settings.messaging_permissions.jump.is_some(),
					"missing tab jump for {target:?}"
				);
				frame(&mut view, &mut state);
				assert!(
					view.settings.messaging_permissions.jump.is_none(),
					"tab heading not found for {target:?}"
				);
			} else {
				// The first modal pass measures an invisible area; navigation must survive it.
				if view.settings.search_jump.is_some() {
					frame(&mut view, &mut state);
				}
				assert!(
					view.settings.search_jump.is_none(),
					"no production control found for {target:?}"
				);
			}
		}
	}

	#[test]
	fn tiny_settings_modal_keeps_its_accessible_close_button_on_screen() {
		crate::i18n::set_current(Language::English);
		for size in [egui::vec2(200.0, 180.0), egui::vec2(250.0, 260.0)] {
			let ctx = egui::Context::default();
			ctx.enable_accesskit();
			let mut view = MessagingUi::default();
			let mut state = test_support::demo_state();
			view.settings.open = true;
			let input = |events| egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
				focused: true,
				events,
				..Default::default()
			};
			let label = Language::English.text("settings-close-control-close-settings-esc");
			let mut bounds = None;
			for _ in 0..8 {
				let output = ctx.run_ui(input(vec![]), |ui| {
					view.show_settings(ui.ctx(), &mut state, &mut Vec::new())
				});
				if let Some(update) = output.platform_output.accesskit_update.as_ref() {
					bounds = bounds.or_else(|| {
						update.nodes.iter().find_map(|(_, node)| {
							(node.label() == Some(label.as_str()))
								.then(|| node.bounds())
								.flatten()
						})
					});
				}
				output.drop_without_applying_deltas();
			}
			let bounds = bounds.expect("close button retains an accessible label");
			assert!(
				bounds.x0 >= 0.0
					&& bounds.y0 >= 0.0
					&& bounds.x1 <= f64::from(size.x)
					&& bounds.y1 <= f64::from(size.y),
				"close bounds {bounds:?}, viewport {size:?}"
			);
			let point = egui::pos2(
				((bounds.x0 + bounds.x1) * 0.5) as f32,
				((bounds.y0 + bounds.y1) * 0.5) as f32,
			);
			for pressed in [true, false] {
				ctx.run_ui(
					input(vec![
						egui::Event::PointerMoved(point),
						egui::Event::PointerButton {
							pos: point,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					]),
					|ui| view.show_settings(ui.ctx(), &mut state, &mut Vec::new()),
				)
				.drop_without_applying_deltas();
			}
			assert!(!view.settings.open, "close remains usable in {size:?}");
		}
	}

	#[test]
	fn tiny_settings_modal_supports_keyboard_close_and_scrolls_to_a_control() {
		crate::i18n::set_current(Language::English);
		let ctx = egui::Context::default();
		ctx.enable_accesskit();
		let mut view = MessagingUi::default();
		let mut state = test_support::demo_state();
		view.settings.open = true;
		let input = |time, events| egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(200.0, 180.0),
			)),
			focused: true,
			time: Some(time),
			events,
			..Default::default()
		};
		view.activate_search_target(target(
			Page::Storage,
			"settings-storage-page-plaintext-login",
		));
		let label = Language::English.text("settings-storage-page-plaintext-login");
		let mut visible = false;
		let mut geometry = None;
		let mut close_id = None;
		for step in 0..30 {
			let output = ctx.run_ui(input(f64::from(step) * 0.05, vec![]), |ui| {
				view.show_settings(ui.ctx(), &mut state, &mut Vec::new());
			});
			if let Some(update) = output.platform_output.accesskit_update.as_ref() {
				let close = Language::English.text("settings-close-control-close-settings-esc");
				if let Some((id, _)) = update
					.nodes
					.iter()
					.find(|(_, node)| node.label() == Some(close.as_str()))
				{
					close_id = Some(*id);
				}
			}
			for clipped in &output.shapes {
				if let egui::Shape::Text(text) = &clipped.shape
					&& text.galley.job.text == label
				{
					geometry = Some((text.visual_bounding_rect(), clipped.clip_rect));
					visible |= clipped.clip_rect.intersects(text.visual_bounding_rect());
				}
			}
			output.drop_without_applying_deltas();
		}
		assert!(
			visible,
			"search target should be scrollable within the tiny modal: {geometry:?}"
		);
		let close_id = close_id.expect("close is exposed to accessibility");
		let key = |key, pressed| egui::Event::Key {
			key,
			physical_key: None,
			pressed,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		let mut focused = false;
		for step in 30..94 {
			ctx.run_ui(
				input(
					f64::from(step) * 0.05,
					vec![key(egui::Key::Tab, true), key(egui::Key::Tab, false)],
				),
				|ui| {
					view.show_settings(ui.ctx(), &mut state, &mut Vec::new());
				},
			)
			.drop_without_applying_deltas();
			if ctx.memory(|memory| memory.focused().map(|id| id.accesskit_id()) == Some(close_id)) {
				focused = true;
				break;
			}
		}
		assert!(focused, "keyboard navigation reaches the close button");
		ctx.run_ui(
			input(
				5.0,
				vec![key(egui::Key::Enter, true), key(egui::Key::Enter, false)],
			),
			|ui| {
				view.show_settings(ui.ctx(), &mut state, &mut Vec::new());
			},
		)
		.drop_without_applying_deltas();
		assert!(!view.settings.open, "keyboard activation closes settings");
	}
}

#[cfg(test)]
mod clock_tests {
	#[test]
	fn settled_language_page_wakes_at_the_next_minute_and_suspends_its_timer_when_minimized() {
		let ctx = egui::Context::default();
		let _zone = crate::local_time::pin_timezone("UTC");
		let _now =
			crate::local_time::pin_now(time::OffsetDateTime::from_unix_timestamp(30).unwrap());
		let mut app = crate::MessagingUi::default();
		let mut frame = |minimized| {
			let mut input = egui::RawInput::default();
			input
				.viewports
				.get_mut(&egui::ViewportId::ROOT)
				.unwrap()
				.minimized = Some(minimized);
			let output = ctx.run_ui(input, |ui| app.language_time_settings(ui));
			let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
			output.drop_without_applying_deltas();
			delay
		};
		for _ in 0..3 {
			frame(false);
		}
		assert_eq!(frame(false), std::time::Duration::from_secs(30));
		for _ in 0..3 {
			frame(true);
		}
		assert_eq!(frame(true), std::time::Duration::MAX);
	}
}

#[derive(Default)]
pub(super) struct Settings {
	pub open: bool,
	page: Page,
	query: String,
	search_jump: Option<SearchTarget>,
	search_cache: Option<(Language, String)>,
	search_results: Vec<SearchResult>,
	search_pages: u32,
	language_time_format: Option<(model::TimeFormat, Language, model::TimeZonePreference)>,
	timezone_query: String,
	timezone_filter: Option<String>,
	timezone_matches: Vec<usize>,
	timezone_fixed: String,
	timezone_fixed_for: Option<i16>,
	custom_timezone: Option<model::TimeZonePreference>,
	clock_preview: Option<(i64, u64, String, String)>,
	pub(super) editor: crate::profile_edit::Editor,
	pub(super) server_editor: crate::server_profile::Editor,
	pub(super) account: crate::account_settings::AccountUi,
	pub(super) owned_cosmetics: crate::owned_cosmetics::Navigation,
	server_profile: bool,
	ime_active: bool,
	pub(super) notifications: crate::notification_settings::Navigation,
	pub(super) messaging_permissions: crate::messaging_permissions::Navigation,
	pub(super) games: crate::registered_games::Page,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
	Account,
	Profile,
	General,
	#[default]
	Appearance,
	Chat,
	MessagingPermissions,
	Privacy,
	ContentFilters,
	Connections,
	Accessibility,
	Notifications,
	/// Discord's Registered Games: activity sharing, the current game and Added Games.
	Activity,
	Voice,
	Keybinds,
	/// Display language and clock style.
	LanguageTime,
	Storage,
	Updates,
	Extensions,
	Themes,
}
impl Page {
	/// Every page in sidebar order; the narrow-window page picker lists them the same way.
	const ALL: [Self; 19] = [
		Self::Account,
		Self::Profile,
		Self::MessagingPermissions,
		Self::Privacy,
		Self::ContentFilters,
		Self::Connections,
		Self::Accessibility,
		Self::Storage,
		Self::Appearance,
		Self::Chat,
		Self::Notifications,
		Self::Voice,
		Self::Keybinds,
		Self::Activity,
		Self::LanguageTime,
		Self::General,
		Self::Updates,
		Self::Themes,
		Self::Extensions,
	];
	/// Sidebar sections: account-level choices first, then how this app looks and behaves,
	/// then community add-ons.
	const SECTIONS: [(&'static str, &'static [Self]); 3] = [
		(
			"section-user",
			&[
				Self::Account,
				Self::Profile,
				Self::MessagingPermissions,
				Self::Privacy,
				Self::ContentFilters,
				Self::Connections,
				Self::Storage,
			],
		),
		(
			"section-app",
			&[
				Self::Appearance,
				Self::Chat,
				Self::Notifications,
				Self::Voice,
				Self::Keybinds,
				Self::Accessibility,
				Self::Activity,
				Self::LanguageTime,
				Self::General,
				Self::Updates,
			],
		),
		("section-customization", &[Self::Themes, Self::Extensions]),
	];
	fn label_key(self) -> &'static str {
		match self {
			Self::Account => "page-account",
			Self::Profile => "page-profile",
			Self::General => "page-general",
			Self::LanguageTime => "page-language-time",
			Self::Appearance => "page-appearance",
			Self::Chat => "page-chat",
			Self::MessagingPermissions => "page-messaging-permissions",
			Self::Privacy => "page-privacy",
			Self::ContentFilters => "page-content-filters",
			Self::Connections => "page-connections",
			Self::Accessibility => "page-accessibility",
			Self::Notifications => "page-notifications",
			Self::Activity => "page-registered-games",
			Self::Voice => "page-voice",
			Self::Keybinds => "page-keybinds",
			Self::Storage => "page-storage",
			Self::Updates => "page-updates",
			Self::Extensions => "page-extensions",
			Self::Themes => "page-themes",
		}
	}
	fn label(self, language: Language) -> String {
		language.text(self.label_key())
	}
	fn description_key(self) -> &'static str {
		match self {
			Self::Account => "description-account",
			Self::Profile => "description-profile",
			Self::General => "description-general",
			Self::LanguageTime => "description-language-time",
			Self::Appearance => "description-appearance",
			Self::Chat => "description-chat",
			Self::MessagingPermissions => "description-messaging-permissions",
			Self::Privacy => "description-privacy",
			Self::ContentFilters => "description-content-filters",
			Self::Connections => "description-connections",
			Self::Accessibility => "description-accessibility",
			Self::Notifications => "description-notifications",
			Self::Activity => "description-registered-games",
			Self::Voice => "description-voice",
			Self::Keybinds => "description-keybinds",
			Self::Storage => "description-storage",
			Self::Updates => "description-updates",
			Self::Extensions => "description-extensions",
			Self::Themes => "description-themes",
		}
	}
	fn description(self, language: Language) -> String {
		language.text(self.description_key())
	}
	fn matches(self, query: &str, language: Language) -> bool {
		if query.trim().is_empty() {
			return true;
		}
		let keywords = match self {
			Self::Account => {
				"my account username email phone mfa security subscription billing profile logout"
			}
			Self::Privacy => {
				"profile privacy audience friends local time data safety warnings training"
			}
			Self::ContentFilters => {
				"content filters sensitive graphic gore self harm blur block profanity slurs sexual"
			}
			Self::Connections => {
				"connections connected accounts spotify github twitch oauth display profile sync activity disconnect"
			}
			Self::Accessibility => {
				"accessibility keyboard reading zoom compact motion animations smooth scrolling"
			}
			Self::Profile => {
				"profile edit server nickname avatar banner theme display name about me bio pronouns color colour cosmetics owned equip decoration effect nameplate frame"
			}
			Self::General => {
				"general windows macos linux login menu bar startup autostart automatically open minimized minimize close tray background title bar caption window buttons decorations borderless tiling graphics gpu adapter render discrete integrated hardware acceleration performance battery"
			}
			Self::Appearance => {
				"appearance customization font typography import ttf otf primary accent hex window effects transparency blur theme dark light system mode zoom scale layout sidebar width people members member list reset colour color preset"
			}
			Self::Chat => {
				"chat messages media reading animate animated gifs autoplay hide image links confirm confirmation external browser smooth scrolling scroll speed motion trackpad wheel hidden channels channel list reset emoji emojis nitro fake suggestions autocomplete locked emoticons chat box automatically convert"
			}
			Self::MessagingPermissions => {
				"messaging permissions spam filters direct messages dm friend requests personalized connected games"
			}
			Self::Notifications => {
				"notifications desktop system alerts overview sounds badges message ring"
			}
			Self::Activity => {
				"game activity playing osu status presence sharing registered games added current game detection detected process program executable rename wrong add hide last played"
			}
			Self::Voice => {
				"voice video camera preview audio microphone speakers devices volume gain noise suppression push to talk"
			}
			Self::LanguageTime => {
				"language time clock 12 hour 24 hour am pm format locale translation timezone time zone utc daylight saving region offset"
			}
			Self::Storage => {
				"data privacy local storage clear cache drafts credentials force sync synchronization server account roles members history"
			}
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
			|| self.label(language).to_lowercase().contains(query)
			|| self.description(language).to_lowercase().contains(query)
	}
}

/// An existing setting and the rendered control or section to reveal. A section target is
/// used when the control requires a different mode; searching never changes that mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SearchTarget {
	page: Page,
	label: &'static str,
	anchor: &'static str,
}

struct SearchResult {
	target: SearchTarget,
	label: String,
}

impl SearchTarget {
	fn matches(self, query: &str, language: Language) -> bool {
		let label = language.text(self.label).to_lowercase();
		query.split_whitespace().all(|word| label.contains(word))
	}
}

const fn target(page: Page, label: &'static str) -> SearchTarget {
	SearchTarget {
		page,
		label,
		anchor: label,
	}
}

const fn section_target(page: Page, label: &'static str, anchor: &'static str) -> SearchTarget {
	SearchTarget {
		page,
		label,
		anchor,
	}
}

const SEARCH_TARGETS: &[SearchTarget] = &[
	target(Page::Profile, "profile-edit-form-profile-picture"),
	target(Page::MessagingPermissions, "messaging-permissions-tab-spam"),
	target(
		Page::MessagingPermissions,
		"messaging-permissions-tab-direct-messages",
	),
	target(
		Page::MessagingPermissions,
		"messaging-permissions-tab-friend-requests",
	),
	target(
		Page::MessagingPermissions,
		"messaging-permissions-tab-connected-games",
	),
	target(Page::LanguageTime, "language-label"),
	target(Page::LanguageTime, "time-format-group"),
	target(Page::LanguageTime, "time-format-12-hour"),
	target(Page::LanguageTime, "time-format-24-hour"),
	target(Page::LanguageTime, "timezone-system"),
	target(Page::LanguageTime, "timezone-custom"),
	section_target(Page::LanguageTime, "timezone-region", "timezone-custom"),
	section_target(Page::LanguageTime, "timezone-fixed", "timezone-custom"),
	target(Page::General, "general-open-at-startup"),
	target(Page::General, "general-start-minimized"),
	#[cfg(target_os = "linux")]
	target(Page::General, "general-hide-decorations"),
	#[cfg(any(target_os = "windows", target_os = "macos"))]
	target(Page::General, "general-hide-title-bar"),
	#[cfg(target_os = "macos")]
	target(Page::General, "general-keep-menu-bar"),
	#[cfg(not(target_os = "macos"))]
	target(Page::General, "general-keep-system-tray"),
	target(Page::General, "general-render-with"),
	target(
		Page::Appearance,
		"settings-appearance-settings-primary-color",
	),
	target(
		Page::Appearance,
		"settings-appearance-settings-transparency-blur",
	),
	target(Page::Appearance, "settings-cosmetics-decorations"),
	target(Page::Appearance, "settings-cosmetics-nameplates"),
	target(Page::Appearance, "settings-cosmetics-effects"),
	target(Page::Appearance, "settings-cosmetics-frames"),
	target(Page::Appearance, "settings-cosmetics-name-styles"),
	target(Page::Appearance, "reading-zoom-row-zoom"),
	target(Page::Appearance, "reading-layout-settings-sidebar-width"),
	target(
		Page::Appearance,
		"reading-layout-settings-show-people-in-servers",
	),
	target(
		Page::Appearance,
		"reading-layout-settings-show-people-in-direct-messages",
	),
	target(Page::Chat, "settings-convert-emoticons"),
	target(Page::Chat, "settings-preload-on-hover"),
	target(Page::Chat, "settings-chat-settings-suggest-nitro-emojis"),
	target(Page::Chat, "reading-chat-reading-settings-animate-gifs"),
	target(
		Page::Chat,
		"reading-chat-reading-settings-hide-image-and-gif-links",
	),
	target(
		Page::Chat,
		"reading-chat-reading-settings-compact-message-spacing",
	),
	target(Page::Chat, "reading-double-click-reaction"),
	target(
		Page::Chat,
		"reading-chat-reading-settings-confirm-before-opening-links",
	),
	target(Page::Chat, "reading-chat-reading-settings-smooth-scrolling"),
	target(Page::Chat, "reading-chat-reading-settings-scrolling-speed"),
	target(Page::Accessibility, "reading-zoom-row-zoom"),
	target(
		Page::Accessibility,
		"reading-chat-reading-settings-compact-message-spacing",
	),
	target(Page::Accessibility, "accessibility-reduce-interface-motion"),
	target(Page::Accessibility, "accessibility-animate-artwork"),
	target(
		Page::Accessibility,
		"reading-chat-reading-settings-smooth-scrolling",
	),
	target(
		Page::Accessibility,
		"reading-chat-reading-settings-scrolling-speed",
	),
	target(Page::Storage, "force-sync-account"),
	target(Page::Storage, "settings-storage-page-clear-cache"),
	target(Page::Storage, "settings-storage-page-plaintext-login"),
	target(Page::Storage, "settings-image-cache-media"),
	target(Page::Storage, "settings-image-cache-artwork"),
	target(Page::Storage, "settings-image-cache-retention"),
	target(Page::Updates, "updates-update-settings-auto-update"),
	target(Page::Updates, "updates-update-settings-release-channel"),
	target(Page::Keybinds, "keybinds-show-voice-enable-global-keybinds"),
	target(
		Page::Activity,
		"settings-activity-settings-share-game-activity",
	),
	target(
		Page::Notifications,
		"notification-settings-notification-settings-enable-desktop-notifications",
	),
	target(
		Page::Notifications,
		"notification-settings-notification-settings-sound-volume",
	),
	target(
		Page::Notifications,
		"notification-settings-notification-settings-disable-all-notification-sounds",
	),
	target(
		Page::Notifications,
		"notification-settings-notification-settings-enable-unread-message-badge",
	),
	section_target(
		Page::Voice,
		"voice-voice-audio-controls-microphone",
		"voice-voice-settings-content-devices-levels",
	),
	section_target(
		Page::Voice,
		"voice-voice-audio-controls-speakers",
		"voice-voice-settings-content-devices-levels",
	),
	section_target(
		Page::Voice,
		"voice-voice-processing-controls-noise-suppression",
		"voice-voice-settings-content-voice-processing",
	),
	target(Page::Voice, "voice-voice-processing-controls-push-to-talk"),
	target(Page::Voice, "voice-voice-settings-content-camera"),
	target(Page::Voice, "voice-video-settings-title"),
];

fn search_targets(query: &str, language: Language) -> impl Iterator<Item = SearchTarget> + '_ {
	let query = query.trim();
	SEARCH_TARGETS
		.iter()
		.copied()
		.filter(move |target| !query.is_empty() && target.matches(query, language))
}

impl Settings {
	fn refresh_search(&mut self, language: Language) {
		let raw_query = self.query.trim();
		if self
			.search_cache
			.as_ref()
			.is_some_and(|(cached_language, cached_query)| {
				*cached_language == language && cached_query.as_str() == raw_query
			}) {
			return;
		}
		let query = raw_query.to_lowercase();
		self.search_results = search_targets(&query, language)
			.map(|target| SearchResult {
				target,
				label: format!(
					"{} · {}",
					language.text(target.label),
					target.page.label(language)
				),
			})
			.collect();
		self.search_pages = 0;
		for page in Page::ALL {
			if page.matches(&query, language)
				|| self
					.search_results
					.iter()
					.any(|result| result.target.page == page)
			{
				self.search_pages |= 1u32 << page as u8;
			}
		}
		self.search_cache = Some((language, raw_query.to_owned()));
	}

	fn search_matches_page(&self, page: Page) -> bool {
		self.search_pages & (1u32 << page as u8) != 0
	}
}

// The target exists only while painting the selected settings page. A scoped, thread-local slot
// avoids locking or allocating for ordinary settings controls and cannot affect another window.
thread_local! {
	static SEARCH_SCROLL_TARGET: RefCell<Option<(&'static str, String)>> = const { RefCell::new(None) };
}

struct SearchScrollScope(Option<(&'static str, String)>);
impl SearchScrollScope {
	fn new(target: Option<SearchTarget>, language: Language) -> Self {
		let previous = SEARCH_SCROLL_TARGET.with(|slot| {
			slot.replace(target.map(|target| (target.anchor, language.text(target.anchor))))
		});
		Self(previous)
	}

	fn revealed(&self) -> bool {
		SEARCH_SCROLL_TARGET.with(|slot| slot.borrow().is_none())
	}
}
impl Drop for SearchScrollScope {
	fn drop(&mut self) {
		SEARCH_SCROLL_TARGET.with(|slot| {
			slot.replace(self.0.take());
		});
	}
}

/// Reveal the matching real control without activating it or changing its value.
pub(super) fn reveal_search_target(ui: &mut egui::Ui, label: &str) {
	let matched = SEARCH_SCROLL_TARGET.with(|slot| {
		slot.borrow()
			.as_ref()
			.is_some_and(|(key, translated)| label == *key || label == translated)
	});
	if matched {
		if ui.is_sizing_pass() {
			// A newly opened Area first measures invisibly; keep the request until its
			// geometry is real rather than scrolling against provisional coordinates.
			ui.ctx().request_repaint();
			return;
		}
		SEARCH_SCROLL_TARGET.with(|slot| {
			slot.borrow_mut().take();
		});
		ui.scroll_to_rect(
			egui::Rect::from_min_size(ui.cursor().min, egui::vec2(1.0, 48.0)),
			Some(egui::Align::Min),
		);
	}
}

fn gpu_label(language: Language, preference: model::GpuPreference) -> String {
	language.text(match preference {
		model::GpuPreference::Automatic => "gpu-automatic",
		model::GpuPreference::HighPerformance => "gpu-high-performance",
		model::GpuPreference::PowerSaving => "gpu-power-saving",
	})
}

fn gpu_description(language: Language, preference: model::GpuPreference) -> String {
	language.text(match preference {
		model::GpuPreference::Automatic => "gpu-automatic-description",
		model::GpuPreference::HighPerformance => "gpu-high-performance-description",
		model::GpuPreference::PowerSaving => "gpu-power-saving-description",
	})
}

impl MessagingUi {
	pub(super) fn open_extension_settings(&mut self, view: extensions::AppView) {
		self.settings.page = match view {
			extensions::AppView::Settings => Page::General,
			extensions::AppView::Account => Page::Account,
			extensions::AppView::ProfileSettings => Page::Profile,
			extensions::AppView::Appearance => Page::Appearance,
			extensions::AppView::MessagingPermissions => Page::MessagingPermissions,
			extensions::AppView::Notifications => Page::Notifications,
			extensions::AppView::Activity => Page::Activity,
			extensions::AppView::Extensions => Page::Extensions,
			extensions::AppView::Themes => Page::Themes,
			extensions::AppView::VoiceSettings => Page::Voice,
			extensions::AppView::Keybinds => Page::Keybinds,
			extensions::AppView::Storage => Page::Storage,
			extensions::AppView::Updates => Page::Updates,
			_ => return,
		};
		self.settings.query.clear();
		self.settings.open = true;
	}

	pub(super) fn theme_preview_navigation(&mut self, ui: &mut egui::Ui) {
		if self.extensions.begin_gallery_preview(ui.ctx()) {
			self.settings.open = false;
		}
		if self.settings.open {
			self.extensions.stop_theme_preview(ui.ctx());
		} else if self.extensions.theme_preview_bar(
			ui,
			if self.shows_title_bar() {
				design::TRAFFIC_LIGHT_INSET
			} else {
				0.0
			},
		) {
			self.settings.open = true;
			self.settings.page = Page::Themes;
			self.settings.query.clear();
		}
	}
	/// Opens settings on the last viewed page, as the macOS Settings menu item does.
	pub fn open_settings(&mut self) {
		self.settings.open = true;
	}
	pub fn open_update_settings(&mut self) {
		self.settings.open = true;
		self.settings.page = Page::Updates;
		self.settings.query.clear();
	}

	pub(super) fn keybinds_shortcut(&mut self, ctx: &egui::Context) {
		if !self.server_settings.is_open()
			&& !self.switcher.is_open()
			&& !self.ime_active
			&& !egui::Popup::is_any_open(ctx)
			&& ctx.memory(|memory| memory.top_modal_layer().is_none() || self.settings.open)
			&& ctx.input(|input| {
				input.focused
					&& !input
						.events
						.iter()
						.any(|event| matches!(event, egui::Event::Ime(_)))
			}) && ctx.input_mut(|input| {
			crate::keybinds::pressed(
				input,
				self.keybinds.chord(model::KeybindAction::ShowShortcuts),
			)
		}) {
			self.settings.open = true;
			self.settings.page = Page::Keybinds;
			self.settings.query.clear();
		}
	}

	pub fn extension_settings_page(&self) -> Option<extensions::ExtensionKind> {
		if !self.settings.open {
			return None;
		}
		match self.settings.page {
			Page::Themes => Some(extensions::ExtensionKind::Theme),
			Page::Extensions => Some(extensions::ExtensionKind::Plugin),
			_ => None,
		}
	}

	pub fn voice_settings_open(&self) -> bool {
		self.settings.open && self.settings.page == Page::Voice
	}

	pub(super) fn open_voice_settings(&mut self) {
		self.settings.open = true;
		self.settings.page = Page::Voice;
		self.settings.query.clear();
	}

	/// Fixture-only entry point: opens the theme maker on the requested editor tab.
	#[cfg(feature = "demo")]
	pub fn preview_theme_maker(&mut self, tab: &str) {
		self.preview_settings("themes");
		self.extensions.preview_theme_maker(tab);
	}
	/// Fixture-only entry point for the native offline settings preview.
	pub fn preview_settings(&mut self, page: &str) {
		self.settings.open = true;
		if page == "activity" {
			self.settings.page = Page::Activity;
			return;
		}
		if page == "profiles-server" {
			self.settings.page = Page::Profile;
			self.settings.server_profile = true;
			return;
		}
		// A label wins; keywords still reach pages that were merged or renamed ("activity").
		if let Some(page) = Page::ALL
			.into_iter()
			.find(|candidate| {
				candidate
					.label(Language::English)
					.to_lowercase()
					.contains(page)
			})
			.or_else(|| {
				Page::ALL
					.into_iter()
					.find(|candidate| candidate.matches(page, Language::English))
			}) {
			self.settings.page = page;
		}
	}
	pub(super) fn show_settings(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<client_core::Command>,
	) {
		let ime_this_frame = ctx.input(|input| {
			if !input.focused {
				self.settings.ime_active = false;
			}
			let mut changed = false;
			for event in &input.events {
				if let egui::Event::Ime(ime) = event {
					changed = true;
					match ime {
						egui::ImeEvent::Preedit { text, .. } => {
							self.settings.ime_active = !text.is_empty()
						}
						egui::ImeEvent::Commit(_) => self.settings.ime_active = false,
						_ => {}
					}
				}
			}
			changed
		});
		let composing = self.settings.ime_active || ime_this_frame;
		let colors = design::palette_for(ctx);
		let size = ctx.content_rect().size() - egui::vec2(32.0, 40.0);
		let width = size.x.clamp(0.0, 1100.0);
		let height = size.y.clamp(0.0, 820.0);
		let compact = width < 280.0 || height < 240.0;
		let wide = width >= 620.0;
		let modal = egui::Modal::new(egui::Id::unique("user-settings"))
			.backdrop_color(egui::Color32::from_black_alpha(180))
			.frame(
				egui::Frame::new()
					.fill(colors.chat.to_opaque())
					.corner_radius(crate::dialog::RADIUS)
					.shadow(ctx.style_of(ctx.theme()).visuals.window_shadow)
					.stroke(egui::Stroke::new(1.0, colors.border)),
			)
			.show(ctx, |ui| {
				ui.set_width(width);
				ui.set_height(height);
				if wide {
					egui::Panel::left("settings-navigation")
						.exact_size(232.0)
						.resizable(false)
						.frame(
							egui::Frame::new()
								.fill(colors.sidebar.to_opaque())
								.corner_radius(egui::CornerRadius {
									nw: crate::dialog::RADIUS,
									sw: crate::dialog::RADIUS,
									ne: 0,
									se: 0,
								})
								.inner_margin(egui::Margin {
									left: 12,
									right: 8,
									top: 20,
									bottom: 16,
								}),
						)
						.show(ui, |ui| self.settings_navigation(ui, state));
				}
				egui::CentralPanel::default()
					.frame(egui::Frame::new().inner_margin(egui::Margin {
						left: if wide {
							40
						} else if compact {
							8
						} else {
							20
						},
						right: if compact { 8 } else { 20 },
						top: if compact { 8 } else { 24 },
						bottom: if compact { 8 } else { 24 },
					}))
					.show(ui, |ui| {
						let editing_theme =
							self.settings.page == Page::Themes && self.extensions.editing_theme();
						let heading = if editing_theme {
							self.language.text("theme-maker")
						} else {
							self.settings.page.label(self.language)
						};
						let description = if editing_theme {
							self.language.text("theme-maker-description")
						} else {
							self.settings.page.description(self.language)
						};
						if page_header(ui, &heading, &description, compact).clicked() {
							self.settings.open = false;
						}
						if !wide {
							ui.add_space(if compact { 2.0 } else { 8.0 });
							if compact {
								ui.horizontal_top(|ui| {
									let search_width = ((ui.available_width()
										- ui.spacing().item_spacing.x) * 0.5)
										.max(1.0);
									ui.allocate_ui_with_layout(
										egui::vec2(search_width, 0.0),
										egui::Layout::top_down(egui::Align::Min),
										|ui| {
											ui.set_width(search_width);
											self.settings_search(ui);
										},
									);
									self.settings_page_picker(ui);
								});
							} else {
								self.settings_search(ui);
								self.settings_page_picker(ui);
							}
						}
						ui.add_space(if compact { 4.0 } else { 16.0 });
						if self.settings.page == Page::Themes && self.extensions.editing_theme() {
							self.extensions.theme_editor_toolbar(ui);
							ui.add_space(12.0);
						}
						self.cancel_search_jump_on_content_interaction(ui);
						let content_scroll = if compact {
							egui::ScrollArea::both()
						} else {
							egui::ScrollArea::vertical()
						};
						content_scroll
							.id_salt((
								"settings-content",
								self.settings.page as u8,
								self.extensions.theme_editor_tab_key(),
							))
							.auto_shrink([false, false])
							.show(ui, |ui| {
								crate::scroll::claim_wheel_content(ui);
								crate::dialog::page_fade(
									ui,
									egui::Id::unique((
										"settings-content",
										self.settings.page as u8,
									)),
								);
								let scroll_padding = if self.settings.page == Page::Profile {
									8.0
								} else {
									0.0
								};
								ui.set_width((ui.available_width() - scroll_padding).min(720.0));
								ui.spacing_mut().item_spacing.y = 12.0;
								let jump = self
									.settings
									.search_jump
									.take()
									.filter(|target| target.page == self.settings.page);
								let section_jump =
									jump.filter(|target| target.page == Page::MessagingPermissions);
								let control_jump =
									jump.filter(|target| target.page != Page::MessagingPermissions);
								let search_scope =
									SearchScrollScope::new(control_jump, self.language);
								match self.settings.page {
									Page::General => self.general_settings(ui, state.demo),
									Page::LanguageTime => self.language_time_settings(ui),
									Page::Account => self.account_page(ui, state, commands),
									Page::Privacy => {
										self.privacy_settings(ui, state, commands, false)
									}
									Page::ContentFilters => {
										self.privacy_settings(ui, state, commands, true)
									}
									Page::Connections => {
										self.connection_settings(ui, state, commands)
									}
									Page::Accessibility => self.accessibility_settings(ui),
									Page::Profile => self.profile_settings(ui, state, commands),
									Page::Appearance => self.appearance_settings(ui, state.demo),
									Page::Chat => self.chat_settings(ui, state.demo),
									Page::MessagingPermissions => {
										self.messaging_permissions_settings(ui, state, commands);
										if let Some(target) = section_jump
											&& let Some(tab) =
												crate::messaging_permissions::Tab::ALL
													.into_iter()
													.find(|tab| tab.label() == target.anchor)
										{
											self.settings.messaging_permissions.jump = Some(tab);
											self.settings.messaging_permissions.active = tab;
											ui.ctx().request_repaint();
										}
									}
									Page::Notifications => {
										self.notification_settings(ui, state.demo)
									}
									Page::Activity => self.activity_settings(ui, state),
									Page::Voice => self.voice_settings_content(
										ui,
										state.demo,
										state.voice.active.is_some(),
										false,
									),
									Page::Storage => self.storage_page(ui, state),
									Page::Updates => self.update_settings(ui, state.demo),
									Page::Keybinds => crate::keybinds::show(
										ui,
										&mut self.keybinds,
										&mut self.keybind_capture,
										self.global_keybind_status,
									),
									Page::Extensions | Page::Themes => {
										self.extensions
											.select_themes(self.settings.page == Page::Themes);
										self.extensions.settings(ui, state);
									}
								}
								if let Some(target) = control_jump
									&& !search_scope.revealed()
								{
									// A profile can still be loading. Keep one bounded target until it appears,
									// the query/page changes, or the settings window closes.
									self.settings.search_jump = Some(target);
								}
								if state.demo {
									ui.add_space(20.0);
									design::hint(ui, &self.language.text("offline-preview"));
								}
								ui.add_space(24.0);
							});
					});
			});
		if modal.backdrop_response.clicked()
			|| modal.response.should_close()
			|| (!composing && modal.should_close())
		{
			self.settings.open = false;
		}
		if self.keybind_capture.is_none()
			&& !composing
			&& modal.is_top_modal
			&& !modal.any_popup_open
			&& ctx.input_mut(|input| {
				crate::keybinds::pressed_exact(
					input,
					self.keybinds.chord(model::KeybindAction::CloseOverlay),
				)
			}) {
			self.settings.open = false;
		}
		if !self.settings.open {
			self.settings.messaging_permissions.requested = false;
			self.settings.ime_active = false;
			self.settings.search_jump = None;
		}
		if !self.settings.open || self.settings.page != Page::MessagingPermissions {
			self.settings.messaging_permissions.jump = None;
		}
		if !self.settings.open || self.settings.page != Page::Account {
			self.settings.account.conceal();
		}
	}

	fn profile_settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<client_core::Command>,
	) {
		ui.horizontal_wrapped(|ui| {
			ui.selectable_value(&mut self.settings.server_profile, false, "User Profile");
			ui.selectable_value(&mut self.settings.server_profile, true, "Server Profiles");
		});
		ui.add_space(8.0);
		if self.settings.server_profile {
			self.settings
				.server_editor
				.show(ui, state, &mut self.avatars, commands);
		} else {
			self.settings
				.editor
				.show(ui, state, &mut self.avatars, commands);
		}
		ui.add_space(16.0);
		self.owned_cosmetic_settings(ui, state, commands);
	}
	pub fn profile_picture_is_banner(&self) -> bool {
		self.settings.editor.banner_picker
	}
	pub fn take_server_profile_image_request(&mut self) -> Option<((u64, model::Id, u64), bool)> {
		self.settings.server_editor.image_request.take()
	}
	pub fn accept_server_profile_image(
		&mut self,
		ctx: &egui::Context,
		scope: (u64, model::Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		if let Err(error) = self.settings.server_editor.accept_image(ctx, scope, result) {
			self.toasts.push(design::Level::Error, error);
		}
	}

	fn settings_navigation(&mut self, ui: &mut egui::Ui, state: &State) {
		let colors = design::palette(ui);
		let language = self.language;
		egui::ScrollArea::vertical()
			.id_salt("settings-navigation-scroll")
			.auto_shrink([false, false])
			.show(ui, |ui| {
				crate::scroll::claim_wheel_content(ui);
				ui.spacing_mut().item_spacing.y = 2.0;
				self.settings_search(ui);
				ui.add_space(12.0);
				for (heading, pages) in Page::SECTIONS {
					let visible: Vec<Page> = pages
						.iter()
						.copied()
						.filter(|page| self.settings.search_matches_page(*page))
						.collect();
					if visible.is_empty() {
						continue;
					}
					ui.add_space(6.0);
					ui.add(egui::Label::new(design::eyebrow(
						ui,
						language.text(heading),
						colors.muted,
					)));
					ui.add_space(2.0);
					for page in visible {
						if nav_item(ui, &page.label(language), self.settings.page == page).clicked()
						{
							if page == Page::MessagingPermissions && self.settings.page != page {
								self.settings.messaging_permissions.requested = false;
							}
							self.settings.page = page;
						}
						if page == Page::MessagingPermissions && self.settings.page == page {
							ui.indent("messaging-permission-sections", |ui| {
								for tab in crate::messaging_permissions::Tab::ALL {
									if nav_item(
										ui,
										tab.label(),
										self.settings.messaging_permissions.active == tab,
									)
									.clicked()
									{
										self.settings.messaging_permissions.jump = Some(tab);
										self.settings.messaging_permissions.active = tab;
									}
								}
							});
						}
						if page == Page::Notifications && self.settings.page == page {
							ui.indent("notification-sections", |ui| {
								for tab in crate::notification_settings::Tab::ALL {
									if nav_item(
										ui,
										tab.label(),
										self.settings.notifications.active == tab,
									)
									.clicked()
									{
										self.settings.notifications.jump = Some(tab);
										self.settings.notifications.active = tab;
									}
								}
							});
						}
					}
				}
				ui.add_space(8.0);
				ui.separator();
				ui.add_space(4.0);
				self.settings_logout(ui, state.demo);
				ui.add_space(12.0);
				ui.label(
					RichText::new(format!("{} {}", model::APP_NAME, self.build.version))
						.size(12.0)
						.color(colors.muted),
				);
				ui.label(
					RichText::new(language.text("unofficial"))
						.size(11.0)
						.color(colors.muted),
				);
			});
	}

	fn settings_page_picker(&mut self, ui: &mut egui::Ui) {
		egui::ComboBox::from_id_salt("settings-page")
			.selected_text(self.settings.page.label(self.language))
			.width(ui.available_width().max(1.0))
			.truncate()
			.show_ui(ui, |ui| {
				for page in Page::ALL {
					if self.settings.search_matches_page(page) {
						ui.selectable_value(
							&mut self.settings.page,
							page,
							page.label(self.language),
						);
					}
				}
			});
	}

	fn activate_search_target(&mut self, target: SearchTarget) {
		if target.page == Page::MessagingPermissions && self.settings.page != target.page {
			self.settings.messaging_permissions.requested = false;
		}
		if target.page == Page::Profile {
			self.settings.server_profile = false;
		}
		self.settings.page = target.page;
		self.settings.search_jump = Some(target);
	}

	fn cancel_search_jump_on_content_interaction(&mut self, ui: &egui::Ui) {
		if !ui.rect_contains_pointer(ui.available_rect_before_wrap()) {
			return;
		}
		let manual = ui.input(|input| {
			input.smooth_scroll_delta() != egui::Vec2::ZERO
				|| input.pointer.any_pressed()
				|| [
					egui::Key::PageUp,
					egui::Key::PageDown,
					egui::Key::Home,
					egui::Key::End,
				]
				.into_iter()
				.any(|key| input.key_pressed(key))
		});
		if manual {
			self.settings.search_jump = None;
			self.settings.messaging_permissions.jump = None;
		}
	}

	fn settings_search(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		let language = self.language;
		egui::Frame::new()
			.fill(colors.raised)
			.corner_radius(6)
			.inner_margin(egui::Margin::symmetric(8, 4))
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					let response = ui.add(
						egui::TextEdit::singleline(&mut self.settings.query)
							.align(egui::Align2::LEFT_CENTER)
							.hint_text(language.text("search"))
							.char_limit(64)
							.frame(egui::Frame::NONE)
							.desired_width((ui.available_width() - 24.0).max(1.0)),
					);
					icons::inline(ui, icons::Icon::Search, 16.0, colors.muted);
					if response.changed() {
						self.settings.search_jump = None;
						self.settings.messaging_permissions.jump = None;
					}
				});
			});
		self.settings.refresh_search(language);
		if !self.settings.query.trim().is_empty() {
			if !self.settings.search_results.is_empty() {
				let max_height = (ui.available_height() * 0.3).clamp(40.0, 200.0);
				let mut selected = None;
				egui::ScrollArea::vertical()
					.id_salt("settings-search-results")
					.max_height(max_height)
					.auto_shrink([false, true])
					.show_rows(ui, 34.0, self.settings.search_results.len(), |ui, range| {
						for index in range {
							let result = &self.settings.search_results[index];
							if nav_item(ui, &result.label, false).clicked() {
								selected = Some(result.target);
							}
						}
					});
				if let Some(target) = selected {
					self.activate_search_target(target);
				}
			}
			if self.settings.search_pages == 0 {
				ui.weak(language.text("no-settings-found"));
			}
		}
	}

	fn settings_logout(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = design::palette(ui);
		let label = self
			.language
			.text(if demo { "exit-preview" } else { "log-out" });
		let (rect, response) =
			ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
		if response.hovered() || response.has_focus() {
			ui.painter().rect_filled(rect, 4, colors.hover);
		}
		ui.painter().text(
			egui::pos2(rect.left() + 10.0, rect.center().y),
			egui::Align2::LEFT_CENTER,
			&label,
			egui::FontId::new(15.0, design::medium_family(ui.ctx())),
			colors.danger,
		);
		icons::paint(
			ui.painter(),
			icons::Icon::External,
			egui::Rect::from_center_size(
				egui::pos2(rect.right() - 18.0, rect.center().y),
				egui::Vec2::splat(16.0),
			),
			colors.danger,
		);
		if response.clicked() {
			self.logout_requested = true;
			self.settings.open = false;
		}
	}

	fn account_page(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<client_core::Command>,
	) {
		let colors = design::palette(ui);
		let name = state
			.user
			.as_ref()
			.map_or("Your account", |u| u.name.as_str())
			.to_owned();
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(8)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 0.0;
				let (banner, _) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 96.0),
					egui::Sense::hover(),
				);
				ui.painter().rect_filled(
					banner,
					egui::CornerRadius {
						nw: 8,
						ne: 8,
						sw: 0,
						se: 0,
					},
					colors.accent,
				);
				egui::Frame::new()
					.inner_margin(egui::Margin {
						left: 16,
						right: 16,
						top: 12,
						bottom: 16,
					})
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.horizontal(|ui| {
							ui.add_space(96.0);
							ui.vertical(|ui| {
								ui.spacing_mut().item_spacing.y = 2.0;
								ui.add(
									egui::Label::new(
										design::semibold(ui, name.clone(), 20.0)
											.color(colors.text_strong),
									)
									.truncate(),
								);
								ui.label(
									RichText::new(crate::i18n::translate_if_key(if state.demo {
										"settings-account-page-offline-preview-synthetic-account"
									} else {
										"settings-account-page-signed-in-with-your-discord-account"
									}))
									.size(13.0)
									.color(colors.muted),
								);
							});
						});
						ui.add_space(16.0);
						egui::Frame::new()
							.fill(colors.chat)
							.corner_radius(8)
							.inner_margin(egui::Margin::symmetric(16, 12))
							.show(ui, |ui| {
								ui.set_width(ui.available_width());
								ui.spacing_mut().item_spacing.y = 10.0;
								account_row(ui, "settings-account-page-display-name", &name);
							});
						ui.add_space(12.0);
						ui.horizontal(|ui| {
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									if design::button(
										ui,
										&crate::i18n::translate(
											"settings-account-page-edit-profile",
										),
										design::ButtonKind::Outline,
									)
									.clicked()
									{
										self.settings.page = Page::Profile;
									}
								},
							);
						});
					});
				// Avatar overlapping the banner edge, ringed by the card surface.
				let avatar = egui::Rect::from_min_size(
					banner.left_bottom() + egui::vec2(16.0, -40.0),
					egui::Vec2::splat(80.0),
				);
				ui.painter()
					.circle_filled(avatar.center(), 44.0, colors.raised);
				ui.scope_builder(egui::UiBuilder::new().max_rect(avatar), |ui| {
					if let Some(user) = &state.user {
						self.avatars.with_avatar_animation(true, |avatars| {
							avatars.show(ui, user, 80.0, state.demo)
						});
					} else {
						design::avatar(ui, &name, 80.0);
					}
				});
			});
		self.account_overview_details(ui, state, commands);
		let label = if state.demo {
			"Exit preview"
		} else {
			"Log out"
		};
		design::group(
			ui,
			&crate::i18n::translate("settings-account-page-session"),
			|ui| {
				if design::row(
					ui,
					label,
					Some(if state.demo {
						"settings-account-page-closes-the-offline-fixture-nothing-is-stored-for-the-preview"
					} else {
						"settings-account-page-removes-the-saved-login-and-clears-this-account-s-local"
					}),
					|ui| design::button(ui, label, design::ButtonKind::Danger),
				)
				.clicked()
				{
					self.logout_requested = true;
					self.settings.open = false;
				}
			},
		);
	}

	fn language_time_settings(&mut self, ui: &mut egui::Ui) {
		let language = self.language;
		let title = language.text("language-group");
		design::group(ui, &title, |ui| {
			let label = language.text("language-label");
			let description = language.text("language-description");
			design::row(ui, &label, Some(&description), |ui| {
				egui::ComboBox::from_id_salt("display-language")
					.selected_text(self.language.name(language))
					.width(ui.available_width().min(220.0))
					.show_ui(ui, |ui| {
						for candidate in Language::ALL {
							ui.selectable_value(
								&mut self.language,
								candidate,
								candidate.name(language),
							);
						}
					});
			});
		});
		let title = language.text("time-format-group");
		design::group(ui, &title, |ui| {
			for (format, key) in [
				(model::TimeFormat::Auto, "time-format-auto"),
				(model::TimeFormat::TwelveHour, "time-format-12-hour"),
				(model::TimeFormat::TwentyFourHour, "time-format-24-hour"),
			] {
				if design::radio_row(ui, self.time_format == format, &language.text(key), None)
					.clicked()
				{
					self.time_format = format;
				}
			}
		});
		let title = language.text("timezone-group");
		design::group(ui, &title, |ui| {
			let system = self.time_zone == model::TimeZonePreference::System;
			if design::radio_row(ui, system, &language.text("timezone-system"), None).clicked() {
				if !system {
					self.settings.custom_timezone = Some(self.time_zone.clone());
				}
				self.time_zone = model::TimeZonePreference::System;
			}
			if design::radio_row(ui, !system, &language.text("timezone-custom"), None).clicked()
				&& system
			{
				self.time_zone = self
					.settings
					.custom_timezone
					.clone()
					.unwrap_or_else(|| model::TimeZonePreference::Named("UTC".into()));
			}
			ui.add_enabled_ui(self.time_zone != model::TimeZonePreference::System, |ui| {
				let previous_fixed =
					matches!(self.time_zone, model::TimeZonePreference::FixedOffset(_));
				let mut fixed = previous_fixed;
				egui::ComboBox::from_id_salt("timezone-kind")
					.selected_text(language.text(if fixed {
						"timezone-fixed"
					} else {
						"timezone-region"
					}))
					.show_ui(ui, |ui| {
						ui.selectable_value(&mut fixed, false, language.text("timezone-region"));
						ui.selectable_value(&mut fixed, true, language.text("timezone-fixed"));
					});
				if fixed != previous_fixed {
					self.time_zone = if fixed {
						model::TimeZonePreference::FixedOffset(0)
					} else {
						model::TimeZonePreference::Named("UTC".into())
					};
				}
				if let model::TimeZonePreference::FixedOffset(minutes) = self.time_zone {
					if self.settings.timezone_fixed_for != Some(minutes) {
						self.settings.timezone_fixed =
							model::TimeZonePreference::fixed_label(minutes);
						self.settings.timezone_fixed_for = Some(minutes);
					}
					ui.horizontal_wrapped(|ui| {
						ui.add(
							egui::TextEdit::singleline(&mut self.settings.timezone_fixed)
								.desired_width(140.0)
								.char_limit(9)
								.hint_text("UTC+03:00"),
						);
						let parsed =
							model::TimeZonePreference::parse_fixed(&self.settings.timezone_fixed);
						if ui
							.add_enabled(
								parsed.is_some(),
								egui::Button::new(language.text("timezone-apply")),
							)
							.clicked()
						{
							self.time_zone = parsed.unwrap_or_default();
						}
					});
					if model::TimeZonePreference::parse_fixed(&self.settings.timezone_fixed)
						.is_none()
					{
						design::hint(ui, &language.text("timezone-offset-invalid"));
					}
				} else {
					let selected = match &self.time_zone {
						model::TimeZonePreference::Named(name) => name.as_str(),
						_ => "UTC",
					};
					egui::ComboBox::from_id_salt("timezone-region")
						.selected_text(selected.replace('_', " "))
						.width(ui.available_width().min(300.0))
						.show_ui(ui, |ui| {
							ui.add(
								egui::TextEdit::singleline(&mut self.settings.timezone_query)
									.char_limit(128)
									.hint_text(language.text("timezone-search")),
							);
							let query = self
								.settings
								.timezone_query
								.trim()
								.replace('_', " ")
								.to_ascii_lowercase();
							let names = crate::local_time::timezone_names();
							if self.settings.timezone_filter.as_deref() != Some(&query) {
								self.settings.timezone_matches = names
									.iter()
									.enumerate()
									.filter_map(|(index, (_, search))| {
										search.contains(&query).then_some(index)
									})
									.collect();
								self.settings.timezone_filter = Some(query);
							}
							ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
							let row_height = ui.spacing().interact_size.y.max(
								ui.text_style_height(&egui::TextStyle::Button)
									+ 2.0 * ui.spacing().button_padding.y,
							);
							egui::ScrollArea::vertical()
								.id_salt("timezone-region-results")
								.max_height(220.0)
								.show_rows(
									ui,
									row_height,
									self.settings.timezone_matches.len(),
									|ui, rows| {
										crate::scroll::claim_wheel_content(ui);
										for row in rows {
											let (name, _) =
												&names[self.settings.timezone_matches[row]];
											ui.selectable_value(
												&mut self.time_zone,
												model::TimeZonePreference::Named(name.clone()),
												name.replace('_', " "),
											)
											.on_hover_text(name);
										}
									},
								);
						});
					design::hint(ui, &language.text("timezone-region-description"));
				}
			});
		});
		let format = (self.time_format, self.language, self.time_zone.clone());
		if self.settings.language_time_format.as_ref() != Some(&format) {
			crate::local_time::set_format(self.time_format, self.language);
			crate::local_time::set_timezone(&self.time_zone);
			self.settings.language_time_format = Some(format);
		}
		let now = crate::local_time::now();
		let minute = now.unix_timestamp().div_euclid(60);
		let revision = crate::local_time::format_revision();
		if self
			.settings
			.clock_preview
			.as_ref()
			.is_none_or(|cached| (cached.0, cached.1) != (minute, revision))
		{
			self.settings.clock_preview = Some((
				minute,
				revision,
				crate::local_time::timezone_label(),
				crate::local_time::full(now),
			));
		}
		if let Some((_, _, label, example)) = &self.settings.clock_preview {
			design::hint(ui, label);
			ui.label(example);
		}
		if ui.is_visible() && !ui.input(|input| input.viewport().minimized.unwrap_or(false)) {
			ui.ctx()
				.request_repaint_after(crate::local_time::minute_deadline(now));
		}
		if let Some(status) = crate::local_time::timezone_status() {
			design::hint(ui, &language.text(status));
		}
	}

	fn general_settings(&mut self, ui: &mut egui::Ui, _demo: bool) {
		let language = self.language;
		let title = language.text("general-startup");
		design::group(ui, &title, |ui| {
			ui.add_enabled_ui(self.startup_available && !self.startup_busy, |ui| {
				let label = language.text("general-open-at-startup");
				let description = language.text("general-open-at-startup-description");
				design::switch(ui, &label, Some(&description), &mut self.startup_enabled);
				design::card_divider(ui);
				// An already-disabled parent has applied the disabled opacity.
				let nested_enabled = !ui.is_enabled() || self.startup_enabled;
				ui.add_enabled_ui(nested_enabled, |ui| {
					let label = language.text("general-start-minimized");
					let description = language.text("general-start-minimized-description");
					design::switch(ui, &label, Some(&description), &mut self.startup_minimized);
				});
			});
			if !self.startup_available {
				design::hint(ui, &language.text("general-startup-unavailable"));
			} else if !self.startup_status.is_empty() {
				design::hint(ui, self.startup_status);
			}
		});
		let title = language.text("general-window");
		design::group(ui, &title, |ui| {
			#[cfg(target_os = "linux")]
			{
				let label = language.text("general-hide-decorations");
				let description = language.text("general-hide-decorations-description");
				design::switch(
					ui,
					&label,
					Some(&description),
					&mut self.hide_window_decorations,
				);
				design::card_divider(ui);
			}
			#[cfg(any(target_os = "windows", target_os = "macos"))]
			{
				let label = language.text("general-hide-title-bar");
				let description = language.text("general-hide-title-bar-description");
				design::switch(ui, &label, Some(&description), &mut self.hide_title_bar);
				design::card_divider(ui);
			}
			ui.add_enabled_ui(self.tray_available, |ui| {
				let label = language.text(if cfg!(target_os = "macos") {
					"general-keep-menu-bar"
				} else {
					"general-keep-system-tray"
				});
				let description = language.text(if cfg!(target_os = "macos") {
					"general-menu-bar-description"
				} else if cfg!(target_os = "linux") {
					"general-linux-tray-description"
				} else {
					"general-windows-tray-description"
				});
				design::switch(ui, &label, Some(&description), &mut self.minimize_to_tray);
			});
			if !self.tray_available {
				design::hint(ui, &language.text("general-tray-unavailable"));
			} else if !self.tray_status.is_empty() {
				design::hint(ui, self.tray_status);
			}
		});
		let title = language.text("general-graphics");
		design::group(ui, &title, |ui| {
			let restart = language.text("general-gpu-restart");
			let detail = if self.gpu_adapter.is_empty() {
				restart
			} else {
				format!(
					"{} {}. {}",
					language.text("general-gpu-current-prefix"),
					self.gpu_adapter,
					restart
				)
			};
			let label = language.text("general-render-with");
			design::row(ui, &label, Some(&detail), |ui| {
				egui::ComboBox::from_id_salt("gpu-preference")
					.selected_text(gpu_label(language, self.gpu_preference))
					.width(ui.available_width().min(220.0))
					.show_ui(ui, |ui| {
						for preference in model::GpuPreference::ALL {
							ui.selectable_value(
								&mut self.gpu_preference,
								preference,
								gpu_label(language, preference),
							)
							.on_hover_text(gpu_description(language, preference));
						}
					});
			});
		});
	}

	/// Compact appearance popup for the signed-out header: mode, colour preset and zoom.
	/// Deliberately narrower than the settings page; everything else lives in Settings.
	pub fn appearance_menu(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		ui.set_min_width(324.0);
		ui.set_max_width(324.0);
		ui.spacing_mut().item_spacing.y = 6.0;
		ui.label(design::eyebrow(
			ui,
			crate::i18n::translate("settings-appearance-menu-mode"),
			colors.muted,
		));
		theme_preference_cards(ui);
		ui.add_space(6.0);
		ui.label(design::eyebrow(
			ui,
			crate::i18n::translate("settings-appearance-menu-theme"),
			colors.muted,
		));
		let current = design::variant();
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);
			for variant in design::Variant::ALL {
				let swatch = design::builtin_colors(ui.visuals().dark_mode, variant);
				let selected = variant == current;
				if preset_swatch(ui, variant.label(), &swatch, selected).clicked() && !selected {
					design::set_variant(variant);
					design::apply(ui.ctx());
					self.theme_variant_changed = Some(variant);
				}
			}
		});
		ui.add_space(6.0);
		ui.label(design::eyebrow(
			ui,
			crate::i18n::translate("settings-appearance-menu-display"),
			colors.muted,
		));
		let mut value = self.reading_preferences;
		ui.spacing_mut().slider_width = 96.0;
		self.zoom_row(ui, &mut value);
		if value != self.reading_preferences {
			self.apply_reading_preferences(ui.ctx(), value);
		}
	}

	fn appearance_settings(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = design::palette(ui);
		ui.add_space(4.0);
		ui.label(design::eyebrow(
			ui,
			crate::i18n::translate("settings-appearance-settings-theme"),
			colors.muted,
		));
		theme_preference_cards(ui);
		self.colour_preset_settings(ui);
		self.custom_font.show(ui);
		design::group(
			ui,
			&crate::i18n::translate("settings-appearance-settings-accent"),
			|ui| {
				let themed_accent = design::theme_sets_accent(ui.visuals().dark_mode);
				design::row(
					ui,
					"settings-appearance-settings-primary-color",
					Some(if themed_accent {
						"settings-appearance-settings-the-active-theme-brings-its-own-accent-it-takes-over"
					} else {
						"settings-appearance-settings-used-for-buttons-selection-and-message-highlights"
					}),
					|ui| {
						ui.add_enabled_ui(!themed_accent, |ui| {
							if self.primary_color.is_some()
								&& design::text_action(
									ui,
									&crate::i18n::translate("settings-appearance-settings-reset"),
								)
								.clicked()
							{
								self.primary_color = None;
							}
							let mut color =
								self.primary_color.unwrap_or(design::DEFAULT_PRIMARY_COLOR);
							if design::color_edit(ui, &mut color)
								.on_hover_text(crate::i18n::translate(
									"settings-appearance-settings-choose-primary-color",
								))
								.changed()
							{
								self.primary_color = Some(color);
							}
						});
					},
				);
			},
		);
		design::group(
			ui,
			&crate::i18n::translate("settings-appearance-settings-window-effects"),
			|ui| {
				design::switch(
					ui,
					"settings-appearance-settings-transparency-blur",
					Some(
						"settings-appearance-settings-restart-serein-after-changing-this-themes-can-customize-effects-while",
					),
					&mut self.transparency_blur,
				);
				if self.transparency_blur {
					design::card_divider(ui);
					design::slider_row(
						ui,
						"settings-appearance-settings-transparency",
						None,
						&mut self.transparency,
						0..=100,
						"%",
					);
					ui.add_space(8.0);
					design::blur_control(
						ui,
						"settings-appearance-settings-blur",
						"settings-appearance-settings-the-system-applies-its-standard-blur-strength",
						&mut self.blur,
					);
				}
			},
		);
		self.cosmetic_settings(ui);
		self.layout_settings(ui, demo);
	}

	/// Device-local switches for profile cosmetics. Hidden ones are neither drawn nor loaded.
	fn cosmetic_settings(&mut self, ui: &mut egui::Ui) {
		design::group(ui, &crate::i18n::translate("settings-cosmetics"), |ui| {
			let cosmetics = &mut self.cosmetics;
			for (index, (label, detail, value)) in [
				(
					"settings-cosmetics-decorations",
					"settings-cosmetics-decorations-description",
					&mut cosmetics.decorations,
				),
				(
					"settings-cosmetics-nameplates",
					"settings-cosmetics-nameplates-description",
					&mut cosmetics.nameplates,
				),
				(
					"settings-cosmetics-effects",
					"settings-cosmetics-effects-description",
					&mut cosmetics.effects,
				),
				(
					"settings-cosmetics-frames",
					"settings-cosmetics-frames-description",
					&mut cosmetics.frames,
				),
				(
					"settings-cosmetics-name-styles",
					"settings-cosmetics-name-styles-description",
					&mut cosmetics.name_styles,
				),
			]
			.into_iter()
			.enumerate()
			{
				if index > 0 {
					design::card_divider(ui);
				}
				design::switch(ui, label, Some(detail), value);
			}
			design::card_divider(ui);
			design::hint(
				ui,
				&crate::i18n::translate("settings-cosmetics-hidden-are-not-downloaded"),
			);
		});
	}

	/// Shows device-local reading, composer and channel-list preferences.
	fn chat_settings(&mut self, ui: &mut egui::Ui, demo: bool) {
		self.chat_reading_settings(ui, demo);
		design::group(ui, &crate::i18n::translate("settings-chat-box"), |ui| {
			design::switch(
				ui,
				"settings-convert-emoticons",
				Some("settings-convert-emoticons-description"),
				&mut self.convert_emoticons,
			);
		});
		design::group(
			ui,
			&crate::i18n::translate("settings-chat-settings-channel-list"),
			|ui| {
				design::switch(
					ui,
					"Show Permission-Locked Channels",
					Some("settings-chat-settings-show-channels-you-cannot-currently-access"),
					&mut self.show_hidden_channels,
				);
				design::switch(
					ui,
					"settings-preload-on-hover",
					Some("settings-preload-on-hover-description"),
					&mut self.preload_on_hover,
				);
			},
		);
		design::group(
			ui,
			&crate::i18n::translate("settings-chat-settings-emoji"),
			|ui| {
				let mut suggest = !self.hide_nitro_emojis;
				design::switch(
					ui,
					"settings-chat-settings-suggest-nitro-emojis",
					Some("settings-chat-settings-suggest-nitro-emojis-description"),
					&mut suggest,
				);
				self.hide_nitro_emojis = !suggest;
			},
		);
	}

	/// Built-in presets plus enabled community themes, one swatch each.
	fn colour_preset_settings(&mut self, ui: &mut egui::Ui) {
		let current = design::variant();
		let mut presets: Vec<_> = design::Variant::ALL
			.into_iter()
			.map(|variant| {
				(
					Some(variant),
					None,
					variant.label().to_owned(),
					design::builtin_colors(ui.visuals().dark_mode, variant),
				)
			})
			.collect();
		presets.extend(self.extensions.entries.iter().filter_map(|entry| {
			if !entry.enabled || entry.manifest.kind != extensions::ExtensionKind::Theme {
				return None;
			}
			Some((
				None,
				Some(entry.manifest.id.clone()),
				entry.manifest.name.clone(),
				design::theme_preview_palette(ui, entry.theme_preview.as_ref()?),
			))
		}));
		design::group(
			ui,
			&crate::i18n::translate("settings-colour-preset-settings-colour-preset"),
			|ui| {
				ui.horizontal_wrapped(|ui| {
					ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);
					for (variant, id, label, swatch) in presets {
						let selected = if let Some(active) = &self.extensions.active_theme {
							id.as_ref() == Some(active)
						} else {
							variant == Some(current)
						};
						let response = preset_swatch(ui, &label, &swatch, selected);
						if response.on_hover_text(&label).clicked()
							&& !selected && !self.extensions.busy
						{
							if let Some(variant) = variant {
								design::set_variant(variant);
								design::apply(ui.ctx());
								self.theme_variant_changed = Some(variant);
							}
							self.extensions
								.queue(ui.ctx(), crate::ExtensionRequest::SelectTheme { id });
						}
					}
				});
			},
		);
	}

	fn activity_settings(&mut self, ui: &mut egui::Ui, state: &State) {
		design::card(ui, |ui| {
			design::switch(
				ui,
				"settings-activity-settings-share-game-activity",
				Some(
					"settings-activity-settings-detect-running-games-and-ask-discord-to-share-them-as",
				),
				&mut self.share_game_activity,
			);
			design::card_divider(ui);
			let playing = self.own_game.is_some() || self.running_game.is_some();
			let action = if self.share_game_activity && state.gateway_connected && !state.demo {
				if self.discord_activity_sharing == Some(false) {
					Some(("settings-activity-enable-on-discord", true))
				} else if self.discord_activity_sharing_retry {
					Some(("settings-activity-check-again", false))
				} else {
					None
				}
			} else {
				None
			};
			// The game itself is shown under Current Game; this row is about sharing it.
			let title = if !self.share_game_activity {
				"settings-activity-sharing-is-off"
			} else if playing {
				"settings-activity-sharing-your-game"
			} else {
				"settings-activity-looking"
			};
			let detail = if state.demo {
				"settings-activity-demo-detail"
			} else {
				self.game_activity_status
			};
			design::row(ui, title, (!detail.is_empty()).then_some(detail), |ui| {
				if let Some((label, enable)) = action {
					ui.add_enabled_ui(!self.discord_activity_sharing_busy, |ui| {
						if design::button(ui, label, design::ButtonKind::Outline).clicked() {
							self.discord_activity_sharing_request = Some(enable);
						}
					});
				}
			});
		});
		ui.add_space(24.0);
		self.registered_games_settings(ui, state.demo);
	}

	/// Background workers can refresh usage only while its controls or progress are visible.
	pub fn force_sync_usage_visible(&self) -> bool {
		self.force_sync_open || (self.settings.open && self.settings.page == Page::Storage)
	}

	fn storage_page(&mut self, ui: &mut egui::Ui, state: &State) {
		self.force_sync_settings(ui, state);
		design::group(
			ui,
			&crate::i18n::translate("settings-storage-page-local-storage"),
			|ui| {
				design::row(
					ui,
					"settings-storage-page-clear-cache",
					Some(
						"settings-storage-page-removes-cached-messages-and-media-drafts-and-your-login-stay",
					),
					|ui| {
						ui.add_enabled_ui(!state.demo, |ui| {
							if design::button(
								ui,
								&crate::i18n::translate("settings-storage-page-clear-cache"),
								design::ButtonKind::Outline,
							)
							.clicked()
							{
								self.clear_cache_requested = true;
							}
						});
					},
				);
				if !state.demo && !self.storage_status.is_empty() {
					design::hint(ui, self.storage_status);
				}
				design::card_divider(ui);
				design::hint(
					ui,
					&crate::i18n::translate(
						"settings-storage-page-messages-and-drafts-are-cached-on-this-device-inside-bounded",
					),
				);
			},
		);
		self.image_cache_settings(ui, state.demo);
		design::group(ui, "settings-storage-page-saved-login", |ui| {
			ui.add_enabled_ui(!state.demo, |ui| {
				design::switch(
					ui,
					"settings-storage-page-plaintext-login",
					Some("settings-storage-page-plaintext-login-description"),
					&mut self.plaintext_login,
				);
			});
		});
		design::group(
			ui,
			&crate::i18n::translate("settings-storage-page-your-privacy"),
			|ui| {
				design::hint(
					ui,
					&crate::i18n::translate(
						"settings-storage-page-serein-does-not-collect-telemetry-or-upload-diagnostics-discord-retains",
					),
				);
			},
		);
	}
}

impl MessagingUi {
	/// Disk budgets for downloaded images, their measured use, and per-cache removal.
	fn image_cache_settings(&mut self, ui: &mut egui::Ui, demo: bool) {
		let usage = self.image_cache_usage;
		let detail = |description: &str, partition: model::CachePartition, limit: u32| {
			let used = match usage {
				Some(usage) => crate::i18n::translate_args(
					"settings-image-cache-usage",
					&[
						(
							"used",
							&format!(
								"{:.2} MiB",
								usage[partition as usize].bytes as f64 / 1_048_576.0
							),
						),
						("limit", &cache_size_label(limit)),
					],
				),
				None => crate::i18n::translate("settings-image-cache-usage-unavailable"),
			};
			format!("{} {used}", crate::i18n::translate(description))
		};
		let media = detail(
			"settings-image-cache-media-description",
			model::CachePartition::Media,
			self.image_cache.media_mib,
		);
		let artwork = detail(
			"settings-image-cache-artwork-description",
			model::CachePartition::Artwork,
			self.image_cache.artwork_mib,
		);
		let mut clear = None;
		let cache = &mut self.image_cache;
		design::group(ui, &crate::i18n::translate("settings-image-cache"), |ui| {
			for (index, (title, detail, partition, value, presets)) in [
				(
					"settings-image-cache-media",
					&media,
					model::CachePartition::Media,
					&mut cache.media_mib,
					&model::MEDIA_CACHE_MIB[..],
				),
				(
					"settings-image-cache-artwork",
					&artwork,
					model::CachePartition::Artwork,
					&mut cache.artwork_mib,
					&model::ARTWORK_CACHE_MIB[..],
				),
			]
			.into_iter()
			.enumerate()
			{
				if index > 0 {
					design::card_divider(ui);
				}
				design::row(ui, title, Some(detail), |ui| {
					preset_combo(ui, title, value, presets, |mib| cache_size_label(*mib));
					ui.add_enabled_ui(!demo, |ui| {
						if design::text_action(
							ui,
							&crate::i18n::translate("settings-image-cache-clear"),
						)
						.on_hover_text(crate::i18n::translate(
							"settings-image-cache-clear-description",
						))
						.clicked()
						{
							clear = Some(partition);
						}
					});
				});
			}
			design::card_divider(ui);
			design::row(
				ui,
				"settings-image-cache-retention",
				Some("settings-image-cache-retention-description"),
				|ui| {
					preset_combo(
						ui,
						"settings-image-cache-retention",
						&mut cache.retention_days,
						&model::CACHE_RETENTION_DAYS,
						|days| {
							crate::i18n::translate_count(
								"settings-image-cache-days",
								usize::from(*days),
								&[],
							)
						},
					);
				},
			);
		});
		if clear.is_some() {
			self.clear_image_cache = clear;
		}
	}
}

/// Compact picker for one bounded preset list.
fn preset_combo<T: Copy + PartialEq>(
	ui: &mut egui::Ui,
	id: &str,
	value: &mut T,
	presets: &[T],
	label: impl Fn(&T) -> String,
) {
	egui::ComboBox::from_id_salt(id)
		.selected_text(label(value))
		.width(ui.available_width().min(120.0))
		.show_ui(ui, |ui| {
			for preset in presets {
				ui.selectable_value(value, *preset, label(preset));
			}
		});
}

fn cache_size_label(mib: u32) -> String {
	if mib >= 1024 && mib.is_multiple_of(1024) {
		format!("{} GiB", mib / 1024)
	} else {
		format!("{mib} MiB")
	}
}

fn account_row(ui: &mut egui::Ui, label: &str, value: &str) {
	let label = crate::i18n::translate_if_key(label);
	let colors = design::palette(ui);
	ui.horizontal(|ui| {
		ui.vertical(|ui| {
			ui.set_width((ui.available_width() - 160.0).max(100.0));
			ui.spacing_mut().item_spacing.y = 2.0;
			ui.label(design::eyebrow(ui, label, colors.muted));
			ui.add(
				egui::Label::new(RichText::new(value).size(15.0).color(colors.text_strong))
					.truncate(),
			);
		});
	});
}

/// Reserve the close column before wrapping a page's title and description.
fn page_header(
	ui: &mut egui::Ui,
	heading: &str,
	description: &str,
	compact: bool,
) -> egui::Response {
	let colors = design::palette(ui);
	ui.horizontal_top(|ui| {
		let text_width = (ui.available_width() - 40.0 - ui.spacing().item_spacing.x).max(1.0);
		ui.allocate_ui_with_layout(
			egui::vec2(text_width, 0.0),
			egui::Layout::top_down(egui::Align::Min),
			|ui| {
				ui.set_width(text_width);
				ui.spacing_mut().item_spacing.y = 2.0;
				let title = egui::Label::new(
					design::semibold(ui, heading, if compact { 20.0 } else { 24.0 })
						.color(colors.text_strong),
				);
				let title = ui.add(if compact {
					title.truncate()
				} else {
					title.wrap()
				});
				if compact {
					title.on_hover_text(description);
				} else {
					ui.add(
						egui::Label::new(RichText::new(description).size(13.0).color(colors.muted))
							.wrap(),
					);
				}
			},
		);
		close_control_mode(ui, compact)
	})
	.inner
}

/// Sidebar entry in the settings modal; the selected page uses the strong surface and text.
pub(super) fn nav_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
	let label = crate::i18n::translate_if_key(label);
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, &label)
	});
	let hot = response.hovered() || response.has_focus();
	if selected {
		design::selection_surface(ui, rect);
	} else if hot {
		ui.painter().rect_filled(rect, 8, colors.hover);
	}
	design::focus_outline(ui, &response, 8);
	let color = if selected {
		colors.text_strong
	} else if hot {
		colors.text
	} else {
		colors.muted
	};
	// Long localized page names elide inside the row; hovering shows the full name.
	let mut job = egui::text::LayoutJob::simple_singleline(
		label.clone(),
		egui::FontId::new(15.0, design::medium_family(ui.ctx())),
		color,
	);
	job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 24.0);
	let galley = ui.painter().layout_job(job);
	let elided = galley.elided;
	ui.painter().galley(
		egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
		galley,
		color,
	);
	if elided {
		response.on_hover_text(label)
	} else {
		response
	}
}

/// Discord's round close button with the "ESC" hint underneath.
pub(super) fn close_control(ui: &mut egui::Ui) -> egui::Response {
	close_control_mode(ui, false)
}

fn close_control_mode(ui: &mut egui::Ui, compact: bool) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(40.0, if compact { 40.0 } else { 56.0 }),
		egui::Sense::click(),
	);
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Button,
			ui.is_enabled(),
			crate::i18n::translate("settings-close-control-close-settings-esc"),
		)
	});
	let hot = response.hovered() || response.has_focus();
	let center = egui::pos2(rect.center().x, rect.top() + 18.0);
	ui.painter().circle(
		center,
		18.0,
		if hot {
			colors.hover
		} else {
			egui::Color32::TRANSPARENT
		},
		egui::Stroke::new(2.0, if hot { colors.text } else { colors.muted }),
	);
	icons::paint(
		ui.painter(),
		icons::Icon::Close,
		egui::Rect::from_center_size(center, egui::Vec2::splat(16.0)),
		if hot {
			colors.text_strong
		} else {
			colors.muted
		},
	);
	if !compact {
		ui.painter().text(
			egui::pos2(rect.center().x, rect.bottom() - 6.0),
			egui::Align2::CENTER_CENTER,
			"ESC",
			egui::FontId::new(11.0, design::semibold_family(ui.ctx())),
			colors.muted,
		);
	}
	response.on_hover_text(crate::i18n::translate(
		"settings-close-control-close-settings-esc",
	))
}

/// Dark, light or system cards with a miniature of each palette and a radio marker.
/// One colour-preset cell: the palette circles, the selection ring and the caption.
fn preset_swatch(
	ui: &mut egui::Ui,
	label: &str,
	swatch: &design::Palette,
	selected: bool,
) -> egui::Response {
	let label = crate::i18n::translate_if_key(label);
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(76.0, 70.0), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, &label)
	});
	let painter = &ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
	if response.hovered() || response.has_focus() {
		painter.rect_filled(rect, 6, colors.hover);
	}
	let center = egui::pos2(rect.center().x, rect.top() + 24.0);
	let [top, bottom] = swatch.backdrop.unwrap_or_else(|| {
		// Opaque presets run from their lightest surface to their darkest one.
		let mut surfaces = [swatch.chat, swatch.selected, swatch.base];
		surfaces.sort_by(|a, b| design::luminance(*b).total_cmp(&design::luminance(*a)));
		[surfaces[0], surfaces[2]]
	});
	gradient_circle(painter, center, 20.0, top, bottom);
	painter.circle_stroke(
		center,
		20.0,
		egui::Stroke::new(
			if selected { 2.5 } else { 1.0 },
			if selected {
				colors.accent
			} else {
				colors.border
			},
		),
	);
	if selected {
		painter.circle_filled(center, 10.0, colors.accent);
		icons::paint(
			painter,
			icons::Icon::Check,
			egui::Rect::from_center_size(center, egui::Vec2::splat(12.0)),
			colors.accent_text,
		);
	}
	painter.text(
		egui::pos2(rect.center().x, rect.bottom() - 10.0),
		egui::Align2::CENTER_CENTER,
		&label,
		egui::FontId::proportional(11.0),
		if selected {
			colors.text_strong
		} else {
			colors.muted
		},
	);
	response
}

/// Diagonal gradient from `top` (top-left) to `bottom` (bottom-right), like the window backdrop.
fn gradient_circle(
	painter: &egui::Painter,
	center: egui::Pos2,
	radius: f32,
	top: egui::Color32,
	bottom: egui::Color32,
) {
	const SEGMENTS: u32 = 48;
	let color_at = |offset: egui::Vec2| {
		let t = (offset.x + offset.y) / (2.0 * std::f32::consts::SQRT_2 * radius) + 0.5;
		top.lerp_to_gamma(bottom, t)
	};
	let mut mesh = egui::Mesh::default();
	mesh.colored_vertex(center, color_at(egui::Vec2::ZERO));
	for i in 0..SEGMENTS {
		let offset =
			egui::Vec2::angled(i as f32 * std::f32::consts::TAU / SEGMENTS as f32) * radius;
		mesh.colored_vertex(center + offset, color_at(offset));
		mesh.add_triangle(0, 1 + i, 1 + (i + 1) % SEGMENTS);
	}
	painter.add(egui::Shape::mesh(mesh));
}

fn theme_preference_cards(ui: &mut egui::Ui) {
	let colors = design::palette(ui);
	let current = ui.ctx().options(|options| options.theme_preference);
	let mut chosen = None;
	ui.horizontal(|ui| {
		ui.spacing_mut().item_spacing.x = 10.0;
		let width = ((ui.available_width() - 20.0) / 3.0).clamp(88.0, 240.0);
		// Narrow cards (the signed-out appearance popup) cannot hold the long system label.
		for (preference, label) in [
			(
				egui::ThemePreference::Dark,
				"theme-editor-appearance-switch-dark",
			),
			(
				egui::ThemePreference::Light,
				"theme-editor-appearance-switch-light",
			),
			(
				egui::ThemePreference::System,
				if width < 140.0 {
					"language-system"
				} else {
					"settings-appearance-sync-with-system"
				},
			),
		] {
			let label = crate::i18n::translate_if_key(label);
			let selected = current == preference;
			let (rect, response) =
				ui.allocate_exact_size(egui::vec2(width, 76.0), egui::Sense::click());
			response.widget_info(|| {
				egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, &label)
			});
			let painter = ui.painter();
			painter.rect(
				rect,
				8,
				if response.hovered() {
					colors.hover
				} else {
					colors.raised
				},
				egui::Stroke::new(
					if selected { 2.0 } else { 1.0 },
					if selected {
						colors.accent
					} else {
						colors.border
					},
				),
				egui::StrokeKind::Inside,
			);
			let swatch = egui::Rect::from_min_size(
				rect.min + egui::vec2(12.0, 12.0),
				egui::vec2(52.0, 34.0),
			);
			let variant = design::variant();
			let (left, right) = match preference {
				egui::ThemePreference::Dark => {
					let p = design::control_colors(true, variant);
					(p.sidebar.to_opaque(), p.chat.to_opaque())
				}
				egui::ThemePreference::Light => {
					let p = design::control_colors(false, variant);
					(p.sidebar.to_opaque(), p.chat.to_opaque())
				}
				egui::ThemePreference::System => (
					design::control_colors(true, variant).chat,
					design::control_colors(false, variant).chat,
				),
			};
			painter.rect_filled(swatch, 6, right);
			painter.rect_filled(
				swatch.with_max_x(swatch.left() + swatch.width() * 0.42),
				egui::CornerRadius {
					nw: 6,
					sw: 6,
					ne: 0,
					se: 0,
				},
				left,
			);
			painter.rect_stroke(
				swatch,
				6,
				egui::Stroke::new(1.0, colors.border),
				egui::StrokeKind::Inside,
			);
			let radio = egui::pos2(rect.right() - 20.0, rect.top() + 20.0);
			painter.circle_stroke(
				radio,
				8.0,
				egui::Stroke::new(
					2.0,
					if selected {
						colors.accent
					} else {
						colors.muted
					},
				),
			);
			if selected {
				painter.circle_filled(radio, 4.5, colors.accent);
			}
			painter.text(
				egui::pos2(rect.left() + 12.0, rect.bottom() - 14.0),
				egui::Align2::LEFT_CENTER,
				&label,
				egui::FontId::new(14.0, design::medium_family(ui.ctx())),
				if selected {
					colors.text_strong
				} else {
					colors.text
				},
			);
			if response.clicked() {
				chosen = Some(preference);
			}
		}
	});
	if let Some(preference) = chosen {
		ui.ctx().set_theme(preference);
	}
}

#[cfg(test)]
mod keybind_tests {
	use super::*;
	#[test]
	fn profile_settings_escape_keeps_ime_composition_and_new_pages_work_narrow() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		let mut state = test_support::demo_state();
		view.settings.open = true;
		view.settings.page = Page::Accessibility;
		let escape = egui::Event::Key {
			key: egui::Key::Escape,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		let frames = [
			vec![
				egui::Event::Ime(egui::ImeEvent::Preedit {
					text: "編集中".into(),
					active_range_chars: None,
				}),
				escape.clone(),
			],
			vec![escape.clone()],
			vec![
				egui::Event::Ime(egui::ImeEvent::Commit("編集中".into())),
				escape.clone(),
			],
			vec![escape],
		];
		for (index, events) in frames.into_iter().enumerate() {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(360.0, 640.0),
					)),
					events,
					..Default::default()
				},
				|ui| view.show_settings(ui.ctx(), &mut state, &mut Vec::new()),
			);
			output.drop_without_applying_deltas();
			assert_eq!(view.settings.open, index != 3, "frame {index}");
		}
		for page in [
			Page::Privacy,
			Page::ContentFilters,
			Page::Connections,
			Page::Accessibility,
		] {
			view.settings.open = true;
			view.settings.page = page;
			for theme in [egui::Theme::Light, egui::Theme::Dark] {
				ctx.set_theme(theme);
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(360.0, 640.0),
						)),
						..Default::default()
					},
					|ui| view.show_settings(ui.ctx(), &mut state, &mut Vec::new()),
				);
				assert!(!output.shapes.is_empty());
				output.drop_without_applying_deltas();
				assert!(view.settings.open);
			}
		}
	}

	#[test]
	fn long_page_headers_wrap_without_displacing_the_close_control() {
		for width in [240.0, 420.0, 640.0] {
			let ctx = egui::Context::default();
			let heading = "A long localized settings heading that needs to wrap";
			let description =
				"This description deliberately occupies several lines in a narrow settings pane. "
					.repeat(3);
			let frame = |events| {
				let mut close = egui::Rect::NOTHING;
				let mut clicked = false;
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width + 32.0, 400.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						ui.set_width(width);
						let available = ui.available_rect_before_wrap();
						let response = page_header(ui, heading, &description, false);
						close = response.rect;
						clicked = response.clicked();
						assert!(
							close.right() <= available.right() + 0.1,
							"width={width} close={close:?}"
						);
						assert_eq!(close.width(), 40.0);
					},
				);
				for shape in &output.shapes {
					if let egui::Shape::Text(text) = &shape.shape {
						assert!(
							text.pos.x + text.galley.size().x <= width + 1.0,
							"text exceeds pane at width={width}"
						);
					}
				}
				output.drop_without_applying_deltas();
				(clicked, close)
			};
			frame(vec![]);
			let (_, close) = frame(vec![]);
			let point = close.center();
			let pointer = |pressed| {
				vec![
					egui::Event::PointerMoved(point),
					egui::Event::PointerButton {
						pos: point,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				]
			};
			frame(pointer(true));
			assert!(
				frame(pointer(false)).0,
				"close is unreachable at width={width}"
			);
		}
	}

	#[test]
	fn keybinds_shortcut_opens_page_and_respects_ime() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		for ime in [true, false] {
			view.ime_active = ime;
			view.settings.query = "theme".into();
			let mut output = ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key: egui::Key::Slash,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::COMMAND,
					}],
					..Default::default()
				},
				|ui| view.keybinds_shortcut(ui.ctx()),
			);
			output.textures_delta.clear();
			assert_eq!(view.settings.open, !ime);
			if !ime {
				assert!(view.settings.page == Page::Keybinds);
				assert!(view.settings.query.is_empty());
			}
		}
	}
}
