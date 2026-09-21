use extensions::*;
use serein_extension_sdk as sdk;

fn test_manifest(capabilities: Vec<Capability>) -> Manifest {
	Manifest {
		api_version: API_VERSION,
		id: "app-check".into(),
		name: "App contract".into(),
		version: "1.0.0".into(),
		author: "Serein".into(),
		license: "MIT".into(),
		source: "https://example.org/source".into(),
		kind: ExtensionKind::Plugin,
		capabilities,
		actions: vec![Action {
			id: "run".into(),
			label: "Run".into(),
			surface: Surface::Panel,
		}],
	}
}

fn snapshot() -> AppSnapshot {
	let user = UserSnapshot {
		id: "1".into(),
		name: "Synthetic".into(),
	};
	let channel = ChannelSnapshot {
		id: "2".into(),
		guild_id: Some("4".into()),
		name: "General".into(),
		kind: 0,
	};
	AppSnapshot {
		context: Some(AppContextSnapshot {
			connected: true,
			user: Some(user.clone()),
			channel: Some(channel.clone()),
		}),
		channels: Some(ChannelDirectorySnapshot {
			items: vec![channel],
			truncated: false,
		}),
		timeline: Some(TimelineSnapshot {
			channel_id: "2".into(),
			messages: vec![MessageSnapshot {
				id: "3".into(),
				author: user.clone(),
				content: "hello".into(),
				attachment_count: 1,
				edited: false,
			}],
			truncated: false,
		}),
		members: Some(MembersSnapshot {
			channel_id: "2".into(),
			items: vec![user],
			truncated: false,
		}),
		presence: Some(PresenceSnapshot {
			items: vec![PresenceEntry {
				user_id: "1".into(),
				status: "online".into(),
			}],
			truncated: false,
		}),
		voice: Some(VoiceSnapshot {
			channel_id: Some("5".into()),
			phase: "connected".into(),
			muted: true,
			deafened: false,
			camera: false,
			streaming: false,
			participants: vec!["1".into()],
		}),
		read_state: Some(ReadSnapshot {
			channel_id: Some("2".into()),
			unread: None,
			mentions: 2,
		}),
		settings: Some(LocalSettingsSnapshot {
			zoom_percent: 100,
			sidebar_width: 236,
			show_members: true,
			animate_gifs: false,
			hide_media_links: true,
		}),
	}
}

fn read_grants() -> Vec<Capability> {
	vec![
		Capability::AppContext,
		Capability::ChannelDirectory,
		Capability::Timeline,
		Capability::Members,
		Capability::Presence,
		Capability::VoiceState,
		Capability::ReadState,
		Capability::LocalSettings,
	]
}

#[test]
fn each_snapshot_group_requires_its_own_grant_and_typed_sdk_roundtrips() {
	let snapshot = snapshot();
	let manifest = test_manifest(read_grants());
	manifest.validate().unwrap();
	snapshot.validate(&manifest).unwrap();
	assert_eq!(
		snapshot.bytes().unwrap(),
		serde_json::to_vec(&snapshot).unwrap().len()
	);
	for denied in read_grants() {
		let mut missing = manifest.clone();
		missing
			.capabilities
			.retain(|capability| *capability != denied);
		assert!(matches!(
			snapshot.validate(&missing),
			Err(Error::Capability)
		));
	}
	let input = Invocation {
		action: "run".into(),
		app: Some(Box::new(snapshot)),
		..Default::default()
	};
	input.validate(&manifest).unwrap();
	let wire = serde_json::to_vec(&input).unwrap();
	let expected = serde_json::to_value(&input.app).unwrap();
	let result = sdk::dispatch_typed(&wire, |input: sdk::AppInvocation| {
		assert_eq!(input.invocation.action, "run");
		assert_eq!(serde_json::to_value(input.app).unwrap(), expected);
		sdk::AppOutput::default()
	})
	.unwrap();
	let output: Output = serde_json::from_slice(&result).unwrap();
	output.validate(&manifest, &input).unwrap();
	assert_eq!(sdk::MAX_APP_SNAPSHOT_BYTES, MAX_APP_SNAPSHOT_BYTES);
	assert_eq!(sdk::MAX_HOST_EFFECTS, MAX_HOST_EFFECTS);
	assert_eq!(sdk::MAX_CAPABILITIES, MAX_CAPABILITIES);
}

