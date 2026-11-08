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
			sdk::InterfaceTweak::UsernameBesideName,
			sdk::InterfaceTweak::OwnerCrown,
			sdk::InterfaceTweak::PlatformIndicators,
			sdk::InterfaceTweak::RoleColoredMentions,
			sdk::InterfaceTweak::ForumListView,
			sdk::InterfaceTweak::ForumGalleryView,
			sdk::InterfaceTweak::ForumSortByActivity,
			sdk::InterfaceTweak::ForumSortByCreation,
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

/// Checks that an SDK background effect reaches the host unchanged, that it needs the
/// `background_effects` grant, an event surface, the action's own grant and an allowed
/// notification or mute action, and that empty server notification changes are rejected.
#[test]
fn background_effects_are_granted_event_output_limited_to_allowed_actions() {
	let notifications = |level| sdk::HostEffect::AppAction {
		action: sdk::AppAction::SetServerNotifications {
			guild_id: "42".into(),
			level,
			muted: None,
			suppress_everyone: Some(true),
			suppress_roles: Some(true),
		},
	};
	let output = sdk::BackgroundEffectsOutput {
		background_effects: vec![notifications(Some(1))],
		..Default::default()
	};
	let host: extensions::Output =
		serde_json::from_slice(&serde_json::to_vec(&output).unwrap()).unwrap();
	assert_eq!(host.background_effects.len(), 1);
	let with = |surface, capabilities: &[Capability]| {
		let mut manifest = manifest("event", surface);
		manifest.capabilities.extend_from_slice(capabilities);
		let input = Invocation {
			action: "event".into(),
			selected_message: (surface == Surface::Message).then(|| "text".into()),
			..Default::default()
		};
		(manifest, input)
	};
	let both = [Capability::BackgroundEffects, Capability::ChannelControl];
	for (surface, allowed) in [
		(Surface::AppEvent, true),
		(Surface::MessageEvent, true),
		(Surface::Activation, false),
		(Surface::Panel, false),
		(Surface::Message, false),
	] {
		let (manifest, input) = with(surface, &both);
		assert_eq!(
			host.validate(&manifest, &input).is_ok(),
			allowed,
			"{surface:?}"
		);
	}
	for partial in [
		&[Capability::BackgroundEffects][..],
		&[Capability::ChannelControl][..],
	] {
		let (manifest, input) = with(Surface::AppEvent, partial);
		assert!(host.validate(&manifest, &input).is_err(), "{partial:?}");
	}
	let (manifest, input) = with(
		Surface::AppEvent,
		&[Capability::BackgroundEffects, Capability::MessageSend],
	);
	let send = extensions::Output {
		background_effects: vec![extensions::HostEffect::AppAction {
			action: extensions::AppAction::SendMessage {
				channel_id: "7".into(),
				content: "hello".into(),
			},
		}],
		..Default::default()
	};
	assert!(
		send.validate(&manifest, &input).is_err(),
		"sending is never a background change"
	);
	let (manifest, input) = with(Surface::AppEvent, &both);
	for bad in [notifications(Some(4)), notifications(None)] {
		let mut bad = serde_json::to_value(bad).unwrap();
		if bad["action"]["level"].is_null() {
			bad["action"]["suppress_everyone"] = serde_json::Value::Null;
			bad["action"]["suppress_roles"] = serde_json::Value::Null;
		}
		let output = extensions::Output {
			background_effects: vec![serde_json::from_value(bad).unwrap()],
			..Default::default()
		};
		assert!(output.validate(&manifest, &input).is_err());
	}
	let twice = extensions::Output {
		background_effects: vec![host.background_effects[0].clone(); 2],
		..Default::default()
	};
	assert!(
		twice.validate(&manifest, &input).is_err(),
		"one change per result"
	);
	let info = extensions::HostInfo::current();
	assert!(info.capabilities.contains(&Capability::BackgroundEffects));
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
