//! Synthetic navigation and shared target-window regressions; no transport is run.
use super::{
	Reply,
	tests::{apply, chat_link_state, deleted_source, message, state},
};
use crate::{
	Command, Envelope, Event, Pending, ReadingCursor, State,
	auth::{AuthState, Failure},
};
use model::{Delivery, Freshness, Id, Message, archives, permissions as p};

fn in_channel(id: u64, channel: u64) -> Message {
	let mut message = message(id);
	message.channel = Id(channel);
	message
}

fn complete(state: &mut State, messages: Vec<Message>) {
	let event = Event::History {
		channel: state.selected.unwrap(),
		request: state.request,
		older: state.history_before.is_some(),
		messages,
	};
	apply(state, event);
}

fn request(command: Command) -> (Id, u64) {
	let Command::History {
		channel, request, ..
	} = command
	else {
		panic!("navigation must only request history")
	};
	(channel, request)
}

fn keep_work(state: &mut State) {
	state.drafts.insert(Id(2), "Other draft".into());
	state.reply = Some(Reply::to(Id(100)));
	state.pending.push(Pending {
		sticker: None,
		channel: Id(1),
		content: "Do not resend".into(),
		attachments: vec![],
		nonce: "synthetic-pending".into(),
		delivery: Delivery::Ambiguous,
		confirmed: None,
	});
}

fn assert_work(state: &State) {
	assert_eq!(state.drafts[&Id(1)], "Unsent draft");
	assert_eq!(state.drafts[&Id(2)], "Other draft");
	assert_eq!(state.pending.len(), 1);
	assert_eq!(state.pending[0].content, "Do not resend");
	assert_eq!(state.pending[0].delivery, Delivery::Ambiguous);
	assert_eq!(state.send_sequence, 0);
	assert!(state.voice.active.is_none());
}

#[test]
fn fresh_loaded_links_jump_offline_without_history_or_draft_side_effects() {
	let mut state = chat_link_state();
	keep_work(&mut state);
	state.gateway_connected = false;
	state.restore_scroll = true;
	let request = state.request;
	let payload = state.timeline.get(Id(100)).unwrap().content.as_ptr();
	assert!(
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
			.unwrap()
			.is_none()
	);
	assert_eq!(state.selected, Some(Id(1)));
	assert_eq!(state.search_target, Some(Id(100)));
	assert!(!state.restore_scroll && !state.history_pending);
	assert_eq!(state.freshness, Freshness::Fresh);
	assert_eq!(state.request, request);
	assert_eq!(
		state.timeline.get(Id(100)).unwrap().content.as_ptr(),
		payload
	);
	assert_eq!(state.reply_target(), Some(Id(100)));
	assert_work(&state);

	for (auth, freshness) in [
		(AuthState::Unauthenticated, Freshness::Fresh),
		(AuthState::Authenticated, Freshness::Stale),
		(AuthState::Authenticated, Freshness::Unavailable),
	] {
		let mut state = chat_link_state();
		state.auth = auth;
		state.freshness = freshness;
		state.gateway_connected = false;
		assert!(
			state
				.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
				.is_err()
		);
		assert!(state.search_target.is_none());
	}
}

#[test]
fn resident_links_skip_recent_and_saved_cursor_fetches_even_offline() {
	for offline in [false, true] {
		let mut state = chat_link_state();
		keep_work(&mut state);
		state.remember_reading(
			Id(1),
			ReadingCursor {
				message: Some(Id(50)), // This saved boundary is not loaded.
				inset: 24.0,
			},
		);
		let payload = state.timeline.get(Id(100)).unwrap().content.as_ptr();
		state.select(Id(2)).unwrap();
		complete(&mut state, vec![in_channel(200, 2)]);
		assert_eq!(state.resident_window_count(), 1);
		if offline {
			apply(&mut state, Event::Disconnected);
		}
		let request = state.request;
		assert!(
			state
				.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
				.unwrap()
				.is_none()
		);
		assert_eq!(state.selected, Some(Id(1)));
		assert_eq!(state.search_target, Some(Id(100)));
		assert!(!state.restore_scroll && !state.history_pending);
		assert_eq!(state.request, request + 1, "cancel only; no eager fetch");
		assert_eq!(state.freshness, Freshness::Fresh);
		assert_eq!(
			state.timeline.get(Id(100)).unwrap().content.as_ptr(),
			payload
		);
		assert_work(&state);
	}
}

