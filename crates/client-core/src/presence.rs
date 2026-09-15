use crate::{Freshness, Id, State};
use model::{MemberPresence, Patch, RichActivity};

pub const MAX_DIRECT_PRESENCES: usize = 256;
pub const MAX_DIRECT_PRESENCE_BYTES: usize = 512 * 1024;

pub const MAX_LOCAL_GAME_ACTIVITY_BYTES: usize = 4 * 1024;

#[derive(Clone)]
pub struct Update {
	pub user: Id,
	pub status: Patch<String>,
	pub custom_status: Patch<String>,
	pub activities: Patch<Vec<RichActivity>>,
}
impl Update {
	pub fn heap_bytes(&self) -> usize {
		let text = |patch: &Patch<String>| match patch {
			Patch::Value(text) => text.capacity(),
			_ => 0,
		};
		text(&self.status)
			+ text(&self.custom_status)
			+ match &self.activities {
				Patch::Value(activities) => activity_bytes(activities),
				_ => 0,
			}
	}
	pub fn merge(&mut self, mut newer: Self) {
		if matches!(&newer.status, Patch::Null)
			|| matches!(&newer.status, Patch::Value(status) if status == "offline")
		{
			newer.activities = Patch::Null;
			newer.custom_status = Patch::Null;
		}
		if !matches!(newer.status, Patch::Absent) {
			self.status = newer.status;
		}
		if !matches!(newer.custom_status, Patch::Absent) {
			self.custom_status = newer.custom_status;
		}
		if !matches!(newer.activities, Patch::Absent) {
			self.activities = newer.activities;
		}
	}
	pub fn resolve(&self, previous: Option<&MemberPresence>) -> MemberPresence {
		let text = |patch: &Patch<String>, old: Option<&String>| match patch {
			Patch::Absent => old.cloned(),
			Patch::Null => None,
			Patch::Value(value) => Some(value.clone()),
		};
		let status = text(&self.status, previous.and_then(|p| p.status.as_ref()));
		let offline = matches!(self.status, Patch::Null) || status.as_deref() == Some("offline");
		MemberPresence {
			user: self.user,
			custom_status: if offline {
				None
			} else {
				text(
					&self.custom_status,
					previous.and_then(|p| p.custom_status.as_ref()),
				)
			},
			activities: if offline {
				vec![]
			} else {
				match &self.activities {
					Patch::Absent => previous.map_or_else(Vec::new, |p| p.activities.clone()),
					Patch::Null => vec![],
					Patch::Value(activities) => activities.clone(),
				}
			},
			status,
		}
	}
}

fn activity_bytes(activities: &Vec<RichActivity>) -> usize {
	activities.capacity() * size_of::<RichActivity>()
		+ activities
			.iter()
			.map(RichActivity::heap_bytes)
			.sum::<usize>()
}

fn online(presence: &MemberPresence) -> bool {
	matches!(presence.status.as_deref(), Some("online" | "idle" | "dnd"))
}

pub fn projected_row_bytes(row: &model::Member, update: &MemberPresence) -> usize {
	if row.status == update.status
		&& row.custom_status == update.custom_status
		&& row.activities == update.activities
	{
		return row.bytes();
	}
	row.bytes()
		- row.status.as_ref().map_or(0, String::capacity)
		- row.custom_status.as_ref().map_or(0, String::capacity)
		- activity_bytes(&row.activities)
		+ update.status.as_ref().map_or(0, String::len)
		+ update.custom_status.as_ref().map_or(0, String::len)
		+ update.activities.len() * size_of::<RichActivity>()
		+ update
			.activities
			.iter()
			.map(|a| {
				a.name.len()
					+ a.details.as_ref().map_or(0, String::len)
					+ a.state.as_ref().map_or(0, String::len)
					+ a.image.as_ref().map_or(0, |image| match image {
						model::ActivityImage::Proxy(path) => path.len(),
						_ => 0,
					})
			})
			.sum::<usize>()
}

impl State {
	/// Invalidates Online membership, without invalidating on activity-only updates.
	/// Connection state and relationship visibility are separate cache keys.
	pub fn direct_presence_epoch(&self) -> u64 {
		self.direct_presence_epoch
	}

	pub fn local_game_activity(&self) -> Option<&RichActivity> {
		(self.user.is_some()
			&& (self.demo
				|| (self.gateway_connected && self.auth == crate::auth::AuthState::Authenticated)))
			.then_some(self.local_game_activity.as_ref())
			.flatten()
	}

