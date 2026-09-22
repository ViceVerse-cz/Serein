//! Composer command discovery and session-only inline application arguments.
use crate::{avatars::Avatars, design, icons, mentions, slash_builtin};
use client_core::{Command, State};
use model::{
	Id,
	application_commands::{CommandOption, Value},
};

const RESULTS: usize = 64;
const ROW: f32 = 56.0;
const RAIL: f32 = 56.0;
const HEADING: f32 = 30.0;
const FOOTER: f32 = 26.0;
const HELP: f32 = 44.0;
const CHIP: f32 = 32.0;
const NO_PERMISSION: &str = "You don't have permission to use this command in this channel.";

#[derive(Clone)]
pub(super) struct Pick {
	id: Option<Id>,
	path: Vec<String>,
	name: String,
	description: String,
	application: String,
	application_id: Option<Id>,
	icon: Option<String>,
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
	stamp: (u64, bool, usize, bool, bool, bool),
	filter: Filter,
	items: Vec<Pick>,
	applications: std::collections::BTreeSet<Id>,
	selected: usize,
	dismissed: bool,
	enabled: bool,
	follow: bool,
	rect: Option<egui::Rect>,
	argument_rect: Option<egui::Rect>,
	focus_argument: bool,
	focused_option: usize,
	pub active: Option<Active>,
	pub error: Option<&'static str>,
	pub run: bool,
	pub retry: bool,
}