#[test]
fn offline_rejected_links_do_not_activate_missing_deleted_or_unrelated_residents() {
	for destination in 0..5 {
		let mut state = chat_link_state();
		keep_work(&mut state);
		state.timeline.delete(Id(50)).unwrap();
		state.select(Id(2)).unwrap();
		complete(&mut state, vec![in_channel(200, 2)]);
		apply(&mut state, Event::Disconnected);
		assert_eq!(state.resident_window_count(), 1);
		let (channel, target) = match destination {
			0 => (Id(1), Id(999)), // Resident exists but does not contain the target.
			1 => (Id(3), Id(100)), // Another channel's resident is not this destination.
			2 => (Id(1), Id(50)),  // Resident tombstone must not change conversations.
			3 => {
				state.channels[0].kind = 5; // Cached identity no longer matches.
				(Id(1), Id(100))
			}
			_ => (Id(2), Id(200)), // Active window was made stale by disconnect.
		};
		state.reply = Some(Reply::to(Id(200)));
		state.search_target = Some(Id(200));
		state.restore_scroll = true;
		let request = state.request;
		let revision = state.revision;
		let back = state.trail.peek_back();
		let payload = state.timeline.get(Id(200)).unwrap().content.as_ptr();
		let error = state.open_chat_link(
			Some(Id(if channel == Id(3) { 20 } else { 10 })),
			channel,
			Some(target),
		);
		assert!(error.is_err(), "destination {destination}");
		assert_eq!(state.selected, Some(Id(2)));
		assert_eq!(state.freshness, Freshness::Stale);
		assert!(!state.history_pending);
		assert_eq!(state.request, request);
		assert_eq!(state.revision, revision);
		assert!(state.trail.peek_back() == back);
		assert_eq!(state.resident_window_count(), 1);
		assert_eq!(state.search_target, Some(Id(200)));
		assert!(state.restore_scroll);
		assert_eq!(state.reply_target(), Some(Id(200)));
		assert_eq!(
			state.timeline.get(Id(200)).unwrap().content.as_ptr(),
			payload
		);
		assert_work(&state);
	}
}

#[test]
fn actual_disconnect_keeps_loaded_rows_stale_until_revalidation() {
	let mut state = chat_link_state();
	keep_work(&mut state);
	let payload = state.timeline.get(Id(100)).unwrap().content.as_ptr();
	apply(&mut state, Event::Disconnected);
	let request = state.request;
	assert!(!state.gateway_connected);
	assert_eq!(state.auth, AuthState::Authenticated);
	assert_eq!(state.freshness, Freshness::Stale);
	assert!(
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
			.is_err()
	);
	assert_eq!(state.request, request);
	assert_eq!(state.freshness, Freshness::Stale);
	assert!(state.search_target.is_none() && !state.history_pending);
	assert_eq!(
		state.timeline.get(Id(100)).unwrap().content.as_ptr(),
		payload
	);
	assert_work(&state);
	// Resume also leaves the active window stale; an online link revalidates it.
	apply(&mut state, Event::Resumed);
	let command = state
		.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
		.unwrap()
		.unwrap();
	assert!(matches!(
		command,
		Command::History {
			before: Some(Id(101)),
			..
		}
	));
	assert_eq!(state.freshness, Freshness::Loading);
	complete(&mut state, vec![message(100)]);
	assert_eq!(state.freshness, Freshness::Fresh);
	assert_eq!(state.search_target, Some(Id(100)));
}

#[test]
fn local_and_remote_links_preserve_an_existing_call_without_voice_commands() {
	for destination in 0..3 {
		let mut state = chat_link_state();
		let mut voice = state.channels[0].clone();
		voice.id = Id(6);
		voice.kind = 2;
		state.channels.push(voice);
		assert!(matches!(
			state.start_call(Id(6), false),
			Some(Command::Voice(_))
		));
		let call = state.voice.active.as_mut().unwrap();
		call.phase = crate::voice::Phase::Connected;
		call.muted = true;
		call.deafened = true;
		let voice_request = call.request;
		state.drafts.insert(Id(2), "Other draft".into());
		let result = match destination {
			0 => {
				state.gateway_connected = false;
				state.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
			}
			1 => state.open_chat_link(Some(Id(10)), Id(2), Some(Id(50))),
			_ => {
				apply(&mut state, Event::Disconnected);
				state.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
			}
		};
		if destination == 1 {
			assert!(matches!(result, Ok(Some(Command::History { .. }))));
			complete(&mut state, vec![in_channel(50, 2)]);
		} else if destination == 0 {
			assert!(matches!(result, Ok(None)));
		} else {
			assert!(result.is_err());
		}
		let call = state.voice.active.as_ref().unwrap();
		assert_eq!((call.channel, call.request), (Id(6), voice_request));
		assert_eq!(call.phase, crate::voice::Phase::Connected);
		assert!(call.muted && call.deafened);
		assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		assert_eq!(state.drafts[&Id(2)], "Other draft");
		assert_eq!(state.send_sequence, 0);
	}
}

