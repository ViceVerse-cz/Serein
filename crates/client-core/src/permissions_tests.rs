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

#[test]
fn complete_member_snapshot_clears_loaded_message_nickname_and_roles() {
	let mut state = state();
	let mut old = message(101, Id(20));
	old.author = cached_person(7, "Old nickname").user;
	old.author_nick = Some("Old nickname".into());
	old.author_roles = vec![Id(11)];
	apply(&mut state, Event::Message(old.clone()));
	let mut webhook = old.clone();
	webhook.id = Id(102);
	webhook.author.webhook = true;
	apply(&mut state, Event::Message(webhook));
	state.request_members().unwrap();
	let mut list = state.members.clone().unwrap();
	let mut member = cached_person(7, "unused");
	member.nick = None;
	member.roles.clear();
	list.slots = vec![Some(model::MemberSlot::Person(member))];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list));
	let loaded = state.timeline.get(Id(101)).unwrap();
	assert!(loaded.author_nick.is_none());
	assert!(loaded.author_roles.is_empty());
	assert_eq!(loaded.content, old.content);
	let webhook = state.timeline.get(Id(102)).unwrap();
	assert_eq!(webhook.author_nick.as_deref(), Some("Old nickname"));
	assert_eq!(webhook.author_roles, vec![Id(11)]);
}

#[test]
fn complete_member_snapshot_fences_unchanged_clears_against_delayed_history() {
	let mut state = state();
	let mut old = message(101, Id(20));
	old.author = cached_person(7, "unused").user;
	apply(&mut state, Event::Message(old.clone()));
	let Command::History { request, .. } = state.history(None) else {
		panic!("history request");
	};
	state.request_members().unwrap();
	let mut list = state.members.clone().unwrap();
	let mut member = cached_person(7, "unused");
	member.nick = None;
	member.roles.clear();
	list.slots = vec![Some(model::MemberSlot::Person(member))];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list));
	old.author_nick = Some("Stale nickname".into());
	old.author_roles = vec![Id(11)];
	old.content = "Updated message body".into();
	apply(
		&mut state,
		Event::History {
			channel: Id(20),
			request,
			older: false,
			messages: vec![old],
		},
	);
	let loaded = state.timeline.get(Id(101)).unwrap();
	assert!(loaded.author_nick.is_none());
	assert!(loaded.author_roles.is_empty());
	assert_eq!(loaded.content, "Updated message body");
}

#[test]
fn complete_member_snapshot_clears_survive_member_cache_eviction() {
	let mut state = state();
	let mut colored = state.guild_roles(Id(10)).unwrap()[1].clone();
	colored.color = 0x112233;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: colored,
		},
	);
	let mut old = message(101, Id(20));
	old.author = cached_person(7, "Old nickname").user;
	old.author_nick = Some("Old nickname".into());
	old.author_roles = vec![Id(11)];
	apply(&mut state, Event::Message(old));
	state.request_members().unwrap();
	let mut list = state.members.clone().unwrap();
	let mut member = cached_person(7, "unused");
	member.nick = None;
	member.roles.clear();
	list.slots = vec![Some(model::MemberSlot::Person(member))];
	list.freshness = Freshness::Fresh;
	list.total = 600;
	apply(&mut state, Event::Members(list.clone()));
	for start in [1000, 1200, 1400] {
		list.slots = (start..start + 200)
			.map(|id| {
				Some(model::MemberSlot::Person(cached_person(
					id,
					"Other nickname",
				)))
			})
			.collect();
		apply(&mut state, Event::Members(list.clone()));
	}
	assert!(state.member_identity(Id(20), Id(7)).is_none());
	assert!(state.member_labels.verified_member(Id(10), Id(7)).is_none());
	assert!(state.presentation_member_roles(Id(20), Id(7)).is_none());
	let loaded = state.timeline.get(Id(101)).unwrap();
	assert_eq!(state.message_author_name(loaded), "Account 7");
	assert_eq!(state.message_author_color(loaded), None);
}

#[test]
fn member_list_update_survives_a_delayed_history_response() {
	let mut state = state();
	let mut old = message(101, Id(20));
	old.author.id = Id(7);
	old.author_nick = Some("Old nickname".into());
	old.author_roles = vec![Id(11)];
	apply(&mut state, Event::Message(old.clone()));
	let Command::History { request, .. } = state.history(None) else {
		panic!("history request");
	};
	state.request_members();
	let mut list = state.members.clone().unwrap();
	list.slots = vec![Some(model::MemberSlot::Person(model::Member {
		user: old.author.clone(),
		nick: Some("New nickname".into()),
		roles: vec![Id(12)],
		status: None,
		custom_status: None,
		activities: vec![],
		clients: Default::default(),
	}))];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list));
	old.content = "Updated message body".into();
	apply(
		&mut state,
		Event::History {
			channel: Id(20),
			request,
			older: false,
			messages: vec![old],
		},
	);
	let message = state.timeline.get(Id(101)).unwrap();
	assert_eq!(message.author_nick.as_deref(), Some("New nickname"));
	assert_eq!(message.author_roles, vec![Id(12)]);
	assert_eq!(message.content, "Updated message body");
}

#[test]
fn partial_member_profile_preserves_artwork_and_new_history_content() {
	let mut state = state();
	let mut old = message(101, Id(20));
	old.author.id = Id(7);
	old.author.avatar = Some("0123456789abcdef0123456789abcdef".into());
	old.author.member_avatar = Some(Box::new(model::GuildAvatar {
		guild: Id(10),
		hash: Some("fedcba9876543210fedcba9876543210".into()),
	}));
	apply(&mut state, Event::Message(old.clone()));
	let Command::History { request, .. } = state.history(None) else {
		panic!("history request");
	};
	let mut profile = old.author.clone();
	profile.name = "Updated name".into();
	profile.avatar = None;
	profile.member_avatar = None;
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Set,
			guild: Id(10),
			user: Id(7),
			profile: Some(profile),
			avatar: Patch::Absent,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: false,
		})),
	);
	old.content = "Updated message body".into();
	apply(
		&mut state,
		Event::History {
			channel: Id(20),
			request,
			older: false,
			messages: vec![old],
		},
	);
	let message = state.timeline.get(Id(101)).unwrap();
	assert_eq!(message.content, "Updated message body");
	assert_eq!(message.author.name, "Updated name");
	assert!(message.author.avatar.is_some());
	assert!(
		message
			.author
			.member_avatar
			.as_ref()
			.unwrap()
			.hash
			.is_some()
	);
}

#[test]
fn omitted_global_name_preserves_display_across_a_delayed_history_page() {
	let mut state = state();
	let mut old = message(101, Id(20));
	old.author.id = Id(7);
	old.author.name = "Existing Display".into();
	old.author.username = Some("old_user".into());
	apply(&mut state, Event::Message(old.clone()));
	let Command::History { request, .. } = state.history(None) else {
		panic!("history request");
	};
	let mut partial = old.author.clone();
	partial.name = "new_user".into();
	partial.username = Some("new_user".into());
	let mut identity = model::MemberIdentity {
		global_name: model::GlobalNameUpdate::Unchanged,
		guild: Id(10),
		user: Id(7),
		profile: Some(partial),
		avatar: Patch::Absent,
		nick: Patch::Absent,
		roles: Patch::Absent,
		departed: false,
	};
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(identity.clone())),
	);
	old.content = "Updated message body".into();
	apply(
		&mut state,
		Event::History {
			channel: Id(20),
			request,
			older: false,
			messages: vec![old],
		},
	);
	let message = state.timeline.get(Id(101)).unwrap();
	assert_eq!(message.content, "Updated message body");
	assert_eq!(message.author.name, "Existing Display");
	assert_eq!(message.author.username.as_deref(), Some("new_user"));
	assert_eq!(state.message_author_name(message), "Existing Display");
	identity.global_name = model::GlobalNameUpdate::Cleared;
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(identity.clone())),
	);
	let message = state.timeline.get(Id(101)).unwrap();
	assert_eq!(message.author.name, "new_user");
	assert_eq!(state.message_author_name(message), "new_user");
	identity.global_name = model::GlobalNameUpdate::Unchanged;
	let profile = identity.profile.as_mut().unwrap();
	profile.name = "later_user".into();
	profile.username = Some("later_user".into());
	apply(&mut state, Event::MemberIdentity(Box::new(identity)));
	let message = state.timeline.get(Id(101)).unwrap();
	assert_eq!(message.author.name, "later_user");
	assert_eq!(state.message_author_name(message), "later_user");
}

#[test]
fn member_search_with_unknown_global_name_does_not_cache_username_as_display_name() {
	let mut state = state();
	let Some(Command::MemberSearch(request)) = state.search_members(Id(20), "account", 0) else {
		panic!("member search");
	};
	let mut member = cached_person(7, "Server nickname");
	member.user.name = "account7".into();
	member.user.username = Some("account7".into());
	apply(
		&mut state,
		Event::MemberSearchPartial {
			request,
			rows: vec![member],
			missing: vec![model::MissingMemberFields {
				user: Id(7),
				global_name: model::GlobalNameUpdate::Unchanged,
				nick: false,
				roles: false,
			}],
		},
	);
	assert_eq!(
		state.member_search[0].rows[0].nick.as_deref(),
		Some("Server nickname")
	);
	assert!(state.cached_member_displays(Id(10)).is_empty());
}

#[test]
fn member_search_preserves_a_known_display_name_when_it_is_omitted() {
	let mut state = state();
	let mut known = cached_person(7, "Old nickname");
	known.user.name = "Known display".into();
	known.user.username = Some("account7".into());
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			guild: Id(10),
			user: Id(7),
			profile: Some(known.user.clone()),
			global_name: model::GlobalNameUpdate::Set,
			avatar: Patch::Absent,
			nick: Patch::Value("Old nickname".into()),
			roles: Patch::Value(known.roles.clone()),
			departed: false,
		})),
	);
	let Some(Command::MemberSearch(request)) = state.search_members(Id(20), "account", 0) else {
		panic!("member search");
	};
	known.user.name = "account7".into();
	known.nick = Some("New nickname".into());
	apply(
		&mut state,
		Event::MemberSearchPartial {
			request,
			rows: vec![known],
			missing: vec![model::MissingMemberFields {
				user: Id(7),
				global_name: model::GlobalNameUpdate::Unchanged,
				nick: false,
				roles: false,
			}],
		},
	);
	let saved = &state.cached_member_displays(Id(10))[0];
	assert_eq!(saved.name, "Known display");
	assert_eq!(saved.nick.as_deref(), Some("New nickname"));
}

