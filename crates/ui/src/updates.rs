//! Device update preferences and host-owned status. No transport or filesystem work lives here.
use crate::{MessagingUi, design, icons};

pub struct Updates {
	pub auto_update: bool,
	pub nightly: bool,
	pub status: String,
	pub busy: bool,
	pub available: bool,
	pub ready: bool,
	pub supported: bool,
	pub flatpak: bool,
	pub linux_update_cmd: Option<String>,
	pub progress: Option<f32>,
	pub check_requested: bool,
	pub download_requested: bool,
	pub restart_requested: bool,
	pub copied_diagnostics: Option<f64>,
	pub copied_command: Option<f64>,
	/// Ready flag the sidebar banner was dismissed at, so a later stage prompts again.
	pub banner_dismissed: Option<bool>,
}
impl Default for Updates {
	fn default() -> Self {
		Self {
			auto_update: false,
			nightly: true,
			status: "Updates have not been checked yet.".into(),
			busy: false,
			available: false,
			ready: false,
			supported: false,
			flatpak: false,
			linux_update_cmd: None,
			progress: None,
			check_requested: false,
			download_requested: false,
			restart_requested: false,
			copied_diagnostics: None,
			copied_command: None,
			banner_dismissed: None,
		}
	}
}
impl MessagingUi {
	/// Whether the account card grows an update row, which it does only while Serein's own
	/// title bar is hidden: the title-bar button is the only other place the prompt appears.
	pub(super) fn shows_update_banner(&mut self) -> bool {
		if self.shows_title_bar() {
			return false;
		}
		if !self.updates.available && !self.updates.ready {
			self.updates.banner_dismissed = None;
			return false;
		}
		self.updates.banner_dismissed != Some(self.updates.ready)
	}

