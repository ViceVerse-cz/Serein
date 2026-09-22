use extensions::*;
use serein_extension_sdk as sdk;

#[test]
fn sdk_manifests_round_trip_all_capabilities_and_surfaces_through_host_validation() {
	let mut manifest = test_manifest(vec![
		Capability::RelationshipControl,
		Capability::AccountControl,
		Capability::AudioSettings,
		Capability::VoiceConnect,
		Capability::CameraControl,
		Capability::MessageSend,
		Capability::MessageManage,
		Capability::ReactionsControl,
		Capability::ReadStateControl,
		Capability::ThreadsControl,
		Capability::ChannelControl,
		Capability::ServerControl,
		Capability::RoleControl,
		Capability::ModerationControl,
		Capability::MediaControl,
		Capability::MessageContent,
		Capability::ForumData,
		Capability::ConversationActivity,
		Capability::ChannelMetadata,
		Capability::MemberDetails,
		Capability::SelectedMessage,
		Capability::Composer,
		Capability::Storage,
		Capability::DeletedMessages,
		Capability::ImageSharing,
		Capability::Appearance,
		Capability::MessageEvents,
		Capability::AppContext,
		Capability::ChannelDirectory,
		Capability::Timeline,
		Capability::Members,
		Capability::Presence,
		Capability::VoiceState,
		Capability::ReadState,
		Capability::LocalSettings,
		Capability::NotificationSettings,
		Capability::Navigation,
		Capability::LocalNotices,
		Capability::ClipboardWrite,
		Capability::VoiceControl,
		Capability::AppEvents,
		Capability::AccountProfile,
		Capability::GuildDirectory,
		Capability::ChannelDetails,
		Capability::DataEvents,
		Capability::MessageDetails,
		Capability::Relationships,
	]);
	manifest.actions = [
		Surface::Message,
		Surface::Composer,
		Surface::Panel,
		Surface::Activation,
		Surface::MessageEvent,
		Surface::AppEvent,
	]
	.into_iter()
	.enumerate()
	.map(|(index, surface)| Action {
		id: format!("action-{index}"),
		label: "Example".into(),
		surface,
	})
	.collect();
	manifest.validate().unwrap();
	let wire = serde_json::to_value(&manifest).unwrap();
	let authored: sdk::Manifest = serde_json::from_value(wire.clone()).unwrap();
	assert_eq!(serde_json::to_value(&authored).unwrap(), wire);
	let imported: Manifest =
		serde_json::from_value(serde_json::to_value(authored).unwrap()).unwrap();
	imported.validate().unwrap();
	assert_eq!(imported, manifest);
	for invalid in [
		serde_json::json!({"unexpected": true}),
		serde_json::json!({"capabilities": ["typo"]}),
		serde_json::json!({"kind": "unknown"}),
		serde_json::json!({"actions": [{"id":"run", "label":"Run", "surface":"typo"}]}),
	] {
		let mut value = wire.clone();
		value
			.as_object_mut()
			.unwrap()
			.extend(invalid.as_object().unwrap().clone());
		assert!(serde_json::from_value::<sdk::Manifest>(value).is_err());
	}
	let mut theme = wire;
	theme["kind"] = serde_json::json!("theme");
	theme.as_object_mut().unwrap().remove("capabilities");
	theme.as_object_mut().unwrap().remove("actions");
	let theme: sdk::Manifest = serde_json::from_value(theme).unwrap();
	assert!(theme.capabilities.is_empty() && theme.actions.is_empty());
	let imported: Manifest = serde_json::from_value(serde_json::to_value(theme).unwrap()).unwrap();
	imported.validate().unwrap();
}

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
		message_content: Some(MessageContentSnapshot {
			channel_id: "2".into(),
			items: vec![],
			truncated: false,
		}),
		forum_data: Some(ForumDataSnapshot {
			channel_id: "2".into(),
			guild_id: "4".into(),
			parent_id: "2".into(),
			posts: vec![],
			truncated: false,
		}),
		conversation_activity: Some(ConversationActivitySnapshot {
			channel_id: "2".into(),
			typing_user_ids: vec!["1".into()],
			pinned_message_ids: None,
			pins_truncated: false,
		}),
		channel_metadata: Some(ChannelMetadataSnapshot {
			channel_id: "2".into(),
			guild_id: "4".into(),
			parent: None,
			category: None,
			topic: Some("Loaded topic".into()),
			topic_truncated: false,
			slowmode_seconds: Some(5),
			nsfw: Some(false),
			thread: None,
			permissions: [
				(ChannelPermission::ViewChannel, Some(true)),
				(ChannelPermission::ManageRoles, None),
			]
			.into(),
		}),
		member_details: Some(MemberDetailsSnapshot {
			channel_id: "2".into(),
			guild_id: "4".into(),
			items: vec![],
			truncated: false,
			roles: None,
			roles_truncated: false,
		}),
		message_details: Some(MessageDetailsSnapshot {
			channel_id: "2".into(),
			truncated: false,
			items: vec![MessageDetailSnapshot {
				id: "3".into(),
				kind: 19,
				reply_to: Some("5".into()),
				mention_ids: vec!["1".into()],
				mentions_truncated: false,
				mention_everyone: false,
				attachments: vec![AttachmentSnapshot {
					id: "6".into(),
					filename: "file.txt".into(),
					size: 42,
					content_type: Some("text/plain".into()),
					spoiler: false,
				}],
				attachments_truncated: false,
				reactions: Some(vec![ReactionSnapshot {
					emoji_id: None,
					emoji_name: Some("??".into()),
					count: 1,
					me: true,
					me_burst: false,
				}]),
				reactions_truncated: false,
			}],
		}),
		relationships: Some(RelationshipsSnapshot {
			items: vec![RelationshipSnapshot {
				user: user.clone(),
				kind: RelationshipKind::Friend,
			}],
			truncated: false,
			friends_known: true,
			requests_known: false,
			restricted_known: true,
		}),
		account_profile: Some(AccountProfileSnapshot {
			user: user.clone(),
			avatar: Some("a_abc123".into()),
			profile: Some(OwnProfileSnapshot {
				display_name: Some("Synthetic".into()),
				bio: "First line\nSecond line".into(),
				pronouns: "they/them".into(),
			}),
		}),
		guilds: Some(GuildDirectorySnapshot {
			items: vec![GuildSnapshot {
				id: "4".into(),
				name: "Synthetic guild".into(),
				icon: Some("abc123".into()),
			}],
			truncated: false,
		}),
		channel_details: Some(ChannelDetailsSnapshot {
			channel: channel.clone(),
			parent_id: Some("6".into()),
			position: 0,
			last_message_id: Some("3".into()),
			message_count: Some(1),
			recipients: vec![user.clone()],
			recipients_truncated: false,
			can_send: true,
			can_read_history: true,
		}),
		context: Some(AppContextSnapshot {
			connected: true,
			user: Some(user.clone()),
			channel: Some(channel.clone()),
		}),
		channels: Some(ChannelDirectorySnapshot {
			items: vec![channel.clone()],
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
			items: vec![user.clone()],
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
			smooth_scrolling: Some(true),
			scroll_speed_percent: Some(125),
		}),
		audio_settings: Some(AudioSettingsSnapshot {
			input_percent: 100,
			output_percent: 80,
			push_to_talk: true,
			input_profile: "custom".into(),
			suppression: "rnnoise".into(),
			suppression_level: 2,
			echo_cancellation: true,
			automatic_gain: false,
			sensitivity_db: Some(-50),
		}),
		own_presence: Some(OwnPresenceSnapshot {
			status: "idle".into(),
			custom_status: "Synthetic status".into(),
			expires_at_ms: Some(123456),
			share_game_activity: false,
		}),
		notification_settings: Some(NotificationSettingsSnapshot {
			new_message: true,
			current_channel: false,
			incoming_ring: true,
			outgoing_ring: false,
			disable_sounds: true,
			unread_badge: false,
			mute: true,
			unmute: false,
			deafen: true,
			undeafen: false,
			camera_on: true,
			screen_share_on: false,
			user_join: true,
			user_leave: false,
			volume: 75,
		}),
	}
}

