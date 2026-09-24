use crate::{Command, Envelope, Event, Reply, State, permissions::Event as PermissionEvent};
use model::{
	Channel, ChannelPatch, Freshness, Guild, Id, Message, MessagePatch, Patch, User,
	permissions as p,
};

const BITS: u128 = p::VIEW_CHANNEL
	| p::READ_MESSAGE_HISTORY
	| p::SEND_MESSAGES
	| p::SEND_MESSAGES_IN_THREADS
	| p::ATTACH_FILES
	| p::ADD_REACTIONS
	| p::CONNECT
	| p::SPEAK
	| p::USE_VAD;

fn large_startup() -> crate::Startup {
	let mut startup = crate::Startup {
		external_stickers: false,
		user: user(),
		guilds: vec![],
		channels: vec![],
		permissions: p::Snapshot::default(),
		read_state: crate::read_state::Event::Snapshot {
			entries: None,
			version: None,
			partial: false,
		},
		notifications: None,
		session_dnd: Some(false),
		warnings: Default::default(),
	};
	let mut reads = Vec::new();
	let mut settings = Vec::new();
	for guild in 100..300 {
		let mut permission_guild = snapshot().guilds.remove(0);
		permission_guild.id = Id(guild);
		permission_guild.roles.as_mut().unwrap()[0].id = Id(guild);
		startup.permissions.guilds.push(permission_guild);
		startup.guilds.push(Guild {
			stickers: None,
			id: Id(guild),
			name: "Synthetic large account".into(),
			icon: None,
			emojis: Some(vec![]),
		});
		let mut setting = crate::notifications::Setting {
			guild: Some(Id(guild)),
			muted: Some(false),
			level: Some(0),
			..Default::default()
		};
		for index in 0..100 {
			let id = guild * 1000 + index;
			let mut item = channel(id, 0, None);
			item.guild = Some(Id(guild));
			item.name = "Synthetic account channel ".repeat(4);
			item.last_message = Some(Id(id + 1));
			startup.channels.push(item);
			startup.permissions.channels.push(p::Channel {
				id: Id(id),
				guild: Id(guild),
				overwrites: Some(vec![]),
			});
			reads.push((Id(id), Some(Id(id)), 0));
			setting.channels.push((Id(id), Some(false), Some(0)));
		}
		settings.push(setting);
	}
	startup.read_state = crate::read_state::Event::Snapshot {
		entries: Some(reads),
		version: Some(1),
		partial: false,
	};
	startup.notifications = Some(crate::notifications::Event::Settings {
		entries: settings,
		replace: true,
	});
	startup
}

#[test]
fn large_startup_retains_permissions_read_state_and_subsequent_channel_threads() {
	let startup = large_startup();
	assert!(startup.bytes() > crate::MAX_EVENT_BYTES);
	let mut state = State::default();
	apply(
		&mut state,
		Event::Startup(Box::new(startup.prepare().unwrap())),
	);
	assert_eq!(state.auth, crate::auth::AuthState::Authenticated);
	assert_eq!((state.guilds.len(), state.channels.len()), (200, 20_000));
	for item in &state.channels {
		assert!(state.can_view(item.id));
		assert_eq!(state.read_marker(item.id), Some(Some(item.id)));
	}
	assert!(state.notification_allowed(Id(299_099)));
	let latest = state
		.channels
		.iter()
		.map(|channel| (channel.id, Patch::Value(Id(channel.id.0 + 2))))
		.collect();
	apply(
		&mut state,
		Event::ReadState(crate::read_state::Event::Latest(latest)),
	);
	assert_eq!(
		state.channel(Id(299_099)).unwrap().last_message,
		Some(Id(299_101))
	);
	let mut new_channel = channel(999_000, 0, None);
	new_channel.guild = Some(Id(299));
	apply(&mut state, Event::ChannelCreated(new_channel));
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(999_000),
			guild: Some(Id(299)),
			overwrites: Patch::Value(vec![]),
		},
	);
	assert!(state.can_view(Id(999_000)));
	let mut thread = channel(999_001, 11, Some(Id(999_000)));
	thread.guild = Some(Id(299));
	apply(
		&mut state,
		Event::ThreadsSync {
			guild: Id(299),
			parents: Some(vec![Id(999_000)]),
			threads: vec![thread],
			removed: vec![],
		},
	);
	assert_eq!(state.channels.len(), 20_002);
	assert!(state.can_view(Id(999_001)));
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(999_000),
			guild: Some(Id(299)),
			overwrites: Patch::Value(vec![p::Overwrite {
				id: Id(2),
				kind: 1,
				allow: 0,
				deny: p::VIEW_CHANNEL,
			}]),
		},
	);
	assert!(!state.can_view(Id(999_000)) && !state.can_view(Id(999_001)));
}

