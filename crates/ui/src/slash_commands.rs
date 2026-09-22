//! Composer command discovery and one session-only application command form.
use crate::{design, mentions, slash_builtin};
use client_core::{Command, State};
use model::{
	Id,
	application_commands::{CommandOption, Value},
};

const RESULTS: usize = 64;
const ROW: f32 = 52.0;

#[derive(Clone)]
pub(super) struct Pick {
	id: Option<Id>,
	path: Vec<String>,
	name: String,
	description: String,
	application: String,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Filter {
	#[default]
	All,
	Builtins,
	Application(Id),
}

pub(super) struct Active {
	pub id: Id,
	pub path: Vec<String>,
	pub values: Vec<(String, String)>,
	name: String,
}

#[derive(Default)]
pub(super) struct Menu {
	channel: Option<Id>,
	generation: u64,
	query: Option<String>,
	stamp: (u64, bool, usize),
	filter: Filter,
	items: Vec<Pick>,
	selected: usize,
	dismissed: bool,
	enabled: bool,
	follow: bool,
	rect: Option<egui::Rect>,
	pub active: Option<Active>,
	pub error: Option<&'static str>,
	pub run: bool,
	pub retry: bool,
	last: Option<Active>,
}

impl Menu {
	pub fn pointer_interacting(&self, ctx: &egui::Context, channel: Id) -> bool {
		self.channel == Some(channel)
			&& self.rect.is_some_and(|rect| {
				ctx.input(|i| {
					i.pointer
						.interact_pos()
						.is_some_and(|point| rect.contains(point))
				})
			})
	}
	pub fn form_has_focus(&self, ctx: &egui::Context) -> bool {
		self.active.is_some()
			&& ctx
				.memory(|memory| memory.focused())
				.and_then(|id| ctx.read_response(id))
				.is_some_and(|response| {
					self.rect
						.is_some_and(|rect| rect.contains(response.rect.center()))
				})
	}
	pub fn refresh(&mut self, state: &State, channel: Id, draft: &str, enabled: bool) {
		if self.channel != Some(channel) || self.generation != state.generation {
			*self = Self {
				channel: Some(channel),
				generation: state.generation,
				..Self::default()
			};
		}
		self.enabled = enabled;
		if self
			.active
			.as_ref()
			.is_some_and(|active| draft.trim_end() != format!("/{}", active.name))
		{
			self.active = None;
			self.error = None;
		}
		let query = draft
			.strip_prefix('/')
			.filter(|query| {
				enabled
					&& query.len() <= 128
					&& !query.contains('\n')
					&& (slash_builtin::query(draft).is_some()
						|| slash_builtin::parse(draft).is_none())
			})
			.map(|query| query.trim_end().to_lowercase());
		let catalog = &state.application_commands;
		let stamp = (catalog.request, catalog.loading, catalog.commands.len());
		if self.query == query && self.stamp == stamp {
			return;
		}
		if self.query != query {
			self.dismissed = false;
			self.selected = 0;
		}
		self.query = query;
		self.stamp = stamp;
		self.rebuild(state);
	}
	fn rebuild(&mut self, state: &State) {
		self.items.clear();
		let Some(query) = self.query.as_deref() else {
			return;
		};
		if matches!(self.filter, Filter::All | Filter::Builtins) {
			for entry in slash_builtin::ALL {
				if entry.name.contains(query) || entry.description.to_lowercase().contains(query) {
					self.items.push(Pick {
						id: None,
						path: Vec::new(),
						name: entry.name.into(),
						description: format!("{}  {}", entry.description, entry.usage),
						application: "Built-In".into(),
					});
				}
			}
		}
		if self.filter != Filter::Builtins {
			for command in &state.application_commands.commands {
				if matches!(self.filter, Filter::Application(id) if id != command.application_id) {
					continue;
				}
				let mut leaves = Vec::new();
				if command
					.options
					.iter()
					.any(|option| matches!(option.kind, 1 | 2))
				{
					for option in &command.options {
						if option.kind == 1 {
							leaves.push((vec![option.name.clone()], option.description.as_str()));
						} else if option.kind == 2 {
							for child in &option.options {
								leaves.push((
									vec![option.name.clone(), child.name.clone()],
									child.description.as_str(),
								));
							}
						}
					}
				} else {
					leaves.push((Vec::new(), command.description.as_str()));
				}
				for (path, description) in leaves {
					let name = if path.is_empty() {
						command.name.clone()
					} else {
						format!("{} {}", command.name, path.join(" "))
					};
					if [name.as_str(), description, &command.application_name]
						.iter()
						.any(|text| text.to_lowercase().contains(query))
					{
						self.items.push(Pick {
							id: Some(command.id),
							path,
							name,
							description: description.into(),
							application: command.application_name.clone(),
						});
						if self.items.len() >= RESULTS {
							break;
						}
					}
				}
				if self.items.len() >= RESULTS {
					break;
				}
			}
		}
		self.selected = self.selected.min(self.items.len().saturating_sub(1));
	}
	pub fn visible(&self) -> bool {
		self.enabled && !self.dismissed && (self.query.is_some() || self.active.is_some())
	}
	pub fn keys(&mut self, ctx: &egui::Context) -> Option<Pick> {
		if !self.visible() || egui::Popup::is_any_open(ctx) {
			return None;
		}
		ctx.input_mut(|input| {
			input.events.retain(|event| {
				!matches!(
					event,
					egui::Event::Key {
						key: egui::Key::Enter | egui::Key::Tab,
						repeat: true,
						..
					}
				)
			});
			if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
				self.dismissed = true;
				return None;
			}
			if self.active.is_some() {
				if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
					self.run = true;
				}
				return None;
			}
			if self.items.is_empty() {
				return None;
			}
			if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
				self.selected = (self.selected + 1) % self.items.len();
				self.follow = true;
			}
			if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
				self.selected = (self.selected + self.items.len() - 1) % self.items.len();
				self.follow = true;
			}
			if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
				|| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
			{
				return self.items.get(self.selected).cloned();
			}
			None
		})
	}
	pub fn accept(&mut self, pick: Pick, draft: &mut String, remaining: usize) -> Option<usize> {
		let completion = format!("/{} ", pick.name);
		if completion.capacity() > remaining.saturating_add(draft.capacity()) {
			self.error = Some("Free some draft space before choosing a command.");
			return None;
		}
		*draft = completion;
		self.active = pick.id.map(|id| Active {
			id,
			path: pick.path,
			values: Vec::new(),
			name: pick.name,
		});
		self.error = None;
		self.dismissed = false;
		Some(draft.chars().count())
	}
	pub fn show(
		&mut self,
		ui: &egui::Ui,
		anchor: egui::Rect,
		state: &State,
		channel: Id,
	) -> Option<Pick> {
		self.rect = None;
		if !self.visible() {
			return None;
		}
		let colors = design::palette(ui);
		let bounds = ui.ctx().content_rect().shrink(8.0);
		let width = anchor.width().min(bounds.width());
		let height = if self.active.is_some() {
			310.0
		} else {
			(self.items.len().clamp(2, 5) as f32) * ROW + 68.0
		};
		let height = height
			.min((anchor.top() - bounds.top() - 8.0).max(96.0))
			.min(bounds.height());
		let position = egui::pos2(
			anchor
				.left()
				.clamp(bounds.left(), (bounds.right() - width).max(bounds.left())),
			(anchor.top() - height - 8.0).max(bounds.top()),
		);
		let mut picked = None;
		let response = egui::Area::new(egui::Id::unique(("slash-commands", channel)))
			.kind(egui::UiKind::Popup)
			.order(egui::Order::Foreground)
			.fixed_pos(position)
			.constrain_to(bounds)
			.show(ui.ctx(), |ui| {
				egui::Frame::new()
					.fill(colors.sidebar)
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(8)
					.inner_margin(8)
					.show(ui, |ui| {
						ui.set_width((width - 18.0).max(80.0));
						ui.horizontal(|ui| {
							if self.active.is_some() && ui.small_button("← Commands").clicked() {
								self.active = None;
								self.error = None;
							}
							ui.label(design::semibold(ui, "Commands", 14.0));
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									if ui
										.add_enabled(
											state.can_request_application_commands(channel)
												&& !state.application_commands.loading,
											egui::Button::new("Refresh apps").small(),
										)
										.clicked()
									{
										self.retry = true;
									}
									if width > 600.0 {
										ui.label(
											egui::RichText::new(if self.active.is_some() {
												"Tab next field · Enter run · Esc close"
											} else {
												"↑↓ choose · Tab / Enter select · Esc close"
											})
											.size(11.0)
											.color(colors.muted),
										);
									}
								},
							);
						});
						ui.separator();
						if self.active.is_some() {
							self.form(ui, state, channel, height - 68.0);
						} else {
							ui.horizontal_top(|ui| {
								ui.vertical(|ui| {
									ui.set_width(38.0);
									let mut filter = self.filter;
									if ui
										.add_sized(
											[34.0, 30.0],
											egui::Button::new("All")
												.selected(filter == Filter::All),
										)
										.clicked()
									{
										filter = Filter::All;
									}
									if ui
										.add_sized(
											[34.0, 30.0],
											egui::Button::new("/")
												.selected(filter == Filter::Builtins),
										)
										.on_hover_text("Built-In")
										.clicked()
									{
										filter = Filter::Builtins;
									}
									egui::ScrollArea::vertical()
										.id_salt("command-apps")
										.max_height((height - 146.0).max(32.0))
										.show(ui, |ui| {
											let mut seen = std::collections::BTreeSet::new();
											for command in &state.application_commands.commands {
												if !seen.insert(command.application_id) {
													continue;
												}
												let label = command
													.application_name
													.chars()
													.next()
													.unwrap_or('A')
													.to_string();
												if ui
													.add_sized(
														[34.0, 30.0],
														egui::Button::new(label).selected(
															filter
																== Filter::Application(
																	command.application_id,
																),
														),
													)
													.on_hover_text(&command.application_name)
													.clicked()
												{
													filter =
														Filter::Application(command.application_id);
												}
											}
										});
									if filter != self.filter {
										self.filter = filter;
										self.selected = 0;
										self.rebuild(state);
									}
								});
								ui.separator();
								ui.vertical(|ui| {
									let follow = std::mem::take(&mut self.follow);
									egui::ScrollArea::vertical()
										.id_salt("command-results")
										.max_height((height - 100.0).max(ROW))
										.auto_shrink([false, true])
										.show(ui, |ui| {
											for (index, item) in self.items.iter().enumerate() {
												let response =
													command_row(ui, item, index == self.selected);
												if follow && index == self.selected {
													response.scroll_to_me(None);
												}
												if response.clicked() {
													picked = Some(item.clone());
												}
											}
											if self.items.is_empty() {
												ui.weak(
													"No commands match. Try another name or application.",
												);
											}
										});
									if state.application_commands.loading {
										ui.weak("Loading application commands…");
									} else if let Some(error) =
										self.error.or(state.application_commands.error)
									{
										ui.colored_label(colors.danger, error);
									} else if self.items.len() == RESULTS {
										ui.weak("Keep typing to narrow the results.");
									}
								});
							});
						}
					});
			});
		self.rect = Some(response.response.rect);
		picked
	}
	fn form(&mut self, ui: &mut egui::Ui, state: &State, channel: Id, height: f32) {
		let Some(active) = self.active.as_mut() else {
			return;
		};
		let Some(command) = state
			.application_commands
			.commands
			.iter()
			.find(|command| command.id == active.id)
		else {
			ui.weak("This command is no longer available. Refresh apps and choose it again.");
			return;
		};
		ui.horizontal(|ui| {
			ui.strong(format!("/{}", active.name));
			ui.weak(&command.application_name);
		});
		let Ok(options) = command.options_at(&active.path) else {
			ui.weak("Choose a subcommand.");
			return;
		};
		active
			.values
			.retain(|(name, _)| options.iter().any(|option| option.name == *name));
		egui::ScrollArea::vertical()
			.id_salt(("slash-arguments", active.id))
			.max_height((height - 85.0).max(36.0))
			.show(ui, |ui| {
				if options.is_empty() {
					ui.weak(&command.description);
				}
				egui::Grid::new(("slash-fields", active.id))
					.num_columns(2)
					.spacing([12.0, 8.0])
					.show(ui, |ui| {
						for option in options {
							ui.label(format!(
								"{}{}",
								option.name,
								if option.required { " *" } else { "" }
							))
							.on_hover_text(&option.description);
							if !active.values.iter().any(|(name, _)| *name == option.name) {
								active.values.push((option.name.clone(), String::new()));
							}
							let value = &mut active
								.values
								.iter_mut()
								.find(|(name, _)| *name == option.name)
								.unwrap()
								.1;
							argument(ui, option, value, state, channel);
							ui.end_row();
						}
					});
			});
		if let Some(error) = self.error.or(state.interactions.error) {
			ui.colored_label(design::palette(ui).danger, error);
		}
		ui.horizontal(|ui| {
			if ui
				.add_enabled(
					!state.interactions.busy()
						&& !options
							.iter()
							.any(|option| option.kind == 11 && option.required),
					egui::Button::new("Run command"),
				)
				.clicked()
			{
				self.run = true;
			}
			if state.interactions.busy() {
				ui.weak("Waiting for the application…");
			} else {
				ui.weak("* required");
			}
		});
	}
}

