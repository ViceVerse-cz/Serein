use extensions::{Action, Capability, ExtensionKind, Invocation, Manifest, Output, Surface};
use serein_extension_sdk as sdk;

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
#[test]
fn host_invocations_and_legacy_handlers_keep_the_v1_contract() {
	assert_eq!(sdk::API_VERSION, extensions::API_VERSION);
	assert_eq!(sdk::MAX_IO_BYTES, extensions::MAX_IO_BYTES);
	for (action, surface) in [
		("message", Surface::Message),
		("compose", Surface::Composer),
		("apply", Surface::Panel),
		("activate", Surface::Activation),
	] {
		let manifest = manifest(action, surface);
		let input = Invocation {
			action: action.into(),
			selected_message: (action == "message").then(|| "Selected message".into()),
			composer: (action == "compose").then(|| "Draft".into()),
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
		let response = sdk::dispatch(&bytes, legacy_handler).unwrap();
		let output: Output = serde_json::from_slice(&response).unwrap();
		output.validate(&manifest, &input).unwrap();
		assert_eq!(output.replacement, input.composer);
		assert_eq!(output.storage, input.storage);
		assert_eq!(output.image_sharing, action == "activate");
		assert_eq!(output.preserve_deleted_messages, action == "activate");
	}
}

#[test]
fn typed_event_handlers_preserve_old_inputs_and_match_host_events() {
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
		let expected_event = serde_json::to_value(input.message_event.as_ref().unwrap()).unwrap();
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
				transparent_all: Some(false),
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

#[test]
fn absent_effects_keep_defaults_and_new_flags_do_not_leak_into_old_outputs() {
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
