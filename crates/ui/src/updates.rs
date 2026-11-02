//! Device update preferences and host-owned status. No transport or filesystem work lives here.
use crate::{MessagingUi, design, icons};

/// One release in the update log, already reduced to plain text by the host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogEntry {
	pub version: String,
	/// `YYYY-MM-DD`, empty when the release has no publish date.
	pub date: String,
	/// Section headings ("Features", "Bug Fixes") with their bullet points.
	pub sections: Vec<(String, Vec<String>)>,
}

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
	/// Feedback for the copy button of the diagnostic log's recent lines.
	pub copied_log: Option<f64>,
	pub copied_command: Option<f64>,
	/// Ready flag the sidebar banner was dismissed at, so a later stage prompts again.
	pub banner_dismissed: Option<bool>,
	/// Recent releases on the selected channel, newest first.
	pub log: Vec<LogEntry>,
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
			copied_log: None,
			copied_command: None,
			banner_dismissed: None,
			log: Vec::new(),
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
			(
				"updates-shows-update-banner-restart-to-update",
				icons::Icon::Reload,
			)
		} else if self.updates.busy {
			(
				"updates-shows-update-banner-updating",
				icons::Icon::Download,
			)
		} else {
			(
				"updates-shows-update-banner-update-available",
				icons::Icon::Download,
			)
		};
		let label = crate::i18n::translate_if_key(label);
		let status = self.updates.status.clone();
		// Reserve the row and interact with it *before* the dismiss button below is added, so
		// that button (registered after, "updates-shows-update-banner-on-top") keeps first claim on an overlapping click.
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
			egui::Label::new(design::medium(ui, &label, 12.0).color(colors.accent))
				.truncate()
				.selectable(false),
		);
		let mut dismiss = false;
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			dismiss = icons::button(
				ui,
				icons::Icon::Close,
				18.0,
				&crate::i18n::translate("updates-update-banner-dismiss-update"),
			)
			.clicked();
		});
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
		let open = !dismiss
			&& response
				.on_hover_text(crate::i18n::translate_if_key(&status))
				.clicked();
		if dismiss {
			self.updates.banner_dismissed = Some(ready);
		}
		if open {
			self.open_update_settings();
		}
	}

	/// Gateway-recovery notice stacked into the account card, styled like the update banner but
	/// tinted with the warning colour. The whole row requests an immediate reconnect.
	pub(super) fn reconnecting_banner(&mut self, ui: &mut egui::Ui, first: bool) {
		let colors = design::palette(ui);
		let label = crate::i18n::translate("reconnecting");
		let action = crate::i18n::translate("reconnect-now");
		let (rect, response) =
			ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
		let hovered = response.hovered() || response.has_focus();
		let top = if first { 8 } else { 0 };
		ui.painter().rect_filled(
			rect,
			egui::CornerRadius {
				nw: top,
				ne: top,
				sw: 0,
				se: 0,
			},
			colors
				.warning
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
		icons::paint(ui.painter(), icons::Icon::Reload, mark, colors.warning);
		ui.add(
			egui::Label::new(design::medium(ui, &label, 12.0).color(colors.warning))
				.truncate()
				.selectable(false),
		);
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			let color = if hovered {
				colors.warning
			} else {
				colors.warning.gamma_multiply(0.75)
			};
			ui.add(
				egui::Label::new(design::medium(ui, &action, 11.0).color(color))
					.truncate()
					.selectable(false),
			);
		});
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &action)
		});
		if response.clicked() {
			self.reconnect_requested = true;
		}
	}

	/// Only locally selected categories cross the report boundary. No environment values,
	/// adapter descriptions, theme names, file paths or interface status prose are exported.
	pub fn diagnostic_environment(&self, ctx: &egui::Context) -> String {
		let backend = match self.gpu_adapter.rsplit_once('(').map(|(_, tail)| tail) {
			Some("Vulkan)") => "Vulkan",
			Some("Metal)") => "Metal",
			Some("Dx12)") => "Dx12",
			Some("Gl)") => "Gl",
			Some("BrowserWebGpu)") => "BrowserWebGpu",
			_ => "unknown",
		};
		let channel = match self.build.channel {
			design::Channel::Stable => "stable",
			design::Channel::Nightly => "nightly",
			design::Channel::Dev => "dev",
		};
		let package = if self.updates.flatpak {
			"flatpak"
		} else {
			"native"
		};
		let focused = ctx.input(|i| i.focused);
		let keyboard_focus = ctx.memory(|m| m.focused().is_some());
		let ime = self.ime_active;
		let capture = self.keybind_capture.is_some();
		format!(
			"# Serein diagnostic report\n\n- **Report schema:** 1\n- **Serein version:** {} ({channel})\n- **Operating system:** {} ({})\n- **Packaging:** {package}\n- **Graphics backend:** {backend}\n- **Graphics preference:** {}\n- **Input state:** window_focused={focused} keyboard_focus={keyboard_focus} ime_active={ime} binding_capture={capture}\n",
			self.build.version,
			std::env::consts::OS,
			std::env::consts::ARCH,
			self.gpu_preference.label(),
		)
	}

	/// Shared formatter for copied and saved reports. May run on the host's export worker.
	pub fn format_diagnostic_report(mut environment: String) -> String {
		use std::fmt::Write;
		let recording = diagnostics::level() != diagnostics::Level::Off;
		let _ = writeln!(
			environment,
			"- **Recording:** {}",
			if recording { "enabled" } else { "disabled" }
		);
		let _ = writeln!(
			environment,
			"- **Disk history:** {}",
			if diagnostics::disk_enabled() {
				"enabled"
			} else {
				"disabled"
			}
		);
		environment.push_str("\n## Recent diagnostics\n\n```text\n");
		if recording {
			environment.push_str(&diagnostics::recent(2_048));
		} else {
			environment.push_str("Recording disabled; earlier events are unavailable.\n");
		}
		environment.push_str("\n```\n");
		// The collector bounds its tail independently; the complete artifact also includes
		// environment and current-state projections, so enforce the report limit here too.
		const MAX_REPORT_BYTES: usize = 1024 * 1024;
		if environment.len() > MAX_REPORT_BYTES {
			environment.truncate(environment.floor_char_boundary(MAX_REPORT_BYTES - 32));
			environment.push_str("\nReport truncated.\n");
		}
		environment
	}

	pub fn diagnostic_info(&self, ctx: &egui::Context) -> String {
		Self::format_diagnostic_report(self.diagnostic_environment(ctx))
	}

	/// Requests the host export worker; the host copies the result and acknowledges completion.
	pub fn copy_diagnostic_info(&mut self, ctx: &egui::Context) {
		self.diagnostics_copy_requested = true;
		ctx.request_repaint();
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
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if self.updates.ready {
						ui.add_enabled_ui(!self.updates.busy, |ui| {
							if design::button(
								ui,
								&crate::i18n::translate(
									"updates-update-settings-restart-to-update",
								),
								design::ButtonKind::Primary,
							)
							.clicked()
							{
								self.updates.restart_requested = true;
							}
						});
					} else if self.updates.available && self.updates.supported {
						ui.add_enabled_ui(!self.updates.busy, |ui| {
							if design::button(
								ui,
								&crate::i18n::translate("updates-update-settings-download-update"),
								design::ButtonKind::Primary,
							)
							.clicked()
							{
								self.updates.download_requested = true;
							}
						});
					} else {
						let allowed = (!cfg!(debug_assertions) || demo) && !self.updates.busy;
						ui.add_enabled_ui(allowed, |ui| {
							if design::button(
								ui,
								&crate::i18n::translate(
									"updates-update-settings-check-for-updates",
								),
								design::ButtonKind::Outline,
							)
							.on_disabled_hover_text(crate::i18n::translate_if_key(
								if cfg!(debug_assertions) && !demo {
									"updates-update-settings-update-checks-are-disabled-in-debug-builds"
								} else {
									"updates-update-settings-finish-the-current-update-before-checking-again"
								},
							))
							.clicked()
							{
								self.updates.check_requested = true;
							}
						});
					}
					if self.updates.busy {
						ui.add(egui::Spinner::new().size(16.0));
					}
					// Reserve the action's width before laying out the status beside it.
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
							.truncate(),
						);
					});
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
					&crate::i18n::translate(
						"updates-update-settings-could-not-load-or-save-update-preferences-changes-may-not",
					),
				);
			}
		});
		self.update_log(ui);
		design::group(
			ui,
			&crate::i18n::translate("updates-update-settings-preferences"),
			|ui| {
				ui.add_enabled_ui(self.updates.supported || demo, |ui| {
					design::switch(
						ui,
						"updates-update-settings-auto-update",
						Some(
							"updates-update-settings-download-updates-in-the-background-restart-when-you-are-ready",
						),
						&mut self.updates.auto_update,
					);
				});
				design::card_divider(ui);
				design::row(
					ui,
					"updates-update-settings-release-channel",
					Some(if self.updates.nightly {
						"updates-update-settings-early-builds-with-the-newest-changes-nightly-releases-can-be"
					} else {
						"updates-update-settings-published-stable-releases-switching-channels-never-installs-an-older-ver"
					}),
					|ui| {
						egui::ComboBox::from_id_salt("update-release-channel")
							.selected_text(crate::i18n::translate_if_key(
								&(if self.updates.nightly {
									crate::i18n::translate("updates-update-settings-nightly")
								} else {
									crate::i18n::translate("updates-update-settings-production")
								}),
							))
							.width(ui.available_width().min(160.0))
							.show_ui(ui, |ui| {
								ui.selectable_value(
									&mut self.updates.nightly,
									false,
									crate::i18n::translate("updates-update-settings-production"),
								);
								ui.selectable_value(
									&mut self.updates.nightly,
									true,
									crate::i18n::translate("updates-update-settings-nightly"),
								);
							});
					},
				);
				if !demo {
					if self.updates.flatpak {
						design::hint(
							ui,
							&crate::i18n::translate(
								"updates-update-settings-flatpak-manages-updates-via-its-repository-run-flatpak-update-or",
							),
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
								"updates-update-settings-package-manager-updates",
								Some(
									"updates-update-settings-serein-was-installed-via-your-distribution-run-this-in-a",
								),
								|ui| {
									if design::button(
										ui,
										&crate::i18n::translate_if_key(if copied_cmd {
											"updates-update-settings-copied"
										} else {
											"updates-update-settings-copy-command"
										}),
										design::ButtonKind::Outline,
									)
									.clicked()
									{
										ui.ctx().copy_text(cmd.clone());
										self.updates.copied_command =
											Some(ui.input(|i| i.time) + 2.5);
										ui.ctx().request_repaint_after(
											std::time::Duration::from_secs(3),
										);
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
								&crate::i18n::translate(
									"updates-update-settings-in-app-installation-requires-a-macos-or-windows-release-package",
								),
							);
						}
					}
				}
			},
		);
		design::group(
			ui,
			&crate::i18n::translate("updates-update-settings-support-diagnostics"),
			|ui| {
				let copied = self
					.updates
					.copied_diagnostics
					.is_some_and(|until| ui.input(|i| i.time) < until);
				let mut copying = false;
				let mut saving = false;
				design::row(
					ui,
					"updates-update-settings-issue-diagnostics",
					Some(
						"updates-update-settings-copy-system-and-client-environment-details-formatted-for-github-issue",
					),
					|ui| {
						if design::button(
							ui,
							&crate::i18n::translate("updates-diagnostics-save"),
							design::ButtonKind::Outline,
						)
						.clicked()
						{
							saving = true;
						}
						if design::button(
							ui,
							&crate::i18n::translate_if_key(if copied {
								"updates-update-settings-copied"
							} else {
								"updates-diagnostics-copy"
							}),
							design::ButtonKind::Outline,
						)
						.clicked()
						{
							copying = true;
						}
					},
				);
				if copying {
					self.copy_diagnostic_info(ui.ctx());
				}
				if saving {
					self.diagnostics_save_requested = true;
				}
				if let Some(path) = &self.diagnostics_saved_path {
					// The file the owner should attach; naming it is enough, no new copy needed.
					let colors = design::palette(ui);
					ui.add_space(4.0);
					ui.add(
						egui::Label::new(egui::RichText::new(path).size(12.0).color(colors.muted))
							.wrap(),
					);
				}
				design::card_divider(ui);
				let mut recording = self.diagnostics_log != diagnostics::Level::Off;
				if design::switch(
					ui,
					"updates-diagnostics-record",
					Some("updates-diagnostics-record-description"),
					&mut recording,
				)
				.changed()
				{
					self.diagnostics_log = if recording {
						diagnostics::Level::Debug
					} else {
						diagnostics::Level::Off
					};
				}
				ui.add_enabled_ui(recording, |ui| {
					design::switch(
						ui,
						"updates-diagnostics-disk",
						Some("updates-diagnostics-disk-description"),
						&mut self.diagnostics_disk,
					);
				});
				if recording && self.diagnostics_disk {
					if design::button(
						ui,
						"updates-update-settings-open-folder",
						design::ButtonKind::Outline,
					)
					.clicked()
					{
						self.open_log_folder_requested = true;
					}
					// Local destination feedback helps recover a previous run after a crash.
					// Paths are never part of the exported report.
					if let Some(path) = &self.diagnostics_log_path {
						ui.add(egui::Label::new(egui::RichText::new(path).size(12.0)).wrap());
					}
				}
				if self.diagnostics_log_failed {
					design::hint(ui, "updates-diagnostics-disk-failed");
				}
				if self.diagnostics_export_failed {
					design::hint(ui, "updates-diagnostics-export-failed");
				}
				if design::button(ui, "updates-diagnostics-clear", design::ButtonKind::Outline)
					.clicked()
				{
					self.diagnostics_clear_requested = true;
				}
			},
		);
	}
}

