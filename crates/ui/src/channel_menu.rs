//! Guild channel actions share one context menu and a session-scoped editor.
use crate::shortcuts::ShortcutView;
use crate::{design, dialog, icons, user_menu};
use client_core::{
	Command, State,
	channel_actions::{Action, CreateKind, Edit, Mute},
};
use model::{Channel, Id, Shortcut};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
	Edit,
	Duplicate,
	Create,
	CreateCategory,
	Delete,
}

enum Intent {
	Read,
	Dialog(Kind),
	Write(Action),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
	Overview,
	Permissions,
	Integrations,
}

struct Dialog {
	channel: Id,
	guild: Id,
	kind: Kind,
	create_kind: CreateKind,
	draft: Edit,
	before: Edit,
	loaded: bool,
	submitted: bool,
	page: Page,
	integrations: crate::server_integrations::IntegrationsUi,
	integrations_opened: bool,
	discard: bool,
	permissions: crate::channel_permissions::PermissionsUi,
	forum: crate::forum_settings::ForumSettingsUi,
}

#[derive(Default)]
pub(super) struct ChannelMenu {
	posts: crate::post_menu::PostMenu,
	pub invite_requested: Option<(Id, Id)>,
	pub shortcut_requested: Option<crate::shortcuts::Intent>,
	requested: Option<(Id, Intent)>,
	dialog: Option<Dialog>,
	feedback: Option<Id>,
	failure: Option<Id>,
	preference_error: bool,
	generation: u64,
}

impl ChannelMenu {
	pub fn is_open(&self) -> bool {
		self.dialog.is_some()
	}

	/// Fixture-only: open the settings dialog for `channel`, as its context menu would.
	#[cfg(feature = "demo")]
	pub fn preview_settings(&mut self, channel: Id, generation: u64) {
		self.generation = generation;
		self.requested = Some((channel, Intent::Dialog(Kind::Edit)));
	}

	/// Fixture-only: open the same creation dialog used by the channel menu.
	#[cfg(feature = "demo")]
	pub fn preview_creation(&mut self, channel: Id, generation: u64) {
		debug_assert_eq!(CreateKind::Announcement.wire_kind(), 5);
		self.generation = generation;
		self.requested = Some((channel, Intent::Dialog(Kind::Create)));
	}

	/// Shows the shared "full" feedback so DM, group and guild pins report capacity alike.
	pub fn report_capacity(&mut self, generation: u64) {
		self.preference_error = true;
		self.generation = generation;
	}

	pub fn context(
		&mut self,
		response: &egui::Response,
		state: &State,
		channel: &Channel,
		view: ShortcutView<'_>,
	) {
		let Some(guild) = channel.guild else { return };
		// Forum posts and text-channel threads both use the thread menu (close, rename, delete).
		if matches!(channel.kind, 10..=12) && state.is_thread_channel(channel.id) {
			self.posts.context(response, state, channel, view);
			if let Some(intent) = self.posts.shortcut_requested.take() {
				self.shortcut_requested = Some(intent);
			}
			self.generation = state.generation;
			return;
		}
		let colors = design::palette_for(&response.ctx);
		user_menu::popup(
			response,
			response.id.with(("channel-menu", state.generation)),
		)
		.frame(
			egui::Frame::popup(&response.ctx.style_of(response.ctx.theme()))
				.fill(colors.chat)
				.inner_margin(8)
				.corner_radius(8),
		)
		.show(|ui| {
			ui.set_width(232.0);
			ui.spacing_mut().button_padding = egui::vec2(12.0, 8.0);
			let available = (state.demo || state.gateway_connected)
				&& !state.channel_action_pending()
				&& state.can_view(channel.id);
			let mut intent = None;
			if row(
				ui,
				"Mark As Read",
				state.can_mark_channel_read(channel.id),
				false,
			)
			.clicked()
			{
				intent = Some(Intent::Read);
			}
			ui.separator();
			if channel.kind != 4
				&& row(
					ui,
					if view.contains(Shortcut::Favorite, channel.id) {
						"Remove From Favorites"
					} else {
						"Add To Favorites"
					},
					view.available(),
					false,
				)
				.on_hover_text("Favorites are saved on this device.")
				.clicked()
			{
				self.shortcut_requested = Some(view.toggle(Shortcut::Favorite, channel.id));
				self.generation = state.generation;
				ui.close();
			}
			ui.separator();
			if state.can_create_server_invite(guild, channel.id)
				&& row(
					ui,
					"Invite to Channel",
					available && !state.server_invite_pending() && !state.server_action_pending(),
					false,
				)
				.clicked()
			{
				self.invite_requested = Some((guild, channel.id));
				self.generation = state.generation;
				ui.close();
			}
			if row(ui, "Copy Link", true, false).clicked() {
				ui.ctx().copy_text(format!(
					"https://discord.com/channels/{guild}/{}",
					channel.id
				));
				ui.close();
			}
			ui.separator();
			ui.add_enabled_ui(available, |ui| {
				if state.guild_channel_muted(channel.id) == Some(true)
					&& row(ui, "Unmute Channel", true, false).clicked()
				{
					intent = Some(Intent::Write(Action::Mute(Mute::Unmute)));
				}
				ui.menu_button("Mute Channel", |ui| {
					for (label, seconds) in [
						("For 15 Minutes", 900),
						("For 1 Hour", 3600),
						("For 3 Hours", 10800),
						("For 8 Hours", 28800),
						("For 24 Hours", 86400),
					] {
						if row(ui, label, true, false).clicked() {
							intent = Some(Intent::Write(Action::Mute(Mute::For(seconds))));
						}
					}
					if row(ui, "Until I Turn It Back On", true, false).clicked() {
						intent = Some(Intent::Write(Action::Mute(Mute::Forever)));
					}
				});
				ui.menu_button("Notification Settings", |ui| {
					let level = state.channel_notification_level(channel.id);
					for (value, label) in [
						(0, "All Messages"),
						(1, "Only @mentions"),
						(2, "Nothing"),
						(3, "Use Server Default"),
					] {
						if ui.selectable_label(level == Some(value), label).clicked() {
							intent = Some(Intent::Write(Action::Notifications(value)));
						}
					}
				});
			});
			if state.can_open_channel_settings(channel.id) {
				ui.separator();
				if row(
					ui,
					if channel.kind == 4 {
						"Edit Category"
					} else {
						"Edit Channel"
					},
					available,
					false,
				)
				.clicked()
				{
					intent = Some(Intent::Dialog(Kind::Edit));
				}
			}
			if state.can_manage_channel(channel.id) {
				for (label, kind) in [
					(
						if channel.kind == 4 {
							"Duplicate Category"
						} else {
							"Duplicate Channel"
						},
						Kind::Duplicate,
					),
					("Create Channel", Kind::Create),
					(
						if channel.kind == 4 {
							"Delete Category"
						} else {
							"Delete Channel"
						},
						Kind::Delete,
					),
				] {
					if row(ui, label, available, kind == Kind::Delete).clicked() {
						intent = Some(Intent::Dialog(kind));
					}
				}
			}
			ui.separator();
			if row(ui, "Copy Channel ID", true, false).clicked() {
				ui.ctx().copy_text(channel.id.to_string());
				ui.close();
			}
			if let Some(intent) = intent {
				self.requested = Some((channel.id, intent));
				self.generation = state.generation;
				ui.close();
			}
		});
	}

