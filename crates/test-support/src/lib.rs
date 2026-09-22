//! Handcrafted synthetic data. No network imports; never evidence of live compatibility.
use client_core::{Envelope, Event, State};
use model::*;
/// Synthetic Tenor-shaped results. Previews under `/synthetic/` are painted locally; no request.
pub fn gif_page(query: Option<&str>) -> model::GifPage {
	const TITLES: [&str; 12] = [
		"Excited wave",
		"Slow clap",
		"Thumbs up",
		"Happy dance",
		"Mind blown",
		"Popcorn time",
		"Cat typing",
		"High five",
		"Facepalm",
		"Confetti",
		"Nodding",
		"Shrug",
	];
	const SIZES: [(u32, u32); 12] = [
		(498, 280),
		(498, 498),
		(320, 240),
		(498, 372),
		(498, 210),
		(400, 500),
		(498, 280),
		(360, 360),
		(498, 320),
		(498, 260),
		(300, 420),
		(498, 280),
	];
	let needle = query.map(str::to_lowercase);
	let gifs = TITLES
		.iter()
		.enumerate()
		.filter(|(_, title)| {
			needle
				.as_deref()
				.is_none_or(|needle| title.to_lowercase().contains(needle) || needle.len() <= 3)
		})
		.map(|(index, title)| model::Gif {
			id: format!("synthetic-{index}"),
			title: (*title).to_owned(),
			url: format!(
				"https://tenor.com/view/synthetic-{index}-gif-{}",
				1000 + index
			),
			preview: format!("https://media.tenor.com/synthetic/{index}/tenor.png"),
			width: SIZES[index].0,
			height: SIZES[index].1,
		})
		.collect();
	model::GifPage {
		gifs,
		categories: if query.is_none() {
			[
				"Agree",
				"Applause",
				"Dance",
				"Excited",
				"Facepalm",
				"Hello",
				"No",
				"Thank you",
			]
			.into_iter()
			.map(str::to_owned)
			.collect()
		} else {
			Vec::new()
		},
	}
}

