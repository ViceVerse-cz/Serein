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
	pub progress: Option<f32>,
	pub check_requested: bool,
	pub download_requested: bool,
	pub restart_requested: bool,
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
			progress: None,
			check_requested: false,
			download_requested: false,
			restart_requested: false,
		}
	}
}
impl MessagingUi {
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
		} else if !self.updates.supported {
			ui.weak("In-app installation requires a supported macOS or Windows release package. Source builds and Linux installations must be updated manually.");
		}
	}
}
