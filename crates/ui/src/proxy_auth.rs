//! Host-owned proxy credentials; never added to plugin form values.
use zeroize::{Zeroize, Zeroizing};

pub struct Request {
	pub endpoint: String,
	pub credentials: Option<(Zeroizing<String>, Zeroizing<String>)>,
}
#[derive(Default)]
pub struct Form {
	username: Zeroizing<String>,
	password: Zeroizing<String>,
	pub request: Option<Request>,
	pub status: String,
	pub busy: bool,
}
impl Form {
	pub fn clear_draft(&mut self) {
		self.username.zeroize();
		self.password.zeroize();
	}
	pub fn show(&mut self, ui: &mut egui::Ui, endpoint: &str, manual: bool) {
		ui.collapsing("Proxy authentication", |ui| {
            ui.label("Credentials stay in Serein's OS credential store and are never shared with the plugin. Apply the proxy URL before saving credentials.");
            ui.add_enabled_ui(manual && !self.busy, |ui| {
                credential_input(ui, "Username", &mut self.username, false, 256);
                credential_input(ui, "Password", &mut self.password, true, 1024);
                ui.horizontal(|ui| {
                    if ui.button("Save credentials").clicked() {
                        self.request = Some(Request { endpoint: endpoint.into(), credentials: Some((std::mem::take(&mut self.username), std::mem::take(&mut self.password))) });
                    }
                    if ui.button("Remove saved credentials").clicked() {
                        self.clear_draft();
                        self.request = Some(Request { endpoint: endpoint.into(), credentials: None });
                    }
                });
            });
            if !manual { ui.label("Choose URL mode to configure proxy authentication."); }
            if !self.status.is_empty() { ui.label(&self.status); }
        });
	}
}

fn credential_input(
	ui: &mut egui::Ui,
	label: &str,
	value: &mut Zeroizing<String>,
	password: bool,
	limit: usize,
) -> egui::Id {
	let label = ui.label(label);
	let mut edit = egui::TextEdit::singleline(&mut **value)
		.id_salt(("host-proxy-credential", password))
		.password(password)
		.char_limit(limit)
		.show(ui);
	// Password masking still records raw text in egui's persistent undo history.
	edit.state.clear_undoer();
	edit.state.store(ui.ctx(), edit.response.id);
	let id = edit.response.id;
	edit.response.response.labelled_by(label.id);
	id
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn clearing_password_cannot_restore_it_from_widget_undo_history() {
		let ctx = egui::Context::default();
		let mut password = Zeroizing::new("synthetic-password".into());
		let mut id = None;
		let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
			id = Some(credential_input(ui, "Password", &mut password, true, 1024));
		});
		output.textures_delta.clear();
		password.zeroize();
		let state = egui::text_edit::TextEditState::load(&ctx, id.unwrap()).unwrap();
		let current = (
			egui::text::CCursorRange::one(egui::text::CCursor::new(0)),
			String::new(),
		);
		assert!(!state.undoer().has_undo(&current));
		assert!(password.is_empty());
	}
}
