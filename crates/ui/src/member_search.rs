use client_core::{Command, State};
use model::Id;

#[derive(Default)]
pub struct Search {
	pub text: String,
	key: Option<(u64, Id, String)>,
	changed: f64,
	sent: bool,
}
impl Search {
	pub fn sync(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		channel: Id,
		slot: usize,
		commands: &mut Vec<Command>,
	) {
		let now = ctx.input(|input| input.time);
		let text = self.text.trim();
		let key = (state.generation, channel, text.to_owned());
		if self.key.as_ref() != Some(&key) {
			self.key = Some(key);
			self.changed = now;
			self.sent = false;
			state.member_search[slot] = Default::default();
		}
		if self.sent && state.gateway_connected && state.member_search[slot].request.is_none() {
			self.sent = false;
		}
		if text.is_empty() || self.sent || state.channel(channel).and_then(|c| c.guild).is_none() {
			return;
		}
		if now - self.changed >= 0.35 {
			if let Some(command) = state.search_members(channel, text, slot) {
				commands.push(command);
				self.sent = true;
			}
		} else {
			ctx.request_repaint_after(std::time::Duration::from_millis(350));
		}
	}
}