fn read_grants() -> Vec<Capability> {
	vec![
		Capability::AudioSettings,
		Capability::AccountControl,
		Capability::MessageContent,
		Capability::ForumData,
		Capability::ConversationActivity,
		Capability::ChannelMetadata,
		Capability::MemberDetails,
		Capability::MessageDetails,
		Capability::Relationships,
		Capability::AccountProfile,
		Capability::GuildDirectory,
		Capability::ChannelDetails,
		Capability::AppContext,
		Capability::ChannelDirectory,
		Capability::Timeline,
		Capability::Members,
		Capability::Presence,
		Capability::VoiceState,
		Capability::ReadState,
		Capability::LocalSettings,
		Capability::NotificationSettings,
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
		HostEffect::SetNotificationSettings {
			settings: NotificationSettingsPatch {
				disable_sounds: Some(true),
				..Default::default()
			},
		},
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

#[test]
fn data_events_require_opt_in_and_the_matching_data_grant() {
	for (kind, grant) in [
		(AppEventKind::Reactions, Capability::ConversationActivity),
		(AppEventKind::Pins, Capability::ConversationActivity),
		(AppEventKind::Typing, Capability::ConversationActivity),
		(AppEventKind::Polls, Capability::MessageContent),
		(AppEventKind::Threads, Capability::ChannelMetadata),
		(AppEventKind::Roles, Capability::MemberDetails),
		(AppEventKind::Permissions, Capability::ChannelMetadata),
		(AppEventKind::Recovered, Capability::MemberDetails),
		(AppEventKind::MessageDetails, Capability::MessageDetails),
		(AppEventKind::Relationships, Capability::Relationships),
		(AppEventKind::Account, Capability::AccountProfile),
		(AppEventKind::Channels, Capability::ChannelDirectory),
		(AppEventKind::Channels, Capability::GuildDirectory),
		(AppEventKind::Channels, Capability::ChannelDetails),
		(AppEventKind::Members, Capability::Members),
		(AppEventKind::Presence, Capability::Presence),
		(AppEventKind::ReadState, Capability::ReadState),
	] {
		let mut manifest =
			test_manifest(vec![Capability::AppEvents, Capability::DataEvents, grant]);
		manifest.actions[0].surface = Surface::AppEvent;
		manifest.validate().unwrap();
		let input = Invocation {
			action: "run".into(),
			app_event: Some(kind),
			..Default::default()
		};
		input.validate(&manifest).unwrap();
		let sdk: sdk::AppInvocation =
			serde_json::from_value(serde_json::to_value(&input).unwrap()).unwrap();
		assert_eq!(
			serde_json::to_value(sdk.app_event).unwrap(),
			serde_json::to_value(kind).unwrap()
		);
		for denied in [Capability::AppEvents, Capability::DataEvents, grant] {
			let mut missing = manifest.clone();
			missing
				.capabilities
				.retain(|capability| *capability != denied);
			assert!(matches!(input.validate(&missing), Err(Error::Capability)));
		}
	}
	assert!(matches!(
		test_manifest(vec![Capability::DataEvents]).validate(),
		Err(Error::Capability)
	));
}

#[test]
fn expanded_snapshots_bound_profile_hashes_and_collections() {
	let manifest = test_manifest(read_grants());
	let mut value = snapshot();
	value
		.account_profile
		.as_mut()
		.unwrap()
		.profile
		.as_mut()
		.unwrap()
		.bio = "x".repeat(2049);
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	value
		.account_profile
		.as_mut()
		.unwrap()
		.profile
		.as_mut()
		.unwrap()
		.bio = "hidden\0text".into();
	assert!(matches!(value.validate(&manifest), Err(Error::Invalid)));
	let mut value = snapshot();
	value.account_profile.as_mut().unwrap().avatar = Some("../secret".into());
	assert!(matches!(value.validate(&manifest), Err(Error::Invalid)));
	let mut value = snapshot();
	value.guilds.as_mut().unwrap().items = (1..=MAX_APP_GUILDS + 1)
		.map(|id| GuildSnapshot {
			id: id.to_string(),
			name: "Guild".into(),
			icon: None,
		})
		.collect();
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	let mut value = snapshot();
	value.channel_details.as_mut().unwrap().recipients = (1..=MAX_CHANNEL_RECIPIENTS + 1)
		.map(|id| UserSnapshot {
			id: id.to_string(),
			name: "User".into(),
		})
		.collect();
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	value.channel_details.as_mut().unwrap().recipients.pop();
	value.validate(&manifest).unwrap();
	let mut value = snapshot();
	let profile = value
		.account_profile
		.as_mut()
		.unwrap()
		.profile
		.as_mut()
		.unwrap();
	profile.display_name = Some("x".repeat(256));
	profile.bio = "x".repeat(2048);
	profile.pronouns = "x".repeat(256);
	value.validate(&manifest).unwrap();
	value
		.account_profile
		.as_mut()
		.unwrap()
		.profile
		.as_mut()
		.unwrap()
		.pronouns
		.push('x');
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	let mut value = snapshot();
	value.channel_details.as_mut().unwrap().parent_id = Some("0".into());
	assert!(matches!(value.validate(&manifest), Err(Error::Invalid)));
	assert_eq!(sdk::MAX_APP_GUILDS, MAX_APP_GUILDS);
	assert_eq!(sdk::MAX_CHANNEL_RECIPIENTS, MAX_CHANNEL_RECIPIENTS);
}

#[test]
fn message_details_and_relationships_bound_nested_data() {
	let manifest = test_manifest(read_grants());
	let original = snapshot();
	let mut value = original.clone();
	let detail = value.message_details.as_mut().unwrap();
	let item = detail.items[0].clone();
	detail.items = (1..=MAX_MESSAGE_DETAILS + 1)
		.map(|id| MessageDetailSnapshot {
			id: id.to_string(),
			..item.clone()
		})
		.collect();
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	value.message_details.as_mut().unwrap().items.pop();
	value.validate(&manifest).unwrap();
	let mut value = original.clone();
	value.message_details.as_mut().unwrap().items[0].mention_ids = (1..=MAX_MESSAGE_MENTIONS + 1)
		.map(|id| id.to_string())
		.collect();
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	let mut value = original.clone();
	let detail = &mut value.message_details.as_mut().unwrap().items[0];
	let attachment = detail.attachments[0].clone();
	detail.attachments = (1..=MAX_MESSAGE_ATTACHMENTS + 1)
		.map(|id| AttachmentSnapshot {
			id: id.to_string(),
			..attachment.clone()
		})
		.collect();
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	for (filename, content_type) in [
		("x".repeat(257), None),
		("ok".into(), Some("x".repeat(129))),
	] {
		let mut value = original.clone();
		let attachment = &mut value.message_details.as_mut().unwrap().items[0].attachments[0];
		attachment.filename = filename;
		attachment.content_type = content_type;
		assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	}
	let mut value = original.clone();
	let reactions = value.message_details.as_mut().unwrap().items[0]
		.reactions
		.as_mut()
		.unwrap();
	reactions.resize(MAX_MESSAGE_REACTIONS + 1, reactions[0].clone());
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	for name in [None, Some("bad\nname".into())] {
		let mut value = original.clone();
		value.message_details.as_mut().unwrap().items[0]
			.reactions
			.as_mut()
			.unwrap()[0]
			.emoji_name = name;
		assert!(matches!(value.validate(&manifest), Err(Error::Invalid)));
	}
	let mut value = original;
	value.relationships.as_mut().unwrap().items = (1..=MAX_RELATIONSHIPS + 1)
		.map(|id| RelationshipSnapshot {
			user: UserSnapshot {
				id: id.to_string(),
				name: "User".into(),
			},
			kind: RelationshipKind::Ignored,
		})
		.collect();
	assert!(matches!(value.validate(&manifest), Err(Error::Limit)));
	value.relationships.as_mut().unwrap().items.pop();
	value.validate(&manifest).unwrap();
	for kind in [
		RelationshipKind::Friend,
		RelationshipKind::IncomingRequest,
		RelationshipKind::OutgoingRequest,
		RelationshipKind::Blocked,
		RelationshipKind::Ignored,
	] {
		let wire = serde_json::to_value(kind).unwrap();
		let sdk: sdk::RelationshipKind = serde_json::from_value(wire.clone()).unwrap();
		assert_eq!(serde_json::to_value(sdk).unwrap(), wire);
	}
	assert_eq!(sdk::MAX_MESSAGE_DETAILS, MAX_MESSAGE_DETAILS);
	assert_eq!(sdk::MAX_MESSAGE_MENTIONS, MAX_MESSAGE_MENTIONS);
	assert_eq!(sdk::MAX_MESSAGE_ATTACHMENTS, MAX_MESSAGE_ATTACHMENTS);
	assert_eq!(sdk::MAX_MESSAGE_REACTIONS, MAX_MESSAGE_REACTIONS);
	assert_eq!(sdk::MAX_RELATIONSHIPS, MAX_RELATIONSHIPS);
}

#[test]
fn discovery_is_forward_tolerant_and_does_not_change_legacy_input() {
	let wire = serde_json::to_value(HostInfo::current()).unwrap();
	let host: sdk::HostInfo = serde_json::from_value(wire.clone()).unwrap();
	assert_eq!(host.api_version, API_VERSION);
	assert!(host.supports("message_content") && host.supports_event("typing"));
	assert!(!host.supports("forum_tags"));
	let mut future = wire;
	future["capabilities"]
		.as_array_mut()
		.unwrap()
		.push(serde_json::json!("future_capability"));
	let host: sdk::HostInfo = serde_json::from_value(future).unwrap();
	assert!(host.supports("future_capability"));
	let old: sdk::AppInvocation =
		serde_json::from_value(serde_json::json!({"action":"run"})).unwrap();
	assert!(old.host.is_none());
	let caps = HostInfo::current().capabilities.to_vec();
	test_manifest(caps.clone()).validate().unwrap();
	assert_eq!(caps.len(), 47);
	assert_eq!(HostInfo::current().app_events.len(), 21);
	assert_eq!(std::collections::BTreeSet::from_iter(caps).len(), 47);
}

#[test]
fn notification_preferences_validate_every_patch_field_and_volume_boundaries() {
	let manifest = test_manifest(vec![Capability::NotificationSettings]);
	let fields = [
		"new_message",
		"current_channel",
		"incoming_ring",
		"outgoing_ring",
		"disable_sounds",
		"unread_badge",
		"mute",
		"unmute",
		"deafen",
		"undeafen",
		"camera_on",
		"screen_share_on",
		"user_join",
		"user_leave",
	];
	for field in fields {
		for value in [false, true] {
			let wire = serde_json::json!({field: value});
			let authored: sdk::NotificationSettingsPatch =
				serde_json::from_value(wire.clone()).unwrap();
			assert_eq!(serde_json::to_value(&authored).unwrap(), wire);
			let patch: NotificationSettingsPatch =
				serde_json::from_value(serde_json::to_value(authored).unwrap()).unwrap();
			HostEffect::SetNotificationSettings { settings: patch }
				.validate(&manifest)
				.unwrap();
		}
	}
	let mut nulls = serde_json::Map::new();
	for field in fields.into_iter().chain(["volume"]) {
		nulls.insert(field.into(), serde_json::Value::Null);
	}
	for wire in [serde_json::json!({}), serde_json::Value::Object(nulls)] {
		let settings: NotificationSettingsPatch = serde_json::from_value(wire).unwrap();
		assert!(matches!(
			HostEffect::SetNotificationSettings { settings }.validate(&manifest),
			Err(Error::Invalid)
		));
	}
	for volume in [0, 100, 101, 255] {
		let patch = NotificationSettingsPatch {
			volume: Some(volume),
			..Default::default()
		};
		assert_eq!(patch.validate().is_ok(), volume <= 100);
		let mut data = snapshot();
		data.notification_settings.as_mut().unwrap().volume = volume;
		assert_eq!(
			data.validate(&test_manifest(read_grants())).is_ok(),
			volume <= 100
		);
	}
	assert!(
		serde_json::from_value::<NotificationSettingsPatch>(serde_json::json!({"volume": 256}))
			.is_err()
	);
	assert!(
		serde_json::from_value::<NotificationSettingsPatch>(serde_json::json!({"unknown": true}))
			.is_err()
	);
}

#[test]
fn scrolling_settings_are_optional_on_older_hosts_and_validate_speed_bounds() {
	let wire = serde_json::json!({
		"zoom_percent": 100, "sidebar_width": 236, "show_members": true,
		"animate_gifs": false, "hide_media_links": true
	});
	let old: sdk::LocalSettingsSnapshot = serde_json::from_value(wire.clone()).unwrap();
	assert_eq!(old.smooth_scrolling, None);
	assert_eq!(old.scroll_speed_percent, None);
	assert_eq!(serde_json::to_value(old).unwrap(), wire);
	let mut host: LocalSettingsSnapshot = serde_json::from_value(wire).unwrap();
	host.validate().unwrap();
	for speed in [0, 24, 25, 300, 301, u16::MAX] {
		let patch = LocalSettingsPatch {
			scroll_speed_percent: Some(speed),
			..Default::default()
		};
		host.scroll_speed_percent = Some(speed);
		assert_eq!(patch.validate().is_ok(), (25..=300).contains(&speed));
		assert_eq!(host.validate().is_ok(), (25..=300).contains(&speed));
	}
	for enabled in [false, true] {
		let authored = sdk::LocalSettingsPatch {
			smooth_scrolling: Some(enabled),
			..Default::default()
		};
		let patch: LocalSettingsPatch =
			serde_json::from_value(serde_json::to_value(authored).unwrap()).unwrap();
		patch.validate().unwrap();
		assert_eq!(patch.smooth_scrolling, Some(enabled));
		assert_eq!(patch.scroll_speed_percent, None);
	}
	let old_app: sdk::AppSnapshot = serde_json::from_str("{}").unwrap();
	assert!(old_app.notification_settings.is_none());
}

#[test]
fn app_actions_round_trip_and_require_foreground_granted_confirmation() {
	let actions = [
		r#"{"type":"send_message","channel_id":"2","content":"hello"}"#,
		r#"{"type":"send_reply","channel_id":"2","message_id":"3","content":"hello","mention":true}"#,
		r#"{"type":"send_sticker","channel_id":"2","sticker_id":"5"}"#,
		r#"{"type":"forward_message","channel_id":"2","message_id":"3","target_channel_ids":["6","7"],"note":"FYI"}"#,
		r#"{"type":"edit_message","channel_id":"2","message_id":"3","content":"edited"}"#,
		r#"{"type":"delete_message","channel_id":"2","message_id":"3"}"#,
		r#"{"type":"set_reaction","channel_id":"2","message_id":"3","emoji":"wave:4","add":true}"#,
		r#"{"type":"set_message_pinned","channel_id":"2","message_id":"3","pinned":true}"#,
		r#"{"type":"mark_read","channel_id":"2","message_id":"3"}"#,
		r#"{"type":"mark_channel_read","channel_id":"2"}"#,
		r#"{"type":"mark_unread","channel_id":"2","message_id":"3"}"#,
		r#"{"type":"mark_guild_read","guild_id":"1"}"#,
		r#"{"type":"jump_to_unread"}"#,
		r#"{"type":"create_thread","channel_id":"2","name":"Thread","message_id":"3"}"#,
		r#"{"type":"create_forum_post","parent_id":"2","title":"Post","content":"hello"}"#,
		r#"{"type":"set_thread_archived","channel_id":"2","archived":true}"#,
		r#"{"type":"set_thread_locked","channel_id":"2","locked":false}"#,
		r#"{"type":"set_thread_followed","channel_id":"2","followed":true}"#,
		r#"{"type":"set_thread_pinned","channel_id":"2","pinned":false}"#,
		r#"{"type":"rename_thread","channel_id":"2","name":"Renamed"}"#,
		r#"{"type":"set_channel_mute","channel_id":"2","duration_seconds":3600}"#,
		r#"{"type":"set_channel_notifications","channel_id":"2","level":2}"#,
		r#"{"type":"set_guild_hide_muted","guild_id":"4","hide":true}"#,
		r#"{"type":"create_channel","guild_id":"4","name":"general","kind":"text"}"#,
		r#"{"type":"create_category","guild_id":"4","name":"Topics"}"#,
		r#"{"type":"duplicate_channel","channel_id":"2","name":"general-copy"}"#,
		r#"{"type":"edit_channel","channel_id":"2","before":{"name":"general","topic":"Old","slowmode":0,"nsfw":false,"overwrites":[]},"after":{"name":"general-chat","topic":"New","slowmode":5,"nsfw":false,"overwrites":[{"id":"4","kind":0,"allow":"1024","deny":"0"}]}}"#,
		r#"{"type":"delete_channel","channel_id":"2"}"#,
		r#"{"type":"move_channel","channel_id":"2","parent_id":"8","position":1,"lock_permissions":true,"shifts":[{"channel_id":"9","position":2}]}"#,
		r#"{"type":"create_server_invite","guild_id":"4","channel_id":"2","max_age":3600,"max_uses":10,"temporary":false}"#,
		r#"{"type":"leave_server","guild_id":"4"}"#,
		r#"{"type":"leave_group","channel_id":"2"}"#,
		r#"{"type":"rename_group","channel_id":"2","name":"Friends"}"#,
		r#"{"type":"close_dm","channel_id":"2"}"#,
		r#"{"type":"set_conversation_muted","channel_id":"2","muted":true}"#,
		r#"{"type":"open_friend_dm","user_id":"4"}"#,
		r#"{"type":"set_friend_nickname","user_id":"4","text":"Friend"}"#,
		r#"{"type":"set_user_note","user_id":"4","text":"Note"}"#,
		r#"{"type":"add_friend","username":"a_friend"}"#,
		r#"{"type":"remove_friend","user_id":"4"}"#,
		r#"{"type":"resolve_friend_request","user_id":"4","accept":true}"#,
		r#"{"type":"set_user_blocked","user_id":"4","blocked":true}"#,
		r#"{"type":"set_own_profile","profile":{"bio":"Hello"}}"#,
		r#"{"type":"set_own_presence","presence":{"status":"idle"}}"#,
		r#"{"type":"set_activity_sharing","enabled":false}"#,
		r#"{"type":"set_audio_settings","settings":{"input_percent":100}}"#,
		r#"{"type":"set_participant_audio","user_id":"4","volume_percent":0,"muted":true}"#,
		r#"{"type":"set_stream_audio","volume_percent":200,"muted":false}"#,
		r#"{"type":"watch_stream","user_id":"4"}"#,
		r#"{"type":"stop_watching"}"#,
		r#"{"type":"decline_call","channel_id":"2"}"#,
		r#"{"type":"join_voice","channel_id":"2","ring":false,"muted":true,"deafened":false}"#,
		r#"{"type":"set_camera","enabled":false}"#,
	];
	for wire in actions {
		let action: AppAction = serde_json::from_str(wire).unwrap();
		let authored: sdk::AppAction = serde_json::from_str(wire).unwrap();
		assert_eq!(
			serde_json::to_value(&authored).unwrap(),
			serde_json::to_value(&action).unwrap()
		);
		action.validate().unwrap();
		let effect = HostEffect::AppAction { action };
		let mut manifest = test_manifest(vec![effect.required_capability()]);
		effect.validate(&manifest).unwrap();
		assert!(matches!(
			effect.validate(&test_manifest(vec![])),
			Err(Error::Capability)
		));
		let output = Output {
			effects: vec![effect],
			..Default::default()
		};
		let input = Invocation {
			action: "run".into(),
			..Default::default()
		};
		output.validate(&manifest, &input).unwrap();
		let authored: sdk::AppOutput =
			serde_json::from_value(serde_json::to_value(&output).unwrap()).unwrap();
		assert_eq!(
			serde_json::to_value(authored.effects).unwrap(),
			serde_json::to_value(&output.effects).unwrap()
		);
		for surface in [
			Surface::Activation,
			Surface::MessageEvent,
			Surface::AppEvent,
		] {
			manifest.actions[0].surface = surface;
			assert!(matches!(
				output.validate(&manifest, &input),
				Err(Error::Capability)
			));
		}
		manifest.actions[0].surface = Surface::Panel;
		let doubled = Output {
			effects: vec![output.effects[0].clone(); 2],
			..Default::default()
		};
		assert!(matches!(
			doubled.validate(&manifest, &input),
			Err(Error::Limit)
		));
	}
}

#[test]
fn app_action_patches_reject_invalid_ranges_conflicts_and_unbounded_text() {
	for wire in [
		r#"{"type":"send_message","channel_id":"0","content":"hello"}"#,
		r#"{"type":"send_message","channel_id":"2","content":" "}"#,
		r#"{"type":"send_reply","channel_id":"2","message_id":"0","content":"hello","mention":false}"#,
		r#"{"type":"send_sticker","channel_id":"2","sticker_id":"0"}"#,
		r#"{"type":"forward_message","channel_id":"2","message_id":"3","target_channel_ids":[],"note":""}"#,
		r#"{"type":"forward_message","channel_id":"2","message_id":"3","target_channel_ids":["6","6"],"note":""}"#,
		r#"{"type":"set_reaction","channel_id":"2","message_id":"3","emoji":"wave:0","add":true}"#,
		r#"{"type":"set_reaction","channel_id":"2","message_id":"3","emoji":"x:4","add":true}"#,
		r#"{"type":"set_own_profile","profile":{}}"#,
		r#"{"type":"set_own_profile","profile":{"global_name":"Name","clear_global_name":true}}"#,
		r#"{"type":"set_own_profile","profile":{"accent_color":16777216}}"#,
		r#"{"type":"set_own_presence","presence":{}}"#,
		r#"{"type":"set_own_presence","presence":{"custom_status":" padded "}}"#,
		r#"{"type":"set_user_note","user_id":"4","text":"note\rline"}"#,
		r#"{"type":"set_own_presence","presence":{"status":"offline"}}"#,
		r#"{"type":"set_own_presence","presence":{"clear_after_seconds":86401}}"#,
		r#"{"type":"set_audio_settings","settings":{}}"#,
		r#"{"type":"set_audio_settings","settings":{"input_percent":201}}"#,
		r#"{"type":"set_audio_settings","settings":{"suppression_level":4}}"#,
		r#"{"type":"set_audio_settings","settings":{"sensitivity_db":-81}}"#,
		r#"{"type":"set_audio_settings","settings":{"sensitivity_db":-40,"open_microphone":true}}"#,
		r#"{"type":"set_audio_settings","settings":{"input_profile":"unknown"}}"#,
		r#"{"type":"set_stream_audio"}"#,
		r#"{"type":"set_participant_audio","user_id":"4","volume_percent":201}"#,
		r#"{"type":"add_friend","username":"invalid..name"}"#,
		r#"{"type":"set_channel_mute","channel_id":"2","duration_seconds":60}"#,
		r#"{"type":"set_channel_notifications","channel_id":"2","level":4}"#,
		r#"{"type":"create_channel","guild_id":"4","name":"general","kind":"stage"}"#,
		r#"{"type":"edit_channel","channel_id":"2","before":{"name":"general","topic":"","slowmode":0,"nsfw":false,"overwrites":[]},"after":{"name":"general","topic":"","slowmode":21601,"nsfw":false,"overwrites":[]}}"#,
		r#"{"type":"edit_channel","channel_id":"2","before":{"name":"general","topic":"","slowmode":0,"nsfw":false,"overwrites":[]},"after":{"name":"general","topic":"","slowmode":0,"nsfw":false,"overwrites":[{"id":"4","kind":2,"allow":"0","deny":"0"}]}}"#,
		r#"{"type":"move_channel","channel_id":"2","parent_id":null,"position":-1,"lock_permissions":false,"shifts":[]}"#,
		r#"{"type":"move_channel","channel_id":"2","parent_id":null,"position":0,"lock_permissions":false,"shifts":[{"channel_id":"9","position":1},{"channel_id":"9","position":2}]}"#,
		r#"{"type":"create_server_invite","guild_id":"4","channel_id":"2","max_age":2592001,"max_uses":0,"temporary":false}"#,
		r#"{"type":"create_server_invite","guild_id":"4","channel_id":"2","max_age":0,"max_uses":101,"temporary":false}"#,
		r#"{"type":"rename_group","channel_id":"2","name":" "}"#,
	] {
		let action: AppAction = serde_json::from_str(wire).unwrap();
		assert!(action.validate().is_err(), "{wire}");
	}
	for wire in [
		r#"{"type":"set_camera","enabled":true,"unknown":true}"#,
		r#"{"type":"set_own_profile","profile":{"unknown":true}}"#,
	] {
		assert!(serde_json::from_str::<AppAction>(wire).is_err());
	}
	for size in [2000, 2001] {
		let action = AppAction::SendMessage {
			channel_id: "2".into(),
			content: "a".repeat(size),
		};
		assert_eq!(action.validate().is_ok(), size == 2000);
	}
	let escaping = HostEffect::AppAction {
		action: AppAction::SendMessage {
			channel_id: "2".into(),
			content: "\n".repeat(1999) + "x",
		},
	};
	escaping
		.validate(&test_manifest(vec![Capability::MessageSend]))
		.unwrap();
	let large = HostEffect::AppAction {
		action: AppAction::CreateForumPost {
			parent_id: "2".into(),
			title: "\u{1f600}".repeat(100),
			content: "\u{1f600}".repeat(2000),
		},
	};
	assert!(matches!(
		large.validate(&test_manifest(vec![Capability::ThreadsControl])),
		Err(Error::Limit)
	));
	OwnProfilePatch {
		clear_global_name: true,
		clear_accent_color: true,
		..Default::default()
	}
	.validate()
	.unwrap();
	OwnPresencePatch {
		custom_status: Some(String::new()),
		clear_after_seconds: Some(0),
		..Default::default()
	}
	.validate()
	.unwrap();
	AudioSettingsPatch {
		input_percent: Some(0),
		output_percent: Some(200),
		sensitivity_db: Some(-80),
		..Default::default()
	}
	.validate()
	.unwrap();
}

#[test]
fn preference_snapshots_are_bounded_and_old_hosts_can_omit_them() {
	let mut data = snapshot();
	let manifest = test_manifest(read_grants());
	data.audio_settings.as_mut().unwrap().input_percent = 201;
	assert!(data.validate(&manifest).is_err());
	data.audio_settings.as_mut().unwrap().input_percent = 100;
	data.audio_settings.as_mut().unwrap().sensitivity_db = None;
	data.validate(&manifest).unwrap();
	data.own_presence.as_mut().unwrap().custom_status = " padded ".into();
	assert!(data.validate(&manifest).is_err());
	data.own_presence.as_mut().unwrap().custom_status = "x".repeat(129);
	assert!(data.validate(&manifest).is_err());
	let old: sdk::AppSnapshot = serde_json::from_str("{}").unwrap();
	assert!(old.audio_settings.is_none() && old.own_presence.is_none());
}