#[test]
fn invalid_optional_startup_data_is_unknown_and_recovers_only_from_full_snapshots() {
	let mut startup = large_startup();
	startup.read_state = crate::read_state::Event::Snapshot {
		entries: Some(vec![(Id(100_000), None, 0); 2]),
		version: None,
		partial: false,
	};
	startup.notifications = Some(crate::notifications::Event::Settings {
		entries: vec![crate::notifications::Setting::default(); 2],
		replace: true,
	});
	startup.warnings.sessions = true;
	let mut state = State::default();
	apply(
		&mut state,
		Event::Startup(Box::new(startup.prepare().unwrap())),
	);
	assert_eq!(state.auth, crate::auth::AuthState::Authenticated);
	assert!(
		state.startup_warnings.read_state
			&& state.startup_warnings.notifications
			&& state.startup_warnings.sessions
	);
	assert_eq!(state.read_marker(Id(100_000)), None);
	assert!(!state.notification_allowed(Id(100_000)));
	state
		.apply_read_state(crate::read_state::Event::Snapshot {
			entries: Some(vec![(Id(100_000), Some(Id(5)), 0)]),
			version: None,
			partial: true,
		})
		.unwrap();
	assert!(state.startup_warnings.read_state);
	assert_eq!(state.read_marker(Id(100_000)), Some(Some(Id(5))));
	assert_eq!(state.read_marker(Id(100_001)), None);
	let replacement = large_startup();
	state.apply_read_state(replacement.read_state).unwrap();
	state
		.apply_notification_preferences(replacement.notifications.unwrap())
		.unwrap();
	assert!(!state.startup_warnings.read_state && !state.startup_warnings.notifications);
	assert!(!state.notification_allowed(Id(100_000)));
	state
		.apply_notification_preferences(crate::notifications::Event::Presence(Some(false)))
		.unwrap();
	assert!(state.notification_allowed(Id(100_000)));
	apply(
		&mut state,
		Event::StartupWarnings(model::account::Warnings {
			presence: true,
			..Default::default()
		}),
	);
	apply(
		&mut state,
		Event::StartupWarnings(model::account::Warnings {
			emojis: true,
			..Default::default()
		}),
	);
	assert!(state.startup_warnings.presence && state.startup_warnings.emojis);
	apply(&mut state, Event::Resync);
	assert_eq!(state.startup_warnings, Default::default());
	assert!(!state.notification_preferences_known());
	let generation = state.generation;
	state.logout();
	state.apply(Envelope {
		generation,
		event: Event::Startup(Box::new(large_startup().prepare().unwrap())),
	});
	assert!(state.channels.is_empty());
	assert_eq!(state.startup_warnings, Default::default());
}

#[test]
fn startup_rejects_duplicate_navigation_and_unused_capacity_before_publication() {
	let mut startup = large_startup();
	startup.channels[1].id = startup.channels[0].id;
	assert!(startup.prepare().is_err());
	let mut state = State::default();
	apply(
		&mut state,
		Event::Ready {
			user: user(),
			guilds: vec![],
			channels: Vec::with_capacity(model::account::MAX_BYTES / size_of::<Channel>() + 1),
			permissions: p::Snapshot::default(),
		},
	);
	assert_eq!(state.auth, crate::auth::AuthState::Failed);
	assert!(state.channels.is_empty());
}

fn user() -> User {
	User {
		primary_guild: None,
		id: Id(2),
		name: "Synthetic member".into(),
		avatar: None,
		webhook: false,
		kind: Default::default(),
		discriminator: 0,
	}
}
fn channel(id: u64, kind: u8, parent: Option<Id>) -> Channel {
	Channel {
		id: Id(id),
		guild: Some(Id(10)),
		parent_id: parent,
		kind,
		name: "Synthetic conversation".into(),
		position: 0,
		recipients: vec![],
		last_message: None,
		icon: None,
		member_list_id: None,
		message_count: None,
	}
}
fn message(id: u64, channel: Id) -> Message {
	Message {
		sticker_items: Vec::new(),
		id: Id(id),
		channel,
		kind: 0,
		author: user(),
		content: "Synthetic server content".into(),
		reactions: Some(vec![]),
		edited: false,
		edited_at: None,
		revision: 0,
		nonce: None,
		reply_to: None,
		reply_deleted: false,
		interaction: None,
		forwarded: false,
		unsupported: false,
		components: vec![],
		application_id: None,
		flags: 0,
		ephemeral: false,
		extra_content: Default::default(),
		embeds: vec![],
		attachments: vec![],
		author_nick: None,
		author_roles: vec![],
		mention_roles: vec![],
		mention_everyone: false,
		suppress_notifications: false,
		mentions: vec![],
		embeds_suppressed: false,
	}
}
fn snapshot() -> p::Snapshot {
	p::Snapshot {
		guilds: vec![p::Guild {
			id: Id(10),
			owner: Some(Id(999)),
			roles: Some(vec![
				p::Role {
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
					id: Id(10),
					bits: BITS,
				},
				p::Role {
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
					id: Id(11),
					bits: 0,
				},
			]),
			member: Some(p::Member {
				roles: vec![],
				timeout_until: None,
			}),
		}],
		channels: vec![
			p::Channel {
				id: Id(20),
				guild: Id(10),
				overwrites: Some(vec![p::Overwrite {
					id: Id(11),
					kind: 0,
					allow: 0,
					deny: p::SEND_MESSAGES,
				}]),
			},
			p::Channel {
				id: Id(21),
				guild: Id(10),
				overwrites: Some(vec![p::Overwrite {
					id: Id(2),
					kind: 1,
					allow: 0,
					deny: p::VIEW_CHANNEL,
				}]),
			},
			p::Channel {
				id: Id(22),
				guild: Id(10),
				overwrites: Some(vec![]),
			},
		],
	}
}
fn apply(state: &mut State, event: Event) {
	state.apply(Envelope {
		generation: state.generation,
		event,
	});
}
fn permission(state: &mut State, event: PermissionEvent) {
	apply(state, Event::Permissions(event));
}
fn deny(state: &mut State, bits: u128) {
	permission(
		state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: Some(Id(10)),
			overwrites: Patch::Value(vec![p::Overwrite {
				id: Id(2),
				kind: 1,
				allow: 0,
				deny: bits,
			}]),
		},
	);
}
fn history(state: &mut State, channel: Id, request: u64, id: u64) {
	apply(
		state,
		Event::History {
			channel,
			request,
			older: false,
			messages: vec![message(id, channel)],
		},
	);
}
fn state() -> State {
	let mut state = State::default();
	apply(
		&mut state,
		Event::Ready {
			user: user(),
			guilds: vec![Guild {
				stickers: None,
				id: Id(10),
				name: "Synthetic guild".into(),
				icon: None,
				emojis: None,
			}],
			channels: vec![
				channel(20, 0, None),
				channel(21, 0, None),
				channel(22, 2, None),
				channel(30, 11, Some(Id(20))),
			],
			permissions: snapshot(),
		},
	);
	let Some(Command::History { request, .. }) = state.select(Id(20)) else {
		panic!("Known non-admin member can open history")
	};
	history(&mut state, Id(20), request, 100);
	assert!(state.can_send(Id(20)) && state.can_read_history(Id(20)));
	state
}

