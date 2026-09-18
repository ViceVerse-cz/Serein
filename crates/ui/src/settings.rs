//! User settings modal in Discord's layout: a sidebar of pages on the left, the selected page
//! on the right, and a round close control with its Escape hint.
use crate::{MessagingUi, design, icons};
use client_core::State;
use egui::RichText;

#[derive(Default)]
pub(super) struct Settings {
	pub open: bool,
	page: Page,
	query: String,
	editor: crate::profile_edit::Editor,
	pub(super) notifications: crate::notification_settings::Navigation,
	pub(super) messaging_permissions: crate::messaging_permissions::Navigation,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
	Account,
	Profile,
	General,
	#[default]
	Appearance,
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
	const ALL: [Self; 13] = [
		Self::Account,
		Self::Profile,
		Self::General,
		Self::Appearance,
		Self::MessagingPermissions,
		Self::Notifications,
		Self::Activity,
		Self::Voice,
		Self::Keybinds,
		Self::Storage,
		Self::Updates,
		Self::Extensions,
		Self::Themes,
	];
	const USER: [Self; 2] = [Self::Account, Self::Profile];
	const APP: [Self; 11] = [
		Self::General,
		Self::Appearance,
		Self::MessagingPermissions,
		Self::Notifications,
		Self::Activity,
		Self::Voice,
		Self::Keybinds,
		Self::Storage,
		Self::Updates,
		Self::Extensions,
		Self::Themes,
	];
	fn label(self) -> &'static str {
		match self {
			Self::Account => "My Account",
			Self::Profile => "Profile",
			Self::General => "General",
			Self::Appearance => "Appearance",
			Self::MessagingPermissions => "Messaging Permissions",
			Self::Notifications => "Notifications",
			Self::Activity => "Game Activity",
			Self::Voice => "Voice & Audio",
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
			Self::Appearance => "Theme, colour preset, zoom and layout.",
			Self::MessagingPermissions => {
				"Control who can contact you and how messages are filtered."
			}
			Self::Notifications => "Choose which notifications you receive and how they appear.",
			Self::Activity => "Show others what you are playing.",
			Self::Voice => "Microphone, speakers and voice processing.",
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
				"general windows macos login menu bar startup autostart automatically open minimized minimize close tray background graphics gpu adapter render discrete integrated hardware acceleration performance battery"
			}
			Self::Appearance => {
				"appearance customization primary accent hex window transparency blur title bar caption tray minimize theme dark light system zoom reading layout sidebar people reset colour color preset animate animated gifs autoplay hide image links confirm confirmation external browser"
			}
			Self::MessagingPermissions => {
				"messaging permissions spam filters direct messages dm friend requests personalized connected games"
			}
			Self::Notifications => {
				"notifications desktop system alerts overview sounds badges message ring"
			}
			Self::Activity => "game activity playing osu status presence sharing",
			Self::Voice => {
				"voice audio microphone speakers devices volume gain noise suppression push to talk"
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
}

impl MessagingUi {
	pub(super) fn theme_preview_navigation(&mut self, ui: &mut egui::Ui) {
		if self.extensions.begin_gallery_preview(ui.ctx()) {
			self.settings.open = false;
		}
		if self.settings.open {
			self.extensions.stop_theme_preview(ui.ctx());
		} else if self.extensions.theme_preview_bar(ui) {
			self.settings.open = true;
			self.settings.page = Page::Themes;
			self.settings.query.clear();
		}
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

	pub(super) fn open_voice_settings(&mut self) {
		self.settings.open = true;
		self.settings.page = Page::Voice;
		self.settings.query.clear();
	}

	/// Fixture-only entry point for the native offline settings preview.
	pub fn preview_settings(&mut self, page: &str) {
		self.settings.open = true;
		if let Some(page) = Page::ALL
			.into_iter()
			.find(|candidate| candidate.label().to_lowercase().contains(page))
		{
			self.settings.page = page;
		}
	}
	pub(super) fn show_settings(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<client_core::Command>,
	) {
		let colors = design::palette_for(ctx);
		let size = ctx.content_rect().size() - egui::vec2(32.0, 40.0);
		let width = size.x.clamp(280.0, 1100.0);
		let height = size.y.clamp(240.0, 820.0);
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
						left: if wide { 40 } else { 20 },
						right: 20,
						top: 24,
						bottom: 24,
					}))
					.show(ui, |ui| {
						let editing_theme =
							self.settings.page == Page::Themes && self.extensions.editing_theme();
						ui.horizontal_top(|ui| {
							ui.vertical(|ui| {
								ui.spacing_mut().item_spacing.y = 2.0;
								ui.label(
									design::semibold(
										ui,
										if editing_theme {
											"Theme maker"
										} else {
											self.settings.page.label()
										},
										20.0,
									)
									.color(colors.text_strong),
								);
								ui.label(
									RichText::new(if editing_theme {
										"Make it yours. Preview changes in your conversations."
									} else {
										self.settings.page.description()
									})
									.size(13.0)
									.color(colors.muted),
								);
							});
							ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
								if close_control(ui).clicked() {
									self.settings.open = false;
								}
							});
						});
						if !wide {
							ui.add_space(8.0);
							self.settings_search(ui);
							egui::ComboBox::from_id_salt("settings-page")
								.selected_text(self.settings.page.label())
								.width(ui.available_width())
								.show_ui(ui, |ui| {
									for page in Page::ALL {
										if page.matches(&self.settings.query.to_lowercase()) {
											ui.selectable_value(
												&mut self.settings.page,
												page,
												page.label(),
											);
										}
									}
								});
						}
						ui.add_space(16.0);
						if self.settings.page == Page::Themes && self.extensions.editing_theme() {
							self.extensions.theme_editor_toolbar(ui);
							ui.add_space(12.0);
						}
						egui::ScrollArea::vertical()
							.id_salt((
								"settings-content",
								self.settings.page as u8,
								self.extensions.theme_editor_tab_key(),
							))
							.auto_shrink([false, false])
							.show(ui, |ui| {
								let scroll_padding = if self.settings.page == Page::Profile {
									8.0
								} else {
									0.0
								};
								ui.set_width((ui.available_width() - scroll_padding).min(720.0));
								ui.spacing_mut().item_spacing.y = 12.0;
								let query = self.settings.query.to_lowercase();
								if !Page::ALL.into_iter().any(|p| p.matches(&query)) {
									ui.label(
										design::semibold(ui, "No settings found", 16.0)
											.color(colors.text_strong),
									);
									ui.weak("Try theme, notifications, voice, or cache.");
									return;
								}
								match self.settings.page {
									Page::General => self.general_settings(ui, state.demo),
									Page::Account => self.account_page(ui, state),
									Page::Profile => self.settings.editor.show(
										ui,
										state,
										&mut self.avatars,
										commands,
									),
									Page::Appearance => {
										self.appearance_settings(ui);
										ui.add_space(8.0);
										self.reading_settings(ui, state.demo);
									}
									Page::MessagingPermissions => {
										self.messaging_permissions_settings(ui, state, commands)
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
								ui.add_space(24.0);
							});
					});
			});
		if modal.should_close() {
			self.settings.open = false;
		}
		if self.keybind_capture.is_none()
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
		}
	}

	fn settings_navigation(&mut self, ui: &mut egui::Ui, state: &State) {
		let colors = design::palette(ui);
		egui::ScrollArea::vertical()
			.id_salt("settings-navigation-scroll")
			.auto_shrink([false, false])
			.show(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 2.0;
				self.settings_search(ui);
				ui.add_space(12.0);
				let query = self.settings.query.to_lowercase();
				for (heading, pages) in [
					("User settings", &Page::USER[..]),
					("App settings", &Page::APP[..]),
				] {
					let visible: Vec<Page> = pages
						.iter()
						.copied()
						.filter(|page| page.matches(&query))
						.collect();
					if visible.is_empty() {
						continue;
					}
					ui.add_space(6.0);
					ui.add(egui::Label::new(design::eyebrow(ui, heading, colors.muted)));
					ui.add_space(2.0);
					for page in visible {
						if nav_item(ui, page.label(), self.settings.page == page).clicked() {
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
					RichText::new(format!("Serein {}", self.build.version))
						.size(12.0)
						.color(colors.muted),
				);
				ui.label(
					RichText::new("Unofficial · not endorsed by Discord")
						.size(11.0)
						.color(colors.muted),
				);
				ui.add_space(2.0);
				let copied = self
					.updates
					.copied_diagnostics
					.is_some_and(|until| ui.input(|i| i.time) < until);
				let link_text = if copied {
					RichText::new("✓ Copied issue info")
						.size(11.0)
						.color(colors.positive)
				} else {
					RichText::new("Copy issue info")
						.size(11.0)
						.color(colors.muted)
				};
				if ui
					.link(link_text)
					.on_hover_text("Copy environment information formatted for GitHub issues")
					.clicked()
				{
					self.copy_diagnostic_info(ui.ctx());
				}
			});
	}

	fn settings_search(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		egui::Frame::new()
			.fill(colors.raised)
			.corner_radius(6)
			.inner_margin(egui::Margin::symmetric(8, 4))
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					let response = ui.add(
						egui::TextEdit::singleline(&mut self.settings.query)
							.hint_text("Search")
							.char_limit(64)
							.frame(egui::Frame::NONE)
							.desired_width(ui.available_width() - 24.0),
					);
					icons::inline(ui, icons::Icon::Search, 16.0, colors.muted);
					if response.changed() {
						let query = self.settings.query.to_lowercase();
						if !self.settings.page.matches(&query)
							&& let Some(page) = Page::ALL.into_iter().find(|p| p.matches(&query))
						{
							self.settings.page = page;
						}
					}
				});
			});
	}

	fn settings_logout(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = design::palette(ui);
		let label = if demo { "Exit preview" } else { "Log out" };
		let (rect, response) =
			ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
		});
		if response.hovered() || response.has_focus() {
			ui.painter().rect_filled(rect, 4, colors.hover);
		}
		ui.painter().text(
			egui::pos2(rect.left() + 10.0, rect.center().y),
			egui::Align2::LEFT_CENTER,
			label,
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

	fn account_page(&mut self, ui: &mut egui::Ui, state: &State) {
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
									RichText::new(if state.demo {
										"Offline preview · synthetic account"
									} else {
										"Signed in with your Discord account"
									})
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
								account_row(ui, "Display name", &name);
								ui.separator();
								account_row(
									ui,
									"Email, password and security",
									"Managed in Discord",
								);
							});
						if ui.button("Edit profile").clicked() {
							self.settings.page = Page::Profile;
						}
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
						self.avatars.show(ui, user, 80.0, state.demo);
					} else {
						design::avatar(ui, &name, 80.0);
					}
				});
			});
		ui.add_space(8.0);
		ui.label(design::eyebrow(ui, "Session", colors.muted));
		design::card(ui, |ui| {
			ui.horizontal(|ui| {
				ui.vertical(|ui| {
					ui.set_width((ui.available_width() - 140.0).max(120.0));
					ui.spacing_mut().item_spacing.y = 2.0;
					ui.label(
						design::medium(
							ui,
							if state.demo {
								"Exit preview"
							} else {
								"Log out"
							},
							15.0,
						)
						.color(colors.text_strong),
					);
					ui.label(
						RichText::new(if state.demo {
							"Closes the offline fixture. Nothing is stored for the preview."
						} else {
							"Removes the saved login and clears this account's local cache and drafts."
						})
						.size(13.0)
						.color(colors.muted),
					);
				});
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if ui
						.add(
							egui::Button::new(
								design::medium(
									ui,
									if state.demo {
										"Exit preview"
									} else {
										"Log out"
									},
									14.0,
								)
								.color(egui::Color32::WHITE),
							)
							.fill(colors.danger)
							.corner_radius(6),
						)
						.clicked()
					{
						self.logout_requested = true;
						self.settings.open = false;
					}
				});
			});
		});
	}

	fn general_settings(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = design::palette(ui);
		ui.add_enabled_ui(self.startup_available && !self.startup_busy, |ui| {
			design::switch(
				ui,
				"Automatically open Serein when your computer starts up",
				None,
				&mut self.startup_enabled,
			);
			ui.add_space(12.0);
			ui.add_enabled_ui(self.startup_enabled, |ui| {
				design::switch(
					ui,
					"Start Serein minimized",
					Some("Start Serein in the background, out of your way."),
					&mut self.startup_minimized,
				);
			});
		});
		if !self.startup_status.is_empty() {
			ui.label(
				RichText::new(self.startup_status)
					.size(12.0)
					.color(colors.muted),
			);
			if self.startup_available
				&& !self.startup_busy
				&& ui.button("Turn off startup").clicked()
			{
				self.startup_disable_requested = true;
			}
		}
		if !self.startup_available {
			ui.label("Automatic startup is currently available on Windows and macOS only.");
		}
		ui.add_space(12.0);
		ui.add_enabled_ui(self.tray_available, |ui| {
			design::switch(
				ui,
				if cfg!(target_os = "macos") {
					"Show Serein in the menu bar"
				} else {
					"Show Serein in System Tray"
				},
				Some(if cfg!(target_os = "macos") {
					"Closing the window keeps Serein in the menu bar. Quit from its menu to exit."
				} else if cfg!(target_os = "linux") {
					"Close keeps Serein running. Use the tray to show, minimize or quit. On Wayland, your desktop controls minimization and restoration."
				} else {
					"Closing the window keeps Serein in the notification area. Quit from its menu to exit."
				}),
				&mut self.minimize_to_tray,
			);
		});
		let status = if self.tray_available {
			self.tray_status
		} else {
			"Tray is unavailable on this platform."
		};
		if !status.is_empty() {
			ui.label(RichText::new(status).size(12.0).color(colors.muted));
		}
		ui.add_space(12.0);
		ui.label(design::eyebrow(ui, "Graphics", colors.muted));
		design::card(ui, |ui| {
			ui.horizontal(|ui| {
				let label = ui.label("Render with");
				egui::ComboBox::from_id_salt("gpu-preference")
					.selected_text(self.gpu_preference.label())
					.width(ui.available_width().min(240.0))
					.show_ui(ui, |ui| {
						for preference in model::GpuPreference::ALL {
							ui.selectable_value(
								&mut self.gpu_preference,
								preference,
								preference.label(),
							)
							.on_hover_text(preference.description());
						}
					})
					.response
					.labelled_by(label.id);
			});
			ui.weak(if self.gpu_adapter.is_empty() {
				"Takes effect the next time Serein starts.".to_owned()
			} else {
				format!(
					"Currently drawing with {}. Takes effect the next time Serein starts.",
					self.gpu_adapter
				)
			});
		});
		if demo {
			ui.add_space(12.0);
			ui.label(
				RichText::new("Offline preview. Startup settings are not saved.")
					.size(12.0)
					.color(colors.muted),
			);
		}
	}

	/// Compact appearance popup for the signed-out header: mode, colour preset and zoom.
	/// Deliberately narrower than the settings page; everything else lives in Settings.
	pub fn appearance_menu(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		ui.set_min_width(324.0);
		ui.set_max_width(324.0);
		ui.spacing_mut().item_spacing.y = 6.0;
		ui.label(design::eyebrow(ui, "Mode", colors.muted));
		theme_preference_cards(ui);
		ui.add_space(6.0);
		ui.label(design::eyebrow(ui, "Theme", colors.muted));
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
		ui.label(design::eyebrow(ui, "Display", colors.muted));
		let mut value = self.reading_preferences;
		ui.spacing_mut().slider_width = 96.0;
		self.zoom_row(ui, &mut value);
		if value != self.reading_preferences {
			self.apply_reading_preferences(ui.ctx(), value);
		}
	}

	fn appearance_settings(&mut self, ui: &mut egui::Ui) {
		self.extensions.reset_theme_button(ui);
		let colors = design::palette(ui);
		ui.add_space(8.0);
		ui.label(design::eyebrow(ui, "Theme", colors.muted));
		theme_preference_cards(ui);
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		{
			ui.add_space(8.0);
			ui.label(design::eyebrow(ui, "Window", colors.muted));
			design::card(ui, |ui| {
				design::switch(
					ui,
					"Hide Serein title bar",
					Some("Use the system title bar and window buttons instead."),
					&mut self.hide_title_bar,
				);
			});
		}
		ui.add_space(8.0);
		ui.label(design::eyebrow(ui, "Customization", colors.muted));
		design::card(ui, |ui| {
			design::switch(
				ui,
				"Transparency & blur",
				Some("Let the desktop show through the app. Custom themes can override this."),
				&mut self.transparency_blur,
			);
			if self.transparency_blur {
				ui.indent("window-effects", |ui| {
					ui.horizontal(|ui| {
						ui.label("Transparency");
						ui.add(egui::Slider::new(&mut self.transparency, 0..=100).suffix("%"));
					});
					ui.horizontal(|ui| {
						ui.label("Blur");
						ui.add(egui::Slider::new(&mut self.blur, 0..=100).suffix("%"))
							.on_hover_text(
								"Zero disables blur; the native compositor controls its exact strength.",
							);
					});
					design::switch(
						ui,
						"Apply to all surfaces",
						Some("Include sidebars, server rail, headers, and composer."),
						&mut self.transparent_all,
					);
				});
			}
			ui.separator();
			ui.horizontal(|ui| {
				let label = ui.label("Primary color");
				let mut color = self.primary_color.unwrap_or(design::DEFAULT_PRIMARY_COLOR);
				if design::color_edit(ui, &mut color)
					.labelled_by(label.id)
					.on_hover_text("Choose primary color")
					.changed()
				{
					self.primary_color = Some(color);
				}
				if ui
					.add_enabled(self.primary_color.is_some(), egui::Button::new("Reset"))
					.clicked()
				{
					self.primary_color = None;
				}
			});
			ui.weak("Used for buttons, selection and message highlights.");
		});
		ui.add_space(8.0);
		ui.label(design::eyebrow(ui, "Colour preset", colors.muted));
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
		let active_label = presets
			.iter()
			.find(|(variant, id, _, _)| {
				if let Some(active) = &self.extensions.active_theme {
					id.as_ref() == Some(active)
				} else {
					*variant == Some(current)
				}
			})
			.map_or(current.label(), |(_, _, label, _)| label.as_str())
			.to_owned();
		design::card(ui, |ui| {
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
			ui.add_space(4.0);
			ui.label(
				RichText::new(format!(
					"{} · saved with your appearance. Gradient presets always use dark text.",
					active_label
				))
				.size(12.0)
				.color(colors.muted),
			);
		});
		ui.add_space(8.0);
		ui.label(design::eyebrow(ui, "Channel list", colors.muted));
		design::card(ui, |ui| {
			design::switch(
				ui,
				"Show hidden channels",
				Some("Show channels you cannot currently access."),
				&mut self.show_hidden_channels,
			);
		});
	}

	fn activity_settings(&mut self, ui: &mut egui::Ui, state: &State) {
		let colors = design::palette(ui);
		design::card(ui, |ui| {
			design::switch(
				ui,
				"Share game activity",
				Some("Detect running games and ask Discord to share them as activity."),
				&mut self.share_game_activity,
			);
			ui.separator();
			let game = self
				.own_game
				.as_deref()
				.filter(|_| self.share_game_activity);
			ui.label(
				design::medium(
					ui,
					game.map_or_else(
						|| {
							if self.share_game_activity {
								"Looking for a running game".into()
							} else {
								"Activity sharing is off".into()
							}
						},
						str::to_owned,
					),
					16.0,
				)
				.color(colors.text_strong),
			);
			ui.label(
				egui::RichText::new(if state.demo {
					"Offline preview: synthetic activity, never shared or saved."
				} else {
					self.game_activity_status
				})
				.size(12.0)
				.color(colors.muted),
			);
			if self.share_game_activity && state.gateway_connected && !state.demo {
				let action = if self.discord_activity_sharing == Some(false) {
					Some(("Enable on Discord", true))
				} else if self.discord_activity_sharing_retry {
					Some(("Check Discord setting again", false))
				} else {
					None
				};
				if let Some((label, enable)) = action
					&& ui
						.add_enabled(
							!self.discord_activity_sharing_busy,
							egui::Button::new(label),
						)
						.clicked()
				{
					self.discord_activity_sharing_request = Some(enable);
				}
			}
		});
	}

	fn storage_page(&mut self, ui: &mut egui::Ui, state: &State) {
		let colors = design::palette(ui);
		ui.label(design::eyebrow(ui, "Local storage", colors.muted));
		design::card(ui, |ui| {
			ui.label(
				"Messages and drafts are cached on this device inside bounded, account-isolated files. Local cache data is not encrypted by Serein; saved login tokens use the OS credential store.",
			);
			ui.label(
				RichText::new(if state.demo {
					"Preview uses session memory only."
				} else {
					self.storage_status
				})
				.size(12.0)
				.color(colors.muted),
			);
			ui.separator();
			ui.horizontal(|ui| {
				ui.vertical(|ui| {
					ui.set_width((ui.available_width() - 140.0).max(120.0));
					ui.spacing_mut().item_spacing.y = 2.0;
					ui.label(design::medium(ui, "Clear cache", 15.0).color(colors.text_strong));
					ui.label(
						RichText::new(
							"Removes cached messages and media. Drafts and your login stay.",
						)
						.size(13.0)
						.color(colors.muted),
					);
				});
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if ui
						.add_enabled(
							!state.demo,
							egui::Button::new(design::medium(ui, "Clear cache", 14.0))
								.stroke(egui::Stroke::new(1.0, colors.border))
								.corner_radius(6),
						)
						.clicked()
					{
						self.clear_cache_requested = true;
					}
				});
			});
		});
		ui.add_space(8.0);
		ui.label(design::eyebrow(ui, "Your privacy", colors.muted));
		design::card(ui, |ui| {
			ui.label(
				"Serein does not collect telemetry or upload diagnostics. Discord retains service-side data according to its own policies.",
			);
		});
	}
}

