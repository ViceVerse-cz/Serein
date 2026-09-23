//! Keybinds: the shortcuts this frontend binds.
use super::kit;
use crate::Serein;
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_keybinds(&mut self, _cx: &mut Context<Self>) -> Div {
		div().child(kit::hint("Keybinds are coming soon."))
	}
}
