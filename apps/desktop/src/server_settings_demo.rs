//! Explicit offline fixture; never loads a saved session or changes a real guild.
use client_core::{Command, Envelope, Event, State};
use model::{
	Id,
	server_settings::{Edit, Settings, Trait},
};

fn snapshot(state: &State, guild: Id) -> Settings {
	let server = state
		.guilds
		.iter()
		.find(|server| server.id == guild)
		.expect("fixture guild");
	Settings {
		guild,
		name: server.name.clone(),
		icon: server.icon.clone(),
		banner_color: Some(0x245ae8),
		traits: vec![
			Trait {
				label: "Games".into(),
				emoji: Some("🎮".into()),
			},
			Trait {
				label: "Community".into(),
				emoji: None,
			},
		],
		description: "A synthetic workspace for friends, games and conversation.".into(),
		online_count: Some(4),
		member_count: Some(9),
		system_channel_id: state
			.channels
			.iter()
			.find(|channel| channel.guild == Some(guild) && channel.kind == 0)
			.map(|channel| channel.id),
		default_message_notifications: 1,
		activity_feed: Some(true),
		features: vec![
			"COMMUNITY".into(),
			model::server_settings::ACTIVITY_ENABLED.into(),
		],
		..Default::default()
	}
}

pub fn execute(state: &State, guild: Id, request: u64, edit: Option<Box<Edit>>) -> Event {
	let mut value = state
		.server_settings
		.snapshot
		.as_ref()
		.filter(|value| value.guild == guild)
		.cloned()
		.unwrap_or_else(|| snapshot(state, guild));
	if let Some(edit) = edit {
		edit.apply(&mut value);
		if matches!(edit.icon, model::Patch::Value(_)) {
			value.icon = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
		}
	}
	assert!(value.valid(), "synthetic server settings remain valid");
	Event::ServerSettings(client_core::server_settings::Event {
		guild,
		request,
		result: Ok(Box::new(value)),
		refreshed: None,
	})
}