#[test]
fn unloaded_links_use_one_exact_page_for_guild_dm_group_and_known_threads() {
	for (guild, id, kind, parent_kind) in [
		(Some(Id(10)), 1, 0, None),
		(Some(Id(10)), 2, 0, None),
		(Some(Id(20)), 3, 0, None),
		(None, 4, 1, None),
		(None, 4, 3, None),
		(Some(Id(10)), 5, 10, Some(5)),
		(Some(Id(10)), 5, 11, Some(0)),
		(Some(Id(10)), 5, 12, Some(0)),
		(Some(Id(10)), 5, 11, Some(15)), // Forum post destination.
		(Some(Id(10)), 5, 11, Some(16)), // Media post destination.
	] {
		let mut state = chat_link_state();
		keep_work(&mut state);
		if id == 5 {
			let mut thread = state.channels[1].clone();
			thread.id = Id(id);
			thread.kind = kind;
			thread.parent_id = Some(Id(2));
			state.channels.push(thread);
		} else {
			state
				.channels
				.iter_mut()
				.find(|c| c.id == Id(id))
				.unwrap()
				.kind = kind;
		}
		if let Some(parent_kind) = parent_kind {
			state.channels[1].kind = parent_kind;
		}
		let command = state
			.open_chat_link(guild, Id(id), Some(Id(50)))
			.unwrap()
			.unwrap();
		assert!(matches!(
			command,
			Command::History {
				channel,
				before: Some(Id(51)),
				after: None,
				request,
			} if channel == Id(id) && request == state.request
		));
		assert!(state.history_pending && state.history_targeted);
		complete(&mut state, vec![in_channel(40, id), in_channel(50, id)]);
		assert_eq!(state.search_target, Some(Id(50)), "UI still needs the cue");
		assert!(state.timeline.get(Id(50)).is_some());
		assert!(!state.history_pending);
		assert!(
			state
				.history_navigation
				.target(Id(id), state.request)
				.is_none()
		);
		assert_work(&state);
	}
}

#[test]
fn exact_window_reports_empty_absent_and_deleted_targets_without_neighbor_jumps() {
	for outcome in 0..5 {
		let mut state = chat_link_state();
		keep_work(&mut state);
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
			.unwrap()
			.unwrap();
		match outcome {
			2 => apply(
				&mut state,
				Event::Delete {
					channel: Id(1),
					id: Id(50),
				},
			),
			3 => apply(
				&mut state,
				Event::DeleteBulk {
					channel: Id(1),
					ids: vec![Id(40), Id(50)],
				},
			),
			4 => apply(&mut state, Event::Message(deleted_source(110, 50, 1))),
			_ => {}
		}
		let messages = if outcome == 0 {
			vec![]
		} else if outcome >= 2 {
			vec![message(40), message(50)]
		} else {
			vec![message(40)]
		};
		complete(&mut state, messages);
		assert!(!state.history_pending);
		assert_eq!(state.freshness, Freshness::Fresh);
		assert!(state.timeline.get(Id(50)).is_none());
		if outcome >= 2 {
			assert_eq!(state.status, "This message was deleted");
			assert!(state.search_target.is_none());
			assert!(state.timeline.is_deleted(Id(50)));
		} else {
			assert_eq!(
				state.status,
				"Message was not returned; it may have been removed or become unavailable"
			);
			assert_eq!(state.search_target, Some(Id(50)));
		}
		assert!(
			state
				.history_navigation
				.target(Id(1), state.request)
				.is_none()
		);
		assert_work(&state);
	}
}

#[test]
fn consumed_scroll_cue_does_not_hide_missing_target_or_racing_deletion() {
	for deleted in [false, true] {
		let mut state = chat_link_state();
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
			.unwrap()
			.unwrap();
		assert_eq!(state.search_target.take(), Some(Id(50)));
		if deleted {
			apply(
				&mut state,
				Event::Delete {
					channel: Id(1),
					id: Id(50),
				},
			);
			assert_eq!(state.status, "This message was deleted");
		}
		complete(&mut state, vec![message(40)]);
		assert_eq!(
			state.status,
			if deleted {
				"This message was deleted"
			} else {
				"Message was not returned; it may have been removed or become unavailable"
			}
		);
		assert!(state.search_target.is_none());
	}
}

