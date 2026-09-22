//! Extension server controls reuse the same permission and stale-state admission as the UI.
use crate::MessagingUi;
use client_core::{Command, State};
use extensions::{AppAction, RolePatch, ServerSettingsPatch};
use model::{Id, Patch, server_admin, server_roles, server_settings};

fn id(value: &str) -> Result<Id, String> {
	value
		.parse::<u64>()
		.ok()
		.filter(|value| *value != 0)
		.map(Id)
		.ok_or_else(|| "Invalid identifier".into())
}

fn optional_id(value: Option<String>, clear: bool) -> Result<Patch<Id>, String> {
	if clear {
		Ok(Patch::Null)
	} else {
		value
			.map(|value| id(&value).map(Patch::Value))
			.transpose()
			.map(|value| value.unwrap_or(Patch::Absent))
	}
}

fn mask(value: Option<String>) -> Result<Option<u128>, String> {
	value
		.map(|value| value.parse().map_err(|_| "Invalid permission mask".into()))
		.transpose()
}

fn settings_edit(value: ServerSettingsPatch) -> Result<server_settings::Edit, String> {
	Ok(server_settings::Edit {
		name: value.name,
		icon: Patch::Absent,
		banner_color: value.banner_color,
		traits: value.traits.map(|values| {
			values
				.into_iter()
				.map(|value| server_settings::Trait {
					label: value.label,
					emoji: value.emoji,
				})
				.collect()
		}),
		description: value.description,
		system_channel_id: optional_id(value.system_channel_id, value.clear_system_channel)?,
		system_channel_flags: value.system_channel_flags,
		activity_feed: value.activity_feed,
		default_message_notifications: value.default_message_notifications,
		afk_channel_id: optional_id(value.afk_channel_id, value.clear_afk_channel)?,
		afk_timeout: value.afk_timeout,
	})
}

fn role_edit(value: RolePatch) -> Result<server_roles::Edit, String> {
	if value.primary_color.is_none()
		&& (value.secondary_color.is_some() || value.tertiary_color.is_some())
		|| value.permissions.is_none() && value.permission_mask.is_some()
	{
		return Err("Incomplete role patch".into());
	}
	let permissions = mask(value.permissions)?;
	let permission_mask = mask(value.permission_mask)?.unwrap_or(u128::MAX);
	Ok(server_roles::Edit {
		name: value.name,
		colors: value.primary_color.map(|primary| server_roles::Colors {
			primary,
			secondary: value.secondary_color,
			tertiary: value.tertiary_color,
		}),
		permissions,
		permission_mask: permissions.map_or(0, |_| permission_mask),
		hoist: value.hoist,
		mentionable: value.mentionable,
		icon: Patch::Absent,
		unicode_emoji: if value.clear_unicode_emoji {
			Patch::Null
		} else {
			value.unicode_emoji.map_or(Patch::Absent, Patch::Value)
		},
	})
}

fn queue(command: Option<Command>, commands: &mut Vec<Command>) -> Result<(), String> {
	commands.push(command.ok_or(
		"This action is unavailable with the current permissions, loaded data or pending operation",
	)?);
	Ok(())
}

impl MessagingUi {
	pub(crate) fn apply_extension_server_action(
		&mut self,
		state: &mut State,
		action: AppAction,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		action.validate().map_err(|error| error.to_string())?;
		let command = match action {
			AppAction::UpdateServerSettings { guild_id, settings } => {
				let guild = id(&guild_id)?;
				if state.server_settings.guild != Some(guild) {
					return Err("Load this server's settings before changing them".into());
				}
				state.save_server_settings(settings_edit(settings)?)
			}
			AppAction::CreateRole { guild_id, role } => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::Roles(server_roles::Action::Create(role_edit(role)?)),
			),
			AppAction::EditRole {
				guild_id,
				role_id,
				role,
			} => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::Roles(server_roles::Action::Edit {
					id: id(&role_id)?,
					edit: role_edit(role)?,
				}),
			),
			AppAction::DeleteRole { guild_id, role_id } => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::Roles(server_roles::Action::Delete(id(&role_id)?)),
			),
			AppAction::MoveRole {
				guild_id,
				role_id,
				position,
			} => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::Roles(server_roles::Action::Move {
					id: id(&role_id)?,
					position,
				}),
			),
			AppAction::SetMemberRole {
				guild_id,
				user_id,
				role_id,
				assigned,
			} => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::SetRole {
					user: id(&user_id)?,
					role: id(&role_id)?,
					assigned,
				},
			),
			AppAction::SetMemberNickname {
				guild_id,
				user_id,
				nickname,
			} => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::SetNickname {
					user: id(&user_id)?,
					nick: nickname,
				},
			),
			AppAction::KickMember { guild_id, user_id } => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::Kick {
					user: id(&user_id)?,
				},
			),
			AppAction::PruneMembers {
				guild_id,
				days,
				execute,
			} => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::Prune { days, execute },
			),
			AppAction::SetMemberListVisible { guild_id, enabled } => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::ShowMembers { enabled },
			),
			AppAction::RenameServerEmoji {
				guild_id,
				emoji_id,
				name,
			} => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::RenameEmoji {
					id: id(&emoji_id)?,
					name,
				},
			),
			AppAction::DeleteServerEmoji { guild_id, emoji_id } => state.request_server_admin(
				id(&guild_id)?,
				server_admin::Action::DeleteEmoji { id: id(&emoji_id)? },
			),
			_ => return Err("Unsupported app action".into()),
		};
		queue(command, commands)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn patches_map_to_native_bounded_models() {
		let edit = role_edit(RolePatch {
			name: Some("Helpers".into()),
			permissions: Some("8".into()),
			permission_mask: Some("12".into()),
			primary_color: Some(0x12_34_56),
			unicode_emoji: Some("🧰".into()),
			..Default::default()
		})
		.unwrap();
		assert_eq!(edit.permissions, Some(8));
		assert_eq!(edit.permission_mask, 12);
		assert_eq!(edit.colors.unwrap().primary, 0x12_34_56);
		assert_eq!(edit.unicode_emoji, Patch::Value("🧰".into()));
		assert!(edit.valid());

		let settings = settings_edit(ServerSettingsPatch {
			name: Some("SDK server".into()),
			clear_system_channel: true,
			afk_channel_id: Some("21".into()),
			afk_timeout: Some(300),
			..Default::default()
		})
		.unwrap();
		assert_eq!(settings.system_channel_id, Patch::Null);
		assert_eq!(settings.afk_channel_id, Patch::Value(Id(21)));
		assert!(settings.valid());
	}

	#[test]
	fn unloaded_server_controls_are_rejected_by_native_admission() {
		let mut state = test_support::demo_state();
		let mut commands = Vec::new();
		assert!(
			MessagingUi::default()
				.apply_extension_server_action(
					&mut state,
					AppAction::KickMember {
						guild_id: "10".into(),
						user_id: "2".into(),
					},
					&mut commands,
				)
				.is_err()
		);
		assert!(commands.is_empty());
	}
}
