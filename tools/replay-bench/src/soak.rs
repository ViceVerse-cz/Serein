//! Offline lifecycle pressure using the shipping reducer and cache budgets.
use client_core::{Command, Envelope, Event, State};
use model::{Channel, Freshness, Id, Message, MessagePatch, Patch};
use std::time::{Duration, Instant};

fn apply(state: &mut State, event: Event) {
	state.apply(Envelope {
		generation: state.generation,
		event,
	});
	assert!(state.timeline.row_count() <= 500);
	assert!(state.timeline.bytes() <= 4 * 1024 * 1024);
	assert!(state.resident_window_count() <= 2);
	assert!(state.resident_history_rows() <= 1475);
	assert!(state.resident_history_bytes() <= 16 * 1024 * 1024 - 66 * 1024);
}

fn ready(state: &mut State) {
	apply(
		state,
		Event::Ready {
			user: test_support::message(1, Id(20)).author,
			guilds: vec![],
			permissions: Default::default(),
			channels: (20..52)
				.map(|id| Channel {
					id: Id(id),
					guild: None,
					parent_id: None,
					kind: 1,
					position: 0,
					name: format!("Synthetic soak {id}"),
					recipients: vec![],
					last_message: None,
					member_list_id: None,
					message_count: None,
					icon: None,
				})
				.collect(),
		},
	);
}

fn message(id: u64, channel: Id, large: bool) -> Message {
	let mut message = test_support::message(id, channel);
	message.content = "x".repeat(if large { 16 * 1024 } else { 128 });
	message
}

fn request(command: Command) -> u64 {
	let Command::History { request, .. } = command else {
		panic!("History required")
	};
	request
}

fn page(state: &mut State, channel: Id, request: u64, base: u64, large: bool) {
	apply(
		state,
		Event::History {
			channel,
			request,
			older: false,
			messages: (1..=50)
				.map(|id| message(base + id, channel, large))
				.collect(),
		},
	);
}

pub fn run(duration: Duration) {
	let started = Instant::now();
	let mut state = State::default();
	let empty_bytes = state.resident_history_bytes();
	ready(&mut state);
	let mut passes = 0_u64;
	let mut visits = 0_u64;
	let mut previous_requests = [None; 32];
	let mut ranges = [(usize::MAX, 0); 2];
	let mut row_evictions = 0;
	let mut logouts = 0;
	let mut byte_evictions = 0;
	// Fixed counters/ranges only: elapsed time never grows a samples or fixture archive.
	while started.elapsed() < duration || passes < 3 {
		for index in 0..32 {
			let channel = Id(20 + index);
			let large = index >= 16;
			let base = 1_000_000 + visits * 1000;
			let current = request(state.select(channel).expect("Synthetic DM is accessible"));
			assert_eq!(
				state.timeline.row_count(),
				0,
				"Oldest conversation must be evicted"
			);
			assert_eq!(state.freshness, Freshness::Loading);
			// Returning to the same channel next pass must not admit an older request.
			if let Some(old_request) = previous_requests[index as usize] {
				page(&mut state, channel, old_request, u64::MAX - 100, false);
				assert_eq!(state.freshness, Freshness::Loading);
				assert!(state.timeline.get(Id(u64::MAX - 99)).is_none());
			}
			apply(
				&mut state,
				Event::Delete {
					channel,
					id: Id(base + 1),
				},
			);
			apply(
				&mut state,
				Event::Patch(MessagePatch {
					sticker_items: Patch::Absent,
					id: Id(base + 2),
					channel,
					content: Patch::Value("Edited during load".into()),
					components: model::Patch::Absent,
					flags: model::Patch::Absent,
					application_id: model::Patch::Absent,
					extra_content: Default::default(),
					reactions: Patch::Absent,
					mentions: Patch::Absent,
					edited: Patch::Absent,
					embeds: Patch::Absent,
					embeds_suppressed: Patch::Absent,
					attachments: Patch::Absent,
				}),
			);
			page(&mut state, channel, current, base, large);
			assert_eq!(state.freshness, Freshness::Fresh);
			assert!(state.timeline.get(Id(base + 1)).is_none());
			assert_eq!(
				state.timeline.get(Id(base + 2)).unwrap().content,
				"Edited during load"
			);
			// More than one full timeline; large messages must hit bytes before the row cap.
			for id in 51..=650 {
				apply(
					&mut state,
					Event::Message(message(base + id, channel, large)),
				);
			}
			assert!(state.timeline.get(Id(base + 51)).is_none());
			if large {
				assert!(state.timeline.row_count() < 500);
				byte_evictions += 1;
			} else {
				assert_eq!(state.timeline.row_count(), 500);
				row_evictions += 1;
			}
			assert!(state.timeline.iter().all(|m| m.channel == channel));
			assert!(
				state
					.timeline
					.row_ids()
					.zip(state.timeline.row_ids().skip(1))
					.all(|(a, b)| a < b)
			);
			if passes >= 2 {
				let range = &mut ranges[usize::from(large)];
				let bytes = state.resident_history_bytes();
				range.0 = range.0.min(bytes);
				range.1 = range.1.max(bytes);
			}
			previous_requests[index as usize] = Some(current);
			visits += 1;
		}
		if (passes + 1).is_multiple_of(16) {
			apply(&mut state, Event::Disconnected);
			apply(&mut state, Event::Resumed);
			assert_eq!(state.freshness, Freshness::Stale);
			let current = request(state.history(None));
			page(
				&mut state,
				Id(51),
				current,
				1_000_000 + visits * 1000,
				false,
			);
			assert_eq!(state.freshness, Freshness::Fresh);
			apply(&mut state, Event::Resync);
			assert_eq!(state.freshness, Freshness::Stale);
			assert_eq!(state.resident_history_rows(), 0);
			ready(&mut state);
		}
		passes += 1;
		if passes.is_multiple_of(256) || started.elapsed() >= duration {
			ready(&mut state);
			let current = request(state.history(None));
			page(
				&mut state,
				Id(51),
				current,
				1_000_000 + visits * 1000,
				false,
			);
			state.drafts.insert(Id(51), "Synthetic unsent draft".into());
			assert!(matches!(state.prepare_send(), Some(Command::Send { .. })));
			state
				.drafts
				.insert(Id(20), "Another synthetic draft".into());
			let generation = state.generation;
			state.logout();
			state.apply(Envelope {
				generation,
				event: Event::Message(message(1, Id(51), false)),
			});
			assert!(state.user.is_none() && state.channels.is_empty() && !state.has_unsent());
			assert_eq!(state.draft_bytes(), 0);
			assert_eq!(state.resident_history_rows(), 0);
			assert_eq!(state.resident_history_bytes(), empty_bytes);
			previous_requests.fill(None);
			logouts += 1;
			ready(&mut state);
		}
	}
	state.logout();
	assert_eq!(state.resident_history_bytes(), empty_bytes);
	println!(
		"Synthetic lifecycle soak: {} passes, {} channel visits, {} live inserts in {:?}; row-pressure visits {}, byte-pressure visits {}, logout cycles {}. Steady retained history estimates: small {:?}, large {:?} bytes. No network, storage, renderer or audio devices; not process RSS, native scrolling or live compatibility.",
		passes,
		visits,
		visits * 600,
		started.elapsed(),
		row_evictions,
		byte_evictions,
		logouts,
		ranges[0],
		ranges[1]
	);
}