fn command_row(ui: &mut egui::Ui, item: &Pick, selected: bool) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
	if !ui.is_rect_visible(rect) {
		return response;
	}
	if selected || response.hovered() {
		ui.painter().rect_filled(
			rect.shrink(1.0),
			5,
			if selected {
				colors.selected
			} else {
				colors.hover
			},
		);
	}
	let source_width = (rect.width() * 0.28).min(150.0);
	let label_width = (rect.width() - source_width - 20.0).max(32.0);
	let mut name = egui::text::LayoutJob::simple_singleline(
		format!("/{}", item.name),
		egui::FontId::proportional(16.0),
		colors.text_strong,
	);
	name.wrap = egui::text::TextWrapping {
		max_width: label_width,
		max_rows: 1,
		break_anywhere: true,
		..Default::default()
	};
	let mut description = egui::text::LayoutJob::simple_singleline(
		item.description.clone(),
		egui::FontId::proportional(12.0),
		colors.muted,
	);
	description.wrap = egui::text::TextWrapping {
		max_width: label_width,
		max_rows: 1,
		break_anywhere: true,
		..Default::default()
	};
	ui.painter().galley(
		rect.min + egui::vec2(8.0, 6.0),
		ui.fonts_mut(|f| f.layout_job(name)),
		colors.text_strong,
	);
	ui.painter().galley(
		rect.min + egui::vec2(8.0, 29.0),
		ui.fonts_mut(|f| f.layout_job(description)),
		colors.muted,
	);
	let mut source = ui.new_child(
		egui::UiBuilder::new()
			.max_rect(egui::Rect::from_min_max(
				egui::pos2(rect.right() - source_width, rect.top()),
				rect.max,
			))
			.layout(egui::Layout::right_to_left(egui::Align::Center)),
	);
	source.add(
		egui::Label::new(
			egui::RichText::new(&item.application)
				.size(11.0)
				.color(colors.muted),
		)
		.truncate(),
	);
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Button,
			true,
			format!(
				"/{} · {} · {}",
				item.name, item.description, item.application
			),
		)
	});
	response.on_hover_text(&item.description)
}