pub fn message(id: u64, channel: Id) -> Message {
	let mut content = match id % 6 {
		0 => "A short synthetic message.".into(),
		1 => "A longer synthetic message which wraps at smaller window sizes. ".repeat(8),
		2 => "Unicode: 日本語 · čeština · العربية · e\u{301} · 👩🏽‍💻".into(),
		3 => "```rust\nfn main() {\n    println!(\"synthetic fixture\");\n}\n```".into(),
		4 => "> A quoted thought\nA second line, and a third.\nThis stays only in session memory."
			.into(),
		_ => "**Synthetic history** — this is an offline fixture, never a Discord reply.".into(),
	};
	if id == 500 {
		content = format!("Hey <@2> — see <#21>. {content}");
	}
	Message {
		reactions: Some(if id == 500 {
			vec![Reaction {
				emoji: ReactionEmoji {
					id: None,
					name: Some("👍".into()),
				},
				count: 3,
				me: false,
				me_burst: false,
			}]
		} else {
			vec![]
		}),
		id: Id(id),
		channel,
		author: User {
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			id: Id(if id.is_multiple_of(2) { 1 } else { 2 }),
			primary_guild: (!id.is_multiple_of(2)).then(|| {
				Box::new(model::ClanTag {
					guild: Id(10),
					tag: "SPDY".into(),
					badge: Some("f".repeat(32)),
				})
			}),
			name: if id.is_multiple_of(2) {
				"You (synthetic)"
			} else {
				"Robin (synthetic)"
			}
			.into(),
		},
		content,
		prior_contents: Default::default(),
		edited: false,
		edited_at: None,
		revision: 0,
		nonce: None,
		reply_to: None,
		kind: 0,
		reply_deleted: false,
		forwarded: false,
		unsupported: false,
		components: vec![],
		sticker_items: vec![],
		application_id: None,
		flags: 0,
		ephemeral: false,
		extra_content: Default::default(),
		embeds: demo_embeds(id),
		attachments: if id == 500 {
			vec![
                Attachment {
                    duration_ms: None,
                    waveform: Vec::new(),
                    id: Id(700),
                    filename: "synthetic-landscape.png".into(),
                    description: Some("Original synthetic landscape · offline preview".into()),
                    content_type: Some("image/png".into()),
                    size: 2048,
                    spoiler: false,
                    media: EmbedMedia {
                        url: Some(
                            "https://cdn.discordapp.com/attachments/1/700/synthetic-landscape.png"
                                .into(),
                        ),
                        proxy_url: None,
                        width: 640,
                        height: 240,
                        ..Default::default()
                    },
                },
                Attachment {
                    duration_ms: None,
                    waveform: Vec::new(),
                    id: Id(702),
                    filename: "synthetic-second-landscape.png".into(),
                    description: Some("Second original landscape · offline gallery preview".into()),
                    content_type: Some("image/png".into()),
                    size: 2048,
                    spoiler: false,
                    media: EmbedMedia {
                        url: Some("https://cdn.discordapp.com/attachments/1/702/synthetic-second-landscape.png".into()),
                        proxy_url: None,
                        width: 480,
                        height: 320,
                        ..Default::default()
                    },
                },
                Attachment {
                    duration_ms: None,
                    waveform: Vec::new(),
                    id: Id(701),
                    filename: "synthetic-notes.txt".into(),
                    description: None,
                    content_type: Some("text/plain".into()),
                    size: 128,
                    spoiler: false,
                    media: EmbedMedia {
                        url: Some(
                            "https://cdn.discordapp.com/attachments/1/701/synthetic-notes.txt"
                                .into(),
                        ),
                        ..Default::default()
                    },
                },
            ]
		} else {
			vec![]
		},
		author_nick: None,
		author_roles: vec![],
		mention_roles: vec![],
		mention_everyone: false,
		suppress_notifications: false,
		mentions: if id == 500 {
			vec![User {
				id: Id(2),
				name: "Robin (synthetic)".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			}]
		} else {
			Vec::new()
		},
		embeds_suppressed: false,
	}
}
fn demo_embeds(id: u64) -> Vec<Embed> {
	if id != 500 {
		return vec![];
	}
	vec![Embed {
        kind: "rich".into(),
        title: Some("A quieter place for your conversations".into()),
        description: Some("**Native embed preview**\nFormatted descriptions, *useful details*, and a static image.\n[Markdown link](https://example.com/synthetic) · https://example.org".into()),
        url: Some("https://example.com/synthetic".into()),
        color: Some(0x68ada4),
        author: Some(EmbedAuthor { name: "Serein · synthetic example".into(), ..Default::default() }),
        fields: vec![EmbedField { name:"Interface".into(),value:"Rust + egui".into(),inline:true },EmbedField { name:"Preview".into(),value:"Offline only".into(),inline:true }],
        image: Some(EmbedMedia { url:Some("https://example.com/synthetic-image.png".into()),width:640,height:240,..Default::default() }),
        footer: Some(EmbedFooter { text:"Synthetic content · no service request".into(),..Default::default() }),
        ..Default::default()
    }, Embed {
        kind: "rich".into(),
        url: Some("https://example.com/synthetic".into()),
        image: Some(EmbedMedia { url: Some("https://example.com/synthetic-image-2.png".into()), width: 320, height: 320, ..Default::default() }),
        ..Default::default()
    }, Embed {
        kind: "rich".into(),
        url: Some("https://example.com/synthetic".into()),
        image: Some(EmbedMedia { url: Some("https://example.com/synthetic-image-3.png".into()), width: 320, height: 320, ..Default::default() }),
        ..Default::default()
    }]
}
pub fn demo_state() -> State {
	let mut state = State {
		demo: true,
		..State::default()
	};
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Ready {
			permissions: model::permissions::Snapshot::default(),
			user: User {
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				id: Id(1),
				name: "You (synthetic)".into(),
			},
			guilds: vec![Guild {
				stickers: None,
				emojis: Some(vec![
					model::CustomEmoji {
						id: Id(9001),
						name: "serein_wave".into(),
						animated: false,
						available: true,
						managed: false,
						roles: Some(vec![]),
					},
					model::CustomEmoji {
						id: Id(9002),
						name: "serein_party".into(),
						animated: true,
						available: true,
						managed: false,
						roles: Some(vec![]),
					},
				]),
				icon: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
				id: Id(10),
				name: "Synthetic workspace".into(),
			}],
			channels: vec![
				Channel {
					last_message: None,
					id: Id(20),
					guild: Some(Id(10)),
					parent_id: Some(Id(23)),
					position: 0,
					name: "getting-started".into(),
					kind: 0,
					recipients: vec![],
					icon: None,
					member_list_id: Some("everyone".into()),
					message_count: None,
				},
				Channel {
					last_message: None,
					id: Id(21),
					guild: Some(Id(10)),
					parent_id: Some(Id(24)),
					position: 0,
					name: "long-form".into(),
					kind: 0,
					recipients: vec![],
					icon: None,
					member_list_id: Some("everyone".into()),
					message_count: None,
				},
				Channel {
					last_message: None,
					id: Id(22),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Robin (synthetic)".into(),
					kind: 1,
					recipients: vec![message(1, Id(22)).author],
					icon: None,
					member_list_id: None,
					message_count: None,
				},
				Channel {
					last_message: None,
					id: Id(23),
					guild: Some(Id(10)),
					parent_id: None,
					position: 0,
					name: "WELCOME".into(),
					kind: 4,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
				},
				Channel {
					id: Id(40),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Message request (synthetic)".into(),
					kind: 1,
					recipients: vec![User {
						id: Id(8004),
						name: "Rowan (synthetic)".into(),
						avatar: None,
						webhook: false,
						kind: Default::default(),
						discriminator: 0,
						primary_guild: None,
					}],
					last_message: None,
					icon: None,
					member_list_id: None,
					message_count: None,
				},
				Channel {
					id: Id(43),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Spam request (synthetic)".into(),
					kind: 1,
					recipients: vec![User {
						id: Id(8005),
						name: "Spam (synthetic)".into(),
						avatar: None,
						webhook: false,
						kind: Default::default(),
						discriminator: 0,
						primary_guild: None,
					}],
					last_message: Some(Id(900)),
					icon: None,
					member_list_id: None,
					message_count: None,
				},
				Channel {
					id: Id(29),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Weekend plans (synthetic)".into(),
					kind: 3,
					recipients: vec![
						message(1, Id(29)).author,
						User {
							id: Id(3),
							name: "Casey (synthetic)".into(),
							avatar: None,
							webhook: false,
							kind: Default::default(),
							discriminator: 0,
							primary_guild: None,
						},
					],
					last_message: None,
					icon: None,
					member_list_id: None,
					message_count: None,
				},
				Channel {
					last_message: None,
					id: Id(24),
					guild: Some(Id(10)),
					parent_id: None,
					position: 1,
					name: "CONVERSATIONS".into(),
					kind: 4,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
				},
				Channel {
					last_message: None,
					id: Id(25),
					guild: Some(Id(10)),
					parent_id: Some(Id(24)),
					position: 1,
					name: "hangout".into(),
					kind: 2,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
				},
				Channel {
					id: Id(26),
					guild: Some(Id(10)),
					parent_id: Some(Id(24)),
					position: 2,
					name: "ideas".into(),
					kind: 15,
					recipients: vec![],
					last_message: None,
					icon: None,
					member_list_id: None,
					message_count: None,
				},
				Channel {
					id: Id(27),
					guild: Some(Id(10)),
					parent_id: Some(Id(26)),
					position: 0,
					name: "A synthetic forum post".into(),
					kind: 11,
					recipients: vec![],
					last_message: Some(Id(1_542_322_755_993_600_000)),
					icon: None,
					member_list_id: None,
					message_count: Some(10),
				},
				Channel {
					id: Id(41),
					guild: Some(Id(10)),
					parent_id: Some(Id(26)),
					position: 0,
					name: "Automatic model retraining on app data".into(),
					kind: 11,
					recipients: vec![],
					last_message: Some(Id(1_546_671_410_380_800_000)),
					icon: None,
					member_list_id: None,
					message_count: Some(0),
				},
				Channel {
					id: Id(42),
					guild: Some(Id(10)),
					parent_id: Some(Id(26)),
					position: 0,
					name: "Different model weights for differently powerful phones".into(),
					kind: 11,
					recipients: vec![],
					last_message: Some(Id(1_547_722_335_191_040_000)),
					icon: None,
					member_list_id: None,
					message_count: Some(6),
				},
				Channel {
					id: Id(28),
					guild: Some(Id(10)),
					parent_id: Some(Id(20)),
					position: 0,
					name: "Introductions thread".into(),
					kind: 11,
					recipients: vec![],
					last_message: Some(Id(1_547_722_335_191_040_000)),
					icon: None,
					member_list_id: None,
					message_count: Some(4),
				},
			],
		},
	});
	state
		.permissions
		.replace(permission_snapshot(&state))
		.unwrap();
	state
		.apply_notification_preferences(client_core::notifications::Event::Settings {
			entries: vec![client_core::notifications::Setting {
				guild: Some(Id(10)),
				muted: Some(false),
				level: Some(3),
				suppress_everyone: Some(false),
				suppress_roles: Some(false),
				hide_muted_channels: None,
				channels: vec![(Id(21), Some(true), Some(3))],
				channel_mute_until: vec![],
			}],
			replace: true,
		})
		.unwrap();
	state.select(Id(20));
	load_page(&mut state, None);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::ReadState(client_core::read_state::Event::Snapshot {
			partial: false,
			entries: Some(vec![(Id(20), Some(Id(495)), 0)]),
			version: Some(1),
		}),
	});
	state.apply(Envelope {
		generation: state.generation,
		event: Event::UserAction(client_core::user_actions::Event::Requests(Some(vec![
			(
				User {
					id: Id(8001),
					name: "Avery".into(),
					avatar: None,
					discriminator: 0,
					primary_guild: None,
					webhook: false,
					kind: Default::default(),
				},
				"avery.synthetic".into(),
				true,
			),
			(
				User {
					id: Id(8003),
					name: "Rowan".into(),
					avatar: None,
					discriminator: 0,
					primary_guild: None,
					webhook: false,
					kind: Default::default(),
				},
				"rowan.synthetic".into(),
				true,
			),
		]))),
	});
	state.apply(Envelope {
		generation: state.generation,
		event: Event::UserAction(client_core::user_actions::Event::MessageRequests(Some(
			vec![Id(40)],
		))),
	});
	state.apply(Envelope {
		generation: state.generation,
		event: Event::UserAction(client_core::user_actions::Event::MessageSpams(Some(vec![
			Id(43),
		]))),
	});
	state.apply(Envelope {
		generation: state.generation,
		event: Event::UserAction(client_core::user_actions::Event::Friends(Some(
			[
				"Robin", "Casey", "Morgan", "Alex", "Sam", "Taylor", "Jamie", "Jordan", "Avery",
				"Quinn", "Riley", "Skyler", "Cameron", "Drew", "Reese", "Parker",
			]
			.into_iter()
			.enumerate()
			.map(|(i, name)| {
				(
					User {
						id: Id(1001 + i as u64),
						name: name.into(),
						avatar: None,
						webhook: false,
						kind: Default::default(),
						discriminator: 0,
						primary_guild: None,
					},
					format!("{}.synthetic", name.to_lowercase()),
				)
			})
			.collect(),
		))),
	});
	state.status = "Offline fixture · no network access";
	state
}
/// Notification rail evidence: synthetic incoming DMs and a guild mention, no OS delivery.
pub fn notification_demo_state() -> State {
	let mut state = demo_state();
	let dm = state
		.channels
		.iter()
		.find(|c| c.guild.is_none() && c.supports_text())
		.unwrap()
		.id;
	let guild = state
		.channels
		.iter()
		.find(|c| c.guild.is_some() && c.supports_text() && Some(c.id) != state.selected)
		.unwrap()
		.id;
	for (id, channel) in [(1001, dm), (1003, dm), (1005, guild)] {
		let mut message = message(id, channel);
		message.mentions = vec![state.user.clone().unwrap()];
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message),
		});
	}
	state.status = "Offline notification fixture · no network or OS alerts";
	state
}
/// Voice-only visual evidence: synthetic membership, no media or gateway commands.
pub fn voice_demo_state() -> State {
	use client_core::voice::{Call, Participant, Phase, RosterEntry};
	use std::time::{Duration, Instant};
	let mut state = demo_state();
	state
		.channels
		.iter_mut()
		.find(|c| c.id == Id(25))
		.unwrap()
		.name = "Room 3,5".into();
	state.channels.push(Channel {
		id: Id(26),
		guild: Some(Id(10)),
		parent_id: Some(Id(24)),
		position: 2,
		name: "Quiet room".into(),
		kind: 2,
		recipients: vec![],
		icon: None,
		member_list_id: None,
		message_count: None,
		last_message: None,
	});
	state.voice.roster = [
		(1, "You (synthetic)", false, false),
		(2, "Robin with a rather long display name", true, true),
		(3, "Fern and the midnight orchestra", true, false),
	]
	.into_iter()
	.map(|(id, name, muted, deafened)| RosterEntry {
		guild: Id(10),
		channel: Id(25),
		participant: Participant {
			user: Id(id),
			muted,
			deafened,
			server_muted: false,
			server_deafened: false,
			video: false,
			streaming: false,
		},
		member: Some(Member {
			roles: vec![],
			user: User {
				id: Id(id),
				name: name.into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			},
			nick: None,
			status: None,
			custom_status: None,
			activities: vec![],
		}),
	})
	.collect();
	// Fern streams in the fixture so the LIVE pill and watch action render offline.
	state.voice.roster[2].participant.streaming = true;
	state.voice.active = Some(Call {
		camera: false,
		watching: None,
		channel: Id(25),
		guild: Some(Id(10)),
		request: 0,
		phase: Phase::Connected,
		connected_at: Some(Instant::now() - Duration::from_secs(3663)),
		muted: false,
		deafened: false,
		server_muted: false,
		server_deafened: false,
		participants: state
			.voice
			.roster
			.iter()
			.map(|entry| entry.participant)
			.collect(),
		error: None,
	});
	state.select(Id(25));
	load_page(&mut state, None);
	state.status = "Offline voice fixture · no microphone or network access";
	state
}
/// Synthetic direct-message call: two participants, connected for a while. Never live audio.
pub fn call_demo_state() -> State {
	use client_core::voice::{Call, Participant, Phase};
	use std::time::{Duration, Instant};
	let mut state = demo_state();
	let _ = state.select(Id(22));
	load_page(&mut state, None);
	let participant = |user, muted| Participant {
		user: Id(user),
		muted,
		deafened: false,
		server_muted: false,
		server_deafened: false,
		video: false,
		streaming: false,
	};
	state.voice.active = Some(Call {
		camera: false,
		watching: None,
		channel: Id(22),
		guild: None,
		request: 0,
		phase: Phase::Connected,
		connected_at: Some(Instant::now() - Duration::from_secs(754)),
		muted: false,
		deafened: false,
		server_muted: false,
		server_deafened: false,
		participants: vec![participant(1, false), participant(2, true)],
		error: None,
	});
	state.status = "Offline call fixture · no microphone or network access";
	state
}
/// Synthetic ongoing DM call on another client; this device has never joined.
pub fn existing_call_demo_state() -> State {
	let mut state = demo_state();
	let _ = state.select(Id(22));
	load_page(&mut state, None);
	state.demo = false;
	state.apply_voice(client_core::voice::Event::Call {
		channel: Id(22),
		ringing: Some(vec![]),
		participants: None,
		unavailable: false,
	});
	state.demo = true;
	state.status = "Offline call fixture · no microphone or network access";
	state
}
pub fn load_page(state: &mut State, before: Option<Id>) {
	load_page_with_cursors(state, before, None);
}
pub fn load_page_with_cursors(state: &mut State, before: Option<Id>, after: Option<Id>) {
	assert!(before.is_none() || after.is_none());
	let channel = state.selected.unwrap();
	let latest = state
		.channels
		.iter()
		.find(|c| c.id == channel)
		.and_then(|c| c.last_message)
		.map_or(500, |id| id.0.max(500));
	let (start, end) = if let Some(after) = after {
		let start = after.0.saturating_add(1);
		(
			start,
			start.saturating_add(50).min(latest.saturating_add(1)),
		)
	} else {
		let end = before.map_or_else(|| latest.saturating_add(1), |id| id.0);
		(end.saturating_sub(50).max(1), end)
	};
	state.apply(Envelope {
		generation: state.generation,
		event: Event::History {
			channel,
			request: state.request,
			older: before.is_some(),
			messages: (start..end).map(|id| message(id, channel)).collect(),
		},
	});
}
/// A confirmed empty guild channel, using the ordinary history response path.
pub fn empty_channel_demo_state(long_name: bool) -> State {
	let mut state = demo_state();
	let channel = state.selected.unwrap();
	if long_name {
		state.channels.iter_mut().find(|c| c.id == channel).unwrap().name =
			"🌙-a-place-for-project-updates-and-the-little-things-that-make-a-community-feel-like-home".into();
	}
	let _ = state.history(None);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::History {
			channel,
			request: state.request,
			older: false,
			messages: vec![],
		},
	});
	state
}

