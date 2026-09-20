//! Session-only, bounded sticker catalog; all sends use the normal pending lifecycle.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Id, Sticker, StickerPack};
pub const MAX_PACKS: usize = 128;
pub const MAX_PACK_BYTES: usize = 1024 * 1024;
#[derive(Default)]
pub struct Stickers {
	pub external_allowed: bool,
	pub packs: Vec<StickerPack>,
	pub loading: bool,
	pub loaded: bool,
	pub error: Option<&'static str>,
	pub recent: Vec<Sticker>,
	pub detail: Option<Sticker>,
	pub detail_loading: Option<Id>,
	pub detail_error: Option<&'static str>,
}
pub fn pack_bytes(packs: &Vec<StickerPack>) -> usize {
	packs.capacity() * size_of::<StickerPack>()
		+ packs
			.iter()
			.map(|p| p.name.capacity() + model::sticker_bytes(&p.stickers))
			.sum::<usize>()
}
impl State {
	pub(crate) fn interrupt_stickers(&mut self) {
		if self.stickers.loading {
			self.stickers.loading = false;
			self.stickers.error = Some("Sticker loading interrupted; try again");
		}
		if self.stickers.detail_loading.take().is_some() {
			self.stickers.detail_error = Some("Sticker loading interrupted; try again");
		}
	}

