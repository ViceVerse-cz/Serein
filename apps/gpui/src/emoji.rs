//! Emoji picker popover for the composer and for adding reactions. Unicode emoji come from
//! `ui::emoji` and draw with the system emoji font; server emoji are listed by name until custom
//! emoji images are loaded.
use crate::autocomplete::{custom_markup, emoji_rank};
use crate::theme::{Icon, color, icon, palette, solid};
use crate::{Serein, input, tooltip};
use client_core::State;
use gpui::{prelude::*, *};
use model::{Id, ReactionEmoji};
use std::ops::Range;

const COLUMNS: usize = 9;
const CUSTOM_COLUMNS: usize = 3;
const CELL: f32 = 40.;
const WIDTH: f32 = COLUMNS as f32 * CELL + 24.;
const LIST_HEIGHT: f32 = 320.;
/// Search results beyond this many cells are dropped; the full palette is ~1,900 emoji.
const MAX_RESULTS: usize = 40 * COLUMNS;

/// CLDR groups in bundled order, each located by its first emoji.
const CATEGORIES: [(&str, &str); 9] = [
	("😀", "Smileys & Emotion"),
	("👋", "People & Body"),
	("🐵", "Animals & Nature"),
	("🍇", "Food & Drink"),
	("🌍", "Travel & Places"),
	("🎃", "Activities"),
	("👓", "Objects"),
	("🏧", "Symbols"),
	("🏁", "Flags"),
];

/// Where a chosen emoji goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
	/// Appended to the composer text.
	Composer,
	/// Added as a reaction to this message.
	React(Id),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Cell {
	Unicode {
		text: &'static str,
		code: &'static str,
	},
	Custom {
		source: Id,
		emoji: model::CustomEmoji,
	},
}

#[derive(Clone, PartialEq, Debug)]
pub enum Row {
	Header(SharedString),
	Cells(Range<usize>),
}

/// A category tab: its glyph (or server initial), name and first row.
pub struct Tab {
	pub glyph: SharedString,
	pub name: SharedString,
	pub row: usize,
}

#[derive(Default)]
pub struct Grid {
	pub cells: Vec<Cell>,
	pub rows: Vec<Row>,
	pub tabs: Vec<Tab>,
}

impl Grid {
	fn push(&mut self, header: SharedString, cells: Vec<Cell>, columns: usize) {
		if cells.is_empty() {
			return;
		}
		self.rows.push(Row::Header(header));
		let start = self.cells.len();
		self.cells.extend(cells);
		let end = self.cells.len();
		self.rows.extend(
			(start..end)
				.step_by(columns)
				.map(|row| Row::Cells(row..(row + columns).min(end))),
		);
	}
}

/// The current server's usable emoji, sorted by name.
fn server_emoji(state: &State) -> Option<(&model::Guild, Vec<Cell>)> {
	let channel = state.selected?;
	let guild = state.channel(channel)?.guild?;
	let guild = state.guild(guild)?;
	let mut cells = guild
		.emojis
		.iter()
		.flatten()
		.filter(|emoji| {
			state
				.custom_emoji_unavailable_reason(channel, guild.id, emoji)
				.is_none()
		})
		.map(|emoji| Cell::Custom {
			source: guild.id,
			emoji: emoji.clone(),
		})
		.collect::<Vec<_>>();
	cells.sort_by(|a, b| match (a, b) {
		(Cell::Custom { emoji: a, .. }, Cell::Custom { emoji: b, .. }) => a.name.cmp(&b.name),
		_ => std::cmp::Ordering::Equal,
	});
	Some((guild, cells))
}