#[test]
fn repeated_clicks_share_pending_page_and_different_targets_supersede_it() {
	let mut state = chat_link_state();
	let (_, old) = request(
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
			.unwrap()
			.unwrap(),
	);
	let revision = state.revision;
	assert!(
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
			.unwrap()
			.is_none()
	);
	assert_eq!(state.request, old);
	assert_eq!(state.revision, revision);
	let (_, current) = request(
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(60)))
			.unwrap()
			.unwrap(),
	);
	assert!(current > old);
	assert_eq!(state.search_target, Some(Id(60)));
	for failure in [Failure::Forbidden, Failure::Network, Failure::Expired] {
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request: old,
				failure,
			},
		);
		assert_eq!(state.search_target, Some(Id(60)));
		assert_eq!(state.auth, AuthState::Authenticated);
		assert!(state.history_pending);
		assert_eq!(state.freshness, Freshness::Loading);
	}
	apply(
		&mut state,
		Event::History {
			channel: Id(1),
			request: old,
			older: true,
			messages: vec![message(50)],
		},
	);
	state.command_rejected(Command::History {
		channel: Id(1),
		before: Some(Id(51)),
		after: None,
		request: old,
	});
	assert_eq!(state.search_target, Some(Id(60)));
	assert_eq!(state.request, current);
	assert!(state.timeline.get(Id(50)).is_none());
	// Stale Forbidden must not erase a newer, independent search view either.
	state.request_search("Still open".into(), None).unwrap();
	let search_request = state.search_request;
	apply(
		&mut state,
		Event::HistoryFailed {
			channel: Id(1),
			request: old,
			failure: Failure::Forbidden,
		},
	);
	assert_eq!(state.search.as_ref().unwrap().request, search_request);
	assert_eq!(state.search_target, Some(Id(60)));
	complete(&mut state, vec![message(60)]);
	assert_eq!(state.search_target, Some(Id(60)));
	assert!(state.timeline.get(Id(60)).is_some());
}

#[test]
fn superseding_history_and_session_reset_clear_only_the_old_navigation() {
	for transition in 0..5 {
		let mut state = chat_link_state();
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
			.unwrap()
			.unwrap();
		let pending = state.request;
		match transition {
			0 => {
				state.history(Some(Id(41)));
				assert!(state.search_target.is_none());
				complete(&mut state, vec![message(40)]);
				assert_ne!(state.status, "This message was deleted");
			}
			1..=3 => {
				let mut user = state.user.clone().unwrap();
				let mut channels = state.channels.clone();
				if transition == 2 {
					user.id = Id(999);
				} else if transition == 3 {
					channels.push(channels[0].clone()); // Invalid READY duplicate.
				}
				let event = Event::Ready {
					user,
					channels,
					guilds: state.guilds.clone(),
					permissions: p::Snapshot {
						guilds: state.permissions.guilds.values().cloned().collect(),
						channels: state.permissions.channels.values().cloned().collect(),
					},
				};
				apply(&mut state, event);
			}
			_ => state.logout(),
		}
		assert!(state.history_navigation.target(Id(1), pending).is_none());
		assert!(state.search_target.is_none() && !state.restore_scroll);
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request: pending,
				older: true,
				messages: vec![message(50)],
			},
		);
		assert!(state.timeline.get(Id(50)).is_none());
	}
}

#[test]
fn switching_channels_and_generations_cannot_restore_or_settle_stale_targets() {
	let mut state = chat_link_state();
	keep_work(&mut state);
	let (old_channel, old) = request(
		state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
			.unwrap()
			.unwrap(),
	);
	let (_, current) = request(
		state
			.open_chat_link(Some(Id(10)), Id(2), Some(Id(60)))
			.unwrap()
			.unwrap(),
	);
	apply(
		&mut state,
		Event::History {
			channel: old_channel,
			request: old,
			older: true,
			messages: vec![message(50)],
		},
	);
	state.apply(Envelope {
		generation: state.generation - 1,
		event: Event::History {
			channel: Id(2),
			request: current,
			older: true,
			messages: vec![in_channel(60, 2)],
		},
	});
	assert_eq!(state.selected, Some(Id(2)));
	assert_eq!(state.search_target, Some(Id(60)));
	assert!(state.history_pending && state.timeline.is_empty());
	assert_work(&state);
	complete(&mut state, vec![in_channel(60, 2)]);
}

