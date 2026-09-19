use crate::{Command, State, auth::AuthState};
use model::{Freshness, Id, Message};
use session_cache::Timeline;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
	target: Id,
	pub mention: bool,
}

impl Reply {
	pub fn to(target: Id) -> Self {
		Self {
			target,
			mention: true,
		}
	}

	pub fn target(self) -> Id {
		self.target
	}
}

/// Accepted known-deleted reply targets from the last applied event (at most 50).
#[derive(Default)]
pub struct ReplyDeletions(pub(crate) Vec<(Id, Id)>);

impl State {
	pub fn reply_target(&self) -> Option<Id> {
		self.reply.map(Reply::target)
	}

	/// Navigate a Discord chat link without sending messages or joining voice.
	pub fn open_chat_link(
		&mut self,
		guild: Option<Id>,
		channel: Id,
		message: Option<Id>,
	) -> Result<Option<Command>, &'static str> {
		if channel.0 == 0 || guild.is_some_and(|id| id.0 == 0) {
			return Err("This chat link is invalid");
		}
		let target = self.channel(channel).ok_or("This chat is unavailable")?;
		if target.guild != guild {
			return Err("This chat does not belong to the linked server");
		}
		if !crate::navigable(target) {
			return Err("This channel kind is unsupported");
		}
		if !self.can_view(channel) {
			return Err("Channel permissions are unavailable or access was revoked");
		}
		let Some(message) = message else {
			return Ok(self.select(channel));
		};
		if message.0 == 0 || message.0.checked_add(1).is_none() {
			return Err("This message link is invalid");
		}
		if !target.supports_text() || !self.can_read_history(channel) {
			return Err("Message history is unavailable with the current permissions");
		}
		if self.auth != AuthState::Authenticated || !self.gateway_connected {
			return Err("Message links are unavailable while disconnected");
		}
		if self.selected == Some(channel) {
			if self.timeline.is_deleted(message) {
				return Err("This message was deleted");
			}
			if self.freshness == Freshness::Fresh
				&& !self.history_pending
				&& self
					.timeline
					.get(message)
					.is_some_and(|m| m.channel == channel)
			{
				self.search_target = Some(message);
				self.revision += 1;
				return Ok(None);
			}
		}
		// The targeted request supersedes selection's recent-history request.
		let recent = self.select(channel);
		if self.timeline.is_deleted(message) {
			// A restored resident window can reveal a deletion after selection.
			self.status = "This message was deleted";
			return Ok(recent);
		}
		Ok(self.open_target_window(message))
	}

	/// All effects belong to one channel, with at most 50 distinct targets.
	pub fn take_reply_deletions(&mut self) -> Vec<(Id, Id)> {
		std::mem::take(&mut self.reply_deletions.0)
	}
	pub(crate) fn accepts_reply_source(&self, message: &Message) -> bool {
		message.id.0 != 0
			&& message.channel.0 != 0
			&& Timeline::valid_message(message)
			&& self.can_view(message.channel)
			&& self
				.channels
				.iter()
				.any(|channel| channel.id == message.channel && channel.supports_text())
			&& (self.selected != Some(message.channel) || self.freshness != Freshness::Unavailable)
	}
	pub(crate) fn record_reply_deletion(&mut self, message: &Message) {
		if !message.reply_deleted {
			return;
		}
		let Some(target) = message
			.reply_to
			.filter(|target| target.0 != 0 && *target < message.id)
		else {
			return;
		};
		let deletion = (message.channel, target);
		// Only one message or an already validated <=50-message history page calls this.
		if !self.reply_deletions.0.contains(&deletion) && self.reply_deletions.0.len() < 50 {
			self.reply_deletions.0.push(deletion);
		}
		self.resident.remove(message.channel);
		self.read_state.activity.delete(message.channel, target);
		if let Some(channel) = self
			.channels
			.iter_mut()
			.find(|channel| channel.id == message.channel && channel.last_message == Some(target))
		{
			self.read_state.activity.observe_latest(channel.id, target);
			channel.last_message = None;
		}
		if self.selected == Some(message.channel) {
			self.clear_search();
			if self.reply_target() == Some(target) {
				self.reply = None;
			}
		}
	}
	pub fn can_open_reply_target(&self, target: Id) -> bool {
		let Some(channel) = self.selected else {
			return false;
		};
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness == Freshness::Fresh
			&& !self.history_pending
			&& target.0 != 0
			&& target.0.checked_add(1).is_some()
			&& !self.timeline.is_deleted(target)
			&& !self.timeline.iter().any(|message| {
				message.channel == channel
					&& message.reply_to == Some(target)
					&& message.reply_deleted
			}) && self.can_read_history(channel)
			&& self
				.channels
				.iter()
				.any(|entry| entry.id == channel && entry.supports_text())
			&& self
				.timeline
				.get(target)
				.is_none_or(|message| message.channel == channel)
			&& (self.reply_target() == Some(target)
				|| self
					.timeline
					.iter()
					.any(|message| message.channel == channel && message.reply_to == Some(target)))
	}

	/// Loaded targets only request a local scroll. Unloaded targets use one bounded
	/// history page; None can therefore mean either a local jump or a rejected action.
	pub fn open_reply_target(&mut self, target: Id) -> Option<Command> {
		if !self.can_open_reply_target(target) {
			return None;
		}
		if self.timeline.get(target).is_some() {
			self.search_target = Some(target);
			self.revision += 1;
			return None;
		}
		self.open_target_window(target)
	}

	pub(crate) fn open_target_window(&mut self, target: Id) -> Option<Command> {
		if target.0 == 0 || self.timeline.is_deleted(target) {
			return None;
		}
		let before = Id(target.0.checked_add(1)?);
		self.timeline.clear_window_preserving_deletions();
		self.newer_cursor = None;
		self.newer_may_have_more = false;
		self.history_targeted = true;
		self.revision += 1;
		self.search_target = Some(target);
		let command = self.history(Some(before));
		self.enforce_resident_budget();
		Some(command)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event, Pending, auth::Failure};
	use model::{Channel, Delivery, Message, User};

	fn message(id: u64) -> Message {
		Message {
			id: Id(id),
			channel: Id(1),
			author: User {
				id: Id(9),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			content: "Synthetic reply".into(),
			reactions: Some(vec![]),
			author_nick: None,
			author_roles: vec![],
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
			kind: 0,
			unsupported: false,
			extra_content: Default::default(),
			embeds: vec![],
			attachments: vec![],
			embeds_suppressed: false,
		}
	}
	fn state() -> State {
		let mut state = State {
			user: Some(message(1).author),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			selected: Some(Id(1)),
			freshness: Freshness::Fresh,
			channels: (1..=3)
				.map(|id| Channel {
					id: Id(id),
					guild: None,
					parent_id: None,
					kind: 1,
					position: 0,
					name: "Synthetic DM".into(),
					recipients: vec![],
					last_message: None,
					icon: None,
					member_list_id: None,
					message_count: None,
				})
				.collect(),
			..State::default()
		};
		let mut source = message(100);
		source.reply_to = Some(Id(50));
		state.timeline.insert(source, false, false).unwrap();
		state
	}
	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
	fn deleted_source(id: u64, target: u64, channel: u64) -> Message {
		let mut source = message(id);
		source.kind = 19;
		source.channel = Id(channel);
		source.reply_to = Some(Id(target));
		source.reply_deleted = true;
		source
	}
	fn chat_link_state() -> State {
		let mut state = state();
		let mut dm = state.channels[0].clone();
		dm.id = Id(4);
		for channel in &mut state.channels {
			channel.guild = Some(Id(if channel.id == Id(3) { 20 } else { 10 }));
			channel.kind = 0;
		}
		state.channels.push(dm);
		state.guilds = [10, 20]
			.into_iter()
			.map(|id| model::Guild {
				id: Id(id),
				name: "Synthetic guild".into(),
				icon: None,
				emojis: None,
			})
			.collect();
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: [10, 20]
					.into_iter()
					.map(|id| model::permissions::Guild {
						id: Id(id),
						owner: Some(Id(9)),
						roles: None,
						member: None,
					})
					.collect(),
				channels: vec![],
			})
			.unwrap();
		state.drafts.insert(Id(1), "Unsent draft".into());
		state
	}

	#[test]
	fn chat_links_select_same_server_other_server_and_dm_without_joining_voice() {
		for (guild, channel, kind) in [
			(Some(Id(10)), Id(2), 0),
			(Some(Id(20)), Id(3), 0),
			(None, Id(4), 1),
			(Some(Id(20)), Id(3), 15),
			(Some(Id(20)), Id(3), 2),
		] {
			let mut state = chat_link_state();
			state
				.channels
				.iter_mut()
				.find(|c| c.id == channel)
				.unwrap()
				.kind = kind;
			let command = state.open_chat_link(guild, channel, None).unwrap();
			assert_eq!(state.selected, Some(channel));
			assert!(command.is_none_or(
				|command| matches!(command, Command::History { channel: id, .. } if id == channel)
			));
			assert!(state.voice.active.is_none());
			assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		}
	}

	#[test]
	fn chat_links_scroll_loaded_messages_or_fetch_one_targeted_page() {
		let mut state = chat_link_state();
		let request = state.request;
		assert!(
			state
				.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
				.unwrap()
				.is_none()
		);
		assert_eq!(state.search_target, Some(Id(100)));
		assert_eq!(state.request, request);
		assert!(state.timeline.get(Id(100)).is_some());
		for (guild, channel) in [(Some(Id(10)), Id(1)), (Some(Id(20)), Id(3)), (None, Id(4))] {
			let mut state = chat_link_state();
			let command = state
				.open_chat_link(guild, channel, Some(Id(50)))
				.unwrap()
				.unwrap();
			assert!(
				matches!(command, Command::History { channel: id, before: Some(Id(51)), after: None, request } if id == channel && request == state.request)
			);
			assert_eq!(state.selected, Some(channel));
			assert_eq!(state.search_target, Some(Id(50)));
			assert!(state.history_targeted && state.history_pending);
			assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		}
	}

	#[test]
	fn chat_links_keep_recent_history_when_a_restored_window_knows_the_target_was_deleted() {
		let mut state = chat_link_state();
		state.timeline.delete(Id(50)).unwrap();
		state.select(Id(2));
		assert_eq!(state.resident_window_count(), 1);
		let command = state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
			.unwrap();
		assert!(matches!(
			command,
			Some(Command::History {
				channel: Id(1),
				before: None,
				..
			})
		));
		assert_eq!(state.status, "This message was deleted");
		assert!(state.search_target.is_none());
	}

	#[test]
	fn chat_links_reject_invalid_unavailable_and_forbidden_targets_before_navigation() {
		for blocked in 0..11 {
			let mut state = chat_link_state();
			let mut guild = Some(Id(20));
			let mut channel = Id(3);
			let mut message = Some(Id(50));
			match blocked {
				0 => channel = Id(0),
				1 => channel = Id(999),
				2 => guild = None,
				3 => message = Some(Id(0)),
				4 => message = Some(Id(u64::MAX)),
				5 => state.channels[2].kind = 4,
				6 => state.user = None,
				7 => state.gateway_connected = false,
				8 => state.auth = AuthState::Unauthenticated,
				9 => state.channels[2].kind = 15,
				_ => {
					guild = Some(Id(10));
					channel = Id(1);
					state.timeline.delete(Id(50)).unwrap();
				}
			}
			let revision = state.revision;
			let request = state.request;
			assert!(
				state.open_chat_link(guild, channel, message).is_err(),
				"case {blocked}"
			);
			assert_eq!(state.selected, Some(Id(1)));
			assert_eq!(state.revision, revision);
			assert_eq!(state.request, request);
			assert!(state.search_target.is_none());
			assert!(state.timeline.get(Id(100)).is_some());
			assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		}
	}

	#[test]
	fn accepted_deletion_effects_are_scoped_bounded_and_reset_on_rejected_generation() {
		let mut state = state();
		state.timeline.insert(message(50), false, false).unwrap();
		apply(&mut state, Event::Message(deleted_source(101, 50, 2)));
		assert_eq!(state.take_reply_deletions(), vec![(Id(2), Id(50))]);
		assert!(state.timeline.get(Id(50)).is_some()); // Inactive channel is independent.
		apply(&mut state, Event::Message(deleted_source(101, 50, 1)));
		assert!(state.timeline.get(Id(50)).is_none());
		state.apply(Envelope {
			generation: state.generation - 1,
			event: Event::Message(deleted_source(102, 40, 1)),
		});
		assert!(state.take_reply_deletions().is_empty());
		assert!(!state.timeline.is_deleted(Id(40)));
		state.history(Some(Id(201)));
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: true,
				messages: (1..=50)
					.map(|target| deleted_source(100 + target, target, 1))
					.collect(),
			},
		);
		let effects = state.take_reply_deletions();
		assert_eq!(effects.len(), 50);
		assert!(effects.iter().all(|(channel, _)| *channel == Id(1)));
		assert!(state.take_reply_deletions().is_empty());
	}

	#[test]
	fn rejected_history_cannot_publish_deletion_effects_or_remove_loaded_bodies() {
		for invalid in 0..6 {
			let mut state = state();
			state.timeline.insert(message(50), false, false).unwrap();
			state.history(Some(Id(150)));
			let mut request = state.request;
			let mut source = deleted_source(101, 50, 1);
			let mut messages = vec![];
			match invalid {
				0 => request += 1,
				1 => source.channel = Id(2),
				2 => source.id = Id(151),
				3 => messages.push(source.clone()),
				4 => source.kind = 0,
				_ => {
					let mut oversized = message(102);
					oversized.content = "x".repeat(64 * 1024 + 1);
					messages.push(oversized);
				}
			}
			messages.push(source);
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: true,
					messages,
				},
			);
			assert!(state.take_reply_deletions().is_empty());
			assert!(state.timeline.get(Id(50)).is_some());
			assert!(!state.timeline.is_deleted(Id(50)));
		}
	}

	#[test]
	fn send_deletion_requires_correlation_and_handles_gateway_first_without_replacing_text() {
		for invalid in 0..3 {
			let mut state = state();
			state.timeline.insert(message(50), false, false).unwrap();
			if invalid != 0 {
				state.pending.push(Pending {
					channel: Id(if invalid == 1 { 2 } else { 1 }),
					content: "Pending".into(),
					attachments: vec![],
					nonce: "synthetic".into(),
					delivery: Delivery::Ambiguous,
					confirmed: None,
				});
			}
			let mut source = deleted_source(101, 50, 1);
			if invalid == 2 {
				source.author.id = Id(8);
			}
			apply(
				&mut state,
				Event::SendResult {
					nonce: "synthetic".into(),
					result: Ok(source),
				},
			);
			assert!(state.take_reply_deletions().is_empty());
			assert!(state.timeline.get(Id(50)).is_some());
		}
		let mut state = state();
		state.timeline.insert(message(50), false, false).unwrap();
		state.pending.push(Pending {
			channel: Id(1),
			content: "Pending".into(),
			attachments: vec![],
			nonce: "synthetic".into(),
			delivery: Delivery::Ambiguous,
			confirmed: None,
		});
		let mut source = deleted_source(101, 50, 1);
		source.reply_deleted = false;
		source.nonce = Some("synthetic".into());
		source.content = "Newer Gateway body".into();
		source.edited_at = Some(20);
		apply(&mut state, Event::Message(source));
		assert!(state.pending.is_empty());
		apply(
			&mut state,
			Event::SendResult {
				nonce: "synthetic".into(),
				result: Ok(deleted_source(101, 50, 1)),
			},
		);
		assert_eq!(state.take_reply_deletions(), vec![(Id(1), Id(50))]);
		assert!(state.timeline.is_deleted(Id(50)));
		let source = state.timeline.get(Id(101)).unwrap();
		assert_eq!(source.content, "Newer Gateway body");
		assert!(source.reply_deleted);
		state.pending.push(Pending {
			channel: Id(2),
			content: "Pending".into(),
			attachments: vec![],
			nonce: "other".into(),
			delivery: Delivery::Ambiguous,
			confirmed: None,
		});
		apply(
			&mut state,
			Event::SendResult {
				nonce: "other".into(),
				result: Ok(deleted_source(102, 40, 2)),
			},
		);
		assert_eq!(state.take_reply_deletions(), vec![(Id(2), Id(40))]);
		assert!(!state.timeline.is_deleted(Id(40)));
	}

	#[test]
	fn send_deletion_capacity_clears_the_known_deleted_body_and_preserves_drafts() {
		let mut state = state();
		state.drafts.insert(Id(1), "Unsent draft".into());
		state.timeline.insert(message(50), false, false).unwrap();
		let mut source = deleted_source(101, 50, 1);
		source.reply_deleted = false;
		source.nonce = Some("synthetic".into());
		state.timeline.insert(source, false, false).unwrap();
		for id in 1000..1000 + session_cache::MAX_MUTATIONS as u64 {
			state.timeline.delete(Id(id)).unwrap();
		}
		apply(
			&mut state,
			Event::SendResult {
				nonce: "synthetic".into(),
				result: Ok(deleted_source(101, 50, 1)),
			},
		);
		assert!(state.timeline.is_empty());
		assert_eq!(state.freshness, Freshness::Stale);
		assert_eq!(state.take_reply_deletions(), vec![(Id(1), Id(50))]);
		assert_eq!(state.drafts[&Id(1)], "Unsent draft");
	}

	#[test]
	fn loaded_reply_target_only_scrolls_and_preserves_composer_work() {
		let mut state = state();
		state.timeline.insert(message(50), false, false).unwrap();
		state.reply = Some(Reply::to(Id(100)));
		state.drafts.insert(Id(1), "Unsent draft".into());
		state.pending.push(Pending {
			channel: Id(1),
			content: "Pending".into(),
			attachments: vec![],
			nonce: "synthetic".into(),
			delivery: Delivery::Ambiguous,
			confirmed: None,
		});
		let request = state.request;
		let revision = state.revision;
		let content = state.timeline.get(Id(50)).unwrap().content.as_ptr();
		assert!(state.can_open_reply_target(Id(50)));
		assert!(state.open_reply_target(Id(50)).is_none());
		assert_eq!(state.search_target, Some(Id(50)));
		assert_eq!(state.request, request);
		assert_eq!(state.revision, revision + 1);
		assert!(!state.history_pending);
		assert_eq!(state.freshness, Freshness::Fresh);
		assert_eq!(
			state.timeline.get(Id(50)).unwrap().content.as_ptr(),
			content
		);
		assert_eq!(state.reply_target(), Some(Id(100)));
		assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		assert_eq!(state.pending[0].delivery, Delivery::Ambiguous);
		assert!(state.can_open_reply_target(Id(100))); // Composer context is also a source.
	}

	#[test]
	fn unloaded_reply_uses_one_scoped_page_and_preserves_deletion_guards() {
		let mut state = state();
		state.reply = Some(Reply::to(Id(50)));
		state.drafts.insert(Id(1), "Unsent draft".into());
		apply(
			&mut state,
			Event::Delete {
				channel: Id(1),
				id: Id(40),
			},
		);
		let command = state.open_reply_target(Id(50)).unwrap();
		let Command::History {
			channel,
			before,
			request,
			..
		} = command
		else {
			panic!()
		};
		assert_eq!(channel, Id(1));
		assert_eq!(before, Some(Id(51)));
		assert_eq!(state.timeline.row_count(), 0);
		assert!(state.timeline.is_deleted(Id(40)));
		assert_eq!(state.freshness, Freshness::Loading);
		assert_eq!(state.reply_target(), Some(Id(50)));
		assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		assert!(state.open_reply_target(Id(50)).is_none());
		assert_eq!(state.request, request);
		// A delete racing the request must also defeat both live and REST bodies.
		apply(
			&mut state,
			Event::Delete {
				channel: Id(1),
				id: Id(50),
			},
		);
		apply(&mut state, Event::Message(message(50)));
		apply(
			&mut state,
			Event::History {
				channel,
				request,
				older: true,
				messages: vec![message(50), message(40), message(30)],
			},
		);
		assert_eq!(state.timeline.row_ids().collect::<Vec<_>>(), vec![Id(30)]);
		assert!(state.timeline.is_deleted(Id(50)));
		assert!(!state.can_open_reply_target(Id(50)));
		assert_eq!(state.freshness, Freshness::Fresh);
	}

	#[test]
	fn reply_targets_require_a_current_scoped_reference_and_safe_state() {
		let state = state();
		assert!(!state.can_open_reply_target(Id(25)));
		assert!(!state.can_open_reply_target(Id(100))); // Loaded alone is not a reference.
		for target in [Id(0), Id(u64::MAX)] {
			let mut invalid = self::state();
			invalid.reply = Some(Reply::to(target));
			assert!(!invalid.can_open_reply_target(target));
			assert!(invalid.open_reply_target(target).is_none());
		}
		for blocked in 0..8 {
			let mut state = self::state();
			match blocked {
				0 => state.auth = AuthState::Unauthenticated,
				1 => state.gateway_connected = false,
				2 => state.freshness = Freshness::Stale,
				3 => state.history_pending = true,
				4 => state.selected = Some(Id(2)),
				5 => state.channels[0].kind = 2,
				6 => {
					state.timeline.delete(Id(50)).unwrap();
				}
				_ => {
					let mut target = message(50);
					target.channel = Id(2);
					state.timeline.insert(target, false, false).unwrap();
				}
			}
			assert!(!state.can_open_reply_target(Id(50)));
			assert!(state.open_reply_target(Id(50)).is_none());
			assert!(state.search_target.is_none());
		}
	}

	#[test]
	fn failure_navigation_logout_and_revocation_retire_target_requests() {
		for transition in 0..4 {
			let mut state = state();
			state.open_reply_target(Id(50)).unwrap();
			let request = state.request;
			let generation = state.generation;
			match transition {
				0 => apply(
					&mut state,
					Event::HistoryFailed {
						channel: Id(1),
						request,
						failure: Failure::Network,
					},
				),
				1 => {
					state.select(Id(2));
				}
				2 => state.logout(),
				_ => apply(&mut state, Event::Unavailable(Id(1))),
			}
			assert!(state.search_target.is_none());
			state.apply(Envelope {
				generation,
				event: Event::History {
					channel: Id(1),
					request,
					older: true,
					messages: vec![message(50)],
				},
			});
			assert!(state.timeline.get(Id(50)).is_none());
		}
	}

	#[test]
	fn explicit_reference_deletion_retires_composer_and_loaded_target() {
		for history in [false, true] {
			let mut state = state();
			state.timeline.insert(message(50), false, false).unwrap();
			state.reply = Some(Reply::to(Id(50)));
			let mut source = message(101);
			source.reply_to = Some(Id(50));
			source.kind = 19;
			source.reply_deleted = true;
			if history {
				state.history(None);
				let request = state.request;
				apply(
					&mut state,
					Event::History {
						channel: Id(1),
						request,
						older: false,
						messages: vec![message(50), source],
					},
				);
			} else {
				apply(&mut state, Event::Message(source));
			}
			assert!(state.timeline.get(Id(50)).is_none());
			assert!(state.timeline.is_deleted(Id(50)));
			assert!(state.reply.is_none());
			assert!(!state.can_open_reply_target(Id(50)));
			assert!(state.open_reply_target(Id(50)).is_none());
		}
	}

	#[test]
	fn fetched_target_ranges_never_become_recent_resident_windows_after_scroll() {
		let mut state = state();
		for channel in [2, 3, 1] {
			state.select(Id(channel)).unwrap();
			let request = state.request;
			let mut loaded = message(100);
			loaded.channel = Id(channel);
			loaded.reply_to = Some(Id(50));
			apply(
				&mut state,
				Event::History {
					channel: Id(channel),
					request,
					older: false,
					messages: vec![loaded],
				},
			);
		}
		assert_eq!(state.resident_window_count(), 2);
		state.open_reply_target(Id(50)).unwrap();
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: true,
				messages: vec![message(50)],
			},
		);
		assert_eq!(state.search_target.take(), Some(Id(50))); // The UI consumes only the cue.
		assert!(state.history_targeted);
		assert!(state.resident_history_rows() <= 1475);
		assert!(state.resident_history_bytes() <= 16 * 1024 * 1024 - 66 * 1024);
		state.select(Id(2)).unwrap();
		assert!(!state.history_targeted);
		state.select(Id(1)).unwrap();
		assert_eq!(state.timeline.row_count(), 0);
		assert!(state.search_target.is_none());
	}

	#[test]
	fn search_and_pin_target_windows_preserve_prior_deletions() {
		for pins in [false, true] {
			let mut state = state();
			state.timeline.delete(Id(40)).unwrap();
			state.search = Some(crate::search::SearchView {
				pins,
				channel: Id(1),
				query: "Synthetic".into(),
				before: None,
				pin_before: None,
				request: 1,
				loading: false,
				error: None,
				page: Some(model::SearchPage {
					hits: vec![model::SearchHit {
						id: Id(50),
						channel: Id(1),
						author: "Synthetic".into(),
						excerpt: "Synthetic".into(),
					}],
					total: 1,
					partial: false,
					pin_cursor: None,
				}),
			});
			let command = state.open_search_hit(Id(50)).unwrap();
			let Command::History { request, .. } = command else {
				panic!()
			};
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: true,
					messages: vec![message(50), message(40)],
				},
			);
			assert!(state.timeline.get(Id(50)).is_some());
			assert!(state.timeline.get(Id(40)).is_none());
			assert!(state.timeline.is_deleted(Id(40)));
		}
	}
}
