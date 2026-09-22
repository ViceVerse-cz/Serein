//! Server invite picker and link settings share one bounded, session-scoped dialog.
use crate::{avatars::Avatars, design, icons};
use client_core::{
	Command, State,
	server_actions::{InviteOptions, InviteStatus},
};
use model::Id;

#[derive(Default)]
pub(super) struct InviteDialog {
	search: String,
	settings: Option<InviteOptions>,
	options: InviteOptions,
	generate: bool,
	pub(super) copied: bool,
}

impl InviteDialog {
	pub fn open(&mut self) {
		*self = Self {
			generate: true,
			..Self::default()
		};
	}
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		target: (Id, &str),
		channel: &mut Option<Id>,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) -> bool {
		let (guild, name) = target;
		if channel.is_none_or(|id| !state.can_create_server_invite(guild, id)) {
			*channel = state.invite_channel(guild);
		}
		// This flag is set only by the user's Create invite menu choice, never by a repaint.
		if std::mem::take(&mut self.generate)
			&& let Some(channel) = *channel
			&& let Some(command) =
				state.create_server_invite_with_options(guild, channel, self.options)
		{
			commands.push(command);
		}
		let mut close = false;
		let settings_open = self.settings.is_some();
		let title = if settings_open {
			"Invite Link Settings".to_owned()
		} else {
			format!("Invite friends to {name}")
		};
		let response = crate::dialog::Dialog::new("server-invite-dialog", title)
			.subtitle(if settings_open {
				"Control how long this link lasts and how many people can use it."
			} else {
				"Share a link so friends can join this server."
			})
			.width(540.0)
			.show(ctx, |d| {
				if settings_open {
					d.scroll(220.0, |ui| {
						self.settings(ui, state, guild, *channel, commands)
					});
				} else {
					d.content(|ui| self.picker(ui, state, guild, channel, avatars, commands));
				}
			});
		close |= response.close;
		close && self.settings.take().is_none()
	}
	fn picker(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		channel: &mut Option<Id>,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		ui.add_space(6.0);
		let before = *channel;
		ui.add_enabled_ui(
			!state.server_action_pending() && !state.server_invite_pending(),
			|ui| {
				let layout = if ui.available_width() < 420.0 {
					egui::Layout::top_down(egui::Align::Min)
				} else {
					egui::Layout::left_to_right(egui::Align::Center)
				};
				ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 0.0), layout, |ui| {
					ui.label(
						egui::RichText::new("Recipients will land in")
							.size(18.0)
							.color(colors.muted),
					);
					let name = state
						.channels
						.iter()
						.find(|c| Some(c.id) == *channel)
						.map_or("No eligible channel", |c| c.name.as_str());
					egui::ComboBox::from_id_salt("invite-channel")
						.selected_text(
							egui::RichText::new(format!("# {name}"))
								.color(colors.muted)
								.size(18.0),
						)
						.width(ui.available_width().min(300.0))
						.wrap_mode(egui::TextWrapMode::Truncate)
						.height(220.0)
						.show_ui(ui, |ui| {
							for c in state
								.channels
								.iter()
								.filter(|c| state.can_create_server_invite(guild, c.id))
							{
								ui.selectable_value(channel, Some(c.id), &c.name);
							}
						});
				});
			},
		);
		if *channel != before {
			state.clear_server_action_result(guild);
			self.copied = false;
			self.generate = true;
		}
		ui.add_space(24.0);
		egui::Frame::new()
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(10)
			.inner_margin(egui::Margin::symmetric(12, 8))
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					let (rect, _) =
						ui.allocate_exact_size(egui::vec2(20.0, 22.0), egui::Sense::hover());
					icons::paint(ui.painter(), icons::Icon::Search, rect, colors.muted);
					ui.add(
						egui::TextEdit::singleline(&mut self.search)
							.hint_text("Search for friends")
							.char_limit(100)
							.font(egui::FontId::proportional(18.0))
							.align(egui::Align2::LEFT_CENTER)
							.desired_width(ui.available_width())
							.frame(egui::Frame::NONE),
					);
				});
			});
		ui.add_space(12.0);
		let query = self.search.to_lowercase();
		let friends: Vec<_> = state
			.friends()
			.filter(|user| {
				user.name.to_lowercase().contains(&query)
					|| state
						.friend_username(user.id)
						.is_some_and(|name| name.to_lowercase().contains(&query))
			})
			.collect();
		let list_height =
			(ui.ctx().content_rect().height() - (ui.cursor().top() - ui.min_rect().top()) - 380.0)
				.clamp(40.0, 240.0);
		let enabled = state.created_invite(guild).is_some()
			&& !state.server_invite_pending()
			&& !state.server_action_pending();
		let mut send = None;
		let spacing = ui.spacing().item_spacing.y;
		ui.spacing_mut().item_spacing.y = 0.0;
		egui::ScrollArea::vertical()
			.id_salt("invite-friends")
			.max_height(list_height)
			.min_scrolled_height(list_height)
			.auto_shrink([false, false])
			.show_rows(ui, 60.0, friends.len(), |ui, range| {
				ui.spacing_mut().item_spacing.y = 0.0;
				for user in &friends[range] {
					ui.push_id((guild, user.id), |ui| {
						let (rect, _) = ui.allocate_exact_size(
							egui::vec2(ui.available_width(), 60.0),
							egui::Sense::hover(),
						);
						if ui.rect_contains_pointer(rect) {
							ui.painter().rect_filled(rect, 6, colors.hover);
						}
						ui.scope_builder(
							egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(0.0, 10.0))),
							|ui| {
								ui.horizontal(|ui| {
									avatars.show(ui, user, 40.0, state.demo);
									ui.add_space(4.0);
									let label_width = (ui.available_width() - 90.0).max(1.0);
									ui.allocate_ui_with_layout(
										egui::vec2(label_width, 40.0),
										egui::Layout::top_down(egui::Align::Min),
										|ui| {
											ui.set_width(label_width);
											ui.add(
												egui::Label::new(design::semibold(
													ui, &user.name, 18.0,
												))
												.truncate(),
											);
											let status = state.server_invite_status(guild, user.id);
											let subtitle =
												if let Some(InviteStatus::Failed(failure)) = status
												{
													failure.label()
												} else {
													state.friend_username(user.id).unwrap_or("")
												};
											ui.add(
												egui::Label::new(
													egui::RichText::new(subtitle).size(12.0).color(
														if matches!(
															status,
															Some(InviteStatus::Failed(_))
														) {
															colors.warning
														} else {
															colors.muted
														},
													),
												)
												.truncate(),
											);
										},
									);
									let status = state.server_invite_status(guild, user.id);
									let label = match status {
										Some(InviteStatus::Sending) => "Sending…",
										Some(InviteStatus::Sent) => "Sent",
										Some(InviteStatus::Failed(
											client_core::auth::Failure::Ambiguous,
										)) => "Uncertain",
										Some(InviteStatus::Failed(_)) => "Retry",
										None => "Invite",
									};
									let can_send = enabled
										&& !matches!(
											status,
											Some(
												InviteStatus::Sent
													| InviteStatus::Sending | InviteStatus::Failed(
													client_core::auth::Failure::Ambiguous
												)
											)
										);
									let button = ui
										.add_enabled_ui(can_send, |ui| {
											design::button(ui, label, design::ButtonKind::Outline)
										})
										.inner;
									button.widget_info(|| {
										egui::WidgetInfo::labeled(
											egui::Role::Button,
											can_send,
											format!("{label} {}", user.name),
										)
									});
									if button.clicked() {
										send = Some(user.id);
									}
									if let Some(InviteStatus::Failed(failure)) = status {
										button.on_hover_text(failure.label());
									}
								});
							},
						);
					});
				}
				if friends.is_empty() {
					ui.colored_label(
						colors.muted,
						if !state.friends_known() {
							"Friends are not available yet."
						} else if query.is_empty() {
							"No friends to invite yet. Share the link below."
						} else {
							"No friends match your search."
						},
					);
				}
			});
		ui.spacing_mut().item_spacing.y = spacing;
		if let Some(user) = send
			&& let Some(command) = state.send_server_invite(guild, user)
		{
			commands.push(command);
		}
		ui.add_space(16.0);
		let y = ui.cursor().top();
		ui.painter().hline(
			ui.min_rect().left() - 30.0..=ui.max_rect().right() + 30.0,
			y,
			egui::Stroke::new(1.0, colors.border),
		);
		ui.add_space(20.0);
		ui.label(design::medium(
			ui,
			"Or, send a server invite link to a friend",
			17.0,
		));
		ui.add_space(8.0);
		self.link(ui, state, guild, *channel, commands);
		ui.add_space(12.0);
		ui.horizontal_wrapped(|ui| {
			let options = state.created_invite_options(guild).unwrap_or(self.options);
			ui.label(
				egui::RichText::new(if options.max_age == 0 {
					"Your invite link never expires.".to_owned()
				} else {
					format!(
						"Your invite link expires in {}.",
						expiry_label(options.max_age)
					)
				})
				.size(12.0)
				.color(colors.muted),
			);
			if ui
				.add_enabled(
					!state.server_action_pending() && !state.server_invite_pending(),
					egui::Button::new(
						egui::RichText::new("Edit link.")
							.size(12.0)
							.color(colors.link),
					)
					.frame(false),
				)
				.clicked()
			{
				self.settings = Some(options);
			}
		});
		if let Some(status) = state.server_action_status(guild)
			&& state.created_invite(guild).is_none()
		{
			design::notice(ui, design::Level::Warning, status);
		}
		if channel.is_none() {
			design::notice(
				ui,
				design::Level::Warning,
				"You need Create Invite permission in a channel to create an invite.",
			);
		}
	}
	fn link(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		channel: Option<Id>,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Frame::new()
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(10)
			.inner_margin(5)
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					let link = state.created_invite(guild);
					let pending = state.server_action_pending();
					let text = link.unwrap_or(if pending {
						"Creating invite link…"
					} else {
						"Create a link to share"
					});
					ui.add_sized(
						[(ui.available_width() - 100.0).max(24.0), 38.0],
						egui::Label::new(egui::RichText::new(text).size(18.0)).truncate(),
					);
					let label = if link.is_some() {
						if self.copied { "Copied" } else { "Copy" }
					} else if pending {
						"Creating…"
					} else {
						"Create link"
					};
					if ui
						.add_enabled_ui(!pending && channel.is_some(), |ui| {
							design::button(ui, label, design::ButtonKind::Primary)
						})
						.inner
						.clicked()
					{
						if let Some(link) = link {
							ui.ctx().copy_text(link.to_owned());
							self.copied = true;
						} else if let Some(channel) = channel
							&& let Some(command) = state.create_server_invite_with_options(
								guild,
								channel,
								self.options,
							) {
							commands.push(command);
						}
					}
				});
			});
	}
	fn settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		channel: Option<Id>,
		commands: &mut Vec<Command>,
	) {
		let pending = state.server_action_pending();
		let mut options = self.settings.unwrap();
		ui.add_space(28.0);
		ui.add_enabled_ui(!pending, |ui| {
			ui.label(design::medium(ui, "Expire After", 18.0));
			ui.add_space(8.0);
			select(ui, "invite-expiry", expiry_label(options.max_age), |ui| {
				for seconds in [1800, 3600, 21600, 43200, 86400, 604800, 2592000, 0] { ui.selectable_value(&mut options.max_age, seconds, expiry_label(seconds)); }
			});
			ui.add_space(24.0);
			ui.label(design::medium(ui, "Max Number of Uses", 18.0));
			ui.add_space(8.0);
			select(ui, "invite-uses", &uses_label(options.max_uses), |ui| {
				for uses in [0, 1, 5, 10, 25, 50, 100] { ui.selectable_value(&mut options.max_uses, uses, uses_label(uses)); }
			});
			ui.add_space(16.0);
			design::switch(ui, "Grant temporary membership", Some("Temporary members are automatically kicked when they disconnect unless a role has been assigned"), &mut options.temporary);
		});
		ui.add_space(24.0);
		let mut back = false;
		ui.horizontal(|ui| {
			if design::button(ui, "Cancel", design::ButtonKind::Outline).clicked() {
				back = true;
			}
			if ui
				.add_enabled_ui(
					!pending && !state.server_invite_pending() && channel.is_some(),
					|ui| {
						design::button(
							ui,
							if pending {
								"Generating…"
							} else {
								"Generate a New Link"
							},
							design::ButtonKind::Primary,
						)
					},
				)
				.inner
				.clicked() && let Some(channel) = channel
				&& let Some(command) =
					state.create_server_invite_with_options(guild, channel, options)
			{
				commands.push(command);
				self.options = options;
				self.copied = false;
				back = true;
			}
		});
		self.settings = if back { None } else { Some(options) };
	}
}
fn expiry_label(seconds: u32) -> &'static str {
	match seconds {
		0 => "Never",
		1800 => "30 minutes",
		3600 => "1 hour",
		21600 => "6 hours",
		43200 => "12 hours",
		86400 => "1 day",
		604800 => "7 days",
		_ => "30 days",
	}
}
fn uses_label(uses: u16) -> String {
	if uses == 0 {
		"No limit".into()
	} else {
		uses.to_string()
	}
}
fn select(ui: &mut egui::Ui, id: &str, label: &str, content: impl FnOnce(&mut egui::Ui)) {
	ui.scope(|ui| {
		ui.spacing_mut().button_padding = egui::vec2(14.0, 12.0);
		ui.spacing_mut().interact_size.y = 48.0;
		egui::ComboBox::from_id_salt(id)
			.selected_text(egui::RichText::new(label).size(18.0))
			.width(ui.available_width())
			.show_ui(ui, content);
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{Event, Pos2, Rect};
	fn draw(
		ctx: &egui::Context,
		dialog: &mut InviteDialog,
		state: &mut State,
		commands: &mut Vec<Command>,
		size: egui::Vec2,
		events: Vec<Event>,
	) -> Vec<(String, Rect)> {
		fn texts(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
			match shape {
				egui::Shape::Text(t) => labels.push((
					t.galley.job.text.clone(),
					t.galley.rect.translate(t.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						texts(shape, labels);
					}
				}
				_ => {}
			}
		}
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
				events,
				..Default::default()
			},
			|ui| {
				dialog.show(
					ui.ctx(),
					state,
					(Id(10), "Synthetic server"),
					&mut Some(Id(20)),
					&mut Avatars::default(),
					commands,
				);
			},
		);
		let mut labels = vec![];
		for shape in &output.shapes {
			texts(&shape.shape, &mut labels);
		}
		output.drop_without_applying_deltas();
		labels
	}
	fn click(
		ctx: &egui::Context,
		dialog: &mut InviteDialog,
		state: &mut State,
		commands: &mut Vec<Command>,
		size: egui::Vec2,
		label: &str,
	) {
		let labels = draw(ctx, dialog, state, commands, size, vec![]);
		let rect = labels
			.iter()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("Missing {label}: {labels:?}"))
			.1;
		assert!(
			rect.left() >= 0.0
				&& rect.right() <= size.x
				&& rect.top() >= 0.0
				&& rect.bottom() <= size.y,
			"{label} outside viewport: {rect:?}"
		);
		for pressed in [true, false] {
			draw(
				ctx,
				dialog,
				state,
				commands,
				size,
				vec![
					Event::PointerMoved(rect.center()),
					Event::PointerButton {
						pos: rect.center(),
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
	}
	#[test]
	fn invite_picker_search_send_settings_and_failure() {
		for (light, size) in [
			(false, egui::vec2(960.0, 760.0)),
			(true, egui::vec2(320.0, 760.0)),
		] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			ctx.set_visuals(if light {
				egui::Visuals::light()
			} else {
				egui::Visuals::dark()
			});
			let mut state = test_support::chat_demo_state();
			let mut dialog = InviteDialog::default();
			dialog.open();
			let mut commands = vec![];
			for _ in 0..3 {
				draw(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			}
			assert_eq!(commands.len(), 1, "only one create on deliberate open");
			let Command::ServerAction { action, request } = commands.pop().unwrap() else {
				panic!("wrong command")
			};
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::ServerAction(
					client_core::server_actions::Event::Written {
						action,
						request,
						result: Ok(Some("synthetic-invite".into())),
					},
				),
			});
			dialog.search = "casey.synthetic".into();
			let labels = draw(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			assert!(labels.iter().any(|(t, _)| t == "Casey"));
			assert!(!labels.iter().any(|(t, _)| t == "Robin"));
			click(&ctx, &mut dialog, &mut state, &mut commands, size, "Invite");
			let Command::SendServerInvite {
				guild,
				user,
				request,
				..
			} = commands.pop().unwrap()
			else {
				panic!("wrong send")
			};
			assert_eq!(user, Id(1002));
			draw(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			assert!(commands.is_empty());
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::ServerAction(
					client_core::server_actions::Event::InviteSent {
						guild,
						user,
						request,
						result: Err(client_core::auth::Failure::Forbidden),
					},
				),
			});
			let labels = draw(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			assert!(labels.iter().any(|(t, _)| t == "Retry"));
			assert!(
				commands.is_empty(),
				"failed sends never retry automatically"
			);
			click(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				"Edit link.",
			);
			dialog.settings.as_mut().unwrap().temporary = true;
			click(&ctx, &mut dialog, &mut state, &mut commands, size, "Cancel");
			assert!(commands.is_empty());
			assert!(!dialog.options.temporary);
			click(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				"Edit link.",
			);
			let options = InviteOptions {
				max_age: 3600,
				max_uses: 5,
				temporary: true,
			};
			dialog.settings = Some(options);
			click(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				"Generate a New Link",
			);
			let command = commands.pop().unwrap();
			assert!(
				matches!(&command, Command::ServerAction { action:client_core::server_actions::Action::CreateInvite { options:sent,.. },..} if *sent == options)
			);
			state.command_rejected(command);
			click(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				"Create link",
			);
			assert!(
				matches!(&commands[0], Command::ServerAction { action:client_core::server_actions::Action::CreateInvite { options:sent,.. },..} if *sent == options),
				"retry preserves chosen settings"
			);
		}
	}
}