fn account_row(ui: &mut egui::Ui, label: &str, value: &str) {
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

/// Sidebar entry in the settings modal; the selected page uses the strong surface and text.
pub(super) fn nav_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::WidgetType::SelectableLabel,
			ui.is_enabled(),
			selected,
			label,
		)
	});
	let hot = response.hovered() || response.has_focus();
	if selected {
		ui.painter().rect_filled(rect, 8, colors.selected);
		// Discord marks the open page with an accent rail at the left edge.
		ui.painter().rect_filled(
			egui::Rect::from_min_size(
				egui::pos2(rect.left(), rect.center().y - 8.0),
				egui::vec2(3.0, 16.0),
			),
			2,
			colors.accent,
		);
	} else if hot {
		ui.painter().rect_filled(rect, 8, colors.hover);
	}
	if response.has_focus() {
		ui.painter().rect_stroke(
			rect.shrink(1.0),
			8,
			egui::Stroke::new(1.0, colors.accent),
			egui::StrokeKind::Inside,
		);
	}
	ui.painter().text(
		egui::pos2(rect.left() + 12.0, rect.center().y),
		egui::Align2::LEFT_CENTER,
		label,
		egui::FontId::new(15.0, design::medium_family(ui.ctx())),
		if selected {
			colors.text_strong
		} else if hot {
			colors.text
		} else {
			colors.muted
		},
	);
	response
}

