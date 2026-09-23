//! Messaging Permissions: who can contact you and how messages are filtered.
use super::kit;
use crate::Serein;
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_permissions(
		&mut self,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Div {
		div().child(kit::hint("Messaging permissions are coming soon."))
	}
}