impl Menu {
	pub fn suspend(&mut self, state: &State, channel: Id) {
		if self.channel != Some(channel) || self.generation != state.generation {
			*self = Self::default();
		}
		self.enabled = false;
		self.rect = None;
		self.argument_rect = None;
		self.run = false;
		self.retry = false;
	}
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
						|| self
							.argument_rect
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
		let stamp = (
			catalog.request,
			catalog.loading,
			catalog.commands.len(),
			state.can_compose(channel),
			state.can_request_application_commands(channel),
			state.can_view(channel),
		);
		if self.query == query && self.stamp == stamp {
			return;
		}
		if self.query != query {
			self.dismissed = false;
			self.selected = 0;
			self.follow = true;
		}
		self.query = query;
		self.stamp = stamp;
		self.rebuild(state);
	}
	fn rebuild(&mut self, state: &State) {
		self.items.clear();
		self.applications.clear();
		let Some(query) = self.query.as_deref() else {
			return;
		};
		let Some(channel) = self.channel else {
			return;
		};
		if matches!(self.filter, Filter::All | Filter::Builtins) {
			for entry in slash_builtin::ALL {
				if entry.command.available(state, channel)
					&& (entry.name.contains(query)
						|| entry.description.to_lowercase().contains(query))
				{
					self.items.push(Pick {
						id: None,
						path: Vec::new(),
						name: entry.name.into(),
						description: entry.description.into(),
						application: "Built-In".into(),
						application_id: None,
						icon: None,
					});
				}
			}
		}
		for command in &state.application_commands.commands {
			if !state.can_use_application_command(channel, command) {
				continue;
			}
			self.applications.insert(command.application_id);
			if self.filter != Filter::Builtins {
				if self.items.len() >= RESULTS
					|| matches!(self.filter, Filter::Application(id) if id != command.application_id)
				{
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
							application_id: Some(command.application_id),
							icon: command.application_icon.clone(),
						});
						if self.items.len() >= RESULTS {
							break;
						}
					}
				}
			}
		}
		self.items.sort_by(|a, b| {
			(
				a.application_id.is_none(),
				&a.application,
				a.application_id,
				&a.name,
			)
				.cmp(&(
					b.application_id.is_none(),
					&b.application,
					b.application_id,
					&b.name,
				))
		});
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
				// Inline fields own Enter; choice controls must open their dropdown first.
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
		self.focus_argument = self.active.is_some();
		self.focused_option = 0;
		Some(draft.chars().count())
	}
	pub fn show(
		&mut self,
		ui: &egui::Ui,
		anchor: egui::Rect,
		state: &State,
		channel: Id,
		avatars: &mut Avatars,
	) -> Option<Pick> {
		self.rect = None;
		if !self.visible() {
			return None;
		}
		let colors = design::palette(ui);
		let bounds = ui.ctx().content_rect().shrink(8.0);
		let width = anchor.width().min(bounds.width());
		// Discord parity: a bare "/" browses by application beside an icon rail; once the
		// user types a name, one flat "commands matching" list with per-row icons takes over.
		let flat = self.query.as_deref().is_some_and(|query| !query.is_empty());
		let footer = state.application_commands.loading
			|| self.error.is_some()
			|| state.application_commands.error.is_some();
		let footer_height = if footer { FOOTER } else { 0.0 };
		let groups = self
			.items
			.iter()
			.map(|item| item.application_id)
			.collect::<std::collections::BTreeSet<_>>()
			.len();
		let height = if self.active.is_some() {
			HELP
		} else if flat {
			(HEADING + (self.items.len().clamp(1, 6) as f32) * ROW + 10.0 + footer_height)
				.min(420.0)
		} else {
			((self.items.len().clamp(2, 6) as f32) * ROW
				+ groups.min(3) as f32 * HEADING
				+ 16.0 + footer_height)
				.min(420.0)
		};
		let height = height
			.min((anchor.top() - bounds.top() - 8.0).max(HELP))
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
				let (rect, _) =
					ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
				ui.painter().rect_filled(
					rect.translate(egui::vec2(0.0, 3.0)).expand(1.0),
					10,
					egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 70 } else { 28 }),
				);
				ui.painter().rect_filled(rect, 8, colors.sidebar);
				ui.painter().rect_stroke(
					rect,
					8,
					egui::Stroke::new(1.0, colors.border),
					egui::StrokeKind::Inside,
				);
				if self.active.is_some() {
					let mut body = ui.new_child(
						egui::UiBuilder::new()
							.max_rect(rect.shrink2(egui::vec2(14.0, 0.0)))
							.layout(egui::Layout::left_to_right(egui::Align::Center)),
					);
					body.set_clip_rect(rect.shrink(1.0));
					body.spacing_mut().item_spacing.x = 8.0;
					self.help(&mut body, state, channel);
					return;
				}
				if flat {
					let content_rect = egui::Rect::from_min_max(
						rect.min + egui::vec2(6.0, 6.0),
						rect.max - egui::vec2(6.0, 4.0 + footer_height),
					);
					let mut body = ui.new_child(
						egui::UiBuilder::new()
							.id_salt("command-content")
							.max_rect(content_rect),
					);
					body.set_clip_rect(content_rect);
					body.spacing_mut().item_spacing.y = 0.0;
					let (heading, _) = body.allocate_exact_size(
						egui::vec2(body.available_width(), HEADING),
						egui::Sense::hover(),
					);
					let title = format!(
						"Commands matching /{}",
						self.query.as_deref().unwrap_or_default()
					)
					.to_uppercase();
					let mut label = body.new_child(egui::UiBuilder::new().max_rect(
						egui::Rect::from_min_max(heading.min + egui::vec2(10.0, 0.0), heading.max),
					));
					label.add(
						egui::Label::new(design::semibold(ui, title, 12.0).color(colors.muted))
							.truncate(),
					);
					let follow = std::mem::take(&mut self.follow);
					egui::ScrollArea::vertical()
						.id_salt("command-results")
						.max_height(content_rect.height() - HEADING)
						.auto_shrink([false, false])
						.show(&mut body, |ui| {
							for (index, item) in self.items.iter().enumerate() {
								let response = command_row(
									ui,
									item,
									index == self.selected,
									Some((avatars, state.demo)),
								);
								if follow && index == self.selected {
									response.scroll_to_me(None);
								}
								if response.clicked() {
									picked = Some(item.clone());
								}
							}
							if self.items.is_empty() {
								empty_row(ui);
							}
						});
					if footer {
						self.footer(ui, rect, state, channel);
					}
					return;
				}
				let rail_rect =
					egui::Rect::from_min_size(rect.min, egui::vec2(RAIL, height - footer_height));
				ui.painter().rect_filled(
					rail_rect.shrink(1.0),
					egui::CornerRadius {
						nw: 7,
						sw: if footer { 0 } else { 7 },
						..Default::default()
					},
					colors.base,
				);
				let mut rail = ui.new_child(
					egui::UiBuilder::new()
						.id_salt("command-rail")
						.max_rect(rail_rect.shrink2(egui::vec2(8.0, 8.0))),
				);
				rail.set_clip_rect(rail_rect.shrink(1.0));
				rail.spacing_mut().item_spacing.y = 6.0;
				let mut filter = self.filter;
				if rail_button(
					&mut rail,
					Filter::All,
					filter,
					None,
					"All commands",
					avatars,
					state.demo,
				)
				.clicked()
				{
					filter = Filter::All;
				}
				egui::ScrollArea::vertical()
					.id_salt("command-apps")
					.max_height((rail_rect.height() - 62.0).max(24.0))
					.auto_shrink([false, true])
					.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
					.show(&mut rail, |ui| {
						let mut seen = std::collections::BTreeSet::new();
						for command in &state.application_commands.commands {
							if !self.applications.contains(&command.application_id)
								|| !seen.insert(command.application_id)
							{
								continue;
							}
							let target = Filter::Application(command.application_id);
							if rail_button(
								ui,
								target,
								filter,
								command.application_icon.as_deref(),
								&command.application_name,
								avatars,
								state.demo,
							)
							.clicked()
							{
								filter = target;
							}
						}
						ui.separator();
						if rail_button(
							ui,
							Filter::Builtins,
							filter,
							None,
							"Built-In",
							avatars,
							state.demo,
						)
						.clicked()
						{
							filter = Filter::Builtins;
						}
					});
				if filter != self.filter {
					self.filter = filter;
					self.selected = 0;
					self.follow = true;
					self.rebuild(state);
				}
				let content_rect = egui::Rect::from_min_max(
					rect.min + egui::vec2(RAIL + 6.0, 6.0),
					rect.max - egui::vec2(6.0, 4.0 + footer_height),
				);
				let mut body = ui.new_child(
					egui::UiBuilder::new()
						.id_salt("command-content")
						.max_rect(content_rect),
				);
				body.set_clip_rect(content_rect);
				body.spacing_mut().item_spacing.y = 0.0;
				let follow = std::mem::take(&mut self.follow);
				egui::ScrollArea::vertical()
					.id_salt("command-results")
					.max_height(content_rect.height())
					.auto_shrink([false, false])
					.show(&mut body, |ui| {
						let mut previous = None;
						for (index, item) in self.items.iter().enumerate() {
							if previous != Some(item.application_id) {
								if index > 0 {
									ui.add_space(6.0);
								}
								let (heading, _) = ui.allocate_exact_size(
									egui::vec2(ui.available_width(), HEADING),
									egui::Sense::hover(),
								);
								let icon_rect = egui::Rect::from_min_size(
									heading.min + egui::vec2(10.0, 7.0),
									egui::Vec2::splat(16.0),
								);
								command_icon(
									ui,
									icon_rect,
									item.application_id
										.map_or(Filter::Builtins, Filter::Application),
									item.icon.as_deref(),
									avatars,
									state.demo,
									&item.application,
								);
								let mut label = ui.new_child(
									egui::UiBuilder::new()
										.max_rect(egui::Rect::from_min_max(
											heading.min + egui::vec2(34.0, 0.0),
											heading.max,
										))
										.layout(egui::Layout::left_to_right(egui::Align::Center)),
								);
								label.add(
									egui::Label::new(
										design::semibold(ui, item.application.to_uppercase(), 12.0)
											.color(colors.muted),
									)
									.truncate(),
								);
								previous = Some(item.application_id);
							}
							let response = command_row(ui, item, index == self.selected, None);
							if follow && index == self.selected {
								response.scroll_to_me(None);
							}
							if response.clicked() {
								picked = Some(item.clone());
							}
						}
						if self.items.is_empty() {
							empty_row(ui);
						}
					});
				if footer {
					self.footer(ui, rect, state, channel);
				}
			});
		self.rect = Some(response.response.rect);
		picked
	}
	/// Loading and failure states only; a healthy list needs no chrome below it.
	fn footer(&mut self, ui: &mut egui::Ui, rect: egui::Rect, state: &State, channel: Id) {
		let colors = design::palette(ui);
		let footer_rect = egui::Rect::from_min_max(
			egui::pos2(rect.left() + 1.0, rect.bottom() - FOOTER),
			rect.max - egui::vec2(1.0, 1.0),
		);
		ui.painter().rect_filled(
			footer_rect,
			egui::CornerRadius {
				sw: 7,
				se: 7,
				..Default::default()
			},
			colors.base,
		);
		let mut body = ui.new_child(
			egui::UiBuilder::new()
				.id_salt("command-footer")
				.max_rect(footer_rect.shrink2(egui::vec2(12.0, 0.0)))
				.layout(egui::Layout::left_to_right(egui::Align::Center)),
		);
		body.set_clip_rect(footer_rect);
		body.spacing_mut().item_spacing.x = 8.0;
		let (hint, color) = if state.application_commands.loading {
			("Loading application commands…", colors.muted)
		} else if let Some(error) = self.error.or(state.application_commands.error) {
			(error, colors.danger)
		} else {
			return;
		};
		if !state.application_commands.loading
			&& state.can_request_application_commands(channel)
			&& icons::button(
				&mut body,
				icons::Icon::Reload,
				18.0,
				"Refresh application commands",
			)
			.clicked()
		{
			self.retry = true;
		}
		body.add(egui::Label::new(egui::RichText::new(hint).size(12.0).color(color)).truncate())
			.on_hover_text(hint);
	}
	/// The active application's icon replaces the attach button, as in the official client.
	pub fn composer_badge(&self, ui: &mut egui::Ui, state: &State, avatars: &mut Avatars) -> bool {
		let Some(active) = self.active.as_ref() else {
			return false;
		};
		let Some(command) = state
			.application_commands
			.commands
			.iter()
			.find(|command| command.id == active.id)
		else {
			return false;
		};
		let (rect, response) =
			ui.allocate_exact_size(egui::Vec2::splat(28.0), egui::Sense::hover());
		command_icon(
			ui,
			rect,
			Filter::Application(command.application_id),
			command.application_icon.as_deref(),
			avatars,
			state.demo,
			&command.application_name,
		);
		response.on_hover_text(&command.application_name);
		true
	}
	pub fn can_submit(&self, state: &State, channel: Id) -> bool {
		self.active.as_ref().is_some_and(|active| {
			!state.interactions.busy()
				&& state.interactions.modal.is_none()
				&& state.application_commands_cover(channel)
				&& !state.application_commands.loading
				&& state.application_commands.error.is_none()
				&& state.application_commands.commands.iter().any(|command| {
					command.id == active.id
						&& state.can_use_application_command(channel, command)
						&& command.options_at(&active.path).is_ok_and(|options| {
							!options
								.iter()
								.any(|option| option.kind == 11 && option.required)
						})
				})
		})
	}
	/// Replace the draft editor with a command token and compact, wrapping argument chips.
	pub fn composer(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: Id,
		send_chord: &model::KeyChord,
	) -> bool {
		self.argument_rect = None;
		let submit_failed = self.error.is_some();
		let Some(active) = self.active.as_mut() else {
			return false;
		};
		let command = state
			.application_commands
			.commands
			.iter()
			.find(|command| command.id == active.id);
		let allowed =
			command.is_some_and(|command| state.can_use_application_command(channel, command));
		let options = command.and_then(|command| command.options_at(&active.path).ok());
		if allowed && let Some(options) = options {
			active
				.values
				.retain(|(name, _)| options.iter().any(|option| option.name == *name));
		}
		let colors = design::palette(ui);
		let width = ui.available_width();
		let focus_first = allowed && std::mem::take(&mut self.focus_argument);
		let mut back = false;
		let response = egui::ScrollArea::vertical()
			.id_salt(("inline-arguments", channel, active.id))
			.max_height(112.0)
			// Request room from the bottom panel; auto-shrink keeps short commands compact.
			.min_scrolled_height(112.0)
			.auto_shrink([false, true])
			.show(ui, |ui| {
				ui.horizontal_wrapped(|ui| {
					// Discord separates the command token and its argument chips by 12px.
					ui.spacing_mut().item_spacing = egui::vec2(12.0, 6.0);
					let name_id = egui::Id::unique(("slash-command", channel, active.id));
					let no_options = options.is_some_and(<[_]>::is_empty);
					let name_submit = no_options
						&& allowed && ui.memory(|memory| memory.has_focus(name_id))
						&& inline_submit(ui, send_chord);
					let mut name_job = egui::text::LayoutJob::simple_singleline(
						format!("/{}", active.name),
						egui::FontId::new(15.0, design::semibold_family(ui.ctx())),
						colors.text_strong,
					);
					name_job.wrap.max_width = (width - 12.0).max(1.0);
					name_job.wrap.max_rows = 1;
					let galley = ui.fonts_mut(|fonts| fonts.layout_job(name_job));
					let (rect, _) = ui.allocate_exact_size(
						egui::vec2((galley.size().x + 12.0).min(width), CHIP),
						egui::Sense::hover(),
					);
					let name = ui.interact(rect, name_id, egui::Sense::click());
					if name.hovered() || name.has_focus() {
						ui.painter().rect_filled(rect, 6, colors.hover);
					}
					ui.painter().galley(
						rect.min + egui::vec2(6.0, (rect.height() - galley.size().y) * 0.5),
						galley,
						colors.text_strong,
					);
					if no_options && allowed {
						if focus_first {
							name.request_focus();
						}
						self.run |= name_submit;
					}
					back = name.clicked() && !name_submit;
					name.widget_info(|| {
						egui::WidgetInfo::labeled(
							egui::Role::Button,
							true,
							format!("/{} · choose a different command", active.name),
						)
					});
					name.on_hover_text(format!("/{} · choose a different command", active.name));
					if !allowed {
						ui.add(
							egui::Label::new(
								egui::RichText::new(if command.is_some() {
									NO_PERMISSION
								} else {
									"Command unavailable. Your arguments are kept."
								})
								.color(colors.muted),
							)
							.truncate(),
						);
						return;
					}
					for (index, option) in options.unwrap_or_default().iter().enumerate() {
						if !active.values.iter().any(|(name, _)| *name == option.name) {
							active.values.push((option.name.clone(), String::new()));
						}
						let value = &mut active
							.values
							.iter_mut()
							.find(|(name, _)| *name == option.name)
							.unwrap()
							.1;
						// Two-tone chip: the option name on a lighter segment, its value on a
						// darker one; the focused chip gets a hairline ring.
						let label = option.name.as_str();
						let label_font = egui::FontId::new(14.0, design::medium_family(ui.ctx()));
						let label_width = ui
							.fonts_mut(|fonts| {
								fonts.layout_no_wrap(
									label.to_owned(),
									label_font.clone(),
									colors.text,
								)
							})
							.size()
							.x
							.min(140.0);
						// Size the value segment for what the control shows: the chosen label
						// and arrow for choice/boolean/mention controls, the text otherwise.
						let shown = if !option.choices.is_empty() || option.kind == 5 {
							option
								.choices
								.iter()
								.find(|choice| value_text(&choice.value) == *value)
								.map_or_else(
									|| {
										if value.is_empty() {
											"Choose…".chars().count()
										} else {
											value.chars().count()
										}
									},
									|choice| choice.name.chars().count(),
								) + 3
						} else if matches!(option.kind, 6..=9) {
							value.chars().count().max("ID or choose…".chars().count()) + 3
						} else {
							value.chars().count()
						};
						let value_width = (shown as f32 * 7.5 + 30.0).clamp(60.0, 220.0);
						let label_span = label_width + 20.0;
						let chip_width = (label_span + value_width).min(width);
						let id =
							egui::Id::unique(("slash-argument", channel, active.id, &option.name));
						let had_focus = ui.memory(|memory| memory.has_focus(id));
						// Discord rings the offending chip in red: a value outside its limits,
						// or a required value still missing after a submission attempt.
						let invalid = option.problem(value).is_some()
							|| (value.is_empty() && option.required && submit_failed);
						ui.allocate_ui_with_layout(
							egui::vec2(chip_width, CHIP),
							egui::Layout::left_to_right(egui::Align::Center),
							|ui| {
								let rect = egui::Rect::from_min_size(
									ui.max_rect().min,
									egui::vec2(chip_width, CHIP),
								);
								let painter = ui.painter();
								// The value sinks below the composer surface; the label sits above it.
								let value_fill = if ui.visuals().dark_mode {
									colors.base
								} else {
									colors.chat
								};
								painter.rect_filled(rect, 6, value_fill);
								painter.rect_filled(
									egui::Rect::from_min_size(
										rect.min,
										egui::vec2(label_span, CHIP),
									),
									egui::CornerRadius {
										nw: 6,
										sw: 6,
										..Default::default()
									},
									colors.hover,
								);
								if invalid || had_focus {
									painter.rect_stroke(
										rect,
										6,
										egui::Stroke::new(
											1.0,
											if invalid { colors.danger } else { colors.muted },
										),
										egui::StrokeKind::Inside,
									);
								}
								ui.set_max_width(chip_width - 8.0);
								ui.spacing_mut().item_spacing.x = 0.0;
								// Choice and mention controls sit flat on the value segment.
								let widgets = &mut ui.visuals_mut().widgets;
								widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
								widgets.inactive.bg_stroke = egui::Stroke::NONE;
								widgets.hovered.bg_stroke = egui::Stroke::NONE;
								widgets.active.bg_stroke = egui::Stroke::NONE;
								widgets.open.bg_stroke = egui::Stroke::NONE;
								ui.add_space(10.0);
								ui.add_sized(
									egui::vec2(label_width, CHIP),
									egui::Label::new(
										egui::RichText::new(label).font(label_font).color(
											if option.required {
												colors.text
											} else {
												colors.muted
											},
										),
									)
									.truncate(),
								)
								.on_hover_text(&option.description);
								ui.add_space(18.0);
								if had_focus
									&& option.choices.is_empty()
									&& option.kind != 5 && inline_submit(ui, send_chord)
								{
									self.run = true;
								}
								let response = argument(ui, option, value, state, channel, id);
								if focus_first && index == 0 {
									response.request_focus();
								}
								if response.gained_focus() || (focus_first && index == 0) {
									response.scroll_to_me(None);
								}
								if response.has_focus() || response.hovered() {
									self.focused_option = index;
								}
							},
						);
					}
				})
			});
		self.argument_rect = Some(response.inner_rect);
		if back {
			self.active = None;
			self.error = None;
			self.dismissed = false;
			self.focus_argument = false;
		}
		true
	}
	fn help(&mut self, ui: &mut egui::Ui, state: &State, channel: Id) {
		let Some(active) = self.active.as_ref() else {
			return;
		};
		let command = state
			.application_commands
			.commands
			.iter()
			.find(|command| command.id == active.id);
		let option = command
			.and_then(|command| command.options_at(&active.path).ok())
			.and_then(|options| options.get(self.focused_option));
		let colors = design::palette(ui);
		// One line, as in the official client: the focused argument's name, then its
		// description. Problems replace the description instead of stacking below it.
		let error = if command
			.is_some_and(|command| !state.can_use_application_command(channel, command))
		{
			Some(NO_PERMISSION)
		} else {
			self.error.or(state.interactions.error)
		};
		let title = option.map_or(active.name.as_str(), |option| option.name.as_str());
		ui.add(
			egui::Label::new(design::semibold(ui, title, 15.0).color(colors.text_strong))
				.truncate(),
		);
		// The focused argument's own problem beats the generic submission error.
		let problem = option.and_then(|option| {
			active
				.values
				.iter()
				.find(|(name, _)| *name == option.name)
				.and_then(|(_, value)| option.problem(value))
		});
		let error = error.filter(|_| problem.is_none());
		let (detail, color) = if let Some(problem) = problem.as_deref() {
			(problem, colors.danger)
		} else if let Some(error) = error {
			(error, colors.danger)
		} else if state.interactions.busy() {
			("Waiting for the application…", colors.muted)
		} else if option.is_some_and(|option| option.kind == 11) {
			(
				"Attachment arguments are not supported yet.",
				colors.warning,
			)
		} else {
			(
				option
					.map(|option| option.description.as_str())
					.or_else(|| command.map(|command| command.description.as_str()))
					.unwrap_or("This command is unavailable. Your arguments are kept."),
				colors.muted,
			)
		};
		let detail = if error.is_none() && option.is_some_and(|option| !option.required) {
			format!("{detail} · Optional")
		} else {
			detail.to_owned()
		};
		ui.add(egui::Label::new(egui::RichText::new(&detail).size(15.0).color(color)).truncate())
			.on_hover_text(detail);
	}
}