/// Discord's round close button with the "ESC" hint underneath.
pub(super) fn close_control(ui: &mut egui::Ui) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 56.0), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::WidgetType::Button,
			ui.is_enabled(),
			"Close settings (Esc)",
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
	ui.painter().text(
		egui::pos2(rect.center().x, rect.bottom() - 6.0),
		egui::Align2::CENTER_CENTER,
		"ESC",
		egui::FontId::new(11.0, design::semibold_family(ui.ctx())),
		colors.muted,
	);
	response.on_hover_text("Close settings (Esc)")
}

/// Dark, light or system cards with a miniature of each palette and a radio marker.
/// One colour-preset cell: the palette circles, the selection ring and the caption.
fn preset_swatch(
	ui: &mut egui::Ui,
	label: &str,
	swatch: &design::Palette,
	selected: bool,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(76.0, 70.0), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, label)
	});
	let painter = &ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
	if response.hovered() || response.has_focus() {
		painter.rect_filled(rect, 6, colors.hover);
	}
	let center = egui::pos2(rect.center().x, rect.top() + 24.0);
	match swatch.backdrop {
		Some([top, bottom]) => {
			painter.circle_filled(center, 20.0, bottom);
			painter.circle_filled(center - egui::vec2(5.0, 5.0), 11.0, top);
		}
		None => {
			painter.circle_filled(center, 20.0, swatch.chat);
			painter.circle_filled(center + egui::vec2(5.0, 5.0), 9.0, swatch.base);
		}
	}
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
		label,
		egui::FontId::proportional(11.0),
		if selected {
			colors.text_strong
		} else {
			colors.muted
		},
	);
	response
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
			(egui::ThemePreference::Dark, "Dark"),
			(egui::ThemePreference::Light, "Light"),
			(
				egui::ThemePreference::System,
				if width < 140.0 {
					"System"
				} else {
					"Sync with system"
				},
			),
		] {
			let selected = current == preference;
			let (rect, response) =
				ui.allocate_exact_size(egui::vec2(width, 76.0), egui::Sense::click());
			response.widget_info(|| {
				egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, label)
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
					let p = design::colors(true, variant);
					(p.sidebar.to_opaque(), p.chat.to_opaque())
				}
				egui::ThemePreference::Light => {
					let p = design::colors(false, variant);
					(p.sidebar.to_opaque(), p.chat.to_opaque())
				}
				egui::ThemePreference::System => (
					design::colors(true, variant).chat.to_opaque(),
					design::colors(false, variant).chat.to_opaque(),
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
				label,
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
