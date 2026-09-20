//! Bounded optimistic edits and pin toggles; server events remain authoritative.
use crate::{State, auth::Failure};
use model::{Id, Message, MessagePatch, Patch};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct MessageActions {
	sequence: u64,
	edits: BTreeMap<(Id, Id), Edit>,
	pins: BTreeMap<(Id, Id), Pin>,
	pub failed_edits: Vec<(Id, Id, String)>,
}
struct Edit {
	request: u64,
	before: String,
	after: String,
	observed: bool,
}
struct Pin {
	request: u64,
	value: bool,
	previous: bool,
	pending: bool,
}
impl MessageActions {
	pub fn pin(&self, channel: Id, message: Id) -> Option<bool> {
		self.pins.get(&(channel, message)).map(|pin| pin.value)
	}
	pub fn edit_pending(&self, channel: Id, message: Id) -> bool {
		self.edits.contains_key(&(channel, message))
	}
	pub fn pin_pending(&self, channel: Id, message: Id) -> bool {
		self.pins
			.get(&(channel, message))
			.is_some_and(|pin| pin.pending)
	}
	pub(crate) fn reconcile_pins(&mut self, channel: Id, page: &model::SearchPage, first: bool) {
		self.pins.retain(|(c, id), pin| {
			*c != channel
				|| pin.pending
				|| (!page.hits.iter().any(|hit| hit.id == *id) && !(first && !page.partial))
		});
	}
	pub fn observe_content(&mut self, channel: Id, message: Id) {
		if let Some(edit) = self.edits.get_mut(&(channel, message)) {
			edit.observed = true;
		}
	}
}
impl State {
	pub(crate) fn cancel_message_actions(&mut self) {
		for ((channel, message), edit) in std::mem::take(&mut self.message_actions.edits) {
			self.resident.remove(channel);
			if !edit.observed
				&& self.selected == Some(channel)
				&& self
					.timeline
					.get(message)
					.is_some_and(|m| m.content == edit.after)
			{
				let _ = self
					.timeline
					.patch(content_patch(channel, message, edit.before));
			}
			self.message_actions
				.failed_edits
				.push((channel, message, edit.after));
		}
		for pin in self
			.message_actions
			.pins
			.values_mut()
			.filter(|pin| pin.pending)
		{
			pin.value = pin.previous;
			pin.pending = false;
		}
		self.revision += 1;
	}
	pub(crate) fn optimistic_edit(
		&mut self,
		channel: Id,
		message: Id,
		content: &str,
	) -> Option<u64> {
		if self.message_actions.edits.len() + self.message_actions.failed_edits.len() >= 8
			|| self.message_actions.edit_pending(channel, message)
		{
			self.status = "Wait for the previous message edit to finish";
			return None;
		}
		let current = self
			.timeline
			.get(message)
			.filter(|m| m.channel == channel)?;
		// Eight snapshots, each limited to a 64 KiB source plus MAX_CONTENT UTF-8 text.
		if current.content.len() > 64 * 1024 {
			return None;
		}
		let before = current.content.clone();
		if self
			.timeline
			.patch(content_patch(channel, message, content.to_owned()))
			.is_err()
		{
			self.status = "Edit could not be applied; reload this conversation";
			return None;
		}
		self.message_actions.sequence = self.message_actions.sequence.wrapping_add(1);
		let request = self.message_actions.sequence;
		self.message_actions.edits.insert(
			(channel, message),
			Edit {
				request,
				before,
				after: content.to_owned(),
				observed: false,
			},
		);
		self.revision += 1;
		Some(request)
	}
	pub fn apply_edit_result(
		&mut self,
		channel: Id,
		message: Id,
		request: u64,
		result: Result<Message, Failure>,
	) {
		if !self
			.message_actions
			.edits
			.get(&(channel, message))
			.is_some_and(|edit| edit.request == request)
		{
			return;
		}
		self.resident.remove(channel);
		let Some(edit) = self.message_actions.edits.remove(&(channel, message)) else {
			return;
		};
		match result {
			Ok(updated)
				if updated.channel == channel
					&& updated.id == message
					&& session_cache::Timeline::valid_message(&updated) =>
			{
				if !edit.observed && self.selected == Some(channel) && self.can_view(channel) {
					let mut patch = content_patch(channel, message, updated.content);
					patch.edited = updated.edited_at.map_or(Patch::Null, Patch::Value);
					patch.mentions = Patch::Value(updated.mentions);
					let _ = self.timeline.patch(patch);
				}
			}
			result => {
				if !edit.observed
					&& self.selected == Some(channel)
					&& self
						.timeline
						.get(message)
						.is_some_and(|m| m.content == edit.after)
				{
					let _ = self
						.timeline
						.patch(content_patch(channel, message, edit.before));
				}
				self.message_actions
					.failed_edits
					.push((channel, message, edit.after));
				let failure = result.err().unwrap_or(Failure::Protocol);
				if failure.ends_session() {
					self.fail(failure);
				} else {
					self.status = "Message edit failed; your edit was kept for retry";
				}
			}
		}
		self.revision += 1;
	}
	pub(crate) fn optimistic_pin(&mut self, channel: Id, message: Id, pinned: bool) -> Option<u64> {
		if self.message_actions.pin_pending(channel, message)
			|| self
				.message_actions
				.pins
				.values()
				.filter(|p| p.pending)
				.count() >= 8
		{
			self.status = "Wait for the previous pin action to finish";
			return None;
		}
		let previous = self.is_pinned(channel, message);
		if self.message_actions.pins.len() >= 128
			&& let Some(key) = self
				.message_actions
				.pins
				.iter()
				.find_map(|(key, pin)| (!pin.pending).then_some(*key))
		{
			self.message_actions.pins.remove(&key);
		}
		self.message_actions.sequence = self.message_actions.sequence.wrapping_add(1);
		let request = self.message_actions.sequence;
		self.message_actions.pins.insert(
			(channel, message),
			Pin {
				request,
				value: pinned,
				previous,
				pending: true,
			},
		);
		self.revision += 1;
		Some(request)
	}
	pub fn apply_pin_result(
		&mut self,
		channel: Id,
		message: Id,
		pinned: bool,
		request: u64,
		result: Result<(), Failure>,
	) {
		let Some(pin) = self
			.message_actions
			.pins
			.get_mut(&(channel, message))
			.filter(|pin| pin.pending && pin.value == pinned && pin.request == request)
		else {
			return;
		};
		pin.pending = false;
		if let Err(failure) = result {
			pin.value = pin.previous;
			if matches!(failure, Failure::Ambiguous | Failure::Network) {
				self.pins_changed = Some(channel);
			}
			if failure.ends_session() {
				self.fail(failure);
			} else {
				self.status = "Pin action failed; the previous state was restored";
			}
		} else {
			if !pinned
				&& let Some(page) = self
					.search
					.as_mut()
					.filter(|v| v.pins && v.channel == channel)
					.and_then(|v| v.page.as_mut())
			{
				page.hits.retain(|hit| hit.id != message);
			}
			self.pins_changed = Some(channel);
		}
		self.revision += 1;
	}
}
fn content_patch(channel: Id, id: Id, content: String) -> MessagePatch {
	MessagePatch {
		sticker_items: Patch::Absent,
		channel,
		id,
		content: Patch::Value(content),
		edited: Patch::Absent,
		mentions: Patch::Absent,
		reactions: Patch::Absent,
		embeds: Patch::Absent,
		embeds_suppressed: Patch::Absent,
		attachments: Patch::Absent,
		components: model::Patch::Absent,
		flags: model::Patch::Absent,
		application_id: model::Patch::Absent,
		extra_content: Default::default(),
	}
}
