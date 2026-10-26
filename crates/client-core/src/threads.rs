//! Active-thread snapshots update only their declared guild/parent scope.
use crate::{MAX_EVENT_BYTES, MAX_NAV, State};
use model::{Channel, Id};
use std::collections::{BTreeMap, BTreeSet};

impl State {
	pub(super) fn apply_threads_sync(
		&mut self,
		guild: Id,
		parents: Option<Vec<Id>>,
		mut threads: Vec<Channel>,
		removed: Vec<Id>,
	) -> Result<(), &'static str> {
		if !self.guilds.iter().any(|g| g.id == guild) {
			return Ok(());
		}
		if threads.len() + removed.len() > MAX_NAV
			|| parents.as_ref().is_some_and(|p| p.len() > MAX_NAV)
			|| threads.iter().map(Channel::bytes).sum::<usize>()
				+ threads.capacity().saturating_sub(threads.len()) * size_of::<Channel>()
				+ removed.capacity() * size_of::<Id>()
				+ parents
					.as_ref()
					.map_or(0, |parents| parents.capacity() * size_of::<Id>())
				> MAX_EVENT_BYTES
		{
			return Err("Thread snapshot exceeds safe capacity");
		}
		let parent_count = parents.as_ref().map_or(0, Vec::len);
		let parents: Option<BTreeSet<_>> = parents.map(|ids| ids.into_iter().collect());
		if parents.as_ref().is_some_and(|p| p.len() != parent_count) {
			return Err("Thread snapshot has duplicate parents");
		}
		let incoming: BTreeSet<_> = threads.iter().map(|c| c.id).collect();
		let removed_count = removed.len();
		let explicit: BTreeSet<_> = removed.into_iter().collect();
		if explicit.len() != removed_count
			|| explicit.iter().any(|id| id.0 == 0 || incoming.contains(id))
		{
			return Err("Thread snapshot has invalid explicit removals");
		}
		let transient = self.archived_thread;
		let in_scope = |c: &Channel| {
			c.guild == Some(guild)
				&& matches!(c.kind, 10..=12)
				&& (Some(c.id) != transient || incoming.contains(&c.id) || explicit.contains(&c.id))
				&& parents
					.as_ref()
					.is_none_or(|p| c.parent_id.is_some_and(|id| p.contains(&id)))
		};
		let mut ids = BTreeSet::new();
		let previous: BTreeMap<_, _> = self.channels.iter().map(|c| (c.id, c)).collect();
		if explicit
			.iter()
			.any(|id| previous.get(id).is_some_and(|old| !in_scope(old)))
		{
			return Err("Thread removal has invalid channel scope");
		}
		if threads.iter().any(|c| {
			!in_scope(c)
				|| c.parent_id.is_none()
				|| c.parent_id == Some(c.id)
				|| !ids.insert(c.id)
				|| previous.get(&c.id).is_some_and(|old| {
					!in_scope(old)
						|| (Some(c.id) == transient
							&& (old.parent_id != c.parent_id || old.kind != c.kind))
				})
		}) {
			return Err("Thread snapshot has invalid channel scope");
		}
		let retained = self.channels.iter().filter(|c| !in_scope(c));
		let retained_len = retained.clone().count() + threads.len();
		if retained_len + self.guilds.len() > MAX_NAV
			|| retained.map(Channel::bytes).sum::<usize>()
				+ threads.iter().map(Channel::bytes).sum::<usize>()
				+ self.guilds.iter().map(model::Guild::bytes).sum::<usize>()
				+ (self.guilds.capacity() - self.guilds.len()) * size_of::<model::Guild>()
				+ self.permissions.bytes()
				> model::account::MAX_BYTES
		{
			return Err("Thread snapshot exceeds safe capacity");
		}
		let mut next = Vec::with_capacity(retained_len);
		for thread in &mut threads {
			if let Some(old) = previous.get(&thread.id) {
				thread.last_message = thread.last_message.max(old.last_message);
			}
		}
		drop(previous);
		if transient.is_some_and(|id| ids.contains(&id)) {
			self.archived_thread = None;
		}
		if self.archives.as_ref().is_some_and(|view| {
			view.guild == guild
				&& parents
					.as_ref()
					.is_none_or(|ids| ids.contains(&view.parent))
		}) {
			self.clear_archives();
		}
		let removed = self
			.channels
			.iter()
			.filter(|c| in_scope(c) && !ids.contains(&c.id))
			.map(|c| c.id)
			.collect();
		self.remove_channels(&removed);
		self.channels.retain(|c| !in_scope(c));
		next.append(&mut self.channels);
		next.extend(threads);
		self.channels = next;
		// The snapshot just replaced this scope, so any fetched forum page is stale.
		if let Some(parent) = self.posts.parent
			&& self.channel(parent).and_then(|c| c.guild) == Some(guild)
			&& parents.as_ref().is_none_or(|ids| ids.contains(&parent))
		{
			self.reload_forum_posts(parent);
		}
		Ok(())
	}

	/// Records membership and archive metadata for known threads of `guild`, or of any
	/// server for the login snapshot.
	pub(super) fn apply_thread_changes(
		&mut self,
		guild: Option<Id>,
		changes: Vec<model::threads::Change>,
	) {
		for change in changes.into_iter().take(MAX_NAV) {
			if !self.channel(change.id).is_some_and(|thread| {
				matches!(thread.kind, 10..=12) && guild.is_none_or(|id| thread.guild == Some(id))
			}) {
				continue;
			}
			self.thread_info
				.entry(change.id)
				.or_default()
				.apply(&change);
		}
	}

	pub fn thread_member(&self, thread: Id) -> Option<&model::threads::Member> {
		self.thread_info.get(&thread)?.member.as_ref()
	}

	/// An active thread the account joined. Retained read history alone is not membership.
	pub fn thread_joined(&self, thread: &Channel) -> bool {
		matches!(thread.kind, 10..=12)
			&& Some(thread.id) != self.archived_thread
			&& self.thread_member(thread.id).is_some()
	}

	/// Unread that keeps a thread listed even beneath a muted parent: a mention, or unread
	/// messages while the thread itself is not muted.
	fn thread_unread_relevant(&self, thread: &Channel, now: i64) -> bool {
		self.mention_count(thread.id) > 0
			|| (self.channel_unread(thread) == Some(true)
				&& !self
					.thread_member(thread.id)
					.is_some_and(|member| member.muted_at(now)))
	}

	/// Discord keeps a joined thread in the channel list until its auto-archive window has
	/// elapsed since its latest activity, even before the service archives it.
	fn thread_recent(&self, thread: &Channel, now: i64) -> bool {
		let meta = self.thread_info.get(&thread.id).and_then(|info| info.meta);
		let latest = model::threads::snowflake_ms(thread.last_message.unwrap_or(thread.id))
			.max(meta.map_or(0, |meta| meta.activity));
		let window = i64::from(meta.map_or(1440, |meta| meta.auto_archive_minutes)) * 60_000;
		latest.saturating_add(window) > now
	}

	/// Threads Discord lists beneath each parent of `guild` in the channel list, in order.
	///
	/// Joined active threads that are recent, pinned, unread or mentioned, newest join
	/// first. A muted parent keeps only unread or mentioned threads, and hiding muted
	/// channels drops muted threads without mentions, unless the parent or one of its
	/// threads is selected. The selected thread is always listed, first when not joined.
	pub fn sidebar_threads(
		&self,
		guild: Id,
		now: i64,
		hide_muted: bool,
	) -> BTreeMap<Id, Vec<&Channel>> {
		let open = self
			.selected
			.and_then(|id| self.channel(id))
			.filter(|thread| thread.guild == Some(guild) && matches!(thread.kind, 10..=12));
		let focus = open.and_then(|thread| thread.parent_id).or(self.selected);
		let mut rows = BTreeMap::<Id, Vec<&Channel>>::new();
		for thread in &self.channels {
			let Some(parent) = thread
				.parent_id
				.filter(|_| thread.guild == Some(guild) && self.thread_joined(thread))
			else {
				continue;
			};
			if !self.channel(parent).is_some_and(|parent| {
				matches!(parent.kind, 0 | 5 | 15 | 16)
					&& parent.guild == thread.guild
					&& parent.parent_id != Some(thread.id)
			}) {
				continue;
			}
			let focused = focus == Some(parent);
			let unread = self.thread_unread_relevant(thread, now);
			// A muted category mutes the forums and channels inside it as well.
			let parent_muted = self.guild_channel_muted(parent) == Some(true)
				|| self
					.channel(parent)
					.and_then(|parent| parent.parent_id)
					.is_some_and(|category| self.guild_channel_muted(category) == Some(true));
			let listed = if !focused && parent_muted {
				unread
			} else {
				unread
					|| self.thread_recent(thread, now)
					|| self
						.thread_info
						.get(&thread.id)
						.is_some_and(|info| info.pinned)
			};
			let muted = self.guild_channel_muted(thread.id) == Some(true)
				|| self
					.thread_member(thread.id)
					.is_some_and(|member| member.muted_at(now));
			if listed && (focused || !hide_muted || !muted || self.mention_count(thread.id) > 0) {
				rows.entry(parent).or_default().push(thread);
			}
		}
		let joined = |thread: &Channel| self.thread_member(thread.id).map_or(0, |m| m.joined);
		for threads in rows.values_mut() {
			threads.sort_unstable_by_key(|thread| std::cmp::Reverse((joined(thread), thread.id)));
		}
		if let Some(open) = open
			&& let Some(parent) = open.parent_id
		{
			let threads = rows.entry(parent).or_default();
			if !threads.iter().any(|thread| thread.id == open.id) {
				threads.insert(0, open);
			}
		}
		rows
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event, auth::AuthState};
	use model::{Freshness, Guild, Message, User};

	#[test]
	fn thread_snapshots_preserve_other_scopes_and_cancel_removed_history() {
		let channel = |id, guild, parent_id: Option<u64>, kind| Channel {
			id: Id(id),
			guild: Some(Id(guild)),
			parent_id: parent_id.map(Id),
			kind,
			name: "Synthetic".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			tags: None,
			message_count: None,
		};
		let mut state = State {
			user: Some(User {
				primary_guild: None,
				cosmetics: None,
				id: Id(9),
				name: "Synthetic member".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![
				Guild {
					default_message_notifications: None,
					stickers: None,
					emojis: None,
					id: Id(1),
					name: "One".into(),
					icon: None,
				},
				Guild {
					default_message_notifications: None,
					stickers: None,
					emojis: None,
					id: Id(2),
					name: "Two".into(),
					icon: None,
				},
			],
			channels: vec![
				channel(10, 1, None, 0),
				channel(11, 1, None, 15),
				channel(100, 1, Some(10), 11),
				channel(101, 1, Some(11), 11),
				channel(20, 2, None, 0),
				channel(200, 2, Some(20), 12),
			],
			selected: Some(Id(100)),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			..State::default()
		};
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: state
					.guilds
					.iter()
					.map(|guild| model::permissions::Guild {
						id: guild.id,
						owner: Some(Id(999)),
						roles: Some(vec![model::permissions::Role {
							name: String::new(),
							color: 0,
							secondary_color: None,
							tertiary_color: None,
							position: 0,
							hoist: false,
							managed: false,
							id: guild.id,
							bits: model::permissions::VIEW_CHANNEL
								| model::permissions::READ_MESSAGE_HISTORY
								| model::permissions::SEND_MESSAGES_IN_THREADS,
						}]),
						member: Some(model::permissions::Member {
							roles: vec![],
							timeout_until: None,
						}),
					})
					.collect(),
				channels: state
					.channels
					.iter()
					.filter(|channel| !matches!(channel.kind, 10..=12))
					.map(|channel| model::permissions::Channel {
						id: channel.id,
						guild: channel.guild.unwrap(),
						overwrites: Some(vec![]),
					})
					.collect(),
			})
			.unwrap();
		state.drafts.insert(Id(100), "unsent thread draft".into());
		state.drafts.insert(Id(10), "unsent parent draft".into());
		let original = state.channels.clone();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ThreadsSync {
				removed: vec![],
				guild: Id(1),
				parents: Some(vec![]),
				threads: vec![],
			},
		});
		assert!(
			state.channels == original,
			"An explicit empty parent list covers no channels"
		);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ThreadsSync {
				removed: vec![],
				guild: Id(999),
				parents: None,
				threads: vec![],
			},
		});
		assert!(state.channels == original);
		for (guild, id) in [(2, 100), (1, 10), (1, 999)] {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::ThreadRemoved {
					guild: Id(guild),
					id: Id(id),
				},
			});
			assert!(state.channels == original);
			assert_eq!(state.freshness, Freshness::Fresh);
		}
		let mut message = Message {
			poll: None,
			sticker_items: Vec::new(),
			id: Id(500),
			channel: Id(100),
			author: User {
				primary_guild: None,
				cosmetics: None,
				id: Id(9),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			content: "Synthetic thread history".into(),
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: vec![],
			reactions: Some(vec![]),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			interaction: None,
			forwarded: false,
			unsupported: false,
			components: vec![],
			application_id: None,
			flags: 0,
			ephemeral: false,
			extra_content: Default::default(),
			embeds: vec![],
			embeds_suppressed: false,
			attachments: vec![],
		};
		state
			.timeline
			.insert(message.clone(), false, false)
			.unwrap();
		let _ = state.history(None);
		let request = state.request;
		assert!(state.request_search("synthetic".into(), None).is_some());
		state.search_target = Some(message.id);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ThreadsSync {
				removed: vec![],
				guild: Id(1),
				parents: Some(vec![Id(10)]),
				threads: vec![channel(102, 1, Some(10), 11)],
			},
		});
		assert!(!state.channels.iter().any(|c| c.id == Id(100)));
		for id in [10, 11, 20, 101, 102, 200] {
			assert!(state.channels.iter().any(|c| c.id == Id(id)));
		}
		assert!(
			state.timeline.is_empty() && state.search.is_none() && state.search_target.is_none()
		);
		assert!(!state.history_pending && state.request != request);
		assert_eq!(state.freshness, Freshness::Unavailable);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::History {
				channel: Id(100),
				request,
				older: false,
				messages: vec![message.clone()],
			},
		});
		assert!(state.timeline.is_empty());
		assert_eq!(state.freshness, Freshness::Unavailable);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ThreadsSync {
				removed: vec![],
				guild: Id(1),
				parents: None,
				threads: vec![],
			},
		});
		assert!(
			!state
				.channels
				.iter()
				.any(|c| c.guild == Some(Id(1)) && matches!(c.kind, 10..=12))
		);
		for id in [10, 11, 20, 200] {
			assert!(state.channels.iter().any(|c| c.id == Id(id)));
		}
		state.selected = Some(Id(200));
		state.freshness = Freshness::Fresh;
		message.channel = Id(200);
		state.timeline.insert(message, false, false).unwrap();
		let _ = state.history(None);
		let request = state.request;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Unavailable(Id(20)),
		});
		assert!(
			!state
				.channels
				.iter()
				.any(|c| matches!(c.id, Id(20) | Id(200)))
		);
		assert!(state.timeline.is_empty() && !state.history_pending && state.request != request);
		assert_eq!(state.freshness, Freshness::Unavailable);
		assert_eq!(
			state.drafts.get(&Id(100)).map(String::as_str),
			Some("unsent thread draft")
		);
		assert_eq!(
			state.drafts.get(&Id(10)).map(String::as_str),
			Some("unsent parent draft")
		);
		assert_eq!(state.guilds.len(), 2);
		assert!(state.channels.iter().any(|c| c.id == Id(10) && c.kind == 0));
	}

	#[test]
	fn invalid_thread_snapshots_leave_navigation_and_drafts_unchanged() {
		let channel = |id, guild, parent_id: Option<u64>, kind| Channel {
			id: Id(id),
			guild: Some(Id(guild)),
			parent_id: parent_id.map(Id),
			kind,
			name: "Synthetic".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			tags: None,
			message_count: None,
		};
		let original = vec![
			channel(10, 1, None, 0),
			channel(11, 1, None, 0),
			channel(100, 1, Some(10), 11),
		];
		let mut oversized = channel(102, 1, Some(10), 11);
		oversized.name = "x".repeat(MAX_EVENT_BYTES + 1);
		for (parents, threads) in [
			(None, vec![channel(102, 1, Some(10), 11); 2]),
			(None, vec![channel(102, 2, Some(10), 11)]),
			(Some(vec![Id(10)]), vec![channel(102, 1, Some(11), 11)]),
			(None, vec![channel(102, 1, Some(10), 0)]),
			(None, vec![channel(10, 1, Some(11), 11)]),
			(None, vec![channel(102, 1, None, 11)]),
			(None, vec![channel(102, 1, Some(102), 11)]),
			(Some(vec![]), vec![channel(102, 1, Some(10), 11)]),
			(Some(vec![Id(10), Id(10)]), vec![]),
			(Some(vec![Id(10); MAX_NAV + 1]), vec![]),
			(
				None,
				(0..=MAX_NAV)
					.map(|n| channel(1000 + n as u64, 1, Some(10), 11))
					.collect(),
			),
			(
				None,
				(0..MAX_NAV)
					.map(|n| channel(1000 + n as u64, 1, Some(10), 11))
					.collect(),
			),
			(None, vec![oversized]),
		] {
			let mut state = State {
				guilds: vec![Guild {
					default_message_notifications: None,
					stickers: None,
					emojis: None,
					id: Id(1),
					name: "One".into(),
					icon: None,
				}],
				channels: original.clone(),
				selected: Some(Id(100)),
				freshness: Freshness::Fresh,
				..State::default()
			};
			state.drafts.insert(Id(100), "keep this draft".into());
			state.apply(Envelope {
				generation: state.generation,
				event: Event::ThreadsSync {
					removed: vec![],
					guild: Id(1),
					parents,
					threads,
				},
			});
			assert!(
				state.channels == original,
				"Invalid snapshots must not partly replace navigation"
			);
			assert_eq!(
				state.drafts.get(&Id(100)).map(String::as_str),
				Some("keep this draft")
			);
			assert_eq!(state.selected, Some(Id(100)));
			assert_eq!(state.freshness, Freshness::Stale);
		}
		for (guild, parents, threads, removed) in [
			(Id(1), None, vec![], vec![Id(100), Id(100)]),
			(
				Id(1),
				None,
				vec![channel(100, 1, Some(10), 11)],
				vec![Id(100)],
			),
			(Id(1), Some(vec![Id(11)]), vec![], vec![Id(100)]),
			(Id(2), None, vec![], vec![Id(100)]),
			(Id(1), None, vec![], vec![Id(10)]),
			(Id(1), None, vec![], vec![Id(0)]),
			(Id(1), None, vec![], vec![Id(100); MAX_NAV + 1]),
		] {
			let mut state = State {
				guilds: [1, 2]
					.into_iter()
					.map(|id| Guild {
						default_message_notifications: None,
						stickers: None,
						id: Id(id),
						name: "Synthetic".into(),
						icon: None,
						emojis: None,
					})
					.collect(),
				channels: original.clone(),
				archived_thread: Some(Id(100)),
				selected: Some(Id(100)),
				..State::default()
			};
			assert!(
				state
					.apply_threads_sync(guild, parents, threads, removed)
					.is_err()
			);
			assert!(
				state.channels == original,
				"Explicit removals are validated before any navigation mutation"
			);
			assert_eq!(state.archived_thread, Some(Id(100)));
		}
	}
}
