//! Bounded invalidation hints for granted app data, never copied account payloads.
use client_core::{Envelope, Event, State};
use extensions::{AppEventKind, Capability};
use model::Id;

const KINDS: [AppEventKind; 5] = [
	AppEventKind::Account,
	AppEventKind::Channels,
	AppEventKind::Members,
	AppEventKind::Presence,
	AppEventKind::ReadState,
];

#[derive(Default, Clone, Copy)]
pub struct Changes([bool; 5]);
impl Changes {
	pub fn capture(state: &State, envelope: &Envelope) -> Self {
		let mut changes = Self::default();
		if envelope.generation != state.generation {
			return changes;
		}
		let readable = |channel| {
			state.selected == Some(channel)
				&& state.gateway_connected
				&& state.can_view(channel)
				&& state.can_read_history(channel)
				&& state.freshness != model::Freshness::Unavailable
		};
		match &envelope.event {
			Event::Startup(_) | Event::Ready { .. } => changes.0 = [true; 5],
			Event::ProfileEdited { user, .. } | Event::Profile { user, .. }
				if state.user.as_ref().is_some_and(|own| own.id == *user) =>
			{
				changes.0[0] = true;
				// Own-user edits also refresh copies in loaded recipients/member rows.
				changes.0[1] = true;
				changes.0[2] = true;
			}
			Event::GuildJoined(_)
			| Event::GuildChanged(_)
			| Event::ChannelCreated(_)
			| Event::ChannelRestored(_)
			| Event::ChannelChanged(_)
			| Event::ThreadChanged { .. }
			| Event::ThreadRemoved { .. }
			| Event::ThreadsSync { .. }
			| Event::ServerAction(_)
			| Event::ServerSettings(_)
			| Event::ChannelAction(_)
			| Event::GroupAction(_)
			| Event::UserAction(_)
			| Event::PostCreated { result: Ok(_), .. }
			| Event::ForumPosts { result: Ok(_), .. }
			| Event::Permissions(_)
			| Event::PermissionsChanged
			| Event::Unavailable(_) => changes.0[1] = true,
			Event::Message(message)
			| Event::SendResult {
				result: Ok(message),
				..
			} if readable(message.channel) && !message.ephemeral && message.flags & 64 == 0 => {
				changes.0[1] = true
			}
			Event::History {
				channel, request, ..
			} if readable(*channel) && *request == state.request => changes.0[1] = true,
			Event::Delete { channel, .. } | Event::DeleteBulk { channel, .. }
				if readable(*channel) =>
			{
				changes.0[1] = true
			}
			Event::ReadState(client_core::read_state::Event::Latest(entries))
				if entries.iter().any(|(channel, _)| readable(*channel)) =>
			{
				changes.0[1] = true
			}
			Event::Members(members)
				if state.selected == Some(members.channel)
					&& state.member_request == members.request =>
			{
				changes.0[2] = true;
				changes.0[3] = true;
			}
			Event::RecipientAdded { channel, .. } | Event::RecipientRemoved { channel, .. }
				if state.selected == Some(*channel) =>
			{
				changes.0[1] = true;
				changes.0[2] = true;
				changes.0[3] = true;
			}
			Event::MemberPresence {
				channel, request, ..
			} if state.selected == Some(*channel) && state.member_request == *request => changes.0[3] = true,
			Event::DirectPresence(updates)
				if state
					.selected
					.and_then(|id| state.channel(id))
					.filter(|channel| channel.guild.is_none())
					.is_some_and(|channel| {
						updates.iter().any(|update| {
							channel.recipients.iter().any(|user| user.id == update.user)
						})
					}) =>
			{
				changes.0[3] = true
			}
			_ => {}
		}
		changes
	}
	pub fn merge(&mut self, other: Self) {
		for (changed, next) in self.0.iter_mut().zip(other.0) {
			*changed |= next;
		}
	}
	pub fn kinds(self, capabilities: &[Capability]) -> impl Iterator<Item = AppEventKind> {
		KINDS
			.into_iter()
			.zip(self.0)
			.filter_map(move |(kind, changed)| {
				(changed && kind.data_granted(capabilities)).then_some(kind)
			})
	}
}