/// Synthetic switcher roster for offline captures; these accounts never exist on Discord.
pub fn demo_accounts(current: &model::User) -> Vec<model::SavedAccount> {
	vec![
		model::SavedAccount {
			id: current.id,
			name: current.name.clone(),
			display: Some("Riley Quinn".into()),
			avatar: current.avatar.clone(),
			discriminator: current.discriminator,
			has_token: true,
		},
		model::SavedAccount {
			id: Id(4242),
			name: "riley.alt".into(),
			display: Some("Riley (alt)".into()),
			avatar: None,
			discriminator: 0,
			has_token: true,
		},
		model::SavedAccount {
			id: Id(4243),
			name: "serein.testing".into(),
			display: None,
			avatar: None,
			discriminator: 0,
			has_token: true,
		},
	]
}

pub fn seed_access_marks(state: &mut State) {
	use model::permissions::{Overwrite, Role, VIEW_CHANNEL};
	const GUILD: Id = Id(10);
	const STAFF: Id = Id(11);
	const ACCESS: Id = Id(60);
	let channel = |id, kind, parent, position, name: &str| Channel {
		last_message: None,
		id: Id(id),
		guild: Some(GUILD),
		parent_id: parent,
		position,
		name: name.into(),
		kind,
		recipients: vec![],
		icon: None,
		member_list_id: None,
		message_count: None,
	};
	state.channels.extend([
		channel(60, 4, None, 2, "ACCESS"),
		channel(61, 0, Some(ACCESS), 0, "staff-notes"),
		channel(62, 0, Some(ACCESS), 1, "secret"),
		channel(63, 2, Some(ACCESS), 2, "locked-hangout"),
		channel(64, 2, Some(ACCESS), 3, "vault"),
		channel(65, 0, Some(ACCESS), 4, "unknown-room"),
	]);
	let mut snapshot = permission_snapshot(state);
	if let Some(guild) = snapshot.guilds.iter_mut().find(|guild| guild.id == GUILD) {
		if let Some(roles) = guild.roles.as_mut() {
			roles.push(Role {
				id: STAFF,
				bits: 0,
				name: "Contributors".into(),
				color: 0,
				position: 1,
				hoist: false,
			});
		}
		if let Some(member) = guild.member.as_mut() {
			member.roles.push(STAFF);
		}
	}
	let deny_everyone = Overwrite {
		id: GUILD,
		kind: 0,
		allow: 0,
		deny: VIEW_CHANNEL,
	};
	let allow_staff = Overwrite {
		id: STAFF,
		kind: 0,
		allow: VIEW_CHANNEL,
		deny: 0,
	};
	let deny_member = Overwrite {
		id: state.user.as_ref().map_or(Id(1), |user| user.id),
		kind: 1,
		allow: 0,
		deny: VIEW_CHANNEL,
	};
	for channel in &mut snapshot.channels {
		match channel.id.0 {
			61 | 63 => channel.overwrites = Some(vec![deny_everyone, allow_staff]),
			62 => channel.overwrites = Some(vec![deny_everyone]),
			64 => channel.overwrites = Some(vec![deny_member]),
			_ => {}
		}
	}
	snapshot.channels.retain(|channel| channel.id != Id(65));
	state.permissions.replace(snapshot).unwrap();
	state
		.apply_notification_preferences(client_core::notifications::Event::Settings {
			entries: vec![client_core::notifications::Setting {
				guild: Some(GUILD),
				muted: Some(false),
				level: Some(3),
				suppress_everyone: Some(false),
				suppress_roles: Some(false),
				hide_muted_channels: None,
				channels: vec![(Id(21), Some(true), Some(3)), (Id(25), Some(true), Some(3))],
				channel_mute_until: vec![],
			}],
			replace: true,
		})
		.unwrap();
	state.revision += 1;
}
pub fn seed_demo_folder_mosaic(state: &mut State) {
	const EXTRA: [(u64, &str, &str); 4] = [
		(11, "North lab", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
		(12, "Ops desk", "cccccccccccccccccccccccccccccccc"),
		(13, "Night shift", "dddddddddddddddddddddddddddddddd"),
		(14, "Archive", "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
	];
	for (id, name, hash) in EXTRA {
		state.guilds.push(Guild {
			stickers: None,
			emojis: None,
			id: Id(id),
			name: name.into(),
			icon: Some(hash.into()),
		});
	}
	state.guild_folders = Some(model::guild_folders::Settings {
		folders: vec![
			model::guild_folders::Folder {
				id: Some(1),
				guild_ids: vec![Id(11), Id(12), Id(13), Id(14)],
				name: Some("Synthetic folder".into()),
				color: Some(0x5865f2),
			},
			model::guild_folders::Folder {
				id: None,
				guild_ids: vec![Id(10)],
				name: None,
				color: None,
			},
		],
		version: 0,
	});
}

/// Additional native chat scenario: fixed dates, grouped authors and unread events.
pub fn chat_demo_state() -> State {
	let mut state = demo_state();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Permissions(client_core::permissions::Event::Role {
			guild: Id(10),
			role: model::permissions::Role {
				id: Id(101),
				name: "Synthetic colored role".into(),
				bits: 0,
				color: 0x68ada4,
				position: 1,
				hoist: false,
			},
		}),
	});
	state.timeline.clear();
	state.older_exhausted = true;
	let texts = [
		"Can we keep the conversation simple?",
		"Names and times, with room for the messages.",
		"And keep my place when earlier history arrives.",
		"Yes. History loads in small pages as you scroll up.",
		"Only nearby messages are rendered. The cache has a fixed memory budget.",
		"A new day, same conversation.",
		"Hey <@2> — see <#21>. This looks much easier to read.",
		"Two new messages arrived while you were away.",
		"Welcome back. All of this is synthetic, offline data.",
	];
	for (i, text) in texts.iter().enumerate() {
		let mut m = message(i as u64 + 1, Id(20));
		// September 9, 2026, 23:55 UTC, one minute between records.
		m.id = Id(((1_788_998_100_000u64 + i as u64 * 60_000 - 1_420_070_400_000) << 22) | 1);
		m.author = message(if !(3..7).contains(&i) { 1 } else { 2 }, Id(20)).author;
		m.content = (*text).into();
		m.author_roles = vec![Id(101)];
		if i == 6 {
			m.mentions = vec![User {
				id: Id(2),
				name: "𝖘𝖓𝖎𝖎𝖝. (synthetic)".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			}];
		}
		if i == 8 {
			m.reply_to = state.timeline.iter().nth(6).map(|original| original.id);
		}
		if i < 7 {
			state.timeline.insert(m, false, false).unwrap();
		} else {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Message(m),
			});
		}
	}
	let read = state.timeline.iter().nth(6).unwrap().id;
	state.apply(Envelope {
		generation: state.generation,
		event: Event::ReadState(client_core::read_state::Event::Ack {
			channel: Id(20),
			message: Some(read),
			manual: true,
			mention_count: None,
			version: Some(2),
		}),
	});
	state.revision += 1;
	state
}
/// Local audio card scenario. Playback generates a quiet tone; never fetches this URL.
pub fn forwarded_demo_state() -> State {
	let mut state = audio_demo_state();
	let messages: Vec<_> = state.timeline.iter().cloned().collect();
	state.timeline.clear();
	for mut message in messages {
		message.forwarded = true;
		message.content.clear();
		state.timeline.insert(message, false, false).unwrap();
	}
	state
}
pub fn audio_demo_state() -> State {
	let mut state = chat_demo_state();
	state.timeline.clear();
	for (id, filename, kind) in [
		(499, "voice-message.ogg", "audio/ogg"),
		(501, "synthetic-melody.wav", "audio/wav"),
		(
			502,
			"a-long-synthetic-audio-filename-for-layout-checks.mp3",
			"audio/mpeg",
		),
	] {
		let mut message = message(id, Id(20));
		message.content =
			"Synthetic audio attachment · click Play to preview a locally generated tone.".into();
		message.embeds.clear();
		message.attachments = vec![Attachment {
			duration_ms: (id == 499).then_some(3000),
			waveform: if id == 499 {
				(0..64)
					.map(|i| (40.0 + 180.0 * (i as f32 * 0.35).sin().abs()) as u8)
					.collect()
			} else {
				Vec::new()
			},
			id: Id(id + 200),
			filename: filename.into(),
			description: None,
			content_type: Some(kind.into()),
			size: 529244,
			spoiler: false,
			media: EmbedMedia {
				url: Some(format!(
					"https://cdn.discordapp.com/attachments/20/{}/{filename}",
					id + 200
				)),
				..Default::default()
			},
		}];
		state.timeline.insert(message, false, false).unwrap();
	}
	state.revision += 1;
	state
}
/// Explicit synthetic permissions, separate from the production unknown-metadata path.
pub fn permission_snapshot(state: &State) -> model::permissions::Snapshot {
	use model::permissions as p;
	let bits = p::VIEW_CHANNEL
		| 1 // CREATE_INSTANT_INVITE, synthetic demo only.
		| p::READ_MESSAGE_HISTORY
		| p::SEND_MESSAGES
		| p::SEND_MESSAGES_IN_THREADS
		| p::ATTACH_FILES
		| p::ADD_REACTIONS
		| p::CONNECT
		| p::SPEAK
		| p::USE_VAD
		| p::MANAGE_THREADS
		| p::CREATE_PUBLIC_THREADS
		| p::MANAGE_CHANNELS;
	p::Snapshot {
		guilds: state
			.guilds
			.iter()
			.map(|guild| p::Guild {
				id: guild.id,
				owner: Some(Id(u64::MAX)),
				roles: Some(vec![p::Role {
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
					id: guild.id,
					bits,
				}]),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
			})
			.collect(),
		channels: state
			.channels
			.iter()
			.filter_map(|channel| {
				channel.guild.map(|guild| p::Channel {
					id: channel.id,
					guild,
					overwrites: Some(vec![]),
				})
			})
			.collect(),
	}
}