	/// Updates only local presentation, never the remote presence caches or timeline revision.
	pub fn set_local_game_activity(&mut self, activity: Option<RichActivity>) -> bool {
		if activity.as_ref().is_some_and(|activity| {
			!activity.valid() || activity.heap_bytes() > MAX_LOCAL_GAME_ACTIVITY_BYTES
		}) {
			return false;
		}
		let next = activity.filter(|_| {
			self.user.is_some()
				&& (self.demo
					|| (self.gateway_connected
						&& self.auth == crate::auth::AuthState::Authenticated))
		});
		if self.local_game_activity == next {
			return false;
		}
		self.local_game_activity = next;
		true
	}

	fn known_presence_user(&self, user: Id) -> bool {
		if self.friend_username(user).is_some() && self.user_blocked(user) == Some(false) {
			return true;
		}
		self.channels.iter().any(|c| {
			c.guild.is_none()
				&& matches!(c.kind, 1 | 3)
				&& self.can_view(c.id)
				&& c.recipients.iter().any(|recipient| recipient.id == user)
		})
	}
	pub fn presence_for(&self, user: Id) -> Option<&MemberPresence> {
		if !self.gateway_connected || !self.known_presence_user(user) {
			return None;
		}
		self.direct_presences.iter().find(|p| p.user == user)
	}
	pub(crate) fn apply_direct_presence(&mut self, updates: &[Update]) {
		if !self.gateway_connected || updates.len() > 100 {
			return;
		}
		let mut membership_changed = false;
		let mut retained_bytes = self
			.direct_presence_bytes
			.filter(|(count, _)| *count == self.direct_presences.len())
			.map_or_else(
				|| {
					self.direct_presences
						.iter()
						.map(MemberPresence::heap_bytes)
						.sum::<usize>()
				},
				|(_, bytes)| bytes,
			);
		for update in updates {
			if !self.known_presence_user(update.user) {
				continue;
			}
			let index = self
				.direct_presences
				.iter()
				.position(|p| p.user == update.user);
			let resolved = update.resolve(index.map(|i| &self.direct_presences[i]));
			if !resolved.valid() || resolved.heap_bytes() > MAX_DIRECT_PRESENCE_BYTES {
				continue;
			}
			if index.is_some_and(|i| self.direct_presences[i] == resolved) {
				continue;
			}
			membership_changed |=
				index.is_some_and(|i| online(&self.direct_presences[i])) != online(&resolved);
			if index.is_some_and(|i| self.direct_presences[i].status.as_deref() == Some("offline"))
				&& matches!(resolved.status.as_deref(), Some("online" | "idle" | "dnd"))
			{
				self.notify_friend_change(
					update.user,
					model::notification_settings::SocialKind::FriendsOnline,
				);
			}
			if let Some(index) = index {
				retained_bytes -= self.direct_presences.remove(index).heap_bytes();
			}
			// ponytail: at most 256 records; FIFO eviction avoids a second cache index.
			while !self.direct_presences.is_empty()
				&& (self.direct_presences.len() >= MAX_DIRECT_PRESENCES
					|| retained_bytes
						+ resolved.heap_bytes()
						+ MAX_DIRECT_PRESENCES * size_of::<MemberPresence>()
						> MAX_DIRECT_PRESENCE_BYTES)
			{
				let evicted = self.direct_presences.remove(0);
				membership_changed |= online(&evicted);
				retained_bytes -= evicted.heap_bytes();
			}
			retained_bytes += resolved.heap_bytes();
			self.direct_presences.push(resolved);
		}
		if membership_changed {
			self.direct_presence_epoch = self.direct_presence_epoch.wrapping_add(1);
		}
		self.direct_presence_bytes = Some((self.direct_presences.len(), retained_bytes));
		if let Some(list) = &mut self.members
			&& list.guild.is_none()
			&& list.freshness == Freshness::Fresh
		{
			let mut bytes = list
				.rows
				.iter()
				.flatten()
				.map(model::Member::bytes)
				.sum::<usize>();
			for row in list.rows.iter_mut().flatten() {
				let presence = self.direct_presences.iter().find(|p| p.user == row.user.id);
				if let Some(presence) = presence {
					if row.status == presence.status
						&& row.custom_status == presence.custom_status
						&& row.activities == presence.activities
					{
						continue;
					}
					let projected = projected_row_bytes(row, presence);
					if bytes - row.bytes() + projected > 128 * 1024 {
						continue;
					}
					bytes = bytes - row.bytes() + projected;
				} else {
					bytes -= row.status.as_ref().map_or(0, String::capacity)
						+ row.custom_status.as_ref().map_or(0, String::capacity)
						+ activity_bytes(&row.activities);
				}
				row.status = presence.and_then(|p| p.status.clone());
				row.custom_status = presence.and_then(|p| p.custom_status.clone());
				row.activities = presence.map_or_else(Vec::new, |p| p.activities.clone());
			}
		}
	}
	pub(crate) fn prune_direct_presence(&mut self) {
		self.direct_presence_bytes = None;
		let old = std::mem::take(&mut self.direct_presences);
		let mut membership_changed = false;
		self.direct_presences = old
			.into_iter()
			.filter(|p| {
				let retained = self.known_presence_user(p.user);
				membership_changed |= !retained && online(p);
				retained
			})
			.collect();
		if membership_changed {
			self.direct_presence_epoch = self.direct_presence_epoch.wrapping_add(1);
		}
	}
	pub(crate) fn clear_direct_presences(&mut self) {
		if self.direct_presences.iter().any(online) {
			self.direct_presence_epoch = self.direct_presence_epoch.wrapping_add(1);
		}
		self.direct_presences.clear();
		self.direct_presence_bytes = None;
	}
	pub(crate) fn apply_member_presence(
		&mut self,
		guild: Id,
		channel: Id,
		request: u64,
		updates: &[model::MemberPresence],
	) {
		if !self.gateway_connected
			|| self.selected != Some(channel)
			|| !self.can_view(channel)
			|| !self.channels.iter().any(|known| {
				known.id == channel && known.guild == Some(guild) && known.supports_text()
			}) || updates.len() > 100
			|| updates.iter().enumerate().any(|(index, update)| {
				!update.valid()
					|| updates[..index]
						.iter()
						.any(|other| other.user == update.user)
			}) {
			return;
		}
		let Some(list) = self.members.as_mut().filter(|list| {
			list.guild == Some(guild)
				&& list.channel == channel
				&& list.request == request
				&& list.freshness == Freshness::Fresh
		}) else {
			return;
		};
		// ponytail: at most 100 loaded rows and updates; index only if the pane cap grows.
		let projected_bytes = list
			.rows
			.iter()
			.flatten()
			.map(|row| {
				let Some(update) = updates.iter().find(|update| update.user == row.user.id) else {
					return row.bytes();
				};
				projected_row_bytes(row, update)
			})
			.sum::<usize>();
		if projected_bytes > 128 * 1024 {
			return;
		}
		for row in list.rows.iter_mut().flatten() {
			if let Some(update) = updates.iter().find(|update| update.user == row.user.id)
				&& (row.status != update.status
					|| row.custom_status != update.custom_status
					|| row.activities != update.activities)
			{
				row.status = update.status.clone();
				row.custom_status = update.custom_status.clone();
				row.activities = update.activities.clone();
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event, auth::AuthState};
	use model::{Channel, Guild, Member, MemberList, MemberPresence, User};

	fn state() -> State {
		let user = User {
			id: Id(2),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		let mut state = State {
			user: Some(user.clone()),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			selected: Some(Id(1)),
			guilds: vec![Guild {
				id: Id(10),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			channels: vec![Channel {
				id: Id(1),
				guild: Some(Id(10)),
				parent_id: None,
				position: 0,
				name: "Synthetic".into(),
				kind: 0,
				recipients: vec![],
				icon: None,
				member_list_id: Some("everyone".into()),
				message_count: None,
				last_message: None,
			}],
			members: Some(MemberList {
				guild: Some(Id(10)),
				channel: Id(1),
				request: 7,
				rows: vec![
					Some(Member {
						roles: vec![],
						user,
						nick: None,
						status: Some("online".into()),
						custom_status: None,
						activities: vec![],
					}),
					None,
				],
				total: 200,
				freshness: Freshness::Fresh,
			}),
			..State::default()
		};
		crate::tests::grant_permissions(&mut state);
		state
	}

	fn event(updates: Vec<MemberPresence>) -> Event {
		Event::MemberPresence {
			guild: Id(10),
			channel: Id(1),
			request: 7,
			updates,
		}
	}

	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}

	fn update(user: u64, status: Option<&str>, custom: Option<&str>) -> MemberPresence {
		MemberPresence {
			user: Id(user),
			status: status.map(str::to_owned),
			custom_status: custom.map(str::to_owned),
			activities: vec![],
		}
	}

	fn row(state: &State) -> &Member {
		state.members.as_ref().unwrap().rows[0].as_ref().unwrap()
	}

	#[test]
	fn local_game_activity_is_bounded_coalesced_and_cleared_at_session_boundaries() {
		let game = RichActivity {
			kind: 0,
			name: "osu!".into(),
			details: Some("Playing a map".into()),
			state: None,
			image: None,
		};
		let mut state = state();
		let revision = state.revision;
		assert!(state.set_local_game_activity(Some(game.clone())));
		let allocation = state.local_game_activity().unwrap().name.as_ptr();
		assert!(!state.set_local_game_activity(Some(game.clone())));
		assert_eq!(
			state.local_game_activity().unwrap().name.as_ptr(),
			allocation
		);
		let mut invalid = game.clone();
		invalid.name = "x".repeat(129);
		assert!(!state.set_local_game_activity(Some(invalid)));
		let mut oversized = game.clone();
		oversized.name = String::with_capacity(MAX_LOCAL_GAME_ACTIVITY_BYTES + 1);
		oversized.name.push_str("osu!");
		assert!(!state.set_local_game_activity(Some(oversized)));
		assert_eq!(state.local_game_activity(), Some(&game));
		assert_eq!(state.revision, revision);
		for event in [
			Event::Disconnected,
			Event::Resync,
			Event::Failure(crate::auth::Failure::Expired),
		] {
			state.auth = AuthState::Authenticated;
			state.gateway_connected = true;
			state.set_local_game_activity(Some(game.clone()));
			apply(&mut state, event);
			assert!(state.local_game_activity.is_none());
		}
		state.auth = AuthState::Authenticated;
		state.gateway_connected = false;
		assert!(!state.set_local_game_activity(Some(game.clone())));
		state.gateway_connected = true;
		state.auth = AuthState::Authenticating;
		assert!(!state.set_local_game_activity(Some(game.clone())));
		state.demo = true;
		assert!(state.set_local_game_activity(Some(game.clone())));
		state.user = None;
		assert!(state.local_game_activity().is_none());
		assert!(state.set_local_game_activity(None));
		state = self::state();
		state.set_local_game_activity(Some(game.clone()));
		apply(
			&mut state,
			Event::Ready {
				permissions: model::permissions::Snapshot::default(),
				user: User {
					id: Id(999),
					name: "Different account".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				},
				guilds: vec![],
				channels: vec![],
			},
		);
		assert!(state.local_game_activity.is_none());
		state = self::state();
		state.set_local_game_activity(Some(game));
		state.logout();
		assert!(state.local_game_activity.is_none());
	}

	#[test]
	fn complete_presence_values_preserve_replace_and_clear_without_timeline_churn() {
		let mut state = state();
		state.members.as_mut().unwrap().rows[0]
			.as_mut()
			.unwrap()
			.custom_status = Some("Old custom status".into());
		let revision = state.revision;
		let resident = (
			state.resident_history_rows(),
			state.resident_history_bytes(),
		);
		// Gateway resolves an absent activity field before delivering this complete record.
		apply(
			&mut state,
			event(vec![update(2, Some("idle"), Some("Old custom status"))]),
		);
		assert_eq!(row(&state).status.as_deref(), Some("idle"));
		assert_eq!(
			row(&state).custom_status.as_deref(),
			Some("Old custom status")
		);
		apply(
			&mut state,
			event(vec![
				update(2, Some("idle"), Some("\u{1f642} Synthetic status")),
				update(3, Some("online"), Some("Unloaded row")),
			]),
		);
		assert_eq!(
			row(&state).custom_status.as_deref(),
			Some("\u{1f642} Synthetic status")
		);
		let allocation = row(&state).custom_status.as_ref().unwrap().as_ptr();
		apply(
			&mut state,
			event(vec![update(
				2,
				Some("idle"),
				Some("\u{1f642} Synthetic status"),
			)]),
		);
		assert_eq!(
			row(&state).custom_status.as_ref().unwrap().as_ptr(),
			allocation
		);
		let list = state.members.as_ref().unwrap();
		assert_eq!(list.total, 200);
		assert_eq!(list.rows.len(), 2);
		assert!(list.rows[1].is_none());
		apply(
			&mut state,
			event(vec![update(2, None, Some("\u{1f642} Synthetic status"))]),
		);
		assert!(row(&state).status.is_none());
		assert_eq!(
			row(&state).custom_status.as_deref(),
			Some("\u{1f642} Synthetic status")
		);
		apply(&mut state, event(vec![update(2, Some("offline"), None)]));
		assert_eq!(row(&state).status.as_deref(), Some("offline"));
		assert!(row(&state).custom_status.is_none());
		assert_eq!(state.revision, revision);
		assert_eq!(
			(
				state.resident_history_rows(),
				state.resident_history_bytes()
			),
			resident
		);
	}

	#[test]
	fn invalid_batches_are_rejected_atomically_including_allocation_bounds() {
		let mut state = state();
		for updates in [
			vec![update(2, None, None), update(2, Some("idle"), None)],
			vec![update(2, None, None), update(0, None, None)],
			vec![update(2, Some("unknown"), None)],
			(1..=101).map(|id| update(id, None, None)).collect(),
			{
				let mut status = String::with_capacity(crate::MAX_MEMBER_PRESENCE_BYTES);
				status.push_str("idle");
				vec![MemberPresence {
					user: Id(2),
					status: Some(status),
					custom_status: None,
					activities: vec![],
				}]
			},
			{
				let mut custom = String::with_capacity(crate::MAX_MEMBER_PRESENCE_BYTES);
				custom.push_str("Bounded text, excessive allocation");
				vec![MemberPresence {
					user: Id(2),
					status: None,
					custom_status: Some(custom),
					activities: vec![],
				}]
			},
			{
				let mut updates = Vec::with_capacity(crate::MAX_MEMBER_PRESENCE_BYTES);
				updates.push(update(2, None, None));
				updates
			},
		] {
			apply(&mut state, event(updates));
			assert_eq!(row(&state).status.as_deref(), Some("online"));
			assert!(row(&state).custom_status.is_none());
		}
		for custom in [
			"".into(),
			" ".into(),
			" padded".into(),
			"padded ".into(),
			"control\ntext".into(),
			"control\u{7f}".into(),
			"x".repeat(129),
			"\u{1f642}".repeat(129),
		] {
			// One invalid unloaded row invalidates the batch before the valid loaded row changes.
			apply(
				&mut state,
				event(vec![update(2, None, None), update(3, None, Some(&custom))]),
			);
			assert_eq!(row(&state).status.as_deref(), Some("online"));
		}
		let maximum = "\u{1f642}".repeat(128);
		let full_batch = event(
			(1..=100)
				.map(|id| update(id, Some("dnd"), Some(&maximum)))
				.collect(),
		);
		assert!(full_batch.bytes() > 8 * 1024);
		assert!(full_batch.bytes() <= crate::MAX_MEMBER_PRESENCE_BYTES);
		apply(&mut state, full_batch);
		assert_eq!(row(&state).status.as_deref(), Some("dnd"));
		assert_eq!(row(&state).custom_status.as_deref(), Some(maximum.as_str()));
	}

	#[test]
	fn projected_row_budget_counts_custom_text_and_retained_equal_allocations() {
		let mut state = state();
		let first = state.members.as_mut().unwrap().rows[0].as_mut().unwrap();
		first.custom_status = Some(String::with_capacity(1024));
		first.custom_status.as_mut().unwrap().push_str("Same text");
		let mut second = first.clone();
		second.user.id = Id(3);
		second.custom_status = None;
		let spare = 128 * 1024 - first.bytes() - second.bytes();
		first.nick = Some("x".repeat(spare));
		state.members.as_mut().unwrap().rows[1] = Some(second);
		assert_eq!(
			state
				.members
				.as_ref()
				.unwrap()
				.rows
				.iter()
				.flatten()
				.map(Member::bytes)
				.sum::<usize>(),
			128 * 1024
		);
		// Equal updates retain their existing capacity; they cannot manufacture room for another row.
		apply(
			&mut state,
			event(vec![
				update(2, Some("online"), Some("Same text")),
				update(3, Some("online"), Some("x")),
			]),
		);
		assert!(
			state.members.as_ref().unwrap().rows[1]
				.as_ref()
				.unwrap()
				.custom_status
				.is_none()
		);
		// Replacing the first value releases its old allocation and admits the complete batch.
		apply(
			&mut state,
			event(vec![
				update(2, Some("idle"), Some("Replacement")),
				update(3, Some("online"), Some("x")),
			]),
		);
		assert_eq!(row(&state).custom_status.as_deref(), Some("Replacement"));
		assert_eq!(
			state.members.as_ref().unwrap().rows[1]
				.as_ref()
				.unwrap()
				.custom_status
				.as_deref(),
			Some("x")
		);
		assert!(
			state
				.members
				.as_ref()
				.unwrap()
				.rows
				.iter()
				.flatten()
				.map(Member::bytes)
				.sum::<usize>()
				<= 128 * 1024
		);
	}

	#[test]
	fn stale_scope_disconnect_and_access_loss_cannot_change_presence() {
		for change in 0..10 {
			let mut state = state();
			let mut envelope = Envelope {
				generation: state.generation,
				event: event(vec![update(2, None, Some("Late custom status"))]),
			};
			match change {
				0 => envelope.generation += 1,
				1 => state.members.as_mut().unwrap().guild = Some(Id(11)),
				2 => state.members.as_mut().unwrap().channel = Id(3),
				3 => state.members.as_mut().unwrap().request += 1,
				4 => state.members.as_mut().unwrap().freshness = Freshness::Loading,
				5 => state.gateway_connected = false,
				6 => state.selected = Some(Id(3)),
				7 => state.permissions = Default::default(),
				8 => state.channels[0].guild = None,
				_ => {
					state.close_members();
				}
			}
			state.apply(envelope);
			if state.members.is_some() {
				assert_eq!(row(&state).status.as_deref(), Some("online"));
				assert!(row(&state).custom_status.is_none());
			}
		}
	}

	fn direct_state() -> State {
		let mut state = state();
		state.user.as_mut().unwrap().id = Id(50);
		state.channels[0].guild = None;
		state.channels[0].kind = 1;
		state.channels[0].recipients = vec![row(&state).user.clone()];
		state.members = None;
		state
	}
	#[test]
	fn friend_presence_without_a_dm_is_retained_but_strangers_are_not() {
		let mut state = direct_state();
		let mut friend = state.user.as_ref().unwrap().clone();
		friend.id = Id(999);
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::UserAction(crate::user_actions::Event::Friends(Some(vec![(
				friend.clone(),
				"friend".into(),
			)]))),
		});
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::UserAction(crate::user_actions::Event::Relationship {
				user: friend.id,
				blocked: false,
			}),
		});
		state.apply_direct_presence(&[
			direct_update(999, Patch::Value("online".into()), Patch::Value(vec![])),
			direct_update(998, Patch::Value("online".into()), Patch::Value(vec![])),
		]);
		assert_eq!(
			state.presence_for(friend.id).unwrap().status.as_deref(),
			Some("online")
		);
		assert!(state.presence_for(Id(998)).is_none());
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::UserAction(crate::user_actions::Event::Friend {
				user: friend.id,
				friend: false,
				profile: None,
			}),
		});
		assert!(state.presence_for(friend.id).is_none());
	}

	fn direct_update(
		user: u64,
		status: Patch<String>,
		activities: Patch<Vec<RichActivity>>,
	) -> Update {
		Update {
			user: Id(user),
			status,
			activities,
			custom_status: Patch::Absent,
		}
	}
	fn activity() -> RichActivity {
		RichActivity {
			kind: 0,
			name: "Synthetic game".into(),
			details: Some("In a match".into()),
			state: None,
			image: None,
		}
	}
	#[test]
	fn activity_artwork_is_included_in_projected_member_bytes() {
		let mut state = state();
		let row = state.members.as_mut().unwrap().rows[0].as_mut().unwrap();
		let mut activity = activity();
		let mut path = String::from("external/synthetic-hash-01/https/example.com/art.png");
		path.reserve(2048);
		activity.image = Some(model::ActivityImage::Proxy(path));
		let update = MemberPresence {
			user: row.user.id,
			status: row.status.clone(),
			custom_status: row.custom_status.clone(),
			activities: vec![activity],
		};
		let projected = projected_row_bytes(row, &update);
		row.activities = update.activities.clone();
		assert_eq!(projected, row.bytes());
	}
	#[test]
	fn direct_activity_patches_are_scoped_clearable_and_do_not_churn_timeline() {
		let mut state = direct_state();
		let revision = state.revision;
		let initial_epoch = state.direct_presence_epoch();
		let playing = || {
			direct_update(
				2,
				Patch::Value("online".into()),
				Patch::Value(vec![activity()]),
			)
		};
		apply(&mut state, Event::DirectPresence(vec![playing()]));
		assert!(state.direct_presence_epoch() > initial_epoch);
		let online_epoch = state.direct_presence_epoch();
		let presence = state.presence_for(Id(2)).unwrap();
		assert_eq!(presence.activities, vec![activity()]);
		let allocation = presence.activities.as_ptr();
		apply(&mut state, Event::DirectPresence(vec![playing()]));
		assert_eq!(
			state.presence_for(Id(2)).unwrap().activities.as_ptr(),
			allocation
		);
		apply(
			&mut state,
			Event::DirectPresence(vec![direct_update(
				2,
				Patch::Value("idle".into()),
				Patch::Absent,
			)]),
		);
		assert_eq!(
			state.presence_for(Id(2)).unwrap().activities,
			vec![activity()]
		);
		apply(
			&mut state,
			Event::DirectPresence(vec![direct_update(
				999,
				Patch::Value("online".into()),
				Patch::Value(vec![activity()]),
			)]),
		);
		assert_eq!(state.direct_presences.len(), 1);
		assert_eq!(state.revision, revision);
		assert_eq!(state.direct_presence_epoch(), online_epoch);
		state.request_members();
		assert_eq!(row(&state).activities, vec![activity()]);
		for clear in [Patch::Null, Patch::Value(vec![])] {
			apply(
				&mut state,
				Event::DirectPresence(vec![direct_update(2, Patch::Absent, clear)]),
			);
			assert!(state.presence_for(Id(2)).unwrap().activities.is_empty());
			assert!(row(&state).activities.is_empty());
			apply(&mut state, Event::DirectPresence(vec![playing()]));
			assert_eq!(state.direct_presence_epoch(), online_epoch);
		}
		for status in [Patch::Value("offline".into()), Patch::Null] {
			let previous_epoch = state.direct_presence_epoch();
			apply(
				&mut state,
				Event::DirectPresence(vec![direct_update(2, status, Patch::Absent)]),
			);
			assert!(state.presence_for(Id(2)).unwrap().activities.is_empty());
			assert!(state.direct_presence_epoch() > previous_epoch);
			let offline_epoch = state.direct_presence_epoch();
			apply(
				&mut state,
				Event::DirectPresence(vec![direct_update(
					2,
					Patch::Value("online".into()),
					Patch::Absent,
				)]),
			);
			assert!(state.presence_for(Id(2)).unwrap().activities.is_empty());
			assert!(state.direct_presence_epoch() > offline_epoch);
			apply(&mut state, Event::DirectPresence(vec![playing()]));
		}
		let connected_epoch = state.direct_presence_epoch();
		apply(&mut state, Event::Disconnected);
		assert!(state.presence_for(Id(2)).is_none());
		assert_eq!(state.direct_presences[0].activities, vec![activity()]);
		apply(&mut state, Event::Resumed);
		assert_eq!(state.direct_presence_epoch(), connected_epoch);
		assert_eq!(
			state.presence_for(Id(2)).unwrap().activities,
			vec![activity()]
		);
		apply(
			&mut state,
			Event::DirectPresence(vec![direct_update(
				2,
				Patch::Value("idle".into()),
				Patch::Absent,
			)]),
		);
		assert_eq!(
			state.presence_for(Id(2)).unwrap().activities,
			vec![activity()]
		);
		for transition in [Event::Resync, Event::Failure(crate::auth::Failure::Expired)] {
			let mut state = direct_state();
			apply(&mut state, Event::DirectPresence(vec![playing()]));
			let epoch = state.direct_presence_epoch();
			apply(&mut state, transition);
			assert!(state.direct_presences.is_empty());
			assert!(state.direct_presence_epoch() > epoch);
		}
		state.logout();
		state.apply(crate::Envelope {
			generation: state.generation - 1,
			event: Event::DirectPresence(vec![playing()]),
		});
		assert!(state.direct_presences.is_empty());
	}

	#[test]
	fn direct_presence_epoch_rejects_invalid_updates_and_clears_at_ready() {
		let mut state = direct_state();
		for status in [Patch::Null, Patch::Value("offline".into())] {
			apply(
				&mut state,
				Event::DirectPresence(vec![direct_update(2, status, Patch::Absent)]),
			);
			assert_eq!(state.direct_presence_epoch(), 0);
		}
		apply(
			&mut state,
			Event::DirectPresence(vec![direct_update(
				2,
				Patch::Value("dnd".into()),
				Patch::Absent,
			)]),
		);
		let epoch = state.direct_presence_epoch();
		for user in [2, 999] {
			apply(
				&mut state,
				Event::DirectPresence(vec![direct_update(
					user,
					Patch::Value("invalid".into()),
					Patch::Absent,
				)]),
			);
			assert_eq!(state.direct_presence_epoch(), epoch);
		}
		state.apply(Envelope {
			generation: state.generation + 1,
			event: Event::DirectPresence(vec![direct_update(2, Patch::Null, Patch::Absent)]),
		});
		assert_eq!(state.direct_presence_epoch(), epoch);
		let user = state.user.clone().unwrap();
		let channels = state.channels.clone();
		apply(
			&mut state,
			Event::Ready {
				permissions: Default::default(),
				user,
				guilds: vec![],
				channels,
			},
		);
		assert!(state.direct_presence_epoch() > epoch);
		assert!(state.direct_presences.is_empty());
		let epoch = state.direct_presence_epoch();
		state.clear_direct_presences();
		assert_eq!(state.direct_presence_epoch(), epoch);
	}

	#[test]
	fn activity_only_byte_eviction_invalidates_online_membership() {
		let mut state = direct_state();
		let large_activity = RichActivity {
			name: "\u{1f642}".repeat(128),
			details: Some("\u{1f642}".repeat(128)),
			state: Some("\u{1f642}".repeat(128)),
			..activity()
		};
		let large = direct_update(
			2,
			Patch::Value("online".into()),
			Patch::Value(vec![large_activity; model::MAX_RICH_ACTIVITIES]),
		);
		let small = direct_update(2, Patch::Value("online".into()), Patch::Absent);
		let full_count = (MAX_DIRECT_PRESENCE_BYTES
			- MAX_DIRECT_PRESENCES * size_of::<MemberPresence>()
			- small.resolve(None).heap_bytes())
			/ large.resolve(None).heap_bytes();
		assert!(full_count + 1 < MAX_DIRECT_PRESENCES);
		let channel = state.channels[0].clone();
		for index in 0..=full_count {
			let user = Id(index as u64 + 2);
			if index > 0 {
				let mut channel = channel.clone();
				channel.id = Id(index as u64 + 1000);
				channel.recipients[0].id = user;
				state.channels.push(channel);
			}
			let mut next = if index == full_count {
				small.clone()
			} else {
				large.clone()
			};
			next.user = user;
			state.apply_direct_presence(&[next]);
		}
		assert_eq!(state.direct_presences.len(), full_count + 1);
		let epoch = state.direct_presence_epoch();
		let revision = state.revision;
		let mut next = large;
		next.user = Id(full_count as u64 + 2);
		next.status = Patch::Absent;
		state.apply_direct_presence(&[next]);
		assert_eq!(state.direct_presences.len(), full_count);
		assert!(state.direct_presence_epoch() > epoch);
		assert!(state.presence_for(Id(2)).is_none());
		assert_eq!(state.revision, revision);
	}

	#[test]
	fn direct_presence_capacity_and_recipient_removal_retire_cached_activity() {
		let mut state = direct_state();
		let channel = state.channels[0].clone();
		for id in 3..=300 {
			let mut channel = channel.clone();
			channel.id = Id(1000 + id);
			channel.recipients[0].id = Id(id);
			state.channels.push(channel);
		}
		for id in 2..=300 {
			apply(
				&mut state,
				Event::DirectPresence(vec![direct_update(
					id,
					Patch::Value("online".into()),
					Patch::Value(vec![activity()]),
				)]),
			);
		}
		assert_eq!(state.direct_presences.len(), MAX_DIRECT_PRESENCES);
		assert!(
			state
				.direct_presences
				.iter()
				.map(MemberPresence::heap_bytes)
				.sum::<usize>()
				+ state.direct_presences.capacity() * size_of::<MemberPresence>()
				<= MAX_DIRECT_PRESENCE_BYTES
		);
		assert!(state.presence_for(Id(2)).is_none());
		let epoch = state.direct_presence_epoch();
		apply(
			&mut state,
			Event::DirectPresence(vec![direct_update(
				2,
				Patch::Value("offline".into()),
				Patch::Absent,
			)]),
		);
		assert!(
			state.direct_presence_epoch() > epoch,
			"offline insertion evicts an online entry at the item cap"
		);
		let epoch = state.direct_presence_epoch();
		apply(
			&mut state,
			Event::RecipientRemoved {
				channel: Id(1300),
				user: Id(300),
			},
		);
		assert!(state.presence_for(Id(300)).is_none());
		assert!(!state.direct_presences.iter().any(|p| p.user == Id(300)));
		assert!(state.direct_presence_epoch() > epoch);
	}
}
