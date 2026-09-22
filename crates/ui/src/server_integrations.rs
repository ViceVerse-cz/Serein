//! Permission-aware integration cards and a single bounded webhook draft.
use crate::{avatars::Avatars, design, dialog, icons};
use client_core::{Command, State};
use egui::{RichText, Vec2};
use model::{
	Id, server_admin,
	server_integrations::{Action, Integration, Snapshot, Webhook},
};

const HELP: &str =
	"https://support.discord.com/hc/en-us/articles/360045093012-Server-Integrations-Page";
const FOLLOW_HELP: &str =
	"https://support.discord.com/hc/en-us/articles/360028384531-Channel-Following-FAQ";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
	#[default]
	Overview,
	Webhooks,
	Follows,
	App(Id),
	Editor,
}
#[derive(Clone, PartialEq, Eq)]
struct Draft {
	id: Option<Id>,
	name: String,
	channel: Option<Id>,
}
struct Deletion {
	action: Action,
	name: String,
}
#[derive(Default)]
pub(super) struct IntegrationsUi {
	page: Page,
	channel: Option<Id>,
	draft: Option<Draft>,
	baseline: Option<Draft>,
	submitted: bool,
	delete: Option<Deletion>,
	deleting: bool,
	error: Option<&'static str>,
	copy_requested: Option<Id>,
	copied: Option<Id>,
}
impl IntegrationsUi {
	pub fn for_channel(channel: Id) -> Self {
		Self {
			channel: Some(channel),
			..Default::default()
		}
	}
	fn can_manage_webhooks(&self, state: &State, guild: Id) -> bool {
		self.channel.map_or_else(
			|| state.can_manage_guild_webhooks(guild),
			|channel| state.can_manage_webhook_channel(guild, channel),
		)
	}