#[test]
fn cross_server_emoji_checks_destination_and_known_source_roles() {
	let mut state = state();
	let mut emoji = model::CustomEmoji {
		id: Id(400),
		name: "party".into(),
		animated: true,
		available: true,
		managed: false,
		roles: Some(vec![]),
	};
	state.guilds.push(Guild {
		stickers: None,
		id: Id(40),
		name: "Emoji source".into(),
		icon: None,
		emojis: Some(vec![emoji.clone()]),
	});
	state.channels.push(Channel {
		guild: None,
		..channel(41, 1, None)
	});
	let (source, found) = state.custom_emoji(emoji.id).unwrap();
	assert_eq!(source.id, Id(40));
	assert_eq!(found, &emoji);
	assert!(state.custom_emoji(Id(999)).is_none());
	assert_eq!(
		state.custom_emoji_unavailable_reason(Id(41), Id(40), &emoji),
		None
	);
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(20), Id(40), &emoji)
			.is_some()
	);
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(30), Id(40), &emoji)
			.is_some()
	);
	state.guilds[0].emojis = Some(vec![model::CustomEmoji {
		id: Id(401),
		..emoji.clone()
	}]);
	let (local, local_emoji) = state.custom_emoji(Id(401)).unwrap();
	assert_eq!(
		state.custom_emoji_unavailable_reason(Id(20), local.id, local_emoji),
		None
	);
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: Some(Id(10)),
			overwrites: Patch::Value(vec![p::Overwrite {
				id: Id(2),
				kind: 1,
				allow: p::USE_EXTERNAL_EMOJIS,
				deny: 0,
			}]),
		},
	);
	for target in [Id(20), Id(30), Id(41)] {
		assert_eq!(
			state.custom_emoji_unavailable_reason(target, Id(40), &emoji),
			None
		);
	}
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(21), Id(40), &emoji)
			.is_some()
	);
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(999), Id(40), &emoji)
			.is_some()
	);
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(41), Id(999), &emoji)
			.is_some()
	);

	emoji.roles = Some(vec![Id(42)]);
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(41), Id(40), &emoji)
			.is_some()
	);
	permission(
		&mut state,
		PermissionEvent::Guild(p::Guild {
			id: Id(40),
			owner: Some(Id(999)),
			roles: None,
			member: Some(p::Member {
				roles: vec![Id(42)],
				timeout_until: None,
			}),
		}),
	);
	assert_eq!(
		state.custom_emoji_unavailable_reason(Id(41), Id(40), &emoji),
		None
	);
	permission(
		&mut state,
		PermissionEvent::Member {
			guild: Id(40),
			roles: Patch::Value(vec![]),
			timeout_until: Patch::Absent,
		},
	);
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(41), Id(40), &emoji)
			.is_some()
	);
	emoji.roles = Some(vec![Id(40)]);
	assert_eq!(
		state.custom_emoji_unavailable_reason(Id(41), Id(40), &emoji),
		None
	);
	emoji.roles = None;
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(41), Id(40), &emoji)
			.is_some()
	);
	emoji.roles = Some(vec![]);
	emoji.managed = true;
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(41), Id(40), &emoji)
			.is_some()
	);
	emoji.managed = false;
	emoji.available = false;
	assert!(
		state
			.custom_emoji_unavailable_reason(Id(41), Id(40), &emoji)
			.is_some()
	);
}

#[test]
fn new_custom_reactions_require_eligibility_but_existing_and_removal_stay_separate() {
	let mut state = state();
	let emoji = model::ReactionEmoji {
		id: Some(Id(400)),
		name: Some("party".into()),
	};
	assert!(!state.can_react(Id(100), Some(&emoji), true));
	assert!(state.prepare_reaction(Id(100), emoji.clone()).is_none());
	state.guilds.push(Guild {
		stickers: None,
		id: Id(40),
		name: "Emoji source".into(),
		icon: None,
		emojis: Some(vec![model::CustomEmoji {
			id: Id(400),
			name: "party".into(),
			animated: false,
			available: true,
			managed: false,
			roles: Some(vec![]),
		}]),
	});
	assert!(!state.can_react(Id(100), Some(&emoji), true));
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: Some(Id(10)),
			overwrites: Patch::Value(vec![p::Overwrite {
				id: Id(2),
				kind: 1,
				allow: p::USE_EXTERNAL_EMOJIS,
				deny: 0,
			}]),
		},
	);
	assert!(state.can_react(Id(100), Some(&emoji), true));
	state.guilds.last_mut().unwrap().emojis = None;
	assert!(!state.can_react(Id(100), Some(&emoji), true));
	deny(&mut state, p::ADD_REACTIONS | p::USE_EXTERNAL_EMOJIS);
	state
		.timeline
		.set_reactions(
			Id(100),
			Some(vec![model::Reaction {
				emoji: emoji.clone(),
				count: 1,
				me: false,
				me_burst: false,
			}]),
		)
		.unwrap();
	assert!(state.can_react(Id(100), Some(&emoji), true));
	assert!(state.can_react(Id(100), Some(&emoji), false));
	permission(
		&mut state,
		PermissionEvent::Member {
			guild: Id(10),
			roles: Patch::Absent,
			timeout_until: Patch::Value(State::permission_time() + 60),
		},
	);
	assert!(!state.can_react(Id(100), Some(&emoji), true));
	assert!(state.can_react(Id(100), Some(&emoji), false));
}

