//! Explicit forwards reuse the bounded send lifecycle without touching composer drafts.
use crate::{Command, Pending, State, auth::AuthState};
use model::{Delivery, Id};

impl State {
	pub fn can_forward(&self, message: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.timeline.get(message).is_some_and(|m| {
				self.selected == Some(m.channel)
					&& self.can_read_history(m.channel)
					&& matches!(m.kind, 0 | 19 | 20 | 23)
					&& !m.ephemeral && !m.unsupported
					&& !m.extra_content.poll
			})
	}

	pub fn prepare_forward(&mut self, message: Id, targets: &[Id], note: &str) -> Vec<Command> {
		let count = targets.len() * if note.trim().is_empty() { 1 } else { 2 };
		if !self.can_forward(message)
			|| targets.is_empty()
			|| targets.len() > 5
			|| targets
				.iter()
				.enumerate()
				.any(|(i, id)| !self.can_compose(*id) || targets[..i].contains(id))
			|| note.chars().count() > crate::MAX_CONTENT
			|| self.pending.len() + count > 64
			|| self.draft_bytes() + count * (note.len() + 128 + size_of::<Pending>())
				> crate::MAX_DRAFT_BYTES
		{
			self.status = "Forward unavailable: check destinations, connection and message limits";
			return Vec::new();
		}
		let source = self.timeline.get(message).unwrap().channel;
		let guild = self.channel(source).and_then(|c| c.guild);
		let mut commands = Vec::with_capacity(count);
		for &channel in targets {
			for content in
				std::iter::once(None).chain((!note.trim().is_empty()).then_some(Some(note)))
			{
				self.send_sequence += 1;
				let epoch = std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap_or_default()
					.as_millis();
				let nonce = crate::fingerprint::nonce(epoch, self.send_sequence);
				self.pending.push(Pending {
					sticker: None,
					channel,
					content: content.unwrap_or("Forwarded message").into(),
					attachments: Vec::new(),
					nonce: nonce.clone(),
					delivery: Delivery::Sending,
					confirmed: None,
				});
				commands.push(if let Some(content) = content {
					Command::Send {
						sticker: None,
						channel,
						content: content.into(),
						nonce,
						reply: None,
					}
				} else {
					Command::Forward {
						source,
						message,
						guild,
						channel,
						nonce,
					}
				});
			}
		}
		commands
	}
}