	pub fn preview(&mut self, webhooks: bool) {
		self.page = if webhooks {
			Page::Webhooks
		} else {
			Page::Overview
		};
	}
	pub fn has_changes(&self) -> bool {
		self.draft != self.baseline || self.submitted
	}
	/// Overview and the webhook lists virtualize their own rows, so they own the page scroll.
	/// The editor form is short and scrolls with the settings content instead.
	pub fn scrolls_itself(&self) -> bool {
		matches!(self.page, Page::Overview | Page::Webhooks | Page::Follows)
	}
	pub fn overlay_open(&self) -> bool {
		self.delete.is_some()
	}
	pub fn sync(&mut self, state: &State, guild: Id) {
		if !self.channel.map_or_else(
			|| state.can_open_integration_settings(guild),
			|channel| state.can_manage_webhook_channel(guild, channel),
		) {
			*self = Self {
				channel: self.channel,
				..Default::default()
			};
			return;
		}
		if self.submitted && !state.server_admin.pending {
			self.submitted = false;
			if state.server_admin.error.is_none() {
				self.draft = None;
				self.baseline = None;
				self.page = Page::Webhooks;
			}
		}
		if self.deleting && !state.server_admin.pending {
			self.deleting = false;
			if state.server_admin.error.is_none() {
				if matches!(
					self.delete.as_ref().map(|d| &d.action),
					Some(Action::DeleteIntegration { .. })
				) {
					self.page = Page::Overview;
				}
				self.delete = None;
			}
		}
		if (!self.can_manage_webhooks(state, guild)
			&& matches!(self.page, Page::Webhooks | Page::Follows | Page::Editor))
			|| self
				.baseline
				.as_ref()
				.and_then(|draft| draft.channel)
				.is_some_and(|channel| !state.can_manage_webhook_channel(guild, channel))
		{
			self.page = Page::Overview;
			self.draft = None;
			self.baseline = None;
			self.delete = None;
		}
		if !state.can_manage_guild(guild) && matches!(self.page, Page::App(_)) {
			self.page = Page::Overview;
		}
		if self
			.delete
			.as_ref()
			.is_some_and(|d| !allowed(state, guild, &d.action))
		{
			self.delete = None;
		}
	}
	fn load_action(&self, state: &State, guild: Id) -> Action {
		Action::Load {
			channel: self.channel,
			integrations: self.channel.is_none() && state.can_manage_guild(guild),
			webhooks: self.can_manage_webhooks(state, guild),
		}
	}
	pub fn load(&mut self, state: &mut State, guild: Id) -> Option<Command> {
		if state.server_admin.pending || state.server_admin.error.is_some() {
			return None;
		}
		if state.server_admin.guild == Some(guild)
			&& state
				.server_admin
				.integrations
				.as_ref()
				.is_some_and(|snapshot| {
					snapshot.channel == self.channel
						&& (self.channel.is_some()
							|| !state.can_manage_guild(guild)
							|| snapshot.integrations.is_some())
						&& (!self.can_manage_webhooks(state, guild) || snapshot.webhooks.is_some())
				}) {
			return None;
		}
		state.request_server_admin(
			guild,
			server_admin::Action::Integrations(self.load_action(state, guild)),
		)
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		let previous_page = self.page;
		let mut action = None;
		ui.set_max_width(ui.available_width().min(720.0));
		if self.page != Page::Overview {
			ui.horizontal(|ui| {
				if ui
					.add_enabled(
						!self.has_changes(),
						egui::Button::new("< Integrations").frame(false),
					)
					.clicked()
				{
					self.page = Page::Overview;
					self.draft = None;
					self.baseline = None;
				}
			});
			ui.add_space(12.0);
		}
		ui.horizontal(|ui| {
			ui.label(design::semibold(
				ui,
				match self.page {
					Page::Overview => "Integrations",
					Page::Webhooks => "Webhooks",
					Page::Follows => "Channels Followed",
					Page::App(_) => "Manage Integration",
					Page::Editor => {
						if self.draft.as_ref().is_some_and(|d| d.id.is_some()) {
							"Edit Webhook"
						} else {
							"Create Webhook"
						}
					}
				},
				20.0,
			));
			if ui
				.add_enabled(
					!state.server_admin.pending && !self.submitted && !self.deleting,
					egui::Button::new("Reload").frame(false),
				)
				.on_hover_text("Reload integrations")
				.clicked()
			{
				action = Some(self.load_action(state, guild));
			}
		});
		ui.add_space(12.0);
		if let Some(error) = state.server_admin.error.or(self.error) {
			design::notice(ui, design::Level::Error, error);
		}
		if state.server_admin.needs_refresh {
			ui.weak("Reload integrations before making more changes. Your draft will be kept.");
		}
		if state.server_admin.pending {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.weak(if state.server_admin.saving {
					"Updating integrations..."
				} else {
					"Loading integrations..."
				});
			});
		}
		if self.page == Page::Editor {
			ui.add_enabled_ui(!state.server_admin.pending, |ui| {
				self.editor(ui, state, guild, avatars, &mut action);
			});
		} else if let Some(snapshot) = &state.server_admin.integrations
			&& snapshot.guild == guild
			&& snapshot.channel == self.channel
		{
			match self.page {
				Page::Overview => self.overview(ui, state, guild, snapshot, avatars),
				Page::Webhooks | Page::Follows => {
					self.webhooks(ui, state, guild, snapshot, avatars)
				}
				Page::App(id) => self.app(ui, state, guild, snapshot, id, avatars),
				Page::Editor => {}
			}
		}
		if self.page != previous_page {
			state.clear_webhook_url();
			self.copied = None;
		} else if let Some(url) = state.take_webhook_url(guild, self.channel) {
			self.copied = Some(url.webhook);
			ui.ctx().copy_text(url.expose().to_owned());
		}
		if let Some(webhook) = self.copy_requested.take()
			&& let Some(channel) = state
				.server_admin
				.integrations
				.as_ref()
				.and_then(|s| s.webhooks.as_ref())
				.and_then(|items| items.iter().find(|w| w.id == webhook))
				.and_then(|w| w.channel)
		{
			self.copied = None;
			action = Some(Action::CopyWebhookUrl {
				scope: self.channel,
				webhook,
				channel,
			});
		}
		if let Some(action) = action {
			let saving = action.write();
			if let Some(command) =
				state.request_server_admin(guild, server_admin::Action::Integrations(action))
			{
				commands.push(command);
				self.submitted = saving;
				self.error = None;
			} else {
				self.error = Some(
					"Could not update integrations. Check your permissions and connection, then reload.",
				);
			}
		}
	}
	fn overview(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		snapshot: &Snapshot,
		avatars: &mut Avatars,
	) {
		ui.label(if self.channel.is_some() {
			"Manage webhooks and followed channels posting to this channel."
		} else {
			"Customize your server with integrations. Manage webhooks, followed channels, and apps connected to your server."
		});
		ui.hyperlink_to("Learn more about managing integrations.", HELP);
		design::divider(ui);
		if self.can_manage_webhooks(state, guild)
			&& let Some(webhooks) = &snapshot.webhooks
		{
			let followed = webhooks.iter().filter(|w| w.kind == 2).count();
			if summary_card(
				ui,
				icons::Icon::Link,
				"Webhooks",
				&format!("{} webhooks", webhooks.len() - followed),
			) {
				self.page = Page::Webhooks;
			}
			ui.add_space(8.0);
			if summary_card(
				ui,
				icons::Icon::Threads,
				"Channels Followed",
				&format!("{followed} channel{}", if followed == 1 { "" } else { "s" }),
			) {
				self.page = Page::Follows;
			}
			design::divider(ui);
		}
		if self.channel.is_none()
			&& state.can_manage_guild(guild)
			&& let Some(integrations) = &snapshot.integrations
		{
			ui.label(design::medium(ui, "Bots and Apps", 15.0));
			ui.add_space(12.0);
			if integrations.is_empty() {
				ui.weak("No integrations in this server.");
			}
			if integrations.len() == model::server_integrations::MAX_INTEGRATIONS {
				ui.weak("Showing the first 50 integrations returned by Discord.");
			}
			let height = design::list_height(ui, 0.0);
			egui::ScrollArea::vertical()
				.id_salt("integration-apps")
				.max_height(height)
				.auto_shrink([false, true])
				.show_rows(ui, 112.0, integrations.len(), |ui, range| {
					for integration in &integrations[range] {
						let width = ui.available_width();
						let (rect, _) =
							ui.allocate_exact_size(Vec2::new(width, 112.0), egui::Sense::hover());
						ui.painter().rect_filled(
							rect.shrink2(Vec2::new(0.0, 6.0)),
							8,
							design::palette(ui).raised,
						);
						card_contents(
							ui,
							egui::UiBuilder::new()
								.id_salt(("integration-app", integration.id))
								.max_rect(rect.shrink2(Vec2::new(16.0, 18.0))),
							|ui| {
								ui.horizontal(|ui| {
									app_avatar(ui, integration, avatars, state.demo);
									let text_width = (ui.available_width() - 94.0).max(48.0);
									ui.allocate_ui_with_layout(
										Vec2::new(text_width, 76.0),
										egui::Layout::top_down(egui::Align::Min),
										|ui| {
											ui.set_width(text_width);
											ui.add(
												egui::Label::new(design::medium(
													ui,
													&integration.name,
													15.0,
												))
												.truncate(),
											)
											.on_hover_text(&integration.name);
											if let Some(user) = &integration.user {
												ui.add(
													egui::Label::new(
														RichText::new(format!(
															"Added by {}",
															user.name
														))
														.size(12.0),
													)
													.truncate(),
												);
											}
											ui.horizontal_wrapped(|ui| {
												chip(ui, service_name(integration));
												if let Some(app) = &integration.application
													&& let Some(webhooks) = &snapshot.webhooks
												{
													let count = webhooks
														.iter()
														.filter(|w| {
															w.application_id == Some(app.id)
														})
														.count();
													if count > 0 {
														chip(
															ui,
															&format!(
																"{count} webhook{}",
																if count == 1 { "" } else { "s" }
															),
														);
													}
												}
											});
										},
									);
									if ui.add(egui::Button::new("Manage >").frame(false)).clicked()
									{
										self.page = Page::App(integration.id);
									}
								});
							},
						);
					}
				});
		}
	}
	fn webhooks(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		snapshot: &Snapshot,
		avatars: &mut Avatars,
	) {
		let follows = self.page == Page::Follows;
		if follows {
			ui.label("Posts from these followed channels are delivered to your server.");
			ui.hyperlink_to("Learn more about following channels", FOLLOW_HELP);
		} else {
			ui.label("Send updates from your apps and services to a channel in this server.");
			if let Some(channel) = self.channel.and_then(|id| state.channel(id)) {
				ui.label(format!("Posting to #{}", channel.name));
			}
			ui.add_space(16.0);
			if let Some(channel) = state.channels.iter().find(|c| {
				c.guild == Some(guild)
					&& self.channel.is_none_or(|id| c.id == id)
					&& matches!(c.kind, 0 | 5 | 15 | 16)
					&& state.can_manage_webhook_channel(guild, c.id)
			}) && primary(ui, "New Webhook", writable(state)).clicked()
			{
				self.draft = Some(Draft {
					id: None,
					name: "Updates".into(),
					channel: Some(channel.id),
				});
				self.baseline = None;
				self.page = Page::Editor;
			}
		}
		ui.add_space(24.0);
		let Some(webhooks) = &snapshot.webhooks else {
			return;
		};
		let rows: Vec<_> = webhooks
			.iter()
			.filter(|w| (w.kind == 2) == follows)
			.collect();
		if rows.is_empty() {
			ui.weak(if follows {
				"No channels followed."
			} else {
				"No webhooks yet."
			});
		}
		let row_height = if ui.available_width() < 360.0 {
			148.0
		} else {
			120.0
		};
		egui::ScrollArea::vertical()
			.id_salt(("integration-webhooks", follows))
			.max_height(design::list_height(ui, 0.0))
			.auto_shrink([false, true])
			.show_rows(ui, row_height, rows.len(), |ui, range| {
				for webhook in &rows[range] {
					let (rect, _) = ui.allocate_exact_size(
						Vec2::new(ui.available_width(), row_height),
						egui::Sense::hover(),
					);
					ui.painter().rect_filled(
						rect.shrink2(Vec2::new(0.0, 4.0)),
						8,
						design::palette(ui).raised,
					);
					card_contents(
						ui,
						egui::UiBuilder::new()
							.id_salt(("integration-webhook", webhook.id))
							.max_rect(rect.shrink2(Vec2::new(16.0, 16.0))),
						|ui| {
							webhook_identity(ui, state, webhook, avatars);
							ui.add_space(10.0);
							ui.horizontal_wrapped(|ui| {
								self.copy_button(ui, state, guild, webhook);
								if webhook.kind == 1
									&& webhook.channel.is_some_and(|id| {
										state.can_manage_webhook_channel(guild, id)
									}) && ui
									.add_enabled(writable(state), egui::Button::new("Edit"))
									.clicked()
								{
									let draft = Draft {
										id: Some(webhook.id),
										name: webhook.name.clone().unwrap_or_default(),
										channel: webhook.channel,
									};
									self.baseline = Some(draft.clone());
									self.draft = Some(draft);
									self.page = Page::Editor;
								}
								let action = Action::DeleteWebhook {
									scope: self.channel,
									webhook: webhook.id,
								};
								if allowed(state, guild, &action)
									&& ui
										.add_enabled(
											writable(state),
											egui::Button::new(
												RichText::new(if follows {
													"Unfollow"
												} else {
													"Delete"
												})
												.color(design::palette(ui).danger),
											),
										)
										.clicked()
								{
									self.delete = Some(Deletion {
										action,
										name: webhook_name(webhook).to_owned(),
									});
								}
							});
						},
					);
				}
			});
	}
	fn copy_button(&mut self, ui: &mut egui::Ui, state: &State, guild: Id, webhook: &Webhook) {
		let Some(channel) = webhook.channel else {
			return;
		};
		let action = Action::CopyWebhookUrl {
			scope: self.channel,
			webhook: webhook.id,
			channel,
		};
		if webhook.kind == 1
			&& allowed(state, guild, &action)
			&& ui
				.add_enabled(
					writable(state),
					egui::Button::new(if self.copied == Some(webhook.id) {
						"Copied!"
					} else {
						"Copy Webhook URL"
					}),
				)
				.clicked()
		{
			self.copy_requested = Some(webhook.id);
		}
	}

	fn app(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		snapshot: &Snapshot,
		id: Id,
		avatars: &mut Avatars,
	) {
		let Some(integration) = snapshot
			.integrations
			.as_ref()
			.and_then(|items| items.iter().find(|i| i.id == id))
		else {
			ui.weak("This integration is no longer available.");
			return;
		};
		ui.horizontal(|ui| {
			app_avatar(ui, integration, avatars, state.demo);
			ui.label(design::semibold(ui, &integration.name, 20.0));
		});
		ui.add_space(16.0);
		if let Some(app) = &integration.application
			&& !app.description.is_empty()
		{
			ui.label(&app.description);
		}
		ui.label(format!("Service: {}", service_name(integration)));
		ui.label(if integration.enabled {
			"Enabled"
		} else {
			"Disabled"
		});
		if let Some(user) = &integration.user {
			ui.horizontal(|ui| {
				avatars.show(ui, user, 24.0, state.demo);
				ui.label(format!("Added by {}", user.name));
			});
		}
		design::divider(ui);
		if let Some(app) = &integration.application
			&& let Some(webhooks) = &snapshot.webhooks
		{
			let count = webhooks
				.iter()
				.filter(|w| w.application_id == Some(app.id))
				.count();
			if count > 0
				&& summary_card(
					ui,
					icons::Icon::Link,
					"Webhooks",
					&format!("{count} linked webhooks"),
				) {
				self.page = Page::Webhooks;
			}
		}
		let action = Action::DeleteIntegration { integration: id };
		if allowed(state, guild, &action) {
			ui.add_space(24.0);
			if ui
				.add_enabled(
					writable(state),
					egui::Button::new(
						RichText::new("Remove Integration").color(design::palette(ui).danger),
					),
				)
				.clicked()
			{
				self.delete = Some(Deletion {
					action,
					name: integration.name.clone(),
				});
			}
		}
	}
	fn editor(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		avatars: &mut Avatars,
		action: &mut Option<Action>,
	) {
		if let Some(webhook) = self.draft.as_ref().and_then(|d| d.id).and_then(|id| {
			state
				.server_admin
				.integrations
				.as_ref()?
				.webhooks
				.as_ref()?
				.iter()
				.find(|w| w.id == id)
		}) {
			webhook_identity(ui, state, webhook, avatars);
			ui.add_space(12.0);
			self.copy_button(ui, state, guild, webhook);
			design::divider(ui);
		}
		let Some(draft) = &mut self.draft else {
			return;
		};
		ui.add_space(12.0);
		let label = design::label(ui, "Name");
		design::input(
			ui,
			egui::TextEdit::singleline(&mut draft.name).char_limit(80),
		)
		.labelled_by(label.id);
		if draft.name.capacity() > 320 {
			draft.name.shrink_to_fit();
		}
		if !model::server_integrations::valid_webhook_name(&draft.name) {
			design::notice(
				ui,
				design::Level::Error,
				"Use 1–80 characters without control characters or the reserved names Discord and Clyde.",
			);
		}
		ui.add_space(16.0);
		design::label(ui, "Channel");
		let name = draft
			.channel
			.and_then(|id| state.channel(id))
			.map_or("Choose a channel", |c| c.name.as_str());
		egui::ComboBox::from_id_salt("webhook-destination")
			.selected_text(name)
			.width(ui.available_width())
			.show_ui(ui, |ui| {
				for channel in state.channels.iter().filter(|c| {
					c.guild == Some(guild)
						&& matches!(c.kind, 0 | 5 | 15 | 16)
						&& (draft.id.is_some() || self.channel.is_none_or(|id| c.id == id))
						&& state.can_manage_webhook_channel(guild, c.id)
				}) {
					ui.selectable_value(
						&mut draft.channel,
						Some(channel.id),
						format!("#{}", channel.name),
					);
				}
			});
		ui.add_space(24.0);
		let save = draft.channel.map(|channel| match draft.id {
			Some(webhook) => Action::EditWebhook {
				scope: self.channel,
				webhook,
				channel,
				name: draft.name.clone(),
			},
			None => Action::CreateWebhook {
				scope: self.channel,
				channel,
				name: draft.name.clone(),
			},
		});
		let valid = save.as_ref().is_some_and(|a| allowed(state, guild, a));
		ui.horizontal(|ui| {
			if primary(
				ui,
				if self.submitted {
					"Saving..."
				} else {
					"Save Changes"
				},
				valid && writable(state) && self.draft != self.baseline,
			)
			.clicked()
			{
				*action = save;
			}
			if ui
				.add_enabled(
					!self.submitted,
					egui::Button::new(if self.baseline.is_some() {
						"Reset"
					} else {
						"Cancel"
					})
					.frame(false),
				)
				.clicked()
			{
				self.draft.clone_from(&self.baseline);
				if self.draft.is_none() {
					self.page = Page::Webhooks;
				}
			}
		});
	}
	pub fn overlays(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) {
		let Some(deletion) = &self.delete else {
			return;
		};
		if !allowed(state, guild, &deletion.action) {
			self.delete = None;
			return;
		}
		let integration = matches!(deletion.action, Action::DeleteIntegration { .. });
		let mut confirm = dialog::Confirm::new(
			"delete-server-integration",
			if integration {
				"Remove integration?"
			} else {
				"Delete webhook?"
			},
			if integration {
				format!(
					"Removing {} also removes its bot and every webhook it owns from this server.",
					deletion.name
				)
			} else {
				format!(
					"{} will stop delivering messages. A followed channel will also stop sending posts to this server.",
					deletion.name
				)
			},
		)
		.danger()
		.confirm_label(if integration { "Remove" } else { "Delete" })
		.enabled(writable(state));
		if let Some(error) = state.server_admin.error {
			confirm = confirm.note(dialog::Level::Error, error);
		}
		let choice = confirm.show(ctx);
		if choice == Some(dialog::Choice::Confirmed)
			&& let Some(command) = state.request_server_admin(
				guild,
				server_admin::Action::Integrations(deletion.action.clone()),
			) {
			commands.push(command);
			self.deleting = true;
		}
		if choice == Some(dialog::Choice::Cancelled) && !self.deleting {
			self.delete = None;
		}
	}
}
fn allowed(state: &State, guild: Id, action: &Action) -> bool {
	state.server_admin_action_allowed(guild, &server_admin::Action::Integrations(action.clone()))
}
fn writable(state: &State) -> bool {
	!state.server_admin.pending
		&& !state.server_admin.needs_refresh
		&& !state.server_settings.saving
		&& (state.demo || state.gateway_connected)
}
fn webhook_name(webhook: &Webhook) -> &str {
	webhook
		.source_channel
		.as_ref()
		.and_then(|s| s.name.as_deref())
		.or(webhook.name.as_deref())
		.unwrap_or("Webhook")
}
fn webhook_identity(ui: &mut egui::Ui, state: &State, webhook: &Webhook, avatars: &mut Avatars) {
	ui.horizontal(|ui| {
		let user = model::User {
			kind: model::AccountKind::Bot,
			webhook: true,
			id: webhook.id,
			name: webhook_name(webhook).to_owned(),
			avatar: webhook.avatar.clone(),
			discriminator: 0,
			primary_guild: None,
		};
		avatars.show_plain(ui, &user, 40.0, state.demo);
		ui.vertical(|ui| {
			ui.add(egui::Label::new(design::semibold(ui, webhook_name(webhook), 15.0)).truncate())
				.on_hover_text(webhook_name(webhook));
			let destination = webhook
				.channel
				.and_then(|id| state.channel(id))
				.map_or("Unknown channel", |c| c.name.as_str());
			ui.add(
				egui::Label::new(
					RichText::new(format!("#{destination}"))
						.size(12.0)
						.color(design::palette(ui).muted),
				)
				.truncate(),
			);
		});
	});
}
fn primary(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
	ui.add_enabled_ui(enabled, |ui| {
		design::button(ui, text, design::ButtonKind::Primary)
	})
	.inner
}
fn app_avatar(ui: &mut egui::Ui, integration: &Integration, avatars: &mut Avatars, demo: bool) {
	if let Some(bot) = integration
		.application
		.as_ref()
		.and_then(|a| a.bot.as_ref())
	{
		avatars.show(ui, bot, 44.0, demo);
	} else {
		icons::inline(ui, icons::Icon::Activities, 44.0, design::palette(ui).muted);
	}
}
fn chip(ui: &mut egui::Ui, text: &str) {
	ui.add(
		egui::Button::new(RichText::new(text).size(12.0))
			.sense(egui::Sense::hover())
			.corner_radius(3),
	);
}
// The card already reserves its full row; child contents must not move the parent cursor.
fn card_contents(
	ui: &mut egui::Ui,
	builder: egui::UiBuilder,
	contents: impl FnOnce(&mut egui::Ui),
) {
	let mut child = ui.new_child(builder);
	contents(&mut child);
}
fn summary_card(ui: &mut egui::Ui, glyph: icons::Icon, name: &str, subtitle: &str) -> bool {
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(Vec2::new(ui.available_width(), 88.0), egui::Sense::click());
	let frame = design::interactive_card_frame(ui, &response);
	ui.painter().add(frame.paint(rect));
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
	icons::paint(
		ui.painter(),
		glyph,
		egui::Rect::from_center_size(
			egui::pos2(rect.left() + 36.0, rect.center().y),
			Vec2::splat(26.0),
		),
		colors.muted,
	);
	let text_rect = egui::Rect::from_min_max(
		rect.min + Vec2::new(72.0, 22.0),
		rect.max - Vec2::new(40.0, 12.0),
	);
	card_contents(
		ui,
		egui::UiBuilder::new()
			.id_salt(("integration-summary", name))
			.max_rect(text_rect),
		|ui| {
			ui.add(
				egui::Label::new(design::medium(ui, name, 15.0))
					.truncate()
					.selectable(false),
			);
			ui.add(
				egui::Label::new(RichText::new(subtitle).size(12.0))
					.truncate()
					.selectable(false),
			);
		},
	);
	icons::paint(
		ui.painter(),
		icons::Icon::ChevronRight,
		egui::Rect::from_center_size(
			egui::pos2(rect.right() - 22.0, rect.center().y),
			Vec2::splat(16.0),
		),
		colors.muted,
	);
	response.clicked()
}

