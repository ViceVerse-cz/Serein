use crate::{Command, State, auth::AuthState, soundboard::*, voice::Phase};
use model::{Id, permissions as p};

fn state() -> State {
	let mut state = State {
		auth: AuthState::Authenticated,
		gateway_connected: true,
		user: Some(model::User {
			id: Id(1),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		}),
		guilds: vec![model::Guild {
			id: Id(10),
			name: "Synthetic server".into(),
			icon: None,
			emojis: None,
			stickers: None,
			default_message_notifications: None,
		}],
		channels: vec![model::Channel {
			id: Id(20),
			guild: Some(Id(10)),
			kind: 2,
			name: "Synthetic voice".into(),
			last_message: None,
			parent_id: None,
			position: 0,
			recipients: vec![],
			icon: None,
			member_list_id: None,
			tags: None,
			message_count: None,
		}],
		..Default::default()
	};
	state
		.permissions
		.replace(p::Snapshot {
			guilds: vec![p::Guild {
				id: Id(10),
				owner: Some(Id(99)),
				roles: Some(vec![p::Role {
					id: Id(10),
					name: "everyone".into(),
					color: 0,
					position: 0,
					hoist: false,
					bits: p::VIEW_CHANNEL | p::CONNECT | p::SPEAK | p::USE_SOUNDBOARD,
				}]),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
			}],
			channels: vec![p::Channel {
				id: Id(20),
				guild: Id(10),
				overwrites: Some(vec![]),
			}],
		})
		.unwrap();
	state.start_call(Id(20), false).unwrap();
	state.voice.active.as_mut().unwrap().phase = Phase::Connected;
	state
}
fn sound(available: bool) -> Sound {
	Sound {
		id: Id(99),
		name: "Synthetic celebration".into(),
		volume: 1.0,
		emoji: Some("🎉".into()),
		emoji_id: None,
		guild: Some(Id(10)),
		available,
	}
}
fn load(state: &mut State) -> Request {
	let Command::Soundboard(request) = state.request_soundboard(Action::Load).unwrap() else {
		panic!("wrong command")
	};
	state.apply_soundboard(Event {
		scope: request.scope,
		request: request.request,
		result: Ok(Outcome::Loaded(vec![sound(true)])),
	});
	request
}

#[test]
fn soundboard_selection_is_explicit_bounded_and_rate_limited_without_ending_call() {
	let mut state = state();
	assert!(state.soundboard.sounds.is_empty());
	assert!(state.request_soundboard(Action::Play(Id(99))).is_none());
	let Command::Soundboard(request) = state.request_soundboard(Action::Load).unwrap() else {
		panic!("wrong command")
	};
	assert!(state.request_soundboard(Action::Load).is_none());
	state.apply_soundboard(Event {
		scope: request.scope,
		request: request.request + 1,
		result: Ok(Outcome::Loaded(vec![sound(true)])),
	});
	assert!(state.soundboard.sounds.is_empty());
	assert!(state.soundboard.pending.is_some());
	state.apply_soundboard(Event {
		scope: request.scope,
		request: request.request,
		result: Ok(Outcome::Loaded(vec![sound(true)])),
	});
	assert!(state.can_play_soundboard(Id(99)));
	assert!(state.request_soundboard(Action::Play(Id(100))).is_none());
	let Command::Soundboard(play) = state.request_soundboard(Action::Play(Id(99))).unwrap() else {
		panic!("wrong command")
	};
	state.apply_soundboard(Event {
		scope: play.scope,
		request: play.request,
		result: Err(crate::auth::Failure::Forbidden),
	});
	assert_eq!(state.voice.active.as_ref().unwrap().phase, Phase::Connected);
	assert_eq!(
		state.soundboard.error,
		Some(crate::auth::Failure::Forbidden)
	);
	assert!(state.request_soundboard(Action::Play(Id(99))).is_none());
	state.soundboard.sounds[0].available = false;
	assert!(!state.can_play_soundboard(Id(99)));
	state.leave_call();
	assert!(state.soundboard.sounds.is_empty());
	assert!(state.soundboard.pending.is_none());
}