	pub fn sidebar_context(
		&mut self,
		response: &egui::Response,
		state: &State,
		guild: Id,
		hide_muted: &mut bool,
	) {
		let colors = design::palette_for(&response.ctx);
		user_menu::popup(
			response,
			response.id.with(("server-channel-area", state.generation)),
		)
		.frame(
			egui::Frame::popup(&response.ctx.style_of(response.ctx.theme()))
				.fill(colors.chat)
				.inner_margin(8)
				.corner_radius(8),
		)
		.show(|ui| {
			ui.set_width(232.0);
			ui.spacing_mut().button_padding = egui::vec2(12.0, 8.0);
			if toggle_row(ui, "Hide Muted Channels", hide_muted).changed() {
				ui.close();
			}
			ui.separator();
			let available =
				(state.demo || state.gateway_connected) && !state.channel_action_pending();
			let anchor = state.channels.iter().find(|channel| {
				channel.guild == Some(guild) && state.can_manage_channel(channel.id)
			});
			if let Some(anchor) = anchor {
				for (label, kind) in [
					("Create Channel", Kind::Create),
					("Create Category", Kind::CreateCategory),
				] {
					if row(ui, label, available, false).clicked() {
						self.requested = Some((anchor.id, Intent::Dialog(kind)));
						self.generation = state.generation;
						ui.close();
					}
				}
			}
			if let Some(channel) = state
				.invite_channel(guild)
				.filter(|channel| state.can_create_server_invite(guild, *channel))
				&& row(
					ui,
					"Invite to Server",
					available && !state.server_invite_pending() && !state.server_action_pending(),
					false,
				)
				.clicked()
			{
				self.invite_requested = Some((guild, channel));
				self.generation = state.generation;
				ui.close();
			}
		});
	}

	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		active: Option<Id>,
		avatars: &mut crate::avatars::Avatars,
		commands: &mut Vec<Command>,
	) {
		if self.generation != state.generation {
			*self = Self::default();
			return;
		}
		self.posts.show(ctx, state, active, commands);
		if let Some((id, intent)) = self.requested.take()
			&& let Some(channel) = state.channel(id)
			&& let Some(guild) = channel.guild.filter(|g| Some(*g) == active)
		{
			match intent {
				Intent::Read => {
					if let Some(command) = state.prepare_mark_channel_read(id) {
						commands.push(command);
					}
				}
				Intent::Write(action) => {
					let on_fail = matches!(action, Action::Mute(_) | Action::Notifications(_));
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
				Intent::Dialog(kind) => {
					self.dialog = Some(Dialog {
						channel: id,
						guild,
						kind,
						create_kind: CreateKind::Text,
						draft: Edit {
							name: if matches!(kind, Kind::Create | Kind::CreateCategory) {
								String::new()
							} else {
								channel.name.chars().take(100).collect()
							},
							topic: String::new(),
							slowmode: 0,
							nsfw: false,
							overwrites: vec![],
							forum: None,
						},
						loaded: kind != Kind::Edit,
						before: Edit::default(),
						submitted: false,
						page: Page::Overview,
						integrations: crate::server_integrations::IntegrationsUi::for_channel(id),
						integrations_opened: false,
						discard: false,
						permissions: Default::default(),
						forum: Default::default(),
					});
					state.clear_channel_action_result(id);
					if kind == Kind::Edit
						&& let Some(command) = state.request_channel_action(id, Action::Load)
					{
						commands.push(command);
					}
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
		self.show_feedback(ctx, state, active);
		let Some(dialog) = &mut self.dialog else {
			return;
		};
		if active != Some(dialog.guild)
			|| state.channel(dialog.channel).is_none()
			|| (dialog.submitted && state.channel_action_succeeded(dialog.channel))
		{
			if dialog.integrations_opened {
				state.close_server_admin();
			}
			self.dialog = None;
			return;
		}
		dialog.integrations.sync(state, dialog.guild);
		if dialog.page == Page::Integrations {
			if !dialog.integrations_opened {
				state.close_server_admin();
				dialog.integrations_opened = true;
			}
			if let Some(command) = dialog.integrations.load(state, dialog.guild) {
				commands.push(command);
			}
		}
		let pending = state.channel_action_pending();
		if !dialog.loaded
			&& !pending
			&& state.channel_action_status(dialog.channel).is_none()
			&& let Some(details) = state.channel_details(dialog.channel)
		{
			dialog.draft = details.clone();
			dialog.before = details.clone();
			dialog.loaded = true;
		}
		let mut close = false;
		let pending_now = pending;
		let allowed = if dialog.kind == Kind::Edit {
			state.can_open_channel_settings(dialog.channel)
		} else {
			state.can_manage_channel(dialog.channel)
		};
		let category = state.channel(dialog.channel).is_some_and(|c| c.kind == 4);
		let mut delete_requested = false;
		let current = dialog.kind != Kind::Edit || state.channel_details(dialog.channel).is_some();
		let (title, subtitle) = match dialog.kind {
			Kind::Edit => (
				if category {
					"Category Settings"
				} else {
					"Channel Settings"
				},
				"Customize settings and who can do what here.",
			),
			Kind::Duplicate => (
				if category {
					"Duplicate Category"
				} else {
					"Duplicate Channel"
				},
				"Copies settings and permissions. Messages are not copied.",
			),
			Kind::Create => ("Create Channel", "Choose a channel type and name."),
			Kind::CreateCategory => ("Create Category", "Categories organize related channels."),
			Kind::Delete => (
				if category {
					"Delete Category?"
				} else {
					"Delete Channel?"
				},
				if category {
					"Deleting a category leaves its channels in the server."
				} else {
					"Deleting a channel removes its messages for everyone."
				},
			),
		};
		let mut builder = dialog::Dialog::new(("channel-dialog", self.generation), title)
			.subtitle(subtitle)
			.width(if dialog.kind == Kind::Edit {
				1080.0
			} else if dialog.kind == Kind::Create {
				480.0
			} else {
				420.0
			});
		if dialog.kind == Kind::Delete {
			builder = builder.danger();
		}
		let response = builder.show(ctx, |d| {
			d.scroll(220.0, |ui| {
				ui.spacing_mut().item_spacing.y = 10.0;
				if !allowed {
					dialog::notice(
						ui,
						dialog::Level::Warning,
						"You no longer have permission to manage this channel.",
					);
				}
				if !dialog.loaded {
					if pending_now {
						ui.horizontal(|ui| {
							ui.spinner();
							ui.label("Loading channel settings…");
						});
					} else {
						dialog::notice(
							ui,
							dialog::Level::Error,
							"Channel settings could not be loaded.",
						);
						if ui
							.add_enabled_ui(allowed, |ui| {
								dialog::action(ui, "Retry", dialog::Action::Neutral)
							})
							.inner
							.clicked() && let Some(command) =
							state.request_channel_action(dialog.channel, Action::Load)
						{
							commands.push(command);
						}
					}
				} else if dialog.kind == Kind::Delete {
					let colors = design::palette(ui);
					ui.add(
						egui::Label::new(
							egui::RichText::new(if category {
								format!("Delete {}? Its channels will remain in the server. This cannot be undone.", dialog.draft.name)
							} else {
								format!("Are you sure you want to delete #{}? Its messages will be permanently deleted. This cannot be undone.", dialog.draft.name)
							})
							.size(14.0)
							.color(colors.text),
						)
						.wrap(),
					);
				} else if dialog.kind == Kind::Edit {
					ui.add_enabled_ui(allowed && !pending_now && current, |ui| {
						delete_requested = dialog.editor(ui, state, avatars, commands);
					});
				} else if let Some(channel) = state.channel(dialog.channel) {
					ui.add_enabled_ui(allowed && !pending_now, |ui| dialog.overview(ui, channel));
					if dialog.kind == Kind::Create {
						let parent = if channel.kind == 4 {
							Some(channel)
						} else {
							channel.parent_id.and_then(|id| state.channel(id))
						};
						if let Some(parent) = parent {
							dialog::hint(
								ui,
								&format!("In {} · inherits category permissions", parent.name),
							);
						} else if channel.parent_id.is_some() {
							dialog::hint(ui, "In this channel’s category · inherits category permissions");
						} else {
							dialog::hint(ui, "At the top of this server · uses server permissions");
						}
					}
				}
				if let Some(status) = state.channel_action_status(dialog.channel) {
					dialog::notice(ui, dialog::Level::Error, status);
				}
				if dialog.loaded && !current {
					dialog::notice(
						ui,
						dialog::Level::Warning,
						"Channel settings need to be refreshed before saving. Reloading replaces this draft.",
					);
					if ui
						.add_enabled_ui(allowed && !pending_now, |ui| {
							dialog::action(ui, "Reload Channel", dialog::Action::Neutral)
						})
						.inner
						.clicked() && let Some(command) =
						state.request_channel_action(dialog.channel, Action::Load)
					{
						commands.push(command);
						dialog.loaded = false;
					}
				}
				if state.demo {
					dialog::hint(ui, "Offline preview · no server changes");
				}
			});
			d.footer(|ui| {
				let valid = dialog.kind == Kind::Delete
					|| if dialog.kind == Kind::Edit {
						dialog.draft.valid()
					} else {
						client_core::channel_actions::valid_name(&dialog.draft.name)
					};
				let label = if pending_now {
					"Working…"
				} else {
					match dialog.kind {
						Kind::Edit => "Save Changes",
						Kind::Duplicate => if category { "Duplicate Category" } else { "Duplicate Channel" },
						Kind::Create => "Create Channel",
						Kind::CreateCategory => "Create Category",
						Kind::Delete => if category { "Delete Category" } else { "Delete Channel" },
					}
				};
				let kind = if dialog.kind == Kind::Delete {
					dialog::Action::Danger
				} else {
					dialog::Action::Primary
				};
				if dialog.page != Page::Integrations {
					ui.add_enabled_ui(
						allowed
							&& dialog.loaded && current
							&& valid && !pending_now
							&& !dialog.integrations.has_changes()
							&& !(dialog.integrations_opened && state.server_admin.saving)
							&& (dialog.kind != Kind::Edit || dialog.draft != dialog.before)
							&& (state.demo || state.gateway_connected),
						|ui| {
							if dialog::action(ui, label, kind).clicked() {
								let action = match dialog.kind {
									Kind::Edit => Action::Edit {
										before: dialog.before.clone(),
										after: dialog.draft.clone(),
									},
									Kind::Duplicate => Action::Duplicate {
										name: dialog.draft.name.clone(),
									},
									Kind::Create => Action::Create {
										name: dialog.draft.name.clone(),
										kind: dialog.create_kind,
									},
									Kind::CreateCategory => Action::CreateCategory {
										name: dialog.draft.name.clone(),
									},
									Kind::Delete => Action::Delete,
								};
								if let Some(command) =
									state.request_channel_action(dialog.channel, action)
								{
									commands.push(command);
									dialog.submitted = true;
								}
							}
						},
					);
				}
				close |= dialog::action(
					ui,
					if pending_now { "Close" } else { "Cancel" },
					dialog::Action::Neutral,
				)
				.clicked();
			});
		});
		let overlay_was_open = dialog.integrations.overlay_open();
		dialog
			.integrations
			.overlays(ctx, state, dialog.guild, commands);
		let mut dismiss = (close || response.close) && !overlay_was_open;
		if dismiss
			&& (dialog.integrations.has_changes()
				|| state.server_admin.saving && dialog.integrations_opened)
		{
			dialog.discard = true;
			dismiss = false;
		}
		if dialog.discard {
			match dialog::Confirm::new(
				"discard-channel-webhook",
				"Discard webhook changes?",
				"Your unsaved webhook changes will be lost.",
			)
			.confirm_label("Discard")
			.enabled(!state.server_admin.saving)
			.show(ctx)
			{
				Some(dialog::Choice::Confirmed) => dismiss = true,
				Some(dialog::Choice::Cancelled) => dialog.discard = false,
				None => {}
			}
		}
		if delete_requested {
			self.requested = Some((dialog.channel, Intent::Dialog(Kind::Delete)));
			if dialog.integrations_opened {
				state.close_server_admin();
			}
			self.dialog = None;
		} else if dismiss {
			if dialog.integrations_opened {
				state.close_server_admin();
			}
			if pending {
				self.feedback = Some(dialog.channel);
			}
			self.dialog = None;
		}
	}

	fn show_feedback(&mut self, ctx: &egui::Context, state: &State, active: Option<Id>) {
		if self.feedback.is_some_and(|id| {
			state.channel_action_succeeded(id)
				|| state.channel(id).is_none_or(|c| c.guild != active)
		}) {
			self.feedback = None;
		}
		if self.feedback.is_none() && !self.preference_error {
			return;
		}
		let mut message = String::new();
		if self.preference_error {
			message.push_str(
				"Saved channel preferences are full. Remove a favorite or pin, or expand a category.",
			);
		}
		if let Some(id) = self.feedback {
			if !message.is_empty() {
				message.push_str("\n\n");
			}
			message.push_str(if state.channel_action_pending() {
				"Updating channel settings…"
			} else {
				state
					.channel_action_status(id)
					.unwrap_or("The channel action could not be started.")
			});
		}
		let dismissed = dialog::Dialog::new("channel-feedback", "Channel action")
			.width(380.0)
			.show(ctx, |d| {
				let mut dismissed = false;
				d.content(|ui| dialog::notice(ui, dialog::Level::Warning, &message));
				d.footer(|ui| {
					dismissed = dialog::action(ui, "Dismiss", dialog::Action::Primary).clicked();
				});
				dismissed
			});
		if dismissed.inner || dismissed.close {
			self.feedback = None;
			self.preference_error = false;
		}
	}
}

impl Dialog {
	fn overview(&mut self, ui: &mut egui::Ui, channel: &Channel) {
		if self.kind == Kind::Create {
			dialog::label(ui, "Channel type");
			design::card(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 2.0;
				for (kind, label, description) in [
					(
						CreateKind::Text,
						"Text",
						"Send messages, images, and files.",
					),
					(CreateKind::Voice, "Voice", "Hang out and talk together."),
					(
						CreateKind::Announcement,
						"Announcement",
						"Share updates. Requires a Community server.",
					),
					(
						CreateKind::Forum,
						"Forum",
						"Organize discussions into separate posts.",
					),
				] {
					if design::radio_row(ui, self.create_kind == kind, label, Some(description))
						.clicked()
					{
						self.create_kind = kind;
					}
				}
			});
			ui.add_space(6.0);
		}
		let label = dialog::label(
			ui,
			if self.kind == Kind::CreateCategory || (channel.kind == 4 && self.kind != Kind::Create)
			{
				"Category name"
			} else {
				"Channel name"
			},
		);
		let name = dialog::input(
			ui,
			egui::TextEdit::singleline(&mut self.draft.name)
				.hint_text(if self.kind == Kind::CreateCategory {
					"new-category"
				} else {
					"new-channel"
				})
				.char_limit(100),
		)
		.labelled_by(label.id);
		if name.changed() {
			self.draft.name.shrink_to_fit();
		}
		if self.kind == Kind::Edit && matches!(channel.kind, 0 | 5) {
			ui.add_space(14.0);
			let label = dialog::label(ui, "Topic");
			let topic = dialog::input(
				ui,
				egui::TextEdit::multiline(&mut self.draft.topic)
					.hint_text("Let everyone know how to use this channel")
					.char_limit(1024)
					.desired_rows(3),
			)
			.labelled_by(label.id);
			if topic.changed() {
				self.draft.topic.shrink_to_fit();
			}
			ui.add_space(14.0);
			dialog::label(ui, "Slowmode");
			crate::forum_settings::slowmode(ui, "slowmode", &mut self.draft.slowmode);
			dialog::hint(
				ui,
				"Members will be restricted to one message in this interval.",
			);
			ui.add_space(6.0);
			design::switch(
				ui,
				"Age-restricted channel",
				Some("Members must confirm they are of age before viewing."),
				&mut self.draft.nsfw,
			);
		}
	}
	fn navigation(
		&mut self,
		ui: &mut egui::Ui,
		channel: &Channel,
		can_delete: bool,
		can_integrate: bool,
		compact: bool,
	) -> bool {
		ui.add(
			egui::Label::new(design::eyebrow(
				ui,
				&channel.name,
				design::palette(ui).muted,
			))
			.truncate(),
		);
		ui.add_space(12.0);
		let pages: Vec<(Page, &str)> = [
			(Page::Overview, "Overview"),
			(Page::Permissions, "Permissions"),
			(Page::Integrations, "Integrations"),
		]
		.into_iter()
		.filter(|(page, _)| *page != Page::Integrations || can_integrate)
		.collect();
		if compact {
			let labels: Vec<&str> = pages.iter().map(|(_, label)| *label).collect();
			let selected = pages
				.iter()
				.position(|(page, _)| *page == self.page)
				.unwrap_or(usize::MAX);
			if let Some(index) = design::segmented(ui, &labels, selected) {
				self.page = pages[index].0;
			}
		} else {
			for (page, label) in &pages {
				if crate::settings::nav_item(ui, label, self.page == *page).clicked() {
					self.page = *page;
				}
			}
		}
		design::card_divider(ui);
		row(
			ui,
			if channel.kind == 4 {
				"Delete Category"
			} else {
				"Delete Channel"
			},
			can_delete && !self.integrations.has_changes(),
			true,
		)
		.clicked()
	}
	fn editor(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		avatars: &mut crate::avatars::Avatars,
		commands: &mut Vec<Command>,
	) -> bool {
		let Some(channel) = state.channel(self.channel).cloned() else {
			return false;
		};
		let can_delete = state.can_manage_channel(channel.id)
			&& !(self.integrations_opened && state.server_admin.saving);
		let can_integrate = state.can_manage_webhook_channel(self.guild, channel.id);
		if self.page == Page::Integrations && !can_integrate {
			self.page = Page::Overview;
		}
		let mut delete = false;
		let mut content = |this: &mut Self, ui: &mut egui::Ui| match this.page {
			Page::Permissions => {
				this.permissions
					.show(ui, state, &channel, &mut this.draft.overwrites)
			}
			Page::Integrations => this
				.integrations
				.show(ui, state, this.guild, avatars, commands),
			Page::Overview => {
				design::section(ui, "Overview", None);
				ui.add_enabled_ui(can_delete, |ui| {
					this.overview(ui, &channel);
					if matches!(channel.kind, 15 | 16) {
						this.forum
							.show(ui, state, this.guild, &mut this.draft, avatars);
					}
				});
			}
		};
		if ui.available_width() >= 850.0 {
			ui.horizontal_top(|ui| {
				ui.allocate_ui_with_layout(
					egui::vec2(180.0, 0.0),
					egui::Layout::top_down(egui::Align::Min),
					|ui| {
						ui.set_width(180.0);
						delete = self.navigation(ui, &channel, can_delete, can_integrate, false);
					},
				);
				ui.add_space(20.0);
				ui.vertical(|ui| content(self, ui));
			});
		} else {
			delete = self.navigation(ui, &channel, can_delete, can_integrate, true);
			content(self, ui);
		}
		delete
	}
}

fn toggle_row(ui: &mut egui::Ui, label: &str, value: &mut bool) -> egui::Response {
	let mut response = row(ui, label, true, false);
	if response.clicked() {
		*value = !*value;
		response.mark_changed();
	}
	response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, *value, label));
	let colors = design::palette(ui);
	let mark = egui::Rect::from_center_size(
		egui::pos2(response.rect.right() - 16.0, response.rect.center().y),
		egui::Vec2::splat(24.0),
	);
	ui.painter().rect(
		mark,
		4,
		if *value { colors.accent } else { colors.base },
		egui::Stroke::new(1.0, colors.border),
		egui::StrokeKind::Inside,
	);
	if *value {
		icons::paint(
			ui.painter(),
			icons::Icon::Check,
			mark.shrink(4.0),
			colors.text_strong,
		);
	}
	response
}

