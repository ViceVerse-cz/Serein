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
					"Mark As Read",
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
						"Remove From Favorites"
					} else {
						"Add To Favorites"
					},
					view.available() && state.channel(post.id).is_some(),
					false,
				)
				.on_hover_text("Favorites are saved on this device.")
				.clicked()
				{
					self.shortcut_requested = Some(view.toggle(Shortcut::Favorite, post.id));
					ui.close();
				}
				ui.separator();
				let followed = details.is_some_and(|d| d.followed);
				if row(
					ui,
					&format!("{} {noun}", if followed { "Unfollow" } else { "Follow" }),
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
						&format!("{} {noun}", if archived { "Open" } else { "Close" }),
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
						&format!("{} {noun}", if locked { "Unlock" } else { "Lock" }),
						ready,
						false,
					)
					.clicked()
				{
					intent = Some(Intent::Write(Action::PostLock(!locked)));
				}
				if state.can_edit_post(post.id)
					&& row(ui, &format!("Edit {noun}"), ready, false).clicked()
				{
					intent = Some(Intent::Edit);
				}
				if row(ui, "Copy Link", true, false).clicked() {
					if let Some(guild) = post.guild {
						ui.ctx()
							.copy_text(format!("https://discord.com/channels/{guild}/{}", post.id));
					}
					ui.close();
				}
				ui.separator();
				ui.add_enabled_ui(ready && followed, |ui| {
					if details.is_some_and(|d| d.muted)
						&& row(ui, &format!("Unmute {noun}"), true, false).clicked()
					{
						intent = Some(Intent::Write(Action::PostMute(Mute::Unmute)));
					}
					ui.menu_button(format!("Mute {noun}"), |ui| {
						for (label, mute) in [
							("For 15 Minutes", Mute::For(900)),
							("For 1 Hour", Mute::For(3600)),
							("For 3 Hours", Mute::For(10800)),
							("For 8 Hours", Mute::For(28800)),
							("For 24 Hours", Mute::For(86400)),
							("Until I Turn It Back On", Mute::Forever),
						] {
							if row(ui, label, true, false).clicked() {
								intent = Some(Intent::Write(Action::PostMute(mute)));
							}
						}
					});
					ui.menu_button("Notification Settings", |ui| {
						for (level, label) in [
							(0, "All Messages"),
							(1, "Only @mentions"),
							(2, "Nothing"),
							(3, "Use Default"),
						] {
							if ui
								.selectable_label(details.is_some_and(|d| d.level == level), label)
								.clicked()
							{
								intent = Some(Intent::Write(Action::PostNotifications(level)));
							}
						}
					});
				})
				.response
				.on_disabled_hover_text(format!(
					"Follow this {} to change its notifications.",
					noun.to_lowercase()
				));
				if state.can_manage_post(post.id) {
					ui.separator();
					let pinned = details.is_some_and(|d| d.pinned);
					if row(
						ui,
						&format!("{} {noun}", if pinned { "Unpin" } else { "Pin" }),
						ready,
						false,
					)
					.clicked()
					{
						intent = Some(Intent::Write(Action::PostPin(!pinned)));
					}
					if row(ui, &format!("Delete {noun}"), ready, true).clicked() {
						intent = Some(Intent::Delete);
					}
				}
				ui.separator();
				if row(ui, "Copy Thread ID", true, false).clicked() {
					ui.ctx().copy_text(post.id.to_string());
					ui.close();
				}
				if self.load.is_some() || state.channel_action_pending() {
					ui.label(format!("Loading {} settings…", noun.to_lowercase()));
				} else if let Some(error) = state
					.channel_action_status(post.id)
					.filter(|_| !state.channel_action_succeeded(post.id))
				{
					ui.colored_label(colors.danger, error);
					if ui.button("Retry").clicked() {
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
									.unwrap_or("The action could not be started."),
							)
						});
						let mut close = false;
						d.footer(|ui| {
							close =
								dialog::action(ui, "Dismiss", dialog::Action::Primary).clicked();
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
			format!("Delete {noun}?")
		} else {
			format!("Edit {noun}")
		};
		let mut builder =
			dialog::Dialog::new(("post-editor", self.generation), &title).width(420.0);
		if editor.delete {
			builder = builder.danger();
		}
		let delete_label = format!("Delete {noun}");
		let result = builder.show(ctx, |d| {
			d.content(|ui| {
				if editor.delete {
					ui.label(format!(
						"Delete {}? Its messages will be permanently deleted. This cannot be undone.",
						editor.name
					));
				} else {
					let label = dialog::label(ui, &format!("{noun} title"));
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
						"You no longer have permission to change this conversation.",
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
								"Save Changes"
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
				close = dialog::action(ui, "Cancel", dialog::Action::Neutral).clicked();
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
		"Post"
	} else {
		"Thread"
	}
}
