#![no_main]

use client_core::{Envelope, Event, State, auth::Failure, permissions::Event as PermissionEvent};
use libfuzzer_sys::fuzz_target;
use model::{Channel, Guild, Id, Message, MessagePatch, Patch, User, permissions as p};
use std::hash::{Hash, Hasher};

fn user() -> User {
	User {
		webhook: false,
		kind: Default::default(),
		id: Id(2),
		name: "Synthetic".into(),
		avatar: None,
		discriminator: 0,
		primary_guild: None,
	}
}

fn ready() -> Event {
	Event::Ready {
		user: user(),
		guilds: vec![Guild {
			stickers: None,
			id: Id(10),
			name: "Synthetic".into(),
			icon: None,
			emojis: None,
		}],
		channels: (20..23)
			.map(|id| Channel {
				id: Id(id),
				guild: Some(Id(10)),
				parent_id: None,
				kind: 0,
				name: "Synthetic".into(),
				icon: None,
				position: 0,
				recipients: vec![],
				last_message: None,
				message_count: None,
				member_list_id: None,
			})
			.collect(),
		permissions: p::Snapshot {
			guilds: vec![p::Guild {
				id: Id(10),
				owner: Some(Id(999)),
				roles: Some(vec![p::Role {
					id: Id(10),
					bits: p::VIEW_CHANNEL | p::READ_MESSAGE_HISTORY | p::SEND_MESSAGES,
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
				}]),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
			}],
			channels: (20..23)
				.map(|id| p::Channel {
					id: Id(id),
					guild: Id(10),
					overwrites: Some(vec![]),
				})
				.collect(),
		},
	}
}

fn payload(selector: u8, remaining: &mut usize) -> String {
	// Repeated bytes reach the actual 64 KiB admission boundary and aggregate
	// eviction without needing a huge corpus. Total expansion is capped per input.
	let size = [0, 1, 1024, 65_535, 65_536, 65_537][usize::from(selector % 6)].min(*remaining);
	*remaining -= size;
	"x".repeat(size)
}

fn message(id: Id, channel: Id, content: String) -> Message {
	Message {
		sticker_items: vec![],
		flags: 0,
		ephemeral: false,
		components: vec![],
		application_id: None,
		id,
		channel,
		author: user(),
		author_nick: None,
		author_roles: vec![],
		content,
		kind: 0,
		reactions: Some(vec![]),
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
		attachments: vec![],
		mention_roles: vec![],
		mention_everyone: false,
		suppress_notifications: false,
		mentions: vec![],
		embeds_suppressed: false,
	}
}

fn apply(state: &mut State, event: Event) {
	// A message/page cannot replace bodies already known deleted in this window.
	// Do not carry this oracle across selection, logout, or deliberate cache reset.
	let guarded: Vec<Id> = match &event {
		Event::Message(message) if state.selected == Some(message.channel) => state
			.timeline
			.is_deleted(message.id)
			.then_some(message.id)
			.into_iter()
			.collect(),
		Event::History {
			channel, messages, ..
		} if state.selected == Some(*channel) => messages
			.iter()
			.filter(|message| state.timeline.is_deleted(message.id))
			.map(|message| message.id)
			.collect(),
		_ => vec![],
	};
	state.apply(Envelope {
		generation: state.generation,
		event,
	});
	for id in guarded {
		assert!(state.timeline.get(id).is_none());
	}
}

fn invariants(state: &State) {
	assert!(state.timeline.row_count() <= session_cache::MAX_MESSAGES);
	assert!(state.timeline.bytes() <= session_cache::MAX_BYTES);
	assert!(state.resident_window_count() <= 2);
	assert!(state.resident_history_rows() <= 1475);
	assert!(state.resident_history_bytes() <= 16 * 1024 * 1024);
	let mut previous = None;
	for id in state.timeline.row_ids() {
		assert!(previous.is_none_or(|old| old < id));
		previous = Some(id);
		if state.timeline.is_deleted(id) {
			assert!(state.timeline.get(id).is_none());
		}
	}
	assert_eq!(state.timeline.len(), state.timeline.iter().count());
	for message in state.timeline.iter() {
		assert_eq!(Some(message.channel), state.selected);
		assert!(!state.timeline.is_deleted(message.id));
		assert!(state.can_view(message.channel));
	}
}

// Compare stable public reading state for deliberately stale events. The reducer
// may clear its per-event effect buffer, so do not demand whole-State equality.
fn fingerprint(state: &State) -> u64 {
	let mut hash = std::collections::hash_map::DefaultHasher::new();
	// Even an ignored current-generation response increments the UI revision.
	(
		state.generation,
		state.selected,
		state.request,
		state.history_pending,
		state.gateway_connected,
	)
		.hash(&mut hash);
	for channel in &state.channels {
		channel.id.hash(&mut hash);
	}
	for id in state.timeline.row_ids() {
		id.hash(&mut hash);
	}
	for message in state.timeline.iter() {
		(
			message.id,
			message.channel,
			message.revision,
			&message.content,
		)
			.hash(&mut hash);
	}
	(
		state.resident_window_count(),
		state.resident_history_rows(),
		state.resident_history_bytes(),
	)
		.hash(&mut hash);
	hash.finish()
}