pub(super) fn row(ui: &mut egui::Ui, label: &str, enabled: bool, danger: bool) -> egui::Response {
	let colors = design::palette(ui);
	ui.add_enabled(
		enabled,
		egui::Button::new(())
			.left_text(design::medium(ui, label, 14.0).color(if danger {
				colors.danger
			} else {
				colors.text
			}))
			.min_size(egui::vec2(ui.available_width(), 34.0))
			.frame_when_inactive(false)
			.corner_radius(4),
	)
}

/// Offline debug command: exercise the real selector, submit button and permission gate.
#[cfg(all(debug_assertions, feature = "demo"))]
pub(super) fn debug_creation(mut state: State) {
	fn locate(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
		match shape {
			egui::Shape::Text(text) if text.galley.job.text == label => {
				Some(text.galley.rect.translate(text.pos.to_vec2()).center())
			}
			egui::Shape::Vec(shapes) => shapes.iter().rev().find_map(|s| locate(s, label)),
			_ => None,
		}
	}
	let ctx = egui::Context::default();
	design::apply(&ctx);
	assert!(state.demo);
	let channel = state
		.channels
		.iter()
		.find(|c| state.can_manage_channel(c.id))
		.unwrap()
		.id;
	let guild = state.channel(channel).unwrap().guild;
	let mut menu = ChannelMenu::default();
	menu.preview_creation(channel, state.generation);
	let mut commands = Vec::new();
	let mut frame = |menu: &mut ChannelMenu, state: &mut State, events, label| {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(760.0, 760.0),
				)),
				events,
				..Default::default()
			},
			|ui| {
				menu.show(
					ui.ctx(),
					state,
					guild,
					&mut crate::avatars::Avatars::default(),
					&mut commands,
				);
			},
		);
		let position = output
			.shapes
			.iter()
			.rev()
			.find_map(|s| locate(&s.shape, label));
		output.drop_without_applying_deltas();
		position
	};
	let pointer = |pos, pressed| {
		vec![
			egui::Event::PointerMoved(pos),
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed,
				modifiers: egui::Modifiers::NONE,
			},
		]
	};
	frame(&mut menu, &mut state, vec![], "Announcement");
	let choice = frame(&mut menu, &mut state, vec![], "Announcement").unwrap();
	for pressed in [true, false] {
		frame(
			&mut menu,
			&mut state,
			pointer(choice, pressed),
			"Announcement",
		);
	}
	assert_eq!(
		menu.dialog.as_ref().unwrap().create_kind,
		CreateKind::Announcement
	);
	menu.dialog.as_mut().unwrap().draft.name = "announcements".into();
	let permissions = state.permissions.clone();
	let metadata = state.permissions.guilds.get_mut(&guild.unwrap()).unwrap();
	metadata.owner = Some(Id(u64::MAX));
	for role in metadata.roles.as_mut().unwrap() {
		role.bits &= !(model::permissions::MANAGE_CHANNELS | model::permissions::ADMINISTRATOR);
	}
	state.permissions.clear_cache();
	assert!(state.can_view(channel) && !state.can_manage_channel(channel));
	assert!(
		state
			.request_channel_action(
				channel,
				Action::Create {
					name: "announcements".into(),
					kind: CreateKind::Announcement,
				}
			)
			.is_none()
	);
	frame(&mut menu, &mut state, vec![], "Create Channel");
	let submit = frame(&mut menu, &mut state, vec![], "Create Channel").unwrap();
	for pressed in [true, false] {
		frame(
			&mut menu,
			&mut state,
			pointer(submit, pressed),
			"Create Channel",
		);
	}
	assert!(!menu.dialog.as_ref().unwrap().submitted);
	state.permissions = permissions;
	state.clear_channel_action_result(channel);
	frame(&mut menu, &mut state, vec![], "Create Channel");
	let submit = frame(&mut menu, &mut state, vec![], "Create Channel").unwrap();
	for pressed in [true, false] {
		frame(
			&mut menu,
			&mut state,
			pointer(submit, pressed),
			"Create Channel",
		);
	}
	assert!(menu.dialog.as_ref().unwrap().submitted);
	assert!(matches!(commands.as_slice(), [Command::ChannelAction {
		action: Action::Create { name, kind: CreateKind::Announcement }, ..
	}] if name == "announcements"));
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
	use model::ChannelPreferences;

	fn labels(shape: &egui::Shape, output: &mut Vec<(String, Rect)>) {
		match shape {
			egui::Shape::Text(text) => output.push((
				text.galley.job.text.clone(),
				text.galley.rect.translate(text.pos.to_vec2()),
			)),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| labels(s, output)),
			_ => {}
		}
	}
	fn pointer(pos: Pos2, button: PointerButton, pressed: bool) -> Vec<Event> {
		vec![
			Event::PointerMoved(pos),
			Event::PointerButton {
				pos,
				button,
				pressed,
				modifiers: Modifiers::NONE,
			},
		]
	}
	struct Harness {
		state: State,
		menu: ChannelMenu,
		prefs: ChannelPreferences,
		commands: Vec<Command>,
		copied: Vec<String>,
		width: f32,
	}
	impl Harness {
		fn frame(
			&mut self,
			ctx: &egui::Context,
			events: Vec<Event>,
		) -> (egui::Response, Vec<(String, Rect)>) {
			let mut row = None;
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(Rect::from_min_size(
						Pos2::ZERO,
						egui::vec2(self.width, 760.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					let response = ui.button("#getting-started");
					self.menu.context(
						&response,
						&self.state,
						self.state.channel(Id(20)).unwrap(),
						ShortcutView::new(&self.prefs, true),
					);
					row = Some(response);
					self.menu.show(
						ui.ctx(),
						&mut self.state,
						Some(Id(10)),
						&mut crate::avatars::Avatars::default(),
						&mut self.commands,
					);
				},
			);
			let mut text = vec![];
			for shape in &output.shapes {
				labels(&shape.shape, &mut text);
			}
			for command in &output.platform_output.commands {
				if let egui::OutputCommand::CopyText(value) = command {
					self.copied.push(value.clone());
				}
			}
			output.drop_without_applying_deltas();
			(row.unwrap(), text)
		}
		fn click(&mut self, ctx: &egui::Context, pos: Pos2, button: PointerButton) {
			for pressed in [true, false] {
				self.frame(ctx, pointer(pos, button, pressed));
			}
		}
	}
	#[test]
	fn channel_integrations_load_and_create_in_the_selected_channel() {
		use model::server_integrations::{Action as IntegrationAction, Snapshot};
		for (width, light) in [(1120.0, false), (720.0, true)] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			if light {
				ctx.set_visuals(egui::Visuals::light());
			}
			let mut state = test_support::chat_demo_state();
			let mut permissions = test_support::permission_snapshot(&state);
			for guild in &mut permissions.guilds {
				guild.owner = state.user.as_ref().map(|u| u.id);
			}
			state.permissions.replace(permissions).unwrap();
			let mut h = Harness {
				state,
				menu: ChannelMenu::default(),
				prefs: Default::default(),
				commands: vec![],
				copied: vec![],
				width,
			};
			h.menu.generation = h.state.generation;
			h.menu.requested = Some((Id(20), Intent::Dialog(Kind::Edit)));
			h.frame(&ctx, vec![]);
			let Command::ChannelAction {
				guild,
				channel,
				request,
				..
			} = h.commands.pop().unwrap()
			else {
				panic!()
			};
			h.state.apply(client_core::Envelope {
				generation: h.state.generation,
				event: client_core::Event::ChannelAction(
					client_core::channel_actions::Event::Finished {
						guild,
						channel,
						request,
						result: Ok(client_core::channel_actions::Outcome::Details(Edit {
							name: "getting-started".into(),
							..Default::default()
						})),
					},
				),
			});
			let (_, text) = h.frame(&ctx, vec![]);
			let nav = text.iter().find(|(s, _)| s == "Integrations").unwrap().1;
			assert!(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 760.0)).contains_rect(nav));
			h.click(&ctx, nav.center(), PointerButton::Primary);
			h.frame(&ctx, vec![]);
			let Command::ServerAdmin {
				request, action, ..
			} = h.commands.pop().unwrap()
			else {
				panic!()
			};
			assert!(matches!(
				*action,
				model::server_admin::Action::Integrations(IntegrationAction::Load {
					channel: Some(Id(20)),
					integrations: false,
					webhooks: true
				})
			));
			h.state.apply(client_core::Envelope {
				generation: h.state.generation,
				event: client_core::Event::ServerAdmin(client_core::server_admin::Event {
					guild,
					request,
					result: Ok(model::server_admin::Result::Integrations(Snapshot {
						guild,
						channel: Some(channel),
						integrations: None,
						webhooks: Some(vec![]),
					})),
				}),
			});
			h.frame(&ctx, vec![]);
			let (_, text) = h.frame(&ctx, vec![]);
			h.click(
				&ctx,
				text.iter()
					.find(|(s, _)| s == "Webhooks")
					.unwrap()
					.1
					.center(),
				PointerButton::Primary,
			);
			let (_, text) = h.frame(&ctx, vec![]);
			assert!(
				text.iter().any(|(s, _)| s == "No webhooks yet."),
				"width {width}: {text:?}"
			);
			h.click(
				&ctx,
				text.iter()
					.find(|(s, _)| s == "New Webhook")
					.unwrap()
					.1
					.center(),
				PointerButton::Primary,
			);
			if width < 850.0 {
				h.frame(
					&ctx,
					vec![
						Event::PointerMoved(egui::pos2(width / 2.0, 500.0)),
						Event::MouseWheel {
							phase: egui::TouchPhase::Move,
							unit: egui::MouseWheelUnit::Point,
							delta: egui::vec2(0.0, -260.0),
							modifiers: Modifiers::NONE,
						},
					],
				);
			}
			let (_, text) = h.frame(&ctx, vec![]);
			let save = text.iter().find(|(s, _)| s == "Save Changes").unwrap().1;
			assert!(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 760.0)).contains_rect(save));
			h.click(&ctx, save.center(), PointerButton::Primary);
			assert!(h.commands.iter().any(|c| matches!(c, Command::ServerAdmin { action, .. } if matches!(action.as_ref(), model::server_admin::Action::Integrations(IntegrationAction::CreateWebhook { scope: Some(Id(20)), channel: Id(20), .. })))), "width {width}; commands {}; text {text:?}; error {:?}", h.commands.len(), h.state.server_admin.error);
		}
	}

	#[test]
	fn create_channel_selects_type_before_submitting() {
		for (kind, label, width, light) in [
			(CreateKind::Text, "Text", 1120.0, false),
			(CreateKind::Voice, "Voice", 760.0, true),
			(CreateKind::Forum, "Forum", 320.0, false),
		] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			ctx.set_visuals(if light {
				egui::Visuals::light()
			} else {
				egui::Visuals::dark()
			});
			let mut state = test_support::chat_demo_state();
			state
				.channels
				.iter_mut()
				.find(|c| c.id == Id(20))
				.unwrap()
				.kind = 4;
			let mut permissions = test_support::permission_snapshot(&state);
			for guild in &mut permissions.guilds {
				guild.owner = state.user.as_ref().map(|u| u.id);
			}
			state.permissions.replace(permissions).unwrap();
			let mut h = Harness {
				state,
				menu: ChannelMenu::default(),
				prefs: Default::default(),
				commands: vec![],
				copied: vec![],
				width,
			};
			let (row, _) = h.frame(&ctx, vec![]);
			h.click(&ctx, row.rect.center(), PointerButton::Secondary);
			let (_, text) = h.frame(&ctx, vec![]);
			h.click(
				&ctx,
				text.iter()
					.find(|(s, _)| s == "Create Channel")
					.unwrap()
					.1
					.center(),
				PointerButton::Primary,
			);
			let (_, text) = h.frame(&ctx, vec![]);
			assert!(text.iter().any(|(s, _)| s == "CHANNEL NAME"));
			assert!(!text.iter().any(|(s, _)| s == "CATEGORY NAME"));
			let viewport = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 760.0));
			for choice in ["Text", "Voice", "Forum"] {
				assert!(viewport.contains_rect(text.iter().find(|(s, _)| s == choice).unwrap().1));
			}
			h.click(
				&ctx,
				text.iter().find(|(s, _)| s == label).unwrap().1.center(),
				PointerButton::Primary,
			);
			assert_eq!(h.menu.dialog.as_ref().unwrap().create_kind, kind);
			assert!(h.commands.is_empty());
			h.menu.dialog.as_mut().unwrap().draft.name = "new-space".into();
			let (_, text) = h.frame(&ctx, vec![]);
			let submit = text
				.iter()
				.rev()
				.find(|(s, _)| s == "Create Channel")
				.unwrap()
				.1;
			assert!(viewport.contains_rect(submit));
			h.click(&ctx, submit.center(), PointerButton::Primary);
			assert!(matches!(h.commands.as_slice(), [Command::ChannelAction {
				action: Action::Create { name, kind: sent }, ..
			}] if name == "new-space" && *sent == kind));
		}
	}

	#[test]
	fn category_and_channel_permissions_load_edit_and_submit_a_preserved_snapshot() {
		use client_core::channel_actions::{Event as ChannelEvent, Outcome};
		use model::permissions as p;
		for (kind, width) in [(4, 1120.0), (0, 720.0), (2, 1120.0)] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let mut state = test_support::chat_demo_state();
			state
				.channels
				.iter_mut()
				.find(|c| c.id == Id(20))
				.unwrap()
				.kind = kind;
			let mut permissions = test_support::permission_snapshot(&state);
			for guild in &mut permissions.guilds {
				guild.owner = state.user.as_ref().map(|u| u.id);
			}
			state.permissions.replace(permissions).unwrap();
			let mut h = Harness {
				state,
				menu: ChannelMenu::default(),
				prefs: Default::default(),
				commands: vec![],
				copied: vec![],
				width,
			};
			let (row, _) = h.frame(&ctx, vec![]);
			h.click(&ctx, row.rect.center(), PointerButton::Secondary);
			let (_, text) = h.frame(&ctx, vec![]);
			let label = if kind == 4 {
				"Edit Category"
			} else {
				"Edit Channel"
			};
			h.click(
				&ctx,
				text.iter().find(|(s, _)| s == label).unwrap().1.center(),
				PointerButton::Primary,
			);
			let Command::ChannelAction {
				guild,
				channel,
				request,
				action: Action::Load,
			} = h.commands.pop().unwrap()
			else {
				panic!("open must load fresh settings")
			};
			let preserved = p::Overwrite {
				id: Id(88),
				kind: 1,
				allow: 1 << 90,
				deny: p::SEND_MESSAGES,
			};
			let before = Edit {
				name: "information".into(),
				overwrites: vec![preserved],
				..Default::default()
			};
			h.state.apply(client_core::Envelope {
				generation: h.state.generation,
				event: client_core::Event::ChannelAction(ChannelEvent::Finished {
					guild,
					channel,
					request,
					result: Ok(Outcome::Details(before.clone())),
				}),
			});
			let (_, text) = h.frame(&ctx, vec![]);
			h.click(
				&ctx,
				text.iter()
					.find(|(s, _)| s == "Permissions")
					.unwrap()
					.1
					.center(),
				PointerButton::Primary,
			);
			let (_, text) = h.frame(&ctx, vec![]);
			let private = if kind == 4 {
				"Private Category"
			} else {
				"Private Channel"
			};
			let rect = text.iter().find(|(s, _)| s == private).unwrap().1;
			assert!(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 760.0)).contains_rect(rect));
			h.click(&ctx, rect.center(), PointerButton::Primary);
			let (_, text) = h.frame(&ctx, vec![]);
			h.click(
				&ctx,
				text.iter()
					.find(|(s, _)| s == "Member 88")
					.unwrap()
					.1
					.center(),
				PointerButton::Primary,
			);
			let (_, text) = h.frame(&ctx, vec![]);
			h.click(
				&ctx,
				text.iter()
					.find(|(s, _)| s == "\u{2713}")
					.unwrap()
					.1
					.center(),
				PointerButton::Primary,
			);
			let (_, text) = h.frame(&ctx, vec![]);
			let save = text.iter().find(|(s, _)| s == "Save Changes").unwrap().1;
			assert!(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 760.0)).contains_rect(save));
			h.click(&ctx, save.center(), PointerButton::Primary);
			let Command::ChannelAction {
				action: Action::Edit {
					before: sent_before,
					after,
				},
				..
			} = h.commands.pop().unwrap()
			else {
				panic!("save must issue an edit")
			};
			assert_eq!(sent_before, before);
			assert!(after.overwrites.contains(&p::Overwrite {
				allow: preserved.allow | p::VIEW_CHANNEL,
				..preserved
			}));
			assert!(
				after.overwrites.iter().any(|o| o.id == guild
					&& o.kind == 0 && o.deny & p::VIEW_CHANNEL != 0
					&& o.allow & p::VIEW_CHANNEL == 0)
			);
			assert_eq!(after.name, before.name);
			assert!(h.commands.is_empty());
		}
	}

	#[test]
	fn context_menu_keyboard_mouse_permissions_and_delete_confirmation() {
		for light in [false, true] {
			for action in [
				"Add To Favorites",
				"Copy Channel ID",
				"Invite to Channel",
				"Delete Channel",
			] {
				let ctx = egui::Context::default();
				design::apply(&ctx);
				ctx.set_visuals(if light {
					egui::Visuals::light()
				} else {
					egui::Visuals::dark()
				});
				let mut state = test_support::chat_demo_state();
				let mut permissions = test_support::permission_snapshot(&state);
				for guild in &mut permissions.guilds {
					guild.owner = state.user.as_ref().map(|u| u.id);
				}
				state.permissions.replace(permissions).unwrap();
				let mut h = Harness {
					state,
					menu: ChannelMenu::default(),
					prefs: ChannelPreferences::default(),
					commands: vec![],
					copied: vec![],
					width: 320.0,
				};
				let (row, _) = h.frame(&ctx, vec![]);
				if light {
					row.request_focus();
					h.frame(
						&ctx,
						vec![Event::Key {
							key: egui::Key::F10,
							physical_key: None,
							pressed: true,
							repeat: false,
							modifiers: Modifiers::SHIFT,
						}],
					);
				} else {
					h.click(&ctx, row.rect.center(), PointerButton::Secondary);
				}
				let (_, text) = h.frame(&ctx, vec![]);
				for expected in [
					"Mark As Read",
					"Add To Favorites",
					"Invite to Channel",
					"Copy Link",
					"Mute Channel",
					"Notification Settings",
					"Edit Channel",
					"Duplicate Channel",
					"Create Channel",
					"Delete Channel",
					"Copy Channel ID",
				] {
					let rect = text
						.iter()
						.find(|(label, _)| label == expected)
						.unwrap_or_else(|| panic!("Missing {expected}: {text:?}"))
						.1;
					assert!(
						Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 760.0))
							.contains_rect(rect),
						"{expected} stays inside the viewport: {rect:?}"
					);
				}
				assert!(h.commands.is_empty());
				h.click(
					&ctx,
					text.iter()
						.find(|(label, _)| label == action)
						.unwrap()
						.1
						.center(),
					PointerButton::Primary,
				);
				assert!(
					h.commands.is_empty(),
					"Opening a dialog does not send a destructive action"
				);
				match action {
					"Add To Favorites" => assert_eq!(
						h.menu.shortcut_requested,
						Some(crate::shortcuts::Intent {
							channel: Id(20),
							kind: Shortcut::Favorite,
							on: true,
						})
					),
					"Copy Channel ID" => assert_eq!(h.copied, ["20"]),
					"Invite to Channel" => {
						assert_eq!(h.menu.invite_requested, Some((Id(10), Id(20))))
					}
					_ => {
						let (_, text) = h.frame(&ctx, vec![]);
						h.click(
							&ctx,
							text.iter()
								.find(|(label, _)| label == "Delete Channel")
								.unwrap()
								.1
								.center(),
							PointerButton::Primary,
						);
						assert_eq!(h.commands.len(), 1);
						assert!(matches!(
							&h.commands[0],
							Command::ChannelAction {
								channel: Id(20),
								action: Action::Delete,
								..
							}
						));
						h.state.generation += 1;
						h.frame(&ctx, vec![]);
						assert!(h.menu.dialog.is_none());
					}
				}
				// A fresh menu with unknown permissions never exposes administrative actions.
				egui::Popup::close_all(&ctx);
				h.state.permissions = Default::default();
				let (row, _) = h.frame(&ctx, vec![]);
				h.click(&ctx, row.rect.center(), PointerButton::Secondary);
				let (_, text) = h.frame(&ctx, vec![]);
				for hidden in [
					"Invite to Channel",
					"Edit Channel",
					"Duplicate Channel",
					"Create Channel",
					"Delete Channel",
				] {
					assert!(!text.iter().any(|(label, _)| label == hidden));
				}
			}
		}
	}
}
