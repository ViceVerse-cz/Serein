//! One temporary, user-operated verification view, tied to one session and invite request.
use client_core::{Command, State};
use eframe::egui;
use std::{sync::Arc, time::Duration};

#[derive(Default)]
pub struct Captcha {
	view: Option<(u64, u64, platform::captcha::CaptchaView)>,
}
impl Captcha {
	pub fn close(&mut self) {
		self.view = None;
	}
	pub fn sync(
		&mut self,
		state: &mut State,
		messaging: &mut ui::MessagingUi,
		window: &Arc<winit::window::Window>,
		ctx: &egui::Context,
		allowed: bool,
	) -> Option<Command> {
		let allowed = allowed && !state.demo;
		let verification = &mut messaging.verification;
		if !allowed {
			if !state.demo {
				verification.active = false;
			}
			verification.start_requested = false;
		}
		let scope = state
			.invite_challenge()
			.map(|(request, _)| (state.generation, request));
		if !allowed
			|| !verification.active
			|| self
				.view
				.as_ref()
				.is_some_and(|(generation, request, _)| scope != Some((*generation, *request)))
		{
			self.close();
		}
		let start =
			verification.bounds.is_some() && std::mem::take(&mut verification.start_requested);
		if start
			&& allowed
			&& let Some((request, challenge)) = state.invite_challenge()
			&& verification.request == Some(request)
		{
			self.close();
			let wake = ctx.clone();
			match platform::captcha::CaptchaView::open(
				window.clone(),
				challenge,
				ctx.theme() == egui::Theme::Dark,
				move || wake.request_repaint(),
			) {
				Ok(view) => self.view = Some((state.generation, request, view)),
				Err(error) => {
					verification.active = false;
					verification.error = Some(error);
				}
			}
		}
		let (_, request, view) = self.view.as_ref()?;
		let request = *request;
		let Some(bounds) = verification.bounds else {
			self.close();
			verification.active = false;
			return None;
		};
		let scale = ctx.pixels_per_point();
		view.set_bounds(
			(bounds.left() * scale).round() as i32,
			(bounds.top() * scale).round() as i32,
			(bounds.width() * scale).round() as u32,
			(bounds.height() * scale).round() as u32,
		);
		ctx.request_repaint_after(if cfg!(target_os = "linux") {
			Duration::from_millis(20)
		} else {
			Duration::from_secs(1)
		});
		let result = if view.expired() {
			Some(Err("Verification expired. Start the check again."))
		} else {
			view.poll()
		};
		if let Some(result) = result {
			self.close();
			verification.active = false;
			match result {
				Ok(solution) => {
					let command = state.resume_invite_challenge(request, solution);
					if command.is_none() {
						verification.error = Some("Verification expired. Try joining again.");
					}
					return command;
				}
				Err(error) => verification.error = Some(error),
			}
		}
		None
	}
}
