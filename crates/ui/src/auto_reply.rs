//! Native settings for the host service. Secret input never enters plugin values.
use client_core::{
	State,
	auto_reply::{Config, MAX_TARGETS, private},
};
use zeroize::{Zeroize, Zeroizing};

pub enum Request {
	Save {
		config: Config,
		key: Zeroizing<String>,
	},
	Forget,
}
#[derive(Default)]
pub struct Form {
	pub config: Config,
	key: Zeroizing<String>,
	query: String,
	pub request: Option<Request>,
	pub status: String,
	pub busy: bool,
	pub connected: bool,
	pub running: bool,
	pub available: bool,
}
impl Form {
	pub fn clear_secret(&mut self) {
		self.key.zeroize();
	}
	pub fn show(&mut self, ui: &mut egui::Ui, state: &State) {
		ui.heading("OpenRouter auto replies");
		if !self.available {
			self.clear_secret();
			ui.label(
				"Auto replies are unavailable in this session. Enable the plugin and sign in before starting.",
			);
			return;
		}
		ui.label("New messages in the DMs you select are sent to OpenRouter and its model provider. Replies are sent from your account with an [Auto reply] label. Provider charges may apply.");
		ui.horizontal_wrapped(|ui| {
			ui.strong(if self.running {
				"Auto replies on"
			} else {
				"Paused"
			});
			if ui.button("Pause now").clicked() {
				self.running = false;
				self.status = "Paused. No new replies will be sent.".into();
			}
		});
		ui.label("One new text message per request; no history, attachments or other conversations. At most one request at a time, one per conversation per minute and 100 per session. Start again after signing in or restarting.");
		ui.add_enabled_ui(!self.running && !self.busy, |ui| {
			key_input(ui, &mut self.key, self.connected);
			ui.small("The key and these settings are saved only in your OS credential store for this Discord account. The plugin cannot read them.");
			let label = ui.label("Model ID");
			ui.add(egui::TextEdit::singleline(&mut self.config.model).char_limit(128).desired_width(f32::INFINITY).hint_text("provider/model" )).labelled_by(label.id);
			let label = ui.label("Reply instructions (up to 1,024 bytes)");
			ui.add(egui::TextEdit::multiline(&mut self.config.instructions).char_limit(1024).desired_rows(3).desired_width(f32::INFINITY)).labelled_by(label.id);
			ui.separator();
			ui.strong(format!("People and group DMs ({}/{MAX_TARGETS})", self.config.channels.len()));
			let label = ui.label("Find a private conversation");
			ui.add(egui::TextEdit::singleline(&mut self.query).char_limit(128).desired_width(f32::INFINITY)).labelled_by(label.id);
			let query = self.query.to_lowercase();
			let mut shown = 0;
			for channel in state.channels.iter().filter(|c| private(c)) {
				let label: String = if channel.name.is_empty() {
					channel.recipients.iter().map(|u| u.name.as_str()).take(10).collect::<Vec<_>>().join(", ")
				} else { channel.name.clone() }.chars().take(128).collect();
				let mut selected = self.config.channels.contains(&channel.id);
				if !selected && (!label.to_lowercase().contains(&query) || shown >= 50) { continue; }
				shown += 1;
				ui.push_id(channel.id.0, |ui| {
					let group = if channel.kind == 3 { "Group DM" } else { "DM" };
					if ui.add_enabled(selected || self.config.channels.len() < MAX_TARGETS, egui::Checkbox::new(&mut selected, format!("{label} · {group} · {}", channel.id.0))).changed() {
						self.config.channels.retain(|id| *id != channel.id);
						if selected { self.config.channels.push(channel.id); }
					}
				});
			}
			if shown == 0 { ui.label("No matching loaded DMs. Open a DM in Serein, then return here."); }
			let missing: Vec<_> = self.config.channels.iter().copied().filter(|id| !state.channel(*id).is_some_and(private)).collect();
			for id in missing {
				if ui.button(format!("Remove unavailable conversation {}", id.0)).clicked() {
					self.config.channels.retain(|saved| *saved != id);
				}
			}
			if ui.add_enabled(client_core::auto_reply::available(state), egui::Button::new("Save and start auto replies")).clicked() {
				match self.config.validate() {
					Ok(()) if self.connected || !self.key.is_empty() => {
						self.request = Some(Request::Save { config: self.config.clone(), key: std::mem::take(&mut self.key) });
						self.running = true;
					}
					Ok(()) => self.status = "Enter an OpenRouter API key first.".into(),
					Err(error) => self.status = error.into(),
				}
			}
		});
		if ui
			.add_enabled(
				!self.busy,
				egui::Button::new("Forget saved key and settings"),
			)
			.clicked()
		{
			self.running = false;
			self.clear_secret();
			self.request = Some(Request::Forget);
		}
		if self.busy {
			ui.spinner();
		}
		if !self.status.is_empty() {
			ui.label(&self.status);
		}
	}
}