/// Synthetic system events; never live Discord history.
pub fn system_demo_state() -> State {
	let mut state = chat_demo_state();
	state.timeline.clear();
	for (i, (kind, content)) in [
		(7, ""),
		(1, ""),
		(6, ""),
		(9, ""),
		(4, "welcome-and-updates"),
		(3, ""),
		(67, ""),
		(30, ""),
		(55, ""),
		(58, ""),
		(59, ""),
		(60, ""),
		(61, ""),
		(62, ""),
		(65, ""),
		(18, "Introductions thread"),
		(222, ""),
	]
	.into_iter()
	.enumerate()
	{
		let mut m = message(i as u64 + 1, Id(20));
		m.id = Id(((1_788_998_100_000u64 + i as u64 * 60_000 - 1_420_070_400_000) << 22) | 1);
		m.kind = kind;
		m.unsupported = true;
		m.content = content.into();
		m.embeds.clear();
		m.attachments.clear();
		m.reactions = Some(vec![]);
		m.mentions = vec![User {
			id: Id(42),
			name: "Casey (synthetic)".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		}];
		state.timeline.insert(m, false, false).unwrap();
	}
	state.revision += 1;
	state
}

/// Fenced code blocks in every Discord shape: language header, one-line fence, bare fence,
/// unknown language and a closing fence at the end of the last content line.
pub fn code_demo_state() -> State {
	let mut state = chat_demo_state();
	state.timeline.clear();
	let texts = [
		"One-liner: ```cargo xtask check``` and an unknown tag:\n```elixir\nIO.puts \"synthetic\"\n```",
		"```json\n{\"name\": \"serein\", \"version\": 1, \"voice\": true, \"tags\": [\"native\", null]}\n```",
		"Here is the reducer entry point:\n```rust\n/// Apply one gateway event.\npub fn apply(&mut self, event: Event) -> Result<(), Error> {\n    let Some(channel) = self.channels.get_mut(&event.channel) else {\n        return Err(Error::Unknown(event.channel));\n    };\n    channel.push(event.message, MAX_MESSAGES)?; // bounded\n    Ok(())\n}\n```\nThe cache stays bounded by bytes and items.",
		"```js\nconst rows = await db.query(\"select id from users where active = $1\", [true]);\nconsole.log(`${rows.length} active`); // synthetic\n```",
	];
	for (i, text) in texts.iter().enumerate() {
		let mut m = message(i as u64 + 1, Id(20));
		m.id = Id(((1_788_998_100_000u64 + i as u64 * 60_000 - 1_420_070_400_000) << 22) | 1);
		m.author = message(if i % 2 == 0 { 1 } else { 2 }, Id(20)).author;
		m.content = (*text).into();
		m.embeds.clear();
		m.attachments.clear();
		m.reactions = Some(vec![]);
		state.timeline.insert(m, false, false).unwrap();
	}
	state.revision += 1;
	state
}