#[test]
fn member_search_caches_an_explicit_global_name_clear() {
	let mut state = state();
	let Some(Command::MemberSearch(request)) = state.search_members(Id(20), "account", 0) else {
		panic!("member search");
	};
	let mut member = cached_person(7, "Server nickname");
	member.user.name = "account7".into();
	member.user.username = Some("account7".into());
	apply(
		&mut state,
		Event::MemberSearchPartial {
			request,
			rows: vec![member],
			missing: vec![model::MissingMemberFields {
				user: Id(7),
				global_name: model::GlobalNameUpdate::Cleared,
				nick: false,
				roles: false,
			}],
		},
	);
	assert_eq!(state.cached_member_displays(Id(10))[0].name, "account7");
}

#[test]
fn member_list_with_unknown_global_name_does_not_cache_username_as_display_name() {
	let mut state = state();
	state.request_members();
	let mut list = state.members.clone().unwrap();
	let mut member = cached_person(7, "Server nickname");
	member.user.name = "account7".into();
	member.user.username = Some("account7".into());
	list.slots = vec![Some(model::MemberSlot::Person(member))];
	list.missing_identity = vec![model::MissingMemberFields {
		user: Id(7),
		global_name: model::GlobalNameUpdate::Unchanged,
		nick: false,
		roles: false,
	}];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list.clone()));
	assert_eq!(
		state.display_member(Id(20), Id(7)).unwrap().nick.as_deref(),
		Some("Server nickname")
	);
	assert!(state.cached_member_displays(Id(10)).is_empty());
	let Some(model::MemberSlot::Person(member)) = &mut list.slots[0] else {
		panic!("member row")
	};
	member.user.name = "Confirmed display".into();
	list.missing_identity.clear();
	apply(&mut state, Event::Members(list));
	assert_eq!(
		state.cached_member_displays(Id(10))[0].name,
		"Confirmed display"
	);
}

#[test]
fn initial_member_snapshot_keeps_a_known_omitted_global_name() {
	let mut state = state();
	let mut message = message(101, Id(20));
	message.author.id = Id(7);
	message.author.name = "Existing Display".into();
	message.author.username = Some("account".into());
	apply(&mut state, Event::Message(message.clone()));
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Set,
			guild: Id(10),
			user: Id(7),
			profile: Some(message.author.clone()),
			avatar: Patch::Absent,
			nick: Patch::Null,
			roles: Patch::Value(vec![Id(11)]),
			departed: false,
		})),
	);
	state.request_members();
	let mut list = state.members.clone().unwrap();
	let mut partial_user = message.author.clone();
	partial_user.name = "account".into();
	list.slots = vec![Some(model::MemberSlot::Person(model::Member {
		user: partial_user,
		nick: None,
		roles: vec![Id(11)],
		status: None,
		custom_status: None,
		activities: vec![],
		clients: Default::default(),
	}))];
	list.missing_identity = vec![model::MissingMemberFields {
		user: Id(7),
		global_name: model::GlobalNameUpdate::Unchanged,
		nick: false,
		roles: false,
	}];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list.clone()));
	let loaded = state.timeline.get(Id(101)).unwrap();
	assert_eq!(state.message_author_name(loaded), "Existing Display");
	let Some(model::MemberSlot::Person(row)) = state.member_slot(0) else {
		panic!("member row");
	};
	assert_eq!(row.user.name, "Existing Display");
	assert_eq!(
		state.cached_member_displays(Id(10))[0].name,
		"Existing Display"
	);
	assert_eq!(
		state.presentation_member_roles(Id(20), Id(7)),
		Some(&[Id(11)][..])
	);
	list.missing_identity[0].global_name = model::GlobalNameUpdate::Cleared;
	apply(&mut state, Event::Members(list));
	let loaded = state.timeline.get(Id(101)).unwrap();
	assert_eq!(state.message_author_name(loaded), "account");
	assert_eq!(state.cached_member_displays(Id(10))[0].name, "account");
	let mut renamed = message.author;
	renamed.name = "renamed".into();
	renamed.username = Some("renamed".into());
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			guild: Id(10),
			user: Id(7),
			profile: Some(renamed),
			avatar: Patch::Absent,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: false,
		})),
	);
	let loaded = state.timeline.get(Id(101)).unwrap();
	assert_eq!(state.message_author_name(loaded), "renamed");
}

#[test]
fn cleared_name_fallback_survives_member_overlay_eviction() {
	let mut state = state();
	state.request_members();
	let mut list = state.members.clone().unwrap();
	list.slots = (1000..1200)
		.map(|id| {
			let mut member = cached_person(id, "");
			member.nick = None;
			member.user.name = member.user.username.as_ref().unwrap().to_string();
			Some(model::MemberSlot::Person(member))
		})
		.collect();
	list.missing_identity = (1000..1200)
		.map(|id| model::MissingMemberFields {
			user: Id(id),
			global_name: model::GlobalNameUpdate::Cleared,
			nick: false,
			roles: false,
		})
		.collect();
	list.freshness = Freshness::Fresh;
	list.total = 200;
	apply(&mut state, Event::Members(list));
	assert!(state.member_identity(Id(20), Id(1000)).is_none());
	let mut next = state.members.clone().unwrap();
	let Some(model::MemberSlot::Person(member)) = &mut next.slots[0] else {
		panic!("member row");
	};
	member.user.name = "new_account".into();
	member.user.username = Some("new_account".into());
	next.missing_identity[0].global_name = model::GlobalNameUpdate::Unchanged;
	apply(&mut state, Event::Members(next));
	assert_eq!(
		state.display_member(Id(20), Id(1000)).unwrap().user.name,
		"new_account"
	);
	let mut profile = state.display_member(Id(20), Id(1000)).unwrap().user.clone();
	profile.name = "latest_account".into();
	profile.username = Some("latest_account".into());
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			guild: Id(10),
			user: Id(1000),
			profile: Some(profile),
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: Patch::Absent,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: false,
		})),
	);
	assert_eq!(
		state.display_member(Id(20), Id(1000)).unwrap().user.name,
		"latest_account"
	);
}

#[test]
fn stale_search_clear_cannot_resurface_after_overlay_eviction() {
	let mut state = state();
	let Some(Command::MemberSearch(request)) = state.search_members(Id(20), "account", 0) else {
		panic!("member search");
	};
	let mut current = cached_person(7, "").user;
	current.name = "Newest display".into();
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			guild: Id(10),
			user: Id(7),
			profile: Some(current.clone()),
			global_name: model::GlobalNameUpdate::Set,
			avatar: Patch::Absent,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: false,
		})),
	);
	let mut stale = cached_person(7, "");
	stale.user.name = "account7".into();
	stale.nick = None;
	apply(
		&mut state,
		Event::MemberSearchPartial {
			request,
			rows: vec![stale],
			missing: vec![model::MissingMemberFields {
				user: Id(7),
				global_name: model::GlobalNameUpdate::Cleared,
				nick: false,
				roles: false,
			}],
		},
	);
	assert_eq!(state.member_search[0].rows[0].user.name, "Newest display");
	assert!(state.member_search[0].missing_identity.is_empty());
	for id in 1000..1130 {
		let user = cached_person(id, "").user;
		apply(
			&mut state,
			Event::MemberIdentity(Box::new(model::MemberIdentity {
				guild: Id(10),
				user: Id(id),
				profile: Some(user),
				global_name: model::GlobalNameUpdate::Set,
				avatar: Patch::Absent,
				nick: Patch::Absent,
				roles: Patch::Absent,
				departed: false,
			})),
		);
	}
	assert!(state.member_identity(Id(20), Id(7)).is_none());
	current.name = "renamed".into();
	current.username = Some("renamed".into());
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			guild: Id(10),
			user: Id(7),
			profile: Some(current),
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: Patch::Absent,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: false,
		})),
	);
	assert_eq!(state.member_search[0].rows[0].user.name, "Newest display");
}

#[test]
fn live_member_identity_reconciles_sidebar_search_and_old_messages() {
	let mut state = state();
	let mut colored = state.guild_roles(Id(10)).unwrap()[1].clone();
	colored.color = 0x112233;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: colored,
		},
	);
	let mut chat = message(101, Id(20));
	chat.author.id = Id(7);
	chat.author_nick = Some("Old nickname".into());
	chat.author_roles = vec![Id(11)];
	let row = model::Member {
		user: chat.author.clone(),
		nick: chat.author_nick.clone(),
		roles: chat.author_roles.clone(),
		status: Some("online".into()),
		custom_status: None,
		activities: vec![],
		clients: Default::default(),
	};
	apply(&mut state, Event::Message(chat.clone()));
	state.request_members();
	let mut list = state.members.clone().unwrap();
	list.slots = vec![Some(model::MemberSlot::Person(row.clone()))];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list.clone()));
	assert_eq!(state.message_author_color(&chat), Some(0x112233));
	let Command::MemberSearch(request) = state.search_members(Id(20), "Old", 0).unwrap() else {
		panic!()
	};
	let update = |nick, roles| model::MemberIdentity {
		global_name: model::GlobalNameUpdate::Unchanged,
		avatar: model::Patch::Absent,
		guild: Id(10),
		user: Id(7),
		profile: None,
		nick,
		roles,
		departed: false,
	};
	let permissions = state.can_send(Id(20));
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(update(
			Patch::Value("New nickname".into()),
			Patch::Value(vec![]),
		))),
	);
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("New nickname"))
	);
	assert_eq!(state.message_author_name(&chat), "New nickname");
	assert_eq!(
		state.message_author_colors(&chat),
		None,
		"empty current roles clear historical color"
	);
	assert_eq!(
		state.can_send(Id(20)),
		permissions,
		"presentation cannot grant permission"
	);
	let Some(model::MemberSlot::Person(cached)) = state.member_slot(0) else {
		panic!("cached sidebar member")
	};
	assert_eq!(cached.nick.as_deref(), Some("New nickname"));
	assert!(cached.roles.is_empty());
	assert_eq!(cached.status.as_deref(), Some("online"));
	apply(
		&mut state,
		Event::MemberSearch {
			request,
			result: Ok(vec![row.clone()]),
		},
	);
	assert_eq!(
		state.member_search[0].rows[0].nick.as_deref(),
		Some("New nickname"),
		"delayed query cannot restore old labels"
	);
	assert!(state.member_search[0].rows[0].roles.is_empty());
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(update(Patch::Absent, Patch::Value(vec![Id(11)])))),
	);
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("New nickname"))
	);
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(update(Patch::Null, Patch::Value(vec![])))),
	);
	assert_eq!(state.member_nickname(Id(20), Id(7)), Some(None));
	assert_eq!(state.message_author_name(&chat), chat.author.name);
	assert!(state.timeline.get(Id(101)).unwrap().author_nick.is_none());
	assert!(state.timeline.get(Id(101)).unwrap().author_roles.is_empty());
	// A later authoritative sidebar snapshot replaces the overlay too.
	apply(&mut state, Event::Members(list));
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Old nickname"))
	);
	let mut departed = update(Patch::Null, Patch::Value(vec![]));
	departed.departed = true;
	apply(&mut state, Event::MemberIdentity(Box::new(departed)));
	assert!(state.member_slot(0).is_none());
	assert!(state.member_search[0].rows.is_empty());
	assert!(state.member_identity(Id(20), Id(7)).unwrap().departed);
	assert!(state.can_send(Id(20)));
}

