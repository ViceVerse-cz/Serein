//! Small offline debug run for the forum card -> menu -> command path.
use client_core::{Command, Envelope};
use eframe::egui;
use model::Id;

fn labels(shape: &egui::Shape, found: &mut Vec<(String, egui::Rect)>) {
	match shape {
		egui::Shape::Text(text) => found.push((
			text.galley.job.text.clone(),
			text.galley.rect.translate(text.pos.to_vec2()),
		)),
		egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, found)),
		_ => {}
	}
}

pub fn check() {
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	let mut permissions = test_support::permission_snapshot(&state);
	for guild in &mut permissions.guilds {
		guild.owner = state.user.as_ref().map(|user| user.id);
		guild
			.roles
			.get_or_insert_default()
			.push(model::permissions::Role {
				id: Id(101),
				bits: 0,
				name: "Synthetic colored role".into(),
				color: 0x68ada4,
				position: 1,
				hoist: false,
			});
	}
	state.permissions.replace(permissions).unwrap();
	state.select(Id(26));
	let post = state.forum_posts(Id(26))[0].clone();
	{
		let mut alerts = test_support::demo_state();
		let owner = alerts.user.clone().unwrap();
		alerts
			.apply_notification_preferences(client_core::notifications::Event::Settings {
				entries: vec![client_core::notifications::Setting {
					guild: post.guild,
					muted: Some(false),
					level: Some(1),
					..Default::default()
				}],
				replace: true,
			})
			.unwrap();
		alerts
			.apply_notification_preferences(client_core::notifications::Event::Presence(Some(
				false,
			)))
			.unwrap();
		let mut message = test_support::message(post.last_message.unwrap().0 + 1, post.id);
		message.author.id = Id(987654321);
		message.mentions = vec![owner.clone()];
		message.content = format!(
			"Hello <@{}> <@!{}> <@&999> <#{}>",
			owner.id, owner.id, post.id
		);
		alerts.apply(Envelope {
			generation: alerts.generation,
			event: client_core::Event::Message(message),
		});
		let notification = alerts
			.take_notification()
			.expect("synthetic mention notification");
		assert_eq!(
			notification.preview,
			format!(
				"Hello @{} @{} @Unknown role #{}",
				owner.name, owner.name, post.name
			)
		);
		assert_eq!(alerts.mention_count(post.id), 1);
		let new_posts = alerts.forum_new_count(Id(26));
		assert!(new_posts > 0);
		alerts
			.apply_read_state(client_core::read_state::Event::Ack {
				channel: post.id,
				message: Some(post.id),
				manual: true,
				mention_count: Some(1),
				version: None,
			})
			.unwrap();
		assert!(alerts.post_unread(alerts.channel(post.id).unwrap()));
		assert_eq!(alerts.forum_new_count(Id(26)), new_posts - 1);
		assert_eq!(alerts.mention_count(post.id), 1);
		println!(
			"Forum notifications debug check passed: readable mentions, post badge, and replies excluded from new posts."
		);
	}

	// READY may contain a cursor before its unjoined forum post is loaded.
	let latest = post.last_message.unwrap();
	state.channels.retain(|channel| channel.id != post.id);
	state.invalidate_navigation();
	state
		.apply_read_state(client_core::read_state::Event::Snapshot {
			entries: Some(vec![(post.id, Some(latest), 0)]),
			version: None,
			partial: false,
		})
		.unwrap();
	state.channels.push(post.clone());
	state.invalidate_navigation();
	assert!(
		!state.post_unread(&post),
		"startup must preserve unloaded thread cursors"
	);
	state
		.apply_read_state(client_core::read_state::Event::Ack {
			channel: post.id,
			message: Some(Id(latest.0 - 2)),
			manual: true,
			mention_count: Some(0),
			version: None,
		})
		.unwrap();
	state.demo = false;
	state.posts.parent = Some(Id(26));
	let Command::ForumSummaries { channels, request } =
		state.request_post_summaries(vec![post.id]).unwrap()
	else {
		panic!("summary request");
	};
	assert_eq!(channels, vec![post.id]);
	let rows: Vec<_> = (0..3)
		.map(|offset| {
			serde_json::json!({
				"id": (latest.0 - offset).to_string(), "channel_id": post.id.to_string(),
				"author": {"id": "987654321", "username": "Synthetic"},
				"content": "Latest synthetic reply"
			})
		})
		.collect();
	let wire = serde_json::to_vec(&rows).unwrap();
	let summary = discord_protocol::decode::<discord_protocol::forum::Recent>(&wire)
		.unwrap()
		.into_summary(post.id)
		.unwrap();
	state.apply_forum_summaries(request, vec![(post.id, Ok(summary))]);
	assert_eq!(state.post_new_count(&post), Some((2, true)));
	let latest_summary = state
		.post_summary(post.id)
		.unwrap()
		.latest
		.as_ref()
		.unwrap();
	let author_id = latest_summary.author_id;
	let webhook = latest_summary.webhook;
	let author_roles = latest_summary.roles.clone();
	assert_eq!(
		state.forum_author_color(post.id, author_id, webhook, &author_roles),
		None
	);
	let Some(Command::MemberSearch(author_request)) = state.request_author_members(&[author_id])
	else {
		panic!("forum author lookup")
	};
	let author_event = client_core::Event::MemberSearch {
		request: author_request,
		result: Ok(vec![model::Member {
			user: model::User {
				id: author_id,
				name: "Synthetic forum author".into(),
				kind: model::AccountKind::Human,
				webhook: false,
				avatar: None,
				discriminator: 0,
				primary_guild: None,
			},
			roles: vec![Id(101)],
			nick: None,
			status: None,
			custom_status: None,
			clients: model::ClientPlatforms::default(),
			activities: Vec::new(),
		}]),
	};
	state.apply(Envelope {
		generation: state.generation,
		event: author_event,
	});
	assert_eq!(
		state.forum_author_color(post.id, author_id, webhook, &author_roles),
		Some(0x68ada4)
	);
	let mut other_forum = state.channel(Id(26)).unwrap().clone();
	other_forum.id = Id(126);
	other_forum.name = "Other synthetic forum".into();
	state.channels.push(other_forum);
	state.invalidate_navigation();
	assert!(state.request_forum_posts(Id(126), false).is_some());
	assert!(state.request_forum_posts(Id(26), false).is_some());
	assert!(
		!state.needs_post_summary(post.id),
		"forum switches reuse a fresh summary"
	);
	let uncached: Vec<_> = state
		.forum_posts(Id(26))
		.into_iter()
		.filter(|candidate| candidate.id != post.id)
		.take(client_core::forum::SUMMARY_BATCH)
		.map(|candidate| candidate.id)
		.collect();
	let batch = state.request_post_summaries(uncached).unwrap();
	assert!(
		matches!(&batch, Command::ForumSummaries { channels, .. } if channels.len() > 1),
		"visible forum summaries share one concurrent batch"
	);
	state.command_rejected(batch);
	assert!(
		discord_protocol::decode::<discord_protocol::forum::Recent>(&wire)
			.unwrap()
			.into_summary(Id(999))
			.is_err()
	);
	assert!(!state.needs_post_summary(post.id));
	let mut bounded = model::forum::Summary {
		messages: (0..50).map(|offset| Id(latest.0 - offset)).collect(),
		latest: Some(model::forum::Latest {
			id: latest,
			channel: post.id,
			author_id: Id(987654321),
			author: "Synthetic".into(),
			roles: vec![],
			webhook: false,
			excerpt: "Reply".into(),
		}),
		complete: false,
	};
	assert!(bounded.valid(post.id));
	bounded.messages.push(Id(latest.0 - 50));
	assert!(!bounded.valid(post.id));
	let mut spoiler = rows[0].clone();
	spoiler["content"] = serde_json::json!("||private spoiler||");
	let summary = discord_protocol::decode::<discord_protocol::forum::Recent>(
		&serde_json::to_vec(&vec![spoiler]).unwrap(),
	)
	.unwrap()
	.into_summary(post.id)
	.unwrap();
	assert!(!summary.latest.unwrap().excerpt.contains("private spoiler"));
	println!(
		"Forum debug check passed: deferred startup cursors, exact unread count, scoped bounded previews, and concealed spoilers."
	);
	state.demo = true;

	let mut view = ui::MessagingUi::default();
	let mut followed = false;
	let mut frame = |events: Vec<egui::Event>| {
		let mut commands = vec![];
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 900.0),
				)),
				events,
				..Default::default()
			},
			|ui| commands = view.show(ui, &mut state),
		);
		let mut text = vec![];
		for shape in &output.shapes {
			labels(&shape.shape, &mut text);
		}
		let copied = output.platform_output.commands.iter().any(
			|c| matches!(c, egui::OutputCommand::CopyText(value) if value == &post.id.to_string()),
		);
		output.drop_without_applying_deltas();
		for command in commands {
			if let Command::ChannelAction {
				guild,
				channel,
				request,
				action,
			} = command
			{
				assert!(
					!matches!(action, client_core::channel_actions::Action::Delete),
					"opening delete confirmation must not delete"
				);
				followed |= matches!(
					action,
					client_core::channel_actions::Action::PostFollow(true)
				);
				let event = crate::channel_demo::execute(
					&state, guild, channel, request, action, &mut 9_000,
				);
				state.apply(Envelope {
					generation: state.generation,
					event,
				});
			}
		}
		assert_eq!(
			state.selected,
			Some(Id(26)),
			"right click must not navigate"
		);
		(text, copied)
	};
	frame(vec![]);
	let (text, _) = frame(vec![]);
	assert!(text.iter().any(|(text, _)| text == "(2 New)"));
	assert!(text.iter().any(|(text, _)| text == "Synthetic:"));
	assert!(
		text.iter()
			.any(|(text, _)| text == "Latest synthetic reply")
	);
	let pos = text
		.iter()
		.find(|(label, rect)| label == &post.name && rect.left() > 300.0)
		.expect("forum card")
		.1
		.center();
	let pointer = |pos, button, pressed| {
		vec![
			egui::Event::PointerMoved(pos),
			egui::Event::PointerButton {
				pos,
				button,
				pressed,
				modifiers: egui::Modifiers::NONE,
			},
		]
	};
	for pressed in [true, false] {
		frame(pointer(pos, egui::PointerButton::Secondary, pressed));
	}
	frame(vec![]);
	let (text, _) = frame(vec![]);
	for label in [
		"Mark As Read",
		"Add To Favorites",
		"Follow Post",
		"Close Post",
		"Lock Post",
		"Edit Post",
		"Pin Post",
		"Delete Post",
		"Copy Link",
		"Mute Post",
		"Notification Settings",
		"Copy Thread ID",
	] {
		assert!(
			text.iter().any(|(text, _)| text == label),
			"missing {label}"
		);
	}
	let pos = text
		.iter()
		.find(|(label, _)| label == "Copy Thread ID")
		.unwrap()
		.1
		.center();
	frame(pointer(pos, egui::PointerButton::Primary, true));
	assert!(frame(pointer(pos, egui::PointerButton::Primary, false)).1);
	let (text, _) = frame(vec![]);
	let card = text
		.iter()
		.find(|(label, rect)| label == &post.name && rect.left() > 300.0)
		.unwrap()
		.1
		.center();
	for pressed in [true, false] {
		frame(pointer(card, egui::PointerButton::Secondary, pressed));
	}
	frame(vec![]);
	let (text, _) = frame(vec![]);
	let follow = text
		.iter()
		.find(|(label, _)| label == "Follow Post")
		.unwrap()
		.1
		.center();
	for pressed in [true, false] {
		frame(pointer(follow, egui::PointerButton::Primary, pressed));
	}
	frame(vec![]);
	for pressed in [true, false] {
		frame(pointer(card, egui::PointerButton::Secondary, pressed));
	}
	frame(vec![]);
	let (text, _) = frame(vec![]);
	assert!(text.iter().any(|(label, _)| label == "Unfollow Post"));
	let delete = text
		.iter()
		.find(|(label, _)| label == "Delete Post")
		.unwrap()
		.1
		.center();
	for pressed in [true, false] {
		frame(pointer(delete, egui::PointerButton::Primary, pressed));
	}
	let (text, _) = frame(vec![]);
	assert!(text.iter().any(|(label, _)| label == "Delete Post?"));
	assert!(followed);
	#[cfg(debug_assertions)]
	{
		state.select(post.id);
		let Some(Command::Members {
			guild: Some(guild),
			channel: Some(channel),
			request,
			thread: true,
			..
		}) = state.request_members()
		else {
			panic!("post must subscribe to its participants");
		};
		let list = discord_gateway::debug_thread_member_check(guild, channel, request);
		state.apply(Envelope {
			generation: state.generation,
			event: client_core::Event::Members(list),
		});
		assert_eq!(
			state.members.as_ref().unwrap().freshness,
			model::Freshness::Fresh
		);
		assert_eq!(
			match state.members.as_ref().unwrap().slots[0].as_ref().unwrap() {
				model::MemberSlot::Person(member) => member.user.id,
				_ => panic!("expected person"),
			},
			Id(987)
		);
		println!(
			"Thread participants debug check passed: user subscription, bounded snapshot, presence, stale scope, empty list, and unsubscribe."
		);
	}
	println!(
		"Post menu debug check passed: right-click, all controls, follow, copy ID, delete confirmation, and unchanged selection."
	);
}