#[test]
fn target_failures_and_access_loss_cancel_request_identity_and_cue() {
	for failure in [
		Failure::Forbidden,
		Failure::Network,
		Failure::Capacity,
		Failure::Expired,
	] {
		let mut state = chat_link_state();
		keep_work(&mut state);
		let (_, pending) = request(
			state
				.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
				.unwrap()
				.unwrap(),
		);
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request: pending,
				failure,
			},
		);
		assert!(!state.history_pending && state.search_target.is_none());
		assert!(state.history_navigation.target(Id(1), pending).is_none());
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request: pending,
				older: true,
				messages: vec![message(50)],
			},
		);
		assert!(state.timeline.get(Id(50)).is_none());
		assert_work(&state);
	}
	for transition in 0..6 {
		let mut state = chat_link_state();
		keep_work(&mut state);
		let command = state
			.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
			.unwrap()
			.unwrap();
		let pending = state.request;
		match transition {
			0 => apply(&mut state, Event::Disconnected),
			1 => apply(&mut state, Event::Unavailable(Id(1))),
			2 => apply(&mut state, Event::PermissionsChanged),
			3 => state.open_home(),
			4 => state.command_rejected(command),
			_ => apply(
				&mut state,
				Event::Permissions(crate::permissions::Event::UnavailableGuild(Id(10))),
			),
		}
		assert!(state.search_target.is_none());
		assert!(state.history_navigation.target(Id(1), pending).is_none());
		assert_work(&state);
	}
}

#[test]
fn concrete_permission_revocations_cancel_exact_navigation_without_voice_changes() {
	for revocation in 0..3 {
		let mut state = chat_link_state();
		keep_work(&mut state);
		let mut voice = state.channels[0].clone();
		voice.id = Id(6);
		voice.kind = 2;
		state.channels.push(voice);
		let role = |id, bits| p::Role {
			id: Id(id),
			bits,
			name: String::new(),
			color: 0,
			secondary_color: None,
			tertiary_color: None,
			position: 0,
			hoist: false,
		};
		apply(
			&mut state,
			Event::Permissions(crate::permissions::Event::Snapshot(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(10),
					owner: Some(Id(999)),
					roles: Some(vec![
						role(10, p::VIEW_CHANNEL | p::CONNECT | p::SPEAK),
						role(11, p::READ_MESSAGE_HISTORY),
					]),
					member: Some(p::Member {
						roles: vec![Id(11)],
						timeout_until: None,
					}),
				}],
				channels: [1, 2, 6]
					.into_iter()
					.map(|id| p::Channel {
						id: Id(id),
						guild: Id(10),
						overwrites: Some(vec![]),
					})
					.collect(),
			})),
		);
		assert!(state.can_read_history(Id(1)));
		assert!(matches!(
			state.start_call(Id(6), false),
			Some(Command::Voice(_))
		)); // Only constructs a synthetic command; no voice transport is run.
		let call = state.voice.active.as_mut().unwrap();
		call.phase = crate::voice::Phase::Connected;
		call.muted = true;
		call.deafened = true;
		let voice_snapshot = |state: &State| {
			let call = state.voice.active.as_ref().unwrap();
			(
				(call.channel, call.guild, call.request, call.phase),
				(
					call.muted,
					call.deafened,
					call.server_muted,
					call.server_deafened,
				),
				(call.camera, call.watching, call.participants.clone()),
				(call.connected_at, call.channel_started_at, call.error),
				(
					state.voice.departed,
					state.voice.incoming,
					state.voice.ring_error,
				),
			)
		};
		let voice_before = voice_snapshot(&state);
		let (channel, pending) = request(
			state
				.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
				.unwrap()
				.unwrap(),
		);
		assert!(state.history_pending);
		assert_eq!(state.search_target, Some(Id(50)));
		assert_eq!(
			state.history_navigation.target(channel, pending),
			Some(Id(50))
		);
		let event = if revocation == 2 {
			// Remove the role that grants history, retaining independent voice access.
			crate::permissions::Event::Member {
				guild: Id(10),
				roles: model::Patch::Value(vec![]),
				timeout_until: model::Patch::Absent,
			}
		} else {
			crate::permissions::Event::Channel {
				channel,
				guild: Some(Id(10)),
				overwrites: model::Patch::Value(vec![p::Overwrite {
					id: Id(10),
					kind: 0,
					allow: 0,
					deny: if revocation == 0 {
						p::VIEW_CHANNEL
					} else {
						p::READ_MESSAGE_HISTORY
					},
				}]),
			}
		};
		apply(&mut state, Event::Permissions(event));
		assert!(!state.can_read_history(channel));
		assert_eq!(
			state.permission(channel, p::VIEW_CHANNEL | p::READ_MESSAGE_HISTORY),
			Some(false)
		);
		assert_eq!(state.can_view(channel), revocation != 0);
		assert_eq!(
			state.status,
			if revocation == 0 {
				"Channel permissions are unavailable or access was revoked"
			} else {
				"Message history is unavailable with the current permissions"
			}
		);
		assert!(!state.history_pending && state.timeline.is_empty());
		assert!(state.search_target.is_none() && !state.restore_scroll);
		assert!(state.history_navigation.target(channel, pending).is_none());
		assert!(state.request > pending);
		assert!(state.can_call(Id(6)));
		assert_eq!(voice_snapshot(&state), voice_before);
		let request = state.request;
		let status = state.status;
		let freshness = state.freshness;
		for stale in [
			Event::History {
				channel,
				request: pending,
				older: true,
				messages: vec![message(50)],
			},
			Event::HistoryFailed {
				channel,
				request: pending,
				failure: Failure::Forbidden,
			},
			Event::HistoryFailed {
				channel,
				request: pending,
				failure: Failure::Network,
			},
		] {
			apply(&mut state, stale);
			assert!(!state.history_pending && state.timeline.is_empty());
			assert!(state.search_target.is_none() && !state.restore_scroll);
			assert!(state.history_navigation.target(channel, pending).is_none());
			// Events increment the general revision even when their page is ignored;
			// the rejected repeat activation itself must not mutate navigation.
			let revision = state.revision;
			assert!(
				state
					.open_chat_link(Some(Id(10)), channel, Some(Id(50)))
					.is_err()
			);
			assert_eq!(state.selected, Some(channel));
			assert_eq!((state.request, state.revision), (request, revision));
			assert_eq!((state.status, state.freshness), (status, freshness));
			assert!(!state.can_read_history(channel));
			assert!(!state.history_pending && state.timeline.is_empty());
			assert!(state.search_target.is_none() && !state.restore_scroll);
			assert!(state.history_navigation.target(channel, request).is_none());
			assert_eq!(voice_snapshot(&state), voice_before);
		}
		assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		assert_eq!(state.drafts[&Id(2)], "Other draft");
		assert_eq!(state.pending.len(), 1);
		assert_eq!(state.pending[0].content, "Do not resend");
		assert_eq!(state.pending[0].delivery, Delivery::Ambiguous);
		assert_eq!(state.send_sequence, 0);
	}
}