fn service_name(integration: &Integration) -> &str {
	match integration.kind.as_str() {
		"discord"
			if integration
				.application
				.as_ref()
				.and_then(|a| a.bot.as_ref())
				.is_some() =>
		{
			"Bot"
		}
		"discord" => "App",
		"twitch" => "Twitch",
		"youtube" => "YouTube",
		_ => &integration.kind,
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	fn state() -> State {
		let mut state = test_support::demo_state();
		let guild = state.guilds[0].id;
		state.permissions.guilds.insert(
			guild,
			model::permissions::Guild {
				id: guild,
				owner: state.user.as_ref().map(|user| user.id),
				roles: Some(vec![]),
				member: Some(model::permissions::Member {
					roles: vec![],
					timeout_until: None,
				}),
			},
		);
		state.permissions.clear_cache();
		state
	}
	#[test]
	fn webhook_cards_copy_once_in_both_settings_scopes_and_fit_small_widths() {
		fn labels(shape: &egui::epaint::Shape, found: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::epaint::Shape::Text(text) => found.push((
					text.galley.text().to_owned(),
					egui::Rect::from_min_size(text.pos, text.galley.size()),
				)),
				egui::epaint::Shape::Vec(shapes) => {
					for shape in shapes {
						labels(shape, found);
					}
				}
				_ => {}
			}
		}
		for width in [280.0, 360.0, 800.0] {
			for scoped in [false, true] {
				let ctx = egui::Context::default();
				design::apply(&ctx);
				let mut state = state();
				let guild = state.guilds[0].id;
				let channel = state
					.channels
					.iter()
					.find(|c| c.guild == Some(guild) && c.kind == 0)
					.unwrap()
					.id;
				let scope = scoped.then_some(channel);
				state.server_admin.guild = Some(guild);
				state.server_admin.integrations = Some(Snapshot {
					guild,
					channel: scope,
					integrations: None,
					webhooks: Some(vec![Webhook {
						id: Id(900),
						guild,
						channel: Some(channel),
						kind: 1,
						name: Some("A long synthetic webhook name ".repeat(2)),
						avatar: None,
						application_id: None,
						user: None,
						source_guild: None,
						source_channel: None,
					}]),
				});
				let mut view = IntegrationsUi {
					page: Page::Webhooks,
					channel: scope,
					..Default::default()
				};
				let mut commands = vec![];
				let mut avatars = Avatars::default();
				{
					let mut render = |state: &mut State, events| {
						ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									Vec2::new(width, 900.0),
								)),
								events,
								..Default::default()
							},
							|ui| {
								view.show(ui, state, guild, &mut avatars, &mut commands);
								assert!(
									ui.min_rect().right() <= width + 1.0,
									"overflow at {width}: {:?}",
									ui.min_rect()
								);
							},
						)
					};
					let output = render(&mut state, vec![]);
					let mut text = vec![];
					for shape in &output.shapes {
						labels(&shape.shape, &mut text);
					}
					let copy = text
						.iter()
						.find(|(text, _)| text == "Copy Webhook URL")
						.expect("visible copy button")
						.1;
					assert!(copy.right() <= width);
					output.drop_without_applying_deltas();
					for pressed in [true, false] {
						render(
							&mut state,
							vec![
								egui::Event::PointerMoved(copy.center()),
								egui::Event::PointerButton {
									pos: copy.center(),
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE,
								},
							],
						)
						.drop_without_applying_deltas();
					}
				}
				let Some(Command::ServerAdmin {
					request, action, ..
				}) = commands.pop()
				else {
					panic!("copy command missing")
				};
				assert!(
					matches!(*action, server_admin::Action::Integrations(Action::CopyWebhookUrl { scope: actual, .. }) if actual == scope)
				);
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::ServerAdmin(client_core::server_admin::Event {
						guild,
						request,
						result: Ok(server_admin::Result::WebhookUrl(
							model::server_integrations::WebhookUrl::new(
								guild,
								Id(900),
								channel,
								"SYNTHETIC_TOKEN",
							)
							.unwrap(),
						)),
					}),
				});
				for expected in [1, 0] {
					let output = ctx.run_ui(egui::RawInput::default(), |ui| {
						view.show(ui, &mut state, guild, &mut avatars, &mut commands)
					});
					assert_eq!(output.platform_output.commands.iter().filter(|c| matches!(c, egui::OutputCommand::CopyText(text) if text == "https://discord.com/api/webhooks/900/SYNTHETIC_TOKEN")).count(), expected);
					output.drop_without_applying_deltas();
				}
			}
		}
	}

	#[test]
	fn draft_survives_failed_save_and_refresh_but_is_cleared_on_permission_loss() {
		let mut state = state();
		let guild = state.guilds[0].id;
		let draft = Draft {
			id: None,
			name: "Updates".into(),
			channel: None,
		};
		let mut view = IntegrationsUi {
			page: Page::Editor,
			draft: Some(draft.clone()),
			submitted: true,
			..Default::default()
		};
		state.server_admin.guild = Some(guild);
		state.server_admin.error = Some("Synthetic request failure");
		view.sync(&state, guild);
		assert!(view.has_changes());
		assert!(view.draft.as_ref() == Some(&draft));
		state.server_admin.error = None;
		view.sync(&state, guild);
		assert!(view.draft.as_ref() == Some(&draft));
		state.guilds.clear();
		view.sync(&state, guild);
		assert!(!view.has_changes());
		assert!(view.draft.is_none());
	}
	#[test]
	fn integration_cards_fit_narrow_and_wide_layouts() {
		for width in [280.0, 360.0, 800.0] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let state = state();
			let guild = state.guilds[0].id;
			let snapshot = Snapshot {
				channel: None,
				guild,
				webhooks: Some(vec![]),
				integrations: Some(vec![Integration {
					id: Id(900),
					name: "A very long synthetic application name ".repeat(4),
					kind: "discord".into(),
					enabled: true,
					user: None,
					synced_at: None,
					role_id: None,
					application: None,
				}]),
			};
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						Vec2::new(width, 1200.0),
					)),
					..Default::default()
				},
				|ui| {
					ui.set_width(width);
					ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
					let right = ui.max_rect().right();
					IntegrationsUi::default().overview(
						ui,
						&state,
						guild,
						&snapshot,
						&mut Avatars::default(),
					);
					assert!(
						ui.min_rect().right() <= right + 1.0,
						"integration cards overflow at {width}: {:?}",
						ui.min_rect()
					);
				},
			);
			output.drop_without_applying_deltas();
		}
	}
	#[test]
	fn summary_cards_preserve_their_reserved_row_height() {
		let ctx = egui::Context::default();
		let output = ctx.run_ui(egui::RawInput::default(), |ui| {
			let before = ui.next_widget_position().y;
			summary_card(ui, icons::Icon::Link, "Webhooks", "2 webhooks");
			assert!(ui.next_widget_position().y >= before + 88.0);
			summary_card(ui, icons::Icon::Threads, "Channels Followed", "0 channels");
			assert!(ui.next_widget_position().y >= before + 176.0);
		});
		output.drop_without_applying_deltas();
	}
}
