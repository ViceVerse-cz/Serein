//! One temporary, user-operated verification view, tied to one session and one pending write.
use client_core::{Command, State, captcha::Verification};
use eframe::egui;
use std::{sync::Arc, time::Duration};

#[derive(Default)]
pub struct Captcha {
	view: Option<(u64, Verification, platform::captcha::CaptchaView)>,
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
		let panel = &mut messaging.verification;
		if !allowed {
			if !state.demo {
				panel.active = false;
			}
			panel.start_requested = false;
		}
		let scope = state
			.verification()
			.map(|(flow, _)| (state.generation, flow));
		if !allowed
			|| !panel.active
			|| self
				.view
				.as_ref()
				.is_some_and(|(generation, flow, _)| scope != Some((*generation, *flow)))
		{
			self.close();
		}
		let start = panel.bounds.is_some() && std::mem::take(&mut panel.start_requested);
		if start
			&& allowed
			&& let Some((flow, challenge)) = state.verification()
			&& panel.verification == Some(flow)
		{
			self.close();
			let wake = ctx.clone();
			match platform::captcha::CaptchaView::open(
				window.clone(),
				challenge,
				ctx.theme() == egui::Theme::Dark,
				move || wake.request_repaint(),
			) {
				Ok(view) => self.view = Some((state.generation, flow, view)),
				Err(error) => {
					panel.active = false;
					panel.error = Some(error);
				}
			}
		}
		let (_, flow, view) = self.view.as_ref()?;
		let flow = *flow;
		let Some(bounds) = panel.bounds else {
			self.close();
			panel.active = false;
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
			panel.active = false;
			match result {
				Ok(solution) => {
					let command = state.resume_verification(flow, solution);
					if command.is_none() {
						panel.error = Some("Verification expired. Start the check again.");
					}
					return command;
				}
				Err(error) => panel.error = Some(error),
			}
		}
		None
	}
}