/// Fixed scalar keys catch local loading/navigation mutations outside the event drain.
#[derive(PartialEq, Eq)]
pub struct DataKey {
	read: (Option<Id>, Option<bool>, u32),
	profile: (u64, bool, bool, bool, bool),
	channel: Option<(Id, Option<Id>, Option<u32>)>,
	directory: (usize, usize),
}
impl DataKey {
	pub fn capture(state: &State) -> Self {
		let selected = state.selected.filter(|id| {
			state.gateway_connected
				&& state.can_view(*id)
				&& state.freshness != model::Freshness::Unavailable
		});
		let profile = &state.own_profile;
		Self {
			read: (
				selected,
				selected.and_then(|id| state.unread(id)),
				selected.map_or(0, |id| state.mention_count(id)),
			),
			profile: (
				profile.request,
				profile.loading,
				profile.reload_required,
				profile.error.is_some(),
				profile.data.is_some(),
			),
			channel: selected
				.filter(|_| state.freshness == model::Freshness::Fresh)
				.and_then(|id| state.channel(id))
				.map(|channel| {
					let readable = state.can_read_history(channel.id);
					(
						channel.id,
						channel.last_message.filter(|_| readable),
						channel.message_count.filter(|_| readable),
					)
				}),
			directory: (state.channels.len(), state.guilds.len()),
		}
	}
	pub fn changed(&self, old: &Self) -> Changes {
		Changes([
			self.profile != old.profile,
			self.channel != old.channel || self.directory != old.directory,
			false,
			false,
			self.read != old.read,
		])
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn data_hints_require_current_generation_context_and_grants() {
		let state = test_support::demo_state();
		let envelope = Envelope {
			generation: state.generation,
			event: Event::MemberPresence {
				guild: Id(10),
				channel: state.selected.unwrap(),
				request: state.member_request,
				updates: Vec::new(),
			},
		};
		let changes = Changes::capture(&state, &envelope);
		assert_eq!(
			changes.kinds(&[Capability::Presence]).collect::<Vec<_>>(),
			vec![AppEventKind::Presence]
		);
		assert_eq!(changes.kinds(&[Capability::Members]).count(), 0);
		let mut stale = envelope;
		stale.generation = state.generation.wrapping_add(1);
		assert_eq!(
			Changes::capture(&state, &stale)
				.kinds(&[Capability::Presence])
				.count(),
			0
		);
		stale.generation = state.generation;
		if let Event::MemberPresence { channel, .. } = &mut stale.event {
			*channel = Id(999);
		}
		assert_eq!(
			Changes::capture(&state, &stale)
				.kinds(&[Capability::Presence])
				.count(),
			0
		);
	}
	#[test]
	fn selected_message_metadata_is_invalidated_without_failed_send_or_other_channel_hints() {
		let state = test_support::demo_state();
		let selected = state.selected.unwrap();
		let has_channels = |event| {
			Changes::capture(
				&state,
				&Envelope {
					generation: state.generation,
					event,
				},
			)
			.kinds(&[Capability::ChannelDetails])
			.any(|kind| kind == AppEventKind::Channels)
		};
		assert!(has_channels(Event::Message(test_support::message(
			99999, selected
		))));
		assert!(!has_channels(Event::Message(test_support::message(
			99999,
			Id(999)
		))));
		assert!(!has_channels(Event::SendResult {
			nonce: "synthetic".into(),
			result: Err(client_core::auth::Failure::Network)
		}));
		let mut private = test_support::message(99999, selected);
		private.ephemeral = true;
		assert!(!has_channels(Event::Message(private)));
	}
	#[test]
	fn local_profile_and_directory_mutations_use_scalar_invalidation_keys() {
		let mut state = test_support::demo_state();
		let old = DataKey::capture(&state);
		assert_eq!(
			old.changed(&old)
				.kinds(&[Capability::AccountProfile, Capability::ChannelDetails])
				.count(),
			0
		);
		state.own_profile.loading = !state.own_profile.loading;
		assert_eq!(
			DataKey::capture(&state)
				.changed(&old)
				.kinds(&[Capability::AccountProfile])
				.collect::<Vec<_>>(),
			vec![AppEventKind::Account]
		);
		let channel = state.selected.unwrap();
		state
			.channels
			.iter_mut()
			.find(|item| item.id == channel)
			.unwrap()
			.last_message = Some(Id(99999));
		assert!(
			DataKey::capture(&state)
				.changed(&old)
				.kinds(&[Capability::ChannelDetails])
				.any(|kind| kind == AppEventKind::Channels)
		);
		let old = DataKey::capture(&state);
		state.guilds.clear();
		assert!(
			DataKey::capture(&state)
				.changed(&old)
				.kinds(&[Capability::GuildDirectory])
				.any(|kind| kind == AppEventKind::Channels)
		);
	}
}