fn inline_submit(ui: &egui::Ui, chord: &model::KeyChord) -> bool {
	!egui::Popup::is_any_open(ui.ctx())
		&& ui.input_mut(|input| {
			!input
				.events
				.iter()
				.any(|event| matches!(event, egui::Event::Ime(_)))
				&& crate::keybinds::pressed_exact(input, chord)
		})
}

fn rail_button(
	ui: &mut egui::Ui,
	target: Filter,
	selected: Filter,
	icon: Option<&str>,
	label: &str,
	avatars: &mut Avatars,
	demo: bool,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(40.0), egui::Sense::click());
	let colors = design::palette(ui);
	if target == selected || response.hovered() {
		ui.painter().rect_filled(rect, 10, colors.hover);
	}
	if target == selected {
		ui.painter().rect_filled(
			egui::Rect::from_min_size(rect.min - egui::vec2(7.0, -10.0), egui::vec2(3.0, 20.0)),
			2,
			colors.text_strong,
		);
	}
	command_icon(ui, rect.shrink(5.0), target, icon, avatars, demo, label);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
	response.on_hover_text(label)
}

fn command_icon(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	source: Filter,
	icon: Option<&str>,
	avatars: &mut Avatars,
	demo: bool,
	label: &str,
) {
	let colors = design::palette(ui);
	match source {
		Filter::All => {
			for y in 0..2 {
				for x in 0..2 {
					let size = rect.width() * 0.32;
					let min = rect.min
						+ rect.size() * 0.1
						+ egui::vec2(x as f32, y as f32) * rect.width() * 0.48;
					ui.painter().rect_filled(
						egui::Rect::from_min_size(min, egui::Vec2::splat(size)),
						2,
						colors.text,
					);
				}
			}
		}
		Filter::Builtins => {
			ui.painter().rect_filled(rect.shrink(2.0), 3, colors.text);
			ui.painter().line_segment(
				[
					rect.min + rect.size() * egui::vec2(0.65, 0.22),
					rect.min + rect.size() * egui::vec2(0.35, 0.78),
				],
				egui::Stroke::new(rect.width() * 0.14, colors.base),
			);
		}
		Filter::Application(id) => {
			if let Some(hash) = icon {
				let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
				avatars.show_icon(
					&mut child,
					Some(format!("application-icon-{id}-{hash}")),
					rect.width(),
					demo,
					label,
				);
			} else {
				icons::paint(
					ui.painter(),
					icons::Icon::GameController,
					rect.shrink(2.0),
					colors.text,
				);
			}
		}
	}
}

