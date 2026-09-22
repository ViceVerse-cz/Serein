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
		match &envelope.event {
			Event::Startup(_) | Event::Ready { .. } => changes.0 = [true; 5],
			Event::ProfileEdited { user, .. } | Event::Profile { user, .. }
				if state.user.as_ref().is_some_and(|own| own.id == *user) =>
			{
				changes.0[0] = true
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
			| Event::Permissions(_)
			| Event::PermissionsChanged
			| Event::Unavailable(_) => changes.0[1] = true,
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
	pub fn read_changed(&mut self) {
		self.0[4] = true;
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

/// The selected summary also changes through local mark-read commands outside the event drain.
#[derive(PartialEq, Eq)]
pub struct ReadKey(Option<Id>, Option<bool>, u32);
impl ReadKey {
	pub fn capture(state: &State) -> Self {
		let selected = state.selected.filter(|id| {
			state.gateway_connected
				&& state.can_view(*id)
				&& state.freshness != model::Freshness::Unavailable
		});
		Self(
			selected,
			selected.and_then(|id| state.unread(id)),
			selected.map_or(0, |id| state.mention_count(id)),
		)
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
}
