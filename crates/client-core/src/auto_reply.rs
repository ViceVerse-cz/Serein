//! Host-owned allowlist and message admission for optional private auto replies.
use crate::{Command, State, auth::AuthState};
pub use model::auto_reply::*;
use model::{AccountKind, Channel, Id, Message};

pub fn private(channel: &Channel) -> bool {
	channel.guild.is_none() && matches!(channel.kind, 1 | 3)
}

pub fn available(state: &State) -> bool {
	!state.demo
		&& state.auth == AuthState::Authenticated
		&& state.gateway_connected
		&& state.user.as_ref().is_some_and(|u| u.id.0 != 0)
}

pub fn incoming(state: &State, config: &Config, message: &Message) -> bool {
	available(state)
		&& config.channels.contains(&message.channel)
		&& state.channel(message.channel).is_some_and(private)
		&& state.can_compose(message.channel)
		&& state.can_read_history(message.channel)
		&& state
			.user
			.as_ref()
			.is_some_and(|u| u.id != message.author.id)
		&& message.author.id.0 != 0
		&& message.author.kind == AccountKind::Human
		&& !message.author.webhook
		&& matches!(message.kind, 0 | 19)
		&& !message.ephemeral
		&& message.flags & 64 == 0
		&& !message.content.trim().is_empty()
		&& message.content.len() <= MAX_MESSAGE_BYTES
		&& !message.content.starts_with(PREFIX)
}

impl State {
	/// Queue a host-authorized private reply without navigating or touching the composer.
	pub fn prepare_auto_reply(&mut self, channel: Id, content: &str) -> Option<Command> {
		if !available(self)
			|| !self.channel(channel).is_some_and(private)
			|| !self.can_compose(channel)
			|| content.chars().count() > 2000
			|| content.len() > MAX_MESSAGE_BYTES
		{
			return None;
		}
		let search_target = self.search_target;
		let command =
			self.prepare_message_content_for(channel, &[], None, true, Some(content), None);
		self.search_target = search_target;
		command
	}
}
