//! Shared window-close routing for all tray adapters. Exit checks remain in the desktop's rendered UI.
use super::egui;

#[derive(Default)]
pub struct State {
	hidden: bool,
	exiting: bool,
	close_after_show: bool,
}

impl State {
	pub fn show(&mut self, ctx: &egui::Context) {
		self.hidden = false;
		ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
		ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
		ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
	}
	pub fn quit(&mut self, ctx: &egui::Context) {
		self.exiting = true;
		self.close_after_show = true;
		self.show(ctx);
	}
	pub fn cancel_quit(&mut self) {
		self.exiting = false;
		self.close_after_show = false;
	}
	pub fn logic(&mut self, ctx: &egui::Context, tray_available: bool) {
		let was_hidden = self.hidden;
		if self.hidden && !tray_available {
			self.show(ctx);
		}
		let (close, visible) = ctx.input(|input| {
			(
				input.viewport().close_requested(),
				input.viewport().visible(),
			)
		});
		if !close {
			return;
		}
		if tray_available && !self.exiting {
			self.hidden = true;
			ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
		} else {
			// Registration can finish while the existing exit dialog is open.
			self.exiting = true;
			if was_hidden || visible == Some(false) || self.close_after_show {
				self.close_after_show = true;
				self.show(ctx);
			} else {
				return;
			}
		}
		ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
		ctx.input_mut(|input| {
			let viewport = input.raw.viewport_id;
			if let Some(info) = input.raw.viewports.get_mut(&viewport) {
				info.events
					.retain(|event| *event != egui::ViewportEvent::Close);
			}
		});
	}
	pub fn ui(&mut self, ctx: &egui::Context) {
		if std::mem::take(&mut self.close_after_show) {
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{ViewportCommand as Command, ViewportId};

	fn input(close: bool, minimized: bool, occluded: bool) -> egui::RawInput {
		let mut input = egui::RawInput::default();
		let viewport = input.viewports.get_mut(&ViewportId::ROOT).unwrap();
		viewport.minimized = Some(minimized);
		viewport.occluded = Some(occluded);
		if close {
			viewport.events.push(egui::ViewportEvent::Close);
		}
		input
	}

	#[test]
	fn close_hides_and_consumes_exit_before_ui_guards() {
		let ctx = egui::Context::default();
		let mut state = State::default();
		let mut output = ctx.run_ui(input(true, false, false), |ui| {
			state.logic(ui.ctx(), true);
			state.ui(ui.ctx());
			assert!(!ui.ctx().input(|input| input.viewport().close_requested()));
		});
		output.textures_delta.clear();
		let commands = &output.viewport_output[&ViewportId::ROOT].commands;
		assert!(commands.contains(&Command::CancelClose));
		assert!(commands.contains(&Command::Visible(false)));
		assert!(!commands.contains(&Command::Close));
		assert!(state.hidden);
	}

	#[test]
	fn host_loss_restores_hidden_window_without_quitting() {
		let ctx = egui::Context::default();
		let mut state = State {
			hidden: true,
			..Default::default()
		};
		let output = ctx.run_logic(&input(false, false, true), |ctx| state.logic(ctx, false));
		let commands = &output.viewport_commands[&ViewportId::ROOT];
		assert!(commands.contains(&Command::Visible(true)));
		assert!(commands.contains(&Command::Minimized(false)));
		assert!(commands.contains(&Command::Focus));
		assert!(!commands.contains(&Command::Close));
		assert!(!state.hidden);
		assert!(!state.close_after_show);
	}

	#[test]
	fn tray_quit_waits_for_ui_then_preserves_exit_request_for_guards() {
		let ctx = egui::Context::default();
		let mut state = State {
			hidden: true,
			..Default::default()
		};
		let output = ctx.run_logic(&input(true, false, true), |ctx| {
			state.quit(ctx);
			state.logic(ctx, true);
		});
		let commands = &output.viewport_commands[&ViewportId::ROOT];
		assert!(commands.contains(&Command::Visible(true)));
		assert!(!commands.contains(&Command::Close));
		let mut output = ctx.run_ui(input(false, false, false), |ui| state.ui(ui.ctx()));
		output.textures_delta.clear();
		assert!(
			output.viewport_output[&ViewportId::ROOT]
				.commands
				.contains(&Command::Close)
		);
		assert!(!state.close_after_show);
		let mut output = ctx.run_ui(input(true, false, false), |ui| {
			state.logic(ui.ctx(), true);
			assert!(ui.ctx().input(|input| input.viewport().close_requested()));
		});
		output.textures_delta.clear();
		assert!(
			!output.viewport_output[&ViewportId::ROOT]
				.commands
				.contains(&Command::CancelClose)
		);
	}

	#[test]
	fn hidden_minimized_and_occluded_close_wait_for_rendered_exit_guards() {
		for (hidden, minimized, occluded) in [
			(true, false, false),
			(false, true, false),
			(false, false, true),
		] {
			let ctx = egui::Context::default();
			let mut state = State {
				hidden,
				..Default::default()
			};
			let output = ctx.run_logic(&input(true, minimized, occluded), |ctx| {
				state.logic(ctx, false);
				assert!(!ctx.input(|input| input.viewport().close_requested()));
			});
			let commands = &output.viewport_commands[&ViewportId::ROOT];
			assert!(commands.contains(&Command::CancelClose));
			assert!(commands.contains(&Command::Visible(true)));
			assert!(!commands.contains(&Command::Close));
			let mut output = ctx.run_ui(input(false, false, false), |ui| state.ui(ui.ctx()));
			output.textures_delta.clear();
			assert!(
				output.viewport_output[&ViewportId::ROOT]
					.commands
					.contains(&Command::Close)
			);
		}
	}

	#[test]
	fn registration_does_not_cancel_an_exit_already_under_review() {
		let ctx = egui::Context::default();
		let mut state = State::default();
		for available in [false, true] {
			let output = ctx.run_logic(&input(true, false, false), |ctx| {
				state.logic(ctx, available);
				assert!(ctx.input(|input| input.viewport().close_requested()));
			});
			assert!(
				!output
					.viewport_commands
					.values()
					.flatten()
					.any(|command| *command == Command::CancelClose)
			);
		}
	}

	#[test]
	fn cancelled_quit_restores_close_to_tray_behavior() {
		let ctx = egui::Context::default();
		let mut state = State::default();
		let _ = ctx.run_logic(&input(false, false, false), |ctx| state.quit(ctx));
		state.cancel_quit();
		assert!(!state.exiting);
		assert!(!state.close_after_show);
		let mut output = ctx.run_ui(input(true, false, false), |ui| {
			state.logic(ui.ctx(), true);
			state.ui(ui.ctx());
		});
		output.textures_delta.clear();
		let commands = &output.viewport_output[&ViewportId::ROOT].commands;
		assert!(commands.contains(&Command::CancelClose));
		assert!(commands.contains(&Command::Visible(false)));
		assert!(!commands.contains(&Command::Close));
	}
}