/// Categories with every base emoji (skin-tone variants are left out of the grid), or ranked
/// search results for a non-empty query.
pub fn grid(state: &State, query: &str) -> Grid {
	let query = query.trim().to_lowercase();
	let mut grid = Grid::default();
	let server = server_emoji(state);
	if !query.is_empty() {
		if let Some((_, cells)) = server {
			let cells = cells
				.into_iter()
				.filter(|cell| match cell {
					Cell::Custom { emoji, .. } => emoji.name.to_lowercase().contains(&query),
					Cell::Unicode { .. } => false,
				})
				.take(MAX_RESULTS)
				.collect();
			grid.push("SERVER EMOJI".into(), cells, CUSTOM_COLUMNS);
		}
		let mut ranked = ui::emoji::unicode()
			.enumerate()
			.filter_map(|(index, (text, code))| {
				let name = &code[1..code.len() - 1];
				emoji_rank(name, &query)
					.map(|score| ((score, name.contains("skin_tone"), index), text, code))
			})
			.collect::<Vec<_>>();
		ranked.sort_unstable_by_key(|(key, _, _)| *key);
		let cells = ranked
			.into_iter()
			.take(MAX_RESULTS)
			.map(|(_, text, code)| Cell::Unicode { text, code })
			.collect();
		grid.push("SEARCH RESULTS".into(), cells, COLUMNS);
		return grid;
	}
	if let Some((guild, cells)) = server
		&& !cells.is_empty()
	{
		grid.tabs.push(Tab {
			glyph: guild
				.name
				.chars()
				.next()
				.map_or_else(|| "S".into(), |c| c.to_uppercase().collect::<String>())
				.into(),
			name: guild.name.clone().into(),
			row: grid.rows.len(),
		});
		grid.push(guild.name.to_uppercase().into(), cells, CUSTOM_COLUMNS);
	}
	let mut category = None;
	let mut sections: Vec<Vec<Cell>> = vec![Vec::new(); CATEGORIES.len()];
	for (text, code) in ui::emoji::unicode() {
		if let Some(index) = CATEGORIES.iter().position(|(first, _)| *first == text) {
			category = Some(index);
		}
		if let Some(index) = category
			&& !code.contains("skin_tone")
		{
			sections[index].push(Cell::Unicode { text, code });
		}
	}
	for ((glyph, name), cells) in CATEGORIES.into_iter().zip(sections) {
		if !cells.is_empty() {
			grid.tabs.push(Tab {
				glyph: glyph.into(),
				name: name.into(),
				row: grid.rows.len(),
			});
		}
		grid.push(name.to_uppercase().into(), cells, COLUMNS);
	}
	grid
}

pub struct Picker {
	target: Target,
	position: Point<Pixels>,
	search: Entity<input::Input>,
	grid: Grid,
	hovered: Option<usize>,
	scroll: UniformListScrollHandle,
}

impl Serein {
	/// Opens the picker next to `position` (window coordinates), focused on its search box.
	pub(crate) fn open_emoji_picker(
		&mut self,
		target: Target,
		position: Point<Pixels>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		// The click that closed the picker by landing on its own toggle must not reopen it.
		if self
			.emoji_closed_at
			.is_some_and(|closed| closed.elapsed() < std::time::Duration::from_millis(250))
		{
			return;
		}
		let search = cx.new(input::Input::new);
		search.update(cx, |input, cx| {
			input.set_placeholder("Find the perfect emoji".into(), cx)
		});
		cx.subscribe(&search, |this, _, _: &input::Submit, cx| {
			if let Some(picker) = &this.emoji_picker
				&& !picker.grid.cells.is_empty()
			{
				this.choose_emoji(0, false, cx);
			}
		})
		.detach();
		cx.subscribe_in(
			&search,
			window,
			|this, search, event: &input::Event, window, cx| match event {
				input::Event::Changed => {
					let query = search.read(cx).value().to_owned();
					let grid = grid(&this.state, &query);
					if let Some(picker) = &mut this.emoji_picker {
						picker.grid = grid;
						picker.hovered = None;
						picker.scroll.scroll_to_item(0, ScrollStrategy::Top);
					}
					cx.notify();
				}
				input::Event::Cancel => this.close_emoji_picker(Some(window), cx),
				_ => {}
			},
		)
		.detach();
		let focus = search.read(cx).focus_handle(cx);
		window.focus(&focus, cx);
		self.emoji_picker = Some(Picker {
			target,
			position,
			search,
			grid: grid(&self.state, ""),
			hovered: None,
			scroll: UniformListScrollHandle::new(),
		});
		cx.notify();
	}

	/// Closes the picker; with a window, composer mode hands focus back to the composer.
	pub(crate) fn close_emoji_picker(
		&mut self,
		window: Option<&mut Window>,
		cx: &mut Context<Self>,
	) {
		let Some(picker) = self.emoji_picker.take() else {
			return;
		};
		if picker.target == Target::Composer
			&& let Some(window) = window
		{
			let focus = self.composer.read(cx).focus_handle(cx);
			window.focus(&focus, cx);
		}
		cx.notify();
	}