#[test]
fn role_rest_catalog_and_self_membership_revoke_selected_history_immediately() {
	use model::{server_admin as admin, server_roles as roles};
	for self_assignment in [false, true] {
		let mut state = state();
		state.auth = crate::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let mut metadata = snapshot().guilds.remove(0);
		let list = metadata.roles.as_mut().unwrap();
		list[0].name = "@everyone".into();
		list[0].bits = 0;
		list[1].name = "History access".into();
		list[1].bits = BITS;
		list[1].position = 1;
		list.push(p::Role {
			id: Id(13),
			name: "Manager".into(),
			bits: p::MANAGE_ROLES | p::MANAGE_GUILD,
			color: 0,
			position: 3,
			hoist: false,
		});
		metadata.member.as_mut().unwrap().roles = vec![Id(11), Id(13)];
		permission(&mut state, PermissionEvent::Guild(metadata.clone()));
		assert!(state.can_read_history(Id(20)) && !state.timeline.is_empty());
		state.server_admin.guild = Some(Id(10));
		let mut member = admin::Member {
			user: user(),
			nick: None,
			roles: vec![Id(11), Id(13)],
			joined_at: None,
			join_source: None,
			invite_code: None,
			flags: None,
			unusual_dm_until: None,
			timeout_until: None,
		};
		state.server_admin.members = Some(admin::Members {
			items: vec![member.clone()],
			roles: metadata
				.roles
				.as_ref()
				.unwrap()
				.iter()
				.map(|role| admin::Role {
					role: role.clone(),
					managed: false,
				})
				.collect(),
			total: 1,
			..Default::default()
		});
		let action = if self_assignment {
			admin::Action::SetRole {
				user: Id(2),
				role: Id(11),
				assigned: false,
			}
		} else {
			admin::Action::Roles(roles::Action::Load)
		};
		let Command::ServerAdmin { guild, request, .. } =
			state.request_server_admin(Id(10), action).unwrap()
		else {
			panic!()
		};
		let result = if self_assignment {
			member.roles = vec![Id(13)];
			admin::Result::Member(member)
		} else {
			let items = metadata
				.roles
				.unwrap()
				.into_iter()
				.map(|role| roles::Role {
					id: role.id,
					name: role.name,
					permissions: if role.id == Id(11) { 0 } else { role.bits },
					position: role.position,
					..Default::default()
				})
				.collect();
			admin::Result::Roles(roles::Result::Catalog {
				catalog: roles::Catalog {
					guild,
					items,
					features: vec![],
				},
				selected: None,
			})
		};
		apply(
			&mut state,
			Event::ServerAdmin(crate::server_admin::Event {
				guild,
				request,
				result: Ok(result),
			}),
		);
		assert!(
			!state.can_view(Id(20)),
			"self assignment: {self_assignment}"
		);
		assert!(
			state.timeline.is_empty(),
			"self assignment: {self_assignment}"
		);
		assert_eq!(state.freshness, Freshness::Unavailable);
	}
}

#[test]
fn message_deletion_uses_manage_messages_without_granting_edit_or_requiring_send() {
	let mut state = state();
	let mut other = message(101, Id(20));
	other.author.id = Id(3);
	apply(&mut state, Event::Message(other));
	assert!(state.can_delete(Id(20), Id(100)));
	assert!(!state.can_delete(Id(20), Id(101)));
	let mut role = snapshot().guilds[0].roles.as_ref().unwrap()[1].clone();
	role.bits = p::MANAGE_MESSAGES;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role,
		},
	);
	permission(
		&mut state,
		PermissionEvent::Member {
			guild: Id(10),
			roles: Patch::Value(vec![Id(11)]),
			timeout_until: Patch::Absent,
		},
	);
	// The role's existing channel overwrite denies SEND_MESSAGES only.
	assert!(!state.can_send(Id(20)));
	assert!(!state.can_edit(Id(20), Id(101)));
	assert!(matches!(
		state.prepare_delete(Id(20), Id(101)),
		Some(Command::Delete {
			channel: Id(20),
			message: Id(101)
		})
	));
	assert!(!state.can_delete(Id(21), Id(101)));
	assert!(!state.can_delete(Id(20), Id(999)));
	let user = state.user.take();
	assert!(!state.can_delete(Id(20), Id(101)));
	state.user = user;
	state.gateway_connected = false;
	assert!(!state.can_delete(Id(20), Id(101)));
	state.gateway_connected = true;
	state.auth = crate::auth::AuthState::Expired;
	assert!(!state.can_delete(Id(20), Id(101)));
	state.auth = crate::auth::AuthState::Unauthenticated;
	assert!(!state.can_delete(Id(20), Id(101)));
	state.auth = crate::auth::AuthState::Authenticated;
	deny(&mut state, p::MANAGE_MESSAGES);
	assert!(state.can_delete(Id(20), Id(100)));
	assert!(state.prepare_delete(Id(20), Id(101)).is_none());
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: Some(Id(10)),
			overwrites: Patch::Null,
		},
	);
	assert!(!state.can_delete(Id(20), Id(100)));
	state.permissions.replace(snapshot()).unwrap();
	state
		.timeline
		.insert(message(100, Id(20)), false, false)
		.unwrap();
	deny(&mut state, p::VIEW_CHANNEL);
	assert!(!state.can_delete(Id(20), Id(100)));
}

#[test]
fn thread_deletion_inherits_parent_overwrites_and_respects_timeout_and_admin() {
	let mut state = state();
	let Some(Command::History { request, .. }) = state.select(Id(30)) else {
		panic!()
	};
	history(&mut state, Id(30), request, 100);
	let mut other = message(101, Id(30));
	other.author.id = Id(3);
	apply(&mut state, Event::Message(other));
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: Some(Id(10)),
			overwrites: Patch::Value(vec![p::Overwrite {
				id: Id(2),
				kind: 1,
				allow: p::MANAGE_MESSAGES,
				deny: p::SEND_MESSAGES_IN_THREADS,
			}]),
		},
	);
	assert!(state.can_delete(Id(30), Id(101)));
	assert!(!state.can_send(Id(30)));
	permission(
		&mut state,
		PermissionEvent::Member {
			guild: Id(10),
			roles: Patch::Absent,
			timeout_until: Patch::Value(i64::MAX),
		},
	);
	assert!(!state.can_delete(Id(30), Id(101)));
	assert!(state.can_delete(Id(30), Id(100)));
	permission(
		&mut state,
		PermissionEvent::Member {
			guild: Id(10),
			roles: Patch::Absent,
			timeout_until: Patch::Null,
		},
	);
	deny(&mut state, p::MANAGE_MESSAGES);
	assert!(!state.can_delete(Id(30), Id(101)));
	let mut role = snapshot().guilds[0].roles.as_ref().unwrap()[0].clone();
	role.bits = p::ADMINISTRATOR;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role,
		},
	);
	assert!(state.can_delete(Id(30), Id(101)));
	assert!(!state.can_edit(Id(30), Id(101)));
}

