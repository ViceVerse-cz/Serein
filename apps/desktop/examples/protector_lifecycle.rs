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
		!installed.preserve_deleted_messages,
		"retention is built-in; activation output must not gate it"
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
	let mut state = test_support::demo_state();
	let channel = state.selected.unwrap();
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
	assert!(
		state.timeline.get_display(id).is_some(),
		"disabling the example does not drop retained rows"
	);
	state.discard_preserved_deleted(id);
	assert!(state.timeline.get_display(id).is_none());
	drop(host);
	std::fs::remove_dir_all(root).unwrap();
	println!(
		"Protector lifecycle passed: bundled worker enable, reload, built-in retention, history refresh, dormant activity, disable leaves rows, local remove."
	);
}