	/// Inserts or reacts with cell `index`. Shift-click keeps the composer picker open.
	fn choose_emoji(&mut self, index: usize, keep_open: bool, cx: &mut Context<Self>) {
		let Some(picker) = &self.emoji_picker else {
			return;
		};
		let Some(cell) = picker.grid.cells.get(index).cloned() else {
			return;
		};
		let target = picker.target;
		// The conversation may have changed since the picker opened.
		if let Cell::Custom { source, emoji } = &cell
			&& self.state.selected.is_none_or(|channel| {
				self.state
					.custom_emoji_unavailable_reason(channel, *source, emoji)
					.is_some()
			}) {
			self.notify_user("That server emoji can't be used here.");
			cx.notify();
			return;
		}
		match target {
			Target::Composer => {
				let text = match &cell {
					Cell::Unicode { text, .. } => (*text).to_owned(),
					Cell::Custom { emoji, .. } => custom_markup(emoji),
				};
				self.composer.update(cx, |input, cx| {
					let value = input.value();
					let space = if value.is_empty() || value.ends_with(char::is_whitespace) {
						""
					} else {
						" "
					};
					let next = format!("{value}{space}{text}");
					input.set_value(next, cx);
				});
				if !keep_open {
					self.emoji_picker = None;
				}
			}
			Target::React(message) => {
				let emoji = match cell {
					Cell::Unicode { text, .. } => ReactionEmoji {
						id: None,
						name: Some(text.to_owned()),
					},
					Cell::Custom { emoji, .. } => ReactionEmoji {
						id: Some(emoji.id),
						name: Some(emoji.name),
					},
				};
				let command = self.state.prepare_reaction(message, emoji);
				if command.is_none() && !self.state.demo {
					self.notify_user("Reactions need a live, fully synced connection.");
				}
				self.dispatch(command);
				self.emoji_picker = None;
			}
		}
		cx.notify();
	}

	fn emoji_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
		let p = palette();
		let Some(picker) = &self.emoji_picker else {
			return Vec::new();
		};
		range
			.filter_map(|ix| picker.grid.rows.get(ix).map(|row| (ix, row)))
			.map(|(ix, row)| match row {
				Row::Header(title) => div()
					.id(("emoji-header", ix))
					.h(px(CELL))
					.px_1()
					.pb_1()
					.flex()
					.items_end()
					.text_size(px(12.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.muted))
					.overflow_hidden()
					.whitespace_nowrap()
					.child(title.clone())
					.into_any_element(),
				Row::Cells(cells) => div()
					.id(("emoji-row", ix))
					.h(px(CELL))
					.flex()
					.children(cells.clone().filter_map(|index| {
						let cell = picker.grid.cells.get(index)?;
						let base = div()
							.id(("emoji-cell", index))
							.h(px(CELL))
							.rounded(px(6.))
							.flex()
							.items_center()
							.cursor_pointer()
							.hover(|d| d.bg(color(p.hover)))
							.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
								if let Some(picker) = &mut this.emoji_picker {
									if *hovered {
										picker.hovered = Some(index);
									} else if picker.hovered == Some(index) {
										picker.hovered = None;
									}
									cx.notify();
								}
							}))
							.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
								let keep_open = event.modifiers().shift;
								this.choose_emoji(index, keep_open, cx);
								if this.emoji_picker.is_none() {
									let focus = this.composer.read(cx).focus_handle(cx);
									window.focus(&focus, cx);
								}
							}));
						Some(match cell {
							Cell::Unicode { text, .. } => base
								.w(px(CELL))
								.justify_center()
								.text_size(px(26.))
								.child(*text)
								.into_any_element(),
							Cell::Custom { emoji, .. } => base
								.w(px(CELL * COLUMNS as f32 / CUSTOM_COLUMNS as f32))
								.px_2()
								.gap_1()
								.child(icon(Icon::Smiley, px(16.), color(p.muted)))
								.child(
									div()
										.min_w_0()
										.text_size(px(13.))
										.text_color(color(p.text))
										.overflow_hidden()
										.whitespace_nowrap()
										.text_ellipsis()
										.child(format!(":{}:", emoji.name)),
								)
								.into_any_element(),
						})
					}))
					.into_any_element(),
			})
			.collect()
	}

	fn emoji_preview(&self) -> AnyElement {
		let p = palette();
		let hovered = self
			.emoji_picker
			.as_ref()
			.and_then(|picker| picker.grid.cells.get(picker.hovered?));
		let (glyph, label) = match hovered {
			Some(Cell::Unicode { text, code }) => (Some(*text), (*code).to_owned()),
			Some(Cell::Custom { emoji, .. }) => (None, format!(":{}:", emoji.name)),
			None => (Some("🙂"), "Pick an emoji".to_owned()),
		};
		div()
			.h(px(48.))
			.px_3()
			.flex()
			.items_center()
			.gap_2()
			.border_t_1()
			.border_color(color(p.border))
			.child(match glyph {
				Some(glyph) => div().text_size(px(26.)).child(glyph).into_any_element(),
				None => icon(Icon::Smiley, px(26.), color(p.muted)).into_any_element(),
			})
			.child(
				div()
					.min_w_0()
					.text_size(px(14.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.text_strong))
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.child(label),
			)
			.into_any_element()
	}

	pub(crate) fn render_emoji_picker(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let picker = self.emoji_picker.as_ref()?;
		let searching = !picker.search.read(cx).value().trim().is_empty();
		let scroll = picker.scroll.clone();
		let tabs = picker
			.grid
			.tabs
			.iter()
			.enumerate()
			.map(|(index, tab)| {
				let row = tab.row;
				let scroll = scroll.clone();
				let custom = tab.glyph.chars().all(|c| c.is_alphanumeric());
				div()
					.id(("emoji-tab", index))
					.size(px(32.))
					.flex_none()
					.rounded(px(6.))
					.flex()
					.items_center()
					.justify_center()
					.cursor_pointer()
					.hover(|d| d.bg(color(p.hover)))
					.tooltip(tooltip(tab.name.clone()))
					.when(custom, |d| {
						d.rounded_full()
							.bg(color(p.raised))
							.text_size(px(13.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.text_strong))
					})
					.when(!custom, |d| d.text_size(px(20.)))
					.on_click(move |_, _, _| scroll.scroll_to_item(row, ScrollStrategy::Top))
					.child(tab.glyph.clone())
			})
			.collect::<Vec<_>>();
		let empty = picker.grid.rows.is_empty();
		let anchor = match picker.target {
			Target::Composer => Anchor::BottomRight,
			Target::React(_) => Anchor::TopRight,
		};
		Some(deferred(
			anchored()
				.anchor(anchor)
				.position(picker.position)
				.snap_to_window_with_margin(px(8.))
				.child(
					div()
						.id("emoji-picker")
						.occlude()
						.w(px(WIDTH))
						.rounded(px(10.))
						.bg(solid(p.base))
						.border_1()
						.border_color(color(p.border))
						.shadow_lg()
						.overflow_hidden()
						.flex()
						.flex_col()
						.font_family(crate::theme::FONT)
						.on_mouse_down_out(cx.listener(|this, _, _, cx| {
							this.emoji_closed_at = Some(std::time::Instant::now());
							this.close_emoji_picker(None, cx)
						}))
						.child(
							div().p_3().pb_2().child(
								div()
									.h(px(36.))
									.px_2()
									.rounded(px(6.))
									.bg(color(p.raised))
									.flex()
									.items_center()
									.gap_2()
									.child(div().flex_1().min_w_0().child(picker.search.clone())),
							),
						)
						.when(!searching && !tabs.is_empty(), |d| {
							d.child(
								div()
									.px(px(10.))
									.pb_1()
									.flex()
									.gap(px(2.))
									.border_b_1()
									.border_color(color(p.border))
									.children(tabs),
							)
						})
						.child(
							div()
								.h(px(LIST_HEIGHT))
								.px_3()
								.when(empty, |d| {
									d.flex()
										.items_center()
										.justify_center()
										.text_color(color(p.muted))
										.child("No emoji match your search.")
								})
								.when(!empty, |d| {
									d.child(
										uniform_list(
											"emoji-grid",
											picker.grid.rows.len(),
											cx.processor(|this, range, _, cx| {
												this.emoji_rows(range, cx)
											}),
										)
										.track_scroll(&scroll)
										.h_full(),
									)
								}),
						)
						.child(self.emoji_preview()),
				),
		))
	}
}