fn value_text(value: &Value) -> String {
	match value {
		Value::String(value) => value.clone(),
		Value::Integer(value) => value.to_string(),
		Value::Number(value) => value.to_string(),
		Value::Boolean(value) => value.to_string(),
	}
}

fn argument(
	ui: &mut egui::Ui,
	option: &CommandOption,
	value: &mut String,
	state: &State,
	channel: Id,
) {
	if !option.choices.is_empty() || option.kind == 5 {
		let label = option
			.choices
			.iter()
			.find(|choice| value_text(&choice.value) == *value)
			.map_or(value.as_str(), |choice| choice.name.as_str());
		egui::ComboBox::from_id_salt(("slash-choice", &option.name))
			.selected_text(if label.is_empty() { "Choose…" } else { label })
			.show_ui(ui, |ui| {
				ui.selectable_value(value, String::new(), "Not set");
				if option.kind == 5 {
					ui.selectable_value(value, "true".into(), "True");
					ui.selectable_value(value, "false".into(), "False");
				}
				for choice in &option.choices {
					ui.selectable_value(value, value_text(&choice.value), &choice.name);
				}
			});
	} else if matches!(option.kind, 6..=9) {
		ui.horizontal(|ui| {
			ui.add(
				egui::TextEdit::singleline(value)
					.hint_text("ID or choose…")
					.char_limit(22)
					.desired_width((ui.available_width() - 45.0).max(70.0)),
			);
			ui.menu_button("▾", |ui| {
				egui::ScrollArea::vertical()
					.max_height(180.0)
					.show(ui, |ui| {
						if matches!(option.kind, 6 | 9) {
							for user in mentions::known_users(state, channel) {
								if ui.button(format!("@{}", user.name)).clicked() {
									*value = user.id.to_string();
									ui.close();
								}
							}
						}
						if matches!(option.kind, 8 | 9) {
							for role in mentions::known_roles(state, channel) {
								if ui.button(format!("@{}", role.name)).clicked() {
									*value = role.id.to_string();
									ui.close();
								}
							}
						}
						if option.kind == 7 {
							let guild = state.channel(channel).and_then(|channel| channel.guild);
							for target in state
								.channels
								.iter()
								.filter(|target| {
									target.guild == guild
										&& state.can_view(target.id) && (option
										.channel_types
										.is_empty() || option
										.channel_types
										.contains(&target.kind))
								})
								.take(256)
							{
								if ui.button(format!("#{}", target.name)).clicked() {
									*value = target.id.to_string();
									ui.close();
								}
							}
						}
					});
			});
		});
	} else if option.kind == 11 {
		ui.weak("Attachment arguments are not supported yet.");
	} else {
		ui.add(
			egui::TextEdit::singleline(value)
				.hint_text(&option.description)
				.char_limit(usize::from(option.max_length.unwrap_or(6000)).min(6000))
				.desired_width(ui.available_width()),
		);
	}
}

