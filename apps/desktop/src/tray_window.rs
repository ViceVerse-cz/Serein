//! Shared window-close routing for all tray adapters. Exit checks remain in the desktop's rendered UI.
use super::egui;

pub struct State {
	pub hidden: bool,
	exiting: bool,
	close_after_show: bool,
	/// The close came from the system (a session logout), so it must not stop at exit checks.
	system_close: bool,
	/// A winit hide was issued to park the window in the tray and has not been
	/// confirmed yet. Compositor IPC hides are trusted; winit visibility and
	/// minimize commands are ignored by some compositors (Hyprland), so their
	/// effect is verified before the close stays cancelled. The check reads
	/// winit's own visibility: egui derives `ViewportInfo::visible` from
	/// minimized/occluded alone, so it never reports an unmapped window.
	hide_pending: bool,
	hide_wait: u8,
	/// Compositor IPC for Wayland sessions where winit cannot hide (Hyprland, KWin).
	compositor: Option<platform::compositor::Hider>,
}

/// Frames a winit hide gets to take effect before it counts as failed.
const HIDE_CONFIRM_FRAMES: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HideOutcome {
	Pending,
	Hidden,
	Failed,
}

fn hide_outcome(waited: u8, visible: Option<bool>, minimized: Option<bool>) -> HideOutcome {
	if visible == Some(false) || minimized == Some(true) {
		HideOutcome::Hidden
	} else if waited >= HIDE_CONFIRM_FRAMES {
		HideOutcome::Failed
	} else {
		HideOutcome::Pending
	}
}

impl Default for State {
	fn default() -> Self {
		Self {
			hidden: false,
			exiting: false,
			close_after_show: false,
			system_close: false,
			hide_pending: false,
			hide_wait: 0,
			compositor: platform::compositor::Hider::detect(),
		}
	}
}

