//! Data & Privacy: what this frontend keeps on the device.
use super::kit;
use crate::Serein;
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_storage(
		&mut self,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Div {
		div().child(kit::hint("Data & Privacy is coming soon."))
	}
}
