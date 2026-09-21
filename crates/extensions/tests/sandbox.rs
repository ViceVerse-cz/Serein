use extensions::*;

#[test]
fn section_opacity_roundtrips_and_stays_bounded() {
	let mut theme = Theme::default();
	theme.dark.background = Some(Background {
		opacity: 100,
		sections: Some(SectionOpacity::default()),
		..Default::default()
	});
	let decoded: Theme = serde_json::from_slice(&serde_json::to_vec(&theme).unwrap()).unwrap();
	assert_eq!(decoded, theme);
	assert!(theme.validate().is_ok());
	theme
		.dark
		.background
		.as_mut()
		.unwrap()
		.sections
		.as_mut()
		.unwrap()
		.member_list = 101;
	assert!(theme.validate().is_err());
}

fn plugin(wasm: &str) -> Package {
	Package {
		manifest: Manifest {
			api_version: API_VERSION,
			id: "test-plugin".into(),
			name: "Test plugin".into(),
			version: "1.0.0".into(),
			author: "Serein".into(),
			license: "MIT".into(),
			source: "https://github.com/example/plugin".into(),
			kind: ExtensionKind::Plugin,
			capabilities: vec![Capability::Composer],
			actions: vec![Action {
				id: "run".into(),
				label: "Run".into(),
				surface: Surface::Composer,
			}],
		},
		theme: None,
		background_image: Vec::new(),
		cover_image: Vec::new(),
		wasm: wat::parse_str(wasm).unwrap(),
	}
}

fn returning(json: &str) -> Package {
	let data: String = json.bytes().map(|b| format!("\\{b:02x}")).collect();
	plugin(&format!(
		r#"(module (memory (export "memory") 1 256)
		(data (i32.const 32768) "{data}")
		(func (export "serein_alloc") (param i32) (result i32) (i32.const 0))
		(func (export "serein_invoke") (param i32 i32) (result i64) (i64.const {})))"#,
		(32768_u64 << 32) | json.len() as u64
	))
}

#[test]
fn cover_bytes_are_theme_only_and_bounded() {
	let mut package = returning("{}");
	package.cover_image = vec![1];
	assert!(package.validate().is_err());
	package.manifest.kind = ExtensionKind::Theme;
	package.manifest.capabilities.clear();
	package.manifest.actions.clear();
	package.theme = Some(Theme::default());
	package.wasm.clear();
	assert!(package.validate().is_ok());
	package.cover_image.resize(MAX_BACKGROUND_BYTES + 1, 0);
	assert!(package.validate().is_err());
}

fn input() -> Invocation {
	Invocation {
		action: "run".into(),
		composer: Some("hello".into()),
		..Default::default()
	}
}