#[test]
fn live_identity_growth_reports_a_scoped_member_search_limit() {
	let mut state = state();
	let Command::MemberSearch(request) = state.search_members(Id(20), "Name", 0).unwrap() else {
		panic!()
	};
	let mut rows = Vec::new();
	for id in 100..200 {
		let mut user = message(id, Id(20)).author;
		user.id = Id(id);
		user.name = "N".repeat(128);
		rows.push(model::Member {
			user,
			nick: None,
			roles: (1000..1080).map(Id).collect(),
			status: None,
			custom_status: None,
			activities: vec![],
			clients: Default::default(),
		});
		apply(
			&mut state,
			Event::MemberIdentity(Box::new(model::MemberIdentity {
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: model::Patch::Absent,
				guild: Id(10),
				user: Id(id),
				profile: None,
				nick: Patch::Value("😀".repeat(128)),
				roles: Patch::Absent,
				departed: false,
			})),
		);
	}
	let bytes = rows.iter().map(model::Member::bytes).sum::<usize>();
	assert!(bytes <= crate::member_search::MAX_BYTES);
	assert!(bytes + 512 * rows.len() > crate::member_search::MAX_BYTES);
	apply(
		&mut state,
		Event::MemberSearch {
			request,
			result: Ok(rows),
		},
	);
	assert!(state.member_search[0].finished);
	assert_eq!(
		state.member_search[0].error,
		Some("Member search exceeded its safety limit")
	);
	assert!(state.member_search[0].rows.is_empty());
	assert!(state.gateway_connected && state.can_send(Id(20)));
}

#[test]
fn member_identity_is_scoped_bounded_and_retired_on_recovery() {
	let mut state = state();
	let update = |guild, user| model::MemberIdentity {
		global_name: model::GlobalNameUpdate::Unchanged,
		avatar: model::Patch::Absent,
		guild: Id(guild),
		user: Id(user),
		profile: None,
		nick: Patch::Value("Nickname".repeat(16)),
		roles: Patch::Value(vec![]),
		departed: false,
	};
	state.apply(Envelope {
		generation: state.generation.wrapping_sub(1),
		event: Event::MemberIdentity(Box::new(update(10, 7))),
	});
	assert!(state.member_identity(Id(20), Id(7)).is_none());
	apply(&mut state, Event::MemberIdentity(Box::new(update(99, 7))));
	assert!(state.member_identity(Id(20), Id(7)).is_none());
	for user in 7..1000 {
		apply(
			&mut state,
			Event::MemberIdentity(Box::new(update(10, user))),
		);
	}
	assert!(state.member_labels.bytes() <= 128 * 1024);
	assert!(state.member_identity(Id(20), Id(7)).is_none());
	assert!(state.member_identity(Id(20), Id(999)).is_some());
	state.select(Id(30));
	assert!(
		state.member_identity(Id(30), Id(999)).is_some(),
		"same guild navigation retains fresh identity"
	);
	apply(&mut state, Event::Disconnected);
	assert_eq!(state.member_labels.bytes(), 0);
	apply(&mut state, Event::MemberIdentity(Box::new(update(10, 999))));
	assert!(state.member_identity(Id(30), Id(999)).is_some());
	apply(&mut state, Event::Resumed);
	assert!(
		state.member_identity(Id(30), Id(999)).is_some(),
		"RESUMED must retain replayed identity changes"
	);
	apply(&mut state, Event::Disconnected);
	state.auth = crate::auth::AuthState::Unauthenticated;
	apply(&mut state, Event::MemberIdentity(Box::new(update(10, 999))));
	assert_eq!(state.member_labels.bytes(), 0);
}

#[test]
fn author_gradients_follow_role_hierarchy_and_reject_invalid_stops() {
	let mut state = state();
	let mut role = state.guild_roles(Id(10)).unwrap()[1].clone();
	role.position = 2;
	role.color = 0x112233;
	role.secondary_color = Some(0x445566);
	role.tertiary_color = Some(0x778899);
	let colors = role.colors();
	let role_id = role.id;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: role.clone(),
		},
	);
	let mut chat = message(100, Id(20));
	chat.author_roles = vec![role_id];
	assert_eq!(state.message_author_colors(&chat), Some(colors));
	assert_eq!(
		state.forum_author_colors(Id(20), chat.author.id, false, &chat.author_roles),
		Some(colors)
	);
	let mut lower = role.clone();
	lower.id = Id(12);
	lower.position = 1;
	lower.secondary_color = None;
	lower.tertiary_color = None;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: lower,
		},
	);
	chat.author_roles.push(Id(12));
	assert_eq!(state.message_author_colors(&chat), Some(colors));
	assert_eq!(
		state.forum_author_colors(Id(20), chat.author.id, false, &chat.author_roles),
		Some(colors)
	);
	chat.author.webhook = true;
	assert_eq!(state.message_author_colors(&chat), None);
	assert_eq!(
		state.forum_author_colors(Id(20), chat.author.id, true, &chat.author_roles),
		None
	);
	for tertiary in [false, true] {
		let mut invalid = role.clone();
		if tertiary {
			invalid.tertiary_color = Some(0x1000000);
		} else {
			invalid.secondary_color = Some(0x1000000);
		}
		assert!(
			state
				.permissions
				.update(PermissionEvent::Role {
					guild: Id(10),
					role: invalid
				})
				.is_err()
		);
		assert_eq!(
			state
				.guild_roles(Id(10))
				.unwrap()
				.iter()
				.find(|role| role.id == role_id)
				.unwrap()
				.colors(),
			colors
		);
	}
}