	/// Update prompt stacked into the account card, the way a call grows its own section.
	/// Tinted with the accent colour (rather than plain link-coloured text on the card's flat
	/// background) so it reads as a distinct, tappable banner instead of a stray line of text.
	pub(super) fn update_banner(&mut self, ui: &mut egui::Ui) {
		let ready = self.updates.ready;
		let colors = design::palette(ui);
		let (label, icon) = if ready {
			("Restart to update", icons::Icon::Reload)
		} else if self.updates.busy {
			("Updating…", icons::Icon::Download)
		} else {
			("Update available", icons::Icon::Download)
		};
		let status = self.updates.status.clone();
		// Reserve the row and interact with it *before* the dismiss button below is added, so
		// that button (registered after, "on top") keeps first claim on an overlapping click.
		let (rect, response) =
			ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
		let hovered = response.hovered() || response.has_focus();
		ui.painter().rect_filled(
			rect,
			egui::CornerRadius {
				nw: 8,
				ne: 8,
				sw: 0,
				se: 0,
			},
			colors
				.accent
				.gamma_multiply(if hovered { 0.18 } else { 0.12 }),
		);
		let mut content = ui.new_child(
			egui::UiBuilder::new()
				.max_rect(rect.shrink2(egui::vec2(10.0, 0.0)))
				.layout(egui::Layout::left_to_right(egui::Align::Center)),
		);
		let ui = &mut content;
		ui.spacing_mut().item_spacing.x = 8.0;
		let (mark, _) = ui.allocate_exact_size(egui::vec2(15.0, 15.0), egui::Sense::hover());
		icons::paint(ui.painter(), icon, mark, colors.accent);
		ui.add(
			egui::Label::new(design::medium(ui, label, 12.0).color(colors.accent))
				.truncate()
				.selectable(false),
		);
		let mut dismiss = false;
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			dismiss = icons::button(ui, icons::Icon::Close, 18.0, "Dismiss update").clicked();
		});
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
		});
		let open = !dismiss && response.on_hover_text(status).clicked();
		if dismiss {
			self.updates.banner_dismissed = Some(ready);
		}
		if open {
			self.open_update_settings();
		}
	}

	/// Formats system and client environment details for GitHub issue reports.
	pub fn diagnostic_info(&self, ctx: &egui::Context) -> String {
		let os = std::env::consts::OS;
		let arch = std::env::consts::ARCH;
		let channel = match self.build.channel {
			design::Channel::Stable => "Stable",
			design::Channel::Nightly => "Nightly",
			design::Channel::Dev => "Dev",
		};
		let theme_mode = match ctx.theme() {
			egui::Theme::Dark => "Dark",
			egui::Theme::Light => "Light",
		};
		let theme_variant = design::variant().label();
		let scale = ctx.pixels_per_point();
		let update_channel = if self.updates.nightly {
			"Nightly"
		} else {
			"Production"
		};

		#[cfg(target_os = "linux")]
		let session_type = std::env::var("XDG_SESSION_TYPE")
			.map(|s| format!(" ({s})"))
			.unwrap_or_default();
		#[cfg(not(target_os = "linux"))]
		let session_type = "";

		#[cfg(target_os = "linux")]
		let package_type = if self.updates.flatpak {
			"\n- **Packaging:** Flatpak".to_owned()
		} else if let Some(cmd) = &self.updates.linux_update_cmd {
			let mgr = if cmd.contains("dnf") {
				"DNF (RPM)"
			} else if cmd.contains("apt") {
				"APT (DEB)"
			} else if cmd.contains("pacman") {
				"Pacman (Arch)"
			} else if cmd.contains("zypper") {
				"Zypper (RPM)"
			} else {
				"Native Package"
			};
			format!("\n- **Packaging:** {mgr}")
		} else {
			"\n- **Packaging:** Native / AppImage".to_owned()
		};
		#[cfg(not(target_os = "linux"))]
		let package_type = "";

		// The adapter and the preference that chose it are what graphics reports hinge on.
		let graphics = if self.gpu_adapter.is_empty() {
			String::new()
		} else {
			format!(
				"\n- **Graphics:** {} ({} preference)",
				self.gpu_adapter,
				self.gpu_preference.label()
			)
		};

		format!(
			"- **Serein Version:** {} ({channel})\n- **Operating System:** {os} ({arch}){session_type}{package_type}{graphics}\n- **Display Scale:** {scale:.2}\n- **Theme:** {theme_mode} ({theme_variant})\n- **Update Channel:** {update_channel}\n- **Auto Update:** {}",
			self.build.version,
			if self.updates.auto_update {
				"Enabled"
			} else {
				"Disabled"
			}
		)
	}

	/// Copies formatted diagnostics to clipboard and sets a temporary feedback countdown.
	pub fn copy_diagnostic_info(&mut self, ctx: &egui::Context) {
		let info = self.diagnostic_info(ctx);
		ctx.copy_text(info);
		self.updates.copied_diagnostics = Some(ctx.input(|i| i.time) + 2.5);
		ctx.request_repaint_after(std::time::Duration::from_secs(3));
	}

	/// Update controls for the signed-out header, rendered inside its menu popup so the
	/// screen never grows a second, movable window.
	pub fn updates_menu(&mut self, ui: &mut egui::Ui, demo: bool) {
		ui.set_min_width(340.0);
		ui.set_max_width(340.0);
		self.update_settings(ui, demo);
	}

	pub(super) fn update_settings(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = design::palette(ui);
		design::card(ui, |ui| {
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 14.0;
				let (badge, _) =
					ui.allocate_exact_size(egui::Vec2::splat(44.0), egui::Sense::hover());
				ui.painter()
					.rect_filled(badge, 12, colors.accent.gamma_multiply(0.16));
				crate::icons::paint(
					ui.painter(),
					crate::icons::Icon::Serein,
					badge.shrink(10.0),
					colors.accent,
				);
				ui.vertical(|ui| {
					ui.spacing_mut().item_spacing.y = 2.0;
					ui.label(
						design::semibold(ui, format!("Serein {}", self.build.version), 17.0)
							.color(colors.text_strong),
					);
					ui.add(
						egui::Label::new(
							egui::RichText::new(&self.updates.status)
								.size(13.0)
								.color(colors.muted),
						)
						.wrap(),
					);
				});
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if self.updates.ready {
						ui.add_enabled_ui(!self.updates.busy, |ui| {
							if design::button(ui, "Restart to update", design::ButtonKind::Primary)
								.clicked()
							{
								self.updates.restart_requested = true;
							}
						});
					} else if self.updates.available && self.updates.supported {
						ui.add_enabled_ui(!self.updates.busy, |ui| {
							if design::button(ui, "Download update", design::ButtonKind::Primary)
								.clicked()
							{
								self.updates.download_requested = true;
							}
						});
					} else {
						let allowed = (!cfg!(debug_assertions) || demo) && !self.updates.busy;
						ui.add_enabled_ui(allowed, |ui| {
							if design::button(ui, "Check for updates", design::ButtonKind::Outline)
								.on_disabled_hover_text(if cfg!(debug_assertions) && !demo {
									"Update checks are disabled in debug builds."
								} else {
									"Finish the current update before checking again."
								})
								.clicked()
							{
								self.updates.check_requested = true;
							}
						});
					}
					if self.updates.busy {
						ui.add(egui::Spinner::new().size(16.0));
					}
				});
			});
			if let Some(progress) = self.updates.progress {
				ui.add_space(10.0);
				ui.add(
					egui::ProgressBar::new(progress)
						.desired_height(6.0)
						.corner_radius(3)
						.fill(colors.accent),
				);
			}
			if self.updates_save_failed && !demo {
				ui.add_space(10.0);
				design::notice(
					ui,
					design::Level::Warning,
					"Could not load or save update preferences. Changes may not survive restart.",
				);
			}
		});
		design::group(ui, "Preferences", |ui| {
			ui.add_enabled_ui(self.updates.supported || demo, |ui| {
				design::switch(
					ui,
					"Auto update",
					Some("Download updates in the background. Restart when you are ready. Serein still checks at startup and periodically when this is off."),
					&mut self.updates.auto_update,
				);
			});
			design::card_divider(ui);
			design::row(
				ui,
				"Release channel",
				Some(if self.updates.nightly {
					"Early builds with the newest changes. Nightly releases can be less reliable."
				} else {
					"Published stable releases. Switching channels never installs an older version."
				}),
				|ui| {
					egui::ComboBox::from_id_salt("update-release-channel")
						.selected_text(if self.updates.nightly {
							"Nightly"
						} else {
							"Production"
						})
						.width(ui.available_width().min(160.0))
						.show_ui(ui, |ui| {
							ui.selectable_value(&mut self.updates.nightly, false, "Production");
							ui.selectable_value(&mut self.updates.nightly, true, "Nightly");
						});
				},
			);
			if !demo {
				if self.updates.flatpak {
					design::hint(
						ui,
						"Flatpak manages updates via its repository. Run `flatpak update` or use GNOME Software / KDE Discover to install new releases.",
					);
				} else if !self.updates.supported {
					if let Some(cmd) = &self.updates.linux_update_cmd {
						design::card_divider(ui);
						let copied_cmd = self
							.updates
							.copied_command
							.is_some_and(|until| ui.input(|i| i.time) < until);
						let cmd = cmd.clone();
						design::row(
							ui,
							"Package manager updates",
							Some(
								"Serein was installed via your distribution. Run this in a terminal to update.",
							),
							|ui| {
								if design::button(
									ui,
									if copied_cmd { "Copied" } else { "Copy command" },
									design::ButtonKind::Outline,
								)
								.clicked()
								{
									ui.ctx().copy_text(cmd.clone());
									self.updates.copied_command = Some(ui.input(|i| i.time) + 2.5);
									ui.ctx()
										.request_repaint_after(std::time::Duration::from_secs(3));
								}
							},
						);
						ui.add_space(6.0);
						egui::Frame::new()
							.fill(colors.base)
							.corner_radius(6)
							.inner_margin(egui::Margin::symmetric(10, 6))
							.show(ui, |ui| {
								ui.set_width(ui.available_width());
								ui.monospace(&cmd);
							});
					} else {
						design::hint(
							ui,
							"In-app installation requires a macOS or Windows release package, or a Linux x86-64 AppImage. Other Linux installations use their package manager.",
						);
					}
				}
			}
		});
		design::group(ui, "Support & diagnostics", |ui| {
			let copied = self
				.updates
				.copied_diagnostics
				.is_some_and(|until| ui.input(|i| i.time) < until);
			if design::row(
				ui,
				"Issue diagnostics",
				Some(
					"Copy system and client environment details formatted for GitHub issue reports.",
				),
				|ui| {
					design::button(
						ui,
						if copied { "Copied" } else { "Copy" },
						design::ButtonKind::Outline,
					)
				},
			)
			.clicked()
			{
				self.copy_diagnostic_info(ui.ctx());
			}
		});
	}
}

