//! Read-only metadata from the selected, readable guild's loaded member pane.
use crate::extension_app::{name, push, text};
use client_core::{State, auth::AuthState};
use extensions::*;
use model::{Freshness, MemberSlot};

fn single(value: &str, limit: usize) -> String {
	text(value, limit)
		.chars()
		.filter(|c| !c.is_control())
		.collect()
}

pub fn snapshot(state: &State) -> Option<MemberDetailsSnapshot> {
	if !(state.demo || state.auth == AuthState::Authenticated)
		|| !state.gateway_connected
		|| state.freshness != Freshness::Fresh
	{
		return None;
	}
	let channel = state.selected?;
	if !state.can_view(channel) || !state.can_read_history(channel) {
		return None;
	}
	let guild = state.channel(channel)?.guild?;
	let members = state.members.as_ref().filter(|m| {
		m.channel == channel && m.guild == Some(guild) && m.freshness == Freshness::Fresh
	})?;
	let catalog = state.guild_roles(guild);
	let mut group = MemberDetailsSnapshot {
		channel_id: channel.0.to_string(),
		guild_id: guild.0.to_string(),
		items: Vec::new(),
		truncated: false,
		roles: catalog.map(|_| Vec::new()),
		roles_truncated: false,
	};
	// Reserve fixed group keys and vector/scalar overhead; push counts each item's wire and storage bytes.
	let mut budget = MAX_MEMBER_DETAILS_BYTES - 512;
	let mut role_budget = budget.min(2048);
	if let (Some(source), Some(roles)) = (catalog, &mut group.roles) {
		for role in source.iter().filter(|r| r.id.0 != 0) {
			let id = role.id.0.to_string();
			if roles.iter().any(|r| r.id == id) {
				continue;
			}
			let before = role_budget;
			if !push(
				roles,
				MemberRoleSnapshot {
					id,
					name: name(&role.name),
					color: role.color & 0xffffff,
					position: role.position,
				},
				&mut role_budget,
				MAX_MEMBER_ROLE_CATALOG,
			) {
				group.roles_truncated = true;
				break;
			}
			budget -= before - role_budget;
		}
	}
	for member in members
		.slots
		.iter()
		.flatten()
		.filter_map(|slot| match slot {
			MemberSlot::Person(m) => Some(m),
			_ => None,
		})
		.filter(|m| m.user.id.0 != 0)
	{
		let id = member.user.id.0.to_string();
		if group.items.iter().any(|m| m.user.id == id) {
			continue;
		}
		let loaded = state
			.profile
			.as_ref()
			.filter(|p| {
				p.user == member.user.id
					&& p.guild == Some(guild)
					&& !p.loading && p.error.is_none()
			})
			.and_then(|p| p.data.as_ref())
			.filter(|p| p.user.id == member.user.id && !p.limited);
		let profile = loaded
			.and_then(|p| p.guild.as_ref())
			.filter(|p| p.guild == guild)
			.map(|p| MemberProfileSnapshot {
				nick: p.nick.as_deref().map(name),
				avatar: p
					.avatar
					.as_ref()
					.filter(|h| model::valid_avatar_hash(h))
					.cloned(),
				bio: text(&p.bio, 1024),
				pronouns: single(&p.pronouns, 256),
				joined_at: p.joined_at.as_deref().map(|v| single(v, 64)),
			});
		let nick = member.nick.as_deref().map(name);
		let display_name = nick
			.clone()
			.or_else(|| loaded.and_then(|p| p.global_name.as_deref()).map(name))
			.unwrap_or_else(|| name(&member.user.name));
		let mut role_ids = Vec::new();
		let mut roles_truncated = false;
		for role in member.roles.iter().filter(|r| r.0 != 0) {
			let id = role.0.to_string();
			if role_ids.contains(&id) {
				continue;
			}
			if role_ids.len() == MAX_MEMBER_DETAIL_ROLES {
				roles_truncated = true;
				break;
			}
			role_ids.push(id);
		}
		let item = MemberDetailSnapshot {
			user: UserSnapshot {
				id,
				name: name(&member.user.name),
			},
			nick,
			display_name,
			role_ids,
			roles_truncated,
			profile,
		};
		let nested_storage = item.role_ids.capacity() * std::mem::size_of::<String>();
		if nested_storage > budget {
			group.truncated = true;
			break;
		}
		budget -= nested_storage;
		if !push(&mut group.items, item, &mut budget, MAX_MEMBER_DETAILS) {
			group.truncated = true;
			break;
		}
	}
	group.truncated |= members.total > group.items.len() as u64;
	group.validate().ok()?;
	Some(group)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn member_details_are_current_scoped_and_byte_bounded() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let user = state.user.clone().unwrap();
		state.members = Some(model::MemberList {
			guild: Some(guild),
			channel,
			request: state.member_request,
			start: 0,
			slots: (1..=30)
				.map(|id| {
					Some(MemberSlot::Person(model::Member {
						user: model::User {
							id: model::Id(id + 9000),
							..user.clone()
						},
						nick: Some("Nick".repeat(80)),
						roles: (1..=40).map(model::Id).collect(),
						status: None,
						custom_status: None,
						clients: model::ClientPlatforms::default(),
						activities: vec![],
					}))
				})
				.collect(),
			total: 30,
			lazy: true,
			freshness: Freshness::Fresh,
			groups: vec![],
			ranges: vec![],
		});
		let value = snapshot(&state).unwrap();
		assert!(value.truncated);
		assert!(value.items.len() <= MAX_MEMBER_DETAILS);
		assert!(
			value
				.items
				.iter()
				.all(|m| m.role_ids.len() <= MAX_MEMBER_DETAIL_ROLES
					&& m.roles_truncated
					&& m.profile.is_none())
		);
		assert!(serde_json::to_vec(&value).unwrap().len() <= MAX_MEMBER_DETAILS_BYTES);
		value.validate().unwrap();

		let member = match state.members.as_ref().unwrap().slots[0].as_ref().unwrap() {
			MemberSlot::Person(m) => m.user.clone(),
			_ => unreachable!(),
		};
		state.profile = Some(client_core::profile::ProfileView {
			user: member.id,
			guild: Some(guild),
			request: state.profile_request,
			loading: false,
			error: None,
			data: Some(model::UserProfile {
				user: member,
				username: "User".into(),
				global_name: Some("Display".into()),
				banner: None,
				accent_color: None,
				bio: String::new(),
				pronouns: String::new(),
				badges: vec![],
				connections: vec![],
				mutual_guilds: vec![],
				guild: Some(model::GuildProfile {
					guild,
					roles: vec![],
					nick: Some("Server nick".into()),
					avatar: None,
					banner: None,
					bio: "Server bio".into(),
					pronouns: "they/them".into(),
					joined_at: Some("2026-01-01T00:00:00Z".into()),
				}),
				theme_colors: None,
				clan: None,
				limited: false,
			}),
		});
		assert_eq!(
			snapshot(&state).unwrap().items[0]
				.profile
				.as_ref()
				.unwrap()
				.bio,
			"Server bio"
		);
		let package = parse_package(include_bytes!(
			"../../../examples/extensions/packages/guild-inspector.serein-extension"
		))
		.unwrap();
		let invocation = Invocation {
			action: "show".into(),
			app: Some(Box::new(AppSnapshot {
				member_details: snapshot(&state),
				..Default::default()
			})),
			..Default::default()
		};
		let output = invoke(&package, &invocation)
			.expect("packaged Guild Inspector must accept the collected member metadata within existing sandbox limits");
		output.validate(&package.manifest, &invocation).unwrap();
		assert!(!output.panel.is_empty() && output.effects.is_empty());
		state.profile.as_mut().unwrap().guild = Some(model::Id(999));
		assert!(snapshot(&state).unwrap().items[0].profile.is_none());
		state.members.as_mut().unwrap().guild = Some(model::Id(999));
		assert!(snapshot(&state).is_none());
		state.members.as_mut().unwrap().guild = Some(guild);
		state.freshness = Freshness::Stale;
		assert!(snapshot(&state).is_none());
	}
}
