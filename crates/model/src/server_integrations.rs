//! On-demand integration metadata. Webhook execution secrets are never retained.
use crate::{Id, User};
pub const MAX_INTEGRATIONS: usize = 50;
pub const MAX_WEBHOOKS: usize = 1000;
pub const MAX_BYTES: usize = 1024 * 1024;
#[derive(Clone)]
pub struct Application {
	pub id: Id,
	pub name: String,
	pub icon: Option<String>,
	pub description: String,
	pub bot: Option<User>,
}
#[derive(Clone)]
pub struct Integration {
	pub id: Id,
	pub name: String,
	pub kind: String,
	pub enabled: bool,
	pub user: Option<User>,
	pub synced_at: Option<i128>,
	pub role_id: Option<Id>,
	pub application: Option<Application>,
}
#[derive(Clone)]
pub struct Source {
	pub id: Id,
	pub name: Option<String>,
}
#[derive(Clone)]
pub struct Webhook {
	pub id: Id,
	pub guild: Id,
	pub channel: Option<Id>,
	pub kind: u8,
	pub name: Option<String>,
	pub avatar: Option<String>,
	pub application_id: Option<Id>,
	pub user: Option<User>,
	pub source_guild: Option<Source>,
	pub source_channel: Option<Source>,
}
#[derive(Clone)]
pub struct Snapshot {
	pub guild: Id,
	/// None is guild-wide; Some restricts webhooks to this channel.
	pub channel: Option<Id>,
	/// None means this permission-scoped resource was not requested.
	pub integrations: Option<Vec<Integration>>,
	pub webhooks: Option<Vec<Webhook>>,
}
impl Integration {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.name.capacity()
			+ self.kind.capacity()
			+ self.user.as_ref().map_or(0, User::heap_bytes)
			+ self.application.as_ref().map_or(0, |app| {
				app.name.capacity()
					+ app.description.capacity()
					+ string_bytes(&app.icon)
					+ app.bot.as_ref().map_or(0, User::heap_bytes)
			})
	}
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& text(&self.name, 400)
			&& !self.kind.is_empty()
			&& text(&self.kind, 80)
			&& self.bytes() <= 8192
			&& user_valid(&self.user)
			&& self.role_id.is_none_or(|id| id.0 != 0)
			&& self
				.synced_at
				.is_none_or(|time| (0..=253_402_300_799_999_999_999_i128).contains(&time))
			&& self.application.as_ref().is_none_or(|app| {
				app.id.0 != 0
					&& text(&app.name, 400)
					&& app.description.len() <= 4096
					&& app.icon.as_deref().is_none_or(crate::valid_avatar_hash)
					&& user_valid(&app.bot)
			})
	}
}
impl Webhook {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ string_bytes(&self.name)
			+ string_bytes(&self.avatar)
			+ self.user.as_ref().map_or(0, User::heap_bytes)
			+ self
				.source_guild
				.as_ref()
				.map_or(0, |source| string_bytes(&source.name))
			+ self
				.source_channel
				.as_ref()
				.map_or(0, |source| string_bytes(&source.name))
	}
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& self.guild.0 != 0
			&& self.kind != 0
			&& self.bytes() <= 4096
			&& self.channel.is_none_or(|id| id.0 != 0)
			&& self.application_id.is_none_or(|id| id.0 != 0)
			&& self.name.as_ref().is_none_or(|name| text(name, 400))
			&& self.avatar.as_deref().is_none_or(crate::valid_avatar_hash)
			&& user_valid(&self.user)
			&& [&self.source_guild, &self.source_channel]
				.into_iter()
				.all(|source| {
					source.as_ref().is_none_or(|source| {
						source.id.0 != 0 && source.name.as_ref().is_none_or(|name| text(name, 400))
					})
				})
	}
}
impl Snapshot {
	pub fn matches_action(&self, action: &Action) -> bool {
		let (integrations, webhooks) = match action {
			Action::Load {
				integrations,
				webhooks,
				..
			} => (*integrations, *webhooks),
			Action::DeleteIntegration { .. } => (true, false),
			_ => (false, true),
		};
		self.channel == action.scope()
			&& self.integrations.is_some() == integrations
			&& self.webhooks.is_some() == webhooks
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.integrations.as_ref().map_or(0, |items| {
				items.iter().map(Integration::bytes).sum::<usize>()
					+ items.capacity().saturating_sub(items.len()) * size_of::<Integration>()
			}) + self.webhooks.as_ref().map_or(0, |items| {
			items.iter().map(Webhook::bytes).sum::<usize>()
				+ items.capacity().saturating_sub(items.len()) * size_of::<Webhook>()
		})
	}
	pub fn valid(&self) -> bool {
		let mut integrations = std::collections::BTreeSet::new();
		let mut webhooks = std::collections::BTreeSet::new();
		self.guild.0 != 0
			&& self
				.channel
				.is_none_or(|channel| channel.0 != 0 && self.integrations.is_none())
			&& self.bytes() <= MAX_BYTES
			&& self.integrations.as_ref().is_none_or(|items| {
				items.len() <= MAX_INTEGRATIONS
					&& items
						.iter()
						.all(|item| item.valid() && integrations.insert(item.id))
			}) && self.webhooks.as_ref().is_none_or(|items| {
			items.len() <= MAX_WEBHOOKS
				&& items.iter().all(|item| {
					item.valid()
						&& item.guild == self.guild
						&& self
							.channel
							.is_none_or(|channel| item.channel == Some(channel))
						&& webhooks.insert(item.id)
				})
		})
	}
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
	Load {
		channel: Option<Id>,
		integrations: bool,
		webhooks: bool,
	},
	CopyWebhookUrl {
		scope: Option<Id>,
		webhook: Id,
		channel: Id,
	},
	CreateWebhook {
		scope: Option<Id>,
		channel: Id,
		name: String,
	},
	EditWebhook {
		scope: Option<Id>,
		webhook: Id,
		channel: Id,
		name: String,
	},
	DeleteWebhook {
		scope: Option<Id>,
		webhook: Id,
	},
	DeleteIntegration {
		integration: Id,
	},
}
impl Action {
	pub fn scope(&self) -> Option<Id> {
		match self {
			Self::Load { channel, .. } => *channel,
			Self::CopyWebhookUrl { scope, .. }
			| Self::CreateWebhook { scope, .. }
			| Self::EditWebhook { scope, .. }
			| Self::DeleteWebhook { scope, .. } => *scope,
			Self::DeleteIntegration { .. } => None,
		}
	}

