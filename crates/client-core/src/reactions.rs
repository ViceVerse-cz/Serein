use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Freshness, Id, Reaction, ReactionEmoji};
use std::collections::BTreeSet;

pub enum Command {
	Read {
		channel: Id,
		message: Id,
		request: u64,
	},
	Set {
		channel: Id,
		message: Id,
		emoji: ReactionEmoji,
		add: bool,
		request: u64,
	},
}
pub enum Event {
	Delta {
		channel: Id,
		message: Id,
		user: Id,
		emoji: ReactionEmoji,
		add: bool,
		burst: bool,
	},
	Cleared {
		channel: Id,
		message: Id,
		emoji: Option<ReactionEmoji>,
	},
	Changed {
		channel: Id,
		message: Id,
	},
	Read {
		channel: Id,
		message: Id,
		request: u64,
		result: Result<Vec<Reaction>, Failure>,
	},
	Written {
		channel: Id,
		message: Id,
		request: u64,
		result: Result<(), Failure>,
	},
}
#[derive(Default)]
pub struct Reactions {
	dirty: BTreeSet<Id>,
	read: Option<(Id, u64)>,
	pub writing: Option<(Id, u64)>,
	// One bounded reaction list before/after the single in-flight write.
	preview: Option<(Id, Vec<Reaction>, Vec<Reaction>)>,
	sequence: u64,
}
impl Reactions {
	pub fn cancel_read(&mut self) {
		if let Some((message, _)) = self.read.take() {
			self.dirty.insert(message);
		}
	}
	pub fn reset(&mut self) {
		self.dirty.clear();
		self.read = None;
		self.writing = None;
		self.preview = None;
		self.sequence = self.sequence.wrapping_add(1);
	}
	pub fn busy(&self) -> bool {
		self.writing.is_some() || self.preview.is_some()
	}
	pub fn display<'a>(&'a self, message: &'a model::Message) -> Option<&'a [Reaction]> {
		self.preview
			.as_ref()
			.filter(|(id, _, _)| *id == message.id)
			.map(|(_, _, after)| after.as_slice())
			.or(message.reactions.as_deref())
	}
	pub fn invalidated(&self, id: Id) -> bool {
		self.dirty.contains(&id) || self.read.is_some_and(|(message, _)| message == id)
	}
}
impl State {
	fn update_reactions(
		&mut self,
		channel: Id,
		message: Id,
		update: impl Fn(&mut Vec<Reaction>) -> Result<(), &'static str>,
	) -> Result<(), &'static str> {
		if self.selected != Some(channel)
			|| !self.can_view(channel)
			|| self.freshness == Freshness::Unavailable
			|| !self.gateway_connected
		{
			return Ok(());
		}
		let preview = self
			.reactions
			.preview
			.as_ref()
			.filter(|(id, _, _)| *id == message);
		let known = self
			.timeline
			.get(message)
			.and_then(|m| m.reactions.as_ref())
			.or_else(|| preview.map(|(_, before, _)| before));
		let Some(mut values) = known.cloned() else {
			// A delta cannot reconstruct a missing snapshot. Keep the bounded readback.
			self.refresh_reactions(message);
			return Ok(());
		};
		update(&mut values)?;
		let mut preview = preview.cloned();
		if let Some((_, before, after)) = &mut preview {
			update(before)?;
			update(after)?;
		}
		// Keep the existing coalesced verification: a snapshot can already include a
		// delayed Gateway event. Paint the delta now, never an empty loading row.
		if !self.queue_reaction_read(message) {
			return Ok(());
		}
		self.timeline.set_reactions(message, Some(values))?;
		if let Some(preview) = preview {
			self.reactions.preview = Some(preview);
		}
		self.revision += 1;
		Ok(())
	}
	pub fn refresh_reactions(&mut self, message: Id) {
		if self.queue_reaction_read(message) {
			let _ = self.timeline.set_reactions(message, None);
			self.revision += 1;
		}
	}
	pub(super) fn queue_reaction_read(&mut self, message: Id) -> bool {
		if self.timeline.get(message).is_none() && !self.history_pending {
			return false;
		}
		if self.reactions.dirty.len() >= session_cache::MAX_MUTATIONS
			&& !self.reactions.dirty.contains(&message)
		{
			self.fail(Failure::Capacity);
			return false;
		}
		self.reactions.dirty.insert(message);
		true
	}
	pub fn next_reaction_read(&mut self) -> Option<crate::Command> {
		if self.reactions.writing.is_none()
			&& self
				.reactions
				.preview
				.as_ref()
				.is_some_and(|(id, _, _)| self.timeline.get(*id).is_none())
		{
			self.reactions.preview = None;
		}
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.freshness != Freshness::Fresh
			|| self.history_pending
			|| self.reactions.read.is_some()
			|| self.reactions.writing.is_some()
		{
			return None;
		}
		let channel = self.selected?;
		if !self.can_read_history(channel) {
			return None;
		}
		self.reactions
			.dirty
			.retain(|id| self.timeline.get(*id).is_some());
		let message = self.reactions.dirty.pop_first()?;
		self.reactions.sequence = self.reactions.sequence.wrapping_add(1);
		let request = self.reactions.sequence;
		self.reactions.read = Some((message, request));
		Some(crate::Command::Reactions(Command::Read {
			channel,
			message,
			request,
		}))
	}
	pub fn prepare_reaction(
		&mut self,
		message: Id,
		emoji: ReactionEmoji,
	) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.freshness != Freshness::Fresh
			|| self.reactions.writing.is_some()
			|| self.reactions.preview.is_some()
			|| !emoji.valid()
			|| emoji.name.is_none()
		{
			return None;
		}
		let channel = self.selected?;
		let reactions = self.timeline.get(message)?.reactions.as_ref()?;
		let existing = reactions.iter().find(|r| r.emoji.same(&emoji));
		if existing.is_none() && reactions.len() >= model::MAX_REACTIONS {
			self.status = "Reaction limit reached";
			return None;
		}
		let add = existing.is_none_or(|r| !r.me);
		if !self.can_react(message, Some(&emoji), add) {
			return None;
		}
		let before = reactions.clone();
		let mut after = before.clone();
		if let Some(reaction) = after.iter_mut().find(|r| r.emoji.same(&emoji)) {
			reaction.count = if add {
				reaction.count.checked_add(1)?
			} else {
				reaction.count.saturating_sub(1)
			};
			reaction.me = add;
		} else {
			after.push(Reaction {
				emoji: emoji.clone(),
				count: 1,
				me: true,
				me_burst: false,
			});
		}
		after.retain(|r| r.count > 0);
		self.reactions.cancel_read();
		self.reactions.preview = Some((message, before, after));
		self.revision += 1;
		self.reactions.sequence = self.reactions.sequence.wrapping_add(1);
		let request = self.reactions.sequence;
		self.reactions.writing = Some((message, request));
		Some(crate::Command::Reactions(Command::Set {
			channel,
			message,
			emoji,
			add,
			request,
		}))
	}
	pub fn apply_reactions(&mut self, event: Event) -> Result<(), &'static str> {
		match event {
			Event::Delta {
				channel,
				message,
				user,
				emoji,
				add,
				burst,
			} => {
				if !emoji.valid() || user.0 == 0 {
					return Err("Invalid reaction delta");
				}
				let own = self.user.as_ref().is_some_and(|me| me.id == user);
				self.update_reactions(channel, message, |values| {
					apply_delta(values, &emoji, own, add, burst)
				})?;
			}
			Event::Cleared {
				channel,
				message,
				emoji,
			} => {
				if emoji.as_ref().is_some_and(|e| !e.valid()) {
					return Err("Invalid reaction emoji");
				}
				self.update_reactions(channel, message, |values| {
					values.retain(|r| emoji.as_ref().is_some_and(|e| !r.emoji.same(e)));
					Ok(())
				})?;
			}
			Event::Changed { channel, message } => {
				if self.selected == Some(channel)
					&& self.can_view(channel)
					&& self.freshness != Freshness::Unavailable
					&& self.gateway_connected
				{
					self.refresh_reactions(message);
				}
			}
			Event::Read {
				channel,
				message,
				request,
				result,
			} => {
				if self.selected != Some(channel) || self.reactions.read != Some((message, request))
				{
					return Ok(());
				}
				self.reactions.read = None;
				if !self.gateway_connected
					|| self.freshness == Freshness::Unavailable
					|| !self.can_read_history(channel)
				{
					return Ok(());
				}
				match result {
					Ok(reactions) if !self.reactions.dirty.contains(&message) => {
						self.timeline.set_reactions(message, Some(reactions))?;
						if self
							.reactions
							.preview
							.as_ref()
							.is_some_and(|(id, _, _)| *id == message)
						{
							self.reactions.preview = None;
						}
					}
					Ok(_) => {} // A newer invalidation schedules one fresh read, never an old snapshot.
					Err(failure) => {
						if self
							.reactions
							.preview
							.as_ref()
							.is_some_and(|(id, _, _)| *id == message)
							&& let Some((_, _, after)) = self.reactions.preview.take()
						{
							self.timeline.set_reactions(message, Some(after))?;
						}
						self.reactions.dirty.remove(&message); // No retry storm on a rejected read.
						self.status = if self
							.timeline
							.get(message)
							.is_some_and(|m| m.reactions.is_some())
						{
							"Reaction counts could not be refreshed; reload the conversation to retry"
						} else {
							"Reactions unavailable; use Reload reactions to retry"
						};
						if failure == Failure::Forbidden {
							self.apply(crate::Envelope {
								generation: self.generation,
								event: crate::Event::Unavailable(channel),
							});
						}
						if failure.ends_session() {
							self.fail(failure);
						}
					}
				}
			}
			Event::Written {
				channel,
				message,
				request,
				result,
			} => {
				if self.selected != Some(channel)
					|| self.reactions.writing != Some((message, request))
				{
					return Ok(());
				}
				self.reactions.writing = None;
				match result {
					Ok(()) => {
						self.refresh_reactions(message);
					}
					Err(failure) => {
						let before = self.reactions.preview.take().map(|(_, before, _)| before);
						// A timed-out write may have succeeded. Read back; never repeat the write.
						if failure == Failure::Ambiguous {
							self.refresh_reactions(message);
						}
						if let Some(before) = before
							&& self
								.timeline
								.get(message)
								.is_some_and(|m| m.reactions.is_none())
						{
							self.timeline.set_reactions(message, Some(before))?;
						}
						self.revision += 1;
						self.status = failure.label();
						if failure.ends_session() {
							self.fail(failure);
						}
					}
				}
			}
		}
		Ok(())
	}
}

