//! Message preparation, send outcomes and unsent draft recovery.
use super::{
	Command, Delivery, Envelope, Event, Freshness, Id, MAX_ATTACHMENTS, MAX_DRAFT_BYTES, Message,
	Pending, Reply, State, Sticker, auth, fingerprint, inbox,
};

impl State {
	pub fn has_unsent(&self) -> bool {
		self.drafts.values().any(|s| !s.is_empty())
			|| self
				.pending
				.iter()
				.any(|p| p.delivery != Delivery::Confirmed)
	}
	pub fn draft_bytes(&self) -> usize {
		self.drafts.values().map(String::capacity).sum::<usize>()
			+ self
				.pending
				.iter()
				.map(|p| {
					p.content.capacity()
						+ p.nonce.capacity()
						+ p.sticker.as_ref().map_or(0, Sticker::heap_bytes)
						+ p.attachments.iter().map(String::capacity).sum::<usize>()
						+ p.attachments.capacity() * size_of::<String>()
						+ size_of::<Pending>()
				})
				.sum::<usize>()
	}
	/// Closes the "sending too quickly" notice for the rest of this burst.
	pub fn dismiss_send_throttle(&mut self) {
		self.send_throttled = false;
		// A drained queue already ended the burst; the next one must be told again.
		self.send_throttle_seen = self.pending.iter().any(|p| p.delivery == Delivery::Sending);
	}
	pub fn prepare_send(&mut self) -> Option<Command> {
		self.prepare_send_with_attachment(None)
	}
	/// Queue explicit text without consuming the composer draft or reply target.
	pub fn prepare_text_send(&mut self, content: &str) -> Option<Command> {
		self.prepare_message_content(&[], None, true, Some(content), None)
	}
	/// Queue an explicit reply without consuming the composer draft or its reply target.
	pub fn prepare_reply_send(
		&mut self,
		target: Id,
		content: &str,
		mention: bool,
	) -> Option<Command> {
		if !self.can_reply_to(target) {
			self.status = "This message is unavailable for replies";
			return None;
		}
		let mut reply = Reply::to(target);
		reply.mention = mention;
		self.prepare_message_content(&[], None, true, Some(content), Some(reply))
	}
	pub fn prepare_send_with_attachment(&mut self, filename: Option<&str>) -> Option<Command> {
		self.prepare_send_with_attachments(filename.as_slice())
	}
	/// Queue the draft with up to ten attachment filenames; an empty slice sends text only.
	pub fn prepare_send_with_attachments(&mut self, filenames: &[&str]) -> Option<Command> {
		self.prepare_message(filenames, None, false)
	}
	/// Send explicitly submitted artwork without consuming a newer text draft.
	pub fn prepare_image_send(&mut self, filenames: &[&str]) -> Option<Command> {
		self.prepare_message(filenames, None, true)
	}
	/// A voice clip is sent alone; the text draft stays available for a separate send.
	pub fn prepare_voice_message(&mut self) -> Option<Command> {
		if !self.can_send_voice_message(self.selected?) {
			self.status = "Voice messages are unavailable with the current permissions";
			return None;
		}
		self.prepare_image_send(&["voice-message.ogg"])
	}
	pub(crate) fn prepare_message(
		&mut self,
		filenames: &[&str],
		sticker: Option<&Sticker>,
		preserve_draft: bool,
	) -> Option<Command> {
		self.prepare_message_content(filenames, sticker, preserve_draft, None, None)
	}
	pub(super) fn prepare_message_content(
		&mut self,
		filenames: &[&str],
		sticker: Option<&Sticker>,
		preserve_draft: bool,
		explicit_content: Option<&str>,
		explicit_reply: Option<Reply>,
	) -> Option<Command> {
		let channel = self.selected?;
		if !self.can_send(channel) || (!filenames.is_empty() && !self.can_attach(channel)) {
			self.status = "Sending is unavailable with the current connection or permissions";
			return None;
		}
		let use_composer_reply = explicit_reply.is_none() && explicit_content.is_none();
		let reply = if use_composer_reply {
			self.reply
		} else {
			explicit_reply
		};
		if reply.is_some_and(|reply| {
			!self.can_read_history(channel) || self.timeline.is_deleted(reply.target())
		}) {
			self.status = "This message is unavailable for replies";
			return None;
		}
		if filenames.len() > MAX_ATTACHMENTS {
			self.status = "Attach up to 10 files per message";
			return None;
		}
		if filenames.iter().any(|name| {
			name.trim().is_empty()
				|| name.len() > 256
				|| matches!(*name, "." | "..")
				|| name
					.chars()
					.any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
		}) {
			self.status = "Attachment filename is invalid or too long";
			return None;
		}
		let content = if let Some(content) = explicit_content {
			content
		} else if sticker.is_some() || preserve_draft {
			""
		} else {
			self.drafts.get(&channel).map_or("", String::as_str)
		};
		if !model::message_options::valid(
			content,
			self.content_limit(),
			!filenames.is_empty() || sticker.is_some(),
		) || self.pending.len() >= 64
			|| self.draft_bytes()
				+ content.len()
				+ filenames
					.iter()
					.map(|name| name.len() + size_of::<String>())
					.sum::<usize>()
				+ size_of::<Pending>()
				+ sticker.map_or(0, Sticker::heap_bytes)
				+ 32 > MAX_DRAFT_BYTES
		{
			self.status = "Send exceeds the session input budget";
			return None;
		}
		let content = content.to_owned();
		self.send_sequence += 1;
		let epoch = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_millis();
		let nonce = fingerprint::nonce(epoch, self.send_sequence);
		if use_composer_reply {
			self.reply = None;
		}
		self.pending.push(Pending {
			sticker: sticker.cloned(),
			channel,
			content: content.clone(),
			attachments: filenames.iter().map(|name| (*name).to_owned()).collect(),
			nonce: nonce.clone(),
			delivery: Delivery::Sending,
			confirmed: None,
			reply,
		});
		if sticker.is_none() && !preserve_draft {
			self.drafts.remove(&channel);
		}
		self.search_target = None;
		Some(Command::Send {
			sticker: sticker.map(|s| s.id),
			channel,
			content,
			nonce,
			reply,
		})
	}
	/// Hands a send the desktop stopped before any transport back to the composer: the
	/// caption returns to an empty draft and the reply to an idle reply bar, so a retry
	/// keeps both. When the composer has moved on, the row fails with `failure` instead
	/// and keeps both for Restore. Returns whether the draft was restored.
	pub fn return_unsent(&mut self, command: Command, failure: &'static str) -> bool {
		let Command::Send {
			channel,
			nonce,
			sticker: None,
			..
		} = command
		else {
			self.command_rejected(command);
			return false;
		};
		let index = self.pending.iter().position(|p| {
			p.nonce == nonce
				&& p.channel == channel
				&& p.delivery == Delivery::Sending
				&& p.sticker.is_none()
		});
		let composer_idle = self.selected == Some(channel)
			&& self.drafts.get(&channel).is_none_or(String::is_empty)
			&& (self.drafts.contains_key(&channel) || self.drafts.len() < 64);
		// Never swap a reply target: a newer reply keeps the composer, the row keeps its own.
		let Some(index) = index
			.filter(|index| composer_idle && !self.pending[*index].reply_conflicts(self.reply))
		else {
			self.apply(Envelope {
				generation: self.generation,
				event: Event::SendResult {
					nonce,
					result: Err(auth::Failure::ProtocolAt(failure)),
				},
			});
			return false;
		};
		// Moving the caption from the row to the draft never grows the draft budget.
		let pending = self.pending.remove(index);
		if !pending.content.is_empty() {
			self.drafts.insert(channel, pending.content);
		}
		if self.reply.is_none() {
			self.reply = pending
				.reply
				.filter(|reply| !self.timeline.is_deleted(reply.target()));
		}
		if !self.pending.iter().any(|p| p.delivery == Delivery::Sending) {
			self.send_throttle_seen = false;
		}
		self.status = failure;
		true
	}
	/// Deleted targets must not come back through Restore or a returned send.
	pub(crate) fn forget_pending_replies(&mut self, channel: Id, deleted: &[Id]) {
		for pending in self.pending.iter_mut().filter(|p| p.channel == channel) {
			if pending
				.reply
				.is_some_and(|reply| deleted.contains(&reply.target()))
			{
				pending.reply = None;
			}
		}
	}
	/// Removes a failed or unconfirmed own row without restoring it, returning its channel.
	pub fn discard_pending(&mut self, nonce: &str) -> Option<Id> {
		let index = self
			.pending
			.iter()
			.position(|p| p.nonce == nonce && p.delivery != Delivery::Sending)?;
		Some(self.pending.remove(index).channel)
	}
	pub(super) fn confirm(&mut self, message: &Message) {
		if self.user.as_ref().map(|u| u.id) != Some(message.author.id) {
			return;
		}
		if let Some(nonce) = &message.nonce {
			if let Some(sticker) = self
				.pending
				.iter()
				.find(|p| p.nonce == *nonce && p.channel == message.channel)
				.and_then(|p| p.sticker.clone())
			{
				self.remember_sticker(sticker);
			}
			self.pending
				.retain(|p| !(p.nonce == *nonce && p.channel == message.channel));
		}
	}
	pub(super) fn fail(&mut self, failure: auth::Failure) {
		// The connection may have ended while its HTTP write was accepted. Retire
		// the pending UI state, but never automatically repeat the mutation.
		if self.inbox.loading
			&& self
				.inbox
				.operation
				.as_ref()
				.is_some_and(inbox::Action::write)
		{
			self.inbox.operation = None;
			self.clear_mentions();
			self.inbox.error = Some("Inbox outcome unknown; refresh before retrying");
		}
		self.typing.clear();
		self.status = failure.label();
		match failure {
			auth::Failure::Expired => self.auth = auth::AuthState::Expired,
			auth::Failure::Challenged => self.auth = auth::AuthState::Challenged,
			_ => {}
		}
		if failure.ends_session() {
			self.server_incidents.clear();
			self.community = Default::default();
			self.reset_thread_starter();
			self.application_commands.clear();
			self.interrupt_stickers();
			self.interrupt_soundboard();
			self.interrupt_gif_favorites();
			self.invalidate_messaging_permissions(Some(failure));
			self.interrupt_own_profile();
			self.interrupt_server_profile();
			self.interrupt_account_settings();
			self.interrupt_owned_cosmetics();
			self.interrupt_badge_settings();
			self.local_game_activity = Default::default();
			self.cancel_message_actions();
			if self.folders_pending {
				self.folders_pending = false;
				self.folders_error = Some(failure.label());
			}
			self.cancel_user_action();
			self.cancel_server_action();
			self.cancel_channel_action();
			self.cancel_server_settings();
			self.cancel_onboarding();
			self.cancel_member_guide();
			self.cancel_server_admin();
			self.cancel_group_action();
			self.cancel_invite_join();
			self.cancel_guild_creation();
			self.clear_direct_presences();
			self.clear_cached_history();
			self.clear_search();
			self.inbox.operation = None;
			self.clear_mentions();
			self.search_target = None;
			self.read_state.cancel();
			self.clear_profile();
			self.profile_cache.clear();
			self.disconnect_voice(failure.label());
			self.gateway_connected = false;
			self.invalidate_members();
			self.cancel_history();
		}
		self.freshness = Freshness::Stale;
	}
}