#[cfg(test)]
mod tests {
	use super::{COLUMNS, Cell, Row, grid};

	#[test]
	fn categories_hold_base_emoji_in_bounded_rows() {
		let state = test_support::chat_demo_state();
		let grid = grid(&state, "");
		// Server tab first, then the nine CLDR groups.
		assert_eq!(grid.tabs.len(), 10);
		assert_eq!(grid.tabs[0].name.as_ref(), "Synthetic workspace");
		assert_eq!(grid.tabs[1].glyph.as_ref(), "😀");
		assert!(matches!(grid.rows[grid.tabs[1].row], Row::Header(_)));
		assert!(grid.cells.iter().any(|cell| matches!(
			cell,
			Cell::Custom { emoji, .. } if emoji.name == "serein_wave"
		)));
		assert!(grid.cells.contains(&Cell::Unicode {
			text: "👍",
			code: ":thumbs_up:"
		}));
		assert!(!grid.cells.iter().any(|cell| matches!(
			cell,
			Cell::Unicode { code, .. } if code.contains("skin_tone")
		)));
		for row in &grid.rows {
			if let Row::Cells(range) = row {
				assert!(!range.is_empty() && range.len() <= COLUMNS);
			}
		}
	}

	#[test]
	fn search_ranks_prefixes_and_matches_server_names() {
		let state = test_support::chat_demo_state();
		let found = grid(&state, "Wave");
		assert!(found.tabs.is_empty());
		assert!(matches!(
			&found.cells[0],
			Cell::Custom { emoji, .. } if emoji.name == "serein_wave"
		));
		assert!(found.cells.contains(&Cell::Unicode {
			text: "🌊",
			code: ":water_wave:"
		}));
		assert!(found.cells.len() <= 2 * super::MAX_RESULTS);
		assert!(grid(&state, "zzzzqq").rows.is_empty());
	}
}