	pub(crate) fn remember_sticker(&mut self, sticker: Sticker) {
		self.stickers.recent.retain(|s| s.id != sticker.id);
		self.stickers.recent.insert(0, sticker);
		self.stickers.recent.truncate(24);
		while model::sticker_bytes(&self.stickers.recent) > 64 * 1024 {
			self.stickers.recent.pop();
		}
	}
	pub fn request_sticker_packs(&mut self) -> Option<Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.stickers.loading
			|| self.stickers.loaded
		{
			return None;
		}
		self.stickers.loading = true;
		self.stickers.error = None;
		Some(Command::StickerPacks)
	}
	pub fn request_sticker(&mut self, id: Id) -> Option<Command> {
		if id.0 == 0
			|| self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.stickers.detail_loading == Some(id)
			|| self.stickers.detail.as_ref().is_some_and(|s| s.id == id)
		{
			return None;
		}
		self.stickers.detail = None;
		self.stickers.detail_error = None;
		self.stickers.detail_loading = Some(id);
		Some(Command::Sticker(id))
	}
	pub fn sticker_requires_nitro(&self, sticker: &Sticker) -> bool {
		!self.stickers.external_allowed
			&& sticker.guild_id.is_some()
			&& sticker.guild_id
				!= self
					.selected
					.and_then(|id| self.channel(id))
					.and_then(|c| c.guild)
	}
	pub fn can_send_sticker(&self, sticker: &Sticker) -> bool {
		let Some(channel) = self.selected else {
			return false;
		};
		let guild = self.channel(channel).and_then(|c| c.guild);
		if let Some(owner) = sticker.guild_id.and_then(|id| self.guild(id))
			&& !owner
				.stickers
				.as_ref()
				.is_some_and(|items| items.iter().any(|s| s.id == sticker.id && s.available))
		{
			return false;
		}

		sticker.valid()
			&& sticker.available
			&& self.can_send(channel)
			&& !self.sticker_requires_nitro(sticker)
			&& (sticker.guild_id.is_none()
				|| sticker.guild_id == guild
				|| guild.is_none()
				|| self.permission(channel, model::permissions::USE_EXTERNAL_STICKERS)
					== Some(true))
	}
	pub fn prepare_sticker_send(&mut self, sticker: &Sticker) -> Option<Command> {
		if !self.can_send_sticker(sticker) {
			self.status = if self.sticker_requires_nitro(sticker) {
				"Nitro is required to use this sticker outside its server"
			} else {
				"This sticker is unavailable in this conversation"
			};
			return None;
		}
		self.prepare_message(&[], Some(sticker), false)
	}
	pub fn discard_pending_sticker(&mut self, nonce: &str) {
		self.pending.retain(|p| {
			p.nonce != nonce
				|| p.sticker.is_none()
				|| matches!(p.delivery, crate::Delivery::Sending)
		});
	}
	pub(crate) fn apply_sticker_packs(&mut self, result: Result<Vec<StickerPack>, Failure>) {
		if !self.stickers.loading {
			return;
		}
		self.stickers.loading = false;
		match result {
			Ok(packs)
				if packs.len() <= MAX_PACKS
					&& pack_bytes(&packs) <= MAX_PACK_BYTES
					&& packs.iter().all(|p| {
						p.id.0 != 0
							&& p.name.len() <= 256
							&& model::valid_stickers(&p.stickers, model::MAX_GUILD_STICKERS)
					}) =>
			{
				self.stickers.packs = packs;
				self.stickers.loaded = true;
				self.stickers.error = None;
			}
			Ok(_) => self.stickers.error = Some(Failure::Capacity.label()),
			Err(e) => self.stickers.error = Some(e.label()),
		}
	}
	pub(crate) fn apply_sticker(&mut self, id: Id, result: Result<Sticker, Failure>) {
		if self.stickers.detail_loading != Some(id) {
			return;
		}
		self.stickers.detail_loading = None;
		match result {
			Ok(s) if s.id == id && s.valid() => self.stickers.detail = Some(s),
			Ok(_) => self.stickers.detail_error = Some(Failure::Protocol.label()),
			Err(e) => self.stickers.detail_error = Some(e.label()),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn sticker_send_preserves_draft_and_failed_send_is_explicit() {
		let mut state = State {
			channels: vec![model::Channel {
				id: Id(1),
				guild: None,
				parent_id: None,
				kind: 1,
				name: "DM".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			selected: Some(Id(1)),
			auth: AuthState::Authenticated,
			freshness: crate::Freshness::Fresh,
			gateway_connected: true,
			..State::default()
		};
		let mut sticker = Sticker {
			id: Id(9),
			name: "Wave".into(),
			description: String::new(),
			tags: "hello".into(),
			format_type: 1,
			guild_id: None,
			pack_id: Some(Id(8)),
			available: true,
		};
		state.drafts.insert(Id(1), "Keep my text".into());
		let command = state.prepare_sticker_send(&sticker).unwrap();
		assert!(
			matches!(&command,Command::Send{sticker:Some(Id(9)),content,..} if content.is_empty())
		);
		assert_eq!(state.drafts[&Id(1)], "Keep my text");
		assert_eq!(state.pending[0].sticker.as_ref(), Some(&sticker));
		let nonce = state.pending[0].nonce.clone();
		state.discard_pending_sticker(&nonce);
		assert_eq!(state.pending.len(), 1);
		state.command_rejected(command);
		assert_eq!(state.pending[0].delivery, crate::Delivery::Rejected);
		state.discard_pending_sticker(&nonce);
		assert!(state.pending.is_empty());
		sticker.available = false;
		assert!(state.prepare_sticker_send(&sticker).is_none());
		assert!(state.request_sticker_packs().is_some());
		assert!(state.request_sticker_packs().is_none());
		state.apply_sticker_packs(Err(Failure::Network));
		assert!(state.stickers.error.is_some());
		assert!(state.request_sticker_packs().is_some());
		state.apply_sticker_packs(Ok(vec![]));
		assert!(state.request_sticker_packs().is_none());
		assert!(state.request_sticker(Id(9)).is_some());
		state.apply_sticker(Id(8), Ok(sticker.clone()));
		assert_eq!(state.stickers.detail_loading, Some(Id(9)));
		state.apply_sticker(Id(9), Ok(sticker));
		assert!(state.stickers.detail.is_some());
		state.stickers.loaded = false;
		assert!(state.request_sticker_packs().is_some());
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::Disconnected,
		});
		assert!(!state.stickers.loading && state.stickers.error.is_some());
		state.gateway_connected = true;
		assert!(state.request_sticker_packs().is_some());
		// A server sticker in a DM needs a confirmed current-account entitlement.
		state.freshness = crate::Freshness::Fresh;
		let mut external = state.stickers.detail.clone().unwrap();
		external.available = true;
		external.guild_id = Some(Id(99));
		external.pack_id = None;
		state.user = Some(crate::tests::message(1).author);
		let own = state.user.as_ref().unwrap().id;
		assert!(state.prepare_sticker_send(&external).is_none());
		for (premium_type, allowed) in [
			(model::Patch::Value(2), true),
			(model::Patch::Absent, true),
			(model::Patch::Value(0), false),
			(model::Patch::Value(3), true),
			(model::Patch::Null, false),
			(model::Patch::Value(1), false),
			(model::Patch::Value(255), false),
		] {
			state.apply(crate::Envelope {
				generation: state.generation,
				event: crate::Event::StickerEntitlement {
					user: own,
					premium_type,
				},
			});
			assert_eq!(state.can_send_sticker(&external), allowed);
		}
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::StickerEntitlement {
				user: Id(999),
				premium_type: model::Patch::Value(2),
			},
		});
		assert!(!state.can_send_sticker(&external));
		state.channels[0].guild = Some(Id(99));
		assert!(!state.sticker_requires_nitro(&external));
		state.channels[0].guild = Some(Id(100));
		assert!(state.sticker_requires_nitro(&external));
	}
}