fn large_startup() -> crate::Startup {
	let mut startup = crate::Startup {
		premium_type: 0,
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
			default_message_notifications: None,
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
fn prepared_startup_and_warnings_cannot_restore_a_logged_out_generation() {
	let startup = large_startup().prepare().unwrap();
	let mut state = State::default();
	let previous = state.generation;
	state.logout();
	let generation = state.generation;
	for event in [
		Event::Startup(Box::new(startup)),
		Event::StartupWarnings(model::account::Warnings {
			read_state: true,
			notifications: true,
			..Default::default()
		}),
	] {
		state.apply(crate::Envelope {
			generation: previous,
			event,
		});
		assert_eq!(state.generation, generation);
		assert_eq!(state.auth, crate::auth::AuthState::Unauthenticated);
		assert!(state.user.is_none() && state.selected.is_none());
		assert!(state.channels.is_empty() && state.guilds.is_empty());
		assert_eq!(state.read_marker(Id(100_000)), None);
		assert_eq!(state.startup_warnings, model::account::Warnings::default());
	}
}

#[test]
fn large_startup_retains_permissions_read_state_and_subsequent_channel_threads() {
	{
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
	{
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
	{
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
}

fn user() -> User {
	User {
		member_avatar: None,
		username: None,
		primary_guild: None,
		cosmetics: None,
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
		tags: None,
		message_count: None,
	}
}
fn message(id: u64, channel: Id) -> Message {
	Message {
		reply_preview: None,
		poll: None,
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
					icon: None,
					unicode_emoji: None,
					name: String::new(),
					color: 0,
					secondary_color: None,
					tertiary_color: None,
					position: 0,
					hoist: false,
					id: Id(10),
					bits: BITS,
				},
				p::Role {
					icon: None,
					unicode_emoji: None,
					name: String::new(),
					color: 0,
					secondary_color: None,
					tertiary_color: None,
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
				default_message_notifications: None,
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

fn expression_hints() -> model::ExpressionCatalog {
	model::ExpressionCatalog {
		guild: Id(10),
		emojis: Some(vec![model::CustomEmoji {
			id: Id(400),
			name: "party".into(),
			animated: false,
			available: false,
			managed: false,
			roles: None,
		}]),
		stickers: Some(vec![model::Sticker {
			id: Id(500),
			name: "Wave".into(),
			description: String::new(),
			tags: String::new(),
			format_type: 1,
			guild_id: Some(Id(10)),
			pack_id: None,
			available: false,
		}]),
	}
}

#[test]
fn expression_hints_cover_joined_guilds_in_guild_channels_and_dms() {
	let mut state = state();
	state.auth = crate::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	let mut other = state.guilds[0].clone();
	other.id = Id(11);
	state.guilds.push(other);
	let first = expression_hints();
	let mut second = first.clone();
	second.guild = Id(11);
	second.stickers.as_mut().unwrap()[0].guild_id = Some(Id(11));
	let first_revision = state.expression_guild_cache_revision(Id(10));
	let second_revision = state.expression_guild_cache_revision(Id(11));
	assert!(state.hydrate_expression_catalog(Id(10), first_revision, first.clone()));
	assert!(state.hydrate_expression_catalog(Id(11), second_revision, second.clone()));
	assert_eq!(state.display_guild_emojis(Id(11)).unwrap().len(), 1);
	assert!(!state.can_send_sticker(&second.stickers.as_ref().unwrap()[0]));
	let mut dm = channel(99, 1, None);
	dm.guild = None;
	state.channels.push(dm);
	state.selected = Some(Id(99));
	assert!(state.can_view(Id(99)));
	assert_eq!(state.display_guild_stickers(Id(10)).unwrap().len(), 1);
	assert_eq!(state.display_guild_stickers(Id(11)).unwrap().len(), 1);
	apply(
		&mut state,
		Event::GuildEmojis {
			guild: Id(11),
			emojis: vec![],
		},
	);
	assert!(state.display_guild_emojis(Id(11)).unwrap().is_empty());
	assert_eq!(state.display_guild_stickers(Id(11)).unwrap().len(), 1);
	assert_eq!(state.display_guild_emojis(Id(10)).unwrap().len(), 1);
	assert_eq!(
		state.expression_guild_cache_revision(Id(10)),
		first_revision
	);
	assert!(!state.hydrate_expression_catalog(Id(11), second_revision, second));
	assert!(state.hydrate_expression_catalog(Id(10), first_revision, first));
	apply(&mut state, Event::GuildRemoved(Id(11)));
	assert!(state.display_guild_stickers(Id(11)).is_none());
}

#[test]
fn expression_hints_survive_unrelated_channel_churn_but_hide_on_access_loss() {
	let mut state = state();
	state.auth = crate::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	let revision = state.expression_guild_cache_revision(Id(10));
	assert!(state.hydrate_expression_catalog(Id(10), revision, expression_hints()));
	apply(&mut state, Event::Unavailable(Id(21)));
	assert_eq!(state.expression_guild_cache_revision(Id(10)), revision);
	assert_eq!(state.display_guild_emojis(Id(10)).unwrap().len(), 1);
	apply(&mut state, Event::Unavailable(Id(20)));
	assert!(state.display_guild_emojis(Id(10)).is_none());
	assert!(!state.hydrate_expression_catalog(Id(10), revision, expression_hints()));
}

#[test]
fn expression_revision_table_overflow_fences_old_reads_and_advances_epoch() {
	let mut state = state();
	state.auth = crate::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	let before = state.expression_guild_cache_revision(Id(10));
	let epoch = state.expression_cache_epoch();
	assert!(state.hydrate_expression_catalog(Id(10), before, expression_hints()));
	for guild in 1000..=1128 {
		apply(
			&mut state,
			Event::GuildEmojis {
				guild: Id(guild),
				emojis: vec![],
			},
		);
	}
	assert_ne!(state.expression_cache_epoch(), epoch);
	assert!(state.display_guild_emojis(Id(10)).is_none());
	assert!(!state.hydrate_expression_catalog(Id(10), before, expression_hints()));
	let fresh = state.expression_guild_cache_revision(Id(10));
	assert!(state.hydrate_expression_catalog(Id(10), fresh, expression_hints()));
}

#[test]
fn cached_standard_packs_are_display_only_until_live_results_arrive() {
	let mut state = state();
	state.auth = crate::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	state.selected = None; // Pack hints are also useful in DMs and outside a guild.
	let hints = model::StickerPackHints {
		packs: vec![model::StickerPack {
			id: Id(600),
			name: "Pack".into(),
			stickers: vec![model::Sticker {
				id: Id(601),
				name: "Wave".into(),
				description: String::new(),
				tags: String::new(),
				format_type: 1,
				guild_id: None,
				pack_id: Some(Id(600)),
				available: false,
			}],
		}],
	};
	let started = state.sticker_pack_cache_revision();
	assert!(state.hydrate_sticker_pack_hints(started, hints.clone()));
	assert!(!state.stickers.loaded);
	assert_eq!(state.display_sticker_packs(), hints.packs);
	assert!(state.prepare_sticker_id_send(Id(601)).is_none());
	assert!(state.request_sticker_packs().is_some());
	apply(
		&mut state,
		Event::StickerPacks(Err(crate::auth::Failure::Network)),
	);
	assert_eq!(state.display_sticker_packs(), hints.packs);
	assert!(!state.stickers.loaded);
	assert!(state.request_sticker_packs().is_some());
	apply(&mut state, Event::StickerPacks(Ok(vec![])));
	assert!(state.stickers.loaded);
	assert!(state.display_sticker_packs().is_empty());
	assert!(!state.hydrate_sticker_pack_hints(started, hints));
	assert_eq!(state.cached_sticker_pack_hints().unwrap().packs.len(), 0);
}

#[test]
fn expression_hints_are_display_only_and_live_empty_updates_win_independently() {
	let mut state = state();
	state.auth = crate::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	let hint = expression_hints();
	let revision = state.expression_guild_cache_revision(Id(10));
	assert!(state.hydrate_expression_catalog(Id(10), revision, hint.clone()));
	assert_eq!(state.expression_guild_cache_revision(Id(10)), revision);
	assert_eq!(state.display_guild_emojis(Id(10)).unwrap()[0].id, Id(400));
	assert_eq!(state.display_guild_stickers(Id(10)).unwrap()[0].id, Id(500));
	assert!(state.custom_emoji(Id(400)).is_none());
	assert!(!state.can_send_sticker(&hint.stickers.as_ref().unwrap()[0]));
	assert!(state.cached_expression_catalog(Id(10)).is_none());
	apply(
		&mut state,
		Event::GuildEmojis {
			guild: Id(10),
			emojis: vec![],
		},
	);
	assert!(state.display_guild_emojis(Id(10)).unwrap().is_empty());
	assert_eq!(state.display_guild_stickers(Id(10)).unwrap()[0].id, Id(500));
	assert!(!state.hydrate_expression_catalog(Id(10), revision, hint));
	apply(
		&mut state,
		Event::GuildStickers {
			guild: Id(10),
			stickers: vec![],
		},
	);
	assert!(state.display_guild_stickers(Id(10)).unwrap().is_empty());
}

#[test]
fn expression_hints_fail_closed_on_access_change_and_cache_clear() {
	let mut state = state();
	state.auth = crate::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	let hint = expression_hints();
	let revision = state.expression_guild_cache_revision(Id(10));
	assert!(state.hydrate_expression_catalog(Id(10), revision, hint.clone()));
	deny(&mut state, p::VIEW_CHANNEL);
	assert!(state.display_guild_emojis(Id(10)).is_none());
	assert!(!state.hydrate_expression_catalog(Id(10), revision, hint.clone()));
	let mut other = self::state();
	other.auth = crate::auth::AuthState::Authenticated;
	other.gateway_connected = true;
	let revision = other.expression_guild_cache_revision(Id(10));
	assert!(other.hydrate_expression_catalog(Id(10), revision, hint));
	other.clear_cached_history();
	assert!(other.display_guild_emojis(Id(10)).is_none());
	assert_ne!(other.expression_guild_cache_revision(Id(10)), revision);
}

#[test]
fn expression_snapshot_does_not_persist_permission_or_availability_fields() {
	let mut state = state();
	state.auth = crate::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	let mut hint = expression_hints();
	let mut live_emoji = hint.emojis.as_ref().unwrap()[0].clone();
	live_emoji.available = true;
	live_emoji.managed = true;
	live_emoji.roles = Some(vec![Id(11)]);
	let mut live_sticker = hint.stickers.as_ref().unwrap()[0].clone();
	live_sticker.available = true;
	state.guilds[0].emojis = Some(vec![live_emoji]);
	state.guilds[0].stickers = Some(vec![live_sticker]);
	assert_eq!(state.cached_expression_catalog(Id(10)), Some(hint.clone()));
	hint.emojis.as_mut().unwrap()[0].available = true;
	assert!(!hint.valid());
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
		default_message_notifications: None,
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
	// Without Nitro only the conversation's own server's static emoji are native.
	let local = local.id;
	let local_emoji = model::CustomEmoji {
		animated: false,
		..local_emoji.clone()
	};
	let animated = model::CustomEmoji {
		animated: true,
		..local_emoji.clone()
	};
	assert!(!state.custom_emoji_requires_nitro(Id(20), local, &local_emoji));
	assert!(state.custom_emoji_requires_nitro(Id(20), local, &animated));
	assert!(state.custom_emoji_requires_nitro(Id(41), Id(40), &emoji));
	for premium_type in [1, 2, 3] {
		state.premium_type = premium_type;
		assert!(!state.custom_emoji_requires_nitro(Id(20), local, &animated));
		assert!(!state.custom_emoji_requires_nitro(Id(41), Id(40), &emoji));
	}
	state.premium_type = 0;
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
		default_message_notifications: None,
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
	assert!(
		!state.can_react(Id(100), Some(&emoji), true),
		"external emoji needs Nitro"
	);
	assert!(state.prepare_reaction(Id(100), emoji.clone()).is_none());
	state.premium_type = 2;
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
	assert!(!state.can_react(Id(100), None, true));
	assert!(state.can_open_reaction_picker(Id(100)));
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
			icon: None,
			unicode_emoji: None,
			id: Id(13),
			name: "Manager".into(),
			bits: p::MANAGE_ROLES | p::MANAGE_GUILD,
			color: 0,
			secondary_color: None,
			tertiary_color: None,
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
	{
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
	{
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
	{
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
	{
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
}

#[test]
fn revoked_view_cannot_return_through_stale_gateway_content_or_old_history() {
	{
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
					poll: model::Patch::Absent,
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
	{
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
					icon: None,
					unicode_emoji: None,
					name: String::new(),
					color: 0,
					secondary_color: None,
					tertiary_color: None,
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
		assert!(!state.can_reply_to(Id(201)));
		assert!(state.prepare_reply_send(Id(201), "Reply", false).is_none());
		state.reply = Some(crate::Reply::to(Id(201)));
		assert!(state.prepare_send().is_none());
		assert_eq!(state.reply_target(), Some(Id(201)));
		assert_eq!(state.drafts[&Id(20)], "New outgoing message");
		assert!(state.pending.is_empty());
		state.reply = None;
		assert!(matches!(
			state.prepare_send(),
			Some(Command::Send {
				channel: Id(20),
				reply: None,
				..
			})
		));
		assert!(!state.history_pending);
	}
}

#[test]
fn repeated_member_sync_keeps_cached_decisions() {
	let mut state = state();
	let member = || PermissionEvent::Member {
		guild: Id(10),
		roles: Patch::Value(vec![Id(11)]),
		timeout_until: Patch::Absent,
	};
	permission(&mut state, member());
	// Channel 21 is not selected, so only this query can populate its decisions.
	state.can_view(Id(21));
	let warm = state.permissions.cached_decisions(Id(21));
	assert!(warm > 0, "Decision queries populate the cache");
	permission(&mut state, member());
	assert_eq!(
		state.permissions.cached_decisions(Id(21)),
		warm,
		"An identical sync must not discard valid decisions"
	);
	permission(
		&mut state,
		PermissionEvent::Member {
			guild: Id(10),
			roles: Patch::Value(vec![]),
			timeout_until: Patch::Absent,
		},
	);
	assert_eq!(
		state.permissions.cached_decisions(Id(21)),
		0,
		"A changed role set invalidates the guild's decisions"
	);
}

#[test]
fn removing_a_channel_keeps_other_channels_cached_decisions() {
	let mut state = state();
	state.can_view(Id(21));
	state.can_view(Id(22));
	let sibling = state.permissions.cached_decisions(Id(21));
	assert!(sibling > 0 && state.permissions.cached_decisions(Id(22)) > 0);
	apply(&mut state, Event::Unavailable(Id(22)));
	assert_eq!(state.permissions.cached_decisions(Id(22)), 0);
	assert_eq!(state.permissions.cached_decisions(Id(21)), sibling);
}

#[test]
fn deleting_an_unassigned_role_prunes_its_overwrites_and_invalidates_cached_decisions() {
	{
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
					icon: None,
					unicode_emoji: None,
					name: String::new(),
					color: 0,
					secondary_color: None,
					tertiary_color: None,
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
	{
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
							tags: Patch::Absent,
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
				assert!(
					state.timeline.is_empty() && !state.history_pending && state.reply.is_none()
				);
				assert_eq!(state.drafts[&Id(30)], "Keep thread draft");
				history(&mut state, Id(30), request, 301);
				assert!(state.timeline.is_empty());
			}
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
				icon: None,
				unicode_emoji: None,
				name: String::new(),
				color: 0,
				secondary_color: None,
				tertiary_color: None,
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
	{
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
			missing_identity: vec![],
			guild: Some(Id(10)),
			channel: Id(20),
			request: first,
			total: 1,
			start: 0,
			slots: vec![Some(model::MemberSlot::Person(model::Member {
				activities: vec![],
				clients: model::ClientPlatforms::default(),
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
	{
		let mut state = state();
		let mut member = model::Member {
			activities: vec![],
			clients: model::ClientPlatforms::default(),
			roles: vec![Id(13), Id(12), Id(11), Id(10)],
			user: user(),
			nick: None,
			status: Some("online".into()),
			custom_status: None,
		};
		let role = |id, position, color, hoist| p::Role {
			icon: None,
			unicode_emoji: None,
			id: Id(id),
			bits: 0,
			name: format!("Role {id}"),
			position,
			color,
			secondary_color: None,
			tertiary_color: None,
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
			missing_identity: vec![],
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
			None,
			"current empty roles remove historical color; omitted wire fields are preserved before projection"
		);
		state.members = Some(crate::MemberList {
			missing_identity: vec![],
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
}

#[test]
fn history_freshness_does_not_disable_authorized_sending() {
	let mut state = state();
	let _ = state.history(Some(Id(100)));
	assert_eq!(state.freshness, Freshness::Loading);
	assert!(state.can_send(Id(20)) && state.can_attach(Id(20)));
	state.gateway_connected = false;
	assert!(state.can_send(Id(20)) && !state.can_attach(Id(20)));
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

#[test]
fn new_animated_reactions_need_entitlement_but_existing_reactions_do_not() {
	let mut state = state();
	let custom = model::CustomEmoji {
		id: Id(400),
		name: "party".into(),
		animated: true,
		available: true,
		managed: false,
		roles: Some(vec![]),
	};
	let emoji = model::ReactionEmoji {
		id: Some(custom.id),
		name: Some(custom.name.clone()),
	};
	state.guilds[0].emojis = Some(vec![custom]);
	assert!(!state.can_react(Id(100), Some(&emoji), true));
	assert!(
		state
			.prepare_set_reaction(Id(100), emoji.clone(), true)
			.is_err()
	);
	state.guilds[0].emojis.as_mut().unwrap()[0].animated = false;
	assert!(
		state.can_react(Id(100), Some(&emoji), true),
		"static local emoji remains free"
	);
	state.guilds[0].emojis.as_mut().unwrap()[0].animated = true;
	state.premium_type = 2;
	assert!(state.can_react(Id(100), Some(&emoji), true));
	state.premium_type = 0;
	state
		.timeline
		.set_reactions(
			Id(100),
			Some(vec![model::Reaction {
				emoji: emoji.clone(),
				count: 2,
				me: false,
				me_burst: false,
			}]),
		)
		.unwrap();
	assert!(state.can_react(Id(100), Some(&emoji), true));
	assert!(matches!(
		state.prepare_reaction(Id(100), emoji),
		Some(Command::Reactions(crate::reactions::Command::Set {
			add: true,
			..
		}))
	));
}

fn cached_person(id: u64, nick: &str) -> model::Member {
	let mut user = user();
	user.id = Id(id);
	user.name = format!("Account {id}");
	user.username = Some(format!("account{id}").into());
	model::Member {
		user,
		nick: Some(nick.into()),
		roles: vec![Id(11)],
		status: Some("online".into()),
		custom_status: None,
		activities: vec![],
		clients: Default::default(),
	}
}

#[test]
fn resolved_member_cache_survives_channel_switch_and_reuses_names() {
	let mut state = state();
	let Command::MemberSearch(request) = state.search_members(Id(20), "nickname", 0).unwrap()
	else {
		panic!()
	};
	apply(
		&mut state,
		Event::MemberSearch {
			request,
			result: Ok(vec![cached_person(7, "Persistent nickname")]),
		},
	);
	state.select(Id(30));
	assert_eq!(
		state.member_nickname(Id(30), Id(7)),
		Some(Some("Persistent nickname"))
	);
	state.select(Id(20));
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Persistent nickname"))
	);
	assert!(state.display_member(Id(20), Id(7)).is_some());
	assert!(
		state.request_author_members(&[Id(7)]).is_none(),
		"fresh observed member does not consume another query cooldown"
	);
	let watches = state.watched_member_users().unwrap();
	assert!(watches.2.contains(&Id(7)));
}

#[test]
fn sidebar_only_member_is_refreshed_and_success_updates_every_consumer() {
	let mut state = state();
	let mut colored = state.guild_roles(Id(10)).unwrap()[1].clone();
	colored.color = 0x112233;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: colored,
		},
	);
	let mut chat = message(101, Id(20));
	chat.author.id = Id(7);
	apply(&mut state, Event::Message(chat.clone()));
	state.request_members().unwrap();
	let mut list = state.members.clone().unwrap();
	let mut incomplete = cached_person(7, "Unused");
	incomplete.nick = None;
	incomplete.roles.clear();
	list.slots = vec![Some(model::MemberSlot::Person(incomplete))];
	list.missing_identity = vec![model::MissingMemberFields {
		global_name: model::GlobalNameUpdate::Set,
		user: Id(7),
		nick: true,
		roles: true,
	}];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list));
	let Some(Command::MemberSearch(request)) = state.request_author_members(&[Id(7)]) else {
		panic!("a sidebar-only row must not suppress its first identity lookup");
	};
	let current_generation = state.generation;
	state.apply(Envelope {
		generation: current_generation.wrapping_add(1),
		event: Event::MemberSearch {
			request: request.clone(),
			result: Ok(vec![cached_person(7, "Stale generation")]),
		},
	});
	assert!(!state.member_search[1].finished);
	apply(
		&mut state,
		Event::MemberSearch {
			request,
			result: Ok(vec![cached_person(7, "Current nickname")]),
		},
	);
	let Some(model::MemberSlot::Person(sidebar)) = state.member_slot(0) else {
		panic!("sidebar row retained");
	};
	assert_eq!(sidebar.nick.as_deref(), Some("Current nickname"));
	assert_eq!(sidebar.roles, vec![Id(11)]);
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Current nickname"))
	);
	assert_eq!(state.message_author_name(&chat), "Current nickname");
	assert_eq!(state.message_author_color(&chat), Some(0x112233));
	assert!(state.request_author_members(&[Id(7)]).is_none());
}

#[test]
fn partial_member_snapshot_across_subscriptions_preserves_labels_but_explicit_clear_wins() {
	let mut state = state();
	let mut colored = state.guild_roles(Id(10)).unwrap()[1].clone();
	colored.color = 0x112233;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: colored,
		},
	);
	state.request_members().unwrap();
	let mut first = state.members.clone().unwrap();
	first.slots = vec![Some(model::MemberSlot::Person(cached_person(
		7,
		"Earlier nickname",
	)))];
	first.freshness = Freshness::Fresh;
	first.total = 1;
	apply(&mut state, Event::Members(first));
	state.close_members();
	permission(
		&mut state,
		PermissionEvent::Channel {
			channel: Id(21),
			guild: Some(Id(10)),
			overwrites: Patch::Value(vec![]),
		},
	);
	let Some(Command::History { request, .. }) = state.select(Id(21)) else {
		panic!("next channel")
	};
	history(&mut state, Id(21), request, 101);
	let mut chat = message(102, Id(21));
	chat.author.id = Id(7);
	chat.author_nick = Some("Earlier nickname".into());
	chat.author_roles = vec![Id(11)];
	apply(&mut state, Event::Message(chat.clone()));
	state.request_members().unwrap();
	let mut partial = state.members.clone().unwrap();
	let mut row = cached_person(7, "unused");
	row.nick = None;
	row.roles.clear();
	partial.slots = vec![Some(model::MemberSlot::Person(row))];
	partial.freshness = Freshness::Fresh;
	partial.total = 1;
	partial.missing_identity = vec![model::MissingMemberFields {
		global_name: model::GlobalNameUpdate::Set,
		user: Id(7),
		nick: true,
		roles: true,
	}];
	apply(&mut state, Event::Members(partial.clone()));
	assert_eq!(
		state.member_nickname(Id(21), Id(7)),
		Some(Some("Earlier nickname"))
	);
	assert_eq!(state.message_author_name(&chat), "Earlier nickname");
	assert_eq!(state.message_author_color(&chat), Some(0x112233));
	let loaded = state.timeline.get(Id(102)).unwrap();
	assert_eq!(loaded.author_nick.as_deref(), Some("Earlier nickname"));
	assert_eq!(loaded.author_roles, vec![Id(11)]);
	let Some(model::MemberSlot::Person(sidebar)) = state.member_slot(0) else {
		panic!("member")
	};
	assert_eq!(sidebar.nick.as_deref(), Some("Earlier nickname"));
	assert_eq!(sidebar.roles, vec![Id(11)]);
	let mut renamed = partial.clone();
	renamed.missing_identity[0].nick = false;
	let Some(model::MemberSlot::Person(member)) = renamed.slots[0].as_mut() else {
		panic!("member")
	};
	member.nick = Some("New nickname".into());
	apply(&mut state, Event::Members(renamed));
	assert_eq!(
		state.member_nickname(Id(21), Id(7)),
		Some(Some("New nickname"))
	);
	assert_eq!(state.message_author_color(&chat), Some(0x112233));
	partial.missing_identity.clear();
	apply(&mut state, Event::Members(partial));
	assert_eq!(state.member_nickname(Id(21), Id(7)), Some(None));
	assert_eq!(state.message_author_color(&chat), None);
	let loaded = state.timeline.get(Id(102)).unwrap();
	assert!(loaded.author_nick.is_none());
	assert!(loaded.author_roles.is_empty());
	let Some(model::MemberSlot::Person(sidebar)) = state.member_slot(0) else {
		panic!("member")
	};
	assert!(sidebar.nick.is_none() && sidebar.roles.is_empty());
}

#[test]
fn cached_history_supplies_role_colors_before_the_member_list_arrives() {
	let mut state = state();
	let mut role = state.guild_roles(Id(10)).unwrap()[1].clone();
	role.color = 0x112233;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role,
		},
	);
	let mut first = message(101, Id(20));
	first.author = cached_person(7, "Server nickname").user;
	first.author_roles = vec![Id(11)];
	let mut second = first.clone();
	second.id = Id(102);
	second.author_roles.clear();
	state.timeline.clear();
	state
		.timeline
		.seed_cache(vec![first, second.clone()])
		.unwrap();
	let loaded = state.timeline.get(second.id).unwrap();
	assert!(loaded.author_roles.is_empty());
	assert_eq!(state.message_author_color(loaded), Some(0x112233));
	let mut other = second.clone();
	other.channel = Id(21);
	assert_eq!(state.message_author_color(&other), None);
	other.channel = Id(20);
	other.author.webhook = true;
	assert_eq!(state.message_author_color(&other), None);
	second.author_roles = vec![Id(11)];
	for departed in [true, false] {
		apply(
			&mut state,
			Event::MemberIdentity(Box::new(model::MemberIdentity {
				guild: Id(10),
				user: second.author.id,
				profile: None,
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: Patch::Absent,
				nick: if departed { Patch::Null } else { Patch::Absent },
				roles: if departed {
					Patch::Absent
				} else {
					Patch::Value(vec![])
				},
				departed,
			})),
		);
		assert_eq!(
			state
				.member_identity(Id(20), second.author.id)
				.unwrap()
				.departed,
			departed
		);
		assert_eq!(state.message_author_color(&second), None);
	}
}

#[test]
fn an_unknown_sidebar_row_does_not_mask_the_message_identity() {
	let mut state = state();
	let mut colored = state.guild_roles(Id(10)).unwrap()[1].clone();
	colored.color = 0x112233;
	permission(
		&mut state,
		PermissionEvent::Role {
			guild: Id(10),
			role: colored,
		},
	);
	state.request_members().unwrap();
	let mut list = state.members.clone().unwrap();
	let mut incomplete = cached_person(7, "unused");
	incomplete.nick = None;
	incomplete.roles.clear();
	list.slots = vec![Some(model::MemberSlot::Person(incomplete))];
	list.missing_identity = vec![model::MissingMemberFields {
		global_name: model::GlobalNameUpdate::Set,
		user: Id(7),
		nick: true,
		roles: true,
	}];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list.clone()));
	let mut chat = message(102, Id(20));
	chat.author.id = Id(7);
	chat.author_nick = Some("Message nickname".into());
	chat.author_roles = vec![Id(11)];
	apply(&mut state, Event::Message(chat.clone()));
	assert_eq!(state.message_author_name(&chat), "Message nickname");
	assert_eq!(state.message_author_color(&chat), Some(0x112233));
	let Some(model::MemberSlot::Person(sidebar)) = state.member_slot(0) else {
		panic!("member")
	};
	assert_eq!(
		state.member_nickname(Id(20), sidebar.user.id),
		Some(Some("Message nickname"))
	);
	assert_eq!(state.display_member_role_ids(Id(20), sidebar), &[Id(11)]);
	assert_eq!(
		state
			.roles_for_display(Id(10), state.display_member_role_ids(Id(20), sidebar))
			.1
			.unwrap()
			.color,
		0x112233
	);
	apply(&mut state, Event::Members(list));
	let Some(model::MemberSlot::Person(sidebar)) = state.member_slot(0) else {
		panic!("member")
	};
	assert_eq!(sidebar.nick.as_deref(), Some("Message nickname"));
	assert_eq!(sidebar.roles, vec![Id(11)]);
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			guild: Id(10),
			user: Id(7),
			profile: None,
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: Patch::Absent,
			nick: Patch::Null,
			roles: Patch::Value(vec![]),
			departed: false,
		})),
	);
	let Some(model::MemberSlot::Person(sidebar)) = state.member_slot(0) else {
		panic!("member")
	};
	assert_eq!(state.member_nickname(Id(20), sidebar.user.id), Some(None));
	assert!(state.display_member_role_ids(Id(20), sidebar).is_empty());
}

#[test]
fn cached_nickname_fills_a_partial_sidebar_row_without_promoting_roles() {
	let mut state = state();
	state.request_members().unwrap();
	let mut list = state.members.clone().unwrap();
	let mut partial = cached_person(7, "unused");
	partial.nick = None;
	partial.roles.clear();
	list.slots = vec![Some(model::MemberSlot::Person(partial))];
	list.missing_identity = vec![model::MissingMemberFields {
		global_name: model::GlobalNameUpdate::Set,
		user: Id(7),
		nick: true,
		roles: true,
	}];
	list.freshness = Freshness::Fresh;
	list.total = 1;
	apply(&mut state, Event::Members(list.clone()));
	let cached = model::MemberDisplay {
		guild: Id(10),
		user: Id(7),
		name: "Display".into(),
		username: Some("account".into()),
		nick: Some("Cached nickname".into()),
	};
	state.hydrate_member_displays(
		Id(10),
		state.member_display_revision(),
		vec![cached.clone()],
	);
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Cached nickname"))
	);
	assert!(state.member_labels.verified_member(Id(10), Id(7)).is_none());
	assert_eq!(state.presentation_member_roles(Id(20), Id(7)), None);
	apply(&mut state, Event::Members(list));
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Cached nickname"))
	);
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			guild: Id(10),
			user: Id(7),
			profile: None,
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: Patch::Absent,
			nick: Patch::Null,
			roles: Patch::Value(vec![]),
			departed: false,
		})),
	);
	state.hydrate_member_displays(Id(10), state.member_display_revision(), vec![cached]);
	assert_eq!(state.member_nickname(Id(20), Id(7)), Some(None));
	assert_eq!(
		state.presentation_member_roles(Id(20), Id(7)),
		Some(&[][..])
	);
}