pub fn execute_admin(
	state: &State,
	guild: Id,
	request: u64,
	action: model::server_admin::Action,
) -> Event {
	use model::server_admin::{Action, Emoji, Emojis, Member, Members, Result as Outcome, Role};
	assert!(
		action.valid(),
		"synthetic admin action must satisfy wire bounds"
	);
	let owner = state.user.as_ref().expect("fixture user");
	let result = match action {
		Action::AuditLog(query) => execute_audit_log(state, guild, query),
		Action::Invites(action) => execute_invites(state, guild, action),
		Action::Integrations(action) => execute_integrations(state, guild, action),
		Action::Roles(action) => execute_roles(state, guild, action),
		Action::LoadEmojis
		| Action::CreateEmoji { .. }
		| Action::RenameEmoji { .. }
		| Action::DeleteEmoji { .. } => {
			let mut page = state.server_admin.emojis.clone().unwrap_or_else(|| Emojis {
				items: vec![Emoji {
					emoji: model::CustomEmoji {
						id: Id(9001),
						name: "serein".into(),
						animated: false,
						available: true,
						managed: false,
						roles: Some(Vec::new()),
					},
					uploader: Some(owner.clone()),
				}],
				static_limit: Some(50),
				animated_limit: Some(50),
			});
			match action {
				Action::CreateEmoji { name, image } => {
					let id = Id(page
						.items
						.iter()
						.map(|row| row.emoji.id.0)
						.max()
						.unwrap_or(9000) + 1);
					page.items.push(Emoji {
						emoji: model::CustomEmoji {
							id,
							name,
							animated: image.starts_with("data:image/gif"),
							available: true,
							managed: false,
							roles: Some(Vec::new()),
						},
						uploader: Some(owner.clone()),
					});
				}
				Action::RenameEmoji { id, name } => {
					if let Some(row) = page.items.iter_mut().find(|row| row.emoji.id == id) {
						row.emoji.name = name;
					}
				}
				Action::DeleteEmoji { id } => page.items.retain(|row| row.emoji.id != id),
				_ => {}
			}
			Outcome::Emojis(page)
		}
		Action::LoadMembers(query) => {
			let mut page = {
				let roles: Vec<_> = state
					.permissions
					.guilds
					.get(&guild)
					.and_then(|guild| guild.roles.as_ref())
					.into_iter()
					.flatten()
					.map(|role| Role {
						role: role.clone(),
						managed: false,
					})
					.collect();
				let roles = state.server_admin.roles.as_ref().map_or(roles, |catalog| {
					catalog
						.items
						.iter()
						.map(|role| Role {
							role: role.permission_role(),
							managed: role.managed,
						})
						.collect()
				});
				let items = [
					"Avery", "Mika", "Rowan", "Sam", "Taylor", "Morgan", "Alex", "Jamie",
				]
				.into_iter()
				.enumerate()
				.map(|(index, name)| {
					let mut user = owner.clone();
					user.id = Id(((1_620_070_400_000u64 + index as u64 * 86_400_000
						- 1_420_070_400_000)
						<< 22) | 1);
					user.name = name.into();
					user.avatar = None;
					Member {
						user,
						nick: None,
						roles: roles
							.iter()
							.filter(|role| role.role.id != guild)
							.skip(index % 4)
							.take(1)
							.map(|role| role.role.id)
							.collect(),
						joined_at: Some(1_789_200_000_000 - index as i128 * 86_400_000),
						join_source: Some(1),
						invite_code: Some("demo".into()),
						flags: Some(0),
						unusual_dm_until: None,
						timeout_until: None,
					}
				})
				.collect::<Vec<_>>();
				Members {
					total: items.len() as u64,
					items,
					roles,
					next: None,
					features: Vec::new(),
					show_in_channel_list: Some(false),
				}
			};
			if let Some(cached) = &state.server_admin.members {
				for row in &mut page.items {
					if let Some(previous) = cached
						.items
						.iter()
						.find(|previous| previous.user.id == row.user.id)
					{
						*row = previous.clone();
					}
				}
			}
			page.items.retain(|row| {
				query.search.is_empty()
					|| row
						.user
						.name
						.to_lowercase()
						.contains(&query.search.to_lowercase())
					|| row.user.id.to_string() == query.search
			});
			match query.sort {
				2 => page.items.sort_by_key(|row| row.joined_at),
				3 => page.items.sort_by_key(|row| std::cmp::Reverse(row.user.id)),
				4 => page.items.sort_by_key(|row| row.user.id),
				_ => page
					.items
					.sort_by_key(|row| std::cmp::Reverse(row.joined_at)),
			}
			Outcome::Members(page)
		}
		Action::SetRole { user, .. } | Action::SetNickname { user, .. } => {
			let mut member = state
				.server_admin
				.members
				.as_ref()
				.and_then(|page| page.items.iter().find(|row| row.user.id == user))
				.expect("fixture target member")
				.clone();
			match action {
				Action::SetRole { role, assigned, .. } => {
					member.roles.retain(|id| *id != role);
					if assigned {
						member.roles.push(role);
					}
				}
				Action::SetNickname { nick, .. } => {
					member.nick = (!nick.is_empty()).then_some(nick)
				}
				_ => {}
			}
			Outcome::Member(member)
		}
		Action::Kick { user } => Outcome::Kicked(user),
		Action::Prune { .. } => Outcome::Pruned(Some(0)),
		Action::ShowMembers { enabled } => Outcome::ChannelList(enabled),
	};
	assert!(
		result.valid(),
		"synthetic administration response must stay bounded"
	);
	Event::ServerAdmin(client_core::server_admin::Event {
		guild,
		request,
		result: Ok(result),
	})
}