#[test]
fn invalid_current_exact_pages_clear_target_and_cannot_be_resurrected() {
	for invalid in 0..5 {
		let mut state = chat_link_state();
		keep_work(&mut state);
		let (channel, pending) = request(
			state
				.open_chat_link(Some(Id(10)), Id(1), Some(Id(1000)))
				.unwrap()
				.unwrap(),
		);
		assert_eq!(
			state.history_navigation.target(channel, pending),
			Some(Id(1000))
		);
		assert_eq!(state.search_target, Some(Id(1000)));
		let messages = match invalid {
			0 => vec![in_channel(40, 2)],         // Wrong-channel payload.
			1 => vec![message(40), message(40)],  // Duplicate IDs.
			2 => vec![message(1001)],             // Exclusive before boundary violation.
			3 => (1..=51).map(message).collect(), // One bounded page allows only 50.
			_ => vec![message(1000)],             // Correct payload with incorrect older flag.
		};
		apply(
			&mut state,
			Event::History {
				channel,
				request: pending,
				older: invalid != 4,
				messages,
			},
		);
		assert!(!state.history_pending, "case {invalid}");
		assert!(state.history_navigation.target(channel, pending).is_none());
		assert!(state.search_target.is_none() && !state.restore_scroll);
		assert_ne!(state.freshness, Freshness::Fresh);
		assert!(state.timeline.is_empty());
		assert!(state.request > pending);
		let request = state.request;
		let status = state.status;
		apply(
			&mut state,
			Event::History {
				channel,
				request: pending,
				older: true,
				messages: vec![message(1000)],
			},
		);
		assert_eq!(state.request, request);
		assert_eq!(state.status, status);
		assert!(!state.history_pending);
		assert!(state.search_target.is_none());
		assert!(state.history_navigation.target(channel, pending).is_none());
		assert!(state.timeline.get(Id(1000)).is_none());
		assert_work(&state);
	}
}

