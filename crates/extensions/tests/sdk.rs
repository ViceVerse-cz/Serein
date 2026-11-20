use extensions::{Action, Capability, ExtensionKind, Invocation, Manifest, Output, Surface};
use serein_extension_sdk as sdk;

/// Builds a one-action plugin manifest whose capabilities are those the action's surface needs.
fn manifest(action: &str, surface: Surface) -> Manifest {
	let mut capabilities = vec![Capability::Storage];
	match surface {
		Surface::Message => capabilities.push(Capability::SelectedMessage),
		Surface::Composer => capabilities.push(Capability::Composer),
		Surface::Panel => capabilities.push(Capability::Appearance),
		Surface::Activation => {
			capabilities.extend([Capability::DeletedMessages, Capability::ImageSharing])
		}
		Surface::MessageEvent => capabilities.push(Capability::MessageEvents),
		Surface::AppEvent => capabilities.push(Capability::AppEvents),
		Surface::Tick => capabilities.push(Capability::Appearance),
		Surface::User | Surface::Channel | Surface::Server | Surface::Role => {
			capabilities.push(Capability::ContextMenus)
		}
	}
	let manifest = Manifest {
		api_version: extensions::API_VERSION,
		id: "sdk-check".into(),
		name: "SDK compatibility check".into(),
		version: "1.0.0".into(),
		author: "Serein contributors".into(),
		license: "MIT".into(),
		source: "https://example.org/source".into(),
		kind: ExtensionKind::Plugin,
		capabilities,
		actions: vec![Action {
			id: action.into(),
			label: action.into(),
			surface,
		}],
	};
	manifest.validate().unwrap();
	manifest
}

