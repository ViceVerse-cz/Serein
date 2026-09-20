//! Shared window-close routing for all tray adapters. Exit checks remain in the desktop's rendered UI.
use super::egui;

pub struct State {
	pub hidden: bool,
	exiting: bool,
	close_after_show: bool,
	/// Compositor IPC for Wayland sessions where winit can neither hide nor minimize (Hyprland).
	compositor: Option<platform::compositor::Hider>,
}

impl Default for State {
	fn default() -> Self {
		Self {
			hidden: false,
			exiting: false,
			close_after_show: false,
			compositor: platform::compositor::Hider::detect(),
		}
	}
}

impl State {
	pub fn show(&mut self, ctx: &egui::Context) {
		self.hidden = false;
		if let Some(compositor) = &self.compositor {
			compositor.show();
		}
		ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
		ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
		ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
		ctx.request_repaint();
	}
	/// Tray "minimize": hides through the compositor where minimization is ignored, otherwise
	/// restores the window first so the minimize request lands on a mapped surface.
	#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
	pub fn minimize(&mut self, ctx: &egui::Context) {
		if let Some(compositor) = &self.compositor {
			self.hidden = true;
			compositor.hide();
			ctx.request_repaint();
			return;
		}
		self.show(ctx);
		ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
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
	pub fn logic(&mut self, ctx: &egui::Context, tray_available: bool, can_hide: bool) {
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
			// Native Wayland cannot hide surfaces through winit, and Hyprland ignores
			// minimization, so its IPC parks the window instead. Elsewhere never label a
			// possibly visible window hidden; the compositor may ignore minimization.
			if let Some(compositor) = &self.compositor {
				self.hidden = true;
				compositor.hide();
			} else {
				self.hidden = can_hide;
				ctx.send_viewport_cmd(if can_hide {
					egui::ViewportCommand::Visible(false)
				} else {
					egui::ViewportCommand::Minimized(true)
				});
			}
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
