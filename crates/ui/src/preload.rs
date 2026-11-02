//! Rows and people the pointer rests on or presses this frame, handed to the core's intent
//! preloading.
use client_core::{Command, State};
use model::Id;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct Preload {
	conversation: Option<Id>,
	pressed: Option<Id>,
	/// A person and whether they are pressed.
	person: Option<(Id, bool)>,
	/// The hovered server's target channel at a state revision; resolving it can scan channels.
	guild_target: Option<(Id, u64, Option<Id>)>,
}
impl Preload {
	/// Notes a conversation row; `channel` is resolved only while the pointer is on the row.
	pub(crate) fn conversation(
		&mut self,
		response: &egui::Response,
		channel: impl FnOnce() -> Option<Id>,
	) {
		let pressed = response.is_pointer_button_down_on();
		if !pressed && !response.contains_pointer() {
			return;
		}
		let Some(channel) = channel() else {
			return;
		};
		self.conversation = Some(channel);
		if pressed {
			self.pressed = Some(channel);
		}
	}
	/// Notes a server icon by the channel its click would open.
	pub(crate) fn guild(&mut self, response: &egui::Response, state: &State, guild: Id) {
		let mut cache = self.guild_target;
		self.conversation(response, || match cache {
			Some((known, revision, target)) if known == guild && revision == state.revision => {
				target
			}
			_ => {
				let target = state.guild_target(guild);
				cache = Some((guild, state.revision, target));
				target
			}
		});
		self.guild_target = cache;
	}
	/// Notes the person a profile card would open for.
	pub(crate) fn person(&mut self, person: Option<(Id, bool)>) {
		self.person = person;
	}
	/// Sends at most one preload per kind for this frame and clears the notes for the next one.
	/// `profile_guild` is the server context a card opened now would use.
	pub(crate) fn finish(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		enabled: bool,
		profile_guild: Option<Id>,
		commands: &mut Vec<Command>,
	) {
		let Self {
			conversation,
			pressed,
			person,
			guild_target,
		} = std::mem::take(self);
		// Kept only while a server icon stays under the pointer.
		self.guild_target = guild_target.filter(|_| conversation.is_some());
		if !enabled {
			return;
		}
		let now = Instant::now();
		if let Some(command) = state.preload_conversation(conversation, pressed, now) {
			commands.push(command);
		}
		let target = person.map(|(user, _)| (user, profile_guild));
		let pressed = person.filter(|(_, pressed)| *pressed).and(target);
		if let Some(command) = state.preload_profile(target, pressed, now) {
			commands.push(command);
		}
		// A resting pointer produces no input events, so wake once when its dwell completes.
		if let Some(wait) = state.preload_wake(now) {
			ctx.request_repaint_after(wait);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_pressed_row_preloads_at_once_and_a_resting_one_wakes_without_input() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		// Preloads are online-only; the synthetic fixture stands in for a connected session.
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		state.observe_last_message(Id(21), Id(2_000));
		let mut preload = Preload::default();
		let frame = |preload: &mut Preload, state: &mut State, events| {
			let mut row = egui::Rect::NOTHING;
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(200.0, 100.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					let response = ui.button("Synthetic row");
					row = response.rect;
					preload.conversation(&response, || Some(Id(21)));
				},
			);
			let mut commands = Vec::new();
			preload.finish(&ctx, state, true, None, &mut commands);
			let wake = ctx.has_requested_repaint();
			output.drop_without_applying_deltas();
			(commands, row.center(), wake)
		};
		let (_, row, _) = frame(&mut preload, &mut state, vec![]);
		let (commands, _, wake) = frame(
			&mut preload,
			&mut state,
			vec![egui::Event::PointerMoved(row)],
		);
		assert!(commands.is_empty(), "passing over a row is not intent");
		assert!(
			wake,
			"a resting pointer needs one wake to complete its dwell"
		);
		let press = egui::Event::PointerButton {
			pos: row,
			button: egui::PointerButton::Primary,
			pressed: true,
			modifiers: egui::Modifiers::NONE,
		};
		let (commands, _, _) = frame(&mut preload, &mut state, vec![press]);
		assert!(
			matches!(
				commands.as_slice(),
				[Command::Prefetch(command)]
					if matches!(**command, Command::History { channel: Id(21), .. })
			),
			"{}",
			commands.len()
		);
		let (commands, _, _) = frame(&mut preload, &mut state, vec![]);
		assert!(commands.is_empty(), "one preload per target");
		// A pressed person preloads the card their click opens, in the given server context.
		preload.person(Some((Id(9), true)));
		let mut commands = Vec::new();
		preload.finish(&ctx, &mut state, true, None, &mut commands);
		assert!(matches!(
			commands.as_slice(),
			[Command::Prefetch(command)]
				if matches!(**command, Command::Profile { user: Id(9), guild: None, .. })
		));
		let mut disabled = Vec::new();
		preload.person(Some((Id(10), true)));
		preload.finish(&ctx, &mut state, false, None, &mut disabled);
		assert!(disabled.is_empty());
	}
}
