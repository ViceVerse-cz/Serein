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
