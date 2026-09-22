#[allow(dead_code)]
#[path = "../src/extensions.rs"]
mod host;
use client_core::{Envelope, Event};
use eframe::egui;

fn job(host: &mut host::ExtensionHost, job: host::Job) -> host::Event {
	host.submit(job, &egui::Context::default()).unwrap();
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
	loop {
		if let Some((_, outcome)) = host.poll() {
			return outcome.unwrap();
		}
		assert!(std::time::Instant::now() < deadline, "worker timed out");
		std::thread::sleep(std::time::Duration::from_millis(10));
	}
}
fn main() {
	let mut state = test_support::demo_state();
	let channel = state.selected.unwrap();
	let ids: Vec<_> = state.timeline.iter().take(3).map(|m| m.id).collect();
	assert_eq!(ids.len(), 3);
	for event in [
		Event::Delete {
			channel,
			id: ids[0],
		},
		Event::DeleteBulk {
			channel,
			ids: ids[1..].to_vec(),
		},
	] {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
	for id in ids {
		assert!(state.timeline.is_deleted(id));
		assert!(state.timeline.get_display(id).is_none());
		state
			.timeline
			.insert(test_support::message(id.0, channel), false, false)
			.unwrap();
		assert!(
			state.timeline.get_display(id).is_none(),
			"late history cannot restore a deletion"
		);
	}
	let root = std::env::temp_dir().join(format!("serein-protector-check-{}", std::process::id()));
	assert!(!root.exists());
	let mut host = host::ExtensionHost::new(root.clone());
	let starter = host::starters().unwrap().remove(0);
	let account = Some("synthetic".to_owned());
	let host::Event::Enabled(installed) = job(
		&mut host,
		host::Job::Enable {
			source: Box::new(starter.source),
			grants: vec![extensions::Capability::DeletedMessages],
			account: account.clone(),
		},
	) else {
		panic!("enable failed")
	};
	assert!(installed.error.is_none());
	assert!(
		installed.preserve_deleted_messages,
		"enabled protector with explicit consent opts into retention"
	);
	let host::Event::Loaded { installed, .. } = job(
		&mut host,
		host::Job::Load {
			account: account.clone(),
		},
	) else {
		panic!("load failed")
	};
	assert_eq!(installed.len(), 1);
	assert!(installed[0].error.is_none());
	assert!(installed[0].preserve_deleted_messages);
	let mut state = test_support::demo_state();
	let channel = state.selected.unwrap();
	state.set_preserve_deleted_messages(installed[0].preserve_deleted_messages);
	let id = state.timeline.iter().next().unwrap().id;
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Delete { channel, id },
	});
	assert!(state.timeline.get(id).is_none());
	assert!(state.timeline.get_display(id).is_some());
	state.history(None);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::History {
			channel,
			request: state.request,
			older: false,
			messages: vec![],
		},
	});
	assert!(state.timeline.get_display(id).is_some());
	let other = state
		.channels
		.iter()
		.find(|entry| {
			entry.id != channel && entry.supports_text() && state.can_read_history(entry.id)
		})
		.unwrap()
		.id;
	state.select(other).unwrap();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Message(test_support::message(6001, channel)),
	});
	state.select(channel).unwrap();
	assert!(
		state.timeline.get_display(id).is_some(),
		"new activity in a dormant channel must not erase retained messages"
	);
	assert_eq!(
		state.timeline.row_count(),
		1,
		"stale live history is still invalidated"
	);
	assert!(matches!(
		job(
			&mut host,
			host::Job::Disable {
				id: "message-delete-protector".into(),
				kind: extensions::ExtensionKind::Plugin,
				account
			}
		),
		host::Event::Disabled(_)
	));
	state.apply(Envelope {
		generation: state.generation,
		event: Event::History {
			channel,
			request: state.request,
			older: false,
			messages: vec![],
		},
	});
	state.select(other).unwrap();
	state.set_preserve_deleted_messages(false);
	state.select(channel).unwrap();
	assert!(state.timeline.get_display(id).is_none());
	assert!(state.timeline.is_deleted(id));
	drop(host);
	std::fs::remove_dir_all(root).unwrap();
	println!(
		"Protector lifecycle passed: default single/bulk removal, stale-history rejection, bundled worker enable, reload, opt-in retention, history refresh, dormant activity, disable clears dormant content."
	);
}
