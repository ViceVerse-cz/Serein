use crate::{design, icons};
use client_core::{Command, State};
use egui::RichText;
use model::Id;

/// Width of the results pane when the window is wide enough to keep the timeline readable.
pub const PANE_WIDTH: f32 = 420.0;
const FILTER_WIDTH: f32 = 444.0;
#[path = "search_filters.rs"]
mod filters;

fn date_id(value: &str) -> Result<Option<u64>, ()> {
	if value.is_empty() {
		return Ok(None);
	}
	let format =
		time::format_description::parse_borrowed::<2>("[year]-[month]-[day]").map_err(|_| ())?;
	let date = time::Date::parse(value, &format).map_err(|_| ())?;
	let millis = date.midnight().assume_utc().unix_timestamp_nanos() / 1_000_000;
	let delta = u64::try_from(millis - 1_420_070_400_000).map_err(|_| ())?;
	if delta == 0 || delta > (u64::MAX >> 22) {
		return Err(());
	}
	Ok(Some(delta << 22))
}

#[derive(Default)]
pub struct SearchUi {
	pub open: bool,
	pins: bool,
	query: String,
	channel: Option<Id>,
	focus: bool,
	composing: bool,
	ime_frame: bool,
	pending_submit: bool,
	filters_open: bool,
	oldest_first: bool,
	hide_highlight: bool,
	filter_draft: Option<filters::Draft>,
	search_anchor: Option<egui::Rect>,
	suggestion_index: usize,
	formats: crate::markdown::FormatCache,
	pub opening: Option<String>,
	pub profile: Option<model::User>,
	pub channel_reference: Option<Id>,
}