/// Offline video attachment. Desktop substitutes its generated MOV fixture only in --demo.
pub fn video_demo_state() -> State {
	let mut state = chat_demo_state();
	state.timeline.clear();
	let mut message = message(601, Id(20));
	message.content =
		"Synthetic video attachment: three seconds of animated test colors and a tone.".into();
	message.embeds.clear();
	message.attachments = vec![Attachment {
		id: Id(801),
		filename: "synthetic-video.mov".into(),
		description: None,
		content_type: Some("video/quicktime".into()),
		size: 120000,
		duration_ms: Some(3000),
		waveform: Vec::new(),
		spoiler: false,
		media: EmbedMedia {
			url: Some("https://cdn.discordapp.com/attachments/20/801/synthetic-video.mov".into()),
			width: 320,
			height: 180,
			..Default::default()
		},
	}];
	state.timeline.insert(message, false, false).unwrap();
	state.revision += 1;
	state
}

/// Offline friends overview using existing synthetic relationship data.
pub fn friends_demo_state() -> State {
	let mut state = demo_state();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::UserAction(client_core::user_actions::Event::Restrictions(Some(vec![
			(
				User {
					id: Id(8101),
					name: "Blocked Example".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				},
				"blocked.synthetic".into(),
				false,
			),
			(
				User {
					id: Id(8102),
					name: "Ignored Example".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				},
				"ignored.synthetic".into(),
				true,
			),
		]))),
	});
	state.apply(Envelope {
		generation: state.generation,
		event: Event::UserAction(client_core::user_actions::Event::Requests(Some(vec![
			(
				model::User {
					id: Id(8001),
					name: "Avery".into(),
					avatar: None,
					discriminator: 0,
					primary_guild: None,
					webhook: false,
					kind: Default::default(),
				},
				"avery.synthetic".into(),
				true,
			),
			(
				model::User {
					id: Id(8002),
					name: "Morgan".into(),
					avatar: None,
					discriminator: 0,
					primary_guild: None,
					webhook: false,
					kind: Default::default(),
				},
				"morgan.synthetic".into(),
				false,
			),
		]))),
	});
	state.selected = None;
	state.apply(Envelope {
		generation: state.generation,
		event: Event::DirectPresence(
			(0..16)
				.map(|i| client_core::presence::Update {
					user: Id(1001 + i),
					status: model::Patch::Value(if i < 7 { "online" } else { "offline" }.into()),
					custom_status: if i == 0 {
						model::Patch::Value("Building something fun".into())
					} else {
						model::Patch::Null
					},
					activities: model::Patch::Value(Vec::new()),
				})
				.collect(),
		),
	});
	state
}