fn role_catalog(state: &State, guild: Id) -> model::server_roles::Catalog {
	use model::server_roles::{Catalog, Colors, Role};
	state.server_admin.roles.clone().unwrap_or_else(|| {
		let mut items = vec![Role {
			id: guild,
			name: "@everyone".into(),
			permissions: model::permissions::VIEW_CHANNEL,
			member_count: Some(8),
			..Default::default()
		}];
		for (index, (name, color, count, managed)) in [
			("Founder", 0xa93226, 2, false),
			("Game For Dev Shooting Blaster 2D", 0x34495e, 0, false),
			("Members", 0x1abc9c, 4, false),
			("Marketing", 0xf1c40f, 1, false),
			("Support", 0x99aab5, 0, false),
			("Bots", 0x99aab5, 2, true),
			("Tickety", 0x99aab5, 1, true),
			("carl-bot", 0x99aab5, 1, true),
		]
		.into_iter()
		.enumerate()
		{
			items.push(Role {
				id: Id(9500 + index as u64),
				name: name.into(),
				colors: Colors {
					primary: color,
					..Default::default()
				},
				permissions: model::permissions::VIEW_CHANNEL,
				position: 8 - index as i32,
				member_count: Some(count),
				managed,
				..Default::default()
			});
		}
		items.sort_by(|a, b| b.position.cmp(&a.position).then_with(|| a.id.cmp(&b.id)));
		Catalog {
			guild,
			items,
			features: vec!["ROLE_ICONS".into(), "ENHANCED_ROLE_COLORS".into()],
		}
	})
}

fn execute_roles(
	state: &State,
	guild: Id,
	action: model::server_roles::Action,
) -> model::server_admin::Result {
	use model::server_roles::{Action, Result as Outcome, Role};
	let mut catalog = role_catalog(state, guild);
	let mut selected = None;
	match action {
		Action::Load => {}
		Action::Create(edit) => {
			let id = Id(catalog
				.items
				.iter()
				.map(|role| role.id.0)
				.max()
				.unwrap_or(9500)
				+ 1);
			let mut role = Role {
				id,
				position: 1,
				member_count: Some(0),
				..Default::default()
			};
			edit.apply(&mut role);
			for existing in &mut catalog.items {
				if existing.position >= 1 {
					existing.position += 1;
				}
			}
			catalog.items.push(role);
			selected = Some(id);
		}
		Action::Edit { id, edit } => {
			if let Some(role) = catalog.items.iter_mut().find(|role| role.id == id) {
				edit.apply(role);
				if matches!(edit.icon, model::Patch::Value(_)) {
					role.icon = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
				}
			}
			selected = Some(id);
		}
		Action::Delete(id) => catalog.items.retain(|role| role.id != id),
		Action::Move { id, position } => {
			if let Some(old) = catalog
				.items
				.iter()
				.find(|role| role.id == id)
				.map(|role| role.position)
			{
				for role in &mut catalog.items {
					if role.id == id {
						role.position = position;
					} else if old < position && role.position > old && role.position <= position {
						role.position -= 1;
					} else if old > position && role.position >= position && role.position < old {
						role.position += 1;
					}
				}
			}
		}
		Action::Members { role, query } => {
			let event = execute_admin(
				state,
				guild,
				0,
				model::server_admin::Action::LoadMembers(query),
			);
			let Event::ServerAdmin(client_core::server_admin::Event {
				result: Ok(model::server_admin::Result::Members(mut page)),
				..
			}) = event
			else {
				unreachable!()
			};
			page.roles = catalog
				.items
				.iter()
				.map(|role| model::server_admin::Role {
					role: role.permission_role(),
					managed: role.managed,
				})
				.collect();
			if let Some(role) = role {
				page.items.retain(|member| member.roles.contains(&role));
			}
			page.total = page.items.len() as u64;
			return model::server_admin::Result::Roles(Outcome::Members { role, page });
		}
	}
	catalog
		.items
		.sort_by(|a, b| b.position.cmp(&a.position).then_with(|| a.id.cmp(&b.id)));
	model::server_admin::Result::Roles(Outcome::Catalog { catalog, selected })
}

