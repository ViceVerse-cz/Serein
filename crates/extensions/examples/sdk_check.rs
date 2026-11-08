//! Offline ABI compatibility and timing check after building the standalone SDK examples.
//! `cargo run --locked --release -p extensions --example sdk_check -- <wasm-directory> <catalog-wasm-directory>`
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
	// Exercise the rebuilt and committed Wasm through host validation, including
	// the expanded lower zoom range and both rejected out-of-range values.
	for zoom in ["50", "79", "150"] {
		input.values.insert("zoom".into(), zoom.into());
		let output = invoke(package, &input).expect("small zoom proposal validates in Wasm");
		assert!(
			matches!(&output.effects[..], [extensions::HostEffect::SetLocalSettings { settings }] if settings.zoom_percent == zoom.parse::<u16>().ok())
		);
	}
	for zoom in ["49", "151"] {
		input.values.insert("zoom".into(), zoom.into());
		assert!(
			invoke(package, &input)
				.expect("invalid zoom is reported by the example")
				.effects
				.is_empty()
		);
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

fn check_safe_ports(catalog_wasm_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
	let offline = rebuilt(
		include_str!("../../../extensions/plugins/offline-composer-tools/manifest.json"),
		&catalog_wasm_dir.join("offline_composer_tools.wasm"),
	)?;
	let text_commands = rebuilt(
		include_str!("../../../extensions/plugins/text-commands/manifest.json"),
		&catalog_wasm_dir.join("text_commands.wasm"),
	)?;
	let view_raw = rebuilt(
		include_str!("../../../extensions/plugins/view-raw-message/manifest.json"),
		&catalog_wasm_dir.join("view_raw_message.wasm"),
	)?;
	let external = rebuilt(
		include_str!("../../../extensions/plugins/external-search-tools/manifest.json"),
		&catalog_wasm_dir.join("external_search_tools.wasm"),
	)?;
	let codecs = rebuilt(
		include_str!("../../../extensions/plugins/text-codecs/manifest.json"),
		&catalog_wasm_dir.join("text_codecs.wasm"),
	)?;
	let composition = rebuilt(
		include_str!("../../../extensions/plugins/message-composition-tools/manifest.json"),
		&catalog_wasm_dir.join("message_composition_tools.wasm"),
	)?;
	let identity = rebuilt(
		include_str!("../../../extensions/plugins/identity-tools/manifest.json"),
		&catalog_wasm_dir.join("identity_tools.wasm"),
	)?;
	let role_members = rebuilt(
		include_str!("../../../extensions/plugins/loaded-role-members/manifest.json"),
		&catalog_wasm_dir.join("loaded_role_members.wasm"),
	)?;
	let status_presets = rebuilt(
		include_str!("../../../extensions/plugins/status-presets/manifest.json"),
		&catalog_wasm_dir.join("status_presets.wasm"),
	)?;
	let reference_tools = rebuilt(
		include_str!("../../../extensions/plugins/message-reference-tools/manifest.json"),
		&catalog_wasm_dir.join("message_reference_tools.wasm"),
	)?;
	let draft_style = rebuilt(
		include_str!("../../../extensions/plugins/draft-style-tools/manifest.json"),
		&catalog_wasm_dir.join("draft_style_tools.wasm"),
	)?;
	let command_palette = rebuilt(
		include_str!("../../../extensions/plugins/command-palette/manifest.json"),
		&catalog_wasm_dir.join("command_palette.wasm"),
	)?;
	let message_history = rebuilt(
		include_str!("../../../extensions/plugins/message-history/manifest.json"),
		&catalog_wasm_dir.join("message_history.wasm"),
	)?;
	let ingtoninator = rebuilt(
		include_str!("../../../extensions/plugins/ingtoninator/manifest.json"),
		&catalog_wasm_dir.join("ingtoninator.wasm"),
	)?;
	let own_sed = rebuilt(
		include_str!("../../../extensions/plugins/own-message-sed/manifest.json"),
		&catalog_wasm_dir.join("own_message_sed.wasm"),
	)?;
	let social = rebuilt(
		include_str!("../../../extensions/plugins/social-link-tools/manifest.json"),
		&catalog_wasm_dir.join("social_link_tools.wasm"),
	)?;
	let offline_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/offline-composer-tools.serein-extension"
	))?;
	let text_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/text-commands.serein-extension"
	))?;
	let raw_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/view-raw-message.serein-extension"
	))?;
	let external_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/external-search-tools.serein-extension"
	))?;
	let codecs_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/text-codecs.serein-extension"
	))?;
	let composition_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/message-composition-tools.serein-extension"
	))?;
	let identity_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/identity-tools.serein-extension"
	))?;
	let role_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/loaded-role-members.serein-extension"
	))?;
	let status_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/status-presets.serein-extension"
	))?;
	let reference_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/message-reference-tools.serein-extension"
	))?;
	let draft_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/draft-style-tools.serein-extension"
	))?;
	let command_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/command-palette.serein-extension"
	))?;
	let history_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/message-history.serein-extension"
	))?;
	let ington_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/ingtoninator.serein-extension"
	))?;
	let sed_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/own-message-sed.serein-extension"
	))?;
	let social_committed = parse_package(include_bytes!(
		"../../../extensions/plugins/packages/social-link-tools.serein-extension"
	))?;
	let role_members_input: Invocation = serde_json::from_value(serde_json::json!({
		"action": "open-profile",
		"storage": "{\"role_id\":\"400\"}",
		"values": {"member": "Synthetic user [300]"},
		"app": {"member_details": {
			"channel_id": "100", "guild_id": "400", "truncated": false,
			"roles": [{"id":"400","name":"Role","color":1223476,"position":1}], "roles_truncated": false,
			"items": [{"user":{"id":"300","name":"Synthetic user"},"display_name":"Synthetic user","role_ids":["400"],"roles_truncated":false}]
		}}
	}))?;
	let mut role_color_input = role_members_input.clone();
	role_color_input.action = "copy-role-color".into();
	role_color_input.storage = None;
	role_color_input.values.clear();
	role_color_input
		.values
		.insert("role".into(), "Role [400]".into());
	let composition_input: Invocation = serde_json::from_value(serde_json::json!({
		"action": "filter-keywords", "composer": "keep\ndrop private line\n",
		"storage": "{\"split_mode\":\"newlines\",\"split_limit\":2000,\"keywords\":[\"drop\"],\"keyword_case_sensitive\":false,\"burst_style\":\"lines\",\"burst_max_segments\":8}"
	}))?;
	let status_presets_input: Invocation = serde_json::from_value(serde_json::json!({
		"action": "apply", "values": {"preset":"Away"},
		"storage": "{\"version\":1,\"presets\":[{\"name\":\"Away\",\"status\":\"idle\"}]}"
	}))?;
	let social_selection = |action: &str, selected: &str| Invocation {
		action: action.into(),
		selected_message: Some(selected.into()),
		selected_message_ref: Some(Box::new(extensions::SelectedMessageRef {
			channel_id: "100".into(),
			message_id: "200".into(),
			author_id: "300".into(),
			guild_id: None,
		})),
		..Default::default()
	};
	let social_cancel_input: Invocation = serde_json::from_value(serde_json::json!({
		"action": "cancel", "values": {"user-id":"10"},
		"app": {"relationships": {
			"items": [{"user":{"id":"10","name":"Outgoing"},"kind":"outgoing_request"}],
			"truncated":false,"friends_known":true,"requests_known":true,"restricted_known":true
		}}
	}))?;
	let social_repeat_input = social_selection("repeat", "hi <@123> @everyone");
	let social_normalize_input = social_selection(
		"normalize-links",
		"see https://canary.discord.com/channels/1/2/3",
	);
	let sed_input: Invocation = serde_json::from_value(serde_json::json!({
		"action": "apply",
		"values": {"target":"200","match":r"a\*b","replace":"x/y","global":"false"},
		"app": {
			"context": {"connected":true,"user":{"id":"1","name":"Me"},"channel":{"id":"20","name":"general","kind":0}},
			"timeline": {"channel_id":"20","truncated":false,"messages":[{"id":"200","author":{"id":"1","name":"Me"},"content":"a*b","attachment_count":0,"edited":false}]},
			"message_details": {"channel_id":"20","items":[{"id":"200","kind":0,"mention_ids":[],"mentions_truncated":false,"mention_everyone":false,"attachments":[],"attachments_truncated":false,"reactions_truncated":false}],"truncated":false}
		}
	}))?;
	for (name, action, package, activation) in [
		(
			"external-search-tools",
			"open",
			&external,
			&external_committed,
		),
		(
			"message-composition-tools",
			"open",
			&composition,
			&composition_committed,
		),
		("status-presets", "open", &status_presets, &status_committed),
		("draft-style-tools", "open", &draft_style, &draft_committed),
		(
			"command-palette",
			"show",
			&command_palette,
			&command_committed,
		),
		("own-message-sed", "show", &own_sed, &sed_committed),
	] {
		for module in [package, activation] {
			module.validate()?;
			let panel = invoke(
				module,
				&Invocation {
					action: action.into(),
					..Default::default()
				},
			)
			.unwrap_or_else(|error| panic!("{name}/{action}: settings panel failed: {error:?}"));
			assert!(!panel.panel.is_empty(), "{name}: settings panel is empty");
		}
	}
	for (name, package, committed, input, check) in [
		(
			"offline-composer-tools",
			&offline,
			&offline_committed,
			Invocation {
				action: "clear-urls".into(),
				composer: Some("https://example.org/?utm_source=test&keep=1".into()),
				..Default::default()
			},
			(|output: &Output| output.replacement.as_deref() == Some("https://example.org/?keep=1"))
				as fn(&Output) -> bool,
		),
		(
			"text-commands",
			&text_commands,
			&text_committed,
			Invocation {
				action: "apply-replacements".into(),
				composer: Some("brb".into()),
				storage: Some(r#"{"rules":[{"find":"brb","replace":"be right back","only_if_includes":""}],"tags":[]}"#.into()),
				..Default::default()
			},
			(|output: &Output| output.replacement.as_deref() == Some("be right back"))
				as fn(&Output) -> bool,
		),
		(
			"view-raw-message",
			&view_raw,
			&raw_committed,
			Invocation {
				action: "view-raw".into(),
				selected_message: Some("**synthetic raw text**".into()),
				..Default::default()
			},
			(|output: &Output| output.effects.is_empty()
				&& output.panel.iter().any(|element| {
					matches!(element, Element::Text { text } if text == "**synthetic raw text**")
				})) as fn(&Output) -> bool,
		),
		(
			"external-search-tools",
			&external,
			&external_committed,
			Invocation {
				action: "translate".into(),
				selected_message: Some("Synthetic translation".into()),
				..Default::default()
			},
			(|output: &Output| output.effects.len() == 1
				&& output.effects[0].required_capability() == extensions::Capability::ExternalNavigation)
				as fn(&Output) -> bool,
		),
		(
			"text-codecs",
			&codecs,
			&codecs_committed,
			Invocation {
				action: "decode".into(),
				composer: Some("SGVsbG8=".into()),
				..Default::default()
			},
			(|output: &Output| output.replacement.as_deref() == Some("Hello"))
				as fn(&Output) -> bool,
		),
		(
			"message-composition-tools",
			&composition,
			&composition_committed,
			composition_input,
			(|output: &Output| output.replacement.as_deref() == Some("keep\n"))
				as fn(&Output) -> bool,
		),
		(
			"identity-tools",
			&identity,
			&identity_committed,
			Invocation {
				action: "copy-user-url".into(),
				selected_message: Some("Synthetic private text".into()),
				selected_message_ref: Some(Box::new(extensions::SelectedMessageRef {
					channel_id: "100".into(),
					message_id: "200".into(),
					author_id: "300".into(),
					guild_id: None,
				})),
				..Default::default()
			},
			(|output: &Output| output.effects.len() == 1
				&& output.effects[0].required_capability() == extensions::Capability::ClipboardWrite)
				as fn(&Output) -> bool,
		),
		(
			"loaded-role-members",
			&role_members,
			&role_committed,
			role_members_input,
			(|output: &Output| output.effects.len() == 1
				&& output.effects[0].required_capability() == extensions::Capability::Navigation)
				as fn(&Output) -> bool,
		),
		(
			"status-presets",
			&status_presets,
			&status_committed,
			status_presets_input,
			(|output: &Output| output.effects.len() == 1
				&& output.effects[0].required_capability() == extensions::Capability::AccountControl)
				as fn(&Output) -> bool,
		),
		(
			"message-reference-tools",
			&reference_tools,
			&reference_committed,
			Invocation {
				action: "search".into(),
				selected_message: Some("Synthetic search".into()),
				selected_message_ref: Some(Box::new(extensions::SelectedMessageRef {
					channel_id: "100".into(),
					message_id: "200".into(),
					author_id: "300".into(),
					guild_id: None,
				})),
				..Default::default()
			},
			(|output: &Output| output.effects.len() == 1
				&& output.effects[0].required_capability() == extensions::Capability::ExternalNavigation)
				as fn(&Output) -> bool,
		),
		(
			"draft-style-tools",
			&draft_style,
			&draft_committed,
			Invocation {
				action: "reverse".into(),
				composer: Some("hello <@42>".into()),
				..Default::default()
			},
			(|output: &Output| output.replacement.as_deref() == Some("<@42> olleh"))
				as fn(&Output) -> bool,
		),
		(
			"command-palette",
			&command_palette,
			&command_committed,
			serde_json::from_value(serde_json::json!({
				"action":"go-destination", "values":{"destination":"Home (Friends)"}
			}))?,
			(|output: &Output| output.effects.len() == 1
				&& output.effects[0].required_capability() == extensions::Capability::Navigation)
				as fn(&Output) -> bool,
		),
		(
			"ingtoninator",
			&ingtoninator,
			&ington_committed,
			Invocation {
				action: "ingtoninate".into(),
				composer: Some("pin".into()),
				..Default::default()
			},
			(|output: &Output| output.replacement.as_deref() == Some("pington"))
				as fn(&Output) -> bool,
		),
		(
			"own-message-sed",
			&own_sed,
			&sed_committed,
			sed_input,
			(|output: &Output| output.effects.len() == 1
				&& output.effects[0].required_capability() == extensions::Capability::MessageManage)
				as fn(&Output) -> bool,
		),
		(
			"social-link-tools",
			&social,
			&social_committed,
			social_cancel_input,
			(|output: &Output| output.effects.len() == 1
				&& output.effects[0].required_capability() == extensions::Capability::RelationshipControl)
				as fn(&Output) -> bool,
		),
		(
			"message-history",
			&message_history,
			&history_committed,
			Invocation {
				action: "enable-history".into(),
				..Default::default()
			},
			(|output: &Output| {
				serde_json::to_value(output).unwrap()
					== serde_json::to_value(Output::default()).unwrap()
			}) as fn(&Output) -> bool,
		),
	] {
		package.validate()?;
		committed.validate()?;
		let output = invoke(committed, &input)?;
		assert!(check(&output), "{name}/committed: behavior failed");
		let output = invoke(package, &input)?;
		assert!(check(&output), "{name}/rebuilt: behavior failed");
		for effect in &output.effects {
			effect.validate(&package.manifest)?;
		}
		println!("{name}/rebuilt: {} Wasm bytes; real ABI behavior passed", package.wasm.len());
	}
	for module in [&social, &social_committed] {
		let normalized = invoke(module, &social_normalize_input)?;
		assert!(matches!(
			normalized.effects.as_slice(),
			[extensions::HostEffect::CopyText { text }] if text == "see https://discord.com/channels/1/2/3"
		));
		let repeated = invoke(module, &social_repeat_input)?;
		match &repeated.effects[0] {
			extensions::HostEffect::AppAction {
				action: extensions::AppAction::SendMessage {
					channel_id,
					content,
				},
			} => {
				assert_eq!(channel_id, "100");
				assert_eq!(content, "hi <@\u{200b}123> @\u{200b}everyone");
			}
			other => panic!("unexpected repeat effect: {other:?}"),
		}
	}
	for module in [&role_members, &role_committed] {
		let output = invoke(module, &role_color_input)?;
		assert!(matches!(
			output.effects.as_slice(),
			[extensions::HostEffect::CopyText { text }] if text == "#12ab34"
		));
	}
	// Every additional package crosses the real Wasm export/JSON boundary in both
	// its rebuilt and committed forms. Their inline suites cover exact behavior;
	// these no-data paths also verify graceful, bounded missing-snapshot handling.
	let additional: &[(&str, &str, &str, &[u8])] = &[
		(
			"account-navigation-tools",
			"open",
			include_str!("../../../extensions/plugins/account-navigation-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/account-navigation-tools.serein-extension"
			),
		),
		(
			"better-search-tools",
			"open",
			include_str!("../../../extensions/plugins/better-search-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/better-search-tools.serein-extension"
			),
		),
		(
			"equicord-community-tools",
			"open",
			include_str!("../../../extensions/plugins/equicord-community-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/equicord-community-tools.serein-extension"
			),
		),
		(
			"greet-sticker-tools",
			"open",
			include_str!("../../../extensions/plugins/greet-sticker-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/greet-sticker-tools.serein-extension"
			),
		),
		(
			"linked-message-tools",
			"preview",
			include_str!("../../../extensions/plugins/linked-message-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/linked-message-tools.serein-extension"
			),
		),
		(
			"message-inspector-tools",
			"copy-magnets",
			include_str!("../../../extensions/plugins/message-inspector-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/message-inspector-tools.serein-extension"
			),
		),
		(
			"message-navigation-tools",
			"jump-latest",
			include_str!("../../../extensions/plugins/message-navigation-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/message-navigation-tools.serein-extension"
			),
		),
		(
			"more-quick-reactions-tools",
			"open",
			include_str!("../../../extensions/plugins/more-quick-reactions-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/more-quick-reactions-tools.serein-extension"
			),
		),
		(
			"music-link-tools",
			"open",
			include_str!("../../../extensions/plugins/music-link-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/music-link-tools.serein-extension"
			),
		),
		(
			"permissions-tools",
			"show",
			include_str!("../../../extensions/plugins/permissions-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/permissions-tools.serein-extension"
			),
		),
		(
			"plus-reacts-tools",
			"plus-react",
			include_str!("../../../extensions/plugins/plus-reacts-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/plus-reacts-tools.serein-extension"
			),
		),
		(
			"server-info-tools",
			"show",
			include_str!("../../../extensions/plugins/server-info-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/server-info-tools.serein-extension"
			),
		),
		(
			"voice-and-chat-tools",
			"open",
			include_str!("../../../extensions/plugins/voice-and-chat-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/voice-and-chat-tools.serein-extension"
			),
		),
		(
			"voice-panel-tools",
			"open",
			include_str!("../../../extensions/plugins/voice-panel-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/voice-panel-tools.serein-extension"
			),
		),
		(
			"voice-rejoin-tools",
			"show",
			include_str!("../../../extensions/plugins/voice-rejoin-tools/manifest.json"),
			include_bytes!(
				"../../../extensions/plugins/packages/voice-rejoin-tools.serein-extension"
			),
		),
	];
	for &(stem, action, manifest, committed_bytes) in additional {
		let rebuilt_package = rebuilt(
			manifest,
			&catalog_wasm_dir.join(format!("{}.wasm", stem.replace('-', "_"))),
		)?;
		let committed_package = parse_package(committed_bytes)?;
		let input: Invocation = serde_json::from_value(serde_json::json!({"action": action}))?;
		let expected = invoke(&committed_package, &input)
			.unwrap_or_else(|error| panic!("{stem}/committed: invocation failed: {error:?}"));
		assert!(
			!expected.panel.is_empty()
				|| expected.replacement.is_some()
				|| expected.storage.is_some()
				|| !expected.effects.is_empty(),
			"{stem}/committed: missing-snapshot path returned no bounded feedback"
		);
		let actual = invoke(&rebuilt_package, &input)
			.unwrap_or_else(|error| panic!("{stem}/rebuilt: invocation failed: {error:?}"));
		assert_eq!(
			serde_json::to_value(&actual)?,
			serde_json::to_value(&expected)?,
			"{stem}: rebuilt and committed behavior differ"
		);
		for effect in &actual.effects {
			effect.validate(&rebuilt_package.manifest)?;
		}
		println!(
			"{stem}/committed+rebuilt: {} Wasm bytes; export and missing-snapshot behavior passed",
			rebuilt_package.wasm.len()
		);
	}
	Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let mut args = std::env::args_os().skip(1);
	let wasm_dir = PathBuf::from(args.next().expect("usage: sdk_check <wasm-directory>"));
	let catalog_wasm_dir = PathBuf::from(
		args.next()
			.expect("usage: sdk_check <wasm-directory> <catalog-wasm-directory>"),
	);
	assert!(
		args.next().is_none(),
		"usage: sdk_check <wasm-directory> <catalog-wasm-directory>"
	);
	check_safe_ports(&catalog_wasm_dir)?;
	check_app_actions(&rebuilt(
		include_str!("../../../examples/extensions/app-actions/manifest.json"),
		&wasm_dir.join("app_actions.wasm"),
	)?);
	let expected = Output {
		preserve_deleted_messages: true,
		..Default::default()
	};
	check(
		"message-delete-protector/committed",
		include_bytes!(
			"../../../extensions/plugins/packages/message-delete-protector.serein-extension"
		),
		&expected,
	);
	let protector = rebuilt(
		include_str!("../../../extensions/plugins/message-delete-protector/manifest.json"),
		&catalog_wasm_dir.join("message_delete_protector.wasm"),
	)?;
	check(
		"message-delete-protector/rebuilt",
		&serde_json::to_vec(&protector)?,
		&expected,
	);
	// Editor plugins have no fixed activation output; their rebuilt panel must still validate.
	for (name, manifest, wasm_file) in [
		(
			"custom-rpc",
			include_str!("../../../extensions/plugins/custom-rpc/manifest.json"),
			"custom_rpc.wasm",
		),
		(
			"api-proxy",
			include_str!("../../../extensions/plugins/api-proxy/manifest.json"),
			"api_proxy.wasm",
		),
	] {
		let package = rebuilt(manifest, &catalog_wasm_dir.join(wasm_file))?;
		package.validate()?;
		let output = invoke(
			&package,
			&Invocation {
				action: "open".into(),
				..Default::default()
			},
		)?;
		assert!(
			!output.panel.is_empty(),
			"{name}/rebuilt: editor panel is empty"
		);
		println!(
			"{name}/rebuilt: {} Wasm bytes; editor panel validated",
			package.wasm.len()
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