#[test]
fn incomplete_author_lookup_retries_after_a_bounded_delay() {
	let mut state = state();
	let Some(Command::MemberSearch(request)) = state.request_author_members(&[Id(7)]) else {
		panic!("initial lookup");
	};
	apply(
		&mut state,
		Event::MemberSearch {
			request,
			result: Ok(vec![]),
		},
	);
	assert!(state.request_author_members(&[Id(7)]).is_none());
	state.member_search[1].retry_after = Some(std::time::Instant::now());
	assert!(matches!(
		state.request_author_members(&[Id(7)]),
		Some(Command::MemberSearch(_))
	));
}

#[test]
fn mixed_partial_member_search_keeps_absent_fields_and_newer_live_updates() {
	let mut state = state();
	let identity = |user, nick: Patch<String>, roles: Patch<Vec<Id>>| model::MemberIdentity {
		global_name: model::GlobalNameUpdate::Unchanged,
		guild: Id(10),
		user: Id(user),
		profile: None,
		avatar: Patch::Absent,
		nick,
		roles,
		departed: false,
	};
	for user in [7, 8, 9] {
		apply(
			&mut state,
			Event::MemberIdentity(Box::new(identity(
				user,
				Patch::Value("Old nickname".into()),
				Patch::Value(vec![Id(11)]),
			))),
		);
	}
	let Some(Command::MemberSearch(request)) = state.request_author_members(&[Id(7), Id(8), Id(9)])
	else {
		panic!("lookup")
	};
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(identity(
			7,
			Patch::Value("New live nickname".into()),
			Patch::Value(vec![]),
		))),
	);
	let mut first = cached_person(7, "ignored");
	first.nick = None;
	let second = cached_person(8, "Current partial nickname");
	let mut third = cached_person(9, "ignored");
	third.nick = None;
	third.roles.clear();
	let rows = vec![first, second, third];
	let missing = vec![
		model::MissingMemberFields {
			global_name: model::GlobalNameUpdate::Set,
			user: Id(7),
			nick: true,
			roles: false,
		},
		model::MissingMemberFields {
			global_name: model::GlobalNameUpdate::Set,
			user: Id(8),
			nick: false,
			roles: true,
		},
	];
	state.apply(Envelope {
		generation: state.generation.wrapping_add(1),
		event: Event::MemberSearchPartial {
			request: request.clone(),
			rows: rows.clone(),
			missing: missing.clone(),
		},
	});
	assert!(!state.member_search[1].finished);
	apply(
		&mut state,
		Event::MemberSearchPartial {
			request,
			rows,
			missing,
		},
	);
	let view = &state.member_search[1];
	assert_eq!(view.rows.len(), 3);
	assert_eq!(view.rows[0].nick.as_deref(), Some("New live nickname"));
	assert!(view.rows[0].roles.is_empty());
	assert_eq!(
		view.rows[1].nick.as_deref(),
		Some("Current partial nickname")
	);
	assert_eq!(view.rows[1].roles, vec![Id(11)]);
	assert!(view.rows[2].nick.is_none() && view.rows[2].roles.is_empty());
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("New live nickname"))
	);
	assert_eq!(
		state.member_nickname(Id(20), Id(8)),
		Some(Some("Current partial nickname"))
	);
	assert_eq!(state.member_nickname(Id(20), Id(9)), Some(None));
	assert!(
		state
			.request_author_members(&[Id(7), Id(8), Id(9)])
			.is_none()
	);
	state.member_search[1].retry_after = Some(std::time::Instant::now());
	let Some(Command::MemberSearch(retry)) = state.request_author_members(&[Id(7), Id(8), Id(9)])
	else {
		panic!("partial retry")
	};
	assert_eq!(retry.users, vec![Id(7), Id(8)]);
}

