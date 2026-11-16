//! Composer input, edits, replies and attachment staging.
use super::{
	AttachmentPaste, Command, Freshness, Id, MAX_DRAFT_BYTES, MentionWrite, MessagingUi, RichText,
	State, SubmittedEdit, TextEdit, attachments, composer_cap, design, dialog, emoji_picker,
	emoticons, formatting, icons, mention_switch, mentions, oversized_text, text_menu, user_menu,
};

impl MessagingUi {
	// `State::status` is retained for callers that inspect the last outcome. In the
	// signed-in UI it is not a visible status line, so an explicit composer action
	// also needs a transient notice. Toasts coalesce repeated identical notices.
	fn composer_notice(
		&mut self,
		state: &mut State,
		status: &'static str,
		key: &'static str,
		level: design::Level,
	) {
		state.status = status;
		self.toasts.push(level, crate::i18n::translate(key));
	}

	// Core retains static status strings for its callers. Translate the known
	// admission failures only at the presentation boundary.
	pub(super) fn composer_send_failure(&mut self, state: &State) {
		let key = match state.status {
			"Nitro is required to use this sticker outside its server" => "composer-sticker-nitro",
			"This sticker is unavailable in this conversation" => "composer-sticker-unavailable",
			"Sending is unavailable with the current connection or permissions" => {
				"composer-gif-unavailable"
			}
			"Attach up to 10 files per message" => "composer-attachment-count",
			"Attachment filename is invalid or too long" => "composer-attachment-name",
			"Send exceeds the session input budget" => "composer-send-budget",
			_ => "composer-send-failed",
		};
		self.toasts
			.push(design::Level::Warning, crate::i18n::translate(key));
	}

