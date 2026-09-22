//! Group-only menus and one session-scoped editor. Selecting an image never sends it.
use crate::shortcuts::{Intent, ShortcutView};
use crate::{avatars::Avatars, design, icons};
use client_core::{Command, State};
use model::{Channel, Id, Patch, Shortcut};

pub type IconRequest = (u64, Id, u64);
struct Dialog {
	channel: Id,
	edit: bool,
	name: String,
	original: String,
	icon: Patch<String>,
	preview: Option<egui::TextureHandle>,
	choosing: bool,
	focus_name: bool,
	error: Option<&'static str>,
	submitted: Option<u64>,
}
#[derive(Default)]
pub(super) struct GroupMenu {
	dialog: Option<Dialog>,
	generation: u64,
	revision: u64,
	mute: Option<(Id, bool)>,
	pub pin_requested: Option<Intent>,
	pub icon_request: Option<IconRequest>,
}
impl GroupMenu {
	fn open(&mut self, state: &State, channel: &Channel, edit: bool) {
		self.generation = state.generation;
		self.revision = self.revision.wrapping_add(1);
		self.icon_request = None;
		self.dialog = Some(Dialog {
			channel: channel.id,
			edit,
			name: channel.name.clone(),
			original: channel.name.clone(),
			icon: Patch::Absent,
			preview: None,
			choosing: false,
			focus_name: edit,
			error: None,
			submitted: None,
		});
	}
	fn menu(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: &Channel,
		view: ShortcutView<'_>,
	) {
		if channel.guild.is_some() || channel.kind != 3 {
			return;
		}
		ui.set_width(224.0);
		let colors = design::palette(ui);
		let enabled = (state.demo || state.gateway_connected)
			&& !state.group_action_pending()
			&& !state.user_action_pending();
		let pinned = view.contains(Shortcut::Pinned, channel.id);
		if ui
			.add_enabled_ui(view.available(), |ui| {
				row(ui, if pinned { "Unpin DM" } else { "Pin DM" }, colors.text)
			})
			.inner
			.on_hover_text("Pinned direct messages are saved on this device.")
			.clicked()
		{
			self.pin_requested = Some(view.toggle(Shortcut::Pinned, channel.id));
			ui.close();
		}
		ui.separator();
		ui.add_enabled_ui(enabled, |ui| {
			if row(ui, "Edit Group", colors.text).clicked() {
				self.open(state, channel, true);
				ui.close();
			}
			ui.separator();
			let muted = state.dm_muted(channel.id) == Some(true);
			if row(
				ui,
				if muted {
					"Unmute Conversation"
				} else {
					"Mute Conversation"
				},
				colors.text,
			)
			.on_hover_text("Mute notifications until you unmute this conversation.")
			.clicked()
			{
				self.mute = Some((channel.id, !muted));
				ui.close();
			}
			ui.separator();
			if row(ui, "Leave Group", colors.danger).clicked() {
				self.open(state, channel, false);
				ui.close();
			}
		});
		if !enabled {
			ui.small("Group actions unavailable while disconnected or busy.");
		}
	}
	pub fn context(
		&mut self,
		response: &egui::Response,
		state: &State,
		channel: &Channel,
		view: ShortcutView<'_>,
	) {
		crate::user_menu::popup(response, response.id.with((state.generation, channel.id)))
			.show(|ui| self.menu(ui, state, channel, view));
	}
	pub fn dropdown(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: &Channel,
		view: ShortcutView<'_>,
	) {
		let response = icons::button(ui, icons::Icon::More, 28.0, "Group menu");
		egui::Popup::menu(&response)
			.id(response.id.with((state.generation, channel.id)))
			.show(|ui| self.menu(ui, state, channel, view));
	}
	pub fn accept_icon(
		&mut self,
		ctx: &egui::Context,
		request: IconRequest,
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		let Some(dialog) = self.dialog.as_mut().filter(|d| {
			d.edit && d.choosing && request == (self.generation, d.channel, self.revision)
		}) else {
			return;
		};
		dialog.choosing = false;
		match result {
			Ok(Some((data, image))) => {
				if data.len() > client_core::group_actions::MAX_ICON_DATA_URI
					|| image.size[0] > 256
					|| image.size[1] > 256
					|| image.pixels.len() > 256 * 256
				{
					dialog.error = Some("Group icon exceeds the size limit");
					return;
				}
				dialog.icon = Patch::Value(data);
				dialog.preview = Some(ctx.load_texture(
					"group-icon-preview",
					image,
					egui::TextureOptions::LINEAR,
				));
				dialog.error = None;
			}
			Ok(None) => {}
			Err(error) => dialog.error = Some(error),
		}
	}
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		if let Some((channel, muted)) = self.mute.take()
			&& let Some(command) = state.set_dm_muted(channel, muted)
		{
			commands.push(command);
		}
		let Some(dialog) = &mut self.dialog else {
			return;
		};
		if self.generation != state.generation
			|| state
				.channel(dialog.channel)
				.is_none_or(|c| c.kind != 3 || c.guild.is_some())
		{
			self.dialog = None;
			self.icon_request = None;
			return;
		}
		if let Some(request) = dialog.submitted
			&& let Some(success) = state.group_action_completed(dialog.channel, request)
		{
			if success {
				self.dialog = None;
				self.icon_request = None;
				return;
			}
			dialog.submitted = None;
			dialog.error = state.group_action_status(dialog.channel);
		}
		let colors = design::palette_for(ctx);
		let mut close = false;
		let busy = dialog.submitted.is_some() || state.group_action_pending();
		let mut builder = crate::dialog::Dialog::new(
			"group-editor",
			if dialog.edit {
				"Edit Group"
			} else {
				"Leave Group?"
			},
		)
		.width(440.0);
		builder = if dialog.edit {
			builder.subtitle("Give this group a name and an icon everyone will recognise.")
		} else {
			builder.danger().subtitle(format!(
				"You will need an invitation to rejoin {}.",
				dialog.name
			))
		};
		let response = builder.show(ctx, |d| {
			d.content(|ui| {
				ui.spacing_mut().item_spacing.y = 10.0;
				if dialog.edit {
					ui.add_space(6.0);
					ui.vertical_centered(|ui| {
						ui.add_enabled_ui(!busy && !dialog.choosing, |ui| {
							let (rect, mut response) = ui.allocate_exact_size(
								egui::Vec2::splat(120.0),
								egui::Sense::click(),
							);
							if let Some(preview) = &dialog.preview {
								egui::Image::new(preview)
									.corner_radius(60)
									.paint_at(ui, rect);
							} else {
								let mut channel = state.channel(dialog.channel).unwrap().clone();
								if matches!(dialog.icon, Patch::Null) {
									channel.icon = None;
								}
								let image = ui
									.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
										avatars.show_group(ui, &channel, 120.0, state.demo)
									})
									.inner;
								response = response.union(image);
							}
							let edit = egui::Rect::from_center_size(
								rect.right_top() + egui::vec2(-18.0, 18.0),
								egui::Vec2::splat(36.0),
							);
							ui.painter().circle_filled(edit.center(), 21.0, colors.chat);
							ui.painter()
								.circle_filled(edit.center(), 18.0, colors.raised);
							icons::paint(
								ui.painter(),
								icons::Icon::Pencil,
								edit.shrink(8.0),
								colors.text,
							);
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									ui.is_enabled(),
									"Change group icon",
								)
							});
							if response.on_hover_text("Change group icon").clicked() {
								dialog.choosing = true;
								dialog.error = None;
								self.icon_request =
									Some((self.generation, dialog.channel, self.revision));
							}
						});
						if dialog.choosing {
							ui.label("Choosing image…");
						}
						if (dialog.preview.is_some()
							|| state
								.channel(dialog.channel)
								.is_some_and(|c| c.icon.is_some()))
							&& !matches!(dialog.icon, Patch::Null)
							&& ui
								.add_enabled(
									!busy && !dialog.choosing,
									egui::Button::new("Remove icon").frame(false),
								)
								.clicked()
						{
							dialog.icon = Patch::Null;
							dialog.preview = None;
						}
					});
					ui.add_space(10.0);
					ui.add_enabled_ui(!busy, |ui| {
						let label = crate::dialog::label(ui, "Group name");
						let response = crate::dialog::input(
							ui,
							egui::TextEdit::singleline(&mut dialog.name)
								.char_limit(100)
								.hint_text("Group name")
								.id(egui::Id::unique(("group-name", self.revision))),
						)
						.labelled_by(label.id);
						if dialog.focus_name {
							response.request_focus();
							dialog.focus_name = false;
						}
					});
				} else if let Some(reason) = state.leave_group_reason(dialog.channel) {
					crate::dialog::notice(ui, crate::dialog::Level::Warning, reason);
				}
				if let Some(error) = dialog.error {
					crate::dialog::notice(ui, crate::dialog::Level::Error, error);
				}
				if state.demo {
					crate::dialog::hint(ui, "Offline preview · no group changes");
				}
			});
			d.footer(|ui| {
				let changed =
					dialog.name != dialog.original || !matches!(dialog.icon, Patch::Absent);
				let valid = if dialog.edit {
					!dialog.name.trim().is_empty() && changed
				} else {
					state.leave_group_reason(dialog.channel).is_none()
				};
				let label = if busy {
					if dialog.edit {
						"Saving…"
					} else {
						"Leaving…"
					}
				} else if dialog.edit {
					"Save"
				} else {
					"Leave Group"
				};
				let kind = if dialog.edit {
					crate::dialog::Action::Primary
				} else {
					crate::dialog::Action::Danger
				};
				ui.add_enabled_ui(!busy && !dialog.choosing && valid, |ui| {
					if crate::dialog::action(ui, label, kind).clicked() {
						let command = if dialog.edit {
							state.edit_group(
								dialog.channel,
								(dialog.name != dialog.original).then(|| dialog.name.clone()),
								dialog.icon.clone(),
							)
						} else {
							state.leave_group(dialog.channel)
						};
						if let Some(command) = command {
							if let Command::GroupAction { request, .. } = &command {
								dialog.submitted = Some(*request);
							}
							dialog.error = None;
							commands.push(command);
						} else {
							dialog.error = state.group_action_status(dialog.channel);
						}
					}
				});
				close |=
					crate::dialog::action(ui, "Cancel", crate::dialog::Action::Neutral).clicked();
			});
		});
		if close || response.close {
			self.dialog = None;
			self.icon_request = None;
		}
	}
}
fn row(ui: &mut egui::Ui, label: &str, color: egui::Color32) -> egui::Response {
	ui.add_sized(
		[ui.available_width(), 40.0],
		egui::Button::new(())
			.left_text(design::medium(ui, label, 14.0).color(color))
			.frame_when_inactive(false)
			.corner_radius(4),
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{Event, Pos2, Rect};
	fn fixture() -> (State, Id) {
		let mut state = test_support::demo_state();
		let channel = state.channels.iter_mut().find(|c| c.kind == 1).unwrap();
		channel.kind = 3;
		channel.name = "Synthetic group".into();
		let id = channel.id;
		(state, id)
	}
	fn pointer(pos: Pos2, secondary: bool, pressed: bool) -> Vec<Event> {
		vec![
			Event::PointerMoved(pos),
			Event::PointerButton {
				pos,
				button: if secondary {
					egui::PointerButton::Secondary
				} else {
					egui::PointerButton::Primary
				},
				pressed,
				modifiers: egui::Modifiers::NONE,
			},
		]
	}
	fn frame(
		ctx: &egui::Context,
		width: f32,
		menu: &mut GroupMenu,
		state: &mut State,
		channel: Id,
		events: Vec<Event>,
		commands: &mut Vec<Command>,
	) -> Vec<(String, Rect)> {
		let mut avatars = Avatars::default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 600.0))),
				events,
				..Default::default()
			},
			|ui| {
				let row = ui.add_sized([220.0, 40.0], egui::Button::new("Synthetic group"));
				let known = state.channel(channel).unwrap().clone();
				let preferences = model::ChannelPreferences::default();
				menu.context(&row, state, &known, ShortcutView::new(&preferences, true));
				menu.show(ui.ctx(), state, &mut avatars, commands);
			},
		);
		fn labels(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
			match shape {
				egui::Shape::Text(t) => out.push((
					t.galley.job.text.clone(),
					t.galley.rect.translate(t.pos.to_vec2()),
				)),
				// Target the edit affordance, independent of the avatar artwork underneath.
				egui::Shape::Circle(c) if c.radius == 18.0 => out.push((
					"icon".into(),
					Rect::from_center_size(c.center, egui::Vec2::splat(36.0)),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						labels(shape, out);
					}
				}
				_ => {}
			}
		}
		let mut text = vec![];
		for shape in &output.shapes {
			labels(&shape.shape, &mut text);
		}
		output.drop_without_applying_deltas();
		text
	}
	#[test]
	fn group_context_does_not_follow_a_reordered_row() {
		let ctx = egui::Context::default();
		let (mut state, channel) = fixture();
		let mut other = state.channel(channel).unwrap().clone();
		other.id = Id(999);
		state.channels.push(other);
		let mut menu = GroupMenu::default();
		let mut commands = vec![];
		frame(
			&ctx,
			640.0,
			&mut menu,
			&mut state,
			channel,
			vec![],
			&mut commands,
		);
		for pressed in [true, false] {
			frame(
				&ctx,
				640.0,
				&mut menu,
				&mut state,
				channel,
				pointer(egui::pos2(100.0, 20.0), true, pressed),
				&mut commands,
			);
		}
		let text = frame(
			&ctx,
			640.0,
			&mut menu,
			&mut state,
			Id(999),
			vec![],
			&mut commands,
		);
		assert!(!text.iter().any(|(text, _)| text == "Edit Group"));
		assert!(commands.is_empty());
	}
	#[test]
	fn group_menu_edit_mute_leave_and_stale_picker() {
		for (light, width) in [(false, 640.0), (true, 320.0)] {
			for action in ["Edit Group", "Mute Conversation", "Leave Group"] {
				let ctx = egui::Context::default();
				design::apply(&ctx);
				ctx.set_visuals(if light {
					egui::Visuals::light()
				} else {
					egui::Visuals::dark()
				});
				let (mut state, channel) = fixture();
				let mut menu = GroupMenu::default();
				let mut commands = vec![];
				frame(
					&ctx,
					width,
					&mut menu,
					&mut state,
					channel,
					vec![],
					&mut commands,
				);
				for pressed in [true, false] {
					frame(
						&ctx,
						width,
						&mut menu,
						&mut state,
						channel,
						pointer(egui::pos2(100.0, 20.0), true, pressed),
						&mut commands,
					);
				}
				let text = frame(
					&ctx,
					width,
					&mut menu,
					&mut state,
					channel,
					vec![],
					&mut commands,
				);
				for expected in ["Edit Group", "Mute Conversation", "Leave Group"] {
					assert!(text.iter().any(|(s, _)| s == expected));
				}
				assert!(
					!text
						.iter()
						.any(|(s, _)| matches!(s.as_str(), "Profile" | "Close DM" | "Block"))
				);
				assert!(commands.is_empty());
				let position = text.iter().find(|(s, _)| s == action).unwrap().1.center();
				for pressed in [true, false] {
					frame(
						&ctx,
						width,
						&mut menu,
						&mut state,
						channel,
						pointer(position, false, pressed),
						&mut commands,
					);
				}
				let text = frame(
					&ctx,
					width,
					&mut menu,
					&mut state,
					channel,
					vec![],
					&mut commands,
				);
				if action == "Mute Conversation" {
					assert_eq!(commands.len(), 1);
					assert_eq!(state.dm_muted(channel), Some(true));
					continue;
				}
				assert!(
					commands.is_empty(),
					"opening edit/leave must not send a write"
				);
				if action == "Edit Group" {
					let icon = text.iter().find(|(s, _)| s == "icon").unwrap().1.center();
					for pressed in [true, false] {
						frame(
							&ctx,
							width,
							&mut menu,
							&mut state,
							channel,
							pointer(icon, false, pressed),
							&mut commands,
						);
					}
					let request = menu.icon_request.take().expect("icon opens picker");
					menu.accept_icon(&ctx, request, Ok(None));
					assert!(!menu.dialog.as_ref().unwrap().choosing);
					menu.dialog.as_mut().unwrap().name = "Renamed group".into();
					assert!(commands.is_empty(), "selecting an icon never saves");
				}
				let label = if action == "Edit Group" {
					"Save"
				} else {
					"Leave Group"
				};
				let text = frame(
					&ctx,
					width,
					&mut menu,
					&mut state,
					channel,
					vec![],
					&mut commands,
				);
				let position = text
					.iter()
					.rev()
					.find(|(s, _)| s == label)
					.unwrap()
					.1
					.center();
				for pressed in [true, false] {
					frame(
						&ctx,
						width,
						&mut menu,
						&mut state,
						channel,
						pointer(position, false, pressed),
						&mut commands,
					);
				}
				assert_eq!(commands.len(), 1);
				let request = match &commands[0] {
					Command::GroupAction { request, .. } => *request,
					_ => panic!("expected group action"),
				};
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::GroupAction(
						client_core::group_actions::Event::Written {
							channel,
							request,
							result: Err(client_core::auth::Failure::Forbidden),
						},
					),
				});
				frame(
					&ctx,
					width,
					&mut menu,
					&mut state,
					channel,
					vec![],
					&mut commands,
				);
				assert!(menu.dialog.as_ref().unwrap().error.is_some());
				if action == "Edit Group" {
					assert_eq!(menu.dialog.as_ref().unwrap().name, "Renamed group");
				}
				let stale = (state.generation, channel, menu.revision);
				menu.open(&state, state.channel(channel).unwrap(), true);
				menu.dialog.as_mut().unwrap().choosing = true;
				menu.accept_icon(&ctx, stale, Err("stale picker error"));
				assert!(menu.dialog.as_ref().unwrap().error.is_none());
				state.generation += 1;
				frame(
					&ctx,
					width,
					&mut menu,
					&mut state,
					channel,
					vec![],
					&mut commands,
				);
				assert!(menu.dialog.is_none());
			}
		}
	}
}