impl MessagingUi {
	/// What changed between the running build and the newest release; hidden when up to date.
	fn update_log(&self, ui: &mut egui::Ui) {
		if self.updates.log.is_empty() {
			return;
		}
		let colors = design::palette(ui);
		design::group(ui, &crate::i18n::translate("updates-update-log"), |ui| {
			for (index, entry) in self.updates.log.iter().enumerate() {
				if index > 0 {
					design::card_divider(ui);
				}
				let title = format!("Serein {}", entry.version);
				egui::CollapsingHeader::new(
					design::semibold(ui, &title, 14.0).color(colors.text_strong),
				)
				.id_salt(("update-log", &entry.version))
				.default_open(index == 0)
				.show(ui, |ui| {
					if !entry.date.is_empty() {
						ui.label(
							egui::RichText::new(&entry.date)
								.size(12.0)
								.color(colors.muted),
						);
					}
					for (heading, items) in &entry.sections {
						if !heading.is_empty() {
							ui.add_space(4.0);
							ui.label(design::semibold(ui, heading, 13.0).color(colors.text_strong));
						}
						for item in items {
							ui.horizontal_top(|ui| {
								ui.label(egui::RichText::new("•").size(13.0).color(colors.muted));
								ui.add(
									egui::Label::new(egui::RichText::new(item).size(13.0)).wrap(),
								);
							});
						}
					}
				});
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
