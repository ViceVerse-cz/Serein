//! Chat page and the Appearance page's Layout group (reading preferences).
use super::kit;
use crate::Serein;
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_chat(&mut self, _cx: &mut Context<Self>) -> Div {
		div().child(kit::hint("Chat settings are coming soon."))
	}

	pub(super) fn settings_layout(&mut self, _cx: &mut Context<Self>) -> Div {
		div()
	}
}