impl State {
	/// Restores the window from any thread. Hyprland sends no frames to a parked window, so
	/// events that must show it cannot wait for the UI to run.
	pub fn restorer(&self) -> impl Fn() + Send + Sync + 'static {
		let compositor = self.compositor.clone();
		move || {
			if let Some(compositor) = &compositor {
				compositor.show();
			}
		}
	}
	pub fn show(&mut self, ctx: &egui::Context) {
		self.hidden = false;
		self.hide_pending = false;
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
	#[allow(dead_code)]
	pub fn minimize(&mut self, ctx: &egui::Context) {
		if let Some(compositor) = usable(&self.compositor) {
			self.hidden = compositor.minimize();
			ctx.request_repaint();
			return;
		}
		self.show(ctx);
		ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
	}
	/// macOS Cmd+W: hides the window without the close path, so it never exits the app.
	#[cfg(target_os = "macos")]
	#[allow(dead_code)]
	pub fn hide(&mut self, ctx: &egui::Context) {
		self.hidden = true;
		ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
	}
	pub fn quit(&mut self, ctx: &egui::Context) {
		self.exiting = true;
		self.close_after_show = true;
		self.show(ctx);
	}
	pub fn cancel_quit(&mut self) {
		self.exiting = false;
		self.close_after_show = false;
		self.system_close = false;
	}
	pub fn system_close(&self) -> bool {
		self.system_close
	}
	/// `native_visible` is winit's `Window::is_visible()`: `None` where the backend
	/// cannot report it (Wayland), which is the same platforms `can_hide` is false on.
	pub fn logic(
		&mut self,
		ctx: &egui::Context,
		tray_available: bool,
		can_hide: bool,
		native_visible: Option<bool>,
	) {
		let was_hidden = self.hidden;
		if self.hidden && !tray_available {
			self.show(ctx);
		}
		let (close, minimized) = ctx.input(|input| {
			(
				input.viewport().close_requested(),
				input.viewport().minimized,
			)
		});
		if self.hide_pending {
			match hide_outcome(self.hide_wait, native_visible, minimized) {
				HideOutcome::Hidden => {
					self.hide_pending = false;
					self.hidden = true;
				}
				// Wayland reports neither visibility nor minimization, so an applied hide
				// and an ignored one look identical there. Without a positive confirmation
				// leave the hide unverified rather than closing a window the user asked to
				// put in the tray.
				HideOutcome::Pending | HideOutcome::Failed if !can_hide => {
					self.hide_pending = false;
				}
				HideOutcome::Pending => {
					self.hide_wait += 1;
					ctx.request_repaint();
				}
				HideOutcome::Failed => {
					// The compositor ignored the hide, so keeping the close cancelled would
					// strand a visible window that cannot be closed. Close for real instead:
					// ViewportCommand::Close raises a close request for a later frame, which
					// the desktop's exit checks still see and can cancel.
					self.hide_pending = false;
					self.exiting = true;
					ctx.send_viewport_cmd(egui::ViewportCommand::Close);
				}
			}
		}
		if !close {
			return;
		}
		let unseen = was_hidden || native_visible == Some(false);
		if tray_available && !self.exiting && !unseen {
			// Native Wayland cannot hide surfaces through winit, Hyprland ignores minimization
			// and KWin keeps minimized windows in the taskbar, so their IPC hides the window
			// instead. Elsewhere never label a possibly visible window hidden; the compositor
			// may ignore minimization.
			if let Some(compositor) = usable(&self.compositor) {
				self.hidden = true;
				compositor.hide();
			} else {
				self.hide_pending = true;
				self.hide_wait = 0;
				ctx.send_viewport_cmd(if can_hide {
					egui::ViewportCommand::Visible(false)
				} else {
					egui::ViewportCommand::Minimized(true)
				});
			}
		} else {
			// Nobody can close a window they cannot see: a hidden window's close comes from the
			// session ending (Plasma closes every window at logout), so exit instead of blocking it.
			self.system_close |= !self.exiting && unseen;
			// Registration can finish while the existing exit dialog is open.
			self.exiting = true;
			if unseen || self.close_after_show {
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

/// Usable compositor IPC; KWin answers asynchronously and may be denied by a sandbox.
fn usable(
	compositor: &Option<platform::compositor::Hider>,
) -> Option<&platform::compositor::Hider> {
	compositor
		.as_ref()
		.filter(|compositor| compositor.available())
}

#[cfg(test)]
mod tests {
	use super::{HIDE_CONFIRM_FRAMES, HideOutcome, hide_outcome};

	#[test]
	fn hide_confirmed_when_window_disappears_or_minimizes() {
		assert_eq!(
			hide_outcome(0, Some(false), None),
			HideOutcome::Hidden,
			"an unmapped window is hidden"
		);
		assert_eq!(
			hide_outcome(0, Some(true), Some(true)),
			HideOutcome::Hidden,
			"a minimized window is hidden even though it stays mapped"
		);
	}

	#[test]
	fn hide_stays_pending_within_the_frame_budget() {
		assert_eq!(
			hide_outcome(0, Some(true), Some(false)),
			HideOutcome::Pending,
			"the compositor gets frames to apply the hide"
		);
		assert_eq!(
			hide_outcome(HIDE_CONFIRM_FRAMES - 1, Some(true), None),
			HideOutcome::Pending,
			"unknown minimized state does not fail the hide early"
		);
	}

	#[test]
	fn hide_fails_when_window_stays_visible_past_the_budget() {
		assert_eq!(
			hide_outcome(HIDE_CONFIRM_FRAMES, Some(true), Some(false)),
			HideOutcome::Failed,
			"a still visible window means the compositor ignored the hide"
		);
		assert_eq!(
			hide_outcome(HIDE_CONFIRM_FRAMES, Some(true), None),
			HideOutcome::Failed,
			"missing minimized state cannot confirm a hide either"
		);
	}
}