fn apply_delta(
	values: &mut Vec<Reaction>,
	emoji: &ReactionEmoji,
	own: bool,
	add: bool,
	burst: bool,
) -> Result<(), &'static str> {
	if let Some(reaction) = values.iter_mut().find(|r| r.emoji.same(emoji)) {
		let me = if burst {
			&mut reaction.me_burst
		} else {
			&mut reaction.me
		};
		// Own Gateway echoes must not double-apply the optimistic toggle or a snapshot.
		if own && *me == add {
			return Ok(());
		}
		reaction.count = if add {
			reaction
				.count
				.checked_add(1)
				.ok_or("Reaction count exceeds capacity")?
		} else {
			reaction.count.saturating_sub(1)
		};
		if own {
			*me = add;
		}
	} else if add {
		if values.len() >= model::MAX_REACTIONS {
			return Err("Reaction data exceeds safe capacity");
		}
		values.push(Reaction {
			emoji: emoji.clone(),
			count: 1,
			me: own && !burst,
			me_burst: own && burst,
		});
	}
	values.retain(|r| r.count > 0);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::{Channel, Message, User, permissions as p};

	#[test]
	fn live_reaction_deltas_preserve_counts_echoes_and_history_races() {
		let message = crate::tests::message(10);
		let channel = message.channel;
		let id = message.id;
		let own = message.author.id;
		let emoji = ReactionEmoji {
			id: None,
			name: Some("x".into()),
		};
		let other = ReactionEmoji {
			id: Some(Id(99)),
			name: None,
		};
		let mut state = State {
			selected: Some(channel),
			user: Some(message.author.clone()),
			channels: vec![Channel {
				id: channel,
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			gateway_connected: true,
			auth: AuthState::Authenticated,
			freshness: Freshness::Fresh,
			..State::default()
		};
		state
			.timeline
			.insert(message.clone(), false, false)
			.unwrap();
		let delta = |user, emoji: &ReactionEmoji, add, burst| Event::Delta {
			channel,
			message: id,
			user,
			emoji: emoji.clone(),
			add,
			burst,
		};
		let visible = |s: &State| {
			s.reactions
				.display(s.timeline.get(id).unwrap())
				.unwrap()
				.to_vec()
		};
		state
			.apply_reactions(delta(Id(500), &emoji, true, false))
			.unwrap();
		state
			.apply_reactions(delta(Id(501), &emoji, true, true))
			.unwrap();
		state
			.apply_reactions(delta(Id(500), &other, true, false))
			.unwrap();
		assert_eq!(
			visible(&state).iter().map(|r| r.count).collect::<Vec<_>>(),
			[2, 1]
		);
		assert!(state.reactions.invalidated(id));
		// Another user and our own echo update both sides of the optimistic write.
		let Some(crate::Command::Reactions(Command::Set { request, .. })) =
			state.prepare_reaction(id, emoji.clone())
		else {
			panic!()
		};
		state
			.apply_reactions(delta(Id(502), &emoji, true, false))
			.unwrap();
		assert_eq!(visible(&state)[0].count, 4);
		state
			.apply_reactions(delta(own, &emoji, true, false))
			.unwrap();
		assert_eq!(visible(&state)[0].count, 4);
		assert!(visible(&state)[0].me);
		state
			.apply_reactions(Event::Written {
				channel,
				message: id,
				request,
				result: Err(Failure::RateLimited),
			})
			.unwrap();
		assert_eq!(
			visible(&state)[0].count,
			4,
			"an observed echo survives a late write error"
		);
		// Normal and burst membership are independent, including optimistic removal.
		state
			.apply_reactions(delta(own, &emoji, true, true))
			.unwrap();
		let Some(crate::Command::Reactions(Command::Set {
			request,
			add: false,
			..
		})) = state.prepare_reaction(id, emoji.clone())
		else {
			panic!()
		};
		state
			.apply_reactions(delta(own, &emoji, false, false))
			.unwrap();
		assert_eq!(visible(&state)[0].count, 4);
		assert!(!visible(&state)[0].me && visible(&state)[0].me_burst);
		state
			.apply_reactions(Event::Written {
				channel,
				message: id,
				request,
				result: Ok(()),
			})
			.unwrap();
		let Some(crate::Command::Reactions(Command::Read { request, .. })) =
			state.next_reaction_read()
		else {
			panic!()
		};
		state
			.apply_reactions(delta(Id(503), &emoji, true, false))
			.unwrap();
		assert_eq!(visible(&state)[0].count, 5);
		state
			.apply_reactions(Event::Read {
				channel,
				message: id,
				request,
				result: Ok(vec![]),
			})
			.unwrap();
		assert_eq!(
			visible(&state)[0].count,
			5,
			"stale HTTP reply cannot undo a live update"
		);
		let Some(crate::Command::Reactions(Command::Read { request, .. })) =
			state.next_reaction_read()
		else {
			panic!()
		};
		state
			.apply_reactions(Event::Read {
				channel,
				message: id,
				request,
				result: Ok(visible(&state)),
			})
			.unwrap();
		let _ = state.history(None);
		state
			.apply_reactions(delta(Id(504), &emoji, true, false))
			.unwrap();
		let mut history_message = message;
		history_message.content = "Newer history content".into();
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::History {
				channel,
				request: state.request,
				older: false,
				messages: vec![history_message],
			},
		});
		assert_eq!(
			visible(&state)[0].count,
			6,
			"history must preserve a newer live delta"
		);
		assert_eq!(
			state.timeline.get(id).unwrap().content,
			"Newer history content"
		);
		let Some(crate::Command::Reactions(Command::Read { request, .. })) =
			state.next_reaction_read()
		else {
			panic!()
		};
		state
			.apply_reactions(Event::Read {
				channel,
				message: id,
				request,
				result: Ok(visible(&state)),
			})
			.unwrap();
		state
			.apply_reactions(Event::Cleared {
				channel,
				message: id,
				emoji: Some(emoji),
			})
			.unwrap();
		assert_eq!(visible(&state).len(), 1);
		state
			.apply_reactions(delta(Id(500), &other, false, false))
			.unwrap();
		assert!(visible(&state).is_empty());
		state
			.apply_reactions(delta(Id(500), &other, true, false))
			.unwrap();
		state
			.apply_reactions(Event::Cleared {
				channel,
				message: id,
				emoji: None,
			})
			.unwrap();
		assert!(visible(&state).is_empty());
		assert!(state.reactions.invalidated(id));
		// A successful write/readback can arrive before our Gateway echo.
		let named = ReactionEmoji {
			id: None,
			name: Some("x".into()),
		};
		let Some(crate::Command::Reactions(Command::Set { request, .. })) =
			state.prepare_reaction(id, named.clone())
		else {
			panic!()
		};
		state
			.apply_reactions(Event::Written {
				channel,
				message: id,
				request,
				result: Ok(()),
			})
			.unwrap();
		let Some(crate::Command::Reactions(Command::Read { request, .. })) =
			state.next_reaction_read()
		else {
			panic!()
		};
		state
			.apply_reactions(Event::Read {
				channel,
				message: id,
				request,
				result: Ok(visible(&state)),
			})
			.unwrap();
		state
			.apply_reactions(delta(own, &named, true, false))
			.unwrap();
		assert_eq!(visible(&state)[0].count, 1);
		state.timeline.set_reactions(id, None).unwrap();
		state
			.apply_reactions(delta(Id(500), &other, true, false))
			.unwrap();
		assert!(
			state.timeline.get(id).unwrap().reactions.is_none(),
			"unknown is not zero"
		);
		assert!(state.next_reaction_read().is_some());
	}

	#[test]
	fn live_reaction_verification_preserves_visible_counts_and_scope() {
		let mut message = crate::tests::message(10);
		let channel = message.channel;
		let id = message.id;
		let emoji = ReactionEmoji {
			id: None,
			name: Some("x".into()),
		};
		let snapshot = |count| {
			vec![Reaction {
				emoji: emoji.clone(),
				count,
				me: false,
				me_burst: false,
			}]
		};
		message.reactions = Some(snapshot(1));
		let mut state = State {
			selected: Some(channel),
			user: Some(message.author.clone()),
			channels: vec![Channel {
				id: channel,
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			gateway_connected: true,
			auth: AuthState::Authenticated,
			freshness: Freshness::Fresh,
			..State::default()
		};
		state.timeline.insert(message, false, false).unwrap();
		let delta = || Event::Delta {
			channel,
			message: id,
			user: Id(500),
			emoji: emoji.clone(),
			add: true,
			burst: false,
		};
		let visible =
			|s: &State| s.reactions.display(s.timeline.get(id).unwrap()).unwrap()[0].count;
		assert!(state.queue_reaction_read(id));
		assert_eq!(visible(&state), 1);
		let Some(crate::Command::Reactions(Command::Read { request, .. })) =
			state.next_reaction_read()
		else {
			panic!()
		};
		// The HTTP snapshot includes an add whose Gateway event has not arrived yet.
		state
			.apply_reactions(Event::Read {
				channel,
				message: id,
				request,
				result: Ok(snapshot(2)),
			})
			.unwrap();
		assert_eq!(visible(&state), 2);
		assert!(state.next_reaction_read().is_none());
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::Reactions(delta()),
		});
		assert_eq!(visible(&state), 3);
		assert!(state.timeline.get(id).unwrap().reactions.is_some());
		let Some(crate::Command::Reactions(Command::Read {
			channel: read_channel,
			message: read_message,
			request,
		})) = state.next_reaction_read()
		else {
			panic!()
		};
		assert_eq!((read_channel, read_message), (channel, id));
		assert!(
			state.next_reaction_read().is_none(),
			"only one verification is in flight"
		);
		assert_eq!(
			visible(&state),
			3,
			"verification must not hide known counts"
		);
		state
			.apply_reactions(Event::Read {
				channel,
				message: id,
				request,
				result: Ok(snapshot(2)),
			})
			.unwrap();
		assert_eq!(
			visible(&state),
			2,
			"verification repairs cross-transport ordering"
		);
		assert!(state.next_reaction_read().is_none());
		for unknown in [false, true] {
			if unknown {
				state.refresh_reactions(id);
			} else {
				assert!(state.queue_reaction_read(id));
			}
			let Some(crate::Command::Reactions(Command::Read { request, .. })) =
				state.next_reaction_read()
			else {
				panic!()
			};
			let mut replacement = crate::tests::message(id.0);
			replacement.content = "New replacement body".into();
			replacement.reactions = Some(snapshot(999));
			state.apply(crate::Envelope {
				generation: state.generation,
				event: crate::Event::Message(replacement),
			});
			assert_eq!(
				state.timeline.get(id).unwrap().content,
				"New replacement body"
			);
			state.apply(crate::Envelope {
				generation: state.generation,
				event: crate::Event::Patch(model::MessagePatch {
					id,
					channel,
					content: model::Patch::Value("New patch body".into()),
					reactions: model::Patch::Value(snapshot(998)),
					mentions: model::Patch::Absent,
					edited: model::Patch::Absent,
					embeds: model::Patch::Absent,
					embeds_suppressed: model::Patch::Absent,
					attachments: model::Patch::Absent,
					extra_content: Default::default(),
				}),
			});
			assert_eq!(state.timeline.get(id).unwrap().content, "New patch body");
			let expected = if unknown { None } else { Some(snapshot(2)) };
			assert_eq!(state.timeline.get(id).unwrap().reactions, expected);
			state
				.apply_reactions(Event::Read {
					channel,
					message: id,
					request,
					result: Ok(snapshot(997)),
				})
				.unwrap();
			assert_eq!(state.timeline.get(id).unwrap().reactions, expected);
			let Some(crate::Command::Reactions(Command::Read { request, .. })) =
				state.next_reaction_read()
			else {
				panic!()
			};
			state
				.apply_reactions(Event::Read {
					channel,
					message: id,
					request,
					result: Ok(snapshot(2)),
				})
				.unwrap();
			assert_eq!(visible(&state), 2);
		}

		let events = || {
			[
				delta(),
				Event::Cleared {
					channel,
					message: id,
					emoji: None,
				},
			]
		};
		for event in events() {
			state.apply(crate::Envelope {
				generation: state.generation - 1,
				event: crate::Event::Reactions(event),
			});
			assert_eq!(visible(&state), 2);
			assert!(!state.reactions.invalidated(id));
		}
		state.gateway_connected = false;
		for event in events() {
			state.apply_reactions(event).unwrap();
			assert_eq!(visible(&state), 2);
			assert!(!state.reactions.invalidated(id));
		}
		state.gateway_connected = true;
		state.channels[0].guild = Some(Id(99));
		assert!(
			!state.can_view(channel),
			"unknown guild permissions fail closed"
		);
		for event in events() {
			state.apply_reactions(event).unwrap();
			assert_eq!(visible(&state), 2);
			assert!(!state.reactions.invalidated(id));
		}
		state.channels[0].guild = None;
		state.freshness = Freshness::Unavailable;
		for event in events() {
			state.apply_reactions(event).unwrap();
			assert_eq!(visible(&state), 2);
			assert!(!state.reactions.invalidated(id));
		}
		state.freshness = Freshness::Fresh;
		state.selected = Some(Id(99));
		for event in events() {
			state.apply_reactions(event).unwrap();
			assert_eq!(visible(&state), 2);
			assert!(!state.reactions.invalidated(id));
		}
	}

	#[test]
	fn live_reaction_deltas_preserve_capacity_and_count_bounds() {
		let mut values: Vec<_> = (1..=model::MAX_REACTIONS)
			.map(|id| Reaction {
				emoji: ReactionEmoji {
					id: Some(Id(id as u64)),
					name: Some("x".repeat(128)),
				},
				count: 1,
				me: false,
				me_burst: false,
			})
			.collect();
		let extra = ReactionEmoji {
			id: Some(Id(1000)),
			name: None,
		};
		let before = values.clone();
		assert!(apply_delta(&mut values, &extra, false, true, false).is_err());
		assert_eq!(
			values, before,
			"a rejected new emoji cannot mutate a full list"
		);
		apply_delta(&mut values, &extra, false, false, false).unwrap();
		assert_eq!(
			values, before,
			"removing an unknown emoji must not create a row"
		);
		let existing = values[0].emoji.clone();
		apply_delta(&mut values, &existing, false, true, false).unwrap();
		assert_eq!(
			values[0].count, 2,
			"a full list still accepts existing emoji updates"
		);
		assert_eq!(values.len(), model::MAX_REACTIONS);
		values[0].count = u32::MAX;
		let before = values.clone();
		assert!(apply_delta(&mut values, &existing, false, true, true).is_err());
		assert_eq!(
			values, before,
			"overflow must not change count or membership"
		);
		values[0].me = true;
		apply_delta(&mut values, &existing, true, true, false).unwrap();
		assert_eq!(
			values[0].count,
			u32::MAX,
			"an own echo remains idempotent at capacity"
		);
		values[0].count = 1;
		apply_delta(&mut values, &existing, true, false, false).unwrap();
		assert!(!values.iter().any(|r| r.emoji.same(&existing)));
		apply_delta(&mut values, &existing, true, false, false).unwrap();
		assert!(model::valid_reactions(&values));
		assert_eq!(values.len(), model::MAX_REACTIONS - 1);
	}

	#[test]
	fn reaction_permissions_distinguish_existing_emoji_and_late_reads() {
		let user = User {
			id: Id(2),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			selected: Some(Id(10)),
			user: Some(user.clone()),
			guilds: vec![model::Guild {
				id: Id(1),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			channels: vec![Channel {
				id: Id(10),
				guild: Some(Id(1)),
				kind: 0,
				name: "Synthetic".into(),
				parent_id: None,
				last_message: None,
				position: 0,
				recipients: vec![],
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..State::default()
		};
		state
			.permissions
			.replace(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(1),
					owner: Some(Id(999)),
					roles: Some(vec![p::Role {
						name: String::new(),
						color: 0,
						position: 0,
						hoist: false,
						id: Id(1),
						bits: p::VIEW_CHANNEL | p::READ_MESSAGE_HISTORY,
					}]),
					member: Some(p::Member {
						roles: vec![],
						timeout_until: None,
					}),
				}],
				channels: vec![p::Channel {
					id: Id(10),
					guild: Id(1),
					overwrites: Some(vec![]),
				}],
			})
			.unwrap();
		let emoji = ReactionEmoji {
			id: None,
			name: Some("a".into()),
		};
		state
			.timeline
			.insert(
				Message {
					kind: 0,
					id: Id(50),
					channel: Id(10),
					author: user,
					content: "Synthetic".into(),
					reactions: Some(vec![]),
					mention_roles: vec![],
					mention_everyone: false,
					suppress_notifications: false,
					mentions: vec![],
					edited: false,
					edited_at: None,
					revision: 0,
					nonce: None,
					reply_to: None,
					reply_deleted: false,
					forwarded: false,
					unsupported: false,
					extra_content: Default::default(),
					embeds: vec![],
					embeds_suppressed: false,
					attachments: vec![],
				},
				true,
				false,
			)
			.unwrap();
		assert!(state.prepare_reaction(Id(50), emoji.clone()).is_none());
		state
			.timeline
			.set_reactions(
				Id(50),
				Some(vec![Reaction {
					emoji: emoji.clone(),
					count: 1,
					me: false,
					me_burst: false,
				}]),
			)
			.unwrap();
		assert!(matches!(
			state.prepare_reaction(Id(50), emoji.clone()),
			Some(crate::Command::Reactions(Command::Set { add: true, .. }))
		));
		let visible = state
			.reactions
			.display(state.timeline.get(Id(50)).unwrap())
			.unwrap();
		assert!(visible[0].me);
		assert_eq!(visible[0].count, 2);
		let (_, request) = state.reactions.writing.unwrap();
		state
			.apply_reactions(Event::Written {
				channel: Id(10),
				message: Id(50),
				request,
				result: Err(Failure::Forbidden),
			})
			.unwrap();
		let visible = state
			.reactions
			.display(state.timeline.get(Id(50)).unwrap())
			.unwrap();
		assert!(!visible[0].me);
		assert_eq!(visible[0].count, 1);
		state.reactions.reset();
		state
			.timeline
			.set_reactions(
				Id(50),
				Some(vec![Reaction {
					emoji: emoji.clone(),
					count: 1,
					me: true,
					me_burst: false,
				}]),
			)
			.unwrap();
		assert!(matches!(
			state.prepare_reaction(Id(50), emoji.clone()),
			Some(crate::Command::Reactions(Command::Set { add: false, .. }))
		));
		assert!(
			state
				.reactions
				.display(state.timeline.get(Id(50)).unwrap())
				.unwrap()
				.is_empty()
		);
		let (_, request) = state.reactions.writing.unwrap();
		state
			.apply_reactions(Event::Written {
				channel: Id(10),
				message: Id(50),
				request,
				result: Err(Failure::RateLimited),
			})
			.unwrap();
		assert!(
			state
				.reactions
				.display(state.timeline.get(Id(50)).unwrap())
				.unwrap()[0]
				.me
		);
		state.reactions.reset();
		state.refresh_reactions(Id(50));
		let Some(crate::Command::Reactions(Command::Read { request, .. })) =
			state.next_reaction_read()
		else {
			panic!()
		};
		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits = p::VIEW_CHANNEL;
		state.permissions.clear_cache();
		state
			.apply_reactions(Event::Read {
				channel: Id(10),
				message: Id(50),
				request,
				result: Ok(vec![Reaction {
					emoji,
					count: 1,
					me: true,
					me_burst: false,
				}]),
			})
			.unwrap();
		assert!(state.timeline.get(Id(50)).unwrap().reactions.is_none());
		state.refresh_reactions(Id(50));
		assert!(state.next_reaction_read().is_none());
		assert!(
			state.can_mark_read(Id(50)),
			"Observed live messages may be marked read without history permission"
		);
		let Some(crate::Command::MarkRead { request, .. }) = state.prepare_mark_read(Id(50)) else {
			panic!()
		};
		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits = 0;
		state.permissions.clear_cache();
		state
			.apply_read_state(crate::read_state::Event::Result {
				channel: Id(10),
				message: Id(50),
				request,
				result: Ok(()),
			})
			.unwrap();
		assert!(state.read_marker(Id(10)).is_none());
		assert!(!state.can_mark_read(Id(50)));
		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits = p::VIEW_CHANNEL;
		state.permissions.clear_cache();
		assert!(
			state.read_marker(Id(10)).flatten().is_none(),
			"A late success cannot record a revoked read marker"
		);
	}
}