// Keep the original public struct-literal and fn(Invocation) -> Output style compiling.
fn legacy_handler(input: sdk::Invocation) -> sdk::Output {
	sdk::Output {
		image_sharing: input.action == "activate",
		preserve_deleted_messages: input.action == "activate",
		appearance: None,
		replacement: input.composer,
		panel: Vec::new(),
		storage: input.storage,
	}
}
/// Round-trips each surface's invocation through the SDK wire format and checks that legacy
/// handlers and the host validator agree on which inputs each surface may carry.
#[test]
fn host_invocations_and_legacy_handlers_keep_the_v1_contract() {
	assert_eq!(sdk::API_VERSION, extensions::API_VERSION);
	assert_eq!(sdk::MAX_IO_BYTES, extensions::MAX_IO_BYTES);
	for (action, surface) in [
		("message", Surface::Message),
		("compose", Surface::Composer),
		("apply", Surface::Panel),
		("activate", Surface::Activation),
		("user", Surface::User),
		("channel", Surface::Channel),
		("server", Surface::Server),
		("role", Surface::Role),
	] {
		let manifest = manifest(action, surface);
		let input = Invocation {
			action: action.into(),
			selected_message: (action == "message").then(|| "Selected message".into()),
			selected_message_ref: (action == "message").then(|| {
				Box::new(extensions::SelectedMessageRef {
					channel_id: "2".into(),
					message_id: "3".into(),
					author_id: "4".into(),
					guild_id: None,
				})
			}),
			composer: (action == "compose").then(|| "Draft".into()),
			menu_target: match surface {
				Surface::User => Some(extensions::MenuTarget::User {
					user_id: "4".into(),
					guild_id: Some("5".into()),
					channel_id: None,
				}),
				Surface::Channel => Some(extensions::MenuTarget::Channel {
					channel_id: "2".into(),
					guild_id: None,
				}),
				Surface::Server => Some(extensions::MenuTarget::Server {
					guild_id: "5".into(),
				}),
				Surface::Role => Some(extensions::MenuTarget::Role {
					guild_id: "5".into(),
					role_id: "6".into(),
					user_id: Some("4".into()),
					name: "Moderator".into(),
					color: 0x3498db,
				}),
				_ => None,
			}
			.map(Box::new),
			storage: Some(r#"{"size":16}"#.into()),
			values: [("size".into(), "18".into())].into(),
			..Default::default()
		};
		input.validate(&manifest).unwrap();
		let bytes = serde_json::to_vec(&input).unwrap();
		assert!(
			serde_json::from_slice::<serde_json::Value>(&bytes)
				.unwrap()
				.get("message_event")
				.is_none()
		);
		let decoded: sdk::Invocation = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(
			decoded,
			sdk::Invocation {
				action: input.action.clone(),
				selected_message: input.selected_message.clone(),
				composer: input.composer.clone(),
				storage: input.storage.clone(),
				values: input.values.clone(),
				double_clicked: false,
			}
		);
		if let Some(target) = &input.menu_target {
			// Menu plugins read the target; a menu action never runs without one or with another kind.
			let menu: sdk::MenuInvocation = serde_json::from_slice(&bytes).unwrap();
			assert_eq!(
				serde_json::to_value(menu.menu_target.unwrap()).unwrap(),
				serde_json::to_value(target).unwrap()
			);
			let mut missing = input.clone();
			missing.menu_target = None;
			assert!(missing.validate(&manifest).is_err());
			let mut other = input.clone();
			other.menu_target = Some(Box::new(match surface {
				Surface::Server => extensions::MenuTarget::User {
					user_id: "4".into(),
					guild_id: None,
					channel_id: None,
				},
				_ => extensions::MenuTarget::Server {
					guild_id: "5".into(),
				},
			}));
			assert!(other.validate(&manifest).is_err());
		} else {
			let mut stray = input.clone();
			stray.menu_target = Some(Box::new(extensions::MenuTarget::Server {
				guild_id: "5".into(),
			}));
			assert!(stray.validate(&manifest).is_err());
		}
		if let Some(target) = &input.selected_message_ref {
			// Message plugins can identify the chosen message; legacy handlers still decode.
			let message: sdk::MessageActionInvocation = serde_json::from_slice(&bytes).unwrap();
			let decoded = message.selected_message_ref.unwrap();
			assert_eq!(
				(decoded.channel_id, decoded.message_id, decoded.author_id),
				(
					target.channel_id.clone(),
					target.message_id.clone(),
					target.author_id.clone()
				)
			);
			// The IDs only travel with the selected text, and must be real IDs.
			let mut detached = input.clone();
			detached.selected_message = None;
			assert!(detached.validate(&manifest).is_err());
			let mut forged = input.clone();
			forged.selected_message_ref.as_mut().unwrap().message_id = "not-an-id".into();
			assert!(forged.validate(&manifest).is_err());
		}
		let response = sdk::dispatch(&bytes, legacy_handler).unwrap();
		let output: Output = serde_json::from_slice(&response).unwrap();
		output.validate(&manifest, &input).unwrap();
		assert_eq!(output.replacement, input.composer);
		assert_eq!(output.storage, input.storage);
		assert_eq!(output.image_sharing, action == "activate");
		assert_eq!(output.preserve_deleted_messages, action == "activate");
	}

	{
		let sdk: sdk::Output = serde_json::from_str("{}").unwrap();
		assert_eq!(sdk, sdk::Output::default());
		let bytes = sdk::dispatch(br#"{"action":"apply","future_context":true}"#, |_| sdk).unwrap();
		let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
		assert!(wire.get("image_sharing").is_none());
		assert!(wire.get("preserve_deleted_messages").is_none());
		assert!(wire.get("appearance").is_none());
		let host: Output = serde_json::from_slice(&bytes).unwrap();
		assert!(!host.image_sharing && !host.preserve_deleted_messages);
		host.validate(
			&manifest("apply", Surface::Panel),
			&Invocation {
				action: "apply".into(),
				..Default::default()
			},
		)
		.unwrap();
	}

	{
		assert_eq!(
			sdk::MAX_EVENT_CONTENT_BYTES,
			extensions::MAX_EVENT_CONTENT_BYTES
		);
		let legacy = sdk::Invocation {
			action: "show".into(),
			selected_message: None,
			composer: None,
			storage: None,
			values: Default::default(),
			double_clicked: false,
		};
		let legacy_bytes = serde_json::to_vec(&legacy).unwrap();
		let extended: sdk::EventInvocation = serde_json::from_slice(&legacy_bytes).unwrap();
		assert_eq!(extended.invocation, legacy);
		assert!(extended.message_event.is_none());
		assert_eq!(serde_json::to_vec(&extended).unwrap(), legacy_bytes);
		for kind in [
			extensions::MessageEventKind::Create,
			extensions::MessageEventKind::Update,
			extensions::MessageEventKind::Delete,
		] {
			let input = Invocation {
				action: "event".into(),
				message_event: Some(Box::new(extensions::MessageEvent {
					kind,
					channel_id: "18446744073709551615".into(),
					message_id: "2".into(),
					author_id: (kind == extensions::MessageEventKind::Create).then(|| "3".into()),
					content: (kind == extensions::MessageEventKind::Create).then(|| "hello".into()),
				})),
				..Default::default()
			};
			let manifest = manifest("event", Surface::MessageEvent);
			input.validate(&manifest).unwrap();
			let bytes = serde_json::to_vec(&input).unwrap();
			let expected_event =
				serde_json::to_value(input.message_event.as_ref().unwrap()).unwrap();
			let response = sdk::dispatch_typed(&bytes, |input: sdk::EventInvocation| {
				assert_eq!(input.invocation.action, "event");
				let event = input.message_event.unwrap();
				assert_eq!(serde_json::to_value(&event).unwrap(), expected_event);
				let mut output = sdk::Output::default();
				output.set_storage_json(&event.kind).unwrap();
				output
			})
			.unwrap();
			let output: Output = serde_json::from_slice(&response).unwrap();
			output.validate(&manifest, &input).unwrap();
			assert!(output.storage.is_some());
		}
	}
}

/// Checks that role targets reject a malformed ID in any field, an out-of-range color and an
/// overlong or multi-line name.
#[test]
fn context_menu_targets_reject_malformed_role_fields() {
	let manifest = manifest("role", Surface::Role);
	let role = |guild: &str, role: &str, user: Option<&str>, name: &str, color: u32| Invocation {
		action: "role".into(),
		menu_target: Some(Box::new(extensions::MenuTarget::Role {
			guild_id: guild.into(),
			role_id: role.into(),
			user_id: user.map(Into::into),
			name: name.into(),
			color,
		})),
		..Default::default()
	};
	let longest = "é".repeat(extensions::MAX_ROLE_NAME_CHARS);
	assert!(
		role("5", "6", Some("4"), "Mod", 0xff_ffff)
			.validate(&manifest)
			.is_ok()
	);
	assert!(
		role("5", "6", None, &longest, 0)
			.validate(&manifest)
			.is_ok()
	);
	for bad in [
		role("x", "6", None, "Mod", 0),
		role("5", "", None, "Mod", 0),
		role("5", "6", Some("-1"), "Mod", 0),
		role("5", "6", None, "Mod", 0x100_0000),
		role("5", "6", None, &format!("{longest}e"), 0),
		role("5", "6", None, "Mod\nerator", 0),
	] {
		assert!(bad.validate(&manifest).is_err());
	}
}

/// Checks that SDK interface-tweak output reaches the host unchanged, that only granted
/// activation and panel actions may set tweaks, and that a tweak listed twice is rejected.
#[test]
fn interface_tweaks_are_granted_activation_or_panel_output_and_unique() {
	let output = sdk::InterfaceTweaksOutput {
		interface_tweaks: Some(vec![
			sdk::InterfaceTweak::StaticTypingIndicator,
			sdk::InterfaceTweak::PlainProfiles,
			sdk::InterfaceTweak::ExpandedRoles,
			sdk::InterfaceTweak::HiddenRequestCounts,
			sdk::InterfaceTweak::OnePingPerDirectMessage,
			sdk::InterfaceTweak::HiddenBlockedMessages,
			sdk::InterfaceTweak::HiddenProfileNotes,
			sdk::InterfaceTweak::LargeStreamPreview,
			sdk::InterfaceTweak::DoubleClickReplyEdit,
		]),
		..Default::default()
	};
	let wire = serde_json::to_vec(&output).unwrap();
	let host: extensions::Output = serde_json::from_slice(&wire).unwrap();
	assert_eq!(
		host.interface_tweaks.as_deref(),
		Some(extensions::InterfaceTweak::ALL)
	);
	let names: Vec<String> = extensions::InterfaceTweak::ALL
		.iter()
		.map(|tweak| {
			serde_json::to_value(tweak)
				.unwrap()
				.as_str()
				.unwrap()
				.to_owned()
		})
		.collect();
	let info: sdk::HostInfo =
		serde_json::from_value(serde_json::to_value(extensions::HostInfo::current()).unwrap())
			.unwrap();
	assert!(names.iter().all(|name| info.supports_tweak(name)));
	let with = |action: &str, surface, capabilities: Vec<Capability>| {
		let mut manifest = manifest(action, surface);
		manifest.capabilities.extend(capabilities);
		let input = Invocation {
			action: action.into(),
			selected_message: (surface == Surface::Message).then(|| "text".into()),
			..Default::default()
		};
		(manifest, input)
	};
	let tweaks = vec![Capability::InterfaceTweaks];
	for (surface, action, allowed) in [
		(Surface::Activation, "activate", true),
		(Surface::Panel, "apply", true),
		(Surface::Message, "message", false),
	] {
		let (granted, input) = with(action, surface, tweaks.clone());
		assert_eq!(host.validate(&granted, &input).is_ok(), allowed, "{action}");
		let (ungranted, input) = with(action, surface, vec![]);
		assert!(
			host.validate(&ungranted, &input).is_err(),
			"{action} without grant"
		);
	}
	let (panel, panel_input) = with("apply", Surface::Panel, tweaks);
	let off = extensions::Output {
		interface_tweaks: Some(Vec::new()),
		..Default::default()
	};
	assert!(off.validate(&panel, &panel_input).is_ok());
	let mut twice = host.clone();
	twice
		.interface_tweaks
		.as_mut()
		.unwrap()
		.push(extensions::InterfaceTweak::PlainProfiles);
	assert!(twice.validate(&panel, &panel_input).is_err());
	let old: sdk::HostInfo = serde_json::from_value(serde_json::json!({
		"api_version": 1, "sdk_revision": 1, "capabilities": [], "app_events": []
	}))
	.unwrap();
	assert!(!old.supports_tweak("plain_profiles"));
}

/// Checks that a granted plugin's interface tree reaches the host unchanged, that only
/// activation, panel and event results may restate it, that an off-bounds tree is rejected
/// like any invalid output, and that a result without one clears it.
#[test]
fn interface_ui_is_granted_activation_panel_or_event_output_and_validated() {
	let tree = sdk::InterfaceUi {
		nodes: vec![
			sdk::InterfaceNode {
				id: "server:10".into(),
				indent: 0,
				icon: Some(sdk::InterfaceIcon::Server),
				text: "Voice club".into(),
				badge: Some("3".into()),
				emphasis: false,
				selected: false,
				tint: None,
				native: Some(sdk::InterfaceNative::JoinVoice {
					channel_id: "30".into(),
				}),
			},
			sdk::InterfaceNode {
				id: String::new(),
				indent: 2,
				icon: Some(sdk::InterfaceIcon::MicrophoneSlash),
				text: "Quill Harper".into(),
				badge: None,
				emphasis: false,
				selected: false,
				tint: Some(sdk::InterfaceTint::Danger),
				native: None,
			},
		],
		window: Some(sdk::InterfaceWindow::Chat {
			channel_id: "20".into(),
		}),
		dock: vec![sdk::InterfaceDockItem {
			id: "dock:10".into(),
			label: "Voice club".into(),
			guild_id: Some("10".into()),
			selected: true,
		}],
		add_server: true,
		controls: vec![sdk::InterfaceControl::Mute, sdk::InterfaceControl::Settings],
		size: Some(sdk::InterfaceSize {
			width: 360,
			height: 640,
		}),
	};
	let expected: extensions::InterfaceUi =
		serde_json::from_value(serde_json::to_value(&tree).unwrap()).unwrap();
	let output = sdk::InterfaceUiOutput {
		interface_ui: Some(tree),
		..Default::default()
	};
	let host: extensions::Output =
		serde_json::from_slice(&serde_json::to_vec(&output).unwrap()).unwrap();
	assert_eq!(host.interface_ui.as_ref(), Some(&expected));
	assert_eq!(
		expected.controls,
		[
			extensions::InterfaceControl::Mute,
			extensions::InterfaceControl::Settings
		],
		"controls reach the host in order"
	);
	assert_eq!(
		expected.nodes[0].native,
		Some(extensions::InterfaceNative::JoinVoice {
			channel_id: "30".into()
		}),
		"a native join reaches the host"
	);
	assert_eq!(
		expected.size,
		Some(extensions::InterfaceSize {
			width: 360,
			height: 640
		})
	);
	let info = serde_json::to_value(extensions::HostInfo::current()).unwrap();
	assert!(
		info["capabilities"]
			.as_array()
			.unwrap()
			.iter()
			.any(|capability| capability == "interface_ui")
	);
	let interface = extensions::Output {
		interface_ui: Some(expected.clone()),
		..Default::default()
	};
	let with = |action: &str, surface, granted: bool| {
		let mut manifest = manifest(action, surface);
		if granted {
			manifest.capabilities.push(Capability::InterfaceUi);
		}
		let input = Invocation {
			action: action.into(),
			..Default::default()
		};
		(manifest, input)
	};
	for (action, surface, allowed) in [
		("activate", Surface::Activation, true),
		("apply", Surface::Panel, true),
		("message-event", Surface::MessageEvent, true),
		("app-event", Surface::AppEvent, true),
		("message", Surface::Message, false),
		("compose", Surface::Composer, false),
		("tick", Surface::Tick, false),
	] {
		let (granted, input) = with(action, surface, true);
		assert_eq!(
			interface.validate(&granted, &input).is_ok(),
			allowed,
			"{action}"
		);
		let (ungranted, input) = with(action, surface, false);
		assert!(
			interface.validate(&ungranted, &input).is_err(),
			"{action} without grant"
		);
	}
	let (panel, panel_input) = with("apply", Surface::Panel, true);
	let mut duplicate = interface.clone();
	let first = duplicate
		.interface_ui
		.as_ref()
		.unwrap()
		.nodes
		.first()
		.cloned()
		.unwrap();
	duplicate.interface_ui.as_mut().unwrap().nodes.push(first);
	assert!(duplicate.validate(&panel, &panel_input).is_err());
	let mut repeated = interface.clone();
	repeated
		.interface_ui
		.as_mut()
		.unwrap()
		.controls
		.push(extensions::InterfaceControl::Mute);
	assert!(
		repeated.validate(&panel, &panel_input).is_err(),
		"repeated control rejected"
	);
	let maximal = extensions::InterfaceUi {
		nodes: (0..extensions::MAX_INTERFACE_NODES)
			.map(|row| extensions::InterfaceNode {
				id: format!("row-{row}"),
				indent: 0,
				icon: None,
				text: "x".repeat(extensions::MAX_INTERFACE_TEXT_BYTES),
				badge: None,
				emphasis: false,
				selected: false,
				tint: None,
				native: None,
			})
			.collect(),
		window: Some(extensions::InterfaceWindow::Call),
		dock: Vec::new(),
		add_server: false,
		controls: vec![
			extensions::InterfaceControl::Disconnect,
			extensions::InterfaceControl::Mute,
			extensions::InterfaceControl::Deafen,
			extensions::InterfaceControl::Settings,
		],
		size: Some(extensions::InterfaceSize {
			width: extensions::MAX_INTERFACE_WINDOW_WIDTH,
			height: extensions::MAX_INTERFACE_WINDOW_HEIGHT,
		}),
	};
	assert!(maximal.validate().is_ok());
	// A chat window must name a snowflake channel id.
	for bad in ["", "general", "1".repeat(21).as_str()] {
		let chat = extensions::InterfaceUi {
			nodes: Vec::new(),
			window: Some(extensions::InterfaceWindow::Chat {
				channel_id: bad.into(),
			}),
			dock: Vec::new(),
			add_server: false,
			controls: Vec::new(),
			size: None,
		};
		assert!(chat.validate().is_err(), "{bad:?} rejected");
	}
	let mut bounded = interface.clone();
	bounded.interface_ui = Some(maximal);
	assert!(bounded.validate(&panel, &panel_input).is_ok());
	let cleared: extensions::Output =
		serde_json::from_value(serde_json::to_value(sdk::Output::default()).unwrap()).unwrap();
	assert!(
		cleared.interface_ui.is_none(),
		"a result without a tree restores the normal layout"
	);
	// One panel result may restate the tree and carry a host effect together; each part
	// needs its own grant and the combination itself is never rejected.
	let combined = extensions::Output {
		interface_ui: Some(expected),
		effects: vec![extensions::HostEffect::Notice {
			text: "Joined".into(),
		}],
		..Default::default()
	};
	let mut both = panel.clone();
	both.capabilities.push(Capability::LocalNotices);
	assert!(combined.validate(&both, &panel_input).is_ok());
	assert!(
		combined.validate(&panel, &panel_input).is_err(),
		"effect without its grant"
	);
	both.capabilities
		.retain(|cap| *cap != Capability::InterfaceUi);
	assert!(
		combined.validate(&both, &panel_input).is_err(),
		"tree without its grant"
	);
	// The reserved row action keeps its SDK spelling; other underscored ids stay invalid.
	let mut clicks = panel.clone();
	clicks.actions.push(Action {
		id: sdk::INTERFACE_CLICK_ACTION.into(),
		label: "Interface row".into(),
		surface: Surface::Panel,
	});
	assert_eq!(
		sdk::INTERFACE_CLICK_ACTION,
		extensions::INTERFACE_CLICK_ACTION
	);
	assert!(clicks.validate().is_ok());
	clicks.actions.last_mut().unwrap().id = "other_click".into();
	assert!(clicks.validate().is_err());
}

/// Checks that the SDK's start screen reaches the host field for field with omitted, `null`
/// and value kept apart, that only granted activation and panel results may carry it (a
/// clear included), and that an off-bounds screen is rejected like any invalid output.
#[test]
fn start_screen_is_granted_activation_or_panel_output_and_validated() {
	let screen = sdk::StartScreen {
		images: vec![sdk::StartImage {
			width: 2,
			height: 2,
			palette: vec!["#33aa55".into(), "#ffffff80".into()],
			frames: vec!["bA==".into(), "AA==".into()],
			fps: 24,
			cell: 4,
		}],
		title: Some("Loading".into()),
		subtitle: Some("Connecting".into()),
		window: Some(sdk::StartWindow {
			width: 480,
			height: 360,
		}),
	};
	let wire = |start_screen| {
		let output = sdk::StartScreenOutput {
			start_screen,
			..Default::default()
		};
		serde_json::from_slice::<Output>(&serde_json::to_vec(&output).unwrap()).unwrap()
	};
	assert!(wire(None).start_screen.is_none(), "omitted keeps it");
	let cleared = wire(Some(None));
	assert_eq!(cleared.start_screen, Some(None), "null clears it");
	let host = wire(Some(Some(screen.clone())));
	let value = host.start_screen.clone().flatten().unwrap();
	assert_eq!(
		serde_json::to_value(&value).unwrap(),
		serde_json::to_value(&screen).unwrap(),
		"every field reaches the host"
	);
	let with = |action: &str, surface, granted: bool| {
		let mut manifest = manifest(action, surface);
		if granted {
			manifest.capabilities.push(Capability::StartScreen);
		}
		let input = Invocation {
			action: action.into(),
			..Default::default()
		};
		(manifest, input)
	};
	for (action, surface, allowed) in [
		("activate", Surface::Activation, true),
		("apply", Surface::Panel, true),
		("message-event", Surface::MessageEvent, false),
		("app-event", Surface::AppEvent, false),
		("tick", Surface::Tick, false),
	] {
		let (granted, input) = with(action, surface, true);
		for output in [&host, &cleared] {
			assert_eq!(
				output.validate(&granted, &input).is_ok(),
				allowed,
				"{action}"
			);
		}
		let (ungranted, input) = with(action, surface, false);
		assert!(
			host.validate(&ungranted, &input).is_err(),
			"{action} without grant"
		);
	}
	let (panel, panel_input) = with("apply", Surface::Panel, true);
	let mut invalid = host.clone();
	if let Some(Some(screen)) = &mut invalid.start_screen {
		screen.images[0].frames[0] = "bAA=".into();
	}
	assert!(
		invalid.validate(&panel, &panel_input).is_err(),
		"a frame longer than its packed cells rejected"
	);
}

/// Checks that the voice roster stays absent from older snapshots, survives a populated
/// SDK-to-host round trip with ids, names and flags, and is bounded and id-checked by the
/// host validator.
#[test]
fn voice_roster_defaults_and_validates() {
	use extensions::{AppSnapshot, VoiceRosterEntry, VoiceSnapshot};
	let old: VoiceSnapshot = serde_json::from_value(serde_json::json!({
		"phase": "idle", "muted": false, "deafened": false, "camera": false,
		"streaming": false, "participants": []
	}))
	.unwrap();
	assert!(old.roster.is_empty(), "older snapshots have no roster");
	let sdk_entry = sdk::VoiceRosterEntry {
		guild_id: "10".into(),
		channel_id: "30".into(),
		user_id: "7".into(),
		name: Some("Quill Harper".into()),
		muted: true,
		deafened: false,
		server_muted: false,
		server_deafened: true,
		speaking: false,
	};
	let sdk_voice = sdk::VoiceSnapshot {
		channel_id: Some("30".into()),
		phase: "connected".into(),
		muted: false,
		deafened: false,
		camera: false,
		streaming: false,
		participants: vec!["7".into()],
		roster: vec![
			sdk_entry.clone(),
			sdk::VoiceRosterEntry {
				name: None,
				..sdk_entry.clone()
			},
		],
	};
	let decoded: VoiceSnapshot =
		serde_json::from_slice(&serde_json::to_vec(&sdk_voice).unwrap()).unwrap();
	assert_eq!(decoded.channel_id.as_deref(), Some("30"));
	assert_eq!(decoded.roster.len(), 2);
	assert_eq!(decoded.roster[0].guild_id, "10");
	assert_eq!(decoded.roster[0].channel_id, "30");
	assert_eq!(decoded.roster[0].user_id, "7");
	assert_eq!(decoded.roster[0].name.as_deref(), Some("Quill Harper"));
	assert_eq!(decoded.roster[0].muted, sdk_entry.muted);
	assert_eq!(decoded.roster[0].server_deafened, sdk_entry.server_deafened);
	assert_eq!(decoded.roster[0].speaking, sdk_entry.speaking);
	assert!(
		decoded.roster[1].name.is_none(),
		"unnamed rows stay unnamed"
	);
	let entry = VoiceRosterEntry {
		guild_id: "10".into(),
		channel_id: "30".into(),
		user_id: "7".into(),
		name: Some("Quill Harper".into()),
		muted: true,
		deafened: false,
		server_muted: false,
		server_deafened: false,
		speaking: false,
	};
	let mut manifest = manifest("apply", Surface::Panel);
	manifest.capabilities.push(Capability::VoiceState);
	let mut snapshot = AppSnapshot {
		voice: Some(VoiceSnapshot {
			roster: vec![entry.clone()],
			..old
		}),
		..Default::default()
	};
	assert!(snapshot.validate(&manifest).is_ok());
	let voice = snapshot.voice.as_mut().unwrap();
	voice.roster[0].user_id = "not-an-id".into();
	assert!(snapshot.validate(&manifest).is_err(), "bad id rejected");
	let voice = snapshot.voice.as_mut().unwrap();
	voice.roster = vec![entry; extensions::MAX_VOICE_ROSTER + 1];
	assert!(
		snapshot.validate(&manifest).is_err(),
		"oversized roster rejected"
	);
}

#[test]
fn every_sdk_panel_and_appearance_field_is_accepted_by_the_host() {
	use sdk::Element::*;
	let palette = sdk::ThemePalette {
		background: Some(sdk::Background {
			opacity: 30,
			fit: sdk::BackgroundFit::Contain,
			target: sdk::BackgroundTarget::Chat,
			sections: Some(sdk::SectionOpacity::default()),
		}),
		colors: [("accent".into(), "#12345678".into())].into(),
		backdrop: Some(["#123456".into(), "#654321".into()]),
	};
	let expected = sdk::Output {
		appearance: Some(sdk::Theme {
			light: palette.clone(),
			dark: palette,
			style: sdk::ThemeStyle {
				transparency_blur: Some(true),
				transparency: Some(25),
				blur: Some(50),
				body_size: Some(16),
				heading_size: Some(20),
				button_size: Some(14),
				icon_size: Some(20),
				small_size: Some(12),
				monospace_size: Some(14),
				item_spacing: Some([8, 8]),
				button_padding: Some([12, 6]),
				control_height: Some(32),
				widget_radius: Some(8),
				window_radius: Some(12),
				menu_radius: Some(12),
			},
		}),
		panel: vec![
			Heading {
				text: "Settings".into(),
			},
			Text {
				text: "Offline SDK check".into(),
			},
			Separator,
			Row {
				children: vec![
					TextInput {
						id: "name".into(),
						label: "Name".into(),
						value: "Serein".into(),
					},
					Checkbox {
						id: "enabled".into(),
						label: "Enabled".into(),
						checked: true,
					},
					Select {
						id: "density".into(),
						label: "Density".into(),
						options: vec!["Compact".into(), "Comfortable".into()],
						value: "Compact".into(),
					},
					Slider {
						id: "size".into(),
						label: "Size".into(),
						min: 10,
						max: 28,
						value: 16,
					},
					Button {
						id: "apply".into(),
						label: "Apply".into(),
					},
				],
			},
		],
		..Default::default()
	};
	let input = Invocation {
		action: "apply".into(),
		..Default::default()
	};
	let bytes = sdk::dispatch(&serde_json::to_vec(&input).unwrap(), |_| expected.clone()).unwrap();
	let host: Output = serde_json::from_slice(&bytes).unwrap();
	host.validate(&manifest("apply", Surface::Panel), &input)
		.unwrap();
	let roundtrip: sdk::Output =
		serde_json::from_slice(&serde_json::to_vec(&host).unwrap()).unwrap();
	assert_eq!(roundtrip, expected);
}

/// Checks that the host mirror of the interface contract keeps every bound and verdict in
/// step with the SDK: the interface and start-screen constants agree, and both validators
/// accept and reject the same candidate trees with the same errors.
#[test]
fn interface_contract_bounds_and_verdicts_match_the_sdk() {
	for (kit, host) in [
		(sdk::MAX_INTERFACE_NODES, extensions::MAX_INTERFACE_NODES),
		(
			sdk::MAX_INTERFACE_TEXT_BYTES,
			extensions::MAX_INTERFACE_TEXT_BYTES,
		),
		(
			sdk::MAX_INTERFACE_BADGE_BYTES,
			extensions::MAX_INTERFACE_BADGE_BYTES,
		),
		(
			sdk::MAX_INTERFACE_ID_BYTES,
			extensions::MAX_INTERFACE_ID_BYTES,
		),
		(
			sdk::MAX_INTERFACE_DOCK_ITEMS,
			extensions::MAX_INTERFACE_DOCK_ITEMS,
		),
		(
			sdk::MAX_START_TITLE_BYTES,
			extensions::MAX_START_TITLE_BYTES,
		),
		(
			sdk::MAX_START_SUBTITLE_BYTES,
			extensions::MAX_START_SUBTITLE_BYTES,
		),
		(sdk::MAX_START_IMAGES, extensions::MAX_START_IMAGES),
		(sdk::MAX_START_FRAMES, extensions::MAX_START_FRAMES),
		(sdk::MAX_START_PALETTE, extensions::MAX_START_PALETTE),
		(
			sdk::MAX_START_FRAME_BYTES,
			extensions::MAX_START_FRAME_BYTES,
		),
	] {
		assert_eq!(kit, host);
	}
	for (kit, host) in [
		(sdk::MAX_INTERFACE_INDENT, extensions::MAX_INTERFACE_INDENT),
		(sdk::MAX_START_FPS, extensions::MAX_START_FPS),
		(
			sdk::MAX_START_CELL_PIXELS,
			extensions::MAX_START_CELL_PIXELS,
		),
	] {
		assert_eq!(kit, host);
	}
	for (kit, host) in [
		(
			sdk::MIN_INTERFACE_WINDOW_SIDE,
			extensions::MIN_INTERFACE_WINDOW_SIDE,
		),
		(
			sdk::MAX_INTERFACE_WINDOW_WIDTH,
			extensions::MAX_INTERFACE_WINDOW_WIDTH,
		),
		(
			sdk::MAX_INTERFACE_WINDOW_HEIGHT,
			extensions::MAX_INTERFACE_WINDOW_HEIGHT,
		),
		(
			sdk::MAX_START_IMAGE_CELLS,
			extensions::MAX_START_IMAGE_CELLS,
		),
		(
			sdk::MIN_START_WINDOW_SIDE,
			extensions::MIN_START_WINDOW_SIDE,
		),
		(
			sdk::MAX_START_WINDOW_WIDTH,
			extensions::MAX_START_WINDOW_WIDTH,
		),
		(
			sdk::MAX_START_WINDOW_HEIGHT,
			extensions::MAX_START_WINDOW_HEIGHT,
		),
	] {
		assert_eq!(kit, host);
	}
	let valid = serde_json::json!({
		"nodes": [
			{"id": "channel:25", "text": "Room", "badge": "2", "indent": 1},
			{"text": "Plain label"},
			{"id": "join:37", "text": "Lobby", "native": {"type": "join_voice", "channel_id": "37"}}
		],
		"dock": [{"id": "server:10", "label": "Voice club", "guild_id": "10"}],
		"window": {"kind": "chat", "channel_id": "20"},
		"size": {"width": 480, "height": 720}
	});
	for (label, json, ok) in [
		("valid tree", valid.clone(), true),
		(
			"control character in row text",
			serde_json::json!({"nodes": [{"id": "a", "text": "bad\u{7}text"}]}),
			false,
		),
		(
			"whitespace-only row text",
			serde_json::json!({"nodes": [{"id": "a", "text": "  \t "}]}),
			false,
		),
		(
			"whitespace-only badge",
			serde_json::json!({"nodes": [{"id": "a", "text": "ok", "badge": " "}]}),
			false,
		),
		(
			"control character in dock label",
			serde_json::json!({"nodes": [{"text": "ok"}], "dock": [{"id": "d", "label": "a\u{1}"}]}),
			false,
		),
		(
			"oversized indent",
			serde_json::json!({"nodes": [{"text": "ok", "indent": sdk::MAX_INTERFACE_INDENT + 1}]}),
			false,
		),
		(
			"non-snowflake chat window",
			serde_json::json!({"nodes": [{"text": "ok"}], "window": {"kind": "chat", "channel_id": "x"}}),
			false,
		),
		(
			"oversized window",
			serde_json::json!({
				"nodes": [{"text": "ok"}],
				"size": {"width": sdk::MAX_INTERFACE_WINDOW_WIDTH + 1, "height": 600}
			}),
			false,
		),
	] {
		let host: extensions::InterfaceUi = serde_json::from_value(json.clone()).unwrap();
		let kit: sdk::InterfaceUi = serde_json::from_value(json).unwrap();
		assert_eq!(host.validate(), kit.validate(), "{label} agrees");
		assert_eq!(host.validate().is_ok(), ok, "{label}");
	}
}