#[test]
fn completed_target_is_not_rechecked_during_older_newer_or_recent_pagination() {
	let mut state = chat_link_state();
	state.channels[0].last_message = Some(Id(150));
	state
		.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
		.unwrap();
	// Use an unloaded target so this test also exercises the private request marker.
	state
		.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
		.unwrap()
		.unwrap();
	complete(&mut state, (1..=50).map(message).collect());
	assert_eq!(state.search_target.take(), Some(Id(50)));
	assert!(state.history_targeted && state.can_load_newer());
	let command = state.newer_history().unwrap();
	assert!(matches!(
		command,
		Command::History {
			after: Some(Id(50)),
			before: None,
			..
		}
	));
	state.status = "";
	complete(&mut state, (51..=100).map(message).collect());
	assert_eq!(state.status, "");
	assert!(state.search_target.is_none());
	state.history(Some(Id(1)));
	complete(&mut state, vec![]);
	assert_eq!(state.status, "");
	assert!(state.search_target.is_none());
	state.history(None);
	complete(&mut state, vec![message(150)]);
	assert!(!state.history_targeted);
	assert_eq!(state.status, "");
}

#[test]
fn shared_reply_search_and_pin_windows_report_absent_targets() {
	for source in 0..3 {
		let mut state = state();
		if source == 0 {
			state.open_reply_target(Id(50)).unwrap();
		} else {
			state.search = Some(crate::search::SearchView {
				pins: source == 2,
				channel: Id(1),
				guild: None,
				query: "Synthetic".into(),
				before: None,
				offset: 0,
				total: None,
				pin_before: None,
				request: 1,
				loading: false,
				error: None,
				page: Some(model::SearchPage {
					hits: vec![model::SearchHit {
						id: Id(50),
						channel: Id(1),
						author: message(1).author,
						mentions: vec![],
						excerpt: "Synthetic".into(),
						attachments: vec![],
						embeds: vec![],
					}],
					total: 1,
					partial: false,
					pin_cursor: None,
				}),
			});
			state.open_search_hit(Id(50)).unwrap();
		}
		complete(&mut state, vec![message(40)]);
		assert_eq!(
			state.status,
			"Message was not returned; it may have been removed or become unavailable"
		);
		assert_eq!(state.search_target, Some(Id(50)));
		assert!(state.timeline.get(Id(40)).is_some());
	}
}

fn archive_state(kind: archives::Kind) -> State {
	let mut state = chat_link_state();
	let mut thread = state.channels[1].clone();
	thread.id = Id(5);
	thread.parent_id = Some(Id(2));
	thread.kind = if kind == archives::Kind::Public {
		11
	} else {
		12
	};
	state.archives = Some(crate::archives::View {
		parent: Id(2),
		guild: Id(10),
		kind,
		before: None,
		request: state.search_request,
		loading: false,
		error: None,
		page: Some(archives::Page {
			threads: vec![thread],
			next: None,
		}),
	});
	state
}

#[test]
fn bounded_cached_archived_threads_and_forum_posts_can_be_admitted() {
	for kind in [
		archives::Kind::Public,
		archives::Kind::Private,
		archives::Kind::JoinedPrivate,
	] {
		let mut state = archive_state(kind);
		if kind == archives::Kind::Public {
			state.channels[1].kind = 15; // The link names the cached forum post, not its parent.
		}
		assert!(state.channel(Id(5)).is_none());
		request(
			state
				.open_chat_link(Some(Id(10)), Id(5), Some(Id(50)))
				.unwrap()
				.unwrap(),
		);
		assert_eq!(state.selected, Some(Id(5)));
		assert_eq!(state.archived_thread, Some(Id(5)));
		assert_eq!(state.channel(Id(5)).unwrap().parent_id, Some(Id(2)));
		complete(&mut state, vec![in_channel(50, 5)]);
		assert_eq!(state.search_target, Some(Id(50)));
		assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		assert!(state.voice.active.is_none());
	}
}

fn member_permissions(state: &mut State, bits: u128) {
	state
		.permissions
		.replace(p::Snapshot {
			guilds: vec![p::Guild {
				id: Id(10),
				owner: Some(Id(999)),
				roles: Some(vec![p::Role {
					id: Id(10),
					name: String::new(),
					color: 0,
					secondary_color: None,
					tertiary_color: None,
					position: 0,
					hoist: false,
					bits,
				}]),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
			}],
			channels: vec![p::Channel {
				id: Id(2),
				guild: Id(10),
				overwrites: Some(vec![]),
			}],
		})
		.unwrap();
}

#[test]
fn private_archive_requires_manage_threads_but_joined_archive_uses_existing_scope() {
	for (kind, permitted) in [
		(archives::Kind::Private, false),
		(archives::Kind::JoinedPrivate, true),
	] {
		let mut state = archive_state(kind);
		member_permissions(&mut state, p::VIEW_CHANNEL | p::READ_MESSAGE_HISTORY);
		assert_eq!(
			state
				.open_chat_link(Some(Id(10)), Id(5), Some(Id(50)))
				.is_ok(),
			permitted
		);
		if !permitted {
			assert!(state.channel(Id(5)).is_none());
			assert_eq!(state.selected, Some(Id(1)));
		}
	}
}