#[test]
fn deletion_never_uses_guild_privileges_for_other_private_channel_messages() {
	for kind in [1, 3] {
		let mut state = state();
		let mut private = channel(40, kind, None);
		private.guild = None;
		state.channels.push(private);
		let Some(Command::History { request, .. }) = state.select(Id(40)) else {
			panic!()
		};
		history(&mut state, Id(40), request, 100);
		let mut other = message(101, Id(40));
		other.author.id = Id(3);
		apply(&mut state, Event::Message(other));
		// Generic private-channel permissions are permissive; ownership must still be required.
		assert_eq!(state.permission(Id(40), p::MANAGE_MESSAGES), Some(true));
		assert!(state.can_delete(Id(40), Id(100)));
		assert!(!state.can_delete(Id(40), Id(101)));
		assert!(!state.can_edit(Id(40), Id(101)));
		let mut automod = message(102, Id(40));
		automod.kind = 24;
		state.timeline.insert(automod, false, false).unwrap();
		assert!(!state.can_delete(Id(40), Id(102)));
		state.channels.retain(|channel| channel.id != Id(40));
		assert!(!state.can_delete(Id(40), Id(100)));
	}
}

#[test]
fn deletion_obeys_documented_message_types_including_automod_exception() {
	let mut state = state();
	for manage in [false, true] {
		let mut metadata = snapshot();
		if manage {
			metadata.guilds[0].roles.as_mut().unwrap()[0].bits |= p::MANAGE_MESSAGES;
		}
		state.permissions.replace(metadata).unwrap();
		for (kind, allowed) in [
			(0, true),
			(7, true),
			(19, true),
			(46, true),
			(3, false),
			(21, false),
			(13, false),
			(255, false),
			(24, manage),
		] {
			state.timeline.clear();
			let mut message = message(100, Id(20));
			message.kind = kind;
			state.timeline.insert(message, false, false).unwrap();
			assert_eq!(
				state.can_delete(Id(20), Id(100)),
				allowed,
				"kind {kind}, manage {manage}"
			);
		}
	}
}

#[test]
fn revoked_view_cannot_return_through_stale_gateway_content_or_old_history() {
	for resync in [false, true] {
		let mut state = state();
		state.drafts.insert(Id(20), "Keep my draft".into());
		let Command::History { request, .. } = state.history(None) else {
			panic!()
		};
		deny(&mut state, p::VIEW_CHANNEL);
		assert!(state.timeline.is_empty());
		assert!(!state.history_pending);
		apply(
			&mut state,
			if resync {
				Event::Resync
			} else {
				Event::Disconnected
			},
		);
		history(&mut state, Id(20), request, 200);
		apply(&mut state, Event::Message(message(201, Id(20))));
		apply(
			&mut state,
			Event::SendResult {
				nonce: "synthetic late confirmation".into(),
				result: Ok(message(202, Id(20))),
			},
		);
		apply(
			&mut state,
			Event::Patch(MessagePatch {
				sticker_items: model::Patch::Absent,
				components: model::Patch::Absent,
				flags: model::Patch::Absent,
				application_id: model::Patch::Absent,
				extra_content: Default::default(),
				id: Id(203),
				channel: Id(20),
				content: Patch::Value("Late inaccessible edit".into()),
				reactions: Patch::Absent,
				mentions: Patch::Absent,
				edited: Patch::Absent,
				embeds: Patch::Absent,
				embeds_suppressed: Patch::Absent,
				attachments: Patch::Absent,
			}),
		);
		assert!(!state.can_view(Id(20)));
		assert!(
			state.timeline.is_empty(),
			"Stale is not permission to display content"
		);
		assert_eq!(state.drafts[&Id(20)], "Keep my draft");

		permission(&mut state, PermissionEvent::Snapshot(snapshot()));
		apply(&mut state, Event::Resumed);
		let Command::History { request, .. } = state.history(None) else {
			panic!()
		};
		history(&mut state, Id(20), request, 203);
		assert_eq!(
			state.timeline.get(Id(203)).unwrap().content,
			"Synthetic server content"
		);
		assert_eq!(state.freshness, Freshness::Fresh);
	}
}

#[test]
fn send_only_access_accepts_new_live_messages_without_restoring_old_history() {
	let mut state = state();
	let Command::History { request, .. } = state.history(None) else {
		panic!()
	};
	deny(&mut state, p::READ_MESSAGE_HISTORY);
	assert!(state.can_view(Id(20)) && state.can_send(Id(20)));
	assert!(!state.can_read_history(Id(20)));
	assert!(state.timeline.is_empty());
	assert!(matches!(state.history(None), Command::CancelSearch));
	history(&mut state, Id(20), request, 200);
	assert!(state.timeline.is_empty());
	apply(&mut state, Event::Message(message(201, Id(20))));
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: p::Role {
				name: String::new(),
				color: 0,
				position: 0,
				hoist: false,
				id: Id(11),
				bits: p::ATTACH_FILES,
			},
		},
	);
	assert!(
		state.timeline.get(Id(201)).is_some(),
		"Unchanged read denial must not erase the live stream"
	);
	state.drafts.insert(Id(20), "New outgoing message".into());
	assert!(matches!(
		state.prepare_send(),
		Some(Command::Send {
			channel: Id(20),
			..
		})
	));
	assert!(!state.history_pending);
}

