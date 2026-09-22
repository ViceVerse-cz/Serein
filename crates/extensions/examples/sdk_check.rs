//! Offline ABI compatibility and timing check after building the standalone SDK examples.
//! `cargo run --locked --release -p extensions --example sdk_check -- <wasm-directory>`
use extensions::{
	Element, Invocation, MAX_MODULE_BYTES, MessageEvent, MessageEventKind, Output, Package, invoke,
	parse_package,
};
use std::{
	io::Read,
	path::{Path, PathBuf},
	time::Instant,
};

fn check(name: &str, bytes: &[u8], expected: &Output) {
	let package = parse_package(bytes).expect("package is valid");
	let input = Invocation {
		action: "activate".into(),
		..Default::default()
	};
	let output = invoke(&package, &input).expect("warmup invocation succeeds");
	assert_eq!(
		serde_json::to_value(output).unwrap(),
		serde_json::to_value(expected).unwrap(),
		"{name}: activation output changed"
	);
	let mut samples = [0_u128; 5];
	for sample in &mut samples {
		let start = Instant::now();
		for _ in 0..20 {
			std::hint::black_box(invoke(&package, &input).expect("invocation succeeds"));
		}
		*sample = start.elapsed().as_nanos() / 20;
	}
	samples.sort_unstable();
	println!(
		"{name}: package_bytes={}, wasm_bytes={}, invoke_median_us={:.3} (5 samples x 20 calls; one warmup; new runtime per call; excludes package construction/parsing and process startup)",
		bytes.len(),
		package.wasm.len(),
		samples[2] as f64 / 1000.0
	);
}

fn rebuilt(manifest: &str, path: &Path) -> Result<Package, Box<dyn std::error::Error>> {
	let mut wasm = Vec::new();
	std::fs::File::open(path)?
		.take(MAX_MODULE_BYTES as u64 + 1)
		.read_to_end(&mut wasm)?;
	assert!(wasm.len() <= MAX_MODULE_BYTES, "Wasm is too large");
	Ok(Package {
		manifest: serde_json::from_str(manifest)?,
		theme: None,
		background_image: Vec::new(),
		cover_image: Vec::new(),
		wasm,
	})
}

