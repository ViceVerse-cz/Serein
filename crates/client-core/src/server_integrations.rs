//! On-demand integration data shares the scoped server administration request lane.
use crate::State;
use model::{
	Id, permissions as p,
	server_integrations::{Action, Snapshot, Webhook},
};

impl State {
	pub fn can_open_integration_settings(&self, guild: Id) -> bool {
		self.can_manage_guild(guild) || self.can_manage_guild_webhooks(guild)
	}
	pub fn can_manage_guild_webhooks(&self, guild: Id) -> bool {
		self.guild_permission(guild, p::MANAGE_WEBHOOKS)
	}
	pub fn can_manage_webhook_channel(&self, guild: Id, channel: Id) -> bool {
		self.channel(channel).is_some_and(|channel| {
			channel.guild == Some(guild) && matches!(channel.kind, 0 | 5 | 15 | 16)
		}) && self.permission(channel, p::VIEW_CHANNEL | p::MANAGE_WEBHOOKS) == Some(true)
	}

	fn integration_webhook(&self, guild: Id, scope: Option<Id>, id: Id) -> Option<&Webhook> {
		(self.server_admin.guild == Some(guild)).then_some(())?;
		self.server_admin
			.integrations
			.as_ref()
			.filter(|page| page.channel == scope)?
			.webhooks
			.as_ref()?
			.iter()
			.find(|hook| hook.id == id && hook.guild == guild)
	}
	pub(crate) fn integration_action_allowed(&self, guild: Id, action: &Action) -> bool {
		let webhook_access = action.scope().map_or_else(
			|| self.can_manage_guild_webhooks(guild),
			|channel| self.can_manage_webhook_channel(guild, channel),
		);
		match action {
			Action::Load {
				integrations,
				webhooks,
				..
			} => {
				(*integrations || *webhooks)
					&& (!integrations || self.can_manage_guild(guild))
					&& (!webhooks || webhook_access)
			}
			Action::CreateWebhook { channel, .. } => {
				webhook_access && self.can_manage_webhook_channel(guild, *channel)
			}
			Action::EditWebhook {
				webhook, channel, ..
			}
			| Action::CopyWebhookUrl {
				webhook, channel, ..
			} => {
				webhook_access
					&& self.can_manage_webhook_channel(guild, *channel)
					&& self
						.integration_webhook(guild, action.scope(), *webhook)
						.is_some_and(|hook| {
							hook.kind == 1
								&& (!matches!(action, Action::CopyWebhookUrl { .. })
									|| hook.channel == Some(*channel))
								&& hook.channel.is_some_and(|channel| {
									self.can_manage_webhook_channel(guild, channel)
								})
						})
			}
			Action::DeleteWebhook { webhook, .. } => {
				webhook_access
					&& self
						.integration_webhook(guild, action.scope(), *webhook)
						.is_some_and(|hook| {
							(1..=3).contains(&hook.kind)
								&& hook.channel.is_some_and(|channel| {
									self.can_manage_webhook_channel(guild, channel)
								})
						})
			}
			Action::DeleteIntegration { integration } => {
				self.can_manage_guild(guild)
					&& self.server_admin.guild == Some(guild)
					&& self
						.server_admin
						.integrations
						.as_ref()
						.and_then(|page| page.integrations.as_ref())
						.is_some_and(|items| items.iter().any(|item| item.id == *integration))
			}
		}
	}
	pub(crate) fn apply_integrations(&mut self, mut page: Snapshot, action: &Action) {
		if let Some(old) = &mut self.server_admin.integrations
			&& old.channel == page.channel
			&& !matches!(action, Action::Load { .. })
		{
			if page.integrations.is_none() {
				page.integrations = old.integrations.take();
			}
			if page.webhooks.is_none() && !matches!(action, Action::DeleteIntegration { .. }) {
				page.webhooks = old.webhooks.take();
			}
		}
		if !page.valid() {
			self.server_admin.integrations = None;
			self.server_admin.needs_refresh = true;
			self.server_admin.error =
				Some("Integration data exceeded safe bounds; reload to continue");
			return;
		}
		self.server_admin.integrations = Some(page);
	}
	pub(crate) fn prune_integration_access(&mut self, guild: Id) {
		let integrations = self.can_manage_guild(guild);
		let webhooks = self.server_admin.integrations.as_ref().is_some_and(|page| {
			page.channel.map_or_else(
				|| self.can_manage_guild_webhooks(guild),
				|channel| self.can_manage_webhook_channel(guild, channel),
			)
		});
		if let Some(page) = &mut self.server_admin.integrations {
			if !integrations {
				page.integrations = None;
			}
			if !webhooks {
				page.webhooks = None;
			}
			if !integrations && !webhooks {
				self.server_admin.integrations = None;
			}
		}
	}
}