#[test]
fn malformed_partial_member_search_is_rejected_without_mutating_identity() {
	for missing in [
		vec![model::MissingMemberFields {
			global_name: model::GlobalNameUpdate::Set,
			user: Id(99),
			nick: true,
			roles: false,
		}],
		vec![
			model::MissingMemberFields {
				global_name: model::GlobalNameUpdate::Set,
				user: Id(7),
				nick: true,
				roles: false
			};
			2
		],
		vec![model::MissingMemberFields {
			global_name: model::GlobalNameUpdate::Set,
			user: Id(7),
			nick: false,
			roles: false,
		}],
	] {
		let mut state = state();
		let Some(Command::MemberSearch(request)) = state.request_author_members(&[Id(7)]) else {
			panic!("lookup")
		};
		apply(
			&mut state,
			Event::MemberSearchPartial {
				request,
				rows: vec![cached_person(7, "Untrusted")],
				missing,
			},
		);
		assert!(state.member_search[1].rows.is_empty());
		assert!(state.member_search[1].error.is_some());
		assert!(state.member_identity(Id(20), Id(7)).is_none());
	}
}

#[test]
fn partial_query_keeps_a_verified_nickname_when_other_fields_change() {
	let mut state = state();
	let Some(Command::MemberSearch(first)) = state.search_members(Id(20), "Known", 0) else {
		panic!("initial search")
	};
	apply(
		&mut state,
		Event::MemberSearch {
			request: first,
			result: Ok(vec![cached_person(7, "Known nickname")]),
		},
	);
	let Some(Command::MemberSearch(second)) = state.search_members(Id(20), "Updated", 0) else {
		panic!("partial search")
	};
	let mut partial = cached_person(7, "unused");
	partial.nick = None;
	partial.roles.clear();
	partial.user.name = "Updated".into();
	apply(
		&mut state,
		Event::MemberSearchPartial {
			request: second,
			rows: vec![partial],
			missing: vec![model::MissingMemberFields {
				global_name: model::GlobalNameUpdate::Set,
				user: Id(7),
				nick: true,
				roles: false,
			}],
		},
	);
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Known nickname"))
	);
	assert!(state.member_search[0].rows[0].roles.is_empty());
	assert_eq!(
		state.presentation_member_roles(Id(20), Id(7)),
		Some(&[][..])
	);
	assert_eq!(
		state.cached_member_displays(Id(10))[0].nick.as_deref(),
		Some("Known nickname")
	);
}