#[test]
fn disconnected_archive_message_links_do_not_retire_an_open_transient_thread() {
	for gate in 0..3 {
		let mut state = archive_state(archives::Kind::Public);
		let mut previous = state
			.archives
			.as_ref()
			.unwrap()
			.page
			.as_ref()
			.unwrap()
			.threads[0]
			.clone();
		previous.id = Id(6);
		state.channels.push(previous);
		state.archived_thread = Some(Id(6));
		state.selected = Some(Id(6));
		state.search_target = Some(Id(100));
		state.restore_scroll = true;
		state.timeline.clear();
		state
			.timeline
			.insert(in_channel(100, 6), false, false)
			.unwrap();
		state.record(crate::Place::Channel(Id(6)));
		match gate {
			0 => state.gateway_connected = false,
			1 => state.auth = AuthState::Unauthenticated,
			_ => apply(&mut state, Event::Disconnected),
		}
		let request = state.request;
		let revision = state.revision;
		let search_request = state.search_request;
		let search_target = state.search_target;
		let restore_scroll = state.restore_scroll;
		let back = state.trail.peek_back();
		let current = state.trail.current();
		let payload = state.timeline.get(Id(100)).unwrap().content.as_ptr();
		assert!(
			state
				.open_chat_link(Some(Id(10)), Id(5), Some(Id(50)))
				.is_err()
		);
		assert_eq!(state.selected, Some(Id(6)));
		assert_eq!(state.archived_thread, Some(Id(6)));
		assert!(state.channel(Id(6)).is_some() && state.channel(Id(5)).is_none());
		assert_eq!((state.request, state.revision), (request, revision));
		assert_eq!(state.search_request, search_request);
		assert_eq!(state.search_target, search_target);
		assert_eq!(state.restore_scroll, restore_scroll);
		assert!(state.trail.peek_back() == back && state.trail.current() == current);
		assert_eq!(
			state.timeline.get(Id(100)).unwrap().content.as_ptr(),
			payload
		);
		assert_eq!(state.archives.is_none(), gate == 2);
		if let Some(view) = state.archives.as_ref() {
			assert!(view.error.is_none());
		}
		assert_eq!(state.drafts[&Id(1)], "Unsent draft");
	}
}

#[test]
fn unknown_invalid_mismatched_and_inaccessible_thread_metadata_fails_closed() {
	for invalid in 0..10 {
		let mut state = archive_state(archives::Kind::Public);
		let mut guild = Some(Id(10));
		let mut channel = Id(5);
		match invalid {
			0 => channel = Id(999),
			1 => guild = Some(Id(20)),
			2 => state.archives.as_mut().unwrap().loading = true,
			3 => state.gateway_connected = false,
			4 => state.auth = AuthState::Unauthenticated,
			5 => {
				state
					.archives
					.as_mut()
					.unwrap()
					.page
					.as_mut()
					.unwrap()
					.threads[0]
					.parent_id = Some(Id(999))
			}
			6 => {
				state
					.archives
					.as_mut()
					.unwrap()
					.page
					.as_mut()
					.unwrap()
					.threads[0]
					.kind = 12
			}
			7 => state.channels[1].guild = Some(Id(20)),
			8 => member_permissions(&mut state, p::VIEW_CHANNEL),
			_ => state.channels[1].kind = 2,
		}
		let request = state.request;
		assert!(
			state.open_chat_link(guild, channel, Some(Id(50))).is_err(),
			"case {invalid}"
		);
		assert_eq!(state.selected, Some(Id(1)));
		assert_eq!(state.request, request);
		assert!(state.channel(Id(5)).is_none());
		assert!(state.timeline.get(Id(100)).is_some());
	}
	for invalid in 0..4 {
		let mut state = archive_state(archives::Kind::Public);
		let mut thread = state
			.archives
			.take()
			.unwrap()
			.page
			.unwrap()
			.threads
			.remove(0);
		match invalid {
			0 => thread.parent_id = None,
			1 => thread.parent_id = Some(Id(999)),
			2 => state.channels[1].guild = Some(Id(20)),
			_ => member_permissions(&mut state, p::VIEW_CHANNEL),
		}
		state.channels.push(thread);
		assert!(
			state
				.open_chat_link(Some(Id(10)), Id(5), Some(Id(50)))
				.is_err()
		);
		assert_eq!(state.selected, Some(Id(1)));
	}
}