#[test]
fn deleting_an_unassigned_role_prunes_its_overwrites_and_invalidates_cached_decisions() {
	let mut state = state();
	// These queries populate the decision cache before each mutation.
	assert!(state.can_view(Id(20)) && state.can_send(Id(20)));
	permission(
		&mut state,
		PermissionEvent::RoleRemoved {
			guild: Id(10),
			id: Id(11),
		},
	);
	assert!(state.can_view(Id(20)) && state.can_send(Id(20)));
	assert!(state.timeline.get(Id(100)).is_some());
	assert!(
		state.permissions.channels[&Id(20)]
			.overwrites
			.as_ref()
			.unwrap()
			.is_empty()
	);
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: p::Role {
				name: String::new(),
				color: 0,
				position: 0,
				hoist: false,
				id: Id(10),
				bits: BITS & !p::SEND_MESSAGES,
			},
		},
	);
	assert!(
		!state.can_send(Id(20)),
		"Role changes must not reuse an earlier cached allow"
	);
	assert!(state.can_read_history(Id(20)) && state.timeline.get(Id(100)).is_some());
}

#[test]
fn thread_target_changes_revoke_content_for_patches_creates_and_snapshots() {
	for parent in [Id(21), Id(22)] {
		for kind in 0..3 {
			let mut state = state();
			let Some(Command::History { request, .. }) = state.select(Id(30)) else {
				panic!()
			};
			history(&mut state, Id(30), request, 300);
			state.reply = Some(Reply::to(Id(300)));
			state.drafts.insert(Id(30), "Keep thread draft".into());
			let Command::History { request, .. } = state.history(None) else {
				panic!()
			};
			let replacement = channel(30, 11, Some(parent));
			let event = match kind {
				0 => Event::ThreadChanged {
					guild: Id(10),
					patch: ChannelPatch {
						icon: model::Patch::Absent,
						id: Id(30),
						parent_id: Patch::Value(parent),
						kind: Patch::Absent,
						message_count: Patch::Absent,
						name: Patch::Absent,
						position: Patch::Absent,
						last_message: Patch::Absent,
					},
				},
				1 => Event::ChannelCreated(replacement),
				_ => Event::ThreadsSync {
					guild: Id(10),
					parents: None,
					threads: vec![replacement],
					removed: vec![],
				},
			};
			apply(&mut state, event);
			assert!(
				!state.can_view(Id(30)),
				"A denied or unsupported parent cannot supply thread access"
			);
			assert!(state.timeline.is_empty() && !state.history_pending && state.reply.is_none());
			assert_eq!(state.drafts[&Id(30)], "Keep thread draft");
			history(&mut state, Id(30), request, 301);
			assert!(state.timeline.is_empty());
		}
	}
}

#[test]
fn malformed_snapshots_are_atomic_and_rejected_permission_events_fail_closed() {
	let mut state = state();
	let original = state.permissions.clone();
	let mut oversized = snapshot();
	oversized.guilds[0].roles = Some(
		(1..=513)
			.map(|id| p::Role {
				name: String::new(),
				color: 0,
				position: 0,
				hoist: false,
				id: Id(id),
				bits: BITS,
			})
			.collect(),
	);
	let mut duplicate = snapshot();
	duplicate.channels.push(duplicate.channels[0].clone());
	for bad in [oversized.clone(), duplicate] {
		assert!(
			state
				.permissions
				.update(PermissionEvent::Snapshot(bad))
				.is_err()
		);
		assert_eq!(state.permissions.guilds, original.guilds);
		assert_eq!(state.permissions.channels, original.channels);
		assert!(state.can_view(Id(20)));
	}
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: None,
			overwrites: Patch::Absent,
		},
	);
	assert!(
		state.can_read_history(Id(20)),
		"Absent overwrite metadata preserves known state"
	);
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: None,
			overwrites: Patch::Null,
		},
	);
	assert_eq!(state.permission(Id(20), p::VIEW_CHANNEL), None);
	assert!(
		state.timeline.is_empty(),
		"Explicit unknown metadata invalidates a cached allow"
	);
	permission(&mut state, PermissionEvent::Snapshot(snapshot()));
	assert!(state.can_view(Id(20)));
	permission(&mut state, PermissionEvent::Snapshot(oversized));
	assert!(!state.can_view(Id(20)) && !state.can_send(Id(20)));
	apply(&mut state, Event::Message(message(400, Id(20))));
	assert!(
		state.timeline.is_empty(),
		"Rejected reducer updates cannot keep granting old access"
	);
}

