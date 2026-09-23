//! General: window and graphics behaviour on this device.
use super::kit;
use crate::Serein;
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_general(
		&mut self,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Div {
		div().child(kit::hint("General settings are coming soon."))
	}
}
