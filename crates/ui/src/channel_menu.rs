//! Guild channel actions share one context menu and a session-scoped editor.
use crate::{design, dialog, icons, user_menu};
use client_core::{
	Command, State,
	channel_actions::{Action, Edit, Mute},
};
use model::{Channel, ChannelPreferences, Id};

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

struct Dialog {
	channel: Id,
	guild: Id,
	kind: Kind,
	draft: Edit,
	before: Edit,
	loaded: bool,
	submitted: bool,
	permissions_page: bool,
	permissions: crate::channel_permissions::PermissionsUi,
}

#[derive(Default)]
pub(super) struct ChannelMenu {
	pub invite_requested: Option<(Id, Id)>,
	requested: Option<(Id, Intent)>,
	dialog: Option<Dialog>,
	feedback: Option<Id>,
	preference_error: bool,
	generation: u64,
}

impl ChannelMenu {
	pub fn context(
		&mut self,
		response: &egui::Response,
		state: &State,
		channel: &Channel,
		prefs: &mut ChannelPreferences,
		prefs_changed: &mut bool,
		preferences_available: bool,
	) {
		let Some(guild) = channel.guild else { return };
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
			let favorite = prefs.is_favorite(channel.id);
			if channel.kind != 4
				&& row(
					ui,
					if favorite {
						"Remove From Favorites"
					} else {
						"Add To Favorites"
					},
					preferences_available,
					false,
				)
				.on_hover_text("Favorites are saved on this device.")
				.clicked()
			{
				let changed = prefs.toggle_favorite(channel.id);
				*prefs_changed |= changed;
				self.preference_error = !changed;
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
			let pinned = prefs.is_pinned(channel.id);
			if channel.kind != 4
				&& row(
					ui,
					if pinned {
						"Unpin Channel From Top"
					} else {
						"Pin Channel to Top"
					},
					preferences_available,
					false,
				)
				.on_hover_text("Pinned channels are saved on this device.")
				.clicked()
			{
				let changed = prefs.toggle_pinned(channel.id);
				*prefs_changed |= changed;
				self.preference_error = !changed;
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
					("Create Text Channel", Kind::Create),
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
		commands: &mut Vec<Command>,
	) {
		if self.generation != state.generation {
			*self = Self::default();
			return;
		}
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
					self.feedback = Some(id);
					if let Some(command) = state.request_channel_action(id, action) {
						commands.push(command);
					}
				}
				Intent::Dialog(kind) => {
					self.dialog = Some(Dialog {
						channel: id,
						guild,
						kind,
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
						},
						loaded: kind != Kind::Edit,
						before: Edit::default(),
						submitted: false,
						permissions_page: false,
						permissions: Default::default(),
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
		self.show_feedback(ctx, state, active);
		let Some(dialog) = &mut self.dialog else {
			return;
		};
		if active != Some(dialog.guild)
			|| state.channel(dialog.channel).is_none()
			|| (dialog.submitted && state.channel_action_succeeded(dialog.channel))
		{
			self.dialog = None;
			return;
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
			Kind::Create => (
				"Create Text Channel",
				"Text channels are where your members talk.",
			),
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
							.add_enabled(allowed, egui::Button::new("Retry"))
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
						delete_requested = dialog.editor(ui, state);
					});
				} else if let Some(channel) = state.channel(dialog.channel) {
					ui.add_enabled_ui(allowed && !pending_now, |ui| dialog.overview(ui, channel));
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
						.add_enabled(allowed && !pending_now, egui::Button::new("Reload Channel"))
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
				ui.add_enabled_ui(
					allowed
						&& dialog.loaded && current
						&& valid && !pending_now
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
								Kind::Create => Action::CreateText {
									name: dialog.draft.name.clone(),
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
				close |= dialog::action(
					ui,
					if pending_now { "Close" } else { "Cancel" },
					dialog::Action::Neutral,
				)
				.clicked();
			});
		});
		if delete_requested {
			self.requested = Some((dialog.channel, Intent::Dialog(Kind::Delete)));
			self.dialog = None;
		} else if close || response.close {
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
			message.push_str("Your favorites and pins are full. Remove one before adding another.");
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
		let label = dialog::label(
			ui,
			if channel.kind == 4 || self.kind == Kind::CreateCategory {
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
			ui.add(
				egui::DragValue::new(&mut self.draft.slowmode)
					.range(0..=21600)
					.suffix(" seconds"),
			);
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
	fn navigation(&mut self, ui: &mut egui::Ui, channel: &Channel, can_delete: bool) -> bool {
		ui.add(
			egui::Label::new(design::eyebrow(
				ui,
				&channel.name,
				design::palette(ui).muted,
			))
			.truncate(),
		);
		ui.add_space(12.0);
		if crate::settings::nav_item(ui, "Overview", !self.permissions_page).clicked() {
			self.permissions_page = false;
		}
		if crate::settings::nav_item(ui, "Permissions", self.permissions_page).clicked() {
			self.permissions_page = true;
		}
		ui.separator();
		row(
			ui,
			if channel.kind == 4 {
				"Delete Category"
			} else {
				"Delete Channel"
			},
			can_delete,
			true,
		)
		.clicked()
	}
	fn editor(&mut self, ui: &mut egui::Ui, state: &State) -> bool {
		let Some(channel) = state.channel(self.channel) else {
			return false;
		};
		let mut delete = false;
		let content = |this: &mut Self, ui: &mut egui::Ui| {
			if this.permissions_page {
				this.permissions
					.show(ui, state, channel, &mut this.draft.overwrites);
			} else {
				design::section(ui, "Overview", None);
				ui.add_enabled_ui(state.can_manage_channel(channel.id), |ui| {
					this.overview(ui, channel)
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
						delete = self.navigation(ui, channel, state.can_manage_channel(channel.id));
					},
				);
				ui.add_space(20.0);
				ui.vertical(|ui| content(self, ui));
			});
		} else {
			delete = self.navigation(ui, channel, state.can_manage_channel(channel.id));
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
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *value, label)
	});
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

fn row(ui: &mut egui::Ui, label: &str, enabled: bool, danger: bool) -> egui::Response {
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

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{Event, Modifiers, PointerButton, Pos2, Rect};

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
		changed: bool,
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
						&mut self.prefs,
						&mut self.changed,
						true,
					);
					row = Some(response);
					self.menu
						.show(ui.ctx(), &mut self.state, Some(Id(10)), &mut self.commands);
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
				changed: false,
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
					changed: false,
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
					"Pin Channel to Top",
					"Copy Link",
					"Mute Channel",
					"Notification Settings",
					"Edit Channel",
					"Duplicate Channel",
					"Create Text Channel",
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
					"Add To Favorites" => assert!(h.prefs.is_favorite(Id(20)) && h.changed),
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
					"Create Text Channel",
					"Delete Channel",
				] {
					assert!(!text.iter().any(|(label, _)| label == hidden));
				}
			}
		}
	}
}