fn empty_row(ui: &mut egui::Ui) {
	let (rect, _) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::hover());
	ui.painter().text(
		rect.center(),
		egui::Align2::CENTER_CENTER,
		"No commands match",
		egui::FontId::proportional(14.0),
		design::palette(ui).muted,
	);
}

/// `icon` paints the source application beside the row; grouped lists carry it in the heading.
fn command_row(
	ui: &mut egui::Ui,
	item: &Pick,
	selected: bool,
	icon: Option<(&mut Avatars, bool)>,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
	if !ui.is_rect_visible(rect) {
		return response;
	}
	if selected || response.hovered() {
		ui.painter().rect_filled(
			rect.shrink2(egui::vec2(0.0, 1.0)),
			6,
			if selected {
				colors.selected
			} else {
				colors.hover
			},
		);
	}
	let text_left = if icon.is_some() { 60.0 } else { 12.0 };
	if let Some((avatars, demo)) = icon {
		let icon_rect = egui::Rect::from_min_size(
			rect.min + egui::vec2(12.0, (ROW - 36.0) * 0.5),
			egui::Vec2::splat(36.0),
		);
		command_icon(
			ui,
			icon_rect,
			item.application_id
				.map_or(Filter::Builtins, Filter::Application),
			item.icon.as_deref(),
			avatars,
			demo,
			&item.application,
		);
	}
	let source_width = (rect.width() * 0.26).min(160.0);
	let label_width = (rect.width() - text_left - source_width - 16.0).max(32.0);
	let mut name = egui::text::LayoutJob::simple_singleline(
		format!("/{}", item.name),
		egui::FontId::new(15.0, design::semibold_family(ui.ctx())),
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
		egui::FontId::proportional(13.0),
		colors.muted,
	);
	description.wrap = egui::text::TextWrapping {
		max_width: label_width,
		max_rows: 1,
		break_anywhere: true,
		..Default::default()
	};
	ui.painter().galley(
		rect.min + egui::vec2(text_left, 9.0),
		ui.fonts_mut(|f| f.layout_job(name)),
		colors.text_strong,
	);
	ui.painter().galley(
		rect.min + egui::vec2(text_left, 30.0),
		ui.fonts_mut(|f| f.layout_job(description)),
		colors.muted,
	);
	let mut source = ui.new_child(
		egui::UiBuilder::new()
			.max_rect(egui::Rect::from_min_max(
				egui::pos2(rect.right() - source_width, rect.top()),
				rect.max - egui::vec2(12.0, 0.0),
			))
			.layout(egui::Layout::right_to_left(egui::Align::Center)),
	);
	source.add(
		egui::Label::new(
			egui::RichText::new(&item.application)
				.size(13.0)
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
	let tooltip = slash_builtin::ALL
		.iter()
		.find(|entry| item.id.is_none() && entry.name == item.name)
		.map_or_else(
			|| item.description.clone(),
			|entry| format!("{}\n{}", item.description, entry.usage),
		);
	response.on_hover_text(tooltip)
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
	id: egui::Id,
) -> egui::Response {
	if !option.choices.is_empty() || option.kind == 5 {
		let label = option
			.choices
			.iter()
			.find(|choice| value_text(&choice.value) == *value)
			.map_or(value.as_str(), |choice| choice.name.as_str());
		egui::ComboBox::from_id_salt(id)
			.width(ui.available_width())
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
			})
			.response
	} else if matches!(option.kind, 6..=9) {
		ui.horizontal(|ui| {
			let response = ui.add(
				egui::TextEdit::singleline(value)
					.id(id)
					.frame(egui::Frame::NONE)
					.hint_text("ID or choose…")
					.char_limit(22)
					.desired_width((ui.available_width() - 25.0).max(24.0)),
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
			response
		})
		.inner
	} else if option.kind == 11 {
		ui.add(egui::Label::new("Unavailable").truncate())
			.on_hover_text("Attachment arguments are not supported yet.")
	} else {
		ui.add(
			egui::TextEdit::singleline(value)
				.id(id)
				.frame(egui::Frame::NONE)
				.font(egui::FontId::proportional(14.0))
				.text_color(design::palette(ui).text_strong)
				.char_limit(usize::from(option.max_length.unwrap_or(6000)).min(6000))
				.desired_width(ui.available_width()),
		)
	}
}

impl crate::MessagingUi {
	#[cfg(feature = "demo")]
	pub fn preview_slash_command_options(&mut self, state: &mut State) {
		if !state.demo || self.slash_commands.active.is_some() {
			return;
		}
		let Some(channel) = state.selected else {
			return;
		};
		let Some(draft) = state.drafts.get(&channel) else {
			return;
		};
		self.slash_commands.refresh(state, channel, draft, true);
		let Some(name) = draft.strip_prefix('/') else {
			return;
		};
		let Some(pick) = self
			.slash_commands
			.items
			.iter()
			.find(|pick| pick.id.is_some() && pick.name == name.trim_end())
			.cloned()
		else {
			return;
		};
		let remaining = client_core::MAX_DRAFT_BYTES.saturating_sub(state.draft_bytes());
		self.slash_commands
			.accept(pick, state.drafts.get_mut(&channel).unwrap(), remaining);
		self.preview_slash_commands();
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
			let mut matching = state
				.application_commands
				.commands
				.iter()
				.filter(|command| command.name == name)
				.peekable();
			if matching.peek().is_some()
				&& !matching.any(|command| state.can_use_application_command(channel, command))
			{
				state.status = NO_PERMISSION;
				self.slash_commands.error = Some(NO_PERMISSION);
				return true;
			}
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
				self.slash_commands = Menu {
					channel: Some(channel),
					generation: state.generation,
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
						let editor = view.slash_commands.active.as_ref().map_or_else(
							|| ui.make_persistent_id("message-input"),
							|active| {
								state
									.application_commands
									.commands
									.iter()
									.find(|command| command.id == active.id)
									.and_then(|command| command.options_at(&active.path).ok())
									.and_then(|options| options.first())
									.map_or_else(
										|| egui::Id::unique(("slash-command", channel, active.id)),
										|option| {
											egui::Id::unique((
												"slash-argument",
												channel,
												active.id,
												&option.name,
											))
										},
									)
							},
						);
						ctx.memory_mut(|memory| memory.request_focus(editor));
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
					&& rect.height() <= 420.0
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
			assert!(!state.can_request_application_commands(channel));
			for source in [
				"/ask",
				"/msg @user hi",
				"/gif cats",
				"/sticker cats",
				"/shrug hello",
			] {
				state.drafts.insert(channel, source.into());
				assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, false))).is_empty());
				assert_eq!(state.drafts[&channel], source);
				assert!(!view.slash_commands.visible());
			}
			let mut permissions = test_support::permission_snapshot(&state);
			for guild in &mut permissions.guilds {
				for role in guild.roles.iter_mut().flatten() {
					role.bits |= model::permissions::USE_APPLICATION_COMMANDS
						| model::permissions::SEND_MESSAGES;
				}
			}
			state.permissions.replace(permissions).unwrap();
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
				application_icon: None,
				default_member_permissions: None,
				permissions: Default::default(),
				application_permissions: Default::default(),
			};
			let Some(Command::ApplicationCommands { request, .. }) =
				state.request_application_commands(channel, true)
			else {
				panic!("catalog request");
			};
			let mut denied = app.clone();
			denied.id = Id(988);
			denied.application_id = Id(989);
			denied.name = "restricted".into();
			denied.permissions.user = Some(false);
			state.apply_application_commands(channel, request, Ok(vec![app, denied]));
			state.drafts.insert(channel, "/".into());
			for _ in 0..3 {
				frame(&mut view, &mut state, None);
			}
			assert_eq!(
				view.slash_commands
					.items
					.iter()
					.filter(|item| item.application_id.is_some())
					.map(|item| item.name.as_str())
					.collect::<Vec<_>>(),
				vec!["ask"],
				"hide denied apps"
			);
			assert_eq!(
				view.slash_commands
					.applications
					.iter()
					.copied()
					.collect::<Vec<_>>(),
				vec![Id(986)]
			);
			state.drafts.insert(channel, "/restricted".into());
			frame(&mut view, &mut state, None);
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, false))).is_empty());
			assert_eq!(state.drafts[&channel], "/restricted");
			assert_eq!(state.status, NO_PERMISSION);
			assert!(!view.emoji_picker.is_open());
			state.drafts.insert(channel, "/ask".into());
			for _ in 0..3 {
				frame(&mut view, &mut state, None);
			}
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, false))).is_empty());
			assert_eq!(state.drafts[&channel], "/ask ");
			assert!(view.slash_commands.active.is_some());
			assert!(
				view.slash_commands.can_submit(&state, channel),
				"message and application permissions enable the normal Send action"
			);
			ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key: egui::Key::Enter,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
					..Default::default()
				},
				|_| {
					assert!(view.slash_commands.keys(&ctx).is_none());
					assert!(!view.slash_commands.run, "field controls must retain Enter");
					assert!(ctx.input(|input| input.key_pressed(egui::Key::Enter)));
				},
			)
			.drop_without_applying_deltas();
			ctx.input_mut(|input| input.keys_down.clear());
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, true))).is_empty());
			assert!(
				!state.interactions.busy(),
				"holding Enter must not execute a selection"
			);
			let rect = view.slash_commands.rect.unwrap();
			assert!(
				rect.right() <= width + 1.0 && rect.height() <= 90.0,
				"compact contextual help: {rect:?}"
			);
			let inline = view.slash_commands.argument_rect.unwrap();
			assert!(inline.width() <= width && inline.height() <= 112.0);
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, false))).is_empty());
			assert!(
				view.slash_commands.error.is_some(),
				"required blank input is rejected"
			);
			view.slash_commands.active.as_mut().unwrap().values[0].1 = "Hello".into();
			state.application_commands.commands[0].permissions.user = Some(false);
			state.application_commands.request += 1;
			frame(&mut view, &mut state, None);
			assert!(view.slash_commands.items.is_empty());
			assert!(view.slash_commands.applications.is_empty());
			assert!(!view.slash_commands.can_submit(&state, channel));
			assert!(frame(&mut view, &mut state, Some((egui::Key::Enter, false))).is_empty());
			assert_eq!(Some(state.status), view.slash_commands.error);
			assert_eq!(state.drafts[&channel], "/ask ");
			assert_eq!(
				view.slash_commands.active.as_ref().unwrap().values[0].1,
				"Hello"
			);
			view.slash_commands.suspend(&state, channel);
			assert!(!view.slash_commands.visible());
			assert_eq!(
				view.slash_commands.active.as_ref().unwrap().values[0].1,
				"Hello"
			);
			state.application_commands.commands[0].permissions.user = Some(true);
			state.application_commands.request += 1;
			frame(&mut view, &mut state, None);
			assert!(
				view.slash_commands
					.items
					.iter()
					.any(|item| item.id == Some(Id(987)))
			);
			assert!(view.slash_commands.applications.contains(&Id(986)));
			let commands = frame(&mut view, &mut state, Some((egui::Key::Enter, false)));
			assert!(matches!(commands.as_slice(), [Command::Interaction(_)]));
			assert!(
				view.slash_commands.active.is_none(),
				"a sent command leaves no sticky status behind"
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
			state.application_commands.loading = false;
			state.interactions = Default::default();
			state.application_commands.commands[0].options = (0..25)
				.map(|index| CommandOption {
					kind: 3,
					name: format!("option_{index}"),
					description: "Synthetic argument".into(),
					..Default::default()
				})
				.collect();
			state.drafts.insert(channel, "/ask ".into());
			view.slash_commands.active = Some(Active {
				id: Id(987),
				name: "ask".into(),
				path: vec![],
				values: vec![],
			});
			for _ in 0..3 {
				frame(&mut view, &mut state, None);
			}
			let inline = view.slash_commands.argument_rect.unwrap();
			assert!(
				inline.width() <= width && (100.0..=112.0).contains(&inline.height()),
				"25 options stay inside the composer: {inline:?}"
			);
			state.application_commands.commands[0].options.clear();
			view.slash_commands.focus_argument = true;
			frame(&mut view, &mut state, None);
			assert!(view.slash_commands.can_submit(&state, channel));
			assert!(
				matches!(
					frame(&mut view, &mut state, Some((egui::Key::Enter, false))).as_slice(),
					[Command::Interaction(_)]
				),
				"a command without options accepts Send from its focused token"
			);
			view.slash_commands.suspend(&state, Id(9999));
			assert!(view.slash_commands.active.is_none());
		}
	}
}