#[cfg(test)]
mod tests {
	use crate::{MessagingUi, State};

	/// Rectangles of the banner's text and icon shapes, in paint order.
	fn scan(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
	) -> (Vec<(String, egui::Rect)>, Vec<egui::Rect>) {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1200.0, 760.0),
				)),
				focused: true,
				..Default::default()
			},
			|ui| {
				view.show(ui, state);
			},
		);
		let mut text = Vec::new();
		let mut images = Vec::new();
		fn walk(
			shape: &egui::Shape,
			text: &mut Vec<(String, egui::Rect)>,
			images: &mut Vec<egui::Rect>,
		) {
			match shape {
				egui::Shape::Text(shape) => text.push((
					shape.galley.job.text.clone(),
					egui::Rect::from_min_size(shape.pos, shape.galley.size()),
				)),
				egui::Shape::Mesh(mesh) => images.push(mesh.calc_bounds()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						walk(shape, text, images);
					}
				}
				_ => {}
			}
		}
		for shape in &output.shapes {
			walk(&shape.shape, &mut text, &mut images);
		}
		output.drop_without_applying_deltas();
		(text, images)
	}

	fn banner(entries: &[(String, egui::Rect)]) -> Option<egui::Rect> {
		entries
			.iter()
			.find(|(label, rect)| label == "Update available" && rect.top() > 100.0)
			.map(|(_, rect)| *rect)
	}

	#[test]
	fn hidden_title_bar_moves_the_update_prompt_above_the_account_card() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		let mut view = MessagingUi::default();
		view.updates.available = true;
		scan(&ctx, &mut view, &mut state);
		// With Serein's own title bar the prompt stays up there, not in the sidebar.
		#[cfg(not(target_os = "linux"))]
		assert!(banner(&scan(&ctx, &mut view, &mut state).0).is_none());
		view.hide_title_bar = true;
		let (text, _) = scan(&ctx, &mut view, &mut state);
		let prompt = banner(&text).expect("sidebar update prompt");
		let card = text
			.iter()
			.find(|(label, _)| label == "Your account" || label == "Kestrel")
			.map(|(_, rect)| *rect);
		if let Some(card) = card {
			assert!(prompt.bottom() < card.top(), "{prompt:?} {card:?}");
		}
		assert!(prompt.left() < 400.0, "{prompt:?}");
	}

	#[test]
	fn dismissing_the_banner_hides_it_until_the_update_is_ready() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		let mut view = MessagingUi {
			hide_title_bar: true,
			..Default::default()
		};
		view.updates.available = true;
		scan(&ctx, &mut view, &mut state);
		let (text, images) = scan(&ctx, &mut view, &mut state);
		let prompt = banner(&text).expect("sidebar update prompt");
		let close = images
			.iter()
			.filter(|rect| {
				rect.width() < 24.0
					&& rect.center().x < 400.0
					&& (rect.center().y - prompt.center().y).abs() < 14.0
			})
			.max_by(|a, b| a.center().x.total_cmp(&b.center().x))
			.copied()
			.expect("dismiss button");
		assert!(close.center().x > prompt.right(), "{close:?} {prompt:?}");
		let click = |ctx: &egui::Context, view: &mut MessagingUi, state: &mut State, pos| {
			for pressed in [true, false] {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(1200.0, 760.0),
						)),
						focused: true,
						events: vec![
							egui::Event::PointerMoved(pos),
							egui::Event::PointerButton {
								pos,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
						..Default::default()
					},
					|ui| {
						view.show(ui, state);
					},
				);
				output.drop_without_applying_deltas();
			}
		};
		click(&ctx, &mut view, &mut state, close.center());
		assert!(banner(&scan(&ctx, &mut view, &mut state).0).is_none());
		assert!(!view.settings.open, "dismissing must not open settings");
		// A finished download is a new prompt, so it speaks up again.
		view.updates.ready = true;
		let (text, _) = scan(&ctx, &mut view, &mut state);
		assert!(
			text.iter()
				.any(|(label, rect)| label == "Restart to update" && rect.top() > 100.0)
		);
	}
}
