//! Filter frequent unrelated events out of expensive navigation cache keys.
//!
//! Keys still advance for every unclassified event and every local revision change.
//! This conservative fallback keeps new event kinds and existing local mutation paths safe.
use crate::{Event, State};

#[derive(Default)]
pub(super) struct Revisions {
	catalog_skipped: u64,
	channels_skipped: u64,
	labels_skipped: u64,
	rail_skipped: u64,
}

impl State {
	fn filtered_revision(&self, skipped: u64) -> u64 {
		self.revision
			.wrapping_sub(skipped)
			.wrapping_add(self.navigation_index.invalidations.get())
	}

	/// Guild/emoji catalog and folder rows; unrelated message traffic reuses these caches.
	pub fn catalog_revision(&self) -> u64 {
		self.filtered_revision(self.navigation_index.view_revisions.catalog_skipped)
	}

	/// Sidebar row membership/order, including DM recency and voice participants.
	pub fn channel_list_revision(&self) -> u64 {
		self.filtered_revision(self.navigation_index.view_revisions.channels_skipped)
	}

	/// Channel and role names used by timeline mentions.
	pub fn channel_labels_revision(&self) -> u64 {
		self.filtered_revision(self.navigation_index.view_revisions.labels_skipped)
	}

	/// Notification rail membership and unread/mention badges.
	pub fn rail_revision(&self) -> u64 {
		self.filtered_revision(self.navigation_index.view_revisions.rail_skipped)
	}

	pub(super) fn filter_view_revisions(&mut self, event: &Event) {
		let guild_message = matches!(event, Event::Message(message)
			if self.channel(message.channel).is_some_and(|channel| channel.guild.is_some()));
		let revisions = &mut self.navigation_index.view_revisions;
		// These events cannot replace guild catalogs or channel/role labels. Message
		// arrival still changes unread badges and DM sorting; member lists affect text
		// geometry, so the timeline keeps its ordinary revision/fingerprint checks.
		if matches!(
			event,
			Event::Message(_)
				| Event::Patch(_)
				| Event::Delete { .. }
				| Event::DeleteBulk { .. }
				| Event::ReadState(_)
				| Event::Reactions(_)
				| Event::Members(_)
				| Event::Voice(_)
		) {
			revisions.catalog_skipped = revisions.catalog_skipped.wrapping_add(1);
			revisions.labels_skipped = revisions.labels_skipped.wrapping_add(1);
		}
		let metadata_unchanged = matches!(
			event,
			Event::Patch(_) | Event::Reactions(_) | Event::Members(_)
		);
		if metadata_unchanged || guild_message {
			revisions.channels_skipped = revisions.channels_skipped.wrapping_add(1);
		}
		if metadata_unchanged {
			revisions.rail_skipped = revisions.rail_skipped.wrapping_add(1);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Envelope;
	use model::{Channel, Id};

	fn keys(state: &State) -> [u64; 4] {
		[
			state.catalog_revision(),
			state.channel_list_revision(),
			state.channel_labels_revision(),
			state.rail_revision(),
		]
	}

	#[test]
	fn message_churn_preserves_catalogs_and_guild_rows_but_updates_dm_order_and_badges() {
		let mut state = State {
			channels: vec![Channel {
				id: Id(1),
				guild: Some(Id(10)),
				parent_id: None,
				position: 0,
				name: "Synthetic".into(),
				kind: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..State::default()
		};
		for id in 1..=10 {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Message(crate::tests::message(id)),
			});
			assert_eq!(keys(&state), [0, 0, 0, id]);
		}
		state.channels[0].guild = None;
		state.channels[0].kind = 1;
		state.invalidate_navigation();
		let before = keys(&state);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(crate::tests::message(11)),
		});
		assert_eq!(
			keys(&state),
			[before[0], before[1] + 1, before[2], before[3] + 1]
		);
		assert_eq!(state.channel(Id(1)).unwrap().last_message, Some(Id(11)));
	}

	#[test]
	fn local_changes_explicit_invalidation_and_unclassified_events_refresh_all_keys() {
		let mut state = State::default();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(crate::tests::message(1)),
		});
		let mut before = keys(&state);
		state.revision += 1;
		assert_eq!(keys(&state), before.map(|key| key + 1));
		before = keys(&state);
		state.invalidate_navigation();
		assert_eq!(keys(&state), before.map(|key| key + 1));
		before = keys(&state);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Disconnected,
		});
		assert!(
			keys(&state)
				.into_iter()
				.zip(before)
				.all(|(after, before)| after > before)
		);
		before = keys(&state);
		state.apply(Envelope {
			generation: state.generation + 1,
			event: Event::Message(crate::tests::message(2)),
		});
		assert_eq!(keys(&state), before);
		state.logout();
		assert_eq!(keys(&state), [0; 4]);
	}
}