impl crate::MessagingUi {
	pub(super) fn slash_status(&mut self, ui: &mut egui::Ui, state: &mut State, channel: Id) {
		if self.slash_commands.channel != Some(channel)
			|| self.slash_commands.generation != state.generation
		{
			return;
		}
		let Some(last) = self.slash_commands.last.as_ref() else {
			return;
		};
		let pending = state
			.interactions
			.pending
			.as_ref()
			.is_some_and(|pending| pending.message.is_none());
		let label = if pending {
			format!("Waiting for /{}…", last.name)
		} else {
			format!("Last command: /{}", last.name)
		};
		ui.horizontal(|ui| {
			ui.weak(label);
			if let Some(error) = state.interactions.error {
				ui.colored_label(design::palette(ui).danger, error);
			}
			if ui
				.add_enabled(
					!pending
						&& state.drafts.get(&channel).is_none_or(String::is_empty)
						&& state.draft_bytes()
							+ self.slash_commands.last.as_ref().unwrap().name.len()
							+ 2 <= client_core::MAX_DRAFT_BYTES,
					egui::Button::new("Edit again").small(),
				)
				.clicked()
			{
				let active = self.slash_commands.last.take().unwrap();
				state.drafts.insert(channel, format!("/{} ", active.name));
				self.draft_changes.push(channel);
				self.slash_commands.active = Some(active);
				self.slash_commands.dismissed = false;
			}
		});
	}
	pub(super) fn send_application_command(
		&mut self,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) -> bool {
		let Some(active) = self.slash_commands.active.as_ref() else {
			let Some(draft) = state
				.drafts
				.get(&channel)
				.filter(|draft| draft.starts_with('/'))
			else {
				return false;
			};
			if slash_builtin::parse(draft).is_some() {
				return false;
			}
			let name = draft[1..].split_whitespace().next().unwrap_or("");
			if state.application_commands.loading
				|| self.slash_commands.visible()
				|| state
					.application_commands
					.commands
					.iter()
					.any(|command| command.name == name)
			{
				state.status = "Choose a command from the list before running it.";
				self.slash_commands.dismissed = false;
				return true;
			}
			return false;
		};
		match state.prepare_application_command(active.id, &active.path, &active.values) {
			Ok(command) => {
				commands.push(command);
				self.clear_draft(state, channel);
				let last = self.slash_commands.active.take();
				self.slash_commands = Menu {
					channel: Some(channel),
					generation: state.generation,
					last,
					..Default::default()
				};
			}
			Err(error) => {
				self.slash_commands.error = Some(error);
				state.status = error;
			}
		}
		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn slash_picker_selects_before_sending_and_preserves_failed_fields() {
		for (width, theme) in [(1000.0, egui::Theme::Dark), (390.0, egui::Theme::Light)] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			ctx.set_theme(theme);
			let mut state = test_support::demo_state();
			let channel = state.selected.unwrap();
			let mut view = crate::MessagingUi::default();
			state.drafts.insert(channel, "/shr".into());
			let frame = |view: &mut crate::MessagingUi,
			             state: &mut State,
			             key: Option<(egui::Key, bool)>| {
				let mut commands = Vec::new();
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 720.0),
						)),
						events: key
							.map(|(key, repeat)| egui::Event::Key {
								key,
								physical_key: None,
								pressed: true,
								repeat,
								modifiers: egui::Modifiers::NONE,
							})
							.into_iter()
							.collect(),
						..Default::default()
					},
					|ui| {
						ui.add_space(610.0);
						ctx.memory_mut(|memory| {
							memory.request_focus(ui.make_persistent_id("message-input"))
						});
						view.composer(ui, state, channel, &ctx, &mut commands);
					},
				);
				output.drop_without_applying_deltas();
				ctx.input_mut(|input| input.keys_down.clear());
				commands
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, None);
			}
			let rect = view.slash_commands.rect.unwrap();
			assert!(
				rect.width() <= width
					&& rect.height() <= 340.0
					&& rect.left() >= 0.0
					&& rect.right() <= width + 1.0,
				"bounded popup: {rect:?}"
			);
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, false))).is_empty());
			assert_eq!(state.drafts[&channel], "/shrug ");
			let commands = frame(&mut view, &mut state, Some((egui::Key::Enter, false)));
			assert!(
				matches!(commands.as_slice(), [Command::Send { content, .. }] if !content.starts_with('/'))
			);
			state.auth = client_core::auth::AuthState::Authenticated;
			state.gateway_connected = true;
			let mut permissions = test_support::permission_snapshot(&state);
			for guild in &mut permissions.guilds {
				for role in guild.roles.iter_mut().flatten() {
					role.bits |= model::permissions::USE_APPLICATION_COMMANDS;
					role.bits &= !model::permissions::SEND_MESSAGES;
				}
			}
			state.permissions.replace(permissions).unwrap();
			assert!(!state.can_compose(channel));
			assert!(state.can_request_application_commands(channel));
			let app = model::application_commands::Command {
				id: Id(987),
				version: Id(1),
				application_id: Id(986),
				guild_id: None,
				kind: 1,
				name: "ask".into(),
				description: "Ask the synthetic app".into(),
				options: vec![CommandOption {
					kind: 3,
					name: "a_long_required_option_name".into(),
					description: "Enter a question".into(),
					required: true,
					..Default::default()
				}],
				contexts: None,
				integration_types: None,
				application_name: "Synthetic app".into(),
			};
			let Some(Command::ApplicationCommands { request, .. }) =
				state.request_application_commands(channel, true)
			else {
				panic!("catalog request");
			};
			state.apply_application_commands(channel, request, Ok(vec![app]));
			state.drafts.insert(channel, "/ask".into());
			for _ in 0..3 {
				frame(&mut view, &mut state, None);
			}
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, false))).is_empty());
			assert_eq!(state.drafts[&channel], "/ask ");
			assert!(view.slash_commands.active.is_some());
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, true))).is_empty());
			assert!(
				!state.interactions.busy(),
				"holding Enter must not execute a selection"
			);
			let rect = view.slash_commands.rect.unwrap();
			assert!(
				rect.right() <= width + 1.0 && rect.height() <= 340.0,
				"bounded form: {rect:?}"
			);
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, false))).is_empty());
			assert!(
				view.slash_commands.error.is_some(),
				"required blank input is rejected"
			);
			view.slash_commands.active.as_mut().unwrap().values[0].1 = "Hello".into();
			let commands = frame(&mut view, &mut state, Some((egui::Key::Enter, false)));
			assert!(matches!(commands.as_slice(), [Command::Interaction(_)]));
			assert!(
				view.slash_commands.last.is_some(),
				"keep fields for an explicit retry"
			);
			assert!(state.drafts.get(&channel).is_none_or(String::is_empty));
			state.drafts.insert(channel, "/unknown".into());
			state.application_commands.loading = true;
			let mut commands = Vec::new();
			assert!(view.send_application_command(&mut state, channel, &mut commands));
			assert!(commands.is_empty());
			assert_eq!(state.drafts[&channel], "/unknown");
			view.slash_commands.active = Some(Active {
				id: Id(999),
				path: Vec::new(),
				values: vec![("text".into(), "keep me".into())],
				name: "missing".into(),
			});
			assert!(view.send_application_command(&mut state, channel, &mut commands));
			assert_eq!(
				view.slash_commands.active.as_ref().unwrap().values[0].1,
				"keep me"
			);
			view.slash_commands.refresh(&state, Id(9999), "", false);
			assert!(view.slash_commands.active.is_none());
		}
	}
}