pub(crate) fn expected(action: &Action, page: &Snapshot) -> bool {
	if !page.matches_action(action) {
		return false;
	}
	match action {
		Action::CopyWebhookUrl { .. } => false,
		Action::Load {
			integrations,
			webhooks,
			..
		} => page.integrations.is_some() == *integrations && page.webhooks.is_some() == *webhooks,
		Action::DeleteIntegration { integration } => {
			page.integrations
				.as_ref()
				.is_some_and(|items| items.iter().all(|item| item.id != *integration))
				&& page.webhooks.is_none()
		}
		Action::DeleteWebhook { webhook, .. } => {
			page.webhooks
				.as_ref()
				.is_some_and(|items| items.iter().all(|item| item.id != *webhook))
				&& page.integrations.is_none()
		}
		Action::EditWebhook {
			webhook,
			channel,
			name,
			scope,
		} => {
			if scope.is_some_and(|scope| scope != *channel) {
				return page
					.webhooks
					.as_ref()
					.is_some_and(|items| items.iter().all(|item| item.id != *webhook));
			}
			page.webhooks.as_ref().is_some_and(|items| {
				items.iter().any(|item| {
					item.id == *webhook
						&& item.channel == Some(*channel)
						&& item.name.as_ref() == Some(name)
				})
			}) && page.integrations.is_none()
		}
		Action::CreateWebhook { channel, name, .. } => {
			page.webhooks.as_ref().is_some_and(|items| {
				items.iter().any(|item| {
					item.kind == 1
						&& item.channel == Some(*channel)
						&& item.name.as_ref() == Some(name)
				})
			}) && page.integrations.is_none()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Command, Envelope, Event, auth::AuthState, server_admin};
	use model::server_admin::{Action as AdminAction, Result as Outcome};
	fn state(bits: u128) -> State {
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
			channels: vec![model::Channel {
				id: Id(3),
				guild: Some(Id(2)),
				parent_id: None,
				kind: 0,
				name: "chat".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..Default::default()
		};
		state.permissions.guilds.insert(
			Id(2),
			p::Guild {
				id: Id(2),
				owner: Some(Id(99)),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
				roles: Some(vec![p::Role {
					id: Id(2),
					name: "@everyone".into(),
					bits,
					color: 0,
					position: 0,
					hoist: false,
				}]),
			},
		);
		state.permissions.channels.insert(
			Id(3),
			p::Channel {
				id: Id(3),
				guild: Id(2),
				overwrites: Some(vec![]),
			},
		);
		state
	}
	fn page() -> Snapshot {
		Snapshot {
			channel: None,
			guild: Id(2),
			integrations: None,
			webhooks: Some(vec![Webhook {
				id: Id(4),
				guild: Id(2),
				channel: Some(Id(3)),
				kind: 1,
				name: Some("Updates".into()),
				avatar: None,
				application_id: None,
				user: None,
				source_guild: None,
				source_channel: None,
			}]),
		}
	}
	fn deliver(state: &mut State, request: u64, result: Result<Outcome, crate::auth::Failure>) {
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(server_admin::Event {
				guild: Id(2),
				request,
				result,
			}),
		});
	}
	#[test]
	fn webhook_url_copy_is_one_shot_and_fenced_on_close_permissions_and_disconnect() {
		for scope in [None, Some(Id(3))] {
			for cancelled in 0..6 {
				let mut state = state(p::VIEW_CHANNEL | p::MANAGE_WEBHOOKS);
				state.server_admin.guild = Some(Id(2));
				let mut snapshot = page();
				snapshot.channel = scope;
				state.server_admin.integrations = Some(snapshot);
				let action = AdminAction::Integrations(Action::CopyWebhookUrl {
					scope,
					webhook: Id(4),
					channel: Id(3),
				});
				assert!(!action.write());
				let Command::ServerAdmin { request, .. } =
					state.request_server_admin(Id(2), action).unwrap()
				else {
					panic!()
				};
				match cancelled {
					1 => state.clear_webhook_url(),
					2 => state.close_server_admin(),
					3 => state.cancel_server_admin(),
					4 => {
						state
							.server_admin
							.permissions_changed(&crate::permissions::Event::Owner {
								guild: Id(2),
								owner: model::Patch::Value(Id(99)),
							})
					}
					5 => {
						state.permissions.guilds.clear();
					}
					_ => {}
				}
				let url = model::server_integrations::WebhookUrl::new(
					Id(2),
					Id(4),
					Id(3),
					"SYNTHETIC_TOKEN",
				)
				.unwrap();
				deliver(&mut state, request, Ok(Outcome::WebhookUrl(url)));
				assert_eq!(
					state.take_webhook_url(Id(2), scope).is_some(),
					cancelled == 0
				);
				assert!(state.take_webhook_url(Id(2), scope).is_none());
			}
		}
	}

	#[test]
	fn channel_integrations_keep_permission_and_response_scope() {
		let mut state = state(p::VIEW_CHANNEL);
		state
			.permissions
			.channels
			.get_mut(&Id(3))
			.unwrap()
			.overwrites = Some(vec![p::Overwrite {
			id: Id(1),
			kind: 1,
			allow: p::MANAGE_WEBHOOKS,
			deny: 0,
		}]);
		assert!(state.can_open_channel_settings(Id(3)));
		assert!(!state.can_manage_channel(Id(3)));
		assert!(
			state
				.request_channel_action(
					Id(3),
					crate::channel_actions::Action::Edit {
						before: crate::channel_actions::Edit {
							name: "chat".into(),
							..Default::default()
						},
						after: crate::channel_actions::Edit::default()
					}
				)
				.is_none()
		);
		assert!(!state.can_open_integration_settings(Id(2)));
		let load = Action::Load {
			channel: Some(Id(3)),
			integrations: false,
			webhooks: true,
		};
		let Command::ServerAdmin { request, .. } = state
			.request_server_admin(Id(2), AdminAction::Integrations(load.clone()))
			.unwrap()
		else {
			panic!()
		};
		let mut scoped = page();
		scoped.channel = Some(Id(3));
		assert!(scoped.valid());
		assert!(!page().matches_action(&load));
		let mut wrong = scoped.clone();
		wrong.webhooks.as_mut().unwrap()[0].channel = Some(Id(9));
		assert!(!wrong.valid());
		deliver(
			&mut state,
			request,
			Ok(Outcome::Integrations(scoped.clone())),
		);
		state.prune_integration_access(Id(2));
		assert!(
			state
				.server_admin
				.integrations
				.as_ref()
				.unwrap()
				.webhooks
				.is_some()
		);
		let edit = Action::EditWebhook {
			scope: Some(Id(3)),
			webhook: Id(4),
			channel: Id(3),
			name: "News".into(),
		};
		assert!(state.integration_action_allowed(Id(2), &edit));
		assert!(state.integration_action_allowed(
			Id(2),
			&Action::DeleteWebhook {
				scope: Some(Id(3)),
				webhook: Id(4)
			}
		));
		assert!(!state.integration_action_allowed(
			Id(2),
			&Action::DeleteWebhook {
				scope: None,
				webhook: Id(4)
			}
		));
		assert!(!state.integration_action_allowed(
			Id(2),
			&Action::DeleteWebhook {
				scope: Some(Id(9)),
				webhook: Id(4)
			}
		));
		let moved = Action::EditWebhook {
			scope: Some(Id(3)),
			webhook: Id(4),
			channel: Id(9),
			name: "News".into(),
		};
		assert!(!state.integration_action_allowed(Id(2), &moved));
		assert!(!expected(&moved, &scoped));
		scoped.webhooks.as_mut().unwrap().clear();
		assert!(expected(&moved, &scoped));
		let Command::ServerAdmin { request, .. } = state
			.request_server_admin(Id(2), AdminAction::Integrations(load))
			.unwrap()
		else {
			panic!()
		};
		deliver(&mut state, request, Ok(Outcome::Integrations(page())));
		assert!(state.server_admin.needs_refresh);
		assert_eq!(
			state.server_admin.integrations.as_ref().unwrap().channel,
			Some(Id(3))
		);
		state
			.permissions
			.channels
			.get_mut(&Id(3))
			.unwrap()
			.overwrites = Some(vec![]);
		state.permissions.clear_cache();
		state.prune_integration_access(Id(2));
		assert!(state.server_admin.integrations.is_none());
	}
	#[test]
	fn integrations_permissions_scope_stale_responses_and_uncertain_writes() {
		let mut state = state(p::MANAGE_WEBHOOKS | p::VIEW_CHANNEL);
		assert!(state.can_open_integration_settings(Id(2)));
		assert!(!state.integration_action_allowed(
			Id(2),
			&Action::Load {
				channel: None,
				integrations: true,
				webhooks: true
			}
		));
		let load = AdminAction::Integrations(Action::Load {
			channel: None,
			integrations: false,
			webhooks: true,
		});
		let Command::ServerAdmin { request, .. } =
			state.request_server_admin(Id(2), load.clone()).unwrap()
		else {
			panic!()
		};
		deliver(&mut state, request + 1, Ok(Outcome::Integrations(page())));
		assert!(state.server_admin.pending);
		deliver(&mut state, request, Ok(Outcome::Integrations(page())));
		assert!(state.server_admin.integrations.is_some());
		let edit = Action::EditWebhook {
			scope: None,
			webhook: Id(4),
			channel: Id(3),
			name: "News".into(),
		};
		assert!(state.integration_action_allowed(Id(2), &edit));
		assert!(!state.integration_action_allowed(
			Id(2),
			&Action::DeleteWebhook {
				scope: None,
				webhook: Id(999)
			}
		));
		assert!(!state.integration_action_allowed(
			Id(2),
			&Action::CreateWebhook {
				scope: None,
				channel: Id(999),
				name: "News".into()
			}
		));
		let Command::ServerAdmin { request, .. } = state
			.request_server_admin(Id(2), AdminAction::Integrations(edit.clone()))
			.unwrap()
		else {
			panic!()
		};
		deliver(&mut state, request, Err(crate::auth::Failure::Ambiguous));
		assert!(state.server_admin.needs_refresh);
		assert!(
			state
				.request_server_admin(Id(2), AdminAction::Integrations(edit))
				.is_none()
		);
		let Command::ServerAdmin { request, .. } = state.request_server_admin(Id(2), load).unwrap()
		else {
			panic!()
		};
		let mut role = state.permissions.guilds[&Id(2)].roles.as_ref().unwrap()[0].clone();
		role.bits = p::VIEW_CHANNEL;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Role { guild: Id(2), role }),
		});
		deliver(&mut state, request, Ok(Outcome::Integrations(page())));
		assert!(state.server_admin.integrations.is_none());
		assert!(!state.can_open_integration_settings(Id(2)));
	}
}
