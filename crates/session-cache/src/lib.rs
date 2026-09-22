//! Explicitly bounded RAM timeline. Patches live for a page; tombstones live for this window.
use model::{Id, Message, MessagePatch, Patch};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_MESSAGES: usize = 500;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MUTATIONS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentSource {
	/// Local optimistic body change; never records a prior.
	Optimistic,
	/// Service-observed content; push the stored body when it differs.
	Observed,
	/// Own edit acknowledgment; push `previous` once when it differs.
	OwnConfirm { previous: String },
	/// Restore pre-optimistic body; never records a prior.
	Rollback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentRevision {
	pub content: String,
	pub edited_at: Option<i128>,
	pub source: ContentSource,
}

fn keep_author_membership(message: &mut Message, roles: &[Id], nick: Option<&str>) {
	if message.author_roles.is_empty() && !roles.is_empty() {
		message.author_roles = roles.to_vec();
	}
	if message.author_nick.is_none()
		&& let Some(nick) = nick.filter(|n| !n.is_empty())
	{
		message.author_nick = Some(nick.to_owned());
	}
}

fn records_prior(message: &Message) -> bool {
	!message.forwarded && !message.is_system()
}

fn apply_edited_at(message: &mut Message, edited_at: Option<i128>) {
	match edited_at {
		Some(at) => {
			message.edited = true;
			message.edited_at = Some(at);
		}
		None => {
			message.edited = false;
			message.edited_at = None;
		}
	}
}

fn record_observed_prior(message: &mut Message, previous: &str, next: &str) {
	if !records_prior(message) || previous == next {
		return;
	}
	if message
		.prior_contents
		.as_slice()
		.last()
		.is_some_and(|last| last == previous)
	{
		return;
	}
	message.prior_contents.push_line(previous.to_owned());
}
#[derive(Default)]
pub struct Timeline {
	// None preserves only the position of a message deleted while it was loaded.
	messages: BTreeMap<Id, Option<Message>>,
	payload_count: usize,
	bytes: usize,
	changed: BTreeSet<Id>,
	patches: BTreeMap<Id, MessagePatch>,
	patch_bytes: usize,
	deleted: BTreeSet<Id>,
	loading: bool,
	retain_older: bool,
	replace: bool,
}
impl Timeline {
	pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Message> {
		self.messages
			.iter()
			.filter(|(id, _)| !self.deleted.contains(id))
			.filter_map(|(_, message)| message.as_ref())
	}
	pub fn get(&self, id: Id) -> Option<&Message> {
		self.get_display(id).filter(|_| !self.is_deleted(id))
	}
	/// Includes deleted payloads for display only; never service actions or persistence.
	pub fn display_iter(&self) -> impl DoubleEndedIterator<Item = &Message> {
		self.messages.values().filter_map(Option::as_ref)
	}
	pub fn get_display(&self, id: Id) -> Option<&Message> {
		self.messages.get(&id).and_then(Option::as_ref)
	}
	pub fn apply_author_membership(&mut self, user: Id, roles: &[Id], nick: Option<&str>) -> bool {
		if user.0 == 0 {
			return false;
		}
		let mut any = false;
		for message in self.messages.values_mut().flatten() {
			if message.author.id != user || message.author.webhook {
				continue;
			}
			let next_roles =
				(!roles.is_empty() && message.author_roles != roles).then(|| roles.to_vec());
			let next_nick = nick.filter(|n| !n.is_empty()).and_then(|n| {
				(message.author_nick.as_deref() != Some(n))
					.then(|| n.chars().take(128).collect::<String>())
			});
			if next_roles.is_none() && next_nick.is_none() {
				continue;
			}
			let before = message.bytes();
			if let Some(roles) = next_roles {
				message.author_roles = roles;
			}
			if let Some(nick) = next_nick {
				message.author_nick = Some(nick);
			}
			self.bytes = self
				.bytes
				.saturating_sub(before)
				.saturating_add(message.bytes());
			any = true;
		}
		if any {
			while self.row_count() > MAX_MESSAGES || self.row_bytes() > MAX_BYTES {
				let item = if self.retain_older {
					self.messages.pop_last()
				} else {
					self.messages.pop_first()
				};
				if let Some((_, Some(old))) = item {
					self.bytes -= old.bytes();
					self.payload_count -= 1;
				}
			}
		}
		any
	}
	pub fn drop_live_history(&mut self) {
		self.cancel_page();
		self.messages.retain(|id, message| {
			let keep = self.deleted.contains(id);
			if !keep && let Some(message) = message {
				self.bytes -= message.bytes();
				self.payload_count -= 1;
			}
			keep
		});
	}
	pub fn is_deleted(&self, id: Id) -> bool {
		self.deleted.contains(&id)
	}
	/// Drop a retained deleted payload while keeping the tombstone.
	/// History cannot restore the body. A second call is a no-op.
	pub fn discard_preserved(&mut self, id: Id) -> bool {
		if !self.deleted.contains(&id) {
			return false;
		}
		let Some(old) = self.messages.get_mut(&id).and_then(Option::take) else {
			return false;
		};
		self.bytes -= old.bytes();
		self.payload_count -= 1;
		true
	}
	pub fn len(&self) -> usize {
		self.iter().count()
	}
	pub fn is_empty(&self) -> bool {
		self.iter().next().is_none()
	}
	/// Reading positions, including ID-only placeholders for previously loaded messages.
	pub fn row_ids(&self) -> impl DoubleEndedIterator<Item = Id> {
		self.messages.keys().copied()
	}
	pub fn row_count(&self) -> usize {
		self.messages.len()
	}
	/// Live and deleted payloads; empty row storage is also charged during eviction.
	pub fn bytes(&self) -> usize {
		self.bytes
	}
	fn row_bytes(&self) -> usize {
		self.bytes - self.payload_count * size_of::<Message>()
			+ self.row_count() * size_of::<Option<Message>>()
	}
	/// Conservative retained allocation estimate, including reconciliation state and
	/// B-tree node slack. This is a budget charge, not an allocator/RSS measurement.
	pub fn retained_bytes(&self) -> usize {
		size_of::<Self>() + self.bytes - self.payload_count * size_of::<Message>()
			+ tree_bytes::<(Id, Option<Message>)>(self.messages.len())
			+ tree_bytes::<Id>(self.changed.len())
			+ tree_bytes::<Id>(self.deleted.len())
			+ tree_bytes::<(Id, MessagePatch)>(self.patches.len())
			+ self.patch_bytes
			- self.patches.len() * size_of::<MessagePatch>()
	}
	pub fn begin_page(&mut self, older: bool) {
		// Set eviction direction before live events can race the history response.
		self.retain_older = older;
		self.replace = !older;
		self.loading = true;
		self.changed.clear();
		self.patches.clear();
		self.patch_bytes = 0;
	}
	pub fn begin_append(&mut self) {
		self.retain_older = false;
		self.replace = false;
		self.loading = true;
		self.changed.clear();
		self.patches.clear();
		self.patch_bytes = 0;
	}
	pub fn cancel_page(&mut self) {
		// Failure stops reconciliation, but preserves the window the user is reading.
		// Starting a recent-page request restores newest-first retention.
		self.loading = false;
		self.changed.clear();
		self.patches.clear();
		self.patch_bytes = 0;
	}
	fn remember(&mut self, id: Id) -> Result<(), &'static str> {
		if self.loading {
			if self.changed.len() >= MAX_MUTATIONS && !self.changed.contains(&id) {
				return Err("Reconciliation capacity exceeded; reload required");
			}
			self.changed.insert(id);
		}
		Ok(())
	}
	pub fn insert(
		&mut self,
		message: Message,
		live: bool,
		older: bool,
	) -> Result<(), &'static str> {
		self.observe_deleted_reference(&message)?;
		if self.deleted.contains(&message.id) {
			return Ok(());
		}
		if live {
			self.remember(message.id)?;
		}
		let mut message = message;
		message.reply_deleted |= matches!(message.kind, 19 | 23)
			&& message.reply_to.is_some_and(|target| {
				target.0 != 0 && target < message.id && self.is_deleted(target)
			});
		if let Some(patch) = self.patches.get(&message.id) {
			apply_patch(&mut message, patch);
		} else if !live && self.changed.contains(&message.id) {
			return Ok(());
		}
		if let Some(previous) = self.get(message.id) {
			if previous
				.edited_at
				.is_some_and(|old| message.edited_at.is_none_or(|new| new < old))
			{
				return Ok(());
			}
			keep_author_membership(
				&mut message,
				&previous.author_roles,
				previous.author_nick.as_deref(),
			);
			let mut priors = previous.prior_contents.clone();
			if previous.content != message.content
				&& records_prior(previous)
				&& records_prior(&message)
				&& priors
					.as_slice()
					.last()
					.is_none_or(|last| last != &previous.content)
			{
				priors.push_line(previous.content.clone());
			}
			message.prior_contents = priors;
			message.revision = previous.revision
				+ u64::from(
					previous.content != message.content
						|| previous.mentions != message.mentions
						|| previous.reactions != message.reactions
						|| previous.edited != message.edited
						|| previous.unsupported != message.unsupported
						|| previous.kind != message.kind
						|| previous.reply_deleted != message.reply_deleted
						|| previous.sticker_items != message.sticker_items
						|| previous.extra_content != message.extra_content
						|| previous.embeds != message.embeds
						|| previous.attachments != message.attachments
						|| previous.embeds_suppressed != message.embeds_suppressed
						|| previous.author_roles != message.author_roles
						|| previous.author_nick != message.author_nick
						|| previous.prior_contents != message.prior_contents,
				);
		}
		self.bytes += message.bytes();
		match self.messages.insert(message.id, Some(message)) {
			Some(Some(old)) => self.bytes -= old.bytes(),
			_ => self.payload_count += 1,
		}
		while self.row_count() > MAX_MESSAGES || self.row_bytes() > MAX_BYTES {
			let item = if older || self.retain_older {
				self.messages.pop_last()
			} else {
				self.messages.pop_first()
			};
			if let Some((_, Some(old))) = item {
				self.bytes -= old.bytes();
				self.payload_count -= 1;
			}
		}
		Ok(())
	}
	/// Shared admission check for an atomic history page and a single message.
	pub fn valid_message(message: &Message) -> bool {
		message.bytes() <= MAX_BYTES
			&& (!message.reply_deleted
				|| (matches!(message.kind, 19 | 23)
					&& message
						.reply_to
						.is_some_and(|target| target.0 != 0 && target < message.id)))
			&& message.content.len() <= 64 * 1024
			&& model::valid_mentions(&message.mentions)
			&& model::valid_mention_roles(&message.mention_roles)
			&& message.author_roles.capacity() <= model::permissions::MAX_MEMBER_ROLES
			&& message.author_roles.iter().all(|role| role.0 != 0)
			&& message
				.author_nick
				.as_ref()
				.is_none_or(|nick| nick.len() <= 512)
			&& message.application_id.is_none_or(|id| id.0 != 0)
			&& model::valid_stickers(&message.sticker_items, model::MAX_MESSAGE_STICKERS)
			&& model::valid_components(&message.components)
			&& model::valid_embeds(&message.embeds)
			&& model::valid_attachments(&message.attachments)
			&& message
				.reactions
				.as_ref()
				.is_none_or(|r| model::valid_reactions(r))
	}
	/// Preserve explicit deletion knowledge even when a correlated response must
	/// not replace the source's newer loaded body.
	pub fn observe_deleted_reference(&mut self, message: &Message) -> Result<(), &'static str> {
		if !Self::valid_message(message) {
			return Err("Message exceeds safe capacity");
		}
		if message.reply_deleted
			&& let Some(target) = message.reply_to
		{
			self.delete(target)?;
			if let Some(source) = self.messages.get_mut(&message.id).and_then(Option::as_mut)
				&& source.channel == message.channel
				&& source.reply_to == Some(target)
				&& matches!(source.kind, 19 | 23)
				&& !source.reply_deleted
			{
				source.reply_deleted = true;
				source.revision += 1;
			}
		}
		Ok(())
	}
	pub fn seed_cache(&mut self, items: Vec<Message>) -> Result<(), &'static str> {
		for item in items {
			self.insert(item, false, false)?;
		}
		Ok(())
	}
	pub fn finish_page(&mut self, items: Vec<Message>, older: bool) -> Result<(), &'static str> {
		self.retain_older = older;
		let prior: BTreeMap<_, _> = self
			.messages
			.iter()
			.filter_map(|(id, message)| {
				let message = message.as_ref()?;
				if message.author_roles.is_empty() && message.author_nick.is_none() {
					return None;
				}
				Some((
					*id,
					(message.author_roles.clone(), message.author_nick.clone()),
				))
			})
			.collect();
		if self.replace {
			self.messages.retain(|id, message| {
				let keep = self.changed.contains(id) || self.deleted.contains(id);
				if !keep && let Some(message) = message {
					self.bytes -= message.bytes();
					self.payload_count -= 1;
				}
				keep
			});
		}
		for mut item in items {
			if let Some((roles, nick)) = prior.get(&item.id) {
				keep_author_membership(&mut item, roles, nick.as_deref());
			}
			self.insert(item, false, older)?;
		}
		self.loading = false;
		self.changed.clear();
		self.patches.clear();
		self.patch_bytes = 0;
		Ok(())
	}
	/// Sole writer that may append to `Message::prior_contents`.
	pub fn observe_content(
		&mut self,
		id: Id,
		revision: ContentRevision,
	) -> Result<(), &'static str> {
		if self.deleted.contains(&id) {
			return Ok(());
		}
		if revision.content.len() > 64 * 1024 {
			return Err("Message patch exceeds capacity");
		}
		self.remember(id)?;
		let retained = self.row_bytes();
		let Some(message) = self.messages.get_mut(&id).and_then(Option::as_mut) else {
			return Ok(());
		};
		let before = message.bytes();
		let restore_content = message.content.clone();
		let restore_priors = message.prior_contents.clone();
		let restore_edited = (message.edited, message.edited_at);
		match &revision.source {
			ContentSource::Optimistic | ContentSource::Rollback => {
				message.content.clone_from(&revision.content);
			}
			ContentSource::Observed => {
				if matches!(
					revision.edited_at,
					Some(new) if message.edited_at.is_some_and(|old| new < old)
				) {
					return Ok(());
				}
				if message.forwarded {
					return Ok(());
				}
				let previous = message.content.clone();
				record_observed_prior(message, &previous, &revision.content);
				message.content.clone_from(&revision.content);
				apply_edited_at(message, revision.edited_at);
			}
			ContentSource::OwnConfirm { previous } => {
				if previous.as_str() != revision.content
					&& message
						.prior_contents
						.as_slice()
						.last()
						.is_none_or(|last| last != previous)
				{
					message.prior_contents.push_line(previous.clone());
				}
				message.content.clone_from(&revision.content);
				apply_edited_at(message, revision.edited_at);
			}
		}
		let after = message.bytes();
		if retained - before + after > MAX_BYTES {
			message.content = restore_content;
			message.prior_contents = restore_priors;
			message.edited = restore_edited.0;
			message.edited_at = restore_edited.1;
			return Err("Message exceeds timeline capacity");
		}
		message.revision += 1;
		self.bytes = self.bytes - before + after;
		Ok(())
	}
	pub fn patch(&mut self, patch: MessagePatch) -> Result<(), &'static str> {
		if self.deleted.contains(&patch.id) {
			return Ok(());
		}
		if matches!(&patch.content, Patch::Value(s) if s.len() > 64 * 1024)
			|| matches!(&patch.reactions, Patch::Value(r) if !model::valid_reactions(r))
			|| matches!(&patch.mentions, Patch::Value(users) if !model::valid_mentions(users))
			|| matches!(&patch.application_id, Patch::Value(id) if id.0 == 0)
			|| matches!(&patch.sticker_items, Patch::Value(s) if !model::valid_stickers(s,model::MAX_MESSAGE_STICKERS))
			|| matches!(&patch.components, Patch::Value(c) if !model::valid_components(c))
			|| matches!(&patch.embeds, Patch::Value(embeds) if !model::valid_embeds(embeds))
			|| matches!(&patch.attachments, Patch::Value(attachments) if !model::valid_attachments(attachments))
		{
			return Err("Message patch exceeds capacity");
		}
		self.remember(patch.id)?;
		if let Some(Some(mut message)) = self.messages.remove(&patch.id) {
			self.bytes -= message.bytes();
			self.payload_count -= 1;
			// Once hydrated, changed protects this record from the in-flight history page.
			if let Some(old) = self.patches.remove(&patch.id) {
				self.patch_bytes -= patch_bytes(&old);
			}
			apply_patch(&mut message, &patch);
			self.insert(message, true, false)?;
		} else if self.loading {
			if self.patches.get(&patch.id).is_some_and(|previous| {
                matches!((&previous.edited, &patch.edited), (Patch::Value(old), Patch::Value(new)) if new < old)
            }) {
                return Ok(());
            }
			let bytes = self.patch_bytes;
			let mut merged = self
				.patches
				.get(&patch.id)
				.cloned()
				.unwrap_or_else(|| patch.clone());
			if !matches!(patch.flags, Patch::Absent) {
				merged.flags = patch.flags;
			}
			if !matches!(patch.sticker_items, Patch::Absent) {
				merged.sticker_items = patch.sticker_items;
			}
			if !matches!(patch.components, Patch::Absent) {
				merged.components = patch.components;
			}
			if !matches!(patch.application_id, Patch::Absent) {
				merged.application_id = patch.application_id;
			}
			if !matches!(patch.content, Patch::Absent) {
				merged.content = patch.content;
			}
			if !matches!(patch.reactions, Patch::Absent) {
				merged.reactions = patch.reactions;
			}
			if !matches!(patch.mentions, Patch::Absent) {
				merged.mentions = patch.mentions;
			}
			if !matches!(patch.edited, Patch::Absent) {
				merged.edited = patch.edited;
			}
			if !matches!(patch.embeds, Patch::Absent) {
				merged.embeds = patch.embeds;
			}
			if !matches!(patch.attachments, Patch::Absent) {
				merged.attachments = patch.attachments;
			}
			if !matches!(patch.embeds_suppressed, Patch::Absent) {
				merged.embeds_suppressed = patch.embeds_suppressed;
			}
			merged.extra_content.merge(&patch.extra_content);
			let replaced = self.patches.get(&patch.id).map_or(0, patch_bytes);
			if bytes - replaced + patch_bytes(&merged) > 1024 * 1024 {
				return Err("Pending patch byte budget exceeded; reload required");
			}
			self.patch_bytes = bytes - replaced + patch_bytes(&merged);
			self.patches.insert(merged.id, merged);
		}
		Ok(())
	}
	pub fn delete(&mut self, id: Id) -> Result<(), &'static str> {
		if self.deleted.len() >= MAX_MUTATIONS && !self.deleted.contains(&id) {
			return Err("Deletion reconciliation capacity exceeded; reload required");
		}
		self.remember(id)?;
		self.deleted.insert(id);
		if let Some(old) = self.patches.remove(&id) {
			self.patch_bytes -= patch_bytes(&old);
		}
		Ok(())
	}
	pub fn clear(&mut self) {
		*self = Self::default();
	}
	/// Change the reading range within the same channel without forgetting known
	/// deletions. Use `clear` when changing channel/session ownership instead.
	pub fn clear_window_preserving_deletions(&mut self) {
		let deleted = std::mem::take(&mut self.deleted);
		*self = Self {
			deleted,
			..Self::default()
		};
	}
	pub fn set_reactions(
		&mut self,
		id: Id,
		reactions: Option<Vec<model::Reaction>>,
	) -> Result<(), &'static str> {
		if reactions
			.as_ref()
			.is_some_and(|r| !model::valid_reactions(r))
		{
			return Err("Reaction data exceeds safe capacity");
		}
		if self.is_deleted(id) {
			return Ok(());
		}
		// Take the footprint before borrowing a live row mutably.
		let retained = self.row_bytes();
		let Some(message) = self.messages.get_mut(&id).and_then(Option::as_mut) else {
			return Ok(());
		};
		let old = message.bytes();
		let new = reactions.as_ref().map_or(0, |r| {
			model::reaction_bytes(r)
				+ r.capacity().saturating_sub(r.len()) * size_of::<model::Reaction>()
		});
		let previous = message.reactions.as_ref().map_or(0, |r| {
			model::reaction_bytes(r)
				+ r.capacity().saturating_sub(r.len()) * size_of::<model::Reaction>()
		});
		if retained - previous + new > MAX_BYTES {
			return Err("Reaction data exceeds timeline capacity");
		}
		message.reactions = reactions;
		message.revision += 1;
		self.bytes = self.bytes - old + message.bytes();
		Ok(())
	}
}
fn tree_bytes<T>(len: usize) -> usize {
	if len == 0 {
		0
	} else {
		// Charge node occupancy slack plus a root, with child/parent pointer space.
		(len + 11) * 3 * (size_of::<T>() + 2 * size_of::<usize>())
	}
}
fn patch_bytes(patch: &MessagePatch) -> usize {
	let content = match &patch.content {
		Patch::Value(value) => value.capacity(),
		_ => 0,
	};
	size_of::<MessagePatch>()
		+ content
		+ match &patch.sticker_items {
			Patch::Value(s) => model::sticker_bytes(s),
			_ => 0,
		} + match &patch.components {
		Patch::Value(c) => model::component_bytes(c),
		_ => 0,
	} + match &patch.reactions {
		Patch::Value(r) => {
			model::reaction_bytes(r)
				+ r.capacity().saturating_sub(r.len()) * size_of::<model::Reaction>()
		}
		_ => 0,
	} + match &patch.mentions {
		Patch::Value(users) => model::mention_bytes(users),
		_ => 0,
	} + match &patch.embeds {
		Patch::Value(value) => {
			model::embed_bytes(value)
				+ value.capacity().saturating_sub(value.len()) * size_of::<model::Embed>()
		}
		_ => 0,
	} + match &patch.attachments {
		Patch::Value(value) => {
			model::attachment_bytes(value)
				+ value.capacity().saturating_sub(value.len()) * size_of::<model::Attachment>()
		}
		_ => 0,
	}
}
/// Apply a previously bounded patch (the caller must validate component and payload limits).
pub fn apply_patch(message: &mut Message, patch: &MessagePatch) {
	if matches!(patch.edited,Patch::Value(new) if message.edited_at.is_some_and(|old|new<old)) {
		return;
	}
	match &patch.mentions {
		Patch::Value(users) => message.mentions.clone_from(users),
		Patch::Null => message.mentions.clear(),
		Patch::Absent => {}
	}
	match &patch.reactions {
		Patch::Absent => {}
		Patch::Null => message.reactions = Some(vec![]),
		Patch::Value(r) => message.reactions = Some(r.clone()),
	}
	match patch.flags {
		Patch::Value(flags) => {
			message.flags = flags;
			message.ephemeral = flags & 64 != 0;
		}
		Patch::Null => {
			message.flags = 0;
			message.ephemeral = false;
		}
		Patch::Absent => {}
	}
	// Gateway updates describe the outer message, never edits to its frozen snapshot.
	if message.forwarded {
		message.revision += 1;
		return;
	}
	match &patch.sticker_items {
		Patch::Value(s) => message.sticker_items.clone_from(s),
		Patch::Null => message.sticker_items.clear(),
		Patch::Absent => {}
	}
	match &patch.components {
		Patch::Value(c) => message.components.clone_from(c),
		Patch::Null => message.components.clear(),
		Patch::Absent => {}
	}
	match &patch.application_id {
		Patch::Value(id) => message.application_id = Some(*id),
		Patch::Null => message.application_id = None,
		Patch::Absent => {}
	}
	match &patch.content {
		Patch::Value(s) => message.content.clone_from(s),
		Patch::Null => message.content.clear(),
		Patch::Absent => {}
	}
	match &patch.edited {
		Patch::Value(at) => {
			message.edited = true;
			message.edited_at = Some(*at);
		}
		Patch::Null => {
			message.edited = false;
			message.edited_at = None;
		}
		Patch::Absent => {}
	}
	match &patch.embeds {
		Patch::Value(embeds) => message.embeds.clone_from(embeds),
		Patch::Null => message.embeds.clear(),
		Patch::Absent => {}
	}
	match &patch.attachments {
		Patch::Value(attachments) => message.attachments.clone_from(attachments),
		Patch::Null => message.attachments.clear(),
		Patch::Absent => {}
	}
	match patch.embeds_suppressed {
		Patch::Value(suppressed) => message.embeds_suppressed = suppressed,
		Patch::Null => message.embeds_suppressed = false,
		Patch::Absent => {}
	}
	patch.extra_content.apply(&mut message.extra_content);
	message.revision += 1;
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn forwarded_snapshot_survives_outer_body_updates() {
		let mut original = message(1);
		original.forwarded = true;
		original.embeds = vec![model::Embed {
			title: Some("Frozen embed".into()),
			..Default::default()
		}];
		let mut timeline = Timeline::default();
		timeline.insert(original.clone(), false, false).unwrap();
		timeline
			.patch(MessagePatch {
				flags: Patch::Absent,
				sticker_items: Patch::Absent,
				components: Patch::Absent,
				application_id: Patch::Absent,
				id: Id(1),
				channel: Id(1),
				content: Patch::Value(String::new()),
				mentions: Patch::Absent,
				reactions: Patch::Value(vec![]),
				edited: Patch::Absent,
				embeds: Patch::Null,
				attachments: Patch::Null,
				embeds_suppressed: Patch::Null,
				extra_content: Default::default(),
			})
			.unwrap();
		let updated = timeline.get(Id(1)).unwrap();
		assert!(updated.forwarded);
		assert_eq!(updated.content, original.content);
		assert_eq!(updated.embeds, original.embeds);
		assert!(updated.revision > original.revision);
	}
	#[test]
	fn deleted_reference_inference_preserves_kind_and_rejects_invalid_legacy_markers() {
		let mut timeline = Timeline::default();
		timeline.delete(Id(50)).unwrap();
		for (id, kind, target, expected) in [
			(100, 0, 50, false),
			(101, 19, 50, true),
			(102, 23, 50, true),
			(40, 19, 50, false),
		] {
			let mut source = message(id);
			source.kind = kind;
			source.reply_to = Some(Id(target));
			timeline.insert(source, false, false).unwrap();
			let loaded = timeline.get(Id(id)).unwrap();
			assert_eq!(loaded.kind, kind);
			assert_eq!(loaded.reply_deleted, expected);
			assert!(Timeline::valid_message(loaded));
		}
		let mut source = message(200);
		source.kind = 23;
		source.reply_to = Some(Id(60));
		timeline.insert(source.clone(), false, false).unwrap();
		source.reply_deleted = true;
		timeline.observe_deleted_reference(&source).unwrap();
		assert_eq!(timeline.get(Id(200)).unwrap().kind, 23);
		assert!(timeline.get(Id(200)).unwrap().reply_deleted);
		assert!(timeline.is_deleted(Id(60)));
	}

	#[test]
	fn explicit_deleted_references_remove_targets_in_both_arrival_orders() {
		for source_first in [false, true] {
			let mut timeline = Timeline::default();
			let mut source = message(100);
			source.reply_to = Some(Id(50));
			source.kind = 19;
			source.reply_deleted = true;
			if !source_first {
				timeline.insert(message(50), false, false).unwrap();
			}
			timeline.insert(source.clone(), false, false).unwrap();
			if source_first {
				timeline.insert(message(50), false, false).unwrap();
			}
			assert!(timeline.is_deleted(Id(50)));
			assert!(timeline.get(Id(50)).is_none());
			assert!(timeline.get(Id(100)).unwrap().reply_deleted);
			source.reply_deleted = false; // Older/partial service knowledge cannot undo deletion.
			timeline.insert(source, false, false).unwrap();
			assert!(timeline.get(Id(100)).unwrap().reply_deleted);
			timeline.clear_window_preserving_deletions();
			timeline.insert(message(50), false, false).unwrap();
			assert!(timeline.get(Id(50)).is_none());
		}
		let mut timeline = Timeline::default();
		let mut source = message(100);
		source.reply_to = Some(Id(50));
		timeline.insert(source.clone(), false, false).unwrap();
		let revision = timeline.get(Id(100)).unwrap().revision;
		source.kind = 19;
		source.reply_deleted = true;
		timeline.insert(source, false, false).unwrap();
		assert_eq!(timeline.get(Id(100)).unwrap().revision, revision + 1);
		for target in [None, Some(Id(0)), Some(Id(100)), Some(Id(101))] {
			let mut invalid = message(100);
			invalid.kind = 19;
			invalid.reply_deleted = true;
			invalid.reply_to = target;
			assert!(Timeline::default().insert(invalid, false, false).is_err());
		}
	}
	#[test]
	fn same_channel_range_reset_keeps_only_bounded_deletion_guards() {
		let mut timeline = Timeline::default();
		timeline.insert(message(100), false, false).unwrap();
		timeline.begin_page(false);
		for id in 1..=MAX_MUTATIONS as u64 {
			timeline.delete(Id(id)).unwrap();
		}
		let retained = timeline.retained_bytes();
		timeline.clear_window_preserving_deletions();
		assert_eq!(timeline.row_count(), 0);
		assert_eq!(timeline.bytes(), 0);
		assert!(timeline.retained_bytes() < retained);
		assert!(timeline.is_deleted(Id(100)));
		assert!(!timeline.is_deleted(Id(2000)));
		timeline.begin_page(true);
		timeline.finish_page(vec![message(100)], true).unwrap();
		assert_eq!(timeline.row_count(), 0);
		assert!(timeline.delete(Id(2000)).is_err()); // Reset does not defeat guard cap.
		timeline.clear();
		assert!(!timeline.is_deleted(Id(100)));
		timeline.insert(message(100), false, false).unwrap();
		assert_eq!(timeline.row_count(), 1);
	}
	#[test]
	fn retained_estimate_charges_rows_mutation_guards_and_pending_patch_capacity() {
		let mut timeline = Timeline::default();
		let empty = timeline.retained_bytes();
		timeline.insert(message(1), false, false).unwrap();
		assert!(timeline.retained_bytes() > empty + timeline.bytes());
		timeline.begin_page(false);
		timeline.delete(Id(2)).unwrap(); // A guard without a loaded row.
		let guarded = timeline.retained_bytes();
		assert_eq!(timeline.row_count(), 1);
		let mut content = String::with_capacity(8192);
		content.push_str("Pending patch");
		timeline
			.patch(MessagePatch {
				flags: Patch::Absent,
				sticker_items: Patch::Absent,
				components: Patch::Absent,
				application_id: Patch::Absent,
				id: Id(3),
				channel: Id(1),
				content: Patch::Value(content),
				extra_content: Default::default(),
				reactions: Patch::Absent,
				mentions: Patch::Absent,
				edited: Patch::Absent,
				embeds: Patch::Absent,
				attachments: Patch::Absent,
				embeds_suppressed: Patch::Absent,
			})
			.unwrap();
		assert!(timeline.retained_bytes() >= guarded + 8192);
		let pending = timeline.retained_bytes();
		timeline.cancel_page();
		assert!(timeline.retained_bytes() < pending);
		assert!(timeline.retained_bytes() > empty);
		assert!(timeline.deleted.contains(&Id(2)));
		timeline.clear();
		assert_eq!(timeline.retained_bytes(), empty);
	}
	#[test]
	fn deleted_rows_keep_payloads_reject_late_content_and_hide_from_get() {
		let mut timeline = Timeline::default();
		let mut loaded = message(10);
		loaded.content = "x".repeat(64 * 1024);
		loaded.author.name = "Synthetic author".repeat(20);
		loaded.mentions = vec![loaded.author.clone()];
		loaded.embeds = vec![model::Embed {
			title: Some("Synthetic embed".into()),
			..Default::default()
		}];
		loaded.attachments = vec![model::Attachment {
			duration_ms: None,
			waveform: Vec::new(),
			id: Id(20),
			filename: "SPOILER_synthetic.png".into(),
			description: Some("Synthetic attachment".into()),
			content_type: Some("image/png".into()),
			size: 100,
			spoiler: true,
			media: model::EmbedMedia {
				url: Some("https://cdn.discordapp.com/attachments/1/20/synthetic.png".into()),
				..Default::default()
			},
		}];
		timeline.insert(loaded, false, false).unwrap();
		let positions = timeline.row_ids().collect::<Vec<_>>();
		let retained_bytes = timeline.bytes();
		assert!(retained_bytes > 64 * 1024);
		timeline.begin_page(false);
		timeline.delete(Id(10)).unwrap();
		timeline.delete(Id(10)).unwrap();
		timeline.delete(Id(5)).unwrap(); // Never loaded: guard only, no fabricated row.
		assert_eq!(timeline.row_ids().collect::<Vec<_>>(), positions);
		assert_eq!(timeline.row_count(), 1);
		assert!(timeline.messages[&Id(10)].is_some());
		assert!(timeline.get_display(Id(10)).is_some());
		assert!(timeline.get(Id(10)).is_none());
		assert_eq!(timeline.len(), 0);
		assert!(timeline.is_empty());
		assert_eq!(timeline.iter().count(), 0);
		assert_eq!(timeline.display_iter().count(), 1);
		assert_eq!(timeline.bytes(), retained_bytes);
		timeline
			.patch(MessagePatch {
				flags: Patch::Absent,
				sticker_items: Patch::Absent,
				components: Patch::Absent,
				application_id: Patch::Absent,
				id: Id(10),
				channel: Id(1),
				content: Patch::Value("late body".into()),
				extra_content: Default::default(),
				reactions: Patch::Absent,
				mentions: Patch::Absent,
				edited: Patch::Absent,
				embeds: Patch::Absent,
				attachments: Patch::Absent,
				embeds_suppressed: Patch::Absent,
			})
			.unwrap();
		timeline
			.finish_page(vec![message(5), message(10)], false)
			.unwrap();
		timeline.insert(message(10), true, false).unwrap();
		timeline.insert(message(10), false, false).unwrap();
		timeline.set_reactions(Id(10), Some(Vec::new())).unwrap();
		assert_eq!(timeline.row_ids().collect::<Vec<_>>(), positions);
		assert_eq!(timeline.bytes(), retained_bytes);
		assert!(timeline.patches.is_empty());
		timeline.begin_page(true);
		timeline.finish_page(vec![message(4)], true).unwrap();
		assert_eq!(timeline.row_ids().collect::<Vec<_>>(), [Id(4), Id(10)]);
		timeline.begin_page(false);
		timeline
			.finish_page(vec![message(10), message(11)], false)
			.unwrap();
		assert_eq!(timeline.row_ids().collect::<Vec<_>>(), [Id(10), Id(11)]);
		assert!(timeline.get(Id(10)).is_none());
		assert!(timeline.get_display(Id(10)).is_some());
		timeline.clear();
		assert_eq!(timeline.row_count(), 0);
		timeline.insert(message(10), false, false).unwrap();
		assert_eq!(timeline.len(), 1);
	}

	#[test]
	fn deleted_rows_share_both_eviction_directions_and_byte_limits() {
		let mut timeline = Timeline::default();
		for id in 1001..=1500 {
			timeline.insert(message(id), false, false).unwrap();
		}
		timeline.delete(Id(1001)).unwrap();
		timeline.delete(Id(1500)).unwrap();
		assert_eq!(timeline.len(), 498);
		assert_eq!(timeline.row_count(), MAX_MESSAGES);
		timeline.insert(message(1501), true, false).unwrap();
		assert_eq!(timeline.row_ids().next(), Some(Id(1002)));
		assert_eq!(timeline.len(), 499);
		timeline.begin_page(true);
		timeline.finish_page(vec![message(1000)], true).unwrap();
		assert_eq!(timeline.row_ids().next_back(), Some(Id(1500)));
		assert!(timeline.get(Id(1500)).is_none());
		timeline.begin_page(true);
		timeline.finish_page(vec![message(999)], true).unwrap();
		assert_eq!(timeline.row_ids().next_back(), Some(Id(1499)));
		assert_eq!(timeline.len(), MAX_MESSAGES);
		timeline.insert(message(1500), false, false).unwrap();
		assert_eq!(timeline.row_ids().next_back(), Some(Id(1499)));
		timeline.begin_page(false);
		timeline.cancel_page();
		for id in 2000..2100 {
			let mut large = message(id);
			large.content = "x".repeat(64 * 1024);
			timeline.insert(large, true, false).unwrap();
			if id % 3 == 0 {
				timeline.delete(Id(id)).unwrap();
			}
			assert!(timeline.row_count() <= MAX_MESSAGES);
			assert!(timeline.row_bytes() <= MAX_BYTES);
			assert_eq!(timeline.len(), timeline.iter().count());
		}
		assert!(timeline.row_count() < MAX_MESSAGES);
		assert!(timeline.row_count() > timeline.len());
	}

	#[test]
	fn unknown_deletions_have_no_rows_and_keep_the_reconciliation_cap() {
		let mut timeline = Timeline::default();
		timeline.insert(message(5000), false, false).unwrap();
		for id in 1..=MAX_MUTATIONS as u64 {
			timeline.delete(Id(id)).unwrap();
		}
		assert_eq!(timeline.row_ids().collect::<Vec<_>>(), [Id(5000)]);
		assert_eq!(timeline.deleted.len(), MAX_MUTATIONS);
		timeline.delete(Id(1)).unwrap();
		assert!(timeline.delete(Id(5000)).is_err());
		assert!(timeline.get(Id(5000)).is_some());
		timeline.clear();
		timeline.insert(message(5000), false, false).unwrap();
		timeline.delete(Id(5000)).unwrap();
		assert_eq!(timeline.row_count(), 1);
		assert!(timeline.is_empty());
	}
	#[test]
	fn content_markers_reconcile_independent_updates_before_and_after_history() {
		let update = |extra_content| MessagePatch {
			flags: Patch::Absent,
			sticker_items: Patch::Absent,
			components: Patch::Absent,
			application_id: Patch::Absent,
			id: Id(1),
			channel: Id(1),
			extra_content,
			reactions: Patch::Absent,
			content: Patch::Absent,
			mentions: Patch::Absent,
			edited: Patch::Absent,
			embeds: Patch::Absent,
			embeds_suppressed: Patch::Absent,
			attachments: Patch::Absent,
		};
		let sticker = model::Sticker {
			id: Id(90),
			name: "Wave".into(),
			description: String::new(),
			tags: String::new(),
			format_type: 1,
			guild_id: None,
			pack_id: None,
			available: true,
		};
		let mut sticker_timeline = Timeline::default();
		sticker_timeline.begin_page(false);
		let mut sticker_patch = update(Default::default());
		sticker_patch.sticker_items = Patch::Value(vec![sticker.clone()]);
		sticker_timeline.patch(sticker_patch).unwrap();
		sticker_timeline.patch(update(Default::default())).unwrap();
		sticker_timeline
			.finish_page(vec![message(1)], false)
			.unwrap();
		assert_eq!(
			sticker_timeline.get(Id(1)).unwrap().sticker_items,
			vec![sticker]
		);
		let mut clear = update(Default::default());
		clear.sticker_items = Patch::Null;
		sticker_timeline.patch(clear).unwrap();
		assert!(
			sticker_timeline
				.get(Id(1))
				.unwrap()
				.sticker_items
				.is_empty()
		);
		let mut original = message(1);
		original.extra_content.sticker_items = true;
		original.extra_content.poll = true;
		let mut timeline = Timeline::default();
		timeline.begin_page(false);
		timeline
			.patch(update(model::ExtraContentPatch {
				components: Patch::Value(true),
				..Default::default()
			}))
			.unwrap();
		timeline
			.patch(update(model::ExtraContentPatch {
				poll: Patch::Null,
				components_v2: Patch::Value(true),
				..Default::default()
			}))
			.unwrap();
		timeline
			.patch(update(model::ExtraContentPatch {
				components: Patch::Value(false),
				..Default::default()
			}))
			.unwrap();
		timeline.finish_page(vec![original.clone()], false).unwrap();
		let current = timeline.get(Id(1)).unwrap();
		assert!(!current.extra_content.poll && !current.extra_content.components);
		assert!(current.extra_content.sticker_items && current.extra_content.components_v2);

		timeline.begin_page(false);
		timeline
			.patch(update(model::ExtraContentPatch {
				sticker_items: Patch::Null,
				stickers: Patch::Value(true),
				..Default::default()
			}))
			.unwrap();
		timeline
			.patch(update(model::ExtraContentPatch {
				components_v2: Patch::Null,
				..Default::default()
			}))
			.unwrap();
		timeline.finish_page(vec![original], false).unwrap();
		let current = timeline.get(Id(1)).unwrap();
		assert_eq!(
			current.extra_content,
			model::ExtraContent {
				stickers: true,
				..Default::default()
			}
		);
		let mut cleared = update(model::ExtraContentPatch {
			stickers: Patch::Null,
			..Default::default()
		});
		cleared.edited = Patch::Value(10);
		timeline.patch(cleared).unwrap();
		let mut stale = update(model::ExtraContentPatch {
			poll: Patch::Value(true),
			..Default::default()
		});
		stale.edited = Patch::Value(9);
		timeline.patch(stale).unwrap();
		assert!(!timeline.get(Id(1)).unwrap().extra_content.any());

		let mut replacement = timeline.get(Id(1)).unwrap().clone();
		let before = replacement.revision;
		replacement.extra_content.components = true;
		timeline.insert(replacement, true, false).unwrap();
		assert_eq!(timeline.get(Id(1)).unwrap().revision, before + 1);
		let mut replacement = timeline.get(Id(1)).unwrap().clone();
		replacement.unsupported = true;
		timeline.insert(replacement.clone(), true, false).unwrap();
		assert_eq!(timeline.get(Id(1)).unwrap().revision, before + 2);
		timeline.begin_page(false);
		timeline.delete(Id(1)).unwrap();
		timeline
			.patch(update(model::ExtraContentPatch {
				poll: Patch::Value(true),
				..Default::default()
			}))
			.unwrap();
		timeline.finish_page(vec![replacement], false).unwrap();
		assert!(timeline.is_empty());
		assert!(timeline.get_display(Id(1)).is_some());
		assert!(timeline.bytes() > 0);
	}

	#[test]
	fn mention_patches_survive_stale_pages_and_release_replaced_users() {
		let mut timeline = Timeline::default();
		let mut original = message(1);
		original.mentions = vec![original.author.clone()];
		timeline.insert(original.clone(), false, false).unwrap();
		let before = timeline.bytes;
		let patch = MessagePatch {
			flags: Patch::Absent,
			sticker_items: Patch::Absent,
			components: Patch::Absent,
			application_id: Patch::Absent,
			extra_content: Default::default(),
			reactions: model::Patch::Absent,
			id: Id(1),
			channel: original.channel,
			content: Patch::Absent,
			mentions: Patch::Value(vec![]),
			edited: Patch::Absent,
			embeds: Patch::Absent,
			embeds_suppressed: Patch::Absent,
			attachments: Patch::Absent,
		};
		timeline.patch(patch.clone()).unwrap();
		assert!(timeline.get(Id(1)).unwrap().mentions.is_empty());
		assert!(timeline.bytes < before);
		timeline.clear();
		timeline.begin_page(false);
		timeline.patch(patch).unwrap();
		timeline.finish_page(vec![original], false).unwrap();
		assert!(timeline.get(Id(1)).unwrap().mentions.is_empty());
	}
	fn message(id: u64) -> Message {
		Message {
			flags: 0,
			sticker_items: vec![],
			components: vec![],
			application_id: None,
			ephemeral: false,
			reactions: Some(vec![]),
			id: Id(id),
			channel: Id(1),
			author: model::User {
				primary_guild: None,
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				id: Id(2),
				name: "Synthetic".into(),
			},
			content: "before".into(),
			prior_contents: Default::default(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			attachments: Vec::new(),
			embeds: Vec::new(),
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: Vec::new(),
			embeds_suppressed: false,
		}
	}
	#[test]
	fn pending_and_cancelled_older_pages_preserve_the_reading_window() {
		let mut timeline = Timeline::default();
		for id in 1..=MAX_MESSAGES as u64 {
			timeline.insert(message(id), false, false).unwrap();
		}
		timeline.begin_page(true);
		timeline.insert(message(501), true, false).unwrap();
		assert!(timeline.get(Id(1)).is_some());
		assert!(timeline.get(Id(501)).is_none());
		timeline.cancel_page(); // Failed/queue-rejected page does not change reading position.
		timeline.insert(message(502), true, false).unwrap();
		assert!(timeline.get(Id(1)).is_some());
		assert!(timeline.get(Id(502)).is_none());
		timeline.begin_page(true);
		timeline.finish_page(vec![message(0)], true).unwrap();
		timeline.begin_page(true); // A further older-page failure preserves that older window.
		timeline.cancel_page();
		timeline.insert(message(503), true, false).unwrap();
		assert!(timeline.get(Id(0)).is_some());
		assert!(timeline.get(Id(503)).is_none());
		timeline.begin_page(false); // Jump/reload latest resets retention immediately.
		timeline.insert(message(504), true, false).unwrap();
		assert!(timeline.get(Id(0)).is_none());
		assert!(timeline.get(Id(504)).is_some());
		timeline.cancel_page();
		timeline.insert(message(505), true, false).unwrap();
		assert!(timeline.get(Id(505)).is_some());
		assert_eq!(timeline.len(), MAX_MESSAGES);
		assert!(timeline.bytes() <= MAX_BYTES);
	}

	#[test]
	fn attachment_only_mutations_clear_without_late_history_resurrection() {
		let attachment = |id| model::Attachment {
			duration_ms: None,
			waveform: Vec::new(),
			id: Id(id),
			filename: "synthetic.png".into(),
			description: None,
			content_type: Some("image/png".into()),
			size: 1024,
			media: model::EmbedMedia {
				url: Some(format!(
					"https://cdn.discordapp.com/attachments/1/{id}/synthetic.png"
				)),
				width: 640,
				height: 480,
				..Default::default()
			},
			spoiler: false,
		};
		let update = |attachments| MessagePatch {
			flags: Patch::Absent,
			sticker_items: Patch::Absent,
			components: Patch::Absent,
			application_id: Patch::Absent,
			extra_content: Default::default(),
			reactions: model::Patch::Absent,
			id: Id(1),
			channel: Id(1),
			content: Patch::Absent,
			edited: Patch::Absent,
			embeds: Patch::Absent,
			mentions: Patch::Absent,
			embeds_suppressed: Patch::Absent,
			attachments,
		};
		let mut timeline = Timeline::default();
		timeline.begin_page(false);
		timeline
			.patch(update(Patch::Value(vec![attachment(10)])))
			.unwrap();
		timeline.patch(update(Patch::Absent)).unwrap();
		timeline.insert(message(1), true, false).unwrap();
		assert_eq!(timeline.get(Id(1)).unwrap().attachments[0].id, Id(10));
		let revision = timeline.get(Id(1)).unwrap().revision;
		timeline
			.patch(update(Patch::Value(vec![attachment(11)])))
			.unwrap();
		timeline.finish_page(vec![message(1)], false).unwrap();
		assert_eq!(timeline.get(Id(1)).unwrap().attachments[0].id, Id(11));
		assert!(timeline.get(Id(1)).unwrap().revision > revision);
		for clear in [Patch::Null, Patch::Value(Vec::new())] {
			timeline.begin_page(false);
			let mut old = message(1);
			old.attachments = vec![attachment(10)];
			timeline.patch(update(clear)).unwrap();
			timeline.finish_page(vec![old], false).unwrap();
			assert!(timeline.get(Id(1)).unwrap().attachments.is_empty());
			assert_eq!(
				timeline.bytes(),
				timeline.iter().map(Message::bytes).sum::<usize>()
			);
		}
		timeline.begin_page(false);
		timeline.delete(Id(1)).unwrap();
		timeline
			.patch(update(Patch::Value(vec![attachment(12)])))
			.unwrap();
		timeline.finish_page(vec![message(1)], false).unwrap();
		assert!(timeline.is_empty());
		assert!(timeline.get_display(Id(1)).is_some());
		timeline.clear();
		timeline.begin_page(false);
		let mut large = attachment(10);
		large.media.url = Some(format!("https://cdn.discordapp.com/{}", "x".repeat(1900)));
		large.media.proxy_url = large.media.url.clone();
		let mut rejected = false;
		for id in 1..100 {
			let mut patch = update(Patch::Value(vec![large.clone(); model::MAX_ATTACHMENTS]));
			patch.id = Id(id);
			if timeline.patch(patch).is_err() {
				rejected = true;
				break;
			}
		}
		assert!(
			rejected,
			"Pending attachments share the one MiB mutation budget"
		);
		assert!(
			!timeline.patches.is_empty(),
			"The budget test uses valid attachments"
		);
	}
	#[test]
	fn embed_only_mutations_merge_clear_and_survive_late_history() {
		let embed = |title: &str| model::Embed {
			title: Some(title.into()),
			..Default::default()
		};
		let update = |embeds| MessagePatch {
			flags: Patch::Absent,
			sticker_items: Patch::Absent,
			components: Patch::Absent,
			application_id: Patch::Absent,
			extra_content: Default::default(),
			reactions: model::Patch::Absent,
			id: Id(1),
			channel: Id(1),
			content: Patch::Absent,
			edited: Patch::Absent,
			embeds,
			mentions: Patch::Absent,
			embeds_suppressed: Patch::Absent,
			attachments: Patch::Absent,
		};
		let mut timeline = Timeline::default();
		timeline.begin_page(false);
		timeline
			.patch(update(Patch::Value(vec![embed("first")])))
			.unwrap();
		let mut suppression = update(Patch::Absent);
		suppression.embeds_suppressed = Patch::Value(true);
		timeline.patch(suppression).unwrap();
		timeline.insert(message(1), true, false).unwrap();
		assert_eq!(
			timeline.get(Id(1)).unwrap().embeds[0].title.as_deref(),
			Some("first")
		);
		assert!(timeline.get(Id(1)).unwrap().embeds_suppressed);
		let revision = timeline.get(Id(1)).unwrap().revision;
		timeline
			.patch(update(Patch::Value(vec![embed("newer")])))
			.unwrap();
		timeline.finish_page(vec![message(1)], false).unwrap();
		assert_eq!(
			timeline.get(Id(1)).unwrap().embeds[0].title.as_deref(),
			Some("newer")
		);
		assert!(timeline.get(Id(1)).unwrap().revision > revision);
		for clear in [Patch::Null, Patch::Value(Vec::new())] {
			timeline.begin_page(false);
			let mut old = message(1);
			old.embeds = vec![embed("stale")];
			timeline.patch(update(clear)).unwrap();
			timeline.finish_page(vec![old], false).unwrap();
			assert!(timeline.get(Id(1)).unwrap().embeds.is_empty());
			assert_eq!(
				timeline.bytes(),
				timeline.iter().map(Message::bytes).sum::<usize>()
			);
		}
		timeline.clear();
		timeline.begin_page(false);
		let large = model::Embed {
			description: Some("x".repeat(16_384)),
			..Default::default()
		};
		let mut rejected = false;
		for id in 1..100 {
			let mut patch = update(Patch::Value(vec![large.clone()]));
			patch.id = Id(id);
			if timeline.patch(patch).is_err() {
				rejected = true;
				break;
			}
		}
		assert!(
			rejected,
			"Pending embeds must share the one MiB patch budget"
		);
	}
	#[test]
	fn mutations_win_over_late_history_and_memory_is_bounded() {
		let mut t = Timeline::default();
		t.begin_page(false);
		t.delete(Id(1)).unwrap();
		t.patch(MessagePatch {
			flags: Patch::Absent,
			sticker_items: Patch::Absent,
			components: Patch::Absent,
			application_id: Patch::Absent,
			extra_content: Default::default(),
			reactions: model::Patch::Absent,
			id: Id(2),
			channel: Id(1),
			content: Patch::Value("after".into()),
			edited: Patch::Value(1),
			embeds: Patch::Absent,
			mentions: Patch::Absent,
			embeds_suppressed: Patch::Absent,
			attachments: Patch::Absent,
		})
		.unwrap();
		t.finish_page(vec![message(1), message(2)], false).unwrap();
		assert!(t.get(Id(1)).is_none());
		assert!(t.get_display(Id(1)).is_none());
		t.patch(MessagePatch {
			flags: Patch::Absent,
			sticker_items: Patch::Absent,
			components: Patch::Absent,
			application_id: Patch::Absent,
			extra_content: Default::default(),
			reactions: model::Patch::Absent,
			id: Id(1),
			channel: Id(1),
			content: Patch::Value("late edit".into()),
			edited: Patch::Absent,
			embeds: Patch::Absent,
			mentions: Patch::Absent,
			embeds_suppressed: Patch::Absent,
			attachments: Patch::Absent,
		})
		.unwrap();
		t.insert(message(1), false, false).unwrap();
		assert!(t.get(Id(1)).is_none());
		assert_eq!(t.get(Id(2)).unwrap().content, "after");
		let mut old = message(2);
		old.edited = true;
		old.edited_at = Some(0);
		t.insert(old, true, false).unwrap();
		assert_eq!(t.get(Id(2)).unwrap().content, "after");
		for id in 3..10_000 {
			t.insert(message(id), true, false).unwrap();
		}
		assert_eq!(t.len(), MAX_MESSAGES);
		assert!(t.bytes() <= MAX_BYTES);
		t.clear();
		assert_eq!(t.bytes(), 0);
		t.begin_page(false);
		t.seed_cache(vec![message(1), message(2)]).unwrap();
		t.finish_page(vec![message(2)], false).unwrap();
		assert!(t.get(Id(1)).is_none());
	}
	#[test]
	fn earlier_window_stays_put_and_out_of_order_pending_edits_do_not_regress() {
		let mut timeline = Timeline::default();
		for id in 1001..=1500 {
			timeline.insert(message(id), false, false).unwrap();
		}
		timeline.begin_page(true);
		timeline
			.finish_page((951..=1000).map(message).collect(), true)
			.unwrap();
		assert_eq!(timeline.iter().next().unwrap().id, Id(951));
		assert_eq!(timeline.iter().next_back().unwrap().id, Id(1450));
		timeline.insert(message(1501), true, false).unwrap();
		assert_eq!(timeline.iter().next().unwrap().id, Id(951));
		assert!(timeline.get(Id(1501)).is_none()); // live arrivals do not evict the reading anchor
		assert_eq!(timeline.len(), MAX_MESSAGES);

		timeline.clear();
		timeline.begin_page(false);
		for (at, content) in [(20, "new edit"), (10, "old edit")] {
			timeline
				.patch(MessagePatch {
					flags: Patch::Absent,
					sticker_items: Patch::Absent,
					components: Patch::Absent,
					application_id: Patch::Absent,
					extra_content: Default::default(),
					reactions: model::Patch::Absent,
					id: Id(1),
					channel: Id(1),
					content: Patch::Value(content.into()),
					edited: Patch::Value(at),
					embeds: Patch::Absent,
					mentions: Patch::Absent,
					embeds_suppressed: Patch::Absent,
					attachments: Patch::Absent,
				})
				.unwrap();
		}
		timeline.insert(message(1), true, false).unwrap();
		assert_eq!(timeline.get(Id(1)).unwrap().content, "new edit");
		timeline.finish_page(vec![message(1)], false).unwrap();
		assert_eq!(timeline.get(Id(1)).unwrap().edited_at, Some(20));

		timeline.begin_page(false);
		timeline.delete(Id(1)).unwrap();
		timeline.cancel_page();
		timeline.begin_page(false);
		timeline
			.finish_page(vec![message(1), message(2)], false)
			.unwrap();
		assert!(timeline.get(Id(1)).is_none());
		assert_eq!(timeline.get(Id(2)).unwrap().content, "before");
	}
}
