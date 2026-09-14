//! Device update preferences and host-owned status. No transport or filesystem work lives here.
use crate::{MessagingUi, design};

pub struct Updates {
	pub auto_update: bool,
	pub nightly: bool,
	pub status: String,
	pub busy: bool,
	pub available: bool,
	pub ready: bool,
	pub supported: bool,
	pub flatpak: bool,
	pub progress: Option<f32>,
	pub check_requested: bool,
	pub download_requested: bool,
	pub restart_requested: bool,
	pub copied_diagnostics: Option<f64>,
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
			progress: None,
			check_requested: false,
			download_requested: false,
			restart_requested: false,
			copied_diagnostics: None,
		}
	}
}
impl MessagingUi {
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
			"\n- **Packaging:** Flatpak"
		} else {
			"\n- **Packaging:** Native / AppImage"
		};
		#[cfg(not(target_os = "linux"))]
		let package_type = "";

		format!(
			"- **Serein Version:** {} ({channel})\n- **Operating System:** {os} ({arch}){session_type}{package_type}\n- **Display Scale:** {scale:.2}\n- **Theme:** {theme_mode} ({theme_variant})\n- **Update Channel:** {update_channel}\n- **Auto Update:** {}",
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
		ui.label(design::semibold(
			ui,
			format!("Serein {}", self.build.version),
			18.0,
		));
		ui.label(&self.updates.status);
		if self.updates_save_failed && !demo {
			ui.colored_label(
				colors.danger,
				"Could not load or save update preferences. Changes may not survive restart.",
			);
		}
		if let Some(progress) = self.updates.progress {
			ui.add(egui::ProgressBar::new(progress).show_percentage());
		}
		ui.horizontal_wrapped(|ui| {
			if ui
				.add_enabled(
					!self.updates.busy && !self.updates.ready,
					egui::Button::new("Check for updates"),
				)
				.on_disabled_hover_text("Finish the current update before checking again.")
				.clicked()
			{
				self.updates.check_requested = true;
			}
			if self.updates.ready {
				if ui
					.add_enabled(!self.updates.busy, egui::Button::new("Restart to update"))
					.clicked()
				{
					self.updates.restart_requested = true;
				}
			} else if self.updates.available
				&& self.updates.supported
				&& ui
					.add_enabled(!self.updates.busy, egui::Button::new("Download update"))
					.clicked()
			{
				self.updates.download_requested = true;
			}
		});
		ui.add_space(12.0);
		ui.separator();
		ui.add_space(12.0);
		ui.add_enabled_ui(self.updates.supported || demo, |ui| {
			design::switch(
				ui,
				"Auto update",
				Some("Download updates in the background. Restart when you are ready."),
				&mut self.updates.auto_update,
			);
		});
		ui.weak("Serein checks for updates at startup and periodically, even when auto update is off. Available updates appear in the title bar.");
		ui.add_space(12.0);
		ui.label(design::eyebrow(ui, "Release channel", colors.muted));
		egui::ComboBox::from_id_salt("update-release-channel")
			.selected_text(if self.updates.nightly {
				"Nightly"
			} else {
				"Production"
			})
			.show_ui(ui, |ui| {
				ui.selectable_value(&mut self.updates.nightly, false, "Production");
				ui.selectable_value(&mut self.updates.nightly, true, "Nightly");
			});
		ui.weak(if self.updates.nightly {
			"Early builds with the newest changes. Nightly releases can be less reliable."
		} else {
			"Published stable releases. Switching channels never installs an older version."
		});
		if demo {
			ui.add_space(12.0);
			ui.weak("Offline preview. Update actions are simulated and preferences are not saved.");
		} else if self.updates.flatpak {
			ui.add_space(12.0);
			ui.weak("Flatpak manages updates via its repository. Run `flatpak update` or use GNOME Software / KDE Discover to install new releases.");
		} else if !self.updates.supported {
			ui.add_space(12.0);
			ui.weak("In-app installation requires a macOS or Windows release package, or a Linux x86-64 AppImage. Other Linux installations use their package manager.");
		}
		ui.add_space(16.0);
		ui.separator();
		ui.add_space(12.0);
		ui.label(design::eyebrow(ui, "Support & Diagnostics", colors.muted));
		ui.weak("Copy system and client environment details formatted for GitHub issue reports.");
		ui.add_space(6.0);
		let copied = self
			.updates
			.copied_diagnostics
			.is_some_and(|until| ui.input(|i| i.time) < until);
		let button_text = if copied {
			"✓ Copied to clipboard!"
		} else {
			"Copy issue diagnostics"
		};
		if ui
			.button(button_text)
			.on_hover_text("Copy environment information formatted for GitHub issues")
			.clicked()
		{
			self.copy_diagnostic_info(ui.ctx());
		}
	}
}
