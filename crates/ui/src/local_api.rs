//! Settings group and desktop-facing state for the opt-in local API (`docs/local-api.md`).
//! The desktop owns the listener and the access key; this group only shows and requests.
use crate::{
	MessagingUi, design,
	i18n::{translate, translate_args},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LocalApiState {
	#[default]
	Off,
	Starting,
	Listening,
	/// The address could not be bound; the desktop keeps retrying.
	Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LocalApiKey {
	#[default]
	Pending,
	/// Kept in the OS credential store.
	Saved,
	/// No credential store took it, so it lasts until Serein closes.
	SessionOnly,
}

/// What the desktop's listener reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalApiStatus {
	pub state: LocalApiState,
	/// The pipe or socket path apps open.
	pub endpoint: String,
	/// The name each connected app gave itself; empty when it gave none.
	pub clients: Vec<String>,
	pub key: LocalApiKey,
}

/// A key action for the desktop, which holds the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalApiRequest {
	CopyKey,
	ResetKey,
}

impl MessagingUi {
	/// The Data & Privacy group: both opt-ins, then the connection details while it is on.
	pub(crate) fn local_api_settings(&mut self, ui: &mut egui::Ui) {
		design::group(ui, "settings-local-api", |ui| {
			design::switch(
				ui,
				"settings-local-api-enable",
				Some("settings-local-api-enable-description"),
				&mut self.local_api,
			);
			design::card_divider(ui);
			ui.add_enabled_ui(self.local_api, |ui| {
				design::switch(
					ui,
					"settings-local-api-control",
					Some("settings-local-api-control-description"),
					&mut self.local_api_control,
				);
			});
			design::card_divider(ui);
			if self.local_api {
				self.local_api_connection(ui);
				design::card_divider(ui);
			}
			design::hint(ui, "settings-local-api-security-detail");
		});
	}

	fn local_api_connection(&mut self, ui: &mut egui::Ui) {
		let now = ui.input(|input| input.time);
		let copied = self.local_api_copied.filter(|(_, until)| *until > now);
		let mut copy = None;
		let status = &self.local_api_status;
		let (title, detail) = match status.state {
			LocalApiState::Off | LocalApiState::Starting => {
				("settings-local-api-starting", String::new())
			}
			LocalApiState::Unavailable => (
				"settings-local-api-unavailable",
				translate("settings-local-api-unavailable-detail"),
			),
			LocalApiState::Listening if status.clients.is_empty() => (
				"settings-local-api-listening",
				translate("settings-local-api-no-apps"),
			),
			LocalApiState::Listening => {
				let unnamed = translate("settings-local-api-unnamed-app");
				let apps = status
					.clients
					.iter()
					.map(|name| {
						if name.is_empty() {
							unnamed.as_str()
						} else {
							name.as_str()
						}
					})
					.collect::<Vec<_>>()
					.join(", ");
				(
					"settings-local-api-listening",
					translate_args("settings-local-api-connected", &[("apps", &apps)]),
				)
			}
		};
		let copy_label = |key: bool, label: &str| {
			translate(if copied.is_some_and(|(of_key, _)| of_key == key) {
				"updates-update-settings-copied"
			} else {
				label
			})
		};
		design::row(
			ui,
			title,
			(!detail.is_empty()).then_some(detail.as_str()),
			|_| {},
		);
		design::card_divider(ui);
		design::row(
			ui,
			"settings-local-api-endpoint",
			Some(&status.endpoint),
			|ui| {
				let label = copy_label(false, "updates-update-settings-copy");
				if design::button(ui, &label, design::ButtonKind::Outline).clicked() {
					ui.ctx().copy_text(status.endpoint.clone());
					copy = Some(false);
				}
			},
		);
		design::card_divider(ui);
		design::row(
			ui,
			"settings-local-api-key",
			Some(if status.key == LocalApiKey::SessionOnly {
				"settings-local-api-key-session"
			} else {
				"settings-local-api-key-description"
			}),
			|ui| {
				ui.add_enabled_ui(status.key != LocalApiKey::Pending, |ui| {
					let label = copy_label(true, "settings-local-api-copy-key");
					if design::button(ui, &label, design::ButtonKind::Outline).clicked() {
						self.local_api_request = Some(LocalApiRequest::CopyKey);
						copy = Some(true);
					}
					let label = translate("settings-local-api-reset-key");
					if design::button(ui, &label, design::ButtonKind::Outline).clicked() {
						self.local_api_request = Some(LocalApiRequest::ResetKey);
					}
				});
			},
		);
		if let Some(key) = copy {
			self.local_api_copied = Some((key, now + 2.5));
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_secs(3));
		}
	}
}