#[test]
fn legacy_literals_and_wire_outputs_remain_unchanged() {
	let invocation = sdk::Invocation {
		action: "run".into(),
		selected_message: None,
		composer: None,
		storage: None,
		values: Default::default(),
	};
	let event = sdk::EventInvocation {
		invocation: invocation.clone(),
		message_event: None,
	};
	let output = sdk::Output {
		image_sharing: false,
		preserve_deleted_messages: false,
		appearance: None,
		replacement: None,
		panel: Vec::new(),
		storage: None,
	};
	let app_input = sdk::AppInvocation {
		invocation: invocation.clone(),
		..Default::default()
	};
	let app_output = sdk::AppOutput {
		output: output.clone(),
		effects: Vec::new(),
	};
	assert_eq!(
		serde_json::to_value(&invocation).unwrap(),
		serde_json::to_value(event).unwrap()
	);
	assert_eq!(
		serde_json::to_value(invocation).unwrap(),
		serde_json::to_value(app_input).unwrap()
	);
	assert_eq!(
		serde_json::to_value(output).unwrap(),
		serde_json::to_value(app_output).unwrap()
	);
	let old: Invocation = serde_json::from_str(r#"{"action":"run"}"#).unwrap();
	assert!(old.app.is_none() && old.app_event.is_none());
	let wire = serde_json::to_value(old).unwrap();
	assert!(wire.get("app").is_none() && wire.get("app_event").is_none());
	assert!(
		serde_json::to_value(Output::default())
			.unwrap()
			.get("effects")
			.is_none()
	);
}

#[test]
fn local_effects_require_matching_grants_and_cannot_run_in_background() {
	for effect in [
		HostEffect::Navigate {
			channel_id: "2".into(),
		},
		HostEffect::Home,
		HostEffect::OpenView {
			view: AppView::VoiceSettings,
		},
		HostEffect::OpenProfile {
			user_id: "1".into(),
		},
		HostEffect::JumpToMessage {
			channel_id: "2".into(),
			message_id: "3".into(),
		},
		HostEffect::Search {
			query: "example".into(),
		},
		HostEffect::Notice {
			text: "Local notice".into(),
		},
		HostEffect::CopyText {
			text: "copied".into(),
		},
		HostEffect::SetVoice {
			muted: true,
			deafened: false,
		},
		HostEffect::LeaveVoice,
		HostEffect::SetLocalSettings {
			settings: LocalSettingsPatch {
				zoom_percent: Some(110),
				..Default::default()
			},
		},
	] {
		assert!(matches!(
			effect.validate(&test_manifest(vec![])),
			Err(Error::Capability)
		));
		let mut manifest = test_manifest(vec![
			effect.required_capability(),
			Capability::AppEvents,
			Capability::MessageEvents,
		]);
		let mut input = Invocation {
			action: "run".into(),
			..Default::default()
		};
		let output = Output {
			effects: vec![effect.clone()],
			..Default::default()
		};
		output.validate(&manifest, &input).unwrap();
		let sdk_output: sdk::AppOutput =
			serde_json::from_slice(&serde_json::to_vec(&output).unwrap()).unwrap();
		assert_eq!(
			serde_json::to_value(sdk_output.effects).unwrap(),
			serde_json::to_value(&output.effects).unwrap()
		);
		for surface in [
			Surface::Activation,
			Surface::AppEvent,
			Surface::MessageEvent,
		] {
			manifest.actions[0].surface = surface;
			assert!(matches!(
				output.validate(&manifest, &input),
				Err(Error::Capability)
			));
		}
		manifest.actions[0].surface = Surface::Composer;
		manifest.capabilities.push(Capability::Composer);
		input.composer = Some("draft".into());
		let mut conflicting = output;
		conflicting.replacement = Some("replacement".into());
		assert!(matches!(
			conflicting.validate(&manifest, &input),
			Err(Error::Capability)
		));
	}
}

#[test]
fn app_events_are_unique_granted_and_have_no_direct_conversation_context() {
	let mut manifest = test_manifest(vec![
		Capability::AppEvents,
		Capability::Storage,
		Capability::Appearance,
	]);
	manifest.actions[0].surface = Surface::AppEvent;
	manifest.validate().unwrap();
	let input = Invocation {
		action: "run".into(),
		app_event: Some(AppEventKind::Settings),
		..Default::default()
	};
	input.validate(&manifest).unwrap();
	Output {
		storage: Some("1".into()),
		appearance: Some(Theme::default()),
		..Default::default()
	}
	.validate(&manifest, &input)
	.unwrap();
	assert!(
		Output {
			panel: vec![Element::Separator],
			..Default::default()
		}
		.validate(&manifest, &input)
		.is_err()
	);
	for field in 0..4 {
		let mut invalid = input.clone();
		match field {
			0 => invalid.app_event = None,
			1 => invalid.composer = Some("draft".into()),
			2 => invalid.selected_message = Some("message".into()),
			_ => {
				invalid.values.insert("value".into(), "text".into());
			}
		}
		assert!(invalid.validate(&manifest).is_err());
	}
	manifest.actions.push(Action {
		id: "second".into(),
		label: "Second".into(),
		surface: Surface::AppEvent,
	});
	assert!(manifest.validate().is_err());
	manifest.actions.pop();
	manifest.actions[0].surface = Surface::Panel;
	assert!(matches!(input.validate(&manifest), Err(Error::Capability)));
	manifest.actions[0].surface = Surface::AppEvent;
	manifest
		.capabilities
		.retain(|capability| *capability != Capability::AppEvents);
	assert!(manifest.validate().is_err());
	assert!(input.validate(&manifest).is_err());
}

#[test]
fn snapshot_and_proposal_limits_include_escaped_wire_bytes() {
	let manifest = test_manifest(read_grants());
	let original = snapshot();
	let mut invalid = original.clone();
	invalid.channels.as_mut().unwrap().items[0].id = "18446744073709551616".into();
	assert!(invalid.validate(&manifest).is_err());
	invalid = original.clone();
	invalid.members.as_mut().unwrap().items[0].name = "x".repeat(257);
	assert!(matches!(invalid.validate(&manifest), Err(Error::Limit)));
	invalid = original.clone();
	let member = invalid.members.as_ref().unwrap().items[0].clone();
	invalid.members.as_mut().unwrap().items = (1..=MAX_APP_MEMBERS + 1)
		.map(|id| UserSnapshot {
			id: id.to_string(),
			..member.clone()
		})
		.collect();
	assert!(matches!(invalid.validate(&manifest), Err(Error::Limit)));
	invalid = original.clone();
	invalid.timeline.as_mut().unwrap().messages[0].content = "\0".repeat(MAX_EVENT_CONTENT_BYTES);
	assert!(matches!(invalid.validate(&manifest), Err(Error::Limit)));
	invalid = original;
	invalid.settings.as_mut().unwrap().zoom_percent = 151;
	assert!(invalid.validate(&manifest).is_err());
	let manifest = test_manifest(vec![Capability::ClipboardWrite, Capability::LocalSettings]);
	for effect in [
		HostEffect::CopyText {
			text: "\0".repeat(4096),
		},
		HostEffect::SetLocalSettings {
			settings: LocalSettingsPatch::default(),
		},
		HostEffect::SetLocalSettings {
			settings: LocalSettingsPatch {
				sidebar_width: Some(361),
				..Default::default()
			},
		},
	] {
		assert!(effect.validate(&manifest).is_err());
	}
	let input = Invocation {
		action: "run".into(),
		..Default::default()
	};
	assert!(matches!(
		Output {
			effects: vec![HostEffect::CopyText { text: "x".into() }; MAX_HOST_EFFECTS + 1],
			..Default::default()
		}
		.validate(&manifest, &input),
		Err(Error::Limit)
	));
}