fuzz_target!(|data: &[u8]| {
	if data.len() > 16 * 1024 {
		return;
	}
	let mut state = State::default();
	apply(&mut state, ready());
	state.select(Id(20));
	let request = state.request;
	apply(
		&mut state,
		Event::History {
			channel: Id(20),
			request,
			older: false,
			messages: vec![message(Id(1), Id(20), "seed".into())],
		},
	);
	let mut remaining = 24 * 1024 * 1024;
	// Eight-byte operations: opcode, channel, ID little-endian, payload size,
	// page count, flags, spare. Partial trailing operations are intentionally ignored.
	for op in data.chunks_exact(8).take(256) {
		let channel = Id(20 + u64::from(op[1] % 3));
		let id = Id(1 + u64::from(u16::from_le_bytes([op[2], op[3]])));
		match op[0] % 20 {
			0 => {
				state.select(channel);
			}
			1 => {
				let before = (op[6] & 1 != 0).then_some(Id(id.0 + 51));
				state.history(before);
			}
			2 | 3 => {
				let stale = op[0] % 20 == 3;
				let prior = stale.then(|| fingerprint(&state));
				let request = if stale {
					state.request.wrapping_sub(1)
				} else {
					state.request
				};
				let older = state.history_before.is_some();
				let messages = (0..usize::from(op[5] % 52))
					.map(|offset| {
						message(
							Id(id.0 + offset as u64),
							channel,
							payload(op[4], &mut remaining),
						)
					})
					.collect();
				apply(
					&mut state,
					Event::History {
						channel,
						request,
						older,
						messages,
					},
				);
				if let Some(prior) = prior {
					assert_eq!(fingerprint(&state), prior);
				}
			}
			4 => {
				let message = message(id, channel, payload(op[4], &mut remaining));
				apply(&mut state, Event::Message(message));
			}
			5 => {
				let content = match op[6] % 3 {
					0 => Patch::Absent,
					1 => Patch::Null,
					_ => Patch::Value(payload(op[4], &mut remaining)),
				};
				apply(
					&mut state,
					Event::Patch(MessagePatch {
						sticker_items: Patch::Absent,
						flags: Patch::Absent,
						components: Patch::Absent,
						application_id: Patch::Absent,
						channel,
						id,
						content,
						extra_content: Default::default(),
						reactions: Patch::Absent,
						mentions: Patch::Absent,
						edited: Patch::Absent,
						embeds: Patch::Absent,
						attachments: Patch::Absent,
						embeds_suppressed: Patch::Absent,
					}),
				);
			}
			6 => apply(&mut state, Event::Delete { channel, id }),
			7 => apply(
				&mut state,
				Event::DeleteBulk {
					channel,
					ids: (0..usize::from(op[5] % 101))
						.map(|offset| Id(id.0 + offset as u64))
						.collect(),
				},
			),
			8 | 9 => {
				let revoked = op[0] % 20 == 8;
				apply(
					&mut state,
					Event::Permissions(PermissionEvent::Channel {
						channel,
						guild: Some(Id(10)),
						overwrites: Patch::Value(if revoked {
							vec![p::Overwrite {
								id: Id(2),
								kind: 1,
								allow: 0,
								deny: p::VIEW_CHANNEL,
							}]
						} else {
							vec![]
						}),
					}),
				);
				if revoked && state.selected == Some(channel) {
					assert!(state.timeline.is_empty());
				}
			}
			10 => apply(&mut state, Event::Disconnected),
			11 => apply(&mut state, Event::Resumed),
			12 => apply(&mut state, Event::Resync),
			13 => {
				state.logout();
				assert!(state.timeline.is_empty());
				assert_eq!(state.resident_window_count(), 0);
				assert!(state.pending.is_empty());
				assert!(state.user.is_none());
			}
			14 => apply(&mut state, ready()),
			15 => {
				let before = fingerprint(&state);
				let revision = state.revision;
				state.apply(Envelope {
					generation: state.generation.wrapping_sub(1),
					event: Event::Delete { channel, id },
				});
				assert_eq!(fingerprint(&state), before);
				assert_eq!(state.revision, revision);
			}
			16 => {
				let request = state.request;
				apply(
					&mut state,
					Event::HistoryFailed {
						channel,
						request,
						failure: Failure::Forbidden,
					},
				);
			}
			17 => {
				let mut source = message(Id(id.0 + 1), channel, payload(op[4], &mut remaining));
				source.kind = 19;
				source.reply_to = Some(id);
				source.reply_deleted = true;
				apply(&mut state, Event::Message(source));
			}
			18 => {
				// A bounded live burst also reaches eviction while a page is pending.
				for offset in 0..usize::from(op[5] % 51) {
					let message = message(
						Id(id.0 + offset as u64),
						channel,
						payload(op[4], &mut remaining),
					);
					apply(&mut state, Event::Message(message));
					invariants(&state);
				}
			}
			_ => {
				state.clear_cached_history();
			}
		}
		invariants(&state);
	}
});