fn invite_snapshot(state: &State, guild: Id) -> model::server_invites::Snapshot {
	use model::server_invites::{Invite, Snapshot};
	if let Some(snapshot) = &state.server_admin.invites {
		return snapshot.clone();
	}
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_nanos() as i128;
	let channel = state.invite_channel(guild);
	Snapshot {
		guild,
		features: vec!["COMMUNITY".into()],
		items: [
			"Avery", "Mika", "Rowan", "Sam", "Taylor", "Morgan", "Alex", "Jamie",
		]
		.into_iter()
		.enumerate()
		.map(|(index, name)| {
			let mut user = state.user.clone().expect("fixture owner");
			user.id = Id(9800 + index as u64);
			user.name = name.into();
			user.avatar = None;
			Invite {
				code: format!("demo-link-{}", index + 1),
				inviter: Some(user),
				channel,
				channel_name: Some(
					[
						"general",
						"game-night",
						"announcements",
						"a-long-channel-name-for-the-community",
					][index % 4]
						.into(),
				),
				uses: Some(if index == 2 { 6 } else { 0 }),
				max_uses: Some(0),
				max_age: Some(if index == 5 { 0 } else { 30 * 86400 }),
				created_at: Some(now - 3600 * 1_000_000_000),
				expires_at: (index != 5).then_some(
					now + (if index == 6 {
						435
					} else {
						2_505_600 - index as i128 * 85123
					}) * 1_000_000_000,
				),
				temporary: Some(false),
				roles: None,
			}
		})
		.collect(),
	}
}

fn execute_invites(
	state: &State,
	guild: Id,
	action: model::server_invites::Action,
) -> model::server_admin::Result {
	use model::server_invites::Action;
	let mut snapshot = invite_snapshot(state, guild);
	match action {
		Action::Load => {}
		Action::Revoke { code } => snapshot.items.retain(|invite| invite.code != code),
		Action::SetPaused { paused } => {
			snapshot
				.features
				.retain(|feature| feature != "INVITES_DISABLED");
			if paused {
				snapshot.features.push("INVITES_DISABLED".into());
			}
		}
	}
	model::server_admin::Result::Invites(snapshot)
}

/// Same offline invite creation for the desktop demo and framebuffer harness.
pub fn execute_action(
	state: &mut State,
	action: client_core::server_actions::Action,
	request: u64,
) -> Event {
	use client_core::server_actions::Action;
	let result = match action {
		Action::CreateInvite {
			guild,
			channel,
			options,
		} => {
			let code = format!("synthetic-{request}");
			if state.server_admin.guild == Some(guild) {
				let mut snapshot = invite_snapshot(state, guild);
				let now = std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap_or_default()
					.as_nanos() as i128;
				snapshot.items.push(model::server_invites::Invite {
					code: code.clone(),
					inviter: state.user.clone(),
					channel: Some(channel),
					channel_name: state.channel(channel).map(|channel| channel.name.clone()),
					uses: Some(0),
					max_uses: Some(u64::from(options.max_uses)),
					max_age: Some(u64::from(options.max_age)),
					created_at: Some(now),
					expires_at: (options.max_age != 0)
						.then_some(now + i128::from(options.max_age) * 1_000_000_000),
					temporary: Some(options.temporary),
					roles: None,
				});
				if !snapshot.valid() {
					return Event::ServerAction(client_core::server_actions::Event::Written {
						action,
						request,
						result: Err(client_core::auth::Failure::Protocol),
					});
				}
				state.server_admin.invites = Some(snapshot);
			}
			Some(code)
		}
		Action::Leave(_) => None,
	};
	Event::ServerAction(client_core::server_actions::Event::Written {
		action,
		request,
		result: Ok(result),
	})
}

