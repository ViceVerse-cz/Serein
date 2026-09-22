//! Run an immutable pre-metadata plugin in the current offline Wasm sandbox.
use extensions::{Element, Error, Invocation, Output, invoke, parse_package};
use serde_json::json;

fn main() {
	let bytes = include_bytes!("../tests/fixtures/sdk-legacy/app-toolbox.serein-extension");
	assert_eq!(bytes.len(), 574_992);
	let package = parse_package(bytes).expect("immutable legacy package validates");
	assert_eq!(package.wasm.len(), 198_370);
	let mut input: Invocation = serde_json::from_value(json!({
        "action": "show",
        "app": {
            "settings": {"zoom_percent":100,"sidebar_width":236,"show_members":true,
                "animate_gifs":true,"hide_media_links":true,"smooth_scrolling":true,"scroll_speed_percent":100},
            "context": {"connected": true, "user": {"id": "300", "name": "Legacy user"},
                "channel": {"id": "100", "name": "legacy", "kind": 0}},
            "account_profile": {"user": {"id": "300", "name": "Legacy user"},
                "profile": {"display_name": "Legacy display", "bio": "Immutable Wasm fixture", "pronouns": "they/them"}},
            "guilds": {"items": [{"id": "400", "name": "Legacy server"}], "truncated": false},
            "channel_details": {"channel": {"id": "100", "name": "legacy", "kind": 0},
                "position": 0, "recipients": [], "recipients_truncated": false,
                "can_send": true, "can_read_history": true},
            "timeline": {"channel_id": "100", "messages": [{"id": "200",
                "author": {"id": "300", "name": "Legacy user"}, "content": "Offline legacy message",
                "attachment_count": 0, "edited": false}], "truncated": false}
        }
    })).unwrap();
	let output =
		invoke(&package, &input).expect("unchanged compiled plugin displays old snapshots");
	assert!(output.effects.is_empty());
	assert!(output.storage.is_none());
	for expected in [
		"Immutable Wasm fixture",
		"Offline legacy message",
		"Joined servers: 1",
	] {
		assert!(
			output.panel.iter().any(|element| {
				matches!(element, Element::Text { text } if text.contains(expected))
			}),
			"missing legacy dashboard text: {expected}"
		);
	}

	input.action = "on-app".into();
	// Data invalidations only: lifecycle delivery is outside this check's scope.
	for kind in ["account", "channels", "members", "presence", "read_state"] {
		input.app_event = Some(serde_json::from_value(json!(kind)).unwrap());
		let output =
			invoke(&package, &input).expect("old event vocabulary still decodes in old Wasm");
		assert_eq!(
			serde_json::to_value(output).unwrap(),
			serde_json::to_value(Output::default()).unwrap()
		);
	}
	for kind in [
		"message_details",
		"relationships",
		"threads",
		"roles",
		"permissions",
		"recovered",
		"reactions",
		"pins",
		"typing",
		"polls",
	] {
		input.app_event =
			Some(serde_json::from_value(json!(kind)).expect("current host recognizes new event"));
		assert!(
			matches!(input.validate(&package.manifest), Err(Error::Capability)),
			"new event {kind} must not reach an old strict event enum"
		);
	}
	println!(
		"immutable legacy App Toolbox: foreground snapshot, 5 old data events and 10 new-event grant rejections passed"
	);
}
