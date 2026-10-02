//! Forum post menus share the bounded channel action worker and local favorites.
use crate::{channel_menu::row, design, dialog, shortcuts::ShortcutView, user_menu};
use client_core::{
	Command, State,
	channel_actions::{Action, Mute},
};
use model::{Channel, Id, Shortcut};

enum Intent {
	Read,
	Write(Action),
	Edit,
	Delete,
}

struct Editor {
	channel: Id,
	name: String,
	delete: bool,
	submitted: bool,
}

#[derive(Default)]
pub(super) struct PostMenu {
	pub shortcut_requested: Option<crate::shortcuts::Intent>,
	requested: Option<(Id, Intent)>,
	load: Option<Id>,
	opened: Option<egui::Id>,
	editor: Option<Editor>,
	feedback: Option<Id>,
	failure: Option<Id>,
	generation: u64,
}

impl PostMenu {
	pub fn context(
		&mut self,
		response: &egui::Response,
		state: &State,
		post: &Channel,
		view: ShortcutView<'_>,
	) {
		if self.generation != state.generation {
			*self = Self {
				generation: state.generation,
				..Self::default()
			};
		}
		let id = response.id.with(("post-menu", state.generation));
		let colors = design::palette_for(&response.ctx);
		let shown = user_menu::popup(response, id)
			.frame(
				egui::Frame::popup(&response.ctx.style_of(response.ctx.theme()))
					.fill(colors.chat)
					.inner_margin(8)
					.corner_radius(8),
			)
			.show(|ui| {
				if self.opened != Some(id) {
					self.opened = Some(id);
					self.load = Some(post.id);
				}
				ui.set_width(232.0);
				// Forum posts and text-channel threads share this menu; only the noun differs.
				let noun = noun(state, post.id);
				let available =
					(state.demo || state.gateway_connected) && !state.channel_action_pending();
				let details = state.post_details(post.id);
				let ready = available && details.is_some() && self.load.is_none();
				let mut intent = None;
				if row(
					ui,
					"post-menu-context-mark-as-read",
					state.can_mark_channel_read(post.id),
					false,
				)
				.clicked()
				{
					intent = Some(Intent::Read);
				}
				ui.separator();
				if row(
					ui,
					if view.contains(Shortcut::Favorite, post.id) {
						"post-menu-context-remove-from-favorites"
					} else {
						"post-menu-context-add-to-favorites"
					},
					view.available() && state.channel(post.id).is_some(),
					false,
				)
				.on_hover_text(crate::i18n::translate(
					"post-menu-context-favorites-are-saved-on-this-device",
				))
				.clicked()
				{
					self.shortcut_requested = Some(view.toggle(Shortcut::Favorite, post.id));
					ui.close();
				}
				ui.separator();
				let followed = details.is_some_and(|d| d.followed);
				if row(
					ui,
					&if followed {
						noun_action(
							noun,
							"post-menu-context-unfollow-post",
							"post-menu-context-unfollow-thread",
						)
					} else {
						noun_action(
							noun,
							"post-menu-context-follow-post",
							"post-menu-context-follow-thread",
						)
					},
					ready && details.is_some_and(|d| !d.archived),
					false,
				)
				.clicked()
				{
					intent = Some(Intent::Write(Action::PostFollow(!followed)));
				}
				let archived = details.is_some_and(|d| d.archived);
				let locked = details.is_some_and(|d| d.locked);
				if state.can_edit_post(post.id)
					&& row(
						ui,
						&if archived {
							noun_action(
								noun,
								"post-menu-context-open-post",
								"post-menu-context-open-thread",
							)
						} else {
							noun_action(
								noun,
								"post-menu-context-close-post",
								"post-menu-context-close-thread",
							)
						},
						ready && (!archived || !locked || state.can_manage_post(post.id)),
						false,
					)
					.clicked()
				{
					intent = Some(Intent::Write(Action::PostArchive(!archived)));
				}
				if state.can_manage_post(post.id)
					&& row(
						ui,
						&if locked {
							noun_action(
								noun,
								"post-menu-context-unlock-post",
								"post-menu-context-unlock-thread",
							)
						} else {
							noun_action(
								noun,
								"post-menu-context-lock-post",
								"post-menu-context-lock-thread",
							)
						},
						ready,
						false,
					)
					.clicked()
				{
					intent = Some(Intent::Write(Action::PostLock(!locked)));
				}
				if state.can_edit_post(post.id)
					&& row(
						ui,
						&noun_action(
							noun,
							"post-menu-context-edit-post",
							"post-menu-context-edit-thread",
						),
						ready,
						false,
					)
					.clicked()
				{
					intent = Some(Intent::Edit);
				}
				if row(ui, "post-menu-context-copy-link", true, false).clicked() {
					if let Some(guild) = post.guild {
						ui.ctx()
							.copy_text(format!("https://discord.com/channels/{guild}/{}", post.id));
					}
					ui.close();
				}
				ui.separator();
				ui.add_enabled_ui(ready && followed, |ui| {
					let unmute = noun_action(
						noun,
						"post-menu-context-unmute-post",
						"post-menu-context-unmute-thread",
					);
					if details.is_some_and(|d| d.muted) && row(ui, &unmute, true, false).clicked() {
						intent = Some(Intent::Write(Action::PostMute(Mute::Unmute)));
					}
					let mute = noun_action(
						noun,
						"post-menu-context-mute-post",
						"post-menu-context-mute-thread",
					);
					ui.menu_button(mute, |ui| {
						for (label, mute) in [
							(
								"channel-menu-report-capacity-for-15-minutes",
								Mute::For(900),
							),
							("channel-menu-report-capacity-for-1-hour", Mute::For(3600)),
							("channel-menu-report-capacity-for-3-hours", Mute::For(10800)),
							("channel-menu-report-capacity-for-8-hours", Mute::For(28800)),
							(
								"channel-menu-report-capacity-for-24-hours",
								Mute::For(86400),
							),
							(
								"channel-menu-context-until-i-turn-it-back-on",
								Mute::Forever,
							),
						] {
							if row(ui, label, true, false).clicked() {
								intent = Some(Intent::Write(Action::PostMute(mute)));
							}
						}
					});
					ui.menu_button(
						crate::i18n::translate("post-menu-context-notification-settings"),
						|ui| {
							for (level, label) in [
								(0, "channel-menu-report-capacity-all-messages"),
								(1, "channel-menu-report-capacity-only-mentions"),
								(2, "channel-menu-report-capacity-nothing"),
								(3, "profile-edit-form-use-default"),
							] {
								if ui
									.selectable_label(
										details.is_some_and(|d| d.level == level),
										crate::i18n::translate_if_key(label),
									)
									.clicked()
								{
									intent = Some(Intent::Write(Action::PostNotifications(level)));
								}
							}
						},
					);
				})
				.response
				.on_disabled_hover_text(noun_action(
					noun,
					"post-menu-context-follow-this-post-to-change-its-notifications",
					"post-menu-context-follow-this-thread-to-change-its-notifications",
				));
				if state.can_manage_post(post.id) {
					ui.separator();
					let pinned = details.is_some_and(|d| d.pinned);
					if row(
						ui,
						&if pinned {
							noun_action(
								noun,
								"post-menu-context-unpin-post",
								"post-menu-context-unpin-thread",
							)
						} else {
							noun_action(
								noun,
								"post-menu-context-pin-post",
								"post-menu-context-pin-thread",
							)
						},
						ready,
						false,
					)
					.clicked()
					{
						intent = Some(Intent::Write(Action::PostPin(!pinned)));
					}
					if row(
						ui,
						&noun_action(
							noun,
							"post-menu-context-delete-post",
							"post-menu-context-delete-thread",
						),
						ready,
						true,
					)
					.clicked()
					{
						intent = Some(Intent::Delete);
					}
				}
				ui.separator();
				if row(ui, "post-menu-context-copy-thread-id", true, false).clicked() {
					ui.ctx().copy_text(post.id.to_string());
					ui.close();
				}
				if self.load.is_some() || state.channel_action_pending() {
					ui.label(format!(
						"{} {} {}…",
						crate::i18n::translate("post-menu-context-loading"),
						crate::i18n::translate_if_key(noun).to_lowercase(),
						crate::i18n::translate("post-menu-context-settings")
					));
				} else if let Some(error) = state
					.channel_action_status(post.id)
					.filter(|_| !state.channel_action_succeeded(post.id))
				{
					ui.colored_label(colors.danger, error);
					if ui
						.button(crate::i18n::translate("post-menu-context-retry"))
						.clicked()
					{
						self.load = Some(post.id);
					}
				}
				if let Some(intent) = intent {
					self.requested = Some((post.id, intent));
					ui.close();
				}
			});
		if shown.is_none() && self.opened == Some(id) {
			self.opened = None;
		}
	}

	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Option<Id>,
		commands: &mut Vec<Command>,
	) {
		if self.generation != state.generation {
			*self = Self {
				generation: state.generation,
				..Self::default()
			};
			return;
		}
		if let Some(id) = self.load
			&& !state.channel_action_pending()
		{
			self.load = None;
			if state.channel(id).is_none() {
				let _ = state.admit_archived_thread(id);
			}
			if state.channel(id).is_some_and(|c| c.guild == guild)
				&& let Some(command) = state.request_channel_action(id, Action::PostLoad)
			{
				commands.push(command);
			}
		}
		if let Some((id, intent)) = self.requested.take()
			&& let Some(post) = state.channel(id).filter(|c| c.guild == guild)
		{
			match intent {
				Intent::Read => {
					if let Some(command) = state.prepare_mark_channel_read(id) {
						commands.push(command);
					}
				}
				Intent::Write(action) => {
					let on_fail =
						matches!(action, Action::PostMute(_) | Action::PostNotifications(_));
					if let Some(command) = state.request_channel_action(id, action) {
						commands.push(command);
						if on_fail {
							self.failure = Some(id);
						} else {
							self.feedback = Some(id);
						}
					} else {
						self.feedback = Some(id);
					}
				}
				Intent::Edit | Intent::Delete => {
					self.editor = Some(Editor {
						channel: id,
						name: post.name.chars().take(100).collect(),
						delete: matches!(intent, Intent::Delete),
						submitted: false,
					});
					state.clear_channel_action_result(id);
				}
			}
		}
		if let Some(id) = self.failure.take() {
			if state.channel_action_pending() {
				self.failure = Some(id);
			} else if !state.channel_action_succeeded(id)
				&& state.channel_action_status(id).is_some()
			{
				self.feedback = Some(id);
			}
		}
		if let Some(id) = self.feedback {
			if state.channel_action_succeeded(id)
				|| state.channel(id).is_none_or(|c| c.guild != guild)
			{
				self.feedback = None;
			} else if !state.channel_action_pending() {
				let title = format!("{} action", noun(state, id));
				let result = dialog::Dialog::new("post-action-error", &title)
					.width(380.0)
					.show(ctx, |d| {
						d.content(|ui| {
							dialog::notice(
								ui,
								dialog::Level::Error,
								state
									.channel_action_status(id)
									.unwrap_or("post-menu-show-the-action-could-not-be-started"),
							)
						});
						let mut close = false;
						d.footer(|ui| {
							close = dialog::action(
								ui,
								"post-menu-show-dismiss",
								dialog::Action::Primary,
							)
							.clicked();
						});
						close
					});
				if result.inner || result.close {
					self.feedback = None;
				}
			}
		}
		let Some(editor) = &mut self.editor else {
			return;
		};
		if state
			.channel(editor.channel)
			.is_none_or(|c| c.guild != guild)
			|| (editor.submitted && state.channel_action_succeeded(editor.channel))
		{
			self.editor = None;
			return;
		}
		let allowed = if editor.delete {
			state.can_manage_post(editor.channel)
		} else {
			state.can_edit_post(editor.channel)
		};
		let mut close = false;
		let noun = noun(state, editor.channel);
		let title = if editor.delete {
			format!(
				"{} {}?",
				crate::i18n::translate("post-menu-show-delete"),
				crate::i18n::translate_if_key(noun)
			)
		} else {
			format!(
				"{} {}",
				crate::i18n::translate("post-menu-show-edit"),
				crate::i18n::translate_if_key(noun)
			)
		};
		let mut builder =
			dialog::Dialog::new(("post-editor", self.generation), &title).width(420.0);
		if editor.delete {
			builder = builder.danger();
		}
		let delete_label = format!(
			"{} {}",
			crate::i18n::translate("post-menu-show-delete"),
			crate::i18n::translate_if_key(noun)
		);
		let result = builder.show(ctx, |d| {
			d.content(|ui| {
				if editor.delete {
					ui.label(format!(
						"{} {}? {}",
						crate::i18n::translate("post-menu-show-delete"),
						editor.name,
						crate::i18n::translate(
							"post-menu-show-its-messages-will-be-permanently-deleted-this-cannot-be-undone"
						)
					));
				} else {
					let label = dialog::label(
						ui,
						&format!(
							"{} {}",
							crate::i18n::translate_if_key(noun),
							crate::i18n::translate("post-menu-show-title")
						),
					);
					dialog::input(
						ui,
						egui::TextEdit::singleline(&mut editor.name).char_limit(100),
					)
					.labelled_by(label.id);
					editor.name.shrink_to_fit();
				}
				if !allowed {
					dialog::notice(
						ui,
						dialog::Level::Warning,
						"post-menu-show-you-no-longer-have-permission-to-change-this-conversation",
					);
				}
				if let Some(error) = state
					.channel_action_status(editor.channel)
					.filter(|_| !state.channel_action_succeeded(editor.channel))
				{
					dialog::notice(ui, dialog::Level::Error, error);
				}
			});
			d.footer(|ui| {
				ui.add_enabled_ui(
					allowed
						&& !state.channel_action_pending()
						&& (state.demo || state.gateway_connected)
						&& (editor.delete
							|| client_core::channel_actions::valid_name(&editor.name)),
					|ui| {
						if dialog::action(
							ui,
							if editor.delete {
								delete_label.as_str()
							} else {
								"post-menu-show-save-changes"
							},
							if editor.delete {
								dialog::Action::Danger
							} else {
								dialog::Action::Primary
							},
						)
						.clicked()
						{
							let action = if editor.delete {
								Action::Delete
							} else {
								Action::PostRename(editor.name.clone())
							};
							if let Some(command) =
								state.request_channel_action(editor.channel, action)
							{
								commands.push(command);
								editor.submitted = true;
							}
						}
					},
				);
				close =
					dialog::action(ui, "post-menu-show-cancel", dialog::Action::Neutral).clicked();
			});
		});
		if close || result.close {
			if editor.submitted {
				self.feedback = Some(editor.channel);
			}
			self.editor = None;
		}
	}
}

/// "Post" inside a forum, "Thread" anywhere else; both use the same bounded channel actions.
fn noun(state: &State, channel: Id) -> &'static str {
	if state.is_forum_post(channel) {
		"post-menu-noun-post"
	} else {
		"post-menu-noun-thread"
	}
}

fn noun_action(noun: &str, post: &str, thread: &str) -> String {
	crate::i18n::translate_if_key(if noun == "post-menu-noun-action-post" {
		post
	} else {
		thread
	})
}
