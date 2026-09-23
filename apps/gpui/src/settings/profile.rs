//! Profile: how you appear across Discord.
use super::kit;
use crate::Serein;
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_profile(
		&mut self,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Div {
		div().child(kit::hint("Profile editing is coming soon."))
	}
}