#[test]
fn soundboard_revokes_catalog_and_stale_completions_on_call_or_access_changes() {
	for change in [
		"guild",
		"stage",
		"phase",
		"unconfirmed",
		"server-mute",
		"server-deaf",
		"self-deaf",
		"gateway",
		"session",
		"generation",
		"speak",
		"soundboard",
		"request",
		"leave",
		"takeover",
	] {
		let mut state = state();
		load(&mut state);
		let Command::Soundboard(request) = state.request_soundboard(Action::Load).unwrap() else {
			panic!("wrong command")
		};
		assert!(state.soundboard.pending.is_some());
		match change {
			"leave" => {
				state.leave_call();
			}
			"takeover" => {
				let call = state.voice.active.as_ref().unwrap();
				state.apply_voice(crate::voice::Event::TakenOver {
					channel: call.channel,
					request: call.request,
				});
			}
			"guild" => state.voice.active.as_mut().unwrap().guild = None,
			"stage" => state.channels[0].kind = 13,
			"phase" => state.voice.active.as_mut().unwrap().phase = Phase::Connecting,
			"unconfirmed" => {
				let call = state.voice.active.as_ref().unwrap();
				state.apply_voice(crate::voice::Event::Progress {
					channel: call.channel,
					request: call.request,
					phase: Phase::Securing,
				});
			}
			"server-mute" => state.voice.active.as_mut().unwrap().server_muted = true,
			"server-deaf" => state.voice.active.as_mut().unwrap().server_deafened = true,
			"self-deaf" => state.voice.active.as_mut().unwrap().deafened = true,
			"gateway" => state.gateway_connected = false,
			"session" => state.auth = AuthState::Expired,
			"generation" => state.generation += 1,
			"request" => state.voice.active.as_mut().unwrap().request += 1,
			"speak" | "soundboard" => {
				state
					.permissions
					.guilds
					.get_mut(&Id(10))
					.unwrap()
					.roles
					.as_mut()
					.unwrap()[0]
					.bits &= !(if change == "speak" {
					p::SPEAK
				} else {
					p::USE_SOUNDBOARD
				});
				state.permissions.clear_cache();
			}
			_ => unreachable!(),
		}
		state.revalidate_soundboard();
		assert!(state.soundboard.sounds.is_empty(), "{change}");
		assert!(state.soundboard.pending.is_none(), "{change}");
		assert!(
			state.request_soundboard(Action::Play(Id(99))).is_none(),
			"{change}"
		);
		state.apply_soundboard(Event {
			scope: request.scope,
			request: request.request,
			result: Ok(Outcome::Loaded(vec![sound(true)])),
		});
		assert!(state.soundboard.sounds.is_empty(), "{change}");
	}
	let mut state = state();
	load(&mut state);
	state.logout();
	assert!(state.soundboard.sounds.is_empty());
}

#[test]
fn soundboard_rejects_over_budget_or_invalid_catalogs_without_losing_current_call() {
	for change in [
		"spare",
		"string-capacity",
		"duplicates",
		"guild",
		"volume",
		"items",
	] {
		let mut state = state();
		load(&mut state);
		let Command::Soundboard(request) = state.request_soundboard(Action::Load).unwrap() else {
			panic!("wrong command")
		};
		let mut sounds = vec![sound(true)];
		match change {
			"spare" => {
				let mut allocated = Vec::with_capacity(MAX_BYTES / size_of::<Sound>() + 1);
				allocated.append(&mut sounds);
				sounds = allocated;
			}
			"string-capacity" => sounds[0].name.reserve(MAX_BYTES),
			"duplicates" => sounds.push(sound(true)),
			"guild" => sounds[0].guild = Some(Id(11)),
			"volume" => sounds[0].volume = f64::NAN,
			"items" => {
				sounds = (1..=MAX_SOUNDS + 1)
					.map(|index| {
						let mut item = sound(true);
						item.id = Id(index as u64);
						item
					})
					.collect();
			}
			_ => unreachable!(),
		}
		state.apply_soundboard(Event {
			scope: request.scope,
			request: request.request,
			result: Ok(Outcome::Loaded(sounds)),
		});
		assert_eq!(
			state.soundboard.error,
			Some(crate::auth::Failure::Protocol),
			"{change}"
		);
		assert!(state.soundboard.pending.is_none(), "{change}");
		assert_eq!(state.soundboard.sounds, vec![sound(true)], "{change}");
		assert_eq!(
			state.voice.active.as_ref().unwrap().phase,
			Phase::Connected,
			"{change}"
		);
	}
}