#[test]
fn partial_search_fields_remain_unknown_until_a_live_update_fills_them() {
	let mut state = state();
	let identity = model::MemberIdentity {
		global_name: model::GlobalNameUpdate::Unchanged,
		guild: Id(10),
		user: Id(7),
		profile: None,
		avatar: Patch::Absent,
		nick: Patch::Value("Known nickname".into()),
		roles: Patch::Value(vec![Id(11)]),
		departed: false,
	};
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(identity.clone())),
	);
	let Some(Command::MemberSearch(request)) = state.search_members(Id(20), "Known", 0) else {
		panic!("search")
	};
	let mut partial = cached_person(7, "unused");
	partial.nick = None;
	partial.roles.clear();
	apply(
		&mut state,
		Event::MemberSearchPartial {
			request,
			rows: vec![partial],
			missing: vec![model::MissingMemberFields {
				global_name: model::GlobalNameUpdate::Set,
				user: Id(7),
				nick: true,
				roles: true,
			}],
		},
	);
	assert_eq!(state.searched_member_nickname(Id(20), Id(7)), None);
	assert_eq!(state.searched_member_roles(Id(20), Id(7)), None);
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Known nickname"))
	);
	assert_eq!(state.presentation_member_roles(Id(20), Id(7)), None);

	let _ = state.search_members(Id(20), "Different", 0).unwrap();
	assert_eq!(state.searched_member_nickname(Id(20), Id(7)), None);
	apply(&mut state, Event::MemberIdentity(Box::new(identity)));
	assert_eq!(
		state.searched_member_nickname(Id(20), Id(7)),
		Some(Some("Known nickname"))
	);
	assert_eq!(
		state.searched_member_roles(Id(20), Id(7)),
		Some(&[Id(11)][..])
	);

	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			guild: Id(10),
			user: Id(7),
			profile: None,
			avatar: Patch::Absent,
			nick: Patch::Null,
			roles: Patch::Value(vec![]),
			departed: false,
		})),
	);
	assert_eq!(state.searched_member_nickname(Id(20), Id(7)), Some(None));
	assert_eq!(state.searched_member_roles(Id(20), Id(7)), Some(&[][..]));
}

#[test]
fn repeated_typing_and_role_updates_fence_lookups_without_resaving_names() {
	use std::time::{Duration, Instant, UNIX_EPOCH};
	let mut state = state();
	let person = cached_person(7, "Current nickname");
	let identity = model::MemberIdentity {
		global_name: model::GlobalNameUpdate::Set,
		avatar: model::Patch::Absent,
		guild: Id(10),
		user: Id(7),
		profile: Some(person.user.clone()),
		nick: Patch::Value("Current nickname".into()),
		roles: Patch::Value(vec![Id(11)]),
		departed: false,
	};
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(identity.clone())),
	);
	let saved_revision = state.member_display_cache_revision();
	let saved_rows = state.cached_member_displays(Id(10));
	assert_eq!(saved_rows.len(), 1);
	let Command::MemberSearch(request) = state.search_members(Id(20), "Current", 0).unwrap() else {
		panic!("search must start")
	};
	let observation = state.member_display_revision();
	let now = Instant::now();
	let wall = UNIX_EPOCH + Duration::from_secs(100);
	for _ in 0..20 {
		state.observe_typing_at(
			crate::typing::Signal {
				channel: Id(20),
				user: Id(7),
				timestamp: 100,
				member: Some(Box::new(identity.clone())),
			},
			wall,
			now,
		);
	}
	assert!(state.member_display_revision() > observation);
	assert_eq!(state.member_display_cache_revision(), saved_revision);
	assert_eq!(state.cached_member_displays(Id(10)), saved_rows);
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			avatar: model::Patch::Absent,
			profile: None,
			nick: Patch::Absent,
			roles: Patch::Value(vec![]),
			..identity.clone()
		})),
	);
	assert_eq!(state.member_display_cache_revision(), saved_revision);
	apply(
		&mut state,
		Event::MemberSearch {
			request,
			result: Ok(vec![cached_person(7, "Stale nickname")]),
		},
	);
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Current nickname"))
	);
	assert!(
		state
			.display_member(Id(20), Id(7))
			.unwrap()
			.roles
			.is_empty()
	);
	let before_change = state.member_display_cache_revision();
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			avatar: model::Patch::Absent,
			profile: None,
			nick: Patch::Value("Changed nickname".into()),
			roles: Patch::Absent,
			..identity.clone()
		})),
	);
	assert_ne!(state.member_display_cache_revision(), before_change);
	assert_eq!(
		state.cached_member_displays(Id(10))[0].nick.as_deref(),
		Some("Changed nickname")
	);
	let before_departure = state.member_display_cache_revision();
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			avatar: model::Patch::Absent,
			profile: None,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: true,
			..identity
		})),
	);
	assert_ne!(state.member_display_cache_revision(), before_departure);
	assert!(state.cached_member_displays(Id(10)).is_empty());
	let before_clear = state.member_display_cache_revision();
	apply(&mut state, Event::Disconnected);
	assert_ne!(state.member_display_cache_revision(), before_clear);
}

#[test]
fn member_chunk_identity_and_closed_pane_remain_available_and_live() {
	let mut state = state();
	state.request_members();
	let mut first = state.members.clone().unwrap();
	first.slots = vec![Some(model::MemberSlot::Person(cached_person(
		7,
		"First nickname",
	)))];
	first.freshness = Freshness::Fresh;
	first.total = 300;
	apply(&mut state, Event::Members(first));
	state.focus_member_ranges(100, 110).unwrap();
	let mut next = state.members.clone().unwrap();
	next.start = 100;
	next.slots = vec![Some(model::MemberSlot::Person(cached_person(
		8,
		"Second nickname",
	)))];
	next.freshness = Freshness::Fresh;
	apply(&mut state, Event::Members(next));
	assert!(state.member_slot(0).is_some());
	assert!(state.display_member(Id(20), Id(7)).is_some());
	state.close_members();
	assert!(state.members.is_none());
	assert!(state.parked_members.is_some());
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: model::Patch::Absent,
			guild: Id(10),
			user: Id(7),
			profile: None,
			nick: Patch::Value("Changed while closed".into()),
			roles: Patch::Value(vec![]),
			departed: false,
		})),
	);
	state.request_members().unwrap();
	assert_eq!(state.members.as_ref().unwrap().freshness, Freshness::Stale);
	let Some(model::MemberSlot::Person(member)) = state.member_slot(0) else {
		panic!()
	};
	assert_eq!(member.nick.as_deref(), Some("Changed while closed"));
	assert!(member.roles.is_empty());
	assert!(state.parked_members.is_none());
	state.close_members();
	apply(&mut state, Event::Disconnected);
	assert!(state.parked_members.is_none());
}

