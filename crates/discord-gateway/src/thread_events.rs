//! Thread dispatch uses existing channel/history behavior; it never joins a thread.
use client_core::{Event, auth::Failure};
use discord_protocol::{
	ChannelDto, decode,
	threads::{ThreadMembers, ThreadUpdate},
};
use model::{Id, Patch};

pub(super) fn decode_event(
	kind: &str,
	bytes: &[u8],
	owner: Option<Id>,
) -> Result<Option<Event>, Failure> {
	let protocol = |_| Failure::Protocol;
	Ok(match kind {
		"THREAD_CREATE" | "THREAD_DELETE" => {
			let mut channel: ChannelDto = decode(bytes).map_err(protocol)?;
			let guild = channel.guild_id.ok_or(Failure::Protocol)?;
			let hidden = channel.is_obfuscated();
			// Validate the thread identity/scope even for a visibility removal. This
			// sanitized temporary is never emitted as an accessible channel.
			channel.flags &= !(1 << 17);
			let channel =
				discord_protocol::threads::into_thread(channel, guild).map_err(protocol)?;
			Some(if kind == "THREAD_DELETE" || hidden {
				Event::ThreadRemoved {
					guild,
					id: channel.id,
				}
			} else {
				Event::ChannelCreated(channel)
			})
		}
		"THREAD_UPDATE" => {
			let update: ThreadUpdate = decode(bytes).map_err(protocol)?;
			if matches!(update.patch.kind, Patch::Null | Patch::Value(0..=9 | 13..=u8::MAX))
				|| matches!(update.patch.parent_id, Patch::Null)
				|| matches!(update.patch.parent_id, Patch::Value(id) if id == update.patch.id)
			{
				return Err(Failure::Protocol);
			}
			Some(
				if update.patch.is_obfuscated()
					|| update.thread_metadata.is_some_and(|m| m.archived)
				{
					Event::ThreadRemoved {
						guild: update.guild_id,
						id: update.patch.id,
					}
				} else {
					Event::ThreadChanged {
						guild: update.guild_id,
						patch: update.patch.into_model(),
					}
				},
			)
		}
		"THREAD_LIST_SYNC" => {
			let sync = decode::<discord_protocol::threads::ThreadListSync>(bytes)
				.map_err(protocol)?
				.into_model()
				.map_err(protocol)?;
			Some(Event::ThreadsSync {
				guild: sync.guild,
				parents: sync.parents,
				threads: sync.threads,
				removed: sync.removed,
			})
		}
		"THREAD_MEMBERS_UPDATE" => {
			let members: ThreadMembers = decode(bytes).map_err(protocol)?;
			owner
				.filter(|id| members.removed_member_ids.contains(id))
				.map(|_| Event::ThreadRemoved {
					guild: members.guild_id,
					id: members.id,
				})
		}
		_ => None,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn hidden_sync_revokes_a_transient_in_one_event_after_full_validation() {
		let parent = decode::<ChannelDto>(br#"{"id":"2","guild_id":"1","type":0,"name":"Parent"}"#)
			.unwrap()
			.into_model();
		let thread = decode::<ChannelDto>(
			br#"{"id":"14","guild_id":"1","parent_id":"2","type":11,"name":"Browsed archive"}"#,
		)
		.unwrap()
		.into_model();
		let mut state = client_core::State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			guilds: vec![model::Guild {
				id: Id(1),
				name: "Synthetic".into(),
				icon: None,
				stickers: None,
				emojis: None,
			}],
			channels: vec![parent, thread],
			archived_thread: Some(Id(14)),
			selected: Some(Id(14)),
			..client_core::State::default()
		};
		state.drafts.insert(Id(14), "Preserved draft".into());
		state.history(None);
		let request = state.request;
		let message = decode::<discord_protocol::MessageDto>(br#"{"id":"99","channel_id":"14","author":{"id":"7","username":"Synthetic"},"content":"Previously visible"}"#).unwrap().into_model();
		state.timeline.seed_cache(vec![message.clone()]).unwrap();
		let hidden: Vec<_> = (3..=14)
			.map(|id| {
				serde_json::json!({
					"id":id.to_string(),"parent_id":"2","type":11,"flags":131072
				})
			})
			.collect();
		let mut invalid = hidden.clone();
		invalid.push(hidden[0].clone());
		let invalid = serde_json::to_vec(
			&serde_json::json!({"guild_id":"1","channel_ids":["2"],"threads":invalid}),
		)
		.unwrap();
		assert!(
			decode_event("THREAD_LIST_SYNC", &invalid, None).is_err(),
			"Malformed later rows must produce no partial removal event"
		);
		assert_eq!(state.archived_thread, Some(Id(14)));
		let body = serde_json::to_vec(
			&serde_json::json!({"guild_id":"1","channel_ids":["2"],"threads":hidden}),
		)
		.unwrap();
		let event = decode_event("THREAD_LIST_SYNC", &body, None)
			.unwrap()
			.unwrap();
		assert!(
			matches!(&event, Event::ThreadsSync { removed, threads, .. }
            if removed.len() == 12 && removed.len() > client_core::EVENT_SLOTS && threads.is_empty()),
			"All explicit revocations fit into one bounded queue slot"
		);
		assert!(event.bytes() < client_core::MAX_EVENT_BYTES);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event,
		});
		assert!(state.archived_thread.is_none());
		assert!(state.channels.iter().all(|c| c.id != Id(14)));
		assert!(state.timeline.is_empty());
		assert!(!state.history_pending);
		assert_eq!(state.freshness, model::Freshness::Unavailable);
		assert_eq!(state.drafts[&Id(14)], "Preserved draft");
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: Event::History {
				channel: Id(14),
				request,
				older: false,
				messages: vec![message],
			},
		});
		assert!(state.timeline.is_empty());
	}

	#[test]
	fn thread_dispatch_rejects_malformed_scope_and_only_removes_the_owner() {
		for kind in ["THREAD_CREATE", "THREAD_UPDATE"] {
			assert!(matches!(
				decode_event(
					kind,
					br#"{"id":"3","guild_id":"1","parent_id":"2","type":11,"flags":131072}"#,
					None
				),
				Ok(Some(Event::ThreadRemoved {
					guild: Id(1),
					id: Id(3)
				}))
			));
			assert!(
				decode_event(
					kind,
					br#"{"id":"3","guild_id":"1","parent_id":"3","type":11,"flags":131072}"#,
					None
				)
				.is_err()
			);
		}
		for body in [
			r#"{"id":"3","guild_id":"1","type":0}"#,
			r#"{"id":"3","guild_id":"1","type":null}"#,
			r#"{"id":"3","guild_id":"1","parent_id":null}"#,
			r#"{"id":"3","guild_id":"1","parent_id":"3"}"#,
			r#"{"id":"3","name":"missing guild"}"#,
		] {
			assert!(decode_event("THREAD_UPDATE", body.as_bytes(), Some(Id(9))).is_err());
		}
		let removal = br#"{"id":"3","guild_id":"1","removed_member_ids":["9"]}"#;
		assert!(matches!(
			decode_event("THREAD_MEMBERS_UPDATE", removal, None),
			Ok(None)
		));
		assert!(matches!(
			decode_event("THREAD_MEMBERS_UPDATE", removal, Some(Id(8))),
			Ok(None)
		));
		assert!(matches!(
			decode_event("THREAD_MEMBERS_UPDATE", removal, Some(Id(9))),
			Ok(Some(Event::ThreadRemoved {
				guild: Id(1),
				id: Id(3)
			}))
		));
		let oversized =
			serde_json::json!({"id":"3","guild_id":"1","removed_member_ids":vec!["9";4001]});
		assert!(
			decode_event(
				"THREAD_MEMBERS_UPDATE",
				&serde_json::to_vec(&oversized).unwrap(),
				Some(Id(9))
			)
			.is_err()
		);
	}
}