fn key_input(ui: &mut egui::Ui, key: &mut Zeroizing<String>, connected: bool) -> egui::Id {
	let label = ui.label("OpenRouter API key");
	let mut edit = egui::TextEdit::singleline(&mut **key)
		.password(true)
		.char_limit(256)
		.id_salt("host-openrouter-key")
		.hint_text(if connected {
			"Saved key (leave blank to keep)"
		} else {
			"Paste API key"
		})
		.desired_width(f32::INFINITY)
		.show(ui);
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
	fn inactive_service_does_not_offer_start_or_retain_draft_key() {
		let ctx = egui::Context::default();
		let mut form = Form {
			key: Zeroizing::new("synthetic-key".into()),
			..Default::default()
		};
		let state = test_support::demo_state();
		let mut output = ctx.run_ui(egui::RawInput::default(), |ui| form.show(ui, &state));
		output.textures_delta.clear();
		let text: String = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::epaint::Shape::Text(text) => Some(text.galley.text()),
				_ => None,
			})
			.collect();
		assert!(text.contains("unavailable") && !text.contains("Save and start"));
		assert!(form.key.is_empty() && form.request.is_none());
	}
	#[test]
	fn discarded_results_erase_unsaved_keys_immediately() {
		use crate::extensions_ui::{ExtensionContext, ExtensionUi};
		for discard in 0..4 {
			let mut state = test_support::demo_state();
			let mut shop = ExtensionUi::default();
			if discard != 2 {
				shop.present_output(
					"synthetic".into(),
					extensions::Invocation::default(),
					ExtensionContext::panel(&state),
					extensions::Output::default(),
					&state,
				);
			}
			shop.auto_reply.key = Zeroizing::new("synthetic-key".into());
			match discard {
				0 => shop.reset_runtime(),
				1 => shop.remove_runtime("synthetic"),
				_ => {
					if discard == 3 {
						state.generation += 1;
					}
					shop.show_result(
						&egui::Context::default(),
						&mut state,
						&mut Vec::new(),
						false,
						&mut crate::avatars::Avatars::default(),
						("", false),
					);
				}
			}
			assert!(shop.auto_reply.key.is_empty());
		}
	}
	#[test]
	fn secret_widget_clears_undo_and_works_in_narrow_light_and_dark_forms() {
		for light in [false, true] {
			let ctx = egui::Context::default();
			if light {
				ctx.set_visuals(egui::Visuals::light());
			}
			let mut form = Form {
				key: Zeroizing::new("synthetic-key".into()),
				available: true,
				..Default::default()
			};
			let state = test_support::demo_state();

			let mut key_id = None;
			let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
				key_id = Some(key_input(ui, &mut form.key, false));
			});
			output.textures_delta.clear();
			form.clear_secret();
			let state_edit = egui::text_edit::TextEditState::load(&ctx, key_id.unwrap()).unwrap();
			let current = (
				egui::text::CCursorRange::one(egui::text::CCursor::new(0)),
				String::new(),
			);
			assert!(!state_edit.undoer().has_undo(&current));
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(320.0, 600.0),
					)),
					..Default::default()
				},
				|ui| {
					form.show(ui, &state);
				},
			);
			output.textures_delta.clear();
			assert!(form.key.is_empty() && form.request.is_none() && !form.running);
		}
	}
}