#[test]
fn member_requests_survive_guild_hydration_and_follow_current_permissions() {
	let mut state = state();
	state.channels[0].member_list_id = Some("everyone".into());
	let request = |state: &mut State| {
		let Some(Command::Members {
			guild: Some(Id(10)),
			list_id: Some(id),
			request,
			..
		}) = state.request_members()
		else {
			panic!("Known channel permissions must produce a member subscription")
		};
		(id, request)
	};
	let (id, first) = request(&mut state);
	assert_eq!(id, "everyone");
	// Subscribing can hydrate a guild: permission snapshot precedes recreated channels.
	permission(&mut state, PermissionEvent::Snapshot(snapshot()));
	apply(&mut state, Event::ChannelCreated(channel(20, 0, None)));
	assert_eq!(state.members.as_ref().unwrap().request, first);
	let mut loaded = model::MemberList {
		guild: Some(Id(10)),
		channel: Id(20),
		request: first,
		total: 1,
		start: 0,
		slots: vec![Some(model::MemberSlot::Person(model::Member {
			activities: vec![],
			roles: vec![],
			user: user(),
			nick: None,
			status: None,
			custom_status: None,
		}))],
		lazy: false,
		groups: vec![],
		ranges: vec![],
		freshness: Freshness::Fresh,
	};
	apply(&mut state, Event::Members(loaded.clone()));
	assert_eq!(state.members.as_ref().unwrap().freshness, Freshness::Fresh);
	let (id, reloaded) = request(&mut state);
	assert_eq!(
		id, "everyone",
		"Reload after GUILD_CREATE must retain a usable identity"
	);
	loaded.request = reloaded;
	apply(&mut state, Event::Members(loaded.clone()));

	// Changing another role's VIEW overwrite changes the list, but not our access.
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: Some(Id(10)),
			overwrites: Patch::Value(vec![p::Overwrite {
				id: Id(11),
				kind: 0,
				allow: 0,
				deny: p::VIEW_CHANNEL,
			}]),
		},
	);
	assert!(state.can_view(Id(20)) && state.members.is_none());
	apply(&mut state, Event::Members(loaded));
	assert!(
		state.members.is_none(),
		"Late old-list rows must not return"
	);
	assert_ne!(request(&mut state).0, "everyone");
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(20),
			guild: Some(Id(10)),
			overwrites: Patch::Null,
		},
	);
	assert!(state.members.is_none());
	// Owner access doesn't make missing list metadata known.
	permission(
		&mut state,
		PermissionEvent::Owner {
			guild: Id(10),
			owner: Patch::Value(Id(2)),
		},
	);
	assert!(matches!(
		state.request_members(),
		Some(Command::Members {
			guild: None,
			list_id: None,
			..
		})
	));
	assert_eq!(
		state.members.as_ref().unwrap().freshness,
		Freshness::Unavailable
	);
	permission(&mut state, PermissionEvent::Snapshot(snapshot()));
	assert!(
		state.members.is_none(),
		"Hydration must wake an unavailable open pane"
	);
	state.history(None);
	let current = state.request;
	history(&mut state, Id(20), current, 100);
	assert_eq!(request(&mut state).0, "everyone");
	assert!(
		state
			.member_list_id(&channel(30, 11, Some(Id(20))))
			.is_none()
	);
}

#[test]
fn member_role_display_tracks_live_role_metadata_and_membership() {
	let mut state = state();
	let mut member = model::Member {
		activities: vec![],
		roles: vec![Id(13), Id(12), Id(11), Id(10)],
		user: user(),
		nick: None,
		status: Some("online".into()),
		custom_status: None,
	};
	let role = |id, position, color, hoist| p::Role {
		id: Id(id),
		bits: 0,
		name: format!("Role {id}"),
		position,
		color,
		hoist,
	};
	for role in [
		role(11, 2, 0x112233, true),
		role(12, 2, 0x445566, true),
		role(13, 3, 0, false),
	] {
		permission(
			&mut state,
			PermissionEvent::Role {
				guild: Id(10),
				role,
			},
		);
	}
	let resolved = |state: &State, member: &model::Member| {
		let (group, color) = state.member_roles(Id(10), member);
		(group.map(|role| role.id), color.map(|role| role.color))
	};
	assert_eq!(resolved(&state, &member), (Some(Id(11)), Some(0x112233)));
	let mut chat = message(100, Id(20));
	chat.author_roles = member.roles.clone();
	assert_eq!(state.message_author_color(&chat), Some(0x112233));
	chat.author.webhook = true;
	assert_eq!(state.message_author_color(&chat), None);
	chat.author.webhook = false;
	state.members = Some(crate::MemberList {
		guild: Some(Id(10)),
		channel: Id(20),
		request: 1,
		total: 1,
		start: 0,
		slots: vec![Some(model::MemberSlot::Person(model::Member {
			roles: vec![],
			..member.clone()
		}))],
		lazy: false,
		groups: vec![],
		ranges: vec![],
		freshness: Freshness::Fresh,
	});
	assert_eq!(
		state.message_author_color(&chat),
		Some(0x112233),
		"empty live membership keeps the message snapshot"
	);
	state.members = Some(crate::MemberList {
		guild: Some(Id(10)),
		channel: Id(20),
		request: 1,
		total: 1,
		start: 0,
		slots: vec![Some(model::MemberSlot::Person(model::Member {
			roles: vec![Id(12)],
			..member.clone()
		}))],
		lazy: false,
		groups: vec![],
		ranges: vec![],
		freshness: Freshness::Fresh,
	});
	assert_eq!(
		state.message_author_color(&chat),
		Some(0x445566),
		"populated live membership refreshes the name color"
	);
	state.members = None;

	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: role(12, 4, 0x778899, false),
		},
	);
	assert_eq!(resolved(&state, &member), (Some(Id(11)), Some(0x778899)));
	assert_eq!(state.message_author_color(&chat), Some(0x778899));
	permission(
		&mut state,
		PermissionEvent::RoleRemoved {
			guild: Id(10),
			id: Id(11),
		},
	);
	assert_eq!(resolved(&state, &member), (None, Some(0x778899)));
	member.roles = vec![Id(13), Id(999)];
	assert_eq!(resolved(&state, &member), (None, None));
	assert!(state.member_roles(Id(99), &member).0.is_none());
	// The default role never gives an individual a group or color, even if malformed.
	let mut everyone = role(10, 999, 0xff_ffff, true);
	everyone.bits = BITS;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: everyone,
		},
	);
	member.roles = vec![Id(10)];
	assert_eq!(resolved(&state, &member), (None, None));
	let before = state.permissions.bytes();
	let mut named = role(14, 0, 0, false);
	named.name.reserve(512);
	let event = PermissionEvent::Role {
		guild: Id(10),
		role: named,
	};
	assert!(event.bytes() >= size_of::<PermissionEvent>() + 512);
	permission(&mut state, event);
	assert!(state.permissions.bytes() >= before + 512);
}

#[test]
fn history_freshness_does_not_disable_authorized_sending() {
	let mut state = state();
	let _ = state.history(Some(Id(100)));
	assert_eq!(state.freshness, Freshness::Loading);
	assert!(state.can_send(Id(20)) && state.can_attach(Id(20)));
	state.gateway_connected = false;
	assert!(!state.can_send(Id(20)));
	state.gateway_connected = true;
	state.freshness = Freshness::Stale;
	assert!(state.can_send(Id(20)));
	state.freshness = Freshness::Loading;
	state.permissions.guilds.clear();
	assert!(!state.can_send(Id(20)) && !state.can_attach(Id(20)));
}