fn check_message_counter(name: &str, package: &Package) {
	package.validate().expect("counter package is valid");
	let mut input = Invocation {
		action: "message-event".into(),
		..Default::default()
	};
	for (kind, counts) in [
		(MessageEventKind::Create, [1, 0, 0]),
		(MessageEventKind::Update, [1, 1, 0]),
		(MessageEventKind::Delete, [1, 1, 1]),
	] {
		input.message_event = Some(Box::new(MessageEvent {
			kind,
			channel_id: "100".into(),
			message_id: "200".into(),
			author_id: (kind == MessageEventKind::Create).then(|| "300".into()),
			content: (kind != MessageEventKind::Delete).then(|| "Synthetic message".into()),
		}));
		let output = invoke(package, &input).expect("counter event succeeds");
		// The only effect is numeric storage; event IDs/content are never retained or displayed.
		assert_eq!(
			serde_json::to_value(&output).unwrap(),
			serde_json::to_value(Output {
				storage: Some(format!(
					r#"{{"create":{},"update":{},"delete":{}}}"#,
					counts[0], counts[1], counts[2]
				)),
				..Default::default()
			})
			.unwrap()
		);
		input.storage = output.storage;
	}
	input.message_event = None;
	input.action = "show".into();
	let shown = invoke(package, &input).expect("counter panel succeeds");
	assert!(shown.storage.is_none());
	assert!(
		matches!(&shown.panel[1], Element::Text { text } if text == "Created: 1\nUpdated: 1\nDeleted: 1")
	);

	input.storage = Some("invalid JSON".into());
	let shown = invoke(package, &input).expect("invalid storage is recoverable");
	assert!(shown.storage.is_none());
	assert!(
		matches!(&shown.panel[1], Element::Text { text } if text == "Saved counts are invalid. Reset counts to start again.")
	);
	input.action = "message-event".into();
	input.message_event = Some(Box::new(MessageEvent {
		kind: MessageEventKind::Delete,
		channel_id: "100".into(),
		message_id: "200".into(),
		author_id: None,
		content: None,
	}));
	let output = invoke(package, &input).expect("event preserves invalid storage");
	assert_eq!(
		serde_json::to_value(output).unwrap(),
		serde_json::to_value(Output::default()).unwrap()
	);
	input.message_event = None;
	input.action = "reset".into();
	let reset = invoke(package, &input).expect("explicit reset recovers storage");
	assert_eq!(
		reset.storage.as_deref(),
		Some(r#"{"create":0,"update":0,"delete":0}"#)
	);
	assert!(
		matches!(&reset.panel[1], Element::Text { text } if text == "Created: 0\nUpdated: 0\nDeleted: 0")
	);
	println!(
		"{name}: wasm_bytes={}, create/update/delete, panel, invalid storage and reset passed",
		package.wasm.len()
	);
	input.action = "message-event".into();
	input.storage = reset.storage;
	input.message_event = Some(Box::new(MessageEvent {
		kind: MessageEventKind::Create,
		channel_id: "100".into(),
		message_id: "200".into(),
		author_id: Some("300".into()),
		content: Some("Synthetic message".into()),
	}));
	input.message_event.as_mut().unwrap().content =
		Some("\u{1F980}".repeat(extensions::MAX_EVENT_CONTENT_BYTES / 4));
	invoke(package, &input).expect("maximum UTF-8 event content fits the sandbox");
	input.message_event.as_mut().unwrap().content = Some("Synthetic message".into());
	let _ = invoke(package, &input).unwrap();
	let mut samples = [0_u128; 5];
	for sample in &mut samples {
		let start = Instant::now();
		for _ in 0..20 {
			std::hint::black_box(invoke(package, &input).unwrap());
		}
		*sample = start.elapsed().as_nanos() / 20;
	}
	samples.sort_unstable();
	println!(
		"{name}/create: invoke_median_us={:.3} (5 samples x 20 calls; one warmup; new runtime per call; excludes worker IO)",
		samples[2] as f64 / 1000.0
	);
}

fn check_app_toolbox(name: &str, package: &Package) {
	use serde_json::json;
	package.validate().expect("toolbox package is valid");
	let mut input: Invocation = serde_json::from_value(json!({
		"action": "show",
		"app": {
			"context": {"connected": true, "user": {"id":"300", "name":"Synthetic user"},
				"channel": {"id":"100", "name":"demo", "kind":0}},
			"channels": {"items":[{"id":"100", "name":"demo", "kind":0}], "truncated":true},
			"timeline": {"channel_id":"100", "messages":[{"id":"200", "author":{"id":"300", "name":"Synthetic user"},
				"content":"Offline toolbox fixture", "attachment_count":0, "edited":false}], "truncated":false},
			"members": {"channel_id":"100", "items":[{"id":"300", "name":"Synthetic user"}], "truncated":false},
			"presence": {"items":[{"user_id":"300", "status":"online"}], "truncated":false},
			"voice": {"channel_id":"100", "phase":"connected", "muted":false, "deafened":false,
				"camera":false, "streaming":false, "participants":["300"]},
			"read_state": {"channel_id":"100", "unread":true, "mentions":2},
			"settings": {"zoom_percent":100, "sidebar_width":240, "show_members":true,
				"animate_gifs":true, "hide_media_links":false}
		}
	})).unwrap();
	let output = invoke(package, &input).expect("toolbox dashboard validates");
	assert!(output.effects.is_empty());
	assert!(
		output
			.panel
			.iter()
			.any(|item| matches!(item, Element::Text { text } if text == "Channels: 1 (partial)"))
	);
	assert!(output.panel.iter().any(
		|item| matches!(item, Element::Text { text } if text.contains("Offline toolbox fixture"))
	));
	let mut extended = serde_json::to_value(&input).unwrap();
	extended["app"]["account_profile"] = json!({"user":{"id":"300","name":"Synthetic user"},"profile":{"display_name":"Synthetic display","bio":"Offline profile fixture","pronouns":"they/them"}});
	extended["app"]["guilds"] =
		json!({"items":[{"id":"400","name":"Synthetic server"}],"truncated":false});
	extended["app"]["channel_details"] = json!({"channel":{"id":"100","name":"demo","kind":0},"position":0,"recipients":[],"recipients_truncated":false,"can_send":true,"can_read_history":true});
	extended["app"]["message_details"] = json!({"channel_id":"100","items":[{"id":"200","kind":0,"reply_to":"199","mention_ids":["300"],"mentions_truncated":false,"mention_everyone":false,"attachments":[{"id":"500","filename":"report.txt","size":123,"content_type":"text/plain","spoiler":false}],"attachments_truncated":false,"reactions":[{"emoji_name":"ok","count":2,"me":true,"me_burst":false}],"reactions_truncated":false}],"truncated":false});
	extended["app"]["relationships"] = json!({"items":[{"user":{"id":"300","name":"Synthetic user"},"kind":"friend"}],"truncated":false,"friends_known":true,"requests_known":false,"restricted_known":false});
	extended["app"]["settings"]["smooth_scrolling"] = json!(true);
	extended["app"]["settings"]["scroll_speed_percent"] = json!(100);
	extended["app"]["notification_settings"] = json!({
		"new_message":true,"current_channel":false,"incoming_ring":true,"outgoing_ring":true,
		"disable_sounds":false,"unread_badge":true,"mute":true,"unmute":true,"deafen":true,
		"undeafen":true,"camera_on":true,"screen_share_on":true,"user_join":true,"user_leave":true,"volume":75
	});
	let extended = serde_json::from_value(extended).unwrap();
	let output = invoke(package, &extended).expect("new snapshot groups fit the sandbox");
	assert!(output.panel.iter().any(
		|item| matches!(item, Element::Text { text } if text.contains("Offline profile fixture"))
	));

	assert!(output.panel.iter().any(
		|item| matches!(item, Element::Text { text } if text.starts_with("Message details: 1"))
	));
	assert!(output.panel.iter().any(
		|item| matches!(item, Element::Text { text } if text.starts_with("Relationships: 1"))
	));
	let mut larger = input.clone();
	let app = larger.app.as_mut().unwrap();
	app.timeline.as_mut().unwrap().messages = (0..50)
		.map(|index| extensions::MessageSnapshot {
			id: (1000 + index).to_string(),
			author: extensions::UserSnapshot {
				id: "300".into(),
				name: "Synthetic user".into(),
			},
			content: "Synthetic café text. ".repeat(11),
			attachment_count: 0,
			edited: false,
		})
		.collect();
	let snapshot_bytes = app.bytes().unwrap();
	assert!((16 * 1024..24 * 1024).contains(&snapshot_bytes));
	invoke(package, &larger).expect("realistic 50-message UTF-8 snapshot fits the sandbox");
	let mut samples = [0_u128; 5];
	for sample in &mut samples {
		let start = Instant::now();
		for _ in 0..20 {
			std::hint::black_box(invoke(package, &larger).unwrap());
		}
		*sample = start.elapsed().as_nanos() / 20;
	}
	samples.sort_unstable();
	println!(
		"{name}/dashboard: snapshot_bytes={}, invoke_median_us={:.3} (5 samples x 20 calls; one warmup; new runtime per call; excludes snapshot construction and worker IO)",
		snapshot_bytes,
		samples[2] as f64 / 1000.0
	);
	for (action, values, effect) in [
		(
			"open-channel",
			json!({"channel":"demo (100)"}),
			json!({"type":"navigate", "channel_id":"100"}),
		),
		("open-view", json!({"view":"home"}), json!({"type":"home"})),
		(
			"open-view",
			json!({"view":"themes"}),
			json!({"type":"open_view", "view":"themes"}),
		),
		(
			"search",
			json!({"query":"synthetic"}),
			json!({"type":"search", "query":"synthetic"}),
		),
		(
			"jump",
			json!({"message-id":"200"}),
			json!({"type":"jump_to_message", "channel_id":"100", "message_id":"200"}),
		),
		(
			"profile",
			json!({"user-id":"300"}),
			json!({"type":"open_profile", "user_id":"300"}),
		),
		(
			"copy",
			json!({}),
			json!({"type":"copy_text", "text":"Connected: true\nAccount: Synthetic user\nConversation: demo"}),
		),
		(
			"notice",
			json!({"notice-text":"Local only"}),
			json!({"type":"notice", "text":"Local only"}),
		),
		(
			"voice",
			json!({"muted":"true", "deafened":"false"}),
			json!({"type":"set_voice", "muted":true, "deafened":false}),
		),
		("leave", json!({}), json!({"type":"leave_voice"})),
		(
			"settings",
			json!({"zoom":"120", "sidebar":"250", "show-members":"false", "animate-gifs":"false", "hide-media-links":"true"}),
			json!({"type":"set_local_settings", "settings":{"zoom_percent":120,"sidebar_width":250,"show_members":false,"animate_gifs":false,"hide_media_links":true}}),
		),
	] {
		input.action = action.into();
		input.values = serde_json::from_value(values).unwrap();
		let output = invoke(package, &input).expect("toolbox proposal validates");
		assert_eq!(
			serde_json::to_value(&output.effects).unwrap(),
			json!([effect]),
			"{action}"
		);
		assert!(output.panel.is_empty());
	}
	input.action = "notifications".into();
	input.values = serde_json::from_value(json!({"sound-volume":"35","disable-sounds":"false","unread-badge":"true","current-channel":"true"})).unwrap();
	let output = invoke(package, &input).expect("notification proposal validates in Wasm");
	assert_eq!(
		serde_json::to_value(output.effects).unwrap(),
		json!([{
			"type":"set_notification_settings","settings":{"volume":35,"disable_sounds":false,"unread_badge":true,"current_channel":true}
		}])
	);
	input.values.insert("sound-volume".into(), "101".into());
	assert!(invoke(package, &input).unwrap().effects.is_empty());
	input.action = "settings".into();
	input.values = serde_json::from_value(json!({"zoom":"100","sidebar":"240","show-members":"true","animate-gifs":"true","hide-media-links":"true","smooth-scrolling":"false","scroll-speed":"200"})).unwrap();
	let output = invoke(package, &input).expect("scrolling proposal validates in Wasm");
	assert!(
		matches!(&output.effects[..], [extensions::HostEffect::SetLocalSettings { settings }] if settings.smooth_scrolling == Some(false) && settings.scroll_speed_percent == Some(200))
	);
	input.action = "on-app".into();
	input.values.clear();
	for kind in [
		extensions::AppEventKind::MessageDetails,
		extensions::AppEventKind::Relationships,
		extensions::AppEventKind::Account,
		extensions::AppEventKind::Channels,
		extensions::AppEventKind::Members,
		extensions::AppEventKind::Presence,
		extensions::AppEventKind::ReadState,
		extensions::AppEventKind::Ready,
		extensions::AppEventKind::Navigation,
		extensions::AppEventKind::Context,
		extensions::AppEventKind::Connection,
		extensions::AppEventKind::Voice,
		extensions::AppEventKind::Settings,
	] {
		input.app_event = Some(kind);
		let output = invoke(package, &input).expect("toolbox event observer validates");
		assert_eq!(
			serde_json::to_value(output).unwrap(),
			serde_json::to_value(Output::default()).unwrap()
		);
	}
	println!(
		"{name}: wasm_bytes={}, snapshot_bytes={}, dashboard, all 12 host effects and passive app events passed",
		package.wasm.len(),
		snapshot_bytes
	);
}

fn check_guild_inspector(name: &str, package: &Package) {
	use serde_json::json;
	let members: Vec<_> = (1..=20)
		.map(|id| {
			json!({
			 "user":{"id":id.to_string(),"name":"Loaded user"},"nick":"Nickname",
			 "display_name":"Nickname","role_ids":["400"],"roles_truncated":false
			})
		})
		.collect();
	let mut input: Invocation = serde_json::from_value(json!({"action":"show","app":{
	 "channel_metadata":{"channel_id":"100","guild_id":"200",
	  "category":{"id":"300","guild_id":"200","name":"Category","kind":4},
	  "topic":"Loaded topic", "topic_truncated":false, "slowmode_seconds":5,"nsfw":false,
	  "permissions":{"view_channel":true,"send_messages":false,"manage_roles":null},
	  "thread":{"message_count":7,"archived":null,"locked":null,"pinned":null}},
	 "member_details":{"channel_id":"100","guild_id":"200","items":members,"truncated":true,
	  "roles":[{"id":"400","name":"Role","color":123,"position":1}],"roles_truncated":false}
	}}))
	.unwrap();
	let output = invoke(package, &input).expect("new guild snapshots fit the real sandbox");
	for expected in [
		"Loaded topic",
		"Nickname",
		"ManageRoles: unknown",
		"Loaded members: 20",
	] {
		assert!(
			output
				.panel
				.iter()
				.any(|e| matches!(e, Element::Text {text} if text.contains(expected))),
			"{name}: {expected}"
		);
	}
	assert!(output.effects.is_empty() && output.storage.is_none());
	input.action = "on-app".into();
	for kind in [
		extensions::AppEventKind::Threads,
		extensions::AppEventKind::Roles,
		extensions::AppEventKind::Permissions,
		extensions::AppEventKind::Recovered,
	] {
		input.app_event = Some(kind);
		let output = invoke(package, &input).expect("new event vocabulary fits the real sandbox");
		assert_eq!(
			serde_json::to_value(output).unwrap(),
			serde_json::to_value(Output::default()).unwrap()
		);
	}
	println!("{name}: bounded guild data and four new event kinds passed");
}

fn check_conversation_inspector(name: &str, package: &Package) {
	let mut fixture: serde_json::Value = serde_json::from_str(include_str!(
		"../../../examples/extensions/conversation-inspector/fixtures/loaded.json"
	))
	.unwrap();
	fixture.as_object_mut().unwrap().remove("host");
	let mut input: Invocation = serde_json::from_value(fixture).unwrap();
	let output = invoke(package, &input)
		.expect("conversation fixture and host discovery fit the real sandbox");
	for expected in [
		"Host revision 1: message_content support true",
		"Synthetic release note",
		"questions, options and results unavailable",
	] {
		assert!(
			output
				.panel
				.iter()
				.any(|element| matches!(element,Element::Text{text} if text.contains(expected))),
			"{name}: {expected}"
		);
	}
	assert!(output.effects.is_empty() && output.storage.is_none());
	input.action = "on-app".into();
	for kind in [
		extensions::AppEventKind::Reactions,
		extensions::AppEventKind::Pins,
		extensions::AppEventKind::Typing,
		extensions::AppEventKind::Polls,
	] {
		input.app_event = Some(kind);
		let output = invoke(package, &input).expect("new activity reasons validate in Wasm");
		assert!(output.panel.is_empty() && output.effects.is_empty() && output.storage.is_none());
	}
	println!("{name}: rich data, host discovery and four focused events passed");
}

fn check_app_actions(package: &Package) {
	package.validate().expect("action example package is valid");
	let mut input: Invocation = serde_json::from_str(r#"{"action":"show","app":{"context":{"connected":true,"channel":{"id":"20","name":"general","kind":0}}}}"#).unwrap();
	let panel = invoke(package, &input).expect("action form runs within sandbox bounds");
	assert!(panel.effects.is_empty());
	assert!(
		panel
			.panel
			.iter()
			.any(|element| matches!(element, Element::Button { id, .. } if id == "propose"))
	);
	input.action = "propose".into();
	input.values.extend([
		("channel".into(), "20".into()),
		("message".into(), "200".into()),
		("text".into(), "Synthetic proposal".into()),
		("emoji".into(), "👍".into()),
	]);
	for (operation, kind) in [
		("Send message", "send_message"),
		("Edit message", "edit_message"),
		("Delete message", "delete_message"),
		("Add reaction", "set_reaction"),
		("Remove reaction", "set_reaction"),
		("Pin message", "set_message_pinned"),
		("Unpin message", "set_message_pinned"),
		("Mark channel read", "mark_channel_read"),
		("Mark message unread", "mark_unread"),
		("Create thread", "create_thread"),
	] {
		input.values.insert("operation".into(), operation.into());
		let output = invoke(package, &input).expect("foreground proposal validates in real Wasm");
		assert_eq!(output.effects.len(), 1);
		let wire = serde_json::to_value(&output.effects[0]).unwrap();
		assert_eq!(wire["type"], "app_action");
		assert_eq!(wire["action"]["type"], kind);
	}
	input.values.insert("channel".into(), "21".into());
	assert!(invoke(package, &input).unwrap().effects.is_empty());
	println!(
		"app-actions: wasm_bytes={}, form, ten foreground proposals and changed-channel rejection passed",
		package.wasm.len()
	);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let mut args = std::env::args_os().skip(1);
	let wasm_dir = PathBuf::from(args.next().expect("usage: sdk_check <wasm-directory>"));
	assert!(args.next().is_none(), "usage: sdk_check <wasm-directory>");
	check_app_actions(&rebuilt(
		include_str!("../../../examples/extensions/app-actions/manifest.json"),
		&wasm_dir.join("app_actions.wasm"),
	)?);
	for (name, committed, manifest, wasm_file, expected) in [
		(
			"message-delete-protector",
			include_bytes!(
				"../../../examples/extensions/packages/message-delete-protector.serein-extension"
			)
			.as_slice(),
			include_str!("../../../examples/extensions/message-delete-protector/manifest.json"),
			"message_delete_protector.wasm",
			Output {
				preserve_deleted_messages: true,
				..Default::default()
			},
		),
		(
			"emoji-sticker-images",
			include_bytes!(
				"../../../examples/extensions/packages/emoji-sticker-images.serein-extension"
			)
			.as_slice(),
			include_str!("../../../examples/extensions/emoji-sticker-images/manifest.json"),
			"emoji_sticker_images.wasm",
			Output {
				image_sharing: true,
				..Default::default()
			},
		),
	] {
		check(&format!("{name}/committed"), committed, &expected);
		let rebuilt = rebuilt(manifest, &wasm_dir.join(wasm_file))?;
		// The shipped legacy protector returns true; its current source uses no-op activation.
		let expected = if name == "message-delete-protector" {
			Output::default()
		} else {
			expected
		};
		check(
			&format!("{name}/rebuilt"),
			&serde_json::to_vec(&rebuilt)?,
			&expected,
		);
	}
	check_message_counter(
		"message-counter/committed",
		&parse_package(include_bytes!(
			"../../../examples/extensions/packages/message-counter.serein-extension"
		))?,
	);
	check_message_counter(
		"message-counter/rebuilt",
		&rebuilt(
			include_str!("../../../examples/extensions/message-counter/manifest.json"),
			&wasm_dir.join("message_counter.wasm"),
		)?,
	);
	check_app_toolbox(
		"app-toolbox/committed",
		&parse_package(include_bytes!(
			"../../../examples/extensions/packages/app-toolbox.serein-extension"
		))?,
	);
	check_app_toolbox(
		"app-toolbox/rebuilt",
		&rebuilt(
			include_str!("../../../examples/extensions/app-toolbox/manifest.json"),
			&wasm_dir.join("app_toolbox.wasm"),
		)?,
	);
	check_guild_inspector(
		"guild-inspector/committed",
		&parse_package(include_bytes!(
			"../../../examples/extensions/packages/guild-inspector.serein-extension"
		))?,
	);
	check_guild_inspector(
		"guild-inspector/rebuilt",
		&rebuilt(
			include_str!("../../../examples/extensions/guild-inspector/manifest.json"),
			&wasm_dir.join("guild_inspector.wasm"),
		)?,
	);
	check_conversation_inspector(
		"conversation-inspector/committed",
		&parse_package(include_bytes!(
			"../../../examples/extensions/packages/conversation-inspector.serein-extension"
		))?,
	);
	check_conversation_inspector(
		"conversation-inspector/rebuilt",
		&rebuilt(
			include_str!("../../../examples/extensions/conversation-inspector/manifest.json"),
			&wasm_dir.join("conversation_inspector.wasm"),
		)?,
	);
	Ok(())
}