	pub fn write(&self) -> bool {
		!matches!(self, Self::Load { .. } | Self::CopyWebhookUrl { .. })
	}
	pub fn valid(&self) -> bool {
		if self.scope().is_some_and(|id| id.0 == 0) {
			return false;
		}
		match self {
			Self::Load {
				channel,
				integrations,
				webhooks,
			} => (*integrations || *webhooks) && (channel.is_none() || !integrations),
			Self::CreateWebhook {
				channel,
				name,
				scope,
			} => {
				scope.is_none_or(|scope| scope == *channel)
					&& channel.0 != 0
					&& name.capacity() <= 320
					&& valid_webhook_name(name)
			}
			Self::EditWebhook {
				webhook,
				channel,
				name,
				..
			} => {
				webhook.0 != 0
					&& channel.0 != 0
					&& name.capacity() <= 320
					&& valid_webhook_name(name)
			}
			Self::CopyWebhookUrl {
				scope,
				webhook,
				channel,
			} => webhook.0 != 0 && channel.0 != 0 && scope.is_none_or(|scope| scope == *channel),
			Self::DeleteWebhook { webhook, .. } => webhook.0 != 0,
			Self::DeleteIntegration { integration } => integration.0 != 0,
		}
	}
	pub fn normalize(&mut self) {
		if let Self::CreateWebhook { name, .. } | Self::EditWebhook { name, .. } = self {
			name.shrink_to_fit();
		}
	}
}
pub fn valid_webhook_name(name: &str) -> bool {
	if name.len() > 320 {
		return false;
	}
	let lower = name.to_ascii_lowercase();
	!name.trim().is_empty()
		&& name.chars().count() <= 80
		&& !name.chars().any(char::is_control)
		&& !lower.contains("discord")
		&& !lower.contains("clyde")
}
fn string_bytes(value: &Option<String>) -> usize {
	value.as_ref().map_or(0, String::capacity)
}
fn text(value: &str, limit: usize) -> bool {
	value.len() <= limit && !value.chars().any(char::is_control)
}
fn user_valid(user: &Option<User>) -> bool {
	user.as_ref()
		.is_none_or(|user| user.id.0 != 0 && user.heap_bytes() <= 1024)
}

/// A one-shot clipboard handoff; never serialized, cloned or included in metadata.
pub struct WebhookUrl {
	pub guild: Id,
	pub webhook: Id,
	pub channel: Id,
	url: zeroize::Zeroizing<String>,
}
impl WebhookUrl {
	pub fn new(guild: Id, webhook: Id, channel: Id, token: &str) -> Option<Self> {
		if guild.0 == 0
			|| webhook.0 == 0
			|| channel.0 == 0
			|| token.is_empty()
			|| token.len() > 256
			|| !token
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
		{
			return None;
		}
		Some(Self {
			guild,
			webhook,
			channel,
			url: zeroize::Zeroizing::new(format!(
				"https://discord.com/api/webhooks/{webhook}/{token}"
			)),
		})
	}
	pub fn expose(&self) -> &str {
		&self.url
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>() + self.url.capacity()
	}
}
impl std::fmt::Debug for WebhookUrl {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("WebhookUrl([REDACTED])")
	}
}