/// Original offline sticker catalog and one received message, used only by the sticker preview.
pub fn seed_stickers(state: &mut State) {
	let sticker = |id, name: &str, guild_id, pack_id| model::Sticker {
		id: Id(id),
		name: name.into(),
		description: "Original synthetic sticker artwork".into(),
		tags: "hello,wave,smile".into(),
		format_type: 1,
		guild_id,
		pack_id,
		available: true,
	};
	let guild_stickers = vec![
		sticker(9101, "Wave", Some(Id(10)), None),
		sticker(9102, "Smile", Some(Id(10)), None),
		sticker(9103, "Celebrate", Some(Id(10)), None),
	];
	if let Some(guild) = state.guilds.iter_mut().find(|guild| guild.id == Id(10)) {
		guild.stickers = Some(guild_stickers.clone());
	}
	state.stickers.recent = vec![guild_stickers[0].clone()];
	state.stickers.packs = vec![model::StickerPack {
		id: Id(9200),
		name: "Serein Friends (synthetic)".into(),
		stickers: vec![
			sticker(9201, "Sleep", None, Some(Id(9200))),
			sticker(9202, "Hello", None, Some(Id(9200))),
			sticker(9203, "Party", None, Some(Id(9200))),
		],
	}];
	state.stickers.loaded = true;
	if let Some(channel) = state.selected {
		state.timeline.clear();
		let mut message = message(501, channel);
		message.content.clear();
		message.embeds.clear();
		message.attachments.clear();
		message.sticker_items = vec![state.stickers.packs[0].stickers[0].clone()];
		message.extra_content.sticker_items = true;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message),
		});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn existing_dm_calls_survive_ringing_hangup_and_resume_but_not_deletion_or_logout() {
		use client_core::voice::{Command as V, Event as E, MAX_DM_CALLS};
		let mut state = existing_call_demo_state();
		state.demo = false;
		let call = |channel, ringing, unavailable| E::Call {
			channel,
			ringing,
			participants: None,
			unavailable,
		};
		assert!(state.voice.has_dm_call(Id(22)));
		assert!(state.voice.active.is_none());
		assert!(state.voice.incoming.is_none());
		state.apply_voice(call(Id(22), Some(vec![Id(1)]), false));
		assert!(state.decline_call().is_some());
		state.apply_voice(call(Id(22), Some(vec![]), false));
		assert!(state.voice.has_dm_call(Id(22)));
		assert!(state.voice.incoming.is_none());
		assert!(matches!(
			state.start_call(Id(22), true),
			Some(client_core::Command::Voice(V::Join { ring: false, .. }))
		));
		state.leave_call();
		assert!(state.voice.has_dm_call(Id(22)));
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Disconnected,
		});
		assert!(state.voice.has_dm_call(Id(22)));
		assert!(state.start_call(Id(22), false).is_none());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Resumed,
		});
		assert!(state.voice.has_dm_call(Id(22)));
		state.apply(Envelope {
			generation: state.generation + 1,
			event: Event::Voice(E::Deleted { channel: Id(22) }),
		});
		assert!(state.voice.has_dm_call(Id(22)));
		state.apply_voice(E::Deleted { channel: Id(22) });
		assert!(!state.voice.has_dm_call(Id(22)));
		assert!(matches!(
			state.start_call(Id(22), true),
			Some(client_core::Command::Voice(V::Join { ring: true, .. }))
		));
		state.leave_call();
		state.apply_voice(call(Id(22), None, false));
		state.apply_voice(call(Id(22), None, true));
		assert!(!state.voice.has_dm_call(Id(22)));
		let template = state
			.channels
			.iter()
			.find(|c| c.id == Id(22))
			.unwrap()
			.clone();
		for id in 100..100 + MAX_DM_CALLS as u64 + 1 {
			state.channels.push(model::Channel {
				id: Id(id),
				..template.clone()
			});
			state.apply_voice(call(Id(id), None, false));
		}
		assert!(!state.voice.has_dm_call(Id(100)));
		assert_eq!(
			state
				.channels
				.iter()
				.filter(|c| state.voice.has_dm_call(c.id))
				.count(),
			MAX_DM_CALLS
		);
		// Duplicate partial updates consume no extra slot; invalid channels cannot establish calls.
		state.apply_voice(call(Id(101), None, false));
		state.apply_voice(call(Id(25), None, false));
		state.apply_voice(call(Id(9999), None, false));
		assert!(!state.voice.has_dm_call(Id(25)));
		assert!(!state.voice.has_dm_call(Id(9999)));
		state.apply(Envelope {
			generation: state.generation,
			event: Event::RecipientRemoved {
				channel: Id(101),
				user: Id(1),
			},
		});
		assert!(!state.voice.has_dm_call(Id(101)));
		state.logout();
		assert!(
			state
				.channels
				.iter()
				.all(|c| !state.voice.has_dm_call(c.id))
		);
		assert!(!state.voice.has_dm_call(Id(102)));
	}
	#[test]
	fn unread_demo_pages_start_after_marker_and_advance_without_latest_substitution() {
		use client_core::Command;
		for (marker, first, last) in [
			(None, 1, 50),
			(Some(Id(10)), 11, 60),
			(Some(Id(490)), 491, 500),
		] {
			let mut state = demo_state();
			state
				.apply_read_state(client_core::read_state::Event::Snapshot {
					entries: Some(vec![(Id(20), marker, 0)]),
					partial: false,
					version: Some(2),
				})
				.unwrap();
			let Some(Command::History { before, after, .. }) = state.open_unread() else {
				panic!()
			};
			assert_eq!(before, None);
			assert_eq!(after, Some(marker.unwrap_or(Id(0))));
			load_page_with_cursors(&mut state, before, after);
			assert_eq!(state.timeline.row_ids().next(), Some(Id(first)));
			assert_eq!(state.timeline.row_ids().last(), Some(Id(last)));
			assert_eq!(state.search_target, Some(Id(first)));
			assert_eq!(state.read_marker(Id(20)), Some(marker));
			if last < 500 {
				let Some(Command::History { before, after, .. }) = state.newer_history() else {
					panic!()
				};
				assert_eq!(after, Some(Id(last)));
				load_page_with_cursors(&mut state, before, after);
				assert_eq!(state.timeline.row_ids().next(), Some(Id(first)));
				assert_eq!(state.timeline.row_ids().last(), Some(Id(last + 50)));
				assert!(state.search_target.is_none());
			} else {
				assert!(state.newer_history().is_none());
			}
		}
	}
	#[test]
	fn notification_preview_history_reaches_latest_and_clears_viewed_badge() {
		let mut state = notification_demo_state();
		let dm = state
			.channels
			.iter()
			.find(|c| c.guild.is_none() && c.supports_text())
			.unwrap()
			.id;
		assert_eq!(state.unread_count(dm), 2);
		state.select(dm);
		load_page(&mut state, None);
		let latest = state.timeline.iter().last().unwrap().id;
		assert_eq!(latest, Id(1003));
		let client_core::Command::MarkRead {
			channel,
			message,
			request,
			..
		} = state.prepare_mark_read(latest).unwrap()
		else {
			panic!()
		};
		state
			.apply_read_state(client_core::read_state::Event::Result {
				channel,
				message,
				request,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(state.unread_count(dm), 0);
		assert_eq!(state.unread(dm), Some(false));
	}
	#[test]
	fn notification_activity_deduplicates_and_preserves_partial_ack() {
		use client_core::{notifications as n, read_state as r};
		let mut state = demo_state();
		let channel = state
			.channels
			.iter()
			.find(|c| c.guild.is_none() && c.supports_text())
			.unwrap()
			.id;
		state
			.apply_notification_preferences(n::Event::Settings {
				entries: vec![n::Setting {
					channel_mute_until: vec![],
					guild: None,
					muted: Some(false),
					suppress_everyone: Some(false),
					suppress_roles: Some(false),
					hide_muted_channels: None,
					level: Some(0),
					channels: vec![],
				}],
				replace: true,
			})
			.unwrap();
		state
			.apply_notification_preferences(n::Event::Presence(Some(false)))
			.unwrap();
		for id in [1001, 1001, 1000, 1003] {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Message(message(id, channel)),
			});
		}
		assert_eq!(state.unread_count(channel), 2);
		assert_eq!(state.mention_count(channel), 2);
		assert_eq!(state.take_notification().unwrap().message, Id(1001));
		assert_eq!(state.take_notification().unwrap().message, Id(1003));
		assert!(state.take_notification().is_none());
		state
			.apply_read_state(r::Event::Ack {
				channel,
				message: Some(Id(1001)),
				manual: false,
				mention_count: None,
				version: Some(2),
			})
			.unwrap();
		assert_eq!(state.unread_count(channel), 1);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(1001, channel)),
		});
		assert_eq!(state.unread_count(channel), 1);
		assert!(state.take_notification().is_none());
		state
			.apply_read_state(r::Event::Ack {
				channel,
				message: Some(Id(1003)),
				manual: false,
				mention_count: Some(0),
				version: Some(3),
			})
			.unwrap();
		assert_eq!(state.unread_count(channel), 0);
		state
			.apply_read_state(r::Event::Ack {
				channel,
				message: None,
				manual: true,
				mention_count: Some(7),
				version: Some(4),
			})
			.unwrap();
		assert_eq!(state.mention_count(channel), 7);
		state
			.apply_read_state(r::Event::Ack {
				channel,
				message: Some(Id(1003)),
				manual: false,
				mention_count: Some(0),
				version: Some(3),
			})
			.unwrap();
		assert_eq!(state.mention_count(channel), 7);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(1004, channel)),
		});
		assert_eq!(state.unread_count(channel), 0);
		assert_eq!(state.unread(channel), Some(false));
		assert!(state.take_notification().is_none());
		let mut new_dm = state
			.channels
			.iter()
			.find(|c| c.id == channel)
			.unwrap()
			.clone();
		new_dm.id = Id(909);
		new_dm.last_message = Some(Id(2001));
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ChannelCreated(new_dm),
		});
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(2001, Id(909))),
		});
		assert_eq!(state.unread_count(Id(909)), 1);
		assert_eq!(state.take_notification().unwrap().message, Id(2001));
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Unavailable(channel),
		});
		assert_eq!(state.unread_count(channel), 0);
		assert!(state.take_notification().is_none());
	}
	#[test]
	fn guild_notification_levels_and_category_mutes_are_respected() {
		use client_core::notifications as n;
		let mut state = demo_state();
		let channel = state
			.channels
			.iter()
			.find(|c| c.guild.is_some() && c.supports_text())
			.unwrap()
			.clone();
		let setting = n::Setting {
			channel_mute_until: vec![],
			guild: channel.guild,
			muted: Some(false),
			suppress_everyone: Some(false),
			suppress_roles: Some(false),
			hide_muted_channels: None,
			level: Some(1),
			channels: vec![],
		};
		state
			.apply_notification_preferences(n::Event::Settings {
				entries: vec![setting.clone()],
				replace: true,
			})
			.unwrap();
		state
			.apply_notification_preferences(n::Event::Presence(Some(false)))
			.unwrap();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(1001, channel.id)),
		});
		assert!(state.take_notification().is_none());
		let mut mention = message(1003, channel.id);
		mention.mentions = vec![state.user.clone().unwrap()];
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(mention),
		});
		assert_eq!(state.take_notification().unwrap().message, Id(1003));
		let mut all = setting.clone();
		all.level = Some(0);
		state
			.apply_notification_preferences(n::Event::Settings {
				entries: vec![all.clone()],
				replace: true,
			})
			.unwrap();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(1005, channel.id)),
		});
		assert_eq!(state.take_notification().unwrap().message, Id(1005));
		all.channels = vec![(channel.parent_id.unwrap_or(channel.id), Some(true), Some(0))];
		state
			.apply_notification_preferences(n::Event::Settings {
				entries: vec![all],
				replace: true,
			})
			.unwrap();
		assert!(!state.notification_allowed(channel.id));
		let mut oversized = setting;
		oversized.channels =
			Vec::with_capacity(32 * 1024 * 1024 / size_of::<(Id, Option<bool>, Option<u8>)>() + 1);
		assert!(
			state
				.apply_notification_preferences(n::Event::Settings {
					entries: vec![oversized],
					replace: true
				})
				.is_err()
		);
		assert!(!state.notification_preferences_known());
	}
	#[test]
	fn notifications_honor_unknown_preferences_mutes_dnd_and_queue_bounds() {
		use client_core::notifications as n;
		let mut state = demo_state();
		let channel = state
			.channels
			.iter()
			.find(|c| c.guild.is_none() && c.supports_text())
			.unwrap()
			.id;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(1001, channel)),
		});
		assert!(state.take_notification().is_none());
		state
			.apply_notification_preferences(n::Event::Settings {
				entries: vec![n::Setting {
					channel_mute_until: vec![],
					guild: None,
					muted: Some(false),
					suppress_everyone: Some(false),
					suppress_roles: Some(false),
					hide_muted_channels: None,
					level: Some(0),
					channels: vec![],
				}],
				replace: true,
			})
			.unwrap();
		state
			.apply_notification_preferences(n::Event::Presence(Some(true)))
			.unwrap();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(1003, channel)),
		});
		assert!(state.take_notification().is_none());
		state
			.apply_notification_preferences(n::Event::Presence(Some(false)))
			.unwrap();
		state
			.apply_notification_preferences(n::Event::Settings {
				entries: vec![n::Setting {
					channel_mute_until: vec![],
					guild: None,
					muted: Some(false),
					suppress_everyone: Some(false),
					suppress_roles: Some(false),
					hide_muted_channels: None,
					level: Some(0),
					channels: vec![(channel, Some(true), Some(0))],
				}],
				replace: true,
			})
			.unwrap();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(1005, channel)),
		});
		assert!(state.take_notification().is_none());
		state
			.apply_notification_preferences(n::Event::Settings {
				entries: vec![n::Setting {
					channel_mute_until: vec![],
					guild: None,
					muted: Some(false),
					suppress_everyone: Some(false),
					suppress_roles: Some(false),
					hide_muted_channels: None,
					level: Some(0),
					channels: vec![],
				}],
				replace: true,
			})
			.unwrap();
		for id in (1007..11007).step_by(2) {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Message(message(id, channel)),
			});
		}
		assert_eq!(state.unread_count(channel), 4096);
		let mut count = 0;
		while state.take_notification().is_some() {
			count += 1;
		}
		assert_eq!(count, 32);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(1007, channel)),
		});
		assert!(state.take_notification().is_none());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message(11009, channel)),
		});
		state.logout();
		assert!(state.take_notification().is_none());
		assert_eq!(state.unread_count(channel), 0);
	}
	#[test]
	fn pin_snapshots_are_scoped_cancellable_and_open_revalidated_history() {
		use client_core::{Command, search::Outcome};
		let mut state = demo_state();
		let page = || SearchPage {
			hits: [480, 499]
				.into_iter()
				.map(|id| SearchHit {
					id: Id(id),
					channel: Id(20),
					author: crate::message(1, Id(20)).author,
					excerpt: "pin".into(),
					attachments: vec![],
					embeds: vec![],
				})
				.collect(),
			total: 0,
			partial: true,
			pin_cursor: Some(100),
		};
		assert!(state.request_older_pins().is_none());
		let Command::Pins { request: old, .. } = state.request_pins().unwrap() else {
			panic!()
		};
		let Command::Pins { request, .. } = state.request_pins().unwrap() else {
			panic!()
		};
		state.apply_search(Id(20), old, Ok(Outcome::Pins(page())));
		assert!(state.search.as_ref().unwrap().loading);
		state.apply_search(Id(20), request, Ok(Outcome::Page(page())));
		assert!(state.search.as_ref().unwrap().page.is_none());
		let Command::Pins { request, .. } = state.request_pins().unwrap() else {
			panic!()
		};
		state.apply_search(Id(21), request, Ok(Outcome::Pins(page())));
		assert!(state.search.as_ref().unwrap().loading);
		state.apply_search(Id(20), request, Ok(Outcome::Pins(page())));
		assert_eq!(
			state.search.as_ref().unwrap().page.as_ref().unwrap().hits[0].id,
			Id(480)
		);
		let Command::Pins {
			before, request, ..
		} = state.request_older_pins().unwrap()
		else {
			panic!()
		};
		assert_eq!(before, Some(100));
		assert!(state.search.as_ref().unwrap().page.is_none());
		assert!(
			state.request_older_pins().is_none(),
			"Do not queue duplicate page requests"
		);
		state.apply_search(Id(20), request, Ok(Outcome::Pins(page())));
		assert!(
			state.search.as_ref().unwrap().error.is_some(),
			"Reject a nonprogressing cursor"
		);
		let retry = state.request_older_pins().unwrap();
		assert!(matches!(
			retry,
			Command::Pins {
				before: Some(100),
				..
			}
		));
		state.command_rejected(retry);
		let Command::Pins {
			before, request, ..
		} = state.request_older_pins().unwrap()
		else {
			panic!()
		};
		assert_eq!(
			before,
			Some(100),
			"Retry the failed page without returning to newest"
		);
		let mut older_page = page();
		older_page.hits[0].id = Id(420);
		older_page.hits[1].id = Id(455);
		older_page.partial = false;
		older_page.pin_cursor = None;
		state.apply_search(Id(20), request, Ok(Outcome::Pins(older_page)));
		let loaded = state.search.as_ref().unwrap().page.as_ref().unwrap();
		assert_eq!(loaded.hits.len(), 2, "Pages replace rather than accumulate");
		assert_eq!(loaded.hits[0].id, Id(420));
		assert!(
			state.request_older_pins().is_none(),
			"Exhaustion stops pagination"
		);
		assert!(
			state.open_search_hit(Id(480)).is_none(),
			"An old page is no longer actionable"
		);
		let Command::Pins {
			before,
			request: newest,
			..
		} = state.request_pins().unwrap()
		else {
			panic!()
		};
		assert_eq!(before, None);
		state.apply_search(Id(20), request, Ok(Outcome::Pins(page())));
		assert!(
			state.search.as_ref().unwrap().loading,
			"A late older page cannot replace Reload"
		);
		state.apply_search(Id(20), newest, Ok(Outcome::Pins(page())));
		assert!(state.open_search_hit(Id(478)).is_none());
		assert!(matches!(
			state.open_search_hit(Id(480)),
			Some(Command::History {
				before: Some(Id(481)),
				..
			})
		));
		load_page(&mut state, Some(Id(481)));
		assert!(state.timeline.get(Id(480)).is_some());
		let command = state.request_pins().unwrap();
		state.command_rejected(command);
		assert!(!state.search.as_ref().unwrap().loading);
		assert!(state.search.as_ref().unwrap().error.is_some());
		let Command::Pins { request, .. } = state.request_pins().unwrap() else {
			panic!()
		};
		state.clear_search();
		state.apply_search(Id(20), request, Ok(Outcome::Pins(page())));
		assert!(state.search.is_none());
		state.gateway_connected = false;
		assert!(state.request_pins().is_none());
	}
	#[test]
	fn search_pages_reject_late_results_and_open_only_revalidated_history() {
		use client_core::{Command, auth::Failure, search::Outcome};
		let mut state = demo_state();
		let page = || {
			Outcome::Page(SearchPage {
				hits: vec![SearchHit {
					id: Id(499),
					channel: Id(20),
					author: crate::message(1, Id(20)).author,
					excerpt: "index text".into(),
					attachments: vec![],
					embeds: vec![],
				}],
				total: 50,
				partial: false,
				pin_cursor: None,
			})
		};
		assert!(state.request_search("x".into(), Some(Id(500))).is_none());
		let Command::Search {
			request: old,
			guild,
			..
		} = state.request_search("first".into(), None).unwrap()
		else {
			panic!()
		};
		assert_eq!(guild, Some(Id(10)));
		let Command::Search { request, .. } = state.request_search("second".into(), None).unwrap()
		else {
			panic!()
		};
		state.apply_search(Id(20), old, Ok(page()));
		assert!(state.search.as_ref().unwrap().loading);
		state.apply_search(Id(20), request, Ok(page()));
		assert_eq!(
			state.timeline.get(Id(499)).unwrap().content,
			message(499, Id(20)).content
		);
		assert!(state.open_search_hit(Id(498)).is_none());
		assert!(matches!(
			state.open_search_hit(Id(499)),
			Some(Command::History {
				channel: Id(20),
				before: Some(Id(500)),
				..
			})
		));
		assert!(state.timeline.is_empty());
		assert_eq!(state.search_target, Some(Id(499)));
		load_page(&mut state, Some(Id(500)));
		assert_eq!(state.timeline.iter().last().unwrap().id, Id(499));
		assert_eq!(
			state.timeline.get(Id(499)).unwrap().content,
			message(499, Id(20)).content
		);
		let Command::Search { request, .. } = state
			.request_search("second".into(), Some(Id(499)))
			.unwrap()
		else {
			panic!()
		};
		state.apply_search(Id(20), request, Ok(page())); // inclusive cursor violates the requested page
		assert!(state.search.as_ref().unwrap().page.is_none());
		let command = state.request_search("third".into(), None).unwrap();
		state.command_rejected(command);
		assert!(!state.search.as_ref().unwrap().loading);
		let Command::Search { request, .. } = state.request_search("fourth".into(), None).unwrap()
		else {
			panic!()
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Delete {
				channel: Id(20),
				id: Id(499),
			},
		});
		state.apply_search(Id(20), request, Ok(page()));
		assert!(state.search.is_none());
		let Command::Search { request, .. } = state.request_search("fifth".into(), None).unwrap()
		else {
			panic!()
		};
		state.select(Id(22));
		state.apply_search(Id(20), request, Err(Failure::Forbidden));
		assert!(state.search.is_none());
		let Command::Search { request, .. } = state.request_search("sixth".into(), None).unwrap()
		else {
			panic!()
		};
		state.apply_search(Id(22), request, Ok(Outcome::Indexing));
		assert!(
			state
				.search
				.as_ref()
				.unwrap()
				.error
				.unwrap()
				.contains("indexing")
		);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Disconnected,
		});
		assert!(state.search.is_none());
		assert!(state.request_search("offline".into(), None).is_none());
		state.logout();
		state.apply(Envelope {
			generation: state.generation - 1,
			event: Event::Search {
				channel: Id(20),
				request,
				result: Ok(page()),
			},
		});
		assert!(state.search.is_none());
	}
	#[test]
	fn read_markers_are_explicit_scoped_and_preserve_newer_service_updates() {
		use client_core::{Command, auth::Failure, read_state::Event as R};
		let mut state = demo_state();
		assert_eq!(state.unread(Id(20)), Some(true));
		assert!(!state.can_mark_read(Id(495)));
		assert!(!state.can_mark_read(Id(900)));
		let Command::MarkRead {
			channel,
			message,
			request,
			..
		} = state.prepare_mark_read(Id(500)).unwrap()
		else {
			panic!()
		};
		assert!(state.prepare_mark_read(Id(499)).is_none());
		assert_eq!(state.read_marker(channel), Some(Some(Id(495))));
		state
			.apply_read_state(R::Ack {
				channel,
				message: Some(Id(480)),
				manual: true,
				mention_count: None,
				version: Some(3),
			})
			.unwrap();
		state
			.apply_read_state(R::Result {
				channel,
				message,
				request,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(state.read_marker(channel), Some(Some(Id(480))));
		state
			.apply_read_state(R::Ack {
				channel,
				message: Some(Id(499)),
				manual: false,
				mention_count: None,
				version: Some(2),
			})
			.unwrap();
		assert_eq!(state.read_marker(channel), Some(Some(Id(480))));
		let Command::MarkRead { request, .. } = state.prepare_mark_read(Id(500)).unwrap() else {
			panic!()
		};
		state
			.apply_read_state(R::Result {
				channel,
				message,
				request,
				result: Err(Failure::Ambiguous),
			})
			.unwrap();
		assert_eq!(state.read_marker(channel), Some(Some(Id(480))));
		let command = state.prepare_mark_read(Id(500)).unwrap();
		state.command_rejected(command);
		assert_eq!(state.freshness, Freshness::Fresh);
		let Command::MarkRead { request, .. } = state.prepare_mark_read(Id(500)).unwrap() else {
			panic!()
		};
		state.select(Id(22));
		state
			.apply_read_state(R::Result {
				channel,
				message,
				request,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(state.unread(channel), Some(false));
		state
			.apply_read_state(R::Latest(vec![(channel, Patch::Value(Id(600)))]))
			.unwrap();
		assert_eq!(state.unread(channel), Some(true));
		state.select(channel);
		load_page(&mut state, None);
		state
			.apply_read_state(R::Ack {
				channel,
				message: None,
				manual: true,
				mention_count: None,
				version: Some(4),
			})
			.unwrap();
		let Command::MarkRead { request, .. } = state.prepare_mark_read(Id(600)).unwrap() else {
			panic!()
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Disconnected,
		});
		state
			.apply_read_state(R::Result {
				channel,
				message,
				request,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(state.read_marker(channel), None);
		assert!(state.prepare_mark_read(message).is_none());
		state.gateway_connected = true;
		assert_eq!(state.read_marker(channel), Some(None));
		state.logout();
		assert_eq!(state.read_marker(channel), None);
		state = demo_state();
		let Command::MarkRead { request: old, .. } = state.prepare_mark_read(message).unwrap()
		else {
			panic!()
		};
		state
			.apply_read_state(R::Snapshot {
				partial: false,
				entries: None,
				version: None,
			})
			.unwrap();
		assert_eq!(state.unread(channel), None);
		let Command::MarkRead { request: new, .. } = state.prepare_mark_read(message).unwrap()
		else {
			panic!()
		};
		assert_ne!(old, new);
		state
			.apply_read_state(R::Result {
				channel,
				message,
				request: old,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(state.read_marker(channel), None);
		state
			.apply_read_state(R::Result {
				channel,
				message,
				request: new,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(state.unread(channel), Some(false));
		state.apply(Envelope {
			generation: state.generation,
			event: Event::RecipientRemoved {
				channel,
				user: Id(1),
			},
		});
		assert_eq!(state.read_marker(channel), None);
		assert!(
			state
				.apply_read_state(R::Snapshot {
					partial: false,
					entries: Some(vec![(Id(22), None, 0), (Id(22), None, 0)]),
					version: None
				})
				.is_err()
		);
		assert_eq!(state.read_marker(Id(22)), None);
		state = demo_state();
		state
			.apply_read_state(R::Snapshot {
				entries: Some(vec![(Id(20), Some(Id(495)), 0)]),
				version: Some(1),
				partial: true,
			})
			.unwrap();
		assert_eq!(state.read_marker(Id(20)), Some(Some(Id(495))));
		assert_eq!(state.read_marker(Id(22)), None);
	}
	#[test]
	fn late_logout_events_and_duplicate_send_confirmations() {
		let mut state = demo_state();
		let old = state.generation;
		state.logout();
		state.apply(Envelope {
			generation: old,
			event: Event::Message(message(42, Id(20))),
		});
		assert!(state.timeline.is_empty());
		assert!(state.user.is_none());
		state = demo_state();
		state.drafts.insert(Id(20), "synthetic outbound".into());
		let client_core::Command::Send { nonce, .. } = state.prepare_send().unwrap() else {
			panic!()
		};
		let mut m = message(900, Id(20));
		m.nonce = Some(nonce.clone());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(m.clone()),
		});
		state.apply(Envelope {
			generation: state.generation,
			event: Event::SendResult {
				nonce,
				result: Ok(m),
			},
		});
		assert_eq!(state.timeline.iter().filter(|m| m.id == Id(900)).count(), 1);
		assert!(state.pending.is_empty());
	}
}