#[test]
fn returns_bounded_composer_proposal_and_releases_invocations() {
	let package = returning(r#"{"replacement":"HELLO","panel":[]}"#);
	let roundtrip = parse_package(&serde_json::to_vec(&package).unwrap()).unwrap();
	for _ in 0..20 {
		assert_eq!(
			invoke(&roundtrip, &input()).unwrap().replacement.as_deref(),
			Some("HELLO")
		);
	}
}

#[test]
fn rejects_imports_start_and_excessive_memory() {
	assert!(
		plugin(r#"(module (import "wasi_snapshot_preview1" "fd_write" (func)))"#)
			.validate()
			.is_err()
	);
	assert!(
		plugin(r#"(module (func $start) (start $start))"#)
			.validate()
			.is_err()
	);
	let package = plugin(
		r#"(module (memory (export "memory") 257) (func (export "serein_alloc") (param i32) (result i32) i32.const 0) (func (export "serein_invoke") (param i32 i32) (result i64) i64.const 0))"#,
	);
	assert!(invoke(&package, &input()).is_err());
}

#[test]
fn fuel_interrupts_infinite_loop_and_memory_growth_traps() {
	for body in [
		"(loop $spin (br $spin)) (i64.const 0)",
		"(drop (memory.grow (i32.const 256))) (i64.const 0)",
	] {
		let package = plugin(&format!(
			r#"(module (memory (export "memory") 1) (func (export "serein_alloc") (param i32) (result i32) i32.const 0) (func (export "serein_invoke") (param i32 i32) (result i64) {body}))"#
		));
		assert!(matches!(invoke(&package, &input()), Err(Error::Execution)));
	}
}

#[test]
fn rejects_oversized_or_invalid_pointers_and_json_responses() {
	for packed in [MAX_IO_BYTES as u64 + 1, (u32::MAX as u64) << 32 | 4] {
		let package = plugin(&format!(
			r#"(module (memory (export "memory") 1) (func (export "serein_alloc") (param i32) (result i32) i32.const 0) (func (export "serein_invoke") (param i32 i32) (result i64) i64.const {packed}))"#
		));
		assert!(invoke(&package, &input()).is_err());
	}
	assert!(invoke(&returning("not json"), &input()).is_err());
	assert!(invoke(&returning(r#"{"send":"not allowed"}"#), &input()).is_err());
}

#[test]
fn enforces_capabilities_and_restricts_context_to_action_surface() {
	assert!(matches!(
		invoke(&returning(r#"{"storage":"private"}"#), &input()),
		Err(Error::Capability)
	));
	let mut request = input();
	request.selected_message = Some("not granted".into());
	assert!(matches!(
		invoke(&returning("{}"), &request),
		Err(Error::Capability)
	));
	let mut package = returning(r#"{"replacement":"changed"}"#);
	package.manifest.actions[0].surface = Surface::Panel;
	request = Invocation {
		action: "run".into(),
		..Default::default()
	};
	assert!(matches!(invoke(&package, &request), Err(Error::Capability)));
}

#[test]
fn rejects_panel_complexity_duplicate_ids_and_unknown_actions() {
	let mut package = returning("{}");
	package.manifest.actions.push(Action {
		id: "panel".into(),
		label: "Panel".into(),
		surface: Surface::Panel,
	});
	for panel in [
		vec![Element::Text { text: "x".into() }; MAX_PANEL_ELEMENTS + 1],
		vec![Element::Button {
			id: "missing".into(),
			label: "Button".into(),
		}],
		vec![
			Element::Checkbox {
				id: "same".into(),
				label: "Same".into(),
				checked: false
			};
			2
		],
	] {
		assert!(
			Output {
				panel,
				..Default::default()
			}
			.validate(&package.manifest, &input())
			.is_err()
		);
	}
	let mut tree = vec![Element::Text {
		text: "nested".into(),
	}];
	for _ in 0..10 {
		tree = vec![Element::Row { children: tree }];
	}
	assert!(
		Output {
			panel: tree,
			..Default::default()
		}
		.validate(&package.manifest, &input())
		.is_err()
	);
}

#[test]
fn validates_themes_catalog_and_path_safe_identifiers() {
	for id in ["../test", "CON", "/tmp", "a/b", "", "test.plugin"] {
		assert!(!valid_id(id));
	}
	assert_eq!(parse_color("#11223380").unwrap(), [17, 34, 51, 128]);
	for color in ["red", "#xyzxyz", "#123", "#abcdef😀"] {
		assert!(parse_color(color).is_err());
	}
	let mut theme = Theme::default();
	theme.dark.colors.insert("chat".into(), "#101010".into());
	assert!(theme.validate().is_ok());
	theme.dark.colors.insert("image".into(), "#101010".into());
	assert!(theme.validate().is_err());
	assert!(parse_package(&vec![b' '; MAX_PACKAGE_BYTES + 1]).is_err());
	assert!(parse_catalog(br#"{"api_version":2,"entries":[]}"#).is_err());
	let entry = CatalogEntry {
		description: String::new(),
		preview: None,
		manifest: returning("{}").manifest,
		release_url: "https://example.com/p.json".into(),
		sha256: "a".repeat(64),
		download_bytes: 100,
		source_commit: "a".repeat(40),
	};
	let catalog = Catalog {
		api_version: API_VERSION,
		entries: vec![entry.clone(), entry],
	};
	assert!(parse_catalog(&serde_json::to_vec(&catalog).unwrap()).is_err());
}

#[test]
fn shipped_rust_examples_execute_through_the_real_abi() {
	let protector = parse_package(include_bytes!(
		"../../../examples/extensions/packages/message-delete-protector.serein-extension"
	))
	.unwrap();
	let input = Invocation {
		action: "activate".into(),
		..Default::default()
	};
	assert!(
		invoke(&protector, &input)
			.unwrap()
			.preserve_deleted_messages
	);
}

#[test]
fn catalog_preview_metadata_is_optional_and_bounded() {
	let mut catalog: serde_json::Value =
		serde_json::from_slice(include_bytes!("../../../extensions/catalog.json")).unwrap();
	for entry in catalog["entries"].as_array_mut().unwrap() {
		entry.as_object_mut().unwrap().remove("description");
		entry.as_object_mut().unwrap().remove("preview");
	}
	let parsed = parse_catalog(&serde_json::to_vec(&catalog).unwrap()).unwrap();
	assert!(
		parsed
			.entries
			.iter()
			.all(|entry| entry.preview.is_none() && entry.description.is_empty())
	);
	catalog["entries"][0]["description"] = serde_json::json!("A preview of the creator's theme.");
	catalog["entries"][0]["preview"] = serde_json::json!({"url": "https://example.org/theme.png", "sha256": "a".repeat(64), "download_bytes": 100});
	assert!(parse_catalog(&serde_json::to_vec(&catalog).unwrap()).is_ok());
	for value in [
		serde_json::json!("http://example.org/preview.png"),
		serde_json::json!("https://name:secret@example.org/preview.png"),
	] {
		let mut invalid = catalog.clone();
		invalid["entries"][0]["preview"]["url"] = value;
		assert!(parse_catalog(&serde_json::to_vec(&invalid).unwrap()).is_err());
	}
	for (field, value) in [
		("sha256", serde_json::json!("invalid")),
		("download_bytes", serde_json::json!(0)),
		(
			"download_bytes",
			serde_json::json!(extensions::MAX_PREVIEW_BYTES + 1),
		),
	] {
		let mut invalid = catalog.clone();
		invalid["entries"][0]["preview"][field] = value;
		assert!(parse_catalog(&serde_json::to_vec(&invalid).unwrap()).is_err());
	}
	for description in ["x".repeat(257), "\u{1F980}".repeat(257), "bad\ntext".into()] {
		let mut invalid = catalog.clone();
		invalid["entries"][0]["description"] = serde_json::json!(description);
		assert!(parse_catalog(&serde_json::to_vec(&invalid).unwrap()).is_err());
	}
}

#[test]
fn image_sharing_plugin_requires_activation_and_capability() {
	let mut package = parse_package(include_bytes!(
		"../../../examples/extensions/packages/emoji-sticker-images.serein-extension"
	))
	.unwrap();
	let input = Invocation {
		action: "activate".into(),
		..Default::default()
	};
	let output = invoke(&package, &input).unwrap();
	assert!(output.image_sharing);
	assert!(output.replacement.is_none() && output.panel.is_empty());
	package.manifest.capabilities.clear();
	assert!(output.validate(&package.manifest, &input).is_err());
	package.manifest.capabilities.push(Capability::ImageSharing);
	package.manifest.actions[0].surface = Surface::Panel;
	assert!(output.validate(&package.manifest, &input).is_err());
	assert!(!serde_json::from_str::<Output>("{}").unwrap().image_sharing);
}

fn message_event() -> MessageEvent {
	MessageEvent {
		kind: MessageEventKind::Create,
		channel_id: "1".into(),
		message_id: "2".into(),
		author_id: Some("3".into()),
		content: Some("hello".into()),
	}
}

fn event_package(response: &str) -> Package {
	let mut package = returning(response);
	package.manifest.capabilities = vec![Capability::MessageEvents];
	package.manifest.actions[0].surface = Surface::MessageEvent;
	package
}

#[test]
fn message_event_payloads_bound_ids_content_and_partial_fields() {
	let event = message_event();
	event.validate().unwrap();
	for invalid in [
		"",
		"0",
		"000",
		"+1",
		"-1",
		" 1",
		"1.0",
		"18446744073709551616",
		"000000000000000000001",
	] {
		for field in 0..3 {
			let mut invalid_event = event.clone();
			match field {
				0 => invalid_event.channel_id = invalid.into(),
				1 => invalid_event.message_id = invalid.into(),
				_ => invalid_event.author_id = Some(invalid.into()),
			}
			assert!(matches!(invalid_event.validate(), Err(Error::Invalid)));
		}
	}
	let mut event = event;
	event.message_id = u64::MAX.to_string();
	event.content = Some("\u{1F980}".repeat(MAX_EVENT_CONTENT_BYTES / 4));
	event.validate().unwrap();
	event.content.as_mut().unwrap().push('x');
	assert!(matches!(event.validate(), Err(Error::Limit)));
	event.content = None;
	assert!(matches!(event.validate(), Err(Error::Invalid)));
	event.kind = MessageEventKind::Update;
	event.validate().unwrap();
	event.author_id = None;
	event.validate().unwrap();
	event.kind = MessageEventKind::Delete;
	event.validate().unwrap();
	for (author_id, content) in [(Some("3".into()), None), (None, Some(String::new()))] {
		event.author_id = author_id;
		event.content = content;
		assert!(matches!(event.validate(), Err(Error::Invalid)));
	}
}

#[test]
fn message_events_require_a_unique_granted_surface_without_other_context() {
	let mut package = event_package("{}");
	let input = Invocation {
		action: "run".into(),
		message_event: Some(Box::new(message_event())),
		..Default::default()
	};
	package.validate().unwrap();
	invoke(&package, &input).unwrap();
	package.manifest.capabilities.clear();
	assert!(matches!(
		package.manifest.validate(),
		Err(Error::Capability)
	));
	assert!(matches!(
		input.validate(&package.manifest),
		Err(Error::Capability)
	));
	package.manifest.capabilities = vec![
		Capability::MessageEvents,
		Capability::SelectedMessage,
		Capability::Composer,
	];
	package.manifest.actions.push(Action {
		id: "second".into(),
		label: "Second".into(),
		surface: Surface::MessageEvent,
	});
	assert!(matches!(package.manifest.validate(), Err(Error::Invalid)));
	package.manifest.actions.pop();
	for surface in [
		Surface::Panel,
		Surface::Activation,
		Surface::Message,
		Surface::Composer,
	] {
		package.manifest.actions[0].surface = surface;
		assert!(matches!(invoke(&package, &input), Err(Error::Capability)));
	}
	package.manifest.actions[0].surface = Surface::MessageEvent;
	for field in 0..4 {
		let mut invalid = input.clone();
		match field {
			0 => invalid.message_event = None,
			1 => invalid.selected_message = Some("private".into()),
			2 => invalid.composer = Some("draft".into()),
			_ => {
				invalid.values.insert("field".into(), "value".into());
			}
		}
		assert!(invoke(&package, &invalid).is_err());
	}
}

#[test]
fn event_effects_allow_granted_storage_and_appearance_without_unsolicited_ui() {
	let input = Invocation {
		action: "run".into(),
		message_event: Some(Box::new(message_event())),
		..Default::default()
	};
	for response in [r#"{"storage":"1"}"#, r#"{"appearance":{}}"#] {
		let mut package = event_package(response);
		assert!(matches!(invoke(&package, &input), Err(Error::Capability)));
		package
			.manifest
			.capabilities
			.extend([Capability::Storage, Capability::Appearance]);
		invoke(&package, &input).unwrap();
	}
	for response in [
		r#"{"panel":[{"type":"text","text":"unsolicited"}]}"#,
		r#"{"replacement":"unsolicited"}"#,
		r#"{"image_sharing":true}"#,
		r#"{"preserve_deleted_messages":true}"#,
	] {
		let mut package = event_package(response);
		package.manifest.capabilities.extend([
			Capability::Composer,
			Capability::ImageSharing,
			Capability::DeletedMessages,
		]);
		assert!(matches!(invoke(&package, &input), Err(Error::Capability)));
	}
}
