use client_core::{Envelope, Event};
use model::Id;
use std::time::Instant;
mod soak;
fn main() {
	if std::env::args().nth(1).as_deref() == Some("--soak") {
		let seconds = std::env::args()
			.nth(2)
			.map_or(Ok(60), |value| value.parse::<u64>())
			.expect("Soak seconds must be an integer from 1 to 3600");
		assert!(
			(1..=3600).contains(&seconds),
			"Soak seconds must be from 1 to 3600"
		);
		soak::run(std::time::Duration::from_secs(seconds));
		return;
	}
	if std::env::args().nth(1).as_deref() == Some("--navigation") {
		navigation();
		return;
	}
	if std::env::args().nth(1).as_deref() == Some("--wide-channels") {
		wide_channels();
		return;
	}
	let start = Instant::now();
	let mut state = test_support::demo_state();
	let mut samples = Vec::new();
	for cycle in 0..200_u64 {
		for id in 1..=500 {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Message(test_support::message(1000 + cycle * 500 + id, Id(20))),
			});
		}
		samples.push(state.timeline.bytes());
		assert!(state.timeline.len() <= 500);
		assert!(state.timeline.bytes() <= 4 * 1024 * 1024);
	}
	let tail = &samples[100..];
	println!(
		"Synthetic reducer replay: 100000 events in {:?}; retained timeline {}..{} estimated bytes, {} records. This is not process RSS, UI frame time, or live compatibility.",
		start.elapsed(),
		tail.iter().min().unwrap(),
		tail.iter().max().unwrap(),
		state.timeline.len()
	);
	state.logout();
	assert_eq!(state.timeline.bytes(), 0);
	assert!(!state.has_unsent());
}

fn wide_channels() {
	let mut state = test_support::demo_state();
	let target = state.channel(Id(20)).expect("demo conversation").clone();
	state.channels = (0..20_000_u64)
		.map(|index| {
			let mut channel = target.clone();
			channel.id = Id(100_000 + index);
			channel
		})
		.chain(std::iter::once(target.clone()))
		.collect();
	state.invalidate_navigation();
	let start = Instant::now();
	for id in 1..=10_000 {
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(test_support::message(1_000_000 + id, Id(20))),
		});
	}
	println!(
		"Synthetic wide-channel reducer: 10000 messages into the last of 20001 channels in {:?}; {} retained records / {} estimated bytes. Excludes startup, RSS, UI and live compatibility.",
		start.elapsed(),
		state.timeline.len(),
		state.timeline.bytes()
	);
	assert_eq!(state.timeline.len(), 500);
	state.logout();
	assert_eq!(state.timeline.bytes(), 0);
}

fn navigation() {
	let mut state = test_support::demo_state();
	let channels = [Id(20), Id(21), Id(22)];
	state.channels = channels
		.into_iter()
		.map(|id| model::Channel {
			id,
			guild: None,
			parent_id: None,
			kind: 1,
			position: 0,
			name: format!("Synthetic conversation {id}"),
			recipients: vec![],
			last_message: None,
			member_list_id: None,
			message_count: None,
			icon: None,
		})
		.collect();
	state.selected = None;
	state.timeline.clear();
	let mut samples = Vec::with_capacity(10_000);
	let mut hits = 0;
	let mut requests = 0;
	for step in 0..10_003 {
		let channel = channels[step % channels.len()];
		let started = Instant::now();
		let command = state.select(channel);
		let elapsed = started.elapsed();
		let Some(client_core::Command::History {
			channel: selected,
			before: None,
			after: None,
			request,
		}) = command
		else {
			panic!("Navigation must revalidate its selected conversation");
		};
		assert_eq!(selected, channel);
		assert_eq!(state.freshness, model::Freshness::Loading);
		assert!(
			state
				.timeline
				.iter()
				.all(|message| message.channel == channel)
		);
		if step >= 3 {
			samples.push(elapsed);
			hits += usize::from(state.timeline.row_count() != 0);
			requests += 1;
		}
		state.apply(Envelope {
			generation: state.generation,
			event: Event::History {
				channel,
				request,
				older: false,
				messages: (1..=50)
					.map(|id| test_support::message(channel.0 * 1000 + id, channel))
					.collect(),
			},
		});
		assert_eq!(state.timeline.len(), 50);
		assert_eq!(state.freshness, model::Freshness::Fresh);
	}
	samples.sort_unstable();
	println!(
		"Synthetic navigation: 10000 selections across three 50-message conversations; immediate previews {hits}/10000; revalidation requests {requests}; select median {:?}, p95 {:?}. Measures core selection only, not native display latency, RSS or Discord compatibility.",
		samples[5000], samples[9499],
	);
	println!(
		"Navigation retained: {} dormant windows; {} active plus dormant rows; {} estimated retained bytes (not RSS).",
		state.resident_window_count(),
		state.resident_history_rows(),
		state.resident_history_bytes()
	);
	state.logout();
	assert_eq!(state.timeline.row_count(), 0);
	assert!(!state.has_unsent());
}
