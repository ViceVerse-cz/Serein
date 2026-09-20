use crate::State;
use model::{
	Id, Patch, permissions as p,
	server_roles::{Action, Edit, Role},
};

impl State {
	pub fn can_open_role_settings(&self, guild: Id) -> bool {
		self.guild_permission(guild, p::MANAGE_ROLES)
	}
	pub fn can_create_guild_role(&self, guild: Id) -> bool {
		self.can_open_role_settings(guild)
			&& self.server_admin.guild == Some(guild)
			&& self
				.server_admin
				.roles
				.as_ref()
				.is_some_and(|catalog| catalog.items.len() < 250)
	}
	fn settings_role(&self, guild: Id, role: Id) -> Option<&Role> {
		(self.server_admin.guild == Some(guild)).then_some(())?;
		self.server_admin
			.roles
			.as_ref()
			.filter(|catalog| catalog.guild == guild)?
			.items
			.iter()
			.find(|value| value.id == role)
	}
	fn own_highest_role(&self, guild: Id) -> Option<&p::Role> {
		let metadata = self.permissions.guilds.get(&guild)?;
		let member = metadata.member.as_ref()?;
		let roles = metadata.roles.as_ref()?;
		let mut highest = roles.iter().find(|role| role.id == guild)?;
		for id in &member.roles {
			let role = roles.iter().find(|role| role.id == *id)?;
			if role.cmp_hierarchy(highest).is_gt() {
				highest = role;
			}
		}
		Some(highest)
	}
	fn current_role(&self, guild: Id, role: Id) -> Option<&p::Role> {
		self.permissions
			.guilds
			.get(&guild)?
			.roles
			.as_ref()?
			.iter()
			.find(|value| value.id == role)
	}
	fn owns_guild(&self, guild: Id) -> bool {
		self.user.as_ref().is_some_and(|user| {
			self.permissions
				.guilds
				.get(&guild)
				.is_some_and(|metadata| metadata.owner == Some(user.id))
		})
	}
	pub fn can_edit_guild_role(&self, guild: Id, role: Id) -> bool {
		if !self.can_open_role_settings(guild) {
			return false;
		}
		let Some(target) = self.settings_role(guild, role) else {
			return false;
		};
		let Some(current) = self.current_role(guild, role) else {
			return false;
		};
		!target.managed
			&& (role == guild
				|| self.owns_guild(guild)
				|| self
					.own_highest_role(guild)
					.is_some_and(|own| own.cmp_hierarchy(current).is_gt()))
	}
	pub fn can_delete_guild_role(&self, guild: Id, role: Id) -> bool {
		role != guild && self.can_edit_guild_role(guild, role)
	}
	pub fn can_move_guild_role(&self, guild: Id, role: Id, position: i32) -> bool {
		self.can_delete_guild_role(guild, role)
			&& position >= 1
			&& self.server_admin.roles.as_ref().is_some_and(|catalog| {
				catalog
					.items
					.iter()
					.map(|role| role.position)
					.max()
					.is_some_and(|max| position <= max)
			}) && (self.owns_guild(guild)
			|| self
				.own_highest_role(guild)
				.is_some_and(|own| position < own.position))
	}
	pub fn can_edit_role_icon(&self, guild: Id, role: Id) -> bool {
		role != guild
			&& self.can_edit_guild_role(guild, role)
			&& self.role_feature(guild, "ROLE_ICONS")
	}
	fn role_feature(&self, guild: Id, feature: &str) -> bool {
		self.server_admin.guild == Some(guild)
			&& self.server_admin.roles.as_ref().is_some_and(|catalog| {
				catalog.guild == guild && catalog.features.iter().any(|value| value == feature)
			})
	}
	pub fn can_use_enhanced_role_colors(&self, guild: Id) -> bool {
		self.can_open_role_settings(guild) && self.role_feature(guild, "ENHANCED_ROLE_COLORS")
	}
	pub fn can_grant_role_permission(&self, guild: Id, bits: u128) -> bool {
		self.guild_permission(guild, bits)
	}
	fn role_edit_allowed(&self, guild: Id, role: Option<Id>, edit: &Edit) -> bool {
		if !edit.valid() || role == Some(guild) && !edit.only_permissions() {
			return false;
		}
		let previous = role.and_then(|role| self.current_role(guild, role));
		let new_bits = edit.permissions.map_or(0, |bits| {
			if let Some(previous) = previous {
				(bits & edit.permission_mask) & !previous.bits
			} else {
				bits
			}
		});
		if !self.can_grant_role_permission(guild, new_bits) {
			return false;
		}
		if edit
			.colors
			.is_some_and(|colors| colors.secondary.is_some() || colors.tertiary.is_some())
			&& !self.can_use_enhanced_role_colors(guild)
		{
			return false;
		}
		if (matches!(edit.icon, Patch::Value(_)) || matches!(edit.unicode_emoji, Patch::Value(_)))
			&& !self.role_feature(guild, "ROLE_ICONS")
		{
			return false;
		}
		true
	}
	pub(crate) fn role_action_allowed(&self, guild: Id, action: &Action) -> bool {
		if !self.can_open_role_settings(guild) || !action.valid() {
			return false;
		}
		match action {
			Action::Load => true,
			Action::Create(edit) => {
				self.can_create_guild_role(guild) && self.role_edit_allowed(guild, None, edit)
			}
			Action::Edit { id, edit } => {
				self.can_edit_guild_role(guild, *id)
					&& self.role_edit_allowed(guild, Some(*id), edit)
			}
			Action::Delete(id) => self.can_delete_guild_role(guild, *id),
			Action::Move { id, position } => self.can_move_guild_role(guild, *id, *position),
			Action::Members { role, .. } => {
				self.can_open_member_settings(guild)
					&& role.is_none_or(|role| self.settings_role(guild, role).is_some())
			}
		}
	}
	pub(crate) fn roles_catalog_matches_permissions(
		&self,
		catalog: &model::server_roles::Catalog,
	) -> bool {
		self.permissions
			.guilds
			.get(&catalog.guild)
			.and_then(|guild| guild.roles.as_ref())
			.is_some_and(|roles| {
				roles.len() == catalog.items.len()
					&& catalog.items.iter().all(|role| {
						roles
							.iter()
							.find(|current| current.id == role.id)
							.is_some_and(|current| {
								current.name == role.name
									&& current.bits == role.permissions
									&& current.position == role.position
									&& current.color == role.colors.primary
									&& current.hoist == role.hoist
							})
					})
			})
	}
	pub(crate) fn apply_roles_catalog(
		&mut self,
		catalog: model::server_roles::Catalog,
		selected: Option<Id>,
	) {
		let guild = catalog.guild;
		if let Some(mut metadata) = self.permissions.guilds.get(&guild).cloned() {
			metadata.roles = Some(catalog.items.iter().map(Role::permission_role).collect());
			if self
				.update_permissions(crate::permissions::Event::Guild(metadata))
				.is_err()
			{
				self.fail(crate::auth::Failure::Capacity);
				return;
			}
		}
		if let Some(page) = &mut self.server_admin.members {
			page.roles = catalog
				.items
				.iter()
				.map(|role| model::server_admin::Role {
					role: role.permission_role(),
					managed: role.managed,
				})
				.collect();
			for member in &mut page.items {
				member
					.roles
					.retain(|id| catalog.items.iter().any(|role| role.id == *id));
			}
		}
		self.permissions.clear_cache();
		self.invalidate_navigation();
		self.server_admin.selected_role = selected
			.or(self.server_admin.selected_role)
			.filter(|id| catalog.items.iter().any(|role| role.id == *id));
		self.server_admin.roles = Some(catalog);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Command, Envelope, Event, auth::AuthState};
	use model::{
		server_admin,
		server_roles::{Catalog, Colors},
	};
	fn state() -> State {
		let roles = vec![
			Role {
				id: Id(2),
				name: "@everyone".into(),
				..Role::default()
			},
			Role {
				id: Id(3),
				name: "Manager".into(),
				position: 3,
				permissions: p::MANAGE_ROLES | p::MANAGE_GUILD | p::VIEW_CHANNEL,
				..Role::default()
			},
			Role {
				id: Id(4),
				name: "Member".into(),
				position: 1,
				permissions: 1 << 110,
				..Role::default()
			},
			Role {
				id: Id(5),
				name: "Integration".into(),
				position: 2,
				managed: true,
				..Role::default()
			},
		];
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				primary_guild: None,
				id: Id(1),
				name: "Synthetic".into(),
				avatar: None,
				discriminator: 0,
				kind: Default::default(),
				webhook: false,
			}),
			guilds: vec![model::Guild {
				stickers: None,
				id: Id(2),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			..State::default()
		};
		state.permissions.guilds.insert(
			Id(2),
			p::Guild {
				id: Id(2),
				owner: Some(Id(99)),
				roles: Some(roles.iter().map(Role::permission_role).collect()),
				member: Some(p::Member {
					roles: vec![Id(3)],
					timeout_until: None,
				}),
			},
		);
		state.server_admin.guild = Some(Id(2));
		state.server_admin.roles = Some(Catalog {
			guild: Id(2),
			items: roles,
			features: vec![],
		});
		state
	}
	#[test]
	fn role_edit_gates_hierarchy_defaults_entitlements_and_new_grants() {
		let state = state();
		assert!(state.can_edit_guild_role(Id(2), Id(4)));
		assert!(!state.can_edit_guild_role(Id(2), Id(3)));
		assert!(!state.can_edit_guild_role(Id(2), Id(5)));
		assert!(state.can_move_guild_role(Id(2), Id(4), 2));
		assert!(!state.can_move_guild_role(Id(2), Id(4), 3));
		assert!(!state.can_delete_guild_role(Id(2), Id(2)));
		assert!(!state.role_action_allowed(
			Id(2),
			&Action::Edit {
				id: Id(2),
				edit: Edit {
					name: Some("rename default".into()),
					..Edit::default()
				}
			}
		));
		assert!(state.role_action_allowed(
			Id(2),
			&Action::Edit {
				id: Id(2),
				edit: Edit {
					permissions: Some(p::VIEW_CHANNEL),
					permission_mask: p::VIEW_CHANNEL,
					..Edit::default()
				}
			}
		));
		assert!(!state.role_action_allowed(
			Id(2),
			&Action::Edit {
				id: Id(4),
				edit: Edit {
					permissions: Some(p::ADMINISTRATOR),
					permission_mask: p::ADMINISTRATOR,
					..Edit::default()
				}
			}
		));
		assert!(state.role_action_allowed(
			Id(2),
			&Action::Edit {
				id: Id(4),
				edit: Edit {
					permissions: Some(0),
					permission_mask: 1 << 110,
					..Edit::default()
				}
			}
		));
		assert!(!state.role_action_allowed(
			Id(2),
			&Action::Edit {
				id: Id(4),
				edit: Edit {
					colors: Some(Colors {
						primary: 1,
						secondary: Some(2),
						tertiary: None
					}),
					..Edit::default()
				}
			}
		));
		assert!(!state.can_edit_role_icon(Id(2), Id(4)));
	}
	#[test]
	fn role_matching_gateway_confirmation_allows_catalog_completion() {
		let mut state = state();
		let edit = Edit {
			name: Some("Renamed".into()),
			..Default::default()
		};
		let Command::ServerAdmin { guild, request, .. } = state
			.request_server_admin(
				Id(2),
				server_admin::Action::Roles(Action::Edit { id: Id(4), edit }),
			)
			.unwrap()
		else {
			panic!()
		};
		let mut confirmed = state.server_admin.roles.as_ref().unwrap().clone();
		let changed = confirmed
			.items
			.iter_mut()
			.find(|role| role.id == Id(4))
			.unwrap();
		changed.name = "Renamed".into();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Role {
				guild,
				role: changed.permission_role(),
			}),
		});
		confirmed.items.reverse(); // Gateway and REST order are independent.
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(crate::server_admin::Event {
				guild,
				request,
				result: Ok(server_admin::Result::Roles(
					model::server_roles::Result::Catalog {
						catalog: confirmed,
						selected: Some(Id(4)),
					},
				)),
			}),
		});
		assert!(!state.server_admin.pending);
		assert!(!state.server_admin.needs_refresh);
		assert!(state.server_admin.error.is_none());
		assert_eq!(
			state
				.server_admin
				.roles
				.as_ref()
				.unwrap()
				.items
				.iter()
				.find(|role| role.id == Id(4))
				.unwrap()
				.name,
			"Renamed"
		);
		assert_eq!(state.server_admin.selected_role, Some(Id(4)));
	}
	#[test]
	fn role_gateway_move_delete_and_stale_catalog_cannot_restore_permissions() {
		let mut state = state();
		let mut moved = state
			.server_admin
			.roles
			.as_ref()
			.unwrap()
			.items
			.iter()
			.find(|role| role.id == Id(4))
			.unwrap()
			.permission_role();
		moved.position = 4;
		let Command::ServerAdmin { guild, request, .. } = state
			.request_server_admin(Id(2), server_admin::Action::Roles(Action::Load))
			.unwrap()
		else {
			panic!()
		};
		let stale = state.server_admin.roles.as_ref().unwrap().clone();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Role { guild, role: moved }),
		});
		assert!(!state.can_edit_guild_role(guild, Id(4)));
		let mut own = state
			.permissions
			.guilds
			.get(&guild)
			.unwrap()
			.roles
			.as_ref()
			.unwrap()
			.iter()
			.find(|role| role.id == Id(3))
			.unwrap()
			.clone();
		own.bits &= !p::MANAGE_GUILD;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Role { guild, role: own }),
		});
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(crate::server_admin::Event {
				guild,
				request,
				result: Ok(server_admin::Result::Roles(
					model::server_roles::Result::Catalog {
						catalog: stale,
						selected: None,
					},
				)),
			}),
		});
		assert!(!state.can_open_member_settings(guild));
		assert!(state.server_admin.needs_refresh);
		assert!(!state.can_edit_guild_role(guild, Id(4)));
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::RoleRemoved { guild, id: Id(4) }),
		});
		assert!(!state.can_edit_guild_role(guild, Id(4)));
	}
	#[test]
	fn role_assignment_completion_rejects_stale_results_and_invalidates_counts() {
		let mut state = state();
		let mut user = state.user.as_ref().unwrap().clone();
		user.id = Id(8);
		let mut member = server_admin::Member {
			user,
			nick: None,
			roles: vec![Id(4)],
			joined_at: None,
			join_source: None,
			invite_code: None,
			flags: None,
			unusual_dm_until: None,
			timeout_until: None,
		};
		let catalog = state.server_admin.roles.as_mut().unwrap();
		catalog
			.items
			.iter_mut()
			.find(|role| role.id == Id(4))
			.unwrap()
			.member_count = Some(10);
		state.server_admin.members = Some(server_admin::Members {
			items: vec![member.clone()],
			roles: catalog
				.items
				.iter()
				.map(|role| server_admin::Role {
					role: role.permission_role(),
					managed: role.managed,
				})
				.collect(),
			total: 10,
			..Default::default()
		});
		state.server_admin.member_role_filter = Some(Id(4));
		let Command::ServerAdmin { guild, request, .. } = state
			.request_server_admin(
				Id(2),
				server_admin::Action::SetRole {
					user: Id(8),
					role: Id(4),
					assigned: false,
				},
			)
			.unwrap()
		else {
			panic!()
		};
		member.roles.clear();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(crate::server_admin::Event {
				guild,
				request: request + 1,
				result: Ok(server_admin::Result::Member(member.clone())),
			}),
		});
		assert_eq!(state.server_admin.members.as_ref().unwrap().items.len(), 1);
		assert_eq!(
			state
				.server_admin
				.roles
				.as_ref()
				.unwrap()
				.items
				.iter()
				.find(|role| role.id == Id(4))
				.unwrap()
				.member_count,
			Some(10)
		);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(crate::server_admin::Event {
				guild,
				request,
				result: Ok(server_admin::Result::Member(member)),
			}),
		});
		assert!(
			state
				.server_admin
				.members
				.as_ref()
				.unwrap()
				.items
				.is_empty()
		);
		assert_eq!(state.server_admin.members.as_ref().unwrap().total, 9);
		assert_eq!(
			state
				.server_admin
				.roles
				.as_ref()
				.unwrap()
				.items
				.iter()
				.find(|role| role.id == Id(4))
				.unwrap()
				.member_count,
			None
		);
	}
	#[test]
	fn role_members_completion_requires_current_directory_permission() {
		let mut state = state();
		let Command::ServerAdmin { guild, request, .. } = state
			.request_server_admin(
				Id(2),
				server_admin::Action::Roles(Action::Members {
					role: Some(Id(4)),
					query: Default::default(),
				}),
			)
			.unwrap()
		else {
			panic!()
		};
		state
			.permissions
			.guilds
			.get_mut(&guild)
			.unwrap()
			.roles
			.as_mut()
			.unwrap()
			.iter_mut()
			.find(|role| role.id == Id(3))
			.unwrap()
			.bits &= !p::MANAGE_GUILD;
		state.permissions.clear_cache();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(crate::server_admin::Event {
				guild,
				request,
				result: Ok(server_admin::Result::Roles(
					model::server_roles::Result::Members {
						role: Some(Id(4)),
						page: Default::default(),
					},
				)),
			}),
		});
		assert!(state.server_admin.members.is_none());
		assert!(!state.server_admin.pending);
	}
	#[test]
	fn role_completion_is_scoped_and_revoked_permission_closes_editor() {
		let mut state = state();
		let Command::ServerAdmin { guild, request, .. } = state
			.request_server_admin(Id(2), server_admin::Action::Roles(Action::Load))
			.unwrap()
		else {
			panic!()
		};
		let catalog = state.server_admin.roles.as_ref().unwrap().clone();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(crate::server_admin::Event {
				guild,
				request: request + 1,
				result: Ok(server_admin::Result::Roles(
					model::server_roles::Result::Catalog {
						catalog: catalog.clone(),
						selected: Some(Id(4)),
					},
				)),
			}),
		});
		assert!(state.server_admin.pending);
		assert_eq!(state.server_admin.selected_role, None);
		state.permissions.guilds.get_mut(&Id(2)).unwrap().member = None;
		state.permissions.clear_cache();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(crate::server_admin::Event {
				guild,
				request,
				result: Ok(server_admin::Result::Roles(
					model::server_roles::Result::Catalog {
						catalog,
						selected: Some(Id(4)),
					},
				)),
			}),
		});
		assert!(state.server_admin.roles.is_none());
		assert!(state.server_admin.guild.is_none());
	}
}