#[test]
fn optimistic_edits_and_pins_roll_back_without_overwriting_newer_content() {
	let mut state = state();
	let original = state.timeline.get(Id(100)).unwrap().content.clone();
	let Some(Command::Edit { request, .. }) =
		state.prepare_edit(Id(20), Id(100), "Local edit".into())
	else {
		panic!()
	};
	assert_eq!(state.timeline.get(Id(100)).unwrap().content, "Local edit");
	state.apply_edit_result(
		Id(20),
		Id(100),
		request,
		Err(crate::auth::Failure::Forbidden),
	);
	assert_eq!(state.timeline.get(Id(100)).unwrap().content, original);
	assert_eq!(
		state.message_actions.failed_edits.pop().unwrap().2,
		"Local edit"
	);
	let Some(Command::Edit { request, .. }) =
		state.prepare_edit(Id(20), Id(100), "Second edit".into())
	else {
		panic!()
	};
	let mut newer = state.timeline.get(Id(100)).unwrap().clone();
	newer.content = "Newer server edit".into();
	newer.edited_at = Some(2);
	newer.edited = true;
	apply(&mut state, Event::Message(newer));
	state.apply_edit_result(
		Id(20),
		Id(100),
		request,
		Err(crate::auth::Failure::Forbidden),
	);
	assert_eq!(
		state.timeline.get(Id(100)).unwrap().content,
		"Newer server edit"
	);
	let request = state.optimistic_pin(Id(20), Id(100), true).unwrap();
	assert!(state.is_pinned(Id(20), Id(100)));
	state.apply_pin_result(
		Id(20),
		Id(100),
		true,
		request,
		Err(crate::auth::Failure::Forbidden),
	);
	assert!(!state.is_pinned(Id(20), Id(100)));
	let Some(Command::Edit { request: old, .. }) =
		state.prepare_edit(Id(20), Id(100), "Interrupted edit".into())
	else {
		panic!()
	};
	apply(&mut state, Event::Disconnected);
	assert_eq!(
		state.timeline.get(Id(100)).unwrap().content,
		"Newer server edit"
	);
	assert_eq!(
		state.message_actions.failed_edits.pop().unwrap().2,
		"Interrupted edit"
	);
	state.gateway_connected = true;
	state.freshness = Freshness::Fresh;
	let Some(Command::Edit { request: new, .. }) =
		state.prepare_edit(Id(20), Id(100), "Retry".into())
	else {
		panic!()
	};
	assert_ne!(old, new);
	state.apply_edit_result(Id(20), Id(100), old, Err(crate::auth::Failure::Forbidden));
	assert!(state.message_actions.edit_pending(Id(20), Id(100)));
	assert_eq!(state.timeline.get(Id(100)).unwrap().content, "Retry");
	state.apply_edit_result(Id(20), Id(100), new, Err(crate::auth::Failure::Forbidden));
	assert_eq!(
		state.timeline.get(Id(100)).unwrap().content,
		"Newer server edit"
	);
	let old = state.optimistic_pin(Id(20), Id(100), true).unwrap();
	state.cancel_message_actions();
	let new = state.optimistic_pin(Id(20), Id(100), true).unwrap();
	state.apply_pin_result(
		Id(20),
		Id(100),
		true,
		old,
		Err(crate::auth::Failure::Forbidden),
	);
	assert!(state.is_pinned(Id(20), Id(100)));
	assert!(state.message_actions.pin_pending(Id(20), Id(100)));
	state.apply_pin_result(
		Id(20),
		Id(100),
		true,
		new,
		Err(crate::auth::Failure::Forbidden),
	);
	assert!(!state.is_pinned(Id(20), Id(100)));
}

#[test]
fn thread_members_load_without_parent_list_and_reject_retired_replies() {
	let mut state = state();
	state.select(Id(30));
	let request = state.request;
	history(&mut state, Id(30), request, 101);
	let Some(Command::Members {
		thread: true,
		guild: Some(Id(10)),
		channel: Some(Id(30)),
		list_id: None,
		request,
		..
	}) = state.request_members()
	else {
		panic!("Threads must request their own participants");
	};
	assert_eq!(
		state.members.as_ref().unwrap().freshness,
		Freshness::Loading
	);
	let mut reply = state.members.clone().unwrap();
	reply.freshness = Freshness::Fresh;
	apply(&mut state, Event::Members(reply.clone()));
	assert_eq!(state.members.as_ref().unwrap().freshness, Freshness::Fresh);
	// Reload retires both successful and failed replies from the previous read.
	state.request_members();
	assert_ne!(state.members.as_ref().unwrap().request, request);
	for freshness in [Freshness::Fresh, Freshness::Unavailable] {
		reply.freshness = freshness;
		apply(&mut state, Event::Members(reply.clone()));
		assert_eq!(
			state.members.as_ref().unwrap().freshness,
			Freshness::Loading
		);
	}
	reply.request = state.members.as_ref().unwrap().request;
	apply(&mut state, Event::Members(reply.clone()));
	assert_eq!(
		state.members.as_ref().unwrap().freshness,
		Freshness::Unavailable
	);
	state.request_members();
	reply = state.members.clone().unwrap();
	reply.freshness = Freshness::Fresh;
	state.close_members();
	apply(&mut state, Event::Members(reply.clone()));
	assert!(state.members.is_none());
	state.request_members();
	deny(&mut state, p::VIEW_CHANNEL);
	assert!(!state.can_view(Id(30)));
	apply(&mut state, Event::Members(reply));
	assert!(
		state
			.members
			.as_ref()
			.is_none_or(|list| list.freshness != Freshness::Fresh)
	);
	assert!(state.request_members().is_none());
}