	/// Ctrl+V or a context-menu Paste in the focused field `target`: starts one bounded read of
	/// the system clipboard (files, images and text from any app), or delivers its text.
	pub(super) fn clipboard_paste(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		channel: Id,
		target: egui::Id,
		allowed: bool,
	) {
		let pasted_text = self.pasted_text.take();
		let paste_key_released = ctx.input(|input| {
			input.events.iter().any(|event| {
				matches!(
					event,
					egui::Event::Key {
						key: egui::Key::V,
						pressed: false,
						..
					}
				)
			})
		});
		// A context-menu Paste reads the clipboard as Ctrl+V does, with the field refocused.
		let menu = self.menu_paste.take_if(|id| *id == target).is_some();
		let paste_enabled = allowed && (menu || ctx.memory(|m| m.has_focus(target)));
		if let Some((paste_channel, pasted_target, text)) = pasted_text {
			if paste_enabled && paste_channel == channel && pasted_target == target {
				ctx.input_mut(|i| i.events.push(egui::Event::Paste(text)));
				self.pasted_text_frame = Some(ctx.cumulative_frame_nr());
			} else {
				self.composer_notice(
					state,
					"Paste cancelled; focus the message and paste again",
					"composer-paste-cancelled",
					design::Level::Warning,
				);
			}
		} else if paste_enabled && self.pasted_text_frame != Some(ctx.cumulative_frame_nr()) {
			let mut request = AttachmentPaste {
				target,
				text: None,
				image: None,
			};
			let mut requested = false;
			ctx.input_mut(|input| {
				// eframe consumes native paste key-down. File-only clipboards can emit
				// no Paste event, so the matching key-up is also a paste trigger.
				let shortcut = input.events.iter().any(|event| {
					matches!(event,
				egui::Event::Key { key: egui::Key::V, pressed, repeat: false, modifiers, .. }
				if !modifiers.shift && (modifiers.ctrl || modifiers.command || modifiers.alt)
					&& (*pressed || !self.paste_key_handled))
				});
				let has_paste = input.events.iter().any(|event| {
					matches!(event, egui::Event::Paste(_) | egui::Event::PasteImage(_))
				});
				if has_paste || shortcut || menu {
					requested = true;
					self.paste_key_handled = true;
					input.events.retain_mut(|event| match event {
						egui::Event::Paste(text) => {
							request.text = Some(std::mem::take(text));
							false
						}
						egui::Event::PasteImage(image) => {
							request.image = Some(image.clone());
							false
						}
						egui::Event::Key {
							key: egui::Key::V, ..
						} => false,
						// Option+V can also produce a platform text character.
						egui::Event::Text(_) if shortcut => false,
						_ => true,
					});
				}
			});
			if requested {
				if request
					.text
					.as_ref()
					.is_some_and(|text| text.len() > MAX_DRAFT_BYTES)
				{
					self.composer_notice(
						state,
						"Pasted text exceeds the draft limit",
						"composer-paste-too-large",
						design::Level::Error,
					);
				} else if request
					.image
					.as_ref()
					.is_some_and(|image| image.pixels.len() > 4 * 1024 * 1024)
				{
					self.composer_notice(
						state,
						"Paste an image with at most 4 million pixels",
						"composer-paste-image-too-large",
						design::Level::Error,
					);
				} else {
					self.attachment_paste_requested = Some(request);
					self.upload_busy = true;
				}
			}
		}
		if paste_key_released {
			self.paste_key_handled = false;
		}
	}
	pub fn has_edit(&self) -> bool {
		self.editing.is_some()
	}
	pub fn has_edit_in(&self, channel: Option<Id>) -> bool {
		self.editing
			.as_ref()
			.is_some_and(|(edit_channel, _, _)| Some(*edit_channel) == channel)
	}
	pub fn messages_deleted(&mut self, ctx: &egui::Context, channel: Id, ids: &[Id]) {
		if ids.len() > 100 {
			return;
		}
		if self
			.deleting
			.is_some_and(|(c, id)| c == channel && ids.contains(&id))
		{
			self.deleting = None;
		}
		let Some((edit_channel, message, _)) = &self.editing else {
			return;
		};
		if *edit_channel != channel || !ids.contains(message) {
			return;
		}
		let untouched = self
			.edit_modified
			.is_some_and(|(c, id, modified)| c == channel && id == *message && !modified);
		if let Some(id) = self.edit_widget_id {
			if untouched {
				egui::text_edit::TextEditState::default().store(ctx, id);
			} else if let Some(mut editor) = egui::text_edit::TextEditState::load(ctx, id) {
				editor.clear_undoer();
				editor.store(ctx, id);
			}
		}
		self.edit_sent = None;
		self.edit_undo_cleared = true;
		if untouched {
			self.editing = None;
			self.edit_modified = None;
			self.edit_widget_id = None;
			self.edit_closed_channel = Some(channel);
		}
	}
	/// Preserves raw active text on failed edits and reconciles unavailable messages.
	pub(super) fn reconcile_edit(&mut self, state: &mut State) {
		let Some((channel, id, content)) = &self.editing else {
			self.edit_modified = None;
			self.edit_undo_cleared = false;
			return;
		};
		// Keep the raw editor text on rejection/interruption; the recovery queue holds
		// the transformed payload, which must not replace the user's draft.
		if let Some(index) = state
			.message_actions
			.failed_edits
			.iter()
			.position(|(c, message, _)| c == channel && message == id)
		{
			state.message_actions.failed_edits.remove(index);
			self.edit_sent = None;
		}
		if self
			.edit_modified
			.is_none_or(|(c, message, _)| c != *channel || message != *id)
		{
			self.edit_undo_cleared = false;
			let modified = state
				.timeline
				.get(*id)
				.filter(|message| message.channel == *channel)
				.is_none_or(|message| message.content != *content);
			self.edit_modified = Some((*channel, *id, modified));
		}
		if state.selected == Some(*channel)
			&& state.timeline.get(*id).is_none()
			&& self.edit_modified.is_some_and(|(_, _, modified)| !modified)
		{
			self.editing = None;
			self.edit_modified = None;
			self.edit_sent = None;
		}
	}
	pub(super) fn clear_draft(&mut self, state: &mut State, channel: Id) {
		if self.draft_restore_pending {
			// Preserve an explicit clear until the outstanding disk snapshot is merged.
			state.drafts.insert(channel, String::new());
		} else {
			state.drafts.remove(&channel);
		}
		self.draft_changes.push(channel);
	}
	pub(super) fn restore_pending(&mut self, state: &mut State, channel: Id, nonce: &str) {
		if let Some(index) = state.pending.iter().position(|p| {
			p.nonce == nonce && p.channel == channel && p.delivery != model::Delivery::Sending
		}) {
			if !state.drafts.contains_key(&channel) && state.drafts.len() >= 64 {
				self.composer_notice(
					state,
					"Draft budget full. Clear an existing draft before restoring pending text",
					"composer-restore-draft-budget",
					design::Level::Warning,
				);
			} else if state.pending[index].reply_conflicts(state.reply) {
				// Restoring now would send this text to the newer reply's author.
				self.composer_notice(
					state,
					"Cancel the current reply before restoring this message",
					"composer-restore-reply-conflict",
					design::Level::Warning,
				);
			} else if state.drafts.get(&channel).is_none_or(String::is_empty) {
				let pending = state.pending.remove(index);
				if !pending.attachments.is_empty() {
					self.composer_notice(
						state,
						"Text restored; reselect the attachment before sending again",
						"composer-restore-reselect-attachment",
						design::Level::Info,
					);
				}
				state.drafts.insert(channel, pending.content);
				self.draft_changes.push(channel);
				// The reply goes back with its caption; a matching active reply is kept as is.
				if state.reply.is_none() && state.selected == Some(channel) {
					state.reply = pending
						.reply
						.filter(|reply| !state.timeline.is_deleted(reply.target()));
				}
			} else {
				self.composer_notice(
					state,
					"Keep or clear the existing draft before restoring pending text",
					"composer-restore-existing-draft",
					design::Level::Warning,
				);
			}
		}
	}
	/// Remembers the author seen while the target was loaded, so jumping to the latest
	/// page or a send that comes back keeps naming who is being replied to.
	pub(super) fn reply_author(&mut self, state: &State, target: Id) -> String {
		if let Some(message) = state.timeline.get(target) {
			if self
				.reply_author
				.as_ref()
				.is_none_or(|(id, name)| *id != target || *name != message.author.name)
			{
				self.reply_author = Some((target, message.author.name.clone()));
			}
		} else if self
			.reply_author
			.as_ref()
			.is_some_and(|(id, _)| *id != target)
		{
			self.reply_author = None;
		}
		self.reply_author
			.as_ref()
			.filter(|(id, _)| *id == target)
			.map_or_else(
				|| crate::i18n::translate("composer-earlier-author"),
				|(_, name)| name.clone(),
			)
	}
	pub(super) fn submit_composer(
		&mut self,
		state: &mut State,
		ctx: &egui::Context,
		commands: &mut Vec<Command>,
	) {
		let Some(channel) = state.selected else {
			return;
		};
		if self.upload_busy || self.image_share_requested.is_some() || !state.can_send(channel) {
			return;
		}
		let draft = state.drafts.get(&channel).map_or("", String::as_str);
		let has_files = !self.selected_files().is_empty();
		let attempted = !model::message_options::content(draft).0.trim().is_empty() || has_files;
		if !has_files && let Some(assets) = model::ImageShare::markdown_only(draft) {
			if !state.can_attach(channel) {
				self.composer_notice(
					state,
					"Attaching images is unavailable here; add text to send a named link.",
					"composer-attach-denied",
					design::Level::Warning,
				);
				return;
			}
			self.image_share_requested =
				Some((state.generation, channel, assets, draft.to_owned()));
			return;
		}
		let original_draft = if self.convert_emoticons {
			state.drafts.get_mut(&channel).map(|draft| {
				let converted = emoticons::convert(draft);
				std::mem::replace(draft, converted)
			})
		} else {
			None
		};
		if let Some(command) = state.prepare_send_with_attachments(
			&self
				.selected_files()
				.iter()
				.map(|(name, _)| name.as_str())
				.collect::<Vec<_>>(),
		) {
			// Consume this selection once, before desktop dispatch.
			self.stage_pending_upload(ctx, &command);
			self.timeline.follow_latest(state);
			commands.push(command);
		} else {
			if let Some(original) = original_draft {
				state.drafts.insert(channel, original);
			}
			if attempted {
				self.composer_send_failure(state);
			}
		}
	}
	/// Submits message drafts or edits after applying the device conversion preference.
	/// Accept or ignore a pending message request, as in Discord's request conversation view.
	pub(super) fn message_request_bar(&mut self, ui: &mut egui::Ui, state: &State, channel: Id) {
		let colors = design::palette(ui);
		let language = self.language;
		let enabled = !state.user_action_pending();
		ui.add_space(6.0);
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(8)
			.inner_margin(egui::Margin::symmetric(14, 10))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				let button = |ui: &mut egui::Ui, label: String, fill, text| {
					ui.add_enabled(
						enabled,
						egui::Button::new(design::medium(ui, &label, 14.0).color(text))
							.fill(fill)
							.stroke(egui::Stroke::NONE)
							.corner_radius(6)
							.min_size(egui::vec2(88.0, 32.0)),
					)
					.clicked()
				};
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					let actions = 2.0 * 88.0 + 8.0 + 16.0;
					ui.allocate_ui_with_layout(
						egui::vec2((ui.available_width() - actions).max(120.0), 0.0),
						egui::Layout::top_down(egui::Align::Min),
						|ui| {
							ui.spacing_mut().item_spacing.y = 2.0;
							ui.label(
								design::semibold(ui, language.text("message-request-title"), 15.0)
									.color(colors.text_strong),
							);
							ui.add(
								egui::Label::new(
									RichText::new(language.text("message-request-banner"))
										.size(13.0)
										.color(colors.muted),
								)
								.wrap(),
							);
						},
					);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						if button(
							ui,
							language.text("message-request-ignore"),
							colors.hover,
							colors.text_strong,
						) {
							self.user_action = Some(user_menu::Action::MessageRequest {
								channel,
								accept: false,
							});
						}
						if button(
							ui,
							language.text("message-request-accept"),
							colors.accent,
							colors.accent_text,
						) {
							self.user_action = Some(user_menu::Action::MessageRequest {
								channel,
								accept: true,
							});
						}
					});
				});
			});
		ui.add_space(8.0);
	}
	pub(super) fn composer(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		ctx: &egui::Context,
		commands: &mut Vec<Command>,
	) {
		let had_edit = self.editing.is_some();
		self.reconcile_edit(state);
		let closed_here = self.edit_closed_channel.take() == Some(channel);
		if closed_here || (had_edit && self.editing.is_none()) {
			egui::text_edit::TextEditState::default()
				.store(ctx, ui.make_persistent_id("message-edit"));
			ui.weak(crate::i18n::translate(
				"lib-composer-message-deleted-the-unchanged-edit-was-closed",
			));
			ctx.request_repaint();
			return;
		}
		let colors = crate::design::palette(ui);
		let keyboard_enabled = !self.switcher_frame
			&& !self.switcher.is_open()
			&& ctx.memory(|memory| memory.top_modal_layer().is_none());
		fn ime_updates_text(event: &egui::Event) -> bool {
			match event {
				egui::Event::Ime(
					egui::ImeEvent::Preedit { text, .. } | egui::ImeEvent::Commit(text),
				) => !text.is_empty(),
				egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { .. }) => true,
				_ => false,
			}
		}
		if keyboard_enabled
			&& self.editing.is_none()
			&& !self.ime_active
			&& !egui::Popup::is_any_open(ctx)
			&& ctx.memory(|memory| memory.has_focus(ui.make_persistent_id("message-input")))
			&& state.drafts.get(&channel).is_none_or(String::is_empty)
			&& ctx.input_mut(|input| {
				!input.events.iter().any(ime_updates_text)
					&& crate::keybinds::pressed_exact(
						input,
						self.keybinds.chord(model::KeybindAction::EditLastMessage),
					)
			}) && let Some(message) = state
			.timeline
			.iter()
			.rev()
			.find(|message| !message.unsupported && state.can_edit(channel, message.id))
		{
			self.editing = Some((channel, message.id, message.content.clone()));
			self.edit_modified = None;
			self.composer_edit = None;
		}
		let editing_key = self
			.editing
			.as_ref()
			.filter(|(c, _, _)| *c == channel)
			.map(|(c, id, _)| (*c, *id));
		let editing_here = editing_key.is_some();
		if let Some((_, id)) = editing_key {
			if state.timeline.get(id).is_some() {
				self.edit_undo_cleared = false;
			} else if !self.edit_undo_cleared {
				let editor_id = ui.make_persistent_id("message-edit");
				if let Some(mut editor) = egui::text_edit::TextEditState::load(ctx, editor_id) {
					editor.clear_undoer();
					editor.store(ctx, editor_id);
				}
				self.edit_undo_cleared = true;
			}
		}
		if !editing_here && !state.can_compose(channel) {
			if self.pending_mention.take().is_some() {
				self.composer_notice(
					state,
					"You don't have permission to mention anyone in this channel.",
					"composer-mention-denied",
					design::Level::Warning,
				);
			}
			// Returning to an edit must restore its focus after visiting a read-only channel.
			self.composer_edit = None;
			self.mention_menu = mentions::Menu::default();
			self.slash_commands.suspend(state, channel);
			self.emoji_picker = emoji_picker::Picker::default();
			self.ime_active = false;
			self.focus_switched_composer = false;
			let gated = state
				.channel(channel)
				.and_then(|c| c.guild)
				.filter(|guild| state.needs_onboarding(*guild));
			if let Some(guild) = gated {
				design::glass_frame(ui, colors.raised, egui::Margin::same(12))
					.corner_radius(8)
					.show(ui, |ui| {
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							if dialog::action(
								ui,
								"lib-composer-onboarding-complete",
								dialog::Action::Primary,
							)
							.clicked()
							{
								state.open_onboarding(guild);
							}
							let hint =
								crate::i18n::translate(if state.verification_pending(guild) {
									"lib-composer-onboarding-rules-pending"
								} else {
									"lib-composer-onboarding-incomplete"
								});
							ui.with_layout(
								egui::Layout::left_to_right(egui::Align::Center),
								|ui| {
									ui.add(
										egui::Label::new(
											egui::RichText::new(&hint).color(colors.muted),
										)
										.truncate(),
									)
									.on_hover_text(&hint);
								},
							);
						});
					});
				return;
			}
			design::glass_frame(ui, colors.raised, egui::Margin::same(12))
				.corner_radius(8)
				.show(ui, |ui| {
					let mut hint = crate::i18n::translate("composer-send-permission");
					ui.add_enabled(
						false,
						TextEdit::singleline(&mut hint)
							.align(egui::Align2::LEFT_CENTER)
							.desired_width(f32::INFINITY)
							.frame(egui::Frame::NONE),
					);
				});
			return;
		}
		let focus_edit =
			keyboard_enabled && editing_key.is_some() && self.composer_edit != editing_key;
		let focus_composer = keyboard_enabled
			&& (std::mem::take(&mut self.focus_switched_composer)
				|| (self.composer_edit != editing_key
					&& (editing_here || self.composer_edit.is_some_and(|(c, _)| c == channel))));
		if self.composer_edit != editing_key {
			self.composer_edit = editing_key;
			self.ime_active = false;
			self.mention_menu = mentions::Menu::default();
			self.emoji_picker = emoji_picker::Picker::default();
		}
		let mut cancel_edit = false;
		if ctx.input(|input| !input.raw.hovered_files.is_empty()) {
			let available = state.can_attach(channel) && !self.attach_busy && !editing_here;
			egui::Frame::new()
				.fill(colors.accent.gamma_multiply(0.12))
				.stroke(egui::Stroke::new(1.5, colors.accent))
				.corner_radius(12)
				.inner_margin(20)
				.show(ui, |ui| {
					ui.set_min_width((ui.available_width() - 2.0).max(0.0));
					ui.label(design::semibold(
						ui,
						crate::i18n::translate_if_key(if available {
							"lib-ime-updates-text-drop-files-to-attach"
						} else {
							"lib-ime-updates-text-attachments-unavailable-right-now"
						}),
						18.0,
					));
					ui.add_space(6.0);
					ui.label(
						egui::RichText::new(crate::i18n::translate_if_key(if available {
							"lib-ime-updates-text-up-to-10-files-500-mb-max-account-limit-applies"
						} else {
							"lib-ime-updates-text-return-to-an-available-conversation-after-the-current-operation-finishes"
						}))
						.color(colors.muted),
					);
				});
			ui.add_space(8.0);
		}
		// Reply/edit context sits flush on top of the input as one rounded block, like Discord.
		let mut cap_top: Option<f32> = None;
		if editing_here {
			let unavailable = editing_key.is_some_and(|(_, id)| state.timeline.get(id).is_none());
			let cap = composer_cap(ui, &colors, |ui| {
				// Reserve controls before laying out the possibly long context text.
				cancel_edit = icons::button(
					ui,
					icons::Icon::Close,
					22.0,
					&crate::i18n::translate("lib-ime-updates-text-cancel-edit"),
				)
				.clicked();
				if unavailable
					&& ui
						.add(
							egui::Button::new(
								RichText::new(crate::i18n::translate(
									"lib-ime-updates-text-copy-edit-text",
								))
								.size(12.0)
								.color(colors.muted),
							)
							.frame(false),
						)
						.clicked() && let Some((_, _, text)) = &self.editing
				{
					ui.ctx().copy_text(text.clone());
				}
				let mut context = crate::i18n::translate_if_key(if unavailable {
					"lib-ime-updates-text-message-unavailable-unsent-edit"
				} else {
					"lib-ime-updates-text-editing-message"
				});
				if self.edit_sent.is_some() {
					context.push_str(" · ");
					context.push_str(&crate::i18n::translate(
						"lib-ime-updates-text-save-requested-check-the-connection-before-retrying",
					));
				}
				ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
					ui.add(
						egui::Label::new(RichText::new(&context).size(13.0).color(colors.muted))
							.truncate(),
					)
					.on_hover_text(context);
				});
			});
			cap_top = Some(cap.top());
		} else if let Some(reply) = state.reply {
			let author = self.reply_author(state, reply.target());
			let cap = composer_cap(ui, &colors, |ui| {
				ui.spacing_mut().item_spacing.x = 6.0;
				if icons::button(
					ui,
					icons::Icon::Close,
					22.0,
					&crate::i18n::translate("lib-ime-updates-text-cancel-reply"),
				)
				.clicked()
				{
					state.reply = None;
				}
				if let Some(reply) = state.reply.as_mut() {
					mention_switch(ui, &colors, &mut reply.mention);
				}
				if ui
					.add_enabled(
						state.can_open_reply_target(reply.target()),
						egui::Button::new(
							RichText::new(crate::i18n::translate(
								"lib-ime-updates-text-view-original",
							))
							.size(12.0)
							.color(colors.muted),
						)
						.frame(false),
					)
					.on_disabled_hover_text(crate::i18n::translate_if_key(
						if state.timeline.is_deleted(reply.target()) {
							"lib-ime-updates-text-the-original-message-was-deleted"
						} else {
							"lib-ime-updates-text-wait-for-readable-current-message-history"
						},
					))
					.clicked()
				{
					self.timeline.request_reply_target(reply.target());
				}
				let prefix = crate::i18n::translate("lib-ime-updates-text-replying-to");
				let context = format!("{prefix} {author}");
				let mut label = egui::text::LayoutJob::default();
				label.append(
					&format!("{prefix} "),
					0.0,
					egui::TextFormat::simple(egui::FontId::proportional(13.0), colors.muted),
				);
				label.append(
					&author,
					0.0,
					egui::TextFormat::simple(
						egui::FontId::new(13.0, design::semibold_family(ui.ctx())),
						colors.text_strong,
					),
				);
				ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
					ui.add(egui::Label::new(label).truncate())
						.on_hover_text(context);
				});
			});
			cap_top = Some(cap.top());
		}
		let full = state.draft_bytes() >= MAX_DRAFT_BYTES
			|| (!state.drafts.contains_key(&channel) && state.drafts.len() >= 64);
		if full && !editing_here {
			ui.label(crate::i18n::translate(
				"lib-ime-updates-text-draft-budget-full-clear-an-existing-draft-to-continue",
			));
			if state.drafts.contains_key(&channel)
				&& ui
					.button(crate::i18n::translate(
						"lib-ime-updates-text-clear-this-draft",
					))
					.clicked()
			{
				self.clear_draft(state, channel);
			}
			return;
		}
		// Wayland/Fcitx can repeatedly emit empty preedits while idle. Only real
		// composition blocks shortcuts; dismissing an active composition still does.
		let ime_this_frame = keyboard_enabled
			&& (self.ime_active || ctx.input(|i| i.events.iter().any(ime_updates_text)));
		if keyboard_enabled {
			ctx.input(|i| {
				for event in &i.events {
					if let egui::Event::Ime(event) = event {
						match event {
							egui::ImeEvent::Preedit { text, .. } => {
								self.ime_active = !text.is_empty()
							}
							egui::ImeEvent::Commit(_) => self.ime_active = false,
							_ => {}
						}
					}
				}
			});
		}
		let composer_id = ui.make_persistent_id(if editing_here {
			"message-edit"
		} else {
			"message-input"
		});
		if editing_here {
			self.edit_widget_id = Some(composer_id);
		}
		// Typing while nothing has keyboard focus starts writing in the composer, like Discord.
		// Focus moves before the editor runs this frame, so it also receives the typed text.
		let typed = keyboard_enabled
			&& !egui::Popup::is_any_open(ctx)
			&& ctx.memory(|m| m.focused().is_none())
			&& ctx.input(|i| {
				i.events.iter().any(|event| match event {
					egui::Event::Paste(_) | egui::Event::PasteImage(_) => true,
					egui::Event::Key {
						key: egui::Key::V,
						pressed: false,
						modifiers,
						..
					} => !modifiers.shift && (modifiers.ctrl || modifiers.command),
					egui::Event::Text(text) => {
						!i.modifiers.command
							&& !i.modifiers.ctrl && text
							.chars()
							.any(|c| !c.is_control() && !c.is_whitespace())
					}
					_ => false,
				})
			});
		if focus_composer || typed {
			ctx.memory_mut(|m| m.request_focus(composer_id));
		}
		if focus_edit || (focus_composer && !editing_here) {
			let mut edit_state = egui::text_edit::TextEditState::default();
			let text = if editing_here {
				self.editing
					.as_ref()
					.map_or("", |(_, _, content)| content.as_str())
			} else {
				state.drafts.get(&channel).map_or("", String::as_str)
			};
			edit_state
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::one(
					egui::text::CCursor::end_of_str(text),
				)));
			edit_state.store(ctx, composer_id);
		}
		let paste_allowed = keyboard_enabled
			&& !editing_here
			&& self.slash_commands.active.is_none()
			&& !self.ime_active
			&& !ime_this_frame;
		self.clipboard_paste(ctx, state, channel, composer_id, paste_allowed);
		if !editing_here
			&& state
				.drafts
				.get(&channel)
				.is_some_and(|draft| draft.starts_with('/'))
			&& let Some(command) = state.request_application_commands(channel, false)
		{
			commands.push(command);
		}
		let composer_content = if editing_here {
			self.editing
				.as_ref()
				.map_or("", |(_, _, text)| text.as_str())
		} else {
			state.drafts.get(&channel).map_or("", String::as_str)
		};
		let effective = if editing_here {
			composer_content
		} else {
			model::message_options::content(composer_content).0
		};
		let count_before = effective.chars().count();
		let max_content = state.content_limit();
		emoji_picker::set_content_limit(max_content);
		let new_content_valid =
			model::message_options::valid(composer_content, max_content, self.attachment.is_some());
		// Discord refuses any file over the account's limit, so such a message is never sent.
		let upload_limit = state.upload_limit();
		let oversized = !editing_here
			&& self
				.selected_files()
				.iter()
				.any(|(_, bytes)| *bytes > upload_limit);
		// Suggestion rows can take focus on press; keep the editor alive until release
		// so the shared member/channel/emoji popup can finish the click.
		let suggestion_pointer = self.mention_menu.pointer_interacting(ctx, channel)
			|| (self.slash_commands.active.is_none()
				&& self.slash_commands.pointer_interacting(ctx, channel));
		if keyboard_enabled && !self.ime_active && !ime_this_frame && suggestion_pointer {
			ctx.memory_mut(|memory| memory.request_focus(composer_id));
		}
		let mention_enabled = keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& ctx.memory(|m| m.has_focus(composer_id));
		self.slash_commands.refresh(
			state,
			channel,
			composer_content,
			!editing_here
				&& keyboard_enabled
				&& !self.ime_active
				&& !ime_this_frame
				&& (mention_enabled || self.slash_commands.active.is_some()),
		);
		let slash_pick = if !editing_here
			&& keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& (mention_enabled || self.slash_commands.form_has_focus(ctx))
		{
			self.slash_commands.keys(ctx)
		} else {
			None
		};
		let mention_users = if mention_enabled || composer_content.contains("<@") {
			mentions::known_users(state, channel)
		} else {
			Vec::new()
		};
		let mass_mentions =
			state.permission(channel, model::permissions::MENTION_EVERYONE) == Some(true);
		let cursor = egui::text_edit::TextEditState::load(ctx, composer_id)
			.and_then(|s| s.cursor.char_range())
			.filter(|r| r.is_empty())
			.map(|r| r.primary.index.0);
		self.mention_menu.hide_nitro_emojis = self.hide_nitro_emojis;
		if std::mem::take(&mut self.mention_menu.retry_games) {
			state.game_mentions.status = client_core::game_mentions::Status::Idle;
		}
		self.mention_menu.refresh(
			state,
			channel,
			composer_content,
			cursor.filter(|_| mention_enabled),
			&mention_users,
		);
		if state.game_mentions.status == client_core::game_mentions::Status::Idle
			&& (self.mention_menu.needs_games() || composer_content.contains("<@$"))
		{
			state.game_mentions.request();
			ctx.request_repaint();
		}
		let mention_pick = if mention_enabled {
			self.mention_menu.keys(ctx)
		} else {
			None
		};
		if std::mem::take(&mut self.composer_claims_tab) && ctx.memory(|m| m.has_focus(composer_id))
		{
			// A menu that closed this frame must not leave Tab for the editor to indent with.
			ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
		}
		let mut editing = if editing_here {
			self.editing.take()
		} else {
			None
		};
		let demo = state.demo;
		let enter = keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& ctx.memory(|m| m.has_focus(composer_id))
			&& ctx.input_mut(|i| {
				crate::keybinds::pressed_exact(
					i,
					self.keybinds.chord(model::KeybindAction::SendMessage),
				)
			});
		let placeholder = state.channel(channel).map_or_else(
			|| crate::i18n::translate("composer-placeholder"),
			|c| {
				if c.guild.is_some() {
					crate::i18n::translate_args(
						"composer-placeholder-channel",
						&[("channel", &c.name)],
					)
				} else {
					crate::i18n::translate_args(
						"composer-placeholder-direct",
						&[("name", state.conversation_name(c))],
					)
				}
			},
		);
		let can_attach = !editing_here
			&& self.slash_commands.active.is_none()
			&& state.can_attach(channel)
			&& !self.attach_busy
			&& self.attachment_files.len() + self.attachment_loading < 10;
		let can_create_poll =
			!editing_here && self.slash_commands.active.is_none() && state.can_create_poll(channel);
		let application_command = !editing_here && self.slash_commands.active.is_some();
		let can_send = if let Some((edit_channel, message)) = editing_key {
			state.freshness == Freshness::Fresh
				&& state.can_edit(edit_channel, message)
				&& count_before > 0
		} else if application_command {
			self.slash_commands.can_submit(state, channel)
		} else {
			state.can_send(channel)
				&& (self.attachment.is_none() || state.can_attach(channel))
				&& !self.upload_busy
				&& !(state.demo && self.attachment.is_some())
				&& new_content_valid
				&& !oversized
		};
		if cap_top.is_some() {
			// The cap and the input form one block: undo the automatic vertical item gap.
			ui.add_space(-ui.spacing().item_spacing.y);
		}
		design::glass_frame(ui, colors.raised, egui::Margin::symmetric(10, 6))
            .corner_radius(if cap_top.is_some() {
                egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }
            } else {
                egui::CornerRadius::same(8)
            })
            .show(ui, |ui| {
                // Outer frame bounds for the autocomplete popout: undo the inner margin and
                // include the reply/edit cap so the popout never covers it.
                let composer_anchor = egui::Rect::from_min_max(
                    egui::pos2(ui.max_rect().left() - 10.0, cap_top.unwrap_or(ui.max_rect().top() - 6.0)),
                    egui::pos2(ui.max_rect().right() + 10.0, ui.max_rect().top()),
                );
                if !editing_here && (self.attachment.is_some() || self.attachment_loading > 0) {
                    self.attachment_tray(ui, state.can_send(channel), upload_limit);
                }
                // Tall drafts stack the actions in bottom-aligned columns so the text keeps the
                // width. Last pass's visible draft height decides.
                let stack_id = composer_id.with("stacked-actions");
                let (stacked, draft_height) = ctx.data(|d| d.get_temp::<(bool, f32)>(stack_id)).unwrap_or_default();
                let column = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
                    if stacked {
                        ui.allocate_ui_with_layout(
                            egui::vec2(28.0, draft_height),
                            egui::Layout::bottom_up(egui::Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                add(ui);
                            },
                        );
                    } else {
                        add(ui);
                    }
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    column(ui, &mut |ui| {
                    // An active application command shows its app in place of the attach button.
                    let attach = if !editing_here && self.slash_commands.composer_badge(ui, state, &mut self.avatars) {
                        None
                    } else {
                        Some(ui
                            .add_enabled_ui(can_attach || can_create_poll || self.voice_messages.available, |ui| {
                                icons::button(ui, icons::Icon::Attach, 28.0, &crate::i18n::translate("lib-ime-updates-text-attach-files"))
                            })
                            .inner
                            .on_hover_text(crate::i18n::translate("lib-ime-updates-text-choose-drop-or-paste-files-ctrl-cmd-option-v-up")))
                    };
                    if !editing_here {
                        self.extensions.composer_menu(ui, state);
                    }
                    if let Some(attach) = attach {
                        egui::Popup::menu(&attach)
                            .id(attach.id.with(("composer-add", state.generation, channel)))
                            .show(|ui| {
                                ui.set_min_width(200.0);
                                ui.spacing_mut().button_padding = egui::vec2(8.0, 6.0);
                                // Discord-style rows: the glyph follows the row's text colour, so it dims when disabled.
                                let item = |icon: icons::Icon, key: &str| {
                                    egui::Button::new((
                                        egui::Atom::paint(egui::Vec2::splat(18.0), move |ui, args| {
                                            icons::paint(ui.painter(), icon, args.rect, args.fallback_text_color);
                                        }),
                                        crate::i18n::translate(key),
                                    ))
                                };
                                if ui.add_enabled(can_attach, item(icons::Icon::File, "lib-ime-updates-text-attach-files")).clicked() {
                                    self.attach_requested = true;
                                    ui.close();
                                }
                                if self.voice_messages.config.is_some() && ui.add_enabled(self.voice_messages.available && !editing_here && self.attachment.is_none() && !self.upload_busy, item(icons::Icon::Microphone, "voice-message-menu")).clicked() {
                                    self.voice_messages.open(state, channel);
                                    ui.close();
                                }
                                if ui.add_enabled(can_create_poll, item(icons::Icon::ChartBar, "lib-ime-updates-text-create-a-poll")).clicked() {
                                    self.poll_creator.open(state, channel);
                                    ui.close();
                                }
                            });
                    }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let mut actions = None;
                        column(ui, &mut |ui| {
                        let send_button = ui
                            .add_enabled_ui(can_send, |ui| {
                                icons::composer(
                                    ui,
                                    icons::Icon::Send,
                                    false,
                                    &crate::i18n::translate_if_key(if editing_here { "lib-ime-updates-text-save-edit" } else if application_command { "lib-ime-updates-text-send-command" } else { "lib-ime-updates-text-send-message" }),
                                )
                            })
                            .inner;
                        let send = enter || send_button.clicked();
                        if count_before + 200 >= max_content {
                            ui.label(
                                RichText::new(format!("{}", max_content.saturating_sub(count_before)))
                                    .size(11.0)
                                    .color(if count_before >= max_content { colors.danger } else { colors.muted }),
                            );
                        }
                        let pick = ui
                            .add_enabled_ui(!application_command && !self.ime_active && !ime_this_frame, |ui| {
                                self.emoji_picker.hide_nitro_emojis = self.hide_nitro_emojis;
                                self.emoji_picker.preload = self.preload_on_hover;
                                self.emoji_picker
                                    .show(ui, state, channel, &mut self.avatars, commands)
                            })
                            .inner;
                        // A chosen GIF is its own message; the typed draft stays untouched.
                        if self.emoji_picker.is_open() {
                            self.reaction_picker.dismiss(state, commands);
                        }
                        let pick = match pick {
                            Some(emoji_picker::Pick::Insert(text)) => {
                                self.reaction_picker.record(&text);
                                Some(text)
                            },
                            Some(emoji_picker::Pick::React(_, _) | emoji_picker::Pick::Choose(_)) => None,
                            Some(emoji_picker::Pick::Sticker(sticker)) => {
                                if editing_here { self.composer_notice(state, "Finish or cancel the edit before sending a sticker.", "composer-sticker-edit", design::Level::Warning); }
                                else if self.upload_busy { self.composer_notice(state, "Wait for the upload before sending a sticker.", "composer-sticker-upload", design::Level::Warning); }
                                else if let Some(command) = state.prepare_sticker_send(&sticker) { self.timeline.follow_latest(state); commands.push(command); }
                                else { self.composer_send_failure(state); }
                                None
                            },
                            Some(emoji_picker::Pick::Send(url)) => {
                                if editing_here {
                                    self.composer_notice(state, "Finish or cancel the edit before sending a GIF.", "composer-gif-edit", design::Level::Warning);
                                } else if self.upload_busy || !state.can_send(channel) {
                                    self.composer_notice(state, "Sending is unavailable with the current connection or permissions", "composer-gif-unavailable", design::Level::Warning);
                                } else {
                                    let saved = state.drafts.insert(channel, url);
                                    let command = state.prepare_send();
                                    match saved {
                                        Some(saved) => {
                                            state.drafts.insert(channel, saved);
                                        }
                                        None => {
                                            state.drafts.remove(&channel);
                                        }
                                    }
                                    if let Some(command) = command {
                                        self.timeline.follow_latest(state);
                                        commands.push(command);
                                    } else {
                                        self.composer_send_failure(state);
                                    }
                                }
                                None
                            }
                            None => None,
                        };
                        actions = Some((send, pick));
                        });
                        let (send, pick) = actions.expect("composer actions ran");
                        let edit = ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width());
                        if application_command {
                            return ui.scope(|ui| {
                                ui.add_enabled_ui(keyboard_enabled, |ui| {
                                    self.slash_commands.composer(ui, state, channel, self.keybinds.chord(model::KeybindAction::SendMessage));
                                });
                                if self.slash_commands.active.is_none() {
                                    ctx.memory_mut(|memory| memory.request_focus(composer_id));
                                }
                                self.slash_commands.show(ui, composer_anchor, state, channel, &mut self.avatars);
                            }).response;
                        }
                        let composer_escape = keyboard_enabled
                            && !self.ime_active
                            && !ime_this_frame
                            && ctx.memory(|m| {
                                m.has_focus(composer_id) || m.had_focus_last_frame(composer_id)
                            });
                        cancel_edit |= editing_here
                            && composer_escape
                            && ctx.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            });
                        if !editing_here
                            && state.reply.is_some()
                            && composer_escape
                            && ctx.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            })
                        {
                            state.reply = None;
                        }
                        let remaining = if editing_here { max_content * 4 } else { MAX_DRAFT_BYTES.saturating_sub(state.draft_bytes()) };
                        // Temporarily own the buffer so suggestions can borrow the current
                        // permission state without cloning the draft or server catalogs.
                        let restore_empty_draft = self.draft_restore_pending && state.drafts.contains_key(&channel);
                        let mut new_draft = if editing_here { String::new() } else {
                            state.drafts.remove(&channel).unwrap_or_default()
                        };
                        let draft = if let Some((_, _, content)) = &mut editing {
                            content
                        } else {
                            &mut new_draft
                        };
						let mut mention_changed = false;
						match self.apply_pending_mention(ctx, composer_id, draft, remaining, editing_here) {
							None => {}
							Some(MentionWrite::Inserted) => mention_changed = true,
							Some(MentionWrite::DidNotFit) => {
								self.composer_notice(state, "Mention will not fit. Shorten this message or free draft space.", "composer-mention-too-large", design::Level::Warning);
							}
						}
						if let Some(pick) = pick {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            let range = edit_state.cursor.char_range();
                            if let Some(cursor) = emoji_picker::insert(draft, &pick, range, remaining, editing_here) {
                                edit_state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::one(
                                        egui::text::CCursor::new(cursor),
                                    )));
                                edit_state.store(ctx, composer_id);
                                mention_changed = true;
                            } else {
                                self.composer_notice(state, "Emoji will not fit. Shorten this message or free draft space.", "composer-emoji-too-large", design::Level::Warning);
                            }
                        }
                        if let Some(pick) = slash_pick
                            && let Some(cursor) = self.slash_commands.accept(pick, draft, remaining) {
                            let mut edit_state = egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            edit_state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
                            edit_state.store(ctx, composer_id);
                            mention_changed = true;
                        }
                        if let Some(pick) = mention_pick
                            && let Some(cursor) = mentions::insert_checked(state, channel, draft, pick, editing_here)
                        {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            edit_state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(cursor),
                                )));
                            edit_state.store(ctx, composer_id);
                            mention_changed = true;
                        }
						if mention_enabled
							&& let Some(cursor) = cursor
							&& let Some(cursor) =
								emoji_picker::complete_shortcode(state, channel, self.hide_nitro_emojis, draft, cursor, remaining, editing_here)
						{
							let mut edit_state = egui::text_edit::TextEditState::load(ctx, composer_id)
								.unwrap_or_default();
							edit_state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
								egui::text::CCursor::new(cursor),
							)));
							edit_state.store(ctx, composer_id);
							mention_changed = true;
						}
                        if keyboard_enabled
                            && !self.ime_active
                            && !ime_this_frame
                            && ctx.memory(|m| m.has_focus(composer_id))
							&& let Some(style) = formatting::Style::consume(ctx, &self.keybinds)
                        {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            let range = edit_state.cursor.char_range();
                            if let Some(range) = formatting::apply(draft, style, range, remaining, editing_here) {
                                edit_state.cursor.set_char_range(Some(range));
                                edit_state.store(ctx, composer_id);
                                mention_changed = true;
                            } else {
                                self.composer_notice(state, "Formatting will not fit. Shorten this message or free draft space.", "composer-formatting-too-large", design::Level::Warning);
                            }
                        }
                        let rich_layout = &mut self.composer_layout;
                        if !self.ime_active
                            && !ime_this_frame
                            && ctx.input(|i| {
                                i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Delete)
                            })
                        {
                            rich_layout.galley_games(
                                ui,
                                draft,
                                ui.available_width(),
                                &mentions::composer_users(state, channel, draft, &mention_users),
                                mentions::known_roles(state, channel),
                                &state.channels,
                                &state.game_mentions,
                                mass_mentions,
                                &mut self.avatars,
                                demo,
                            );
                            rich_layout.select_deleted_inline(ctx, composer_id);
                        }
                        let selection = text_menu::selection_for_menu(ctx, composer_id);
                        let mut output = egui::ScrollArea::vertical()
                            .id_salt((composer_id, channel, editing_key))
                            .max_height(ui.ctx().viewport_rect().height() * 0.5)
                            .min_scrolled_height(ui.ctx().viewport_rect().height() * 0.5)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                crate::scroll::claim_wheel_content(ui);
                                let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width: f32| {
                                    rich_layout.galley_games(
                                        ui,
                                        buffer.as_str(),
                                        width,
                                        &mentions::composer_users(state, channel, buffer.as_str(), &mention_users),
                                        mentions::known_roles(state, channel),
                                        &state.channels,
                                        &state.game_mentions,
                                        mass_mentions,
                                        &mut self.avatars,
                                        demo,
                                    )
                                };
                                // Multiline edits wrap their hint; a pre-laid galley keeps a
                                // long channel name on one elided line.
                                let hint = if draft.is_empty() {
                                    let mut job = egui::text::LayoutJob::simple_singleline(
                                        placeholder.clone(),
                                        egui::TextStyle::Body.resolve(ui.style()),
                                        ui.visuals().weak_text_color(),
                                    );
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                                    egui::WidgetText::from(ui.painter().layout_job(job))
                                } else {
                                    egui::WidgetText::default()
                                };
                                let output = TextEdit::multiline(draft)
                                    .interactive(keyboard_enabled)
                                    .layouter(&mut layouter)
                                    .id(composer_id)
                                    .event_filter(egui::EventFilter {
                                        horizontal_arrows: true, vertical_arrows: true, escape: editing_here,
                                        ..Default::default()
                                    })
                                    .char_limit(max_content + if editing_here { 0 } else { model::message_options::PREFIX_ALLOWANCE })
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY)
                                    // Horizontal layouts reserve the interaction height, including around icons.
                                    .min_size(egui::vec2(0.0, ui.spacing().interact_size.y))
                                    .align(egui::Align2::LEFT_CENTER)
                                    .frame(egui::Frame::NONE)
                                    .hint_text(hint)
                                    .show(ui);
                                rich_layout.paint(ui, &output, &mut self.avatars);
                                output
                            }).inner;
                        if !self.ime_active && !ime_this_frame {
                            rich_layout.snap_cursor(&mut output, ctx);
                        }
                        if mention_changed || (mention_enabled && suggestion_pointer) {
                            output.response.request_focus();
                        }
                        let mention_cursor = output
                            .cursor_range
                            .filter(|r| r.is_empty())
                            .map(|r| r.primary.index.0)
                            .or(cursor.filter(|_| suggestion_pointer))
                            .filter(|_| mention_enabled);
                        self.mention_menu
                            .refresh(state, channel, draft, mention_cursor, &mention_users);
                        if let Some(pick) = self.mention_menu.show(ui, composer_anchor, &mut self.avatars, demo)
                            && let Some(cursor) = mentions::insert_checked(state, channel, draft, pick, editing_here)
                        {
                            output
                                .state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(cursor),
                                )));
                            output.state.clone().store(ctx, composer_id);
                            output.response.request_focus();
                            mention_changed = true;
                        }
                        #[cfg(not(feature = "demo"))]
                        let slash_preview = false;
                        #[cfg(feature = "demo")]
                        let slash_preview = state.demo && self.preview_slash_commands;
                        self.slash_commands.refresh(state, channel, draft,
                            !editing_here && keyboard_enabled && !self.ime_active && !ime_this_frame
                                && (output.response.has_focus() || suggestion_pointer || self.slash_commands.active.is_some() || slash_preview));
                        if let Some(pick) = self.slash_commands.show(ui, composer_anchor, state, channel, &mut self.avatars)
                            && let Some(cursor) = self.slash_commands.accept(pick, draft, remaining) {
                            output.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
                            output.state.clone().store(ctx, composer_id);
                            output.response.request_focus();
                            mention_changed = true;
                        }
                        // egui moves focus on unclaimed Tab before any menu can consume it.
                        self.composer_claims_tab = mention_enabled
                            && (self.mention_menu.claims_tab() || (!editing_here && self.slash_commands.claims_tab(ctx)));
                        if self.composer_claims_tab {
                            ctx.memory_mut(|m| m.set_focus_lock_filter(composer_id, egui::EventFilter {
                                tab: true, horizontal_arrows: true, vertical_arrows: true, escape: editing_here,
                            }));
                        }
                        if text_menu::show(&mut output.response, composer_id, draft, paste_allowed, selection) {
                            self.menu_paste = Some(composer_id);
                        }
                        let edit = output.response;
                        let cleared = draft.is_empty();
                        if edit.changed() || mention_changed {
                            if editing_here && let Some((_, _, modified)) = &mut self.edit_modified {
                                *modified = true;
                            }
                            if editing_here {
                                self.edit_sent = None;
                            } else if cleared {
                                self.clear_draft(state, channel);
                            } else {
                                self.draft_changes.push(channel);
                            }
                        }
                        self.mention_lookup.text = self.mention_menu.member_query().to_owned();
                        self.mention_lookup.sync(ctx, state, channel, 0, commands);
                        if !editing_here && (!new_draft.is_empty() || restore_empty_draft) {
                            state.drafts.insert(channel, new_draft);
                        }
                        edit.response
                            }).inner
                        }).inner;
                        // Stack once three buttons fit beside the draft; unstack only at one line,
                        // so the width gained by stacking cannot flip the layout back and forth.
                        // The text edit reports its full content height; the scroll area shows at most half the viewport.
                        let height = edit.rect.height().min(ctx.viewport_rect().height() * 0.5);
                        let stacked = if height >= 3.0 * 28.0 + 8.0 {
                            true
                        } else if height <= ui.spacing().interact_size.y + 1.0 {
                            false
                        } else {
                            stacked
                        };
                        ctx.data_mut(|d| d.insert_temp(stack_id, (stacked, height)));
                        if std::mem::take(&mut self.slash_commands.retry)
                            && let Some(command) = state.request_application_commands(channel, true) {
                            commands.push(command);
                        }
                        let slash_run = std::mem::take(&mut self.slash_commands.run)
                            && keyboard_enabled && !self.ime_active && !ime_this_frame
                            && self.slash_commands.can_submit(state, channel);
                        if (send || slash_run) && !cancel_edit {
                            if let Some((edit_channel, message, content)) = &editing {
                                if state.freshness == Freshness::Fresh
                                    && let Some(command) = state.prepare_edit(*edit_channel, *message,
                                        if self.convert_emoticons { emoticons::convert(content) } else { content.clone() })
                                {
                                    if let Command::Edit { content: submitted, .. } = &command {
                                        self.edit_sent = Some(SubmittedEdit {
                                            channel: *edit_channel,
                                            message: *message,
                                            draft: content.clone(),
                                            content: submitted.clone(),
                                        });
                                    }
                                    commands.push(command);
                                } else {
                                    state.status = "Edit kept. Wait for your current message and connection, and enter nonempty text.";
                                    self.toasts.push(design::Level::Warning, crate::i18n::translate("composer-edit-kept"));
                                }
                            } else if self.send_application_command(state, channel, commands)
                                || self.handle_builtin_slash(state, channel, ctx, commands) {
                            } else if oversized {
                                self.toasts.push(design::Level::Error, oversized_text(upload_limit));
                            } else if !self.upload_busy && !(state.demo && self.attachment.is_some()) {
                                self.submit_composer(state, ctx, commands);
                            }
                            if !application_command { edit.request_focus(); }
                            if application_command && self.slash_commands.active.is_none() {
                                ctx.memory_mut(|memory| memory.request_focus(composer_id));
                            }
                        }
                    });
                });
                if let Some((edit_channel, message)) = editing_key {
                    if state.freshness != Freshness::Fresh || !state.can_edit(edit_channel, message) { ui.weak(crate::i18n::translate("lib-ime-updates-text-editing-this-message-is-unavailable-your-text-is-kept-until")); }
                } else if !state.can_send(channel) && !application_command {
                    ui.weak(crate::i18n::translate("lib-ime-updates-text-sending-messages-is-unavailable-in-this-conversation-your-draft-is"));
                } else if self.attachment.is_some() && !state.can_attach(channel) {
                    ui.weak(crate::i18n::translate("lib-ime-updates-text-attaching-files-is-unavailable-here-remove-the-attachment-to-send"));
                }
            });
		if editing_here {
			self.editing = if cancel_edit { None } else { editing };
			if cancel_edit {
				self.edit_sent = None;
			}
		}
		if self.edit_sent.as_ref().is_some_and(|submitted| {
			self.editing
				.as_ref()
				.is_some_and(|(channel, message, draft)| {
					*channel == submitted.channel
						&& *message == submitted.message
						&& *draft == submitted.draft
				}) && !state
				.message_actions
				.edit_pending(submitted.channel, submitted.message)
				&& state
					.timeline
					.get(submitted.message)
					.is_some_and(|message| {
						message.channel == submitted.channel && message.content == submitted.content
					})
		}) {
			self.editing = None;
			self.edit_sent = None;
		}
	}
	/// Selected-file cards above the composer input, in the style of Discord's upload tray.
	pub(super) fn attachment_tray(&mut self, ui: &mut egui::Ui, can_host: bool, upload_limit: u64) {
		let colors = design::palette(ui);
		let textures = self.attachment_textures(ui.ctx());
		ui.add_space(4.0);
		let files = self.selected_files();
		// Public hosting is only offered for files Discord would refuse.
		if files.iter().any(|(_, bytes)| *bytes > upload_limit) {
			design::notice(ui, design::Level::Error, &oversized_text(upload_limit));
			ui.add_space(6.0);
		}
		egui::ScrollArea::horizontal()
			.id_salt("pending-attachments")
			.show(ui, |ui| {
				crate::scroll::claim_wheel_content(ui);
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					for (index, (filename, bytes)) in files.iter().enumerate() {
						ui.push_id(index, |ui| {
							ui.vertical(|ui| {
								if ui
									.scope(|ui| {
										attachments::pending_card(
											ui,
											filename,
											*bytes,
											textures.get(index).and_then(Option::as_ref),
											!self.attach_busy,
										)
									})
									.inner
								{
									self.remove_attachment_index = Some(index);
								}
								if *bytes > upload_limit
									&& ui
										.add_enabled_ui(!self.upload_busy && can_host, |ui| {
											design::button(
												ui,
												"public-upload-host-file",
												design::ButtonKind::Neutral,
											)
										})
										.inner
										.clicked()
								{
									self.host_attachment_requested = Some(index);
								}
							});
						});
					}
					for index in 0..self.attachment_loading.min(10) {
						ui.push_id(("loading", index), attachments::loading_card);
					}
				});
			});
		ui.add_space(8.0);
		let line = ui.max_rect().x_range();
		let y = ui.cursor().top();
		ui.painter()
			.hline(line, y, egui::Stroke::new(1.0, colors.border));
		ui.add_space(6.0);
	}
	pub(super) fn apply_pending_mention(
		&mut self,
		ctx: &egui::Context,
		composer_id: egui::Id,
		draft: &mut String,
		remaining: usize,
		editing: bool,
	) -> Option<MentionWrite> {
		let user_id = self.pending_mention.take()?;
		if user_id == Id(0) {
			return None;
		}
		let loaded = egui::text_edit::TextEditState::load(ctx, composer_id);
		let had_editor = loaded.is_some();
		let mut edit_state = loaded.unwrap_or_default();
		let range = edit_state
			.cursor
			.char_range()
			.filter(|_| had_editor)
			.or_else(|| {
				Some(egui::text::CCursorRange::one(egui::text::CCursor::new(
					draft.chars().count(),
				)))
			});
		let start = range.map_or(draft.chars().count(), |range| {
			range
				.as_sorted_char_range()
				.start
				.0
				.min(draft.chars().count())
		});
		let glue = start > 0
			&& draft
				.chars()
				.nth(start - 1)
				.is_some_and(|c| !c.is_whitespace());
		let token = mentions::user_mention_token(user_id);
		let token = if glue { format!(" {token}") } else { token };
		let Some(cursor) = emoji_picker::insert(draft, &token, range, remaining, editing) else {
			return Some(MentionWrite::DidNotFit);
		};
		edit_state
			.cursor
			.set_char_range(Some(egui::text::CCursorRange::one(
				egui::text::CCursor::new(cursor),
			)));
		edit_state.store(ctx, composer_id);
		Some(MentionWrite::Inserted)
	}
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
		match shape {
			egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
			egui::Shape::Vec(shapes) => {
				for shape in shapes {
					collect_text(shape, out);
				}
			}
			_ => {}
		}
	}

	pub(crate) fn frame(ctx: &egui::Context, show: impl FnMut(&mut egui::Ui)) -> Vec<String> {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 700.0),
				)),
				..Default::default()
			},
			show,
		);
		let mut labels = Vec::new();
		for shape in &output.shapes {
			collect_text(&shape.shape, &mut labels);
		}
		output.drop_without_applying_deltas();
		labels
	}

	#[test]
	fn cancelled_paste_is_visible_once_in_the_selected_language() {
		struct ResetLanguage;
		impl Drop for ResetLanguage {
			fn drop(&mut self) {
				crate::i18n::set_current(crate::i18n::Language::English);
			}
		}
		let _reset = ResetLanguage;
		crate::i18n::set_current(crate::i18n::Language::Spanish);
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let channel = state.selected.expect("demo conversation");
		state.drafts.insert(channel, "Keep this draft".into());
		let mut view = MessagingUi::default();
		let target = egui::Id::unique("synthetic-composer");
		for _ in 0..2 {
			view.pasted_text = Some((channel, target, "clipboard text".into()));
			frame(&ctx, |ui| {
				view.clipboard_paste(ui.ctx(), &mut state, channel, target, false);
				view.toasts.show(ui.ctx(), 0.0);
			});
		}
		let labels = frame(&ctx, |ui| view.toasts.show(ui.ctx(), 0.0));
		assert_eq!(
			labels
				.iter()
				.filter(|text| *text == "Pegado cancelado; enfoca el mensaje e inténtalo de nuevo")
				.count(),
			1,
			"Repeated failures produce one translated notice"
		);
		assert_eq!(
			state.status,
			"Paste cancelled; focus the message and paste again"
		);
		assert_eq!(state.drafts[&channel], "Keep this draft");
		assert!(view.attachment_paste_requested.is_none());
	}

	#[test]
	fn rejected_send_keeps_the_draft_and_explains_why() {
		crate::i18n::set_current(crate::i18n::Language::English);
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let channel = state.selected.expect("demo conversation");
		for _ in 0..64 {
			state.drafts.insert(channel, "Queued".into());
			assert!(state.prepare_send().is_some());
		}
		state.drafts.insert(channel, "Keep this draft".into());
		let mut view = MessagingUi::default();
		let mut commands = Vec::new();
		for _ in 0..2 {
			frame(&ctx, |ui| {
				view.submit_composer(&mut state, ui.ctx(), &mut commands);
				view.toasts.show(ui.ctx(), 0.0);
			});
		}
		let labels = frame(&ctx, |ui| view.toasts.show(ui.ctx(), 0.0));
		assert_eq!(
			labels
				.iter()
				.filter(|text| *text
					== "Message could not be queued. Free draft space or wait for pending messages to finish.")
				.count(),
			1
		);
		assert!(commands.is_empty());
		assert_eq!(state.drafts[&channel], "Keep this draft");
		assert_eq!(state.pending.len(), 64);
	}
}