#[test]
fn persisted_member_names_are_display_only_and_do_not_override_live_updates() {
	let mut state = state();
	let before = state.can_send(Id(20));
	let row = model::MemberDisplay {
		guild: Id(10),
		user: Id(7),
		name: "Account".into(),
		username: Some("account".into()),
		nick: Some("Disk nickname".into()),
	};
	let started = state.member_display_revision();
	state.hydrate_member_displays(Id(10), started, vec![row.clone()]);
	assert_eq!(
		state.member_nickname(Id(20), Id(7)),
		Some(Some("Disk nickname"))
	);
	assert!(matches!(
		state.member_identity(Id(20), Id(7)).unwrap().roles,
		Patch::Absent
	));
	assert_eq!(state.can_send(Id(20)), before);
	assert!(
		state.request_author_members(&[Id(7)]).is_some(),
		"disk names do not assert fresh roles or suppress refresh"
	);
	let started = state.member_display_revision();
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: model::Patch::Absent,
			guild: Id(10),
			user: Id(7),
			profile: None,
			nick: Patch::Null,
			roles: Patch::Absent,
			departed: false,
		})),
	);
	state.hydrate_member_displays(Id(10), started, vec![row.clone()]);
	assert_eq!(state.member_nickname(Id(20), Id(7)), Some(None));
	let started = state.member_display_revision();
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: model::Patch::Absent,
			guild: Id(10),
			user: Id(7),
			profile: None,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: true,
		})),
	);
	state.hydrate_member_displays(Id(10), started, vec![row]);
	assert!(state.member_identity(Id(20), Id(7)).unwrap().departed);
	assert!(state.cached_member_displays(Id(10)).is_empty());
	apply(&mut state, Event::Disconnected);
	assert_eq!(state.member_labels.bytes(), 0);
}

#[test]
fn nickname_updates_preserve_navigation_caches_and_cooldown_stays_pending() {
	let mut state = state();
	let keys = |state: &State| {
		[
			state.catalog_revision(),
			state.channel_list_revision(),
			state.channel_labels_revision(),
			state.rail_revision(),
		]
	};
	let before = keys(&state);
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: model::Patch::Absent,
			guild: Id(10),
			user: Id(7),
			profile: None,
			nick: Patch::Value("A nickname".into()),
			roles: Patch::Absent,
			departed: false,
		})),
	);
	assert_eq!(keys(&state), before);
	let Command::MemberSearch(request) = state.search_members(Id(20), "Name", 0).unwrap() else {
		panic!()
	};
	apply(
		&mut state,
		Event::MemberSearchCooldown {
			request: request.clone(),
			seconds: 30,
		},
	);
	assert!(!state.member_search[0].finished);
	assert!(state.member_search[0].cooldown_until.is_some());
	assert!(state.member_search[0].error.is_none());
	let mut stale = request.clone();
	stale.nonce = stale.nonce.wrapping_sub(1);
	let deadline = state.member_search[0].cooldown_until;
	apply(
		&mut state,
		Event::MemberSearchCooldown {
			request: stale,
			seconds: 60,
		},
	);
	assert_eq!(state.member_search[0].cooldown_until, deadline);
	apply(
		&mut state,
		Event::MemberSearch {
			request,
			result: Ok(vec![cached_person(7, "Name")]),
		},
	);
	assert!(state.member_search[0].finished);
	assert!(state.member_search[0].cooldown_until.is_none());
}

#[test]
fn disk_hydration_preserves_live_departures_and_most_recent_bounded_rows() {
	let mut state = state();
	apply(
		&mut state,
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: model::Patch::Absent,
			guild: Id(10),
			user: Id(7),
			profile: None,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: true,
		})),
	);
	let started = state.member_display_revision();
	let rows = (7..4103)
		.map(|id| model::MemberDisplay {
			guild: Id(10),
			user: Id(id),
			name: "Account".into(),
			username: None,
			nick: Some(format!("Cached {id}")),
		})
		.collect();
	state.hydrate_member_displays(Id(10), started, rows);
	assert!(state.member_identity(Id(20), Id(7)).unwrap().departed);
	assert_eq!(state.member_nickname(Id(20), Id(8)), Some(Some("Cached 8")));
	assert!(state.member_identity(Id(20), Id(4102)).is_none());
	assert!(state.member_labels.bytes() <= 128 * 1024);
	let revision = state.member_display_revision();
	let ui_revision = state.revision;
	state.hydrate_member_displays(Id(10), revision, vec![]);
	assert_eq!(
		(state.member_display_revision(), state.revision),
		(revision, ui_revision)
	);
}

#[test]
fn viewport_member_watch_prioritizes_old_visible_authors_without_repeated_changes() {
	let mut state = state();
	for id in 1000..1200 {
		let mut message = message(id, Id(20));
		message.author.id = Id(id);
		apply(&mut state, Event::Message(message));
	}
	state.request_author_members(&[Id(90000), Id(90001)]);
	let revision = state.member_watch_revision();
	state.request_author_members(&[Id(90000), Id(90001)]);
	assert_eq!(state.member_watch_revision(), revision);
	let (_, _, users) = state.watched_member_users().unwrap();
	assert_eq!(users.len(), 128);
	assert!(users.windows(2).all(|ids| ids[0] < ids[1]));
	assert!(users.contains(&Id(90000)) && users.contains(&Id(90001)));
	let membership = |departed| {
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			guild: Id(10),
			user: Id(1199),
			profile: None,
			avatar: Patch::Absent,
			nick: if departed {
				Patch::Null
			} else {
				Patch::Value("Returned".into())
			},
			roles: Patch::Absent,
			departed,
		}))
	};
	assert!(users.contains(&Id(1199)));
	apply(&mut state, membership(true));
	let (_, _, without_departed) = state.watched_member_users().unwrap();
	assert_eq!(without_departed.len(), 128);
	assert!(!without_departed.contains(&Id(1199)));
	assert!(without_departed.contains(&Id(90000)));
	apply(&mut state, membership(false));
	let (_, _, after_return) = state.watched_member_users().unwrap();
	assert!(after_return.contains(&Id(1199)));
	state.request_author_members(&[Id(90002)]);
	assert_ne!(state.member_watch_revision(), revision);
}

#[test]
fn member_watch_keeps_the_first_distinct_users_when_messages_repeat() {
	let mut state = state();
	state.timeline.clear();
	for index in 0..400 {
		let mut chat = message(10_000 + index, Id(20));
		chat.author.id = Id(1_000 + index % 200);
		apply(&mut state, Event::Message(chat));
	}
	state.request_author_members(&[Id(90_000), Id(0), Id(90_001), Id(90_000)]);
	let mut expected: Vec<_> = (1_074..1_200).map(Id).collect();
	expected.extend([Id(90_000), Id(90_001)]);
	let (_, _, users) = state.watched_member_users().unwrap();
	assert_eq!(users, expected);
	let departed = |departed| {
		Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			guild: Id(10),
			user: Id(1_199),
			profile: None,
			avatar: Patch::Absent,
			nick: if departed {
				Patch::Null
			} else {
				Patch::Value("Returned".into())
			},
			roles: Patch::Absent,
			departed,
		}))
	};
	apply(&mut state, departed(true));
	expected.retain(|id| *id != Id(1_199));
	expected.insert(0, Id(1_073));
	assert_eq!(state.watched_member_users().unwrap().2, expected);
	apply(&mut state, departed(false));
	expected.remove(0);
	expected.insert(125, Id(1_199));
	assert_eq!(state.watched_member_users().unwrap().2, expected);
}

fn member_reply_after_disk_hint(role_update: bool) {
	for slot in 0..=1 {
		let mut state = state();
		if slot == 1 {
			apply(
				&mut state,
				Event::MemberIdentity(Box::new(model::MemberIdentity {
					global_name: model::GlobalNameUpdate::Unchanged,
					avatar: model::Patch::Absent,
					guild: Id(10),
					user: Id(8),
					profile: None,
					nick: Patch::Value("Other live member".into()),
					roles: Patch::Absent,
					departed: false,
				})),
			);
		}
		let permissions = state.can_send(Id(20));
		let disk_started = state.member_display_revision();
		let command = if slot == 0 {
			state.search_members(Id(20), "Current", slot)
		} else {
			state.request_author_members(&[Id(7)])
		};
		let Some(Command::MemberSearch(request)) = command else {
			panic!("synthetic member query must start");
		};
		state.hydrate_member_displays(
			Id(10),
			disk_started,
			vec![model::MemberDisplay {
				guild: Id(10),
				user: Id(7),
				name: "Old display name".into(),
				username: Some("old_username".into()),
				nick: Some("Old cached nickname".into()),
			}],
		);
		assert_eq!(
			state.member_nickname(Id(20), Id(7)),
			Some(Some("Old cached nickname"))
		);
		assert_eq!(state.member_display_revision(), disk_started);
		if role_update {
			apply(
				&mut state,
				Event::MemberIdentity(Box::new(model::MemberIdentity {
					global_name: model::GlobalNameUpdate::Unchanged,
					avatar: model::Patch::Absent,
					guild: Id(10),
					user: Id(7),
					profile: None,
					nick: Patch::Absent,
					roles: Patch::Value(vec![]),
					departed: false,
				})),
			);
		}
		let mut fresh = cached_person(7, "Current nickname");
		fresh.user.name = "Current display name".into();
		fresh.user.username = Some("current_username".into());
		apply(
			&mut state,
			Event::MemberSearch {
				request,
				result: Ok(vec![fresh]),
			},
		);
		let row = &state.member_search[slot].rows[0];
		assert_eq!(
			row.nick.as_deref(),
			Some("Current nickname"),
			"an older disk hint must not acquire a newer observation revision"
		);
		assert_eq!(row.user.name, "Current display name");
		assert_eq!(row.user.username.as_deref(), Some("current_username"));
		assert_eq!(row.roles, if role_update { vec![] } else { vec![Id(11)] });
		assert_eq!(
			state.member_nickname(Id(20), Id(7)),
			Some(Some("Current nickname"))
		);
		// Once the query validates the hint, a later partial update may preserve its fields.
		apply(
			&mut state,
			Event::MemberIdentity(Box::new(model::MemberIdentity {
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: model::Patch::Absent,
				guild: Id(10),
				user: Id(7),
				profile: None,
				nick: Patch::Absent,
				roles: Patch::Value(vec![Id(11)]),
				departed: false,
			})),
		);
		let identity = state.display_member(Id(20), Id(7)).unwrap();
		assert_eq!(identity.user.name, "Current display name");
		assert_eq!(
			state.member_nickname(Id(20), Id(7)),
			Some(Some("Current nickname"))
		);
		assert_eq!(state.can_send(Id(20)), permissions);
	}
}

#[test]
fn inflight_member_reply_overrides_older_disk_hint() {
	member_reply_after_disk_hint(false);
}

#[test]
fn inflight_member_reply_keeps_new_roles_without_promoting_disk_names() {
	member_reply_after_disk_hint(true);
}
