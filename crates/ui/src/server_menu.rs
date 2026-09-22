//! Server actions start from explicit menu choices, with one session-scoped dialog.
use crate::{avatars::Avatars, design, dialog, icons, server_invite::InviteDialog};
use client_core::{Command, State};
use model::Id;

#[derive(Clone, Copy)]
enum Dialog {
	Invite { guild: Id, channel: Option<Id> },
	Leave(Id),
}

impl Dialog {
	fn guild(self) -> Id {
		match self {
			Self::Invite { guild, .. } | Self::Leave(guild) => guild,
		}
	}
}

#[derive(Default)]
pub(super) struct ServerMenu {
	pub settings_requested: Option<Id>,
	pub mark_read_requested: Option<Id>,
	dialog: Option<Dialog>,
	generation: u64,
	invite: InviteDialog,
}

impl ServerMenu {
	pub fn read_item(&mut self, ui: &mut egui::Ui, state: &State, guild: Id) -> bool {
		if ui
			.add_enabled_ui(state.can_mark_guild_read(guild), |ui| {
				menu_row(
					ui,
					icons::Icon::Check,
					"Mark As Read",
					design::palette(ui).text,
				)
			})
			.inner
			.clicked()
		{
			self.mark_read_requested = Some(guild);
			ui.close();
			return true;
		}
		false
	}
	pub fn settings_item(&mut self, ui: &mut egui::Ui, state: &State, guild: Id) -> bool {
		if (state.can_manage_guild(guild)
			|| state.can_open_role_settings(guild)
			|| state.can_open_emoji_settings(guild)
			|| state.can_open_integration_settings(guild)
			|| state.can_open_audit_log_settings(guild)
			|| state.can_open_member_settings(guild))
			&& menu_row(
				ui,
				icons::Icon::Gear,
				"Server Settings",
				design::palette(ui).text,
			)
			.clicked()
		{
			self.settings_requested = Some(guild);
			ui.close();
			return true;
		}
		false
	}
	pub fn leave_item(&mut self, ui: &mut egui::Ui, state: &mut State, guild: Id) -> bool {
		if state.leave_server_reason(guild).is_some() {
			return false;
		}
		let available = !state.server_action_pending()
			&& !state.server_invite_pending()
			&& (state.demo || state.gateway_connected);
		if ui
			.add_enabled_ui(available, |ui| {
				menu_row(
					ui,
					icons::Icon::ArrowRight,
					"Leave server",
					design::palette(ui).danger,
				)
			})
			.inner
			.clicked()
		{
			state.clear_server_action_result(guild);
			self.dialog = Some(Dialog::Leave(guild));
			self.generation = state.generation;
			ui.close();
			return true;
		}
		false
	}
	pub fn open_invite(&mut self, state: &mut State, guild: Id, channel: Id) {
		if !state.can_create_server_invite(guild, channel) {
			return;
		}
		state.clear_server_action_result(guild);
		self.dialog = Some(Dialog::Invite {
			guild,
			channel: Some(channel),
		});
		self.generation = state.generation;
		self.invite.open();
	}
	pub fn header(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		title: &str,
	) -> egui::Response {
		ui.push_id(("server-menu", guild), |ui| {
			let colors = design::palette(ui);
			let button = ui.add_sized(
				[ui.available_width(), 40.0],
				egui::Button::new(()).frame(false),
			);
			egui::ContainerAtom::new((
				design::semibold(ui, title, 15.0),
				egui::Atom::paint(egui::Vec2::splat(14.0), |ui, args| {
					icons::paint(
						ui.painter(),
						icons::Icon::ChevronDown,
						args.rect,
						colors.muted,
					);
				}),
			))
			.gap(5.0)
			.align2(egui::Align2::LEFT_CENTER)
			.wrap_mode(egui::TextWrapMode::Truncate)
			.fallback_text_color(colors.text_strong)
			.measure(ui, button.rect.size())
			.paint_at(ui, button.rect);
			button.widget_info(|| {
				egui::WidgetInfo::labeled(
					egui::Role::Button,
					ui.is_enabled(),
					format!("Server menu, {title}"),
				)
			});
			egui::Popup::menu(&button)
				.frame(
					egui::Frame::popup(ui.style())
						.fill(colors.chat)
						.inner_margin(8)
						.corner_radius(8),
				)
				.show(|ui| {
					ui.set_width(232.0);
					let available = !state.server_action_pending()
						&& !state.server_invite_pending()
						&& (state.demo || state.gateway_connected);
					self.read_item(ui, state, guild);
					ui.separator();
					self.settings_item(ui, state, guild);
					if ui
						.add_enabled_ui(available, |ui| {
							menu_row(ui, icons::Icon::AddPeople, "Create invite", colors.text)
						})
						.inner
						.clicked()
					{
						state.clear_server_action_result(guild);
						self.dialog = Some(Dialog::Invite {
							guild,
							channel: state.invite_channel(guild),
						});
						self.generation = state.generation;
						self.invite.open();
						ui.close();
					}
					ui.separator();
					self.leave_item(ui, state, guild);
					if !available {
						ui.small(if state.server_action_pending() {
							"A server action is in progress."
						} else {
							"Reconnect to manage this server."
						});
					}
				});
			button
		})
		.inner
	}

	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		active: Option<Id>,
		commands: &mut Vec<Command>,
		avatars: &mut Avatars,
	) {
		let Some(mut dialog) = self.dialog else {
			return;
		};
		let guild = dialog.guild();
		if self.generation != state.generation || active != Some(guild) {
			self.dialog = None;
			self.invite = InviteDialog::default();
			return;
		}
		let Some(name) = state
			.guilds
			.iter()
			.find(|g| g.id == guild)
			.map(|g| g.name.clone())
		else {
			self.dialog = None;
			self.invite = InviteDialog::default();
			return;
		};
		if let Dialog::Invite { channel, .. } = &mut dialog {
			let close = self
				.invite
				.show(ctx, state, (guild, &name), channel, avatars, commands);
			self.dialog = if close { None } else { Some(dialog) };
			if close {
				self.invite = InviteDialog::default();
			}
			return;
		}
		let pending = state.server_action_pending();
		let reason = state.leave_server_reason(guild);
		let mut leave = false;
		let mut close = false;
		let response = dialog::Dialog::new("server-action-dialog", "Leave server?")
			.danger()
			.width(440.0)
			.show(ctx, |d| {
				d.content(|ui| {
					let colors = design::palette(ui);
					ui.spacing_mut().item_spacing.y = 10.0;
					ui.add(
						egui::Label::new(
							egui::RichText::new(format!(
								"Are you sure you want to leave {name}? You will not be able to rejoin this server unless you are re-invited."
							))
							.size(14.0)
							.color(colors.text),
						)
						.wrap(),
					);
					if let Some(reason) = reason {
						dialog::notice(ui, dialog::Level::Warning, reason);
					}
					if let Some(status) = state.server_action_status(guild) {
						dialog::notice(ui, dialog::Level::Error, status);
					}
					if state.demo {
						dialog::hint(ui, "Offline preview · no server changes");
					}
				});
				d.footer(|ui| {
					ui.add_enabled_ui(!pending && reason.is_none(), |ui| {
						leave = dialog::action(
							ui,
							if pending { "Leaving…" } else { "Leave Server" },
							dialog::Action::Danger,
						)
						.clicked();
					});
					close |= dialog::action(
						ui,
						if pending { "Close" } else { "Cancel" },
						dialog::Action::Neutral,
					)
					.clicked();
				});
			});
		if leave && let Some(command) = state.leave_server(guild) {
			commands.push(command);
		}
		self.dialog = if close || response.close {
			None
		} else {
			Some(dialog)
		};
	}
}