impl State {
	pub(super) fn apply_send_result(
		&mut self,
		nonce: String,
		result: Result<Message, auth::Failure>,
	) -> Result<(), &'static str> {
		let mut reconciliation = Ok(());
		match result {
			Ok(m) => {
				let correlated = self
					.user
					.as_ref()
					.is_some_and(|user| user.id == m.author.id)
					&& (self
						.pending
						.iter()
						.any(|pending| pending.nonce == nonce && pending.channel == m.channel)
						|| self.timeline.get(m.id).is_some_and(|known| {
							known.channel == m.channel
								&& known.author.id == m.author.id
								&& known.nonce.as_deref() == Some(nonce.as_str())
						}));
				if correlated {
					if let Some(sticker) = self
						.pending
						.iter()
						.find(|p| p.nonce == nonce && p.channel == m.channel)
						.and_then(|p| p.sticker.clone())
					{
						self.remember_sticker(sticker);
					}
					self.observe_last_message(m.channel, m.id);
				}
				let accepted_reference = correlated && self.accepts_reply_source(&m);
				if accepted_reference {
					self.record_reply_deletion(&m);
				}
				if let Some(p) = self.pending.iter_mut().find(|p| p.nonce == nonce) {
					p.delivery = Delivery::Confirmed;
					p.confirmed = Some(m.id);
				}
				if accepted_reference
					&& self.selected == Some(m.channel)
					&& let Err(status) = self.timeline.observe_deleted_reference(&m)
				{
					reconciliation = Err(status);
				}
				// A correlated send is a live mutation even before its gateway echo.
				if reconciliation.is_ok()
					&& self.selected == Some(m.channel)
					&& self.can_view(m.channel)
					&& self.freshness != Freshness::Unavailable
					&& self.timeline.get(m.id).is_none()
					&& !self.history_targeted
					&& (!m.reply_deleted || accepted_reference)
					&& self.timeline.insert(m, correlated, false).is_err()
				{
					reconciliation = Err("Message exceeds safe capacity");
				}
			}
			Err(f) => {
				if let Some(p) = self
					.pending
					.iter_mut()
					.find(|p| p.nonce == nonce && p.delivery != Delivery::Confirmed)
				{
					p.delivery = if f == auth::Failure::Ambiguous {
						Delivery::Ambiguous
					} else {
						Delivery::Rejected
					};
				}
				if f.ends_session() {
					self.fail(f);
				} else {
					self.status = f.label();
				}
			}
		}
		self.pending.retain(|p| p.delivery != Delivery::Confirmed);
		if !self.pending.iter().any(|p| p.delivery == Delivery::Sending) {
			self.send_throttle_seen = false;
		}
		reconciliation
	}
}