pub fn open(state: &mut State, messaging: &mut ui::MessagingUi) {
	let guild = state.guilds[0].id;
	let user = state.user.as_ref().expect("fixture user").id;
	let mut permissions = test_support::permission_snapshot(state);
	let permission = permissions
		.guilds
		.iter_mut()
		.find(|permission| permission.id == guild)
		.unwrap();
	permission.owner = Some(user);
	let scenario = std::env::args().find_map(|arg| {
		arg.strip_prefix("--demo-server-permission=")
			.map(str::to_owned)
	});
	if let Some(scenario) = scenario.as_deref() {
		permission.owner = Some(Id(u64::MAX));
		if let Some(roles) = &mut permission.roles {
			roles[0].bits |= match scenario {
				"admin" => model::permissions::ADMINISTRATOR,
				"manager" => model::permissions::MANAGE_GUILD,
				"webhooks" => model::permissions::MANAGE_WEBHOOKS,
				"audit" => model::permissions::VIEW_AUDIT_LOG,
				_ => 0,
			};
		}
		if scenario == "unknown" {
			permission.roles = None;
		}
	}
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Permissions(client_core::permissions::Event::Snapshot(permissions)),
	});
	if let Some(page) =
		std::env::args().find_map(|arg| arg.strip_prefix("--demo-server-page=").map(str::to_owned))
	{
		if let Some(Command::ServerAdmin {
			guild,
			request,
			action,
		}) = messaging.preview_server_admin(state, guild, &page)
		{
			let event = execute_admin(state, guild, request, *action);
			state.apply(Envelope {
				generation: state.generation,
				event,
			});
		}
		return;
	}
	if let Some(Command::ServerSettings {
		guild,
		request,
		edit,
	}) = state.load_server_settings(guild)
	{
		let event = execute(state, guild, request, edit);
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
	if let Some(Command::ServerSettings {
		guild,
		request,
		edit,
	}) = messaging.preview_server_settings(state, guild)
	{
		let event = execute(state, guild, request, edit);
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
}

fn execute_integrations(
	state: &State,
	guild: Id,
	action: model::server_integrations::Action,
) -> model::server_admin::Result {
	use model::server_integrations::{Action, Application, Integration, Snapshot, Source, Webhook};
	let channel = state
		.channels
		.iter()
		.find(|channel| channel.guild == Some(guild) && channel.kind == 0)
		.unwrap()
		.id;
	let mut page = state.server_admin.integrations.clone().unwrap_or(Snapshot {
		guild,
		channel: None,
		integrations: None,
		webhooks: None,
	});
	if page.integrations.is_none() {
		page.integrations = Some(
			[
				("Orbit", "Moderation and community tools."),
				("Tickets", "A helping hand for your support team."),
			]
			.into_iter()
			.enumerate()
			.map(|(index, (name, description))| {
				let id = Id(9800 + index as u64);
				let mut bot = state.user.as_ref().unwrap().clone();
				bot.id = id;
				bot.name = name.into();
				bot.avatar = None;
				bot.kind = model::AccountKind::Bot;
				Integration {
					id,
					name: name.into(),
					kind: "discord".into(),
					enabled: true,
					user: state.user.clone(),
					synced_at: None,
					role_id: None,
					application: Some(Application {
						id,
						name: name.into(),
						icon: None,
						description: description.into(),
						bot: Some(bot),
					}),
				}
			})
			.collect(),
		);
	}
	if page.webhooks.is_none() {
		page.webhooks = Some(
			(0..3)
				.map(|index| Webhook {
					id: Id(9900 + index),
					guild,
					channel: Some(channel),
					kind: if index == 2 { 2 } else { 1 },
					name: Some(
						["Build updates", "Support updates", "Community news"][index as usize]
							.into(),
					),
					avatar: None,
					application_id: (index == 1).then_some(Id(9801)),
					user: state.user.clone(),
					source_guild: (index == 2).then(|| Source {
						id: Id(9990),
						name: Some("Serein Community".into()),
					}),
					source_channel: (index == 2).then(|| Source {
						id: Id(9991),
						name: Some("announcements".into()),
					}),
				})
				.collect(),
		);
	}
	match action {
		Action::CopyWebhookUrl {
			webhook, channel, ..
		} => {
			return model::server_admin::Result::WebhookUrl(
				model::server_integrations::WebhookUrl::new(
					guild,
					webhook,
					channel,
					"SYNTHETIC_DEMO_WEBHOOK_TOKEN",
				)
				.expect("valid synthetic URL"),
			);
		}
		Action::Load {
			channel,
			integrations,
			webhooks,
		} => {
			page.channel = channel;
			if !integrations {
				page.integrations = None;
			}
			if !webhooks {
				page.webhooks = None;
			}
		}
		Action::CreateWebhook {
			channel,
			name,
			scope,
		} => {
			page.channel = scope;
			let items = page.webhooks.as_mut().unwrap();
			let id = Id(items.iter().map(|item| item.id.0).max().unwrap_or(9900) + 1);
			items.push(Webhook {
				id,
				guild,
				channel: Some(channel),
				kind: 1,
				name: Some(name),
				avatar: None,
				application_id: None,
				user: state.user.clone(),
				source_guild: None,
				source_channel: None,
			});
			page.integrations = None;
		}
		Action::EditWebhook {
			scope,
			webhook,
			channel,
			name,
		} => {
			page.channel = scope;
			let hook = page
				.webhooks
				.as_mut()
				.unwrap()
				.iter_mut()
				.find(|hook| hook.id == webhook)
				.unwrap();
			hook.channel = Some(channel);
			hook.name = Some(name);
			page.integrations = None;
		}
		Action::DeleteWebhook { webhook, scope } => {
			page.channel = scope;
			page.webhooks
				.as_mut()
				.unwrap()
				.retain(|hook| hook.id != webhook);
			page.integrations = None;
		}
		Action::DeleteIntegration { integration } => {
			page.integrations
				.as_mut()
				.unwrap()
				.retain(|item| item.id != integration);
			page.webhooks = None;
		}
	}
	if let Some(channel) = page.channel
		&& let Some(hooks) = &mut page.webhooks
	{
		hooks.retain(|hook| hook.channel == Some(channel));
	}
	model::server_admin::Result::Integrations(page)
}

fn execute_audit_log(
	state: &State,
	guild: Id,
	query: model::server_audit_log::Query,
) -> model::server_admin::Result {
	use model::{
		Patch,
		server_audit_log::{Change, Entry, PAGE_SIZE, Page},
	};
	let mut moderator = state.user.as_ref().unwrap().clone();
	moderator.id = Id(9801);
	moderator.name = "Morgan".into();
	moderator.avatar = None;
	let mut owner = state.user.as_ref().unwrap().clone();
	owner.name = "Avery".into();
	let users = vec![owner, moderator];
	let channel = state
		.channels
		.iter()
		.find(|channel| channel.guild == Some(guild) && channel.kind == 0)
		.unwrap()
		.id;
	let entries: Vec<_> = (0..75)
		.map(|index| {
			let action_type = [30, 61, 60, 1, 40, 40, 50, 31, 72][index % 9];
			let (key, name) = match action_type {
				30 | 31 => ("name", "Community".to_owned()),
				60 | 61 => ("name", "serein".to_owned()),
				1 => ("name", "Synthetic Workspace".to_owned()),
				40 => ("code", format!("demoInvite{index}")),
				50 => ("name", "Build updates".to_owned()),
				_ => ("count", "1".to_owned()),
			};
			let mut changes = vec![Change {
				key: key.into(),
				old: if matches!(action_type, 1 | 31 | 61) {
					Patch::Value("Previous name".into())
				} else {
					Patch::Absent
				},
				new: Patch::Value(name.clone()),
			}];
			if action_type == 40 {
				changes.extend(
					[
						("channel_id", channel.to_string()),
						("max_uses", "0".into()),
						("max_age", "2592000".into()),
						("temporary", "false".into()),
					]
					.into_iter()
					.map(|(key, value)| Change {
						key: key.into(),
						old: Patch::Absent,
						new: Patch::Value(value),
					}),
				);
			}
			Entry {
				// Fixed synthetic September 12, 2026 timestamps; never real account history.
				id: Id(
					((1_789_243_200_000u64 - index as u64 * 900_000 - 1_420_070_400_000) << 22)
						+ index as u64,
				),
				user_id: Some(users[index % 2].id),
				target_id: Some(if action_type == 40 {
					name
				} else {
					(9700 + index).to_string()
				}),
				action_type,
				reason: (index == 0).then(|| "Organize community permissions (synthetic).".into()),
				changes,
				options: vec![],
			}
		})
		.filter(|entry| {
			query.user.is_none_or(|user| entry.user_id == Some(user))
				&& query
					.action
					.is_none_or(|action| entry.action_type == action)
				&& query.before.is_none_or(|before| entry.id < before)
		})
		.take(PAGE_SIZE)
		.collect();
	let page = Page {
		guild,
		has_more: entries.len() == PAGE_SIZE,
		entries,
		users,
	};
	assert!(page.valid_response());
	model::server_admin::Result::AuditLog(page)
}