fn menu_row(
	ui: &mut egui::Ui,
	icon: icons::Icon,
	label: &str,
	color: egui::Color32,
) -> egui::Response {
	ui.scope(|ui| {
		ui.spacing_mut().button_padding = egui::vec2(36.0, 8.0);
		let response = ui.add_sized(
			[ui.available_width(), 36.0],
			egui::Button::new(())
				.left_text(design::medium(ui, label, 14.0).color(color))
				.frame_when_inactive(false)
				.corner_radius(4),
		);
		icons::paint(
			ui.painter(),
			icon,
			egui::Rect::from_center_size(
				egui::pos2(response.rect.left() + 18.0, response.rect.center().y),
				egui::Vec2::splat(20.0),
			),
			if ui.is_enabled() {
				color
			} else {
				color.gamma_multiply(0.5)
			},
		);
		response
	})
	.inner
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{Event, Pos2, Rect};

	fn labels(shape: &egui::Shape, text: &mut Vec<(String, Rect)>) {
		match shape {
			egui::Shape::Text(t) => text.push((
				t.galley.job.text.clone(),
				t.galley.rect.translate(t.pos.to_vec2()),
			)),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| labels(s, text)),
			_ => {}
		}
	}
	fn pointer(pos: Pos2, pressed: bool) -> Vec<Event> {
		vec![
			Event::PointerMoved(pos),
			Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed,
				modifiers: egui::Modifiers::NONE,
			},
		]
	}
	fn frame(
		ctx: &egui::Context,
		width: f32,
		menu: &mut ServerMenu,
		state: &mut State,
		events: Vec<Event>,
		commands: &mut Vec<Command>,
	) -> (egui::Response, Vec<(String, Rect)>) {
		let copied_before = menu.invite.copied;
		let mut response = None;
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 550.0))),
				events,
				..Default::default()
			},
			|ui| {
				response = Some(menu.header(
					ui,
					state,
					Id(10),
					"A long synthetic server name for the narrow sidebar",
				));
				menu.show(
					ui.ctx(),
					state,
					Some(Id(10)),
					commands,
					&mut Avatars::default(),
				);
			},
		);
		let mut text = vec![];
		for shape in &output.shapes {
			labels(&shape.shape, &mut text);
		}
		if menu.invite.copied && !copied_before {
			assert!(output.platform_output.commands.iter().any(|command| {
				matches!(command, egui::OutputCommand::CopyText(link) if link == "https://discord.gg/synthetic-invite")
			}));
		}
		output.drop_without_applying_deltas();
		(response.unwrap(), text)
	}
	#[test]
	fn shared_server_items_hide_settings_without_access_and_leave_for_owners() {
		for (owner, known) in [(false, true), (true, true), (false, false)] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let mut state = test_support::chat_demo_state();
			let guild = Id(10);
			let mut snapshot = test_support::permission_snapshot(&state);
			for permissions in &mut snapshot.guilds {
				permissions.owner = known.then_some(if owner {
					state.user.as_ref().unwrap().id
				} else {
					Id(u64::MAX)
				});
				if let Some(roles) = &mut permissions.roles {
					for role in roles {
						role.bits = 0;
					}
				}
			}
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::Permissions(client_core::permissions::Event::Snapshot(
					snapshot,
				)),
			});
			let mut menu = ServerMenu::default();
			let output = ctx.run_ui(egui::RawInput::default(), |ui| {
				ui.set_width(232.0);
				menu.settings_item(ui, &state, guild);
				menu.leave_item(ui, &mut state, guild);
			});
			let mut text = vec![];
			for shape in &output.shapes {
				labels(&shape.shape, &mut text);
			}
			assert_eq!(
				text.iter().any(|(label, _)| label == "Server Settings"),
				owner && known
			);
			assert_eq!(
				text.iter().any(|(label, _)| label == "Leave server"),
				!owner && known
			);
			assert!(menu.settings_requested.is_none() && menu.dialog.is_none());
			output.drop_without_applying_deltas();
		}
	}
	#[test]
	fn server_menu_requires_explicit_invite_and_leave_confirmation() {
		for (light, width) in [(false, 320.0), (true, 320.0), (false, 960.0), (true, 960.0)] {
			for action in ["Create invite", "Leave server"] {
				let ctx = egui::Context::default();
				design::apply(&ctx);
				ctx.set_visuals(if light {
					egui::Visuals::light()
				} else {
					egui::Visuals::dark()
				});
				let mut state = test_support::chat_demo_state();
				let mut menu = ServerMenu::default();
				let mut commands = vec![];
				let (button, _) = frame(&ctx, width, &mut menu, &mut state, vec![], &mut commands);
				if light {
					button.request_focus();
					frame(
						&ctx,
						width,
						&mut menu,
						&mut state,
						vec![Event::Key {
							key: egui::Key::Enter,
							physical_key: None,
							pressed: true,
							repeat: false,
							modifiers: egui::Modifiers::NONE,
						}],
						&mut commands,
					);
				} else {
					for pressed in [true, false] {
						frame(
							&ctx,
							width,
							&mut menu,
							&mut state,
							pointer(button.rect.center(), pressed),
							&mut commands,
						);
					}
				}
				let (_, text) = frame(&ctx, width, &mut menu, &mut state, vec![], &mut commands);
				assert!(commands.is_empty());
				let position = text
					.iter()
					.find(|(t, _)| t == action)
					.unwrap_or_else(|| panic!("Missing {action}: {text:?}"))
					.1
					.center();
				for pressed in [true, false] {
					frame(
						&ctx,
						width,
						&mut menu,
						&mut state,
						pointer(position, pressed),
						&mut commands,
					);
				}
				let (_, text) = frame(&ctx, width, &mut menu, &mut state, vec![], &mut commands);
				assert_eq!(
					commands.len(),
					usize::from(action == "Create invite"),
					"Create invite is deliberate; Leave still requires confirmation"
				);
				if action == "Create invite" {
					let heading = text
						.iter()
						.find(|(t, _)| t.starts_with("Invite friends to "))
						.unwrap()
						.1;
					let footer = text
						.iter()
						.find(|(t, _)| t == "Or, send a server invite link to a friend")
						.unwrap()
						.1;
					assert!(
						(heading.left() - footer.left()).abs() < 1.0,
						"heading and footer align left: {heading:?} {footer:?}"
					);
				}
				if action == "Leave server" {
					let position = text
						.iter()
						.rev()
						.find(|(t, _)| t == "Leave Server")
						.unwrap()
						.1
						.center();
					for pressed in [true, false] {
						frame(
							&ctx,
							width,
							&mut menu,
							&mut state,
							pointer(position, pressed),
							&mut commands,
						);
					}
				}
				assert_eq!(
					commands.len(),
					1,
					"one explicit confirmation sends one command"
				);
				assert!(matches!(&commands[0], Command::ServerAction { .. }));
				frame(&ctx, width, &mut menu, &mut state, vec![], &mut commands);
				assert_eq!(commands.len(), 1);
				if action == "Create invite" {
					let Command::ServerAction { action, request } = commands[0] else {
						unreachable!()
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
					let (_, text) =
						frame(&ctx, width, &mut menu, &mut state, vec![], &mut commands);
					let copy = text.iter().find(|(t, _)| t == "Copy").unwrap().1;
					assert!(
						copy.left() >= 0.0 && copy.right() <= width,
						"copy stays inside the viewport"
					);
					for pressed in [true, false] {
						frame(
							&ctx,
							width,
							&mut menu,
							&mut state,
							pointer(copy.center(), pressed),
							&mut commands,
						);
					}
					assert!(menu.invite.copied);
					assert_eq!(commands.len(), 1, "copying does not create another invite");
				}
				state.generation += 1;
				frame(&ctx, width, &mut menu, &mut state, vec![], &mut commands);
				assert!(menu.dialog.is_none(), "old account dialogs close");
			}
		}
	}
}