impl SearchUi {
	pub fn toggle(&mut self, pins: bool) -> bool {
		self.open = !self.open || self.pins != pins;
		self.pins = pins;
		self.focus = self.open;
		self.filters_open = self.open && !pins;
		self.open
	}
	/// Fixture-only: open a text search for `query` and submit it on the next frame.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview(&mut self, query: &str) {
		self.open = true;
		self.pins = false;
		self.query = query.to_owned();
		self.pending_submit = true;
		self.filters_open = filters::active_user_token(query).is_some();
		self.focus = self.filters_open;
	}
	/// Fixture-only: open the pins popout and request the first page on the next frame.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_pins(&mut self) {
		self.open = true;
		self.pins = true;
		self.pending_submit = true;
	}
	/// True while the pane shows pinned messages rather than query results.
	pub fn pins(&self) -> bool {
		self.pins
	}
	/// Per-frame bookkeeping: Escape closes, navigation resets and closed views cancel requests.
	pub fn sync(&mut self, ctx: &egui::Context, state: &mut State, commands: &mut Vec<Command>) {
		if self.open
			&& self.filter_draft.is_none()
			&& ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
		{
			self.open = false;
			self.filters_open = false;
		}
		if self.channel != state.selected {
			self.channel = state.selected;
			// A fixture preview adopts the initial selection instead of closing.
			if !self.pending_submit {
				self.query.clear();
				self.filter_draft = None;
				self.open = false;
			}
		}
		if !self.open {
			self.formats.retain(|_| false);
			self.filters_open = false;
			self.filter_draft = None;
			if state.search.is_some() {
				commands.push(state.clear_search());
			}
			return;
		}
		if state
			.search
			.as_ref()
			.is_some_and(|view| view.pins != self.pins)
			|| (!state.can_search() && state.search.is_some())
		{
			commands.push(state.clear_search());
		}
		self.formats.retain(|id| {
			state
				.search
				.as_ref()
				.and_then(|view| view.page.as_ref())
				.is_some_and(|page| page.hits.iter().any(|hit| hit.id == id))
		});
		self.ime_frame = self.composing;
		ctx.input(|i| {
			for event in &i.events {
				if let egui::Event::Ime(event) = event {
					self.ime_frame = true;
					self.composing =
						matches!(event, egui::ImeEvent::Preedit { text,.. } if !text.is_empty());
				}
			}
		});
	}
	/// Query field shown in the conversation header while a text search is open.
	pub fn header_input(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let allowed = state.can_search();
		let mut submit = false;
		let frame = egui::Frame::new()
			.fill(colors.base)
			.corner_radius(6)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.inner_margin(egui::Margin::symmetric(8, 0))
			.show(ui, |ui| {
				ui.set_height(28.0);
				ui.horizontal_centered(|ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					let output = egui::TextEdit::singleline(&mut self.query)
						.char_limit(256)
						.frame(egui::Frame::NONE)
						.hint_text("Search")
						.desired_width((ui.available_width() - 28.0).max(30.0))
						.show(ui);
					let input = output.response;
					input.widget_info(|| {
						egui::WidgetInfo::labeled(
							egui::WidgetType::TextEdit,
							true,
							"Search messages in this conversation",
						)
					});
					if self.focus {
						input.request_focus();
						self.focus = false;
						// Selecting a filter appends to the query; keep the caret at the end
						// instead of leaving it at its stale position from before the change.
						let mut cursor_state = output.state;
						let end = egui::text::CCursor::new(self.query.chars().count());
						cursor_state
							.cursor
							.set_char_range(Some(egui::text::CCursorRange::one(end)));
						cursor_state.store(ui.ctx(), input.id);
					}
					if input.clicked() || input.changed() {
						self.filters_open = true;
						self.suggestion_index = 0;
					}
					let valid = allowed && model::search_terms(&self.query).is_ok();
					submit = valid
						&& input.lost_focus()
						&& ui.input(|i| i.key_pressed(egui::Key::Enter))
						&& !self.ime_frame;
					if icons::button(ui, icons::Icon::Close, 22.0, "Close search").clicked() {
						self.open = false;
						self.filters_open = false;
					}
				});
			});
		self.search_anchor = Some(frame.response.rect);
		if submit && let Some(command) = state.request_search(self.query.trim().into(), None) {
			commands.push(command);
			self.filters_open = false;
		}
	}
	pub fn results_visible(&self, state: &State) -> bool {
		self.open && !self.pins && (self.pending_submit || state.search.is_some())
	}
	fn open_filters(&mut self) {
		self.filter_draft = Some(filters::Draft::new(&self.query));
		self.filters_open = false;
	}
	pub fn overlays(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		avatars: &mut crate::avatars::Avatars,
		commands: &mut Vec<Command>,
	) {
		if !self.open || self.pins {
			return;
		}
		let mut submit = false;
		if let Some(draft) = &mut self.filter_draft {
			match draft.show(ctx, state, avatars) {
				filters::Action::Apply(query) => {
					self.query = query;
					self.filter_draft = None;
					submit = true;
				}
				filters::Action::Cancel => self.filter_draft = None,
				filters::Action::None => {}
			}
		} else if self.filters_open
			&& let Some(anchor) = self.search_anchor
		{
			let colors = design::palette_for(ctx);
			let bounds = ctx.content_rect().shrink(8.0);
			let width = FILTER_WIDTH.min(bounds.width());
			let popup = egui::Area::new(egui::Id::unique("search-filters"))
				.order(egui::Order::Foreground)
				.fixed_pos(egui::pos2(
					(anchor.right() - width).max(bounds.left()),
					anchor.bottom() + 8.0,
				))
				.constrain_to(bounds)
				.show(ctx, |ui| {
					egui::Frame::new()
						.fill(colors.base)
						.stroke(egui::Stroke::new(1.0, colors.border))
						.corner_radius(8)
						.inner_margin(10)
						.show(ui, |ui| {
							ui.set_width((width - 20.0).max(1.0));
							ui.spacing_mut().item_spacing.y = 4.0;
							if let Some((start, key, typed)) =
								filters::active_user_token(&self.query)
							{
								let key = key.to_owned();
								let users = filters::users(state);
								let matching: Vec<_> = users
									.iter()
									.copied()
									.filter(|user| {
										user.name.to_lowercase().contains(&typed.to_lowercase())
											|| user.id.to_string() == typed
									})
									.collect();
								ui.label(
									design::semibold(
										ui,
										if key == "from" {
											"From User"
										} else {
											"Mentions User"
										},
										13.0,
									)
									.color(colors.muted),
								);
								let down = ctx.input_mut(|i| {
									i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
								});
								let up = ctx.input_mut(|i| {
									i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
								});
								if down {
									self.suggestion_index = self.suggestion_index.saturating_add(1);
								}
								if up {
									self.suggestion_index = self.suggestion_index.saturating_sub(1);
								}
								self.suggestion_index =
									self.suggestion_index.min(matching.len().saturating_sub(1));
								let enter = !self.ime_frame
									&& ctx.input_mut(|i| {
										i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
									});
								let mut chosen = None;
								egui::ScrollArea::vertical()
									.max_height(260.0)
									.show(ui, |ui| {
										for (index, user) in matching.iter().enumerate() {
											let row = filters::user_row(
												ui,
												user,
												avatars,
												state.demo,
												index == self.suggestion_index,
											);
											if index == self.suggestion_index && (up || down) {
												row.scroll_to_me(None);
											}
											if row.clicked()
												|| (enter && index == self.suggestion_index)
											{
												chosen = Some(user.id);
											}
										}
										if matching.is_empty() {
											ui.label("No matching users in this conversation.");
										}
									});
								if let Some(id) = chosen {
									let query = format!("{}{key}:{id} ", &self.query[..start]);
									if model::search_terms(&query).is_ok() {
										self.query = query;
										self.filters_open = false;
										self.focus = true;
									}
								}
							} else {
								submit = ui
									.add_enabled_ui(
										state.can_search()
											&& model::search_terms(&self.query).is_ok(),
										|ui| {
											filters::suggestion_row(
												ui,
												"search",
												&format!("Search for {}", self.query),
												"",
											)
										},
									)
									.inner
									.clicked();
								ui.separator();
								ui.add_space(6.0);
								ui.horizontal(|ui| {
									ui.add_space(10.0);
									ui.label(
										design::semibold(ui, "Filters", 13.0).color(colors.muted),
									);
								});
								for (title, detail, key) in [
									("From a specific user", "from: user", "from"),
									(
										"Includes a specific type of data",
										"has: link, embed or file",
										"has",
									),
									("Mentions a specific user", "mentions: user", "mentions"),
									("More filters", "dates, author type, and more", ""),
								] {
									let row = filters::suggestion_row(ui, key, title, detail);
									if row.clicked() {
										if key == "from" || key == "mentions" {
											let query = format!("{} {key}:", self.query.trim());
											if query.len() <= 1024 && query.chars().count() <= 256 {
												self.query = query.trim_start().to_owned();
												self.focus = true;
											}
										} else {
											self.open_filters();
										}
									}
								}
							}
						});
				});
			let outside = ctx.input(|i| {
				i.pointer.any_pressed()
					&& i.pointer
						.interact_pos()
						.is_some_and(|p| !anchor.contains(p) && !popup.response.rect.contains(p))
			});
			if outside {
				self.filters_open = false;
			}
		}
		if submit {
			if self.query.is_empty() {
				commands.push(state.clear_search());
			} else if let Some(command) = state.request_search(self.query.trim().to_owned(), None) {
				commands.push(command);
			}
			self.filters_open = false;
		}
	}
	/// Anchored pinned-messages popout under the header pin button, like Discord's.
	pub fn pins_popout(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		anchor: egui::Rect,
		dm: bool,
		commands: &mut Vec<Command>,
	) {
		if !(self.open && self.pins) {
			return;
		}
		if std::mem::take(&mut self.pending_submit)
			&& let Some(command) = state.request_pins()
		{
			commands.push(command);
		}
		let colors = design::palette_for(ctx);
		let bounds = ctx.content_rect().shrink(8.0);
		let width = PANE_WIDTH.min(bounds.width());
		let max_height = (bounds.height() * 0.7).clamp(240.0, 560.0);
		let x = (anchor.right() - width)
			.min(bounds.right() - width)
			.max(bounds.left());
		let y = (anchor.bottom() + 8.0).min(bounds.bottom() - 120.0);
		let area = egui::Area::new(egui::Id::unique("pins-popout"))
			.kind(egui::UiKind::Popup)
			.order(egui::Order::Foreground)
			.fixed_pos(egui::pos2(x, y))
			.constrain_to(bounds)
			.interactable(true)
			.show(ctx, |ui| {
				egui::Frame::new()
					.fill(colors.sidebar)
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(8)
					.shadow(egui::epaint::Shadow {
						offset: [0, 8],
						blur: 24,
						spread: 0,
						color: egui::Color32::from_black_alpha(96),
					})
					.show(ui, |ui| {
						ui.set_width(width);
						ui.set_max_height(max_height);
						ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
						// Header.
						egui::Frame::new()
							.fill(colors.base)
							.corner_radius(egui::CornerRadius {
								nw: 8,
								ne: 8,
								..Default::default()
							})
							.inner_margin(egui::Margin::symmetric(16, 0))
							.show(ui, |ui| {
								ui.set_width(ui.available_width());
								ui.set_height(48.0);
								ui.horizontal_centered(|ui| {
									ui.spacing_mut().item_spacing.x = 8.0;
									icons::inline(ui, icons::Icon::Pin, 20.0, colors.muted);
									ui.label(
										design::semibold(ui, "Pinned Messages", 16.0)
											.color(colors.text_strong),
									);
									ui.with_layout(
										egui::Layout::right_to_left(egui::Align::Center),
										|ui| {
											if icons::button(ui, icons::Icon::Close, 28.0, "Close")
												.clicked()
											{
												self.open = false;
											}
											let reload = ui
												.add_enabled_ui(state.can_search(), |ui| {
													icons::button(
														ui,
														icons::Icon::Reload,
														28.0,
														"Reload pins",
													)
												})
												.inner;
											if self.focus {
												reload.request_focus();
												self.focus = false;
											}
											if reload.clicked()
												&& let Some(command) = state.request_pins()
											{
												commands.push(command);
											}
										},
									);
								});
							});
						ui.painter().hline(
							ui.max_rect().x_range(),
							ui.cursor().top(),
							egui::Stroke::new(1.0, colors.border),
						);
						let view = state.search.as_ref().filter(|view| view.pins);
						let empty = view.map_or(state.can_search(), |view| {
							view.page.as_ref().is_some_and(|page| {
								page.hits
									.iter()
									.all(|hit| !state.is_pinned(hit.channel, hit.id))
							})
						});
						if empty {
							Self::pins_empty(ui, dm);
						} else {
							egui::Frame::new()
								.inner_margin(egui::Margin::symmetric(12, 12))
								.show(ui, |ui| {
									ui.set_width(ui.available_width());
									self.pins_content(ui, state, commands);
								});
						}
					});
			});
		let clicked_outside = ctx.input(|i| i.pointer.any_pressed())
			&& !area.response.rect.contains(
				ctx.input(|i| i.pointer.interact_pos())
					.unwrap_or(area.response.rect.center()),
			) && !anchor.contains(
			ctx.input(|i| i.pointer.interact_pos())
				.unwrap_or(anchor.center()),
		);
		if clicked_outside {
			self.open = false;
		}
	}
	fn pins_empty(ui: &mut egui::Ui, dm: bool) {
		let colors = design::palette(ui);
		egui::Frame::new()
			.inner_margin(egui::Margin::symmetric(24, 36))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.vertical_centered(|ui| {
					let (rect, _) =
						ui.allocate_exact_size(egui::Vec2::splat(64.0), egui::Sense::hover());
					let face = colors.muted.gamma_multiply(0.35);
					ui.painter().circle_filled(rect.center(), 32.0, face);
					icons::paint(
						ui.painter(),
						icons::Icon::Pin,
						rect.shrink(16.0),
						colors.text,
					);
					ui.add_space(20.0);
					ui.label(
						design::medium(
							ui,
							if dm {
								"This direct message doesn't have\nany pinned messages… yet."
							} else {
								"This channel doesn't have\nany pinned messages… yet."
							},
							15.0,
						)
						.color(colors.text_strong),
					);
				});
			});
	}
	/// Pinned message cards, load-more control and status lines.
	fn pins_content(&mut self, ui: &mut egui::Ui, state: &mut State, commands: &mut Vec<Command>) {
		let colors = design::palette(ui);
		let allowed = state.can_search();
		let mut older_pins = false;
		let mut target = None;
		ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
		if !allowed {
			ui.label(
				RichText::new(
					"Pinned messages are unavailable while disconnected or without channel access.",
				)
				.small()
				.color(colors.muted),
			);
		}
		if let Some(view) = state.search.as_ref().filter(|view| view.pins) {
			if view.loading {
				ui.label(
					RichText::new(if view.pin_before.is_some() {
						"Loading older pins…"
					} else {
						"Loading pinned messages…"
					})
					.small()
					.color(colors.muted),
				);
			}
			if let Some(error) = view.error {
				ui.label(RichText::new(error).color(colors.danger));
			}
			let retry = view.error.is_some() && view.pin_before.is_some();
			if retry
				|| view
					.page
					.as_ref()
					.is_some_and(|page| page.pin_cursor.is_some())
			{
				older_pins = ui
					.push_id("older-pins", |ui| {
						ui.add_enabled(
							allowed && !view.loading,
							egui::Button::new(if retry {
								"Retry older pins"
							} else {
								"Older pins"
							}),
						)
					})
					.inner
					.clicked();
			}
			if let Some(page) = &view.page {
				egui::ScrollArea::vertical()
					.id_salt(("pins", view.request))
					.auto_shrink([false, true])
					.show(ui, |ui| {
						ui.spacing_mut().item_spacing.y = 8.0;
						for hit in &page.hits {
							if view.pins && !state.is_pinned(hit.channel, hit.id) {
								continue;
							}
							ui.push_id(hit.id, |ui| {
								Self::hit_card(ui, hit, allowed, &mut target);
							});
						}
					});
				if page.pin_cursor.is_none() && !view.loading && page.partial {
					ui.label(
						RichText::new(
							"More pins may exist, but this page has no usable continuation.",
						)
						.small()
						.color(colors.muted),
					);
				}
			}
		}
		if older_pins && let Some(command) = state.request_older_pins() {
			commands.push(command);
		}
		if let Some(target) = target
			&& let Some(command) = state.open_search_hit(target)
		{
			commands.push(command);
			self.open = false;
		}
	}
	fn hit_card(ui: &mut egui::Ui, hit: &model::SearchHit, allowed: bool, target: &mut Option<Id>) {
		let colors = design::palette(ui);
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(8)
			.inner_margin(egui::Margin::same(10))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal(|ui| {
					ui.label(design::semibold(ui, &hit.author, 14.0).color(colors.text_strong));
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						if ui
							.add_enabled(
								allowed && hit.id.0 < u64::MAX,
								egui::Button::new(RichText::new("Jump").size(12.0)),
							)
							.on_hover_text("Open this message in the timeline")
							.clicked()
						{
							*target = Some(hit.id);
						}
					});
				});
				ui.add(
					egui::Label::new(RichText::new(&hit.excerpt).color(colors.text))
						.wrap()
						.selectable(true),
				);
			});
	}
	/// Results pane rendered where the member list normally lives.
	pub fn pane(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		avatars: &mut crate::avatars::Avatars,
	) {
		let colors = design::palette(ui);
		let allowed = state.can_search();
		let mut submit = std::mem::take(&mut self.pending_submit) && !self.pins;
		let mut older = None;
		let mut target = None;
		ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
		ui.horizontal(|ui| {
			let title = if self.pins {
				"Pinned Messages".to_owned()
			} else {
				match state.search.as_ref().and_then(|view| view.page.as_ref()) {
					Some(page) if !state.search.as_ref().is_some_and(|v| v.loading) => {
						format!(
							"{} Result{}",
							page.total,
							if page.total == 1 { "" } else { "s" }
						)
					}
					_ => "Search".to_owned(),
				}
			};
			ui.label(design::semibold(ui, title, 16.0).color(colors.text_strong));
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				if !self.pins {
					ui.menu_button("⚙", |ui| {
						ui.checkbox(&mut self.hide_highlight, "Hide matching-text highlight");
					})
					.response
					.on_hover_text("Search settings");
					ui.menu_button("↕ Sort", |ui| {
						ui.label("Order on this results page");
						ui.radio_value(&mut self.oldest_first, false, "Newest first");
						ui.radio_value(&mut self.oldest_first, true, "Oldest first");
					});
					if ui.button("Filters").clicked() {
						self.open_filters();
					}
				}
				if self.pins && icons::button(ui, icons::Icon::Close, 28.0, "Close").clicked() {
					self.open = false;
				}
				if self.pins {
					let reload = ui.add_enabled(allowed, egui::Button::new("Reload pins"));
					if self.focus {
						reload.request_focus();
						self.focus = false;
					}
					submit = reload.clicked();
				}
			});
		});
		ui.separator();
		if !self.pins {
			ui.horizontal(|ui| {
				if let Some(view) = &state.search
					&& let Some(page) = &view.page
				{
					if let Some(last) = page.hits.last()
						&& (page.total > page.hits.len() as u64 || page.partial)
						&& ui
							.add_enabled(allowed && !view.loading, egui::Button::new("Older"))
							.clicked()
					{
						older = Some((view.query.clone(), Some(last.id)));
					}
					if view.before.is_some()
						&& ui
							.add_enabled(allowed && !view.loading, egui::Button::new("Newest"))
							.clicked()
					{
						older = Some((view.query.clone(), None));
					}
				}
			});
		}
		if self.pins {
			if submit && let Some(command) = state.request_pins() {
				commands.push(command);
			}
			self.pins_content(ui, state, commands);
			return;
		}
		if !allowed {
			ui.label(
				RichText::new(
					"Messages are unavailable while disconnected or without channel access.",
				)
				.small()
				.color(colors.muted),
			);
		}
		if state.search.is_none() {
			ui.label(
				RichText::new("Type a query above and press Enter.")
					.small()
					.color(colors.muted),
			);
		}
		if let Some(view) = &state.search {
			if view.loading {
				ui.label(RichText::new("Searching…").small().color(colors.muted));
			}
			if let Some(error) = view.error {
				ui.label(RichText::new(error).color(colors.danger));
			}
			if let Some(page) = &view.page {
				if page.partial {
					ui.label(
						RichText::new("Indexing is incomplete; results may be missing.")
							.small()
							.color(colors.muted),
					);
				}
				if page.hits.is_empty() {
					ui.label(
						RichText::new("No matching messages in this page.").color(colors.muted),
					);
				}
				let content_query = model::search_terms(&view.query)
					.map(|(content, _)| content)
					.unwrap_or_default();
				egui::ScrollArea::vertical()
					.id_salt(("search-results", view.request))
					.auto_shrink([false, false])
					.show(ui, |ui| {
						ui.spacing_mut().item_spacing.y = 8.0;
						for index in 0..page.hits.len() {
							let hit = &page.hits[if self.oldest_first {
								page.hits.len() - 1 - index
							} else {
								index
							}];
							ui.push_id(hit.id, |ui| {
								self.result_card(
									ui,
									state,
									hit,
									if self.hide_highlight {
										""
									} else {
										&content_query
									},
									avatars,
									&mut target,
								);
							});
						}
					});
			}
		}
		if submit && let Some(command) = state.request_search(self.query.trim().into(), None) {
			commands.push(command);
		}
		if let Some((query, before)) = older
			&& let Some(command) = state.request_search(query, before)
		{
			commands.push(command);
		}
		if let Some(target) = target
			&& let Some(command) = state.open_search_hit(target)
		{
			commands.push(command);
			self.open = false;
		}
	}
	fn result_card(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		hit: &model::SearchHit,
		query: &str,
		avatars: &mut crate::avatars::Avatars,
		target: &mut Option<Id>,
	) {
		let colors = design::palette(ui);
		if let Some(channel) = state.channels.iter().find(|c| c.id == hit.channel) {
			ui.label(
				design::semibold(
					ui,
					format!(
						"{} {}",
						if channel.guild.is_none() { "@" } else { "#" },
						channel.name
					),
					14.0,
				)
				.color(colors.text_strong),
			);
		}
		egui::Frame::new()
			.fill(colors.base)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(10)
			.inner_margin(12)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal_top(|ui| {
					let (avatar, _) =
						ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::hover());
					ui.painter()
						.circle_filled(avatar.center(), 18.0, colors.accent);
					ui.painter().text(
						avatar.center(),
						egui::Align2::CENTER_CENTER,
						hit.author.chars().next().unwrap_or('?'),
						egui::FontId::proportional(16.0),
						egui::Color32::WHITE,
					);
					ui.vertical(|ui| {
						ui.set_width(ui.available_width());
						ui.horizontal_wrapped(|ui| {
							ui.label(
								design::semibold(ui, &hit.author, 15.0).color(colors.text_strong),
							);
							if ui
								.add_enabled(
									state.can_search() && hit.id.0 < u64::MAX,
									egui::Button::new("Jump"),
								)
								.clicked()
							{
								*target = Some(hit.id);
							}
							// Fixture IDs do not encode a real creation timestamp.
							if hit.id.0 >= (1 << 22) {
								let seconds = ((hit.id.0 >> 22) + 1_420_070_400_000) / 1000;
								if let Ok(utc) =
									time::OffsetDateTime::from_unix_timestamp(seconds as i64)
								{
									let local = crate::local_time::local(utc);
									ui.label(
										RichText::new(format!(
											"{:02}:{:02}",
											local.hour(),
											local.minute()
										))
										.size(11.0)
										.color(colors.muted),
									);
								}
							}
						});
						let id = ui.scope_id().with(("search-spoilers", &hit.excerpt));
						let mut revealed = ui.data(|data| data.get_temp::<u32>(id).unwrap_or(0));
						let mut surface = crate::select::Surface::new(ui, "search-result");
						self.formats.get(hit.id, &hit.excerpt).show_search(
							ui,
							&mut self.opening,
							&crate::mentions::known_users(state, hit.channel),
							&mut self.profile,
							(
								&state.channels,
								&mut self.channel_reference,
								&state.guilds,
								crate::mentions::known_roles(state, hit.channel),
							),
							(avatars, state.demo, &mut revealed),
							&mut surface,
							query,
						);
						surface.finish(ui);
						if revealed != 0 {
							ui.data_mut(|data| data.insert_temp(id, revealed));
						}
					});
				});
			});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn run(ui: &mut egui::Ui, view: &mut SearchUi, state: &mut State, commands: &mut Vec<Command>) {
		view.sync(ui.ctx(), state, commands);
		if view.open {
			if !view.pins {
				view.header_input(ui, state, commands);
			}
			view.pane(ui, state, commands, &mut crate::avatars::Avatars::default());
		}
	}
	#[test]
	fn pins_reload_is_keyboard_operable_and_close_cancels_at_narrow_width() {
		for dark in [false, true] {
			let mut state = State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				selected: Some(Id(1)),
				channels: vec![model::Channel {
					id: Id(1),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Synthetic".into(),
					kind: 3,
					recipients: vec![],
					member_list_id: None,
					message_count: None,
					icon: None,
					last_message: None,
				}],
				..State::default()
			};
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut view = SearchUi {
				channel: Some(Id(1)),
				..SearchUi::default()
			};
			assert!(view.toggle(true));
			let mut commands = Vec::new();
			for frame in 0..3 {
				let events = if frame == 2 {
					vec![egui::Event::Key {
						key: egui::Key::Enter,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}]
				} else {
					vec![]
				};
				let mut output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(420.0, 480.0),
						)),
						events,
						..Default::default()
					},
					|ui| run(ui, &mut view, &mut state, &mut commands),
				);
				assert!(output.platform_output.commands.is_empty());
				output.textures_delta.clear();
				if frame < 2 {
					assert!(commands.is_empty());
				}
			}
			assert_eq!(
				commands
					.iter()
					.filter(|c| matches!(c, Command::Pins { .. }))
					.count(),
				1
			);
			assert!(state.search.as_ref().unwrap().pins);
			let mut output = ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key: egui::Key::Escape,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
					..Default::default()
				},
				|ui| run(ui, &mut view, &mut state, &mut commands),
			);
			output.textures_delta.clear();
			assert!(!view.open);
			assert!(state.search.is_none());
			assert!(matches!(commands.last(), Some(Command::CancelSearch)));
			let cursor = 1_700_000_000_000_000_000i128;
			for (loading, continuation, available, retry, expected) in [
				(false, Some(cursor), true, false, true),
				(false, None, true, true, true),
				(true, Some(cursor), true, false, false),
				(false, None, true, false, false),
				(false, Some(cursor), false, false, false),
			] {
				state.gateway_connected = true;
				state.request_pins().unwrap();
				let page = state.search.as_mut().unwrap();
				page.loading = loading;
				page.pin_before = retry.then_some(cursor);
				page.error = retry.then_some("Synthetic pin request failed");
				page.page = (!retry).then(|| model::SearchPage {
					hits: vec![model::SearchHit {
						id: Id(10),
						channel: Id(1),
						author: "Synthetic".into(),
						excerpt: "Synthetic pinned message".into(),
					}],
					total: 1,
					partial: continuation.is_some(),
					pin_cursor: continuation,
				});
				state.gateway_connected = available;
				let ctx = egui::Context::default();
				ctx.set_visuals(if dark {
					egui::Visuals::dark()
				} else {
					egui::Visuals::light()
				});
				let mut view = SearchUi {
					channel: Some(Id(1)),
					pins: true,
					open: true,
					focus: true,
					..SearchUi::default()
				};
				let mut commands = Vec::new();
				// Focus starts on Reload; Tab reaches Older (or Retry older) when enabled.
				// Disabled/exhausted states must not submit another pin-page request.
				for key in [None, None, Some(egui::Key::Tab), Some(egui::Key::Enter)] {
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(420.0, 480.0),
							)),
							events: key
								.into_iter()
								.map(|key| egui::Event::Key {
									key,
									physical_key: None,
									pressed: true,
									repeat: false,
									modifiers: egui::Modifiers::NONE,
								})
								.collect(),
							..Default::default()
						},
						|ui| run(ui, &mut view, &mut state, &mut commands),
					);
					assert!(output.platform_output.commands.is_empty());
					output.drop_without_applying_deltas();
				}
				assert_eq!(
					commands
						.iter()
						.filter(|c| matches!(c, Command::Pins { .. }))
						.count(),
					usize::from(expected)
				);
				if expected {
					assert!(
						matches!(commands.last(), Some(Command::Pins { before: Some(value), .. }) if *value == cursor)
					);
					let page = state.search.as_ref().unwrap();
					assert_eq!(page.pin_before, Some(cursor));
					assert!(page.loading && page.page.is_none());
				}
			}
		}
	}
	#[test]
	fn keyboard_search_is_explicit_and_ime_commit_does_not_submit() {
		for ime in [false, true] {
			let mut state = State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				selected: Some(Id(1)),
				..State::default()
			};
			state.channels.push(model::Channel {
				id: Id(1),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic".into(),
				kind: 1,
				recipients: vec![],
				member_list_id: None,
				message_count: None,
				icon: None,
				last_message: None,
			});
			let ctx = egui::Context::default();
			let mut view = SearchUi {
				channel: Some(Id(1)),
				open: true,
				focus: true,
				query: "synthetic".into(),
				..SearchUi::default()
			};
			let mut commands = Vec::new();
			for frame in 0..3 {
				let mut events = Vec::new();
				if frame == 2 {
					if ime {
						events.push(egui::Event::Ime(egui::ImeEvent::Commit("語".into())));
					}
					events.push(egui::Event::Key {
						key: egui::Key::Enter,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					});
				}
				let mut output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(640.0, 480.0),
						)),
						events,
						..Default::default()
					},
					|ui| run(ui, &mut view, &mut state, &mut commands),
				);
				assert!(output.platform_output.commands.is_empty());
				output.textures_delta.clear();
				if frame < 2 {
					assert!(commands.is_empty());
				}
			}
			assert_eq!(
				commands
					.iter()
					.filter(|c| matches!(c, Command::Search { .. }))
					.count(),
				usize::from(!ime)
			);
			view.open = false;
			let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
				run(ui, &mut view, &mut state, &mut commands)
			});
			output.textures_delta.clear();
			assert!(state.search.is_none());
		}
	}
}
