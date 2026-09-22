//! Search the bundled Unicode palette and joined servers' bounded catalogs.
use crate::avatars::Avatars;
use client_core::{Command, State};
use model::Id;
use std::sync::OnceLock;

const NAMES: &str = include_str!("../../../assets/twemoji/names.tsv");
const CELL: f32 = 40.0;

/// Replace the composer's scalar-index selection without exceeding its character or RAM budget.
pub(crate) fn insert(
	draft: &mut String,
	text: &str,
	range: Option<egui::text::CCursorRange>,
	remaining: usize,
) -> Option<usize> {
	let count = draft.chars().count();
	let (start, end) = range.map_or((count, count), |range| {
		let range = range.as_sorted_char_range();
		(range.start.0.min(count), range.end.0.min(count))
	});
	let inserted = text.chars().count();
	if count - (end - start) + inserted > client_core::MAX_CONTENT {
		return None;
	}
	let byte_start = draft
		.char_indices()
		.nth(start)
		.map_or(draft.len(), |(i, _)| i);
	let byte_end = draft
		.char_indices()
		.nth(end)
		.map_or(draft.len(), |(i, _)| i);
	let bytes = draft.len() - (byte_end - byte_start) + text.len();
	let budget = draft.capacity().saturating_add(remaining);
	if bytes > budget {
		return None;
	}
	// draft_bytes measures capacity, so avoid String's geometric growth crossing the budget.
	if bytes > draft.capacity() {
		let mut replacement = String::with_capacity(bytes);
		if replacement.capacity() > budget {
			return None;
		}
		replacement.push_str(&draft[..byte_start]);
		replacement.push_str(text);
		replacement.push_str(&draft[byte_end..]);
		*draft = replacement;
	} else {
		draft.replace_range(byte_start..byte_end, text);
	}
	Some(start + inserted)
}

/// Replace an exact completed Unicode shortcode immediately before the caret.
pub(crate) fn complete_shortcode(
	draft: &mut String,
	cursor: usize,
	remaining: usize,
) -> Option<usize> {
	let end = draft
		.char_indices()
		.nth(cursor)
		.map_or(draft.len(), |(index, _)| index);
	let prefix = draft[..end].strip_suffix(':')?;
	let start = prefix.rfind(':')?;
	if prefix[..start]
		.chars()
		.next_back()
		.is_some_and(|c| !c.is_whitespace() && !matches!(c, '(' | '[' | '{'))
	{
		return None;
	}
	let name = &prefix[start + 1..];
	if !(2..=64).contains(&name.chars().count()) {
		return None;
	}
	let emoji = standard()
		.iter()
		.zip(shortcodes())
		.find_map(|((emoji, _), code)| {
			code[1..code.len() - 1]
				.eq_ignore_ascii_case(name)
				.then_some(*emoji)
		})?;
	let start = draft[..start].chars().count();
	insert(
		draft,
		emoji,
		Some(egui::text::CCursorRange::two(
			egui::text::CCursor::new(start),
			egui::text::CCursor::new(cursor),
		)),
		remaining,
	)
}

pub(crate) fn standard() -> &'static [(&'static str, &'static str)] {
	static ENTRIES: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
	ENTRIES.get_or_init(|| {
		NAMES
			.lines()
			.map(|line| line.split_once('\t').expect("bundled emoji name"))
			.collect()
	})
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
	Stickers,
	Emoji,
	Gifs,
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum GifSection {
	Home,
	Favorites,
	Trending,
	Category(String),
}

/// What the composer does with a picked item.
pub(crate) enum Pick {
	Image(model::ImageShare),
	Sticker(model::Sticker),
	/// Insert text at the caret (emoji or custom emoji markup).
	Insert(String),
	/// Send this GIF address as its own message right away, like Discord.
	Send(String),
	React(Id, model::ReactionEmoji),
}

enum GifAction {
	Toggle(model::Gif),
	Send(String),
}

enum GifMode {
	Home,
	Favorites,
	/// Typing paused less than the debounce ago; keep showing the previous view.
	Waiting,
	Remote(Option<String>),
}
impl GifMode {
	/// The remote query this view needs: `Some(None)` is trending, `None` needs nothing.
	fn wanted(&self) -> Option<Option<&str>> {
		match self {
			GifMode::Home => Some(None),
			GifMode::Remote(query) => Some(query.as_deref()),
			GifMode::Favorites | GifMode::Waiting => None,
		}
	}
}

const CUSTOM_LIMIT: usize = model::MAX_GUILD_EMOJIS;

/// Case-insensitive substring test against an already lowercased `needle`. ASCII names
/// (Discord permits only `[A-Za-z0-9_]`) compare in place; only non-ASCII server names allocate.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
	if haystack.is_ascii() && needle.is_ascii() {
		haystack
			.as_bytes()
			.windows(needle.len())
			.any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
	} else {
		haystack.to_lowercase().contains(needle)
	}
}

/// Retain indices only; cap search results independently of the joined-server count.
fn custom_matches(state: &State, server: Option<Id>, query: &str) -> Vec<(usize, usize)> {
	let query = query.trim().to_lowercase();
	let mut matching: Vec<_> = state
		.guilds
		.iter()
		.enumerate()
		.filter(|(_, guild)| !query.is_empty() || server == Some(guild.id))
		.flat_map(|(guild_index, guild)| {
			let query = &query;
			let source_matches = !query.is_empty() && contains_ignore_case(&guild.name, query);
			guild
				.emojis
				.iter()
				.flatten()
				.enumerate()
				.filter_map(move |(emoji_index, emoji)| {
					(source_matches || query.is_empty() || contains_ignore_case(&emoji.name, query))
						.then_some((guild_index, emoji_index))
				})
		})
		.take(CUSTOM_LIMIT)
		.collect();
	matching.sort_unstable_by_key(|&(guild, emoji)| {
		let guild = &state.guilds[guild];
		(
			guild.id,
			guild.emojis.as_ref().expect("matched catalog")[emoji].id,
		)
	});
	matching
}

#[derive(Default)]
struct CustomMatches {
	key: Option<(u64, u64, Option<Id>, Option<Id>)>,
	query: Box<str>,
	// At most CUSTOM_LIMIT index pairs (16 KiB on 64-bit), with no catalog clones/references.
	entries: Box<[(usize, usize)]>,
}

impl CustomMatches {
	fn update(&mut self, state: &State, server: Option<Id>, query: &str) -> bool {
		let key = (
			state.generation,
			state.revision,
			state.user.as_ref().map(|user| user.id),
			server,
		);
		if self.key == Some(key) && self.query.as_ref() == query {
			return false;
		}
		// All production catalog/name/membership mutations advance State::revision.
		// ponytail: unrelated events also invalidate; add a catalog epoch only if churn matters.
		self.entries = custom_matches(state, server, query).into_boxed_slice();
		// The UI admits 64 Unicode scalars. Oversized internal queries are never retained.
		self.key = (query.len() <= 64 * 4).then_some(key);
		self.query = if self.key.is_some() {
			query.into()
		} else {
			Box::default()
		};
		true
	}

	fn len(&self) -> usize {
		self.entries.len()
	}

	fn get<'a>(
		&self,
		state: &'a State,
		index: usize,
	) -> Option<(&'a model::Guild, &'a model::CustomEmoji)> {
		let &(guild, emoji) = self.entries.get(index)?;
		let guild = state.guilds.get(guild)?;
		Some((guild, guild.emojis.as_ref()?.get(emoji)?))
	}
}

pub(crate) struct Picker {
	pub image_sharing_enabled: bool,
	stickers: crate::stickers::Browser,
	reaction: Option<(Id, egui::Rect, egui::Id)>,
	// ponytail: session-only Unicode usage; persist if cross-launch favorites are needed.
	frequent: Vec<(usize, u32)>,
	open: bool,
	pending_open: bool,
	focus: bool,
	channel: Option<Id>,
	generation: u64,
	server: Option<Id>,
	query: String,
	matches: Vec<usize>,
	custom: CustomMatches,
	tab: Tab,
	gif_section: GifSection,
	gif_query: String,
	/// Time the GIF query last changed; the search fires once typing pauses.
	gif_changed_at: Option<f64>,
}

impl Default for Picker {
	fn default() -> Self {
		// Initialize the static catalog during application creation, outside rendering.
		Self {
			image_sharing_enabled: false,
			stickers: crate::stickers::Browser::default(),
			reaction: None,
			frequent: Vec::with_capacity(32),
			open: false,
			pending_open: false,
			focus: false,
			channel: None,
			generation: 0,
			server: None,
			query: String::new(),
			matches: (0..standard().len()).collect(),
			custom: CustomMatches::default(),
			tab: Tab::Emoji,
			gif_section: GifSection::Home,
			gif_query: String::new(),
			gif_changed_at: None,
		}
	}
}

const GIF_DEBOUNCE: f64 = 0.3;

impl Picker {
	/// The same bundled Unicode catalog and cells, without composer or network actions.
	pub(crate) fn unicode_button(&mut self, ui: &mut egui::Ui, selected: &mut Option<String>) {
		self.unicode_button_with(ui, selected, true);
	}
	/// `removable` offers clearing the choice; insertion targets have nothing to clear.
	pub(crate) fn unicode_button_with(
		&mut self,
		ui: &mut egui::Ui,
		selected: &mut Option<String>,
		removable: bool,
	) {
		let button = if let Some(image) = selected
			.as_deref()
			.and_then(|emoji| crate::emoji::image(ui.ctx(), emoji, 22.0))
		{
			ui.add(
				egui::Button::image(image)
					.frame(false)
					.min_size(egui::Vec2::splat(28.0)),
			)
		} else if selected.is_none() {
			crate::icons::button(ui, crate::icons::Icon::Smile, 28.0, "Choose emoji")
		} else {
			ui.add_sized(
				[28.0, 28.0],
				egui::Button::new(selected.as_deref().unwrap_or("☺")).frame(false),
			)
		}
		.on_hover_text("Choose emoji");
		button.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Choose emoji")
		});
		if button.clicked() {
			self.query.clear();
			self.filter();
		}
		egui::Popup::menu(&button)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.show(|ui| {
				ui.set_width(280.0);
				let colors = crate::design::palette(ui);
				if ui
					.add(
						egui::TextEdit::singleline(&mut self.query)
							.hint_text("Search emoji")
							.char_limit(64)
							.desired_width(f32::INFINITY),
					)
					.changed()
				{
					self.filter();
				}
				if removable && ui.button("Remove emoji").clicked() {
					*selected = None;
					ui.close();
				}
				egui::ScrollArea::vertical().max_height(240.0).show_rows(
					ui,
					CELL,
					self.matches.len().div_ceil(6),
					|ui, rows| {
						for row in rows {
							ui.horizontal(|ui| {
								ui.spacing_mut().item_spacing.x = 4.0;
								for &index in self.matches.iter().skip(row * 6).take(6) {
									let (text, name) = standard()[index];
									let image = crate::emoji::image(ui.ctx(), text, 32.0);
									if cell(ui, image, name, true, &colors)
										.on_hover_text(name)
										.clicked()
									{
										*selected = Some(text.into());
										ui.close();
									}
								}
							});
						}
					},
				);
			});
	}

	pub(crate) fn is_open(&self) -> bool {
		self.open
	}

	pub(crate) fn dismiss(&mut self, state: &mut State, commands: &mut Vec<Command>) {
		if std::mem::take(&mut self.open) {
			self.close_gifs(state, commands);
		}
	}

	/// Fixture-only: open the popout on the next frame regardless of navigation resets.
	#[cfg(any(test, feature = "demo"))]
	pub(crate) fn preview(&mut self) {
		self.pending_open = true;
	}
	/// Fixture-only: open the GIFs tab at `section` (`""`, `favorites`, `trending` or a query).
	#[cfg(any(test, feature = "demo"))]
	pub(crate) fn preview_gifs(&mut self, section: &str) {
		self.pending_open = true;
		self.tab = Tab::Gifs;
		self.gif_query.clear();
		self.gif_changed_at = None;
		self.gif_section = match section {
			"" => GifSection::Home,
			"favorites" => GifSection::Favorites,
			"trending" => GifSection::Trending,
			query => {
				self.gif_query = query.to_owned();
				GifSection::Home
			}
		};
	}
	pub(crate) fn open_stickers(&mut self, sticker: Option<&model::Sticker>) {
		self.pending_open = true;
		self.tab = Tab::Stickers;
		self.focus = true;
		if let Some(sticker) = sticker {
			self.stickers.focus(sticker);
		}
	}
	pub(crate) fn open_gifs(&mut self, query: &str) {
		self.pending_open = true;
		self.tab = Tab::Gifs;
		self.focus = true;
		self.gif_section = GifSection::Home;
		self.gif_query = query.chars().take(64).collect();
		self.gif_changed_at = None;
	}
	pub(crate) fn search_stickers(&mut self, query: &str) {
		self.open_stickers(None);
		self.stickers.target = None;
		self.stickers.query = query.chars().take(64).collect();
	}

	fn filter(&mut self) {
		let query = self.query.trim().to_lowercase();
		self.matches.clear();
		self.matches.extend(
			standard()
				.iter()
				.enumerate()
				.filter(|(_, (text, name))| name.contains(&query) || text.contains(&query))
				.map(|(index, _)| index),
		);
	}
	fn close_gifs(&mut self, state: &mut State, commands: &mut Vec<Command>) {
		if self.reaction.is_some() {
			return;
		}
		self.gif_section = GifSection::Home;
		self.gif_query.clear();
		self.gif_changed_at = None;
		if let Some(command) = state.clear_gifs() {
			commands.push(command);
		}
	}

	pub(crate) fn sync(&mut self, state: &State, channel: Option<Id>) {
		if self.channel != channel || self.generation != state.generation {
			self.custom = CustomMatches::default();
			if self.generation != state.generation {
				self.frequent.clear();
			}
			self.reaction = None;
			self.channel = channel;
			self.generation = state.generation;
			self.open = false;
			self.server = None;
			self.query.clear();
			self.filter();
			if !self.pending_open {
				self.stickers = crate::stickers::Browser::default();
				self.tab = Tab::Emoji;
				self.gif_section = GifSection::Home;
				self.gif_query.clear();
				self.gif_changed_at = None;
			}
		}
	}

	pub(crate) fn open_reaction(
		&mut self,
		state: &State,
		message: Id,
		anchor: egui::Rect,
		trigger: egui::Id,
	) {
		let Some(channel) = state.selected else {
			return;
		};
		self.sync(state, Some(channel));
		self.reaction = Some((message, anchor, trigger));
		self.open = true;
		self.focus = true;
		self.tab = Tab::Emoji;
		self.server = None;
		self.query.clear();
		self.filter();
	}

	pub(crate) fn show_reaction(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		let Some(channel) = state.selected else {
			self.open = false;
			return;
		};
		self.sync(state, Some(channel));
		let Some((message, anchor, trigger_id)) = self.reaction.filter(|_| self.open) else {
			return;
		};
		if !state
			.timeline
			.get(message)
			.is_some_and(|m| m.channel == channel)
		{
			self.open = false;
			return;
		}
		let trigger = ui.interact(anchor, trigger_id, egui::Sense::hover());
		// The real trigger is the hover-toolbar button, which is not registered while the
		// popout covers the message row. Keep a node for the id focus is returned to, or
		// AccessKit's tree validation panics on a focused id missing from the node list.
		trigger.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Add reaction")
		});
		if let Some(Pick::React(message, emoji)) =
			self.popup(ui, state, channel, avatars, commands, &trigger, None)
			&& let Some(command) = state.prepare_reaction(message, emoji)
		{
			commands.push(command);
		}
	}

	pub(crate) fn record(&mut self, text: &str) {
		let Some(index) = standard().iter().position(|(emoji, _)| *emoji == text) else {
			return;
		};
		let count = self
			.frequent
			.iter()
			.position(|(i, _)| *i == index)
			.map_or(1, |position| {
				self.frequent.remove(position).1.saturating_add(1)
			});
		if self.frequent.len() == 32 {
			self.frequent.pop();
		}
		self.frequent.insert(0, (index, count));
		self.frequent
			.sort_by_key(|entry| std::cmp::Reverse(entry.1));
	}

	fn favorites(&self) -> Vec<usize> {
		let mut favorites: Vec<_> = self.frequent.iter().take(8).map(|(i, _)| *i).collect();
		for text in ["👍", "❤️", "😂", "🎉", "👀", "✅", "🙏", "😢"] {
			if favorites.len() == 8 {
				break;
			}
			if let Some(index) = standard().iter().position(|(emoji, _)| *emoji == text)
				&& !favorites.contains(&index)
			{
				favorites.push(index);
			}
		}
		favorites
	}

	fn images(&self) -> bool {
		self.image_sharing_enabled && self.reaction.is_none()
	}

	fn pick(&self, emoji: model::ReactionEmoji, text: String) -> Pick {
		if self.images()
			&& let Some(id) = emoji.id
		{
			return Pick::Image(model::ImageShare::Emoji {
				id,
				animated: text.starts_with("<a:"),
			});
		}
		match self.reaction {
			Some((message, _, _)) => Pick::React(message, emoji),
			None => Pick::Insert(text),
		}
	}

	fn can_pick(&self, state: &State, emoji: &model::ReactionEmoji) -> bool {
		if self.images() && emoji.id.is_some() {
			return self
				.channel
				.is_some_and(|channel| state.can_send(channel) && state.can_attach(channel));
		}
		match self.reaction {
			Some((message, _, _)) => {
				!state.reactions.busy() && state.can_react(message, Some(emoji), true)
			}
			None => emoji.id.is_none_or(|id| {
				state.custom_emoji(id).is_some_and(|(guild, emoji)| {
					self.channel.is_some_and(|channel| {
						state
							.custom_emoji_unavailable_reason(channel, guild.id, emoji)
							.is_none()
					})
				})
			}),
		}
	}

	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) -> Option<Pick> {
		self.sync(state, Some(channel));
		if std::mem::take(&mut self.pending_open) {
			self.open = true;
		}
		let was_open = self.open;
		// Rightmost first: the composer lays these out right-to-left like Discord's tray.
		let trigger = crate::icons::toggle(
			ui,
			crate::icons::Icon::Smile,
			28.0,
			self.open && self.tab == Tab::Emoji,
			"Insert an emoji",
		);
		let gif_trigger = crate::icons::toggle(
			ui,
			crate::icons::Icon::Gif,
			28.0,
			self.open && self.tab == Tab::Gifs,
			"Send a GIF",
		);
		for (response, tab) in [(&trigger, Tab::Emoji), (&gif_trigger, Tab::Gifs)] {
			if response.clicked() {
				if self.open && self.tab == tab {
					self.open = false;
				} else {
					self.open = true;
					self.tab = tab;
					self.focus = true;
				}
			}
		}
		if !self.open {
			if was_open {
				self.close_gifs(state, commands);
			}
			return None;
		}
		self.popup(
			ui,
			state,
			channel,
			avatars,
			commands,
			&trigger,
			Some(&gif_trigger),
		)
	}

	#[allow(clippy::too_many_arguments)]
	fn popup(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
		trigger: &egui::Response,
		gif_trigger: Option<&egui::Response>,
	) -> Option<Pick> {
		if ui.is_enabled()
			&& ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
		{
			self.open = false;
			self.close_gifs(state, commands);
			trigger.request_focus();
			return None;
		}
		let colors = crate::design::palette(ui);
		let demo = state.demo;
		if self
			.server
			.is_some_and(|id| !state.guilds.iter().any(|guild| guild.id == id))
		{
			self.server = None;
		}
		let bounds = ui.ctx().content_rect().shrink(8.0);
		let width = WIDTH.min(bounds.width());
		let height = HEIGHT.min(bounds.height());
		// Anchor above the composer with the right edge on the trigger, like a Discord popout.
		let x = (trigger.rect.right() - width)
			.min(bounds.right() - width)
			.max(bounds.left());
		let y = (trigger.rect.top() - 8.0 - height).max(bounds.top());
		let mut selected: Option<Pick> = None;
		let mut used = None;
		let mut gif_action: Option<GifAction> = None;
		let mut hovered: Option<(Option<egui::Image<'static>>, String, String)> = None;
		let mut hovered_source: Option<&str> = None;
		let mut hovered_gif: Option<String> = None;
		let gifs_tab = self.tab == Tab::Gifs;
		let stickers_tab = self.tab == Tab::Stickers;
		let mut hovered_sticker = None;
		if stickers_tab
			&& !state.stickers.loaded
			&& !state.stickers.loading
			&& state.stickers.error.is_none()
			&& let Some(command) = state.request_sticker_packs()
		{
			commands.push(command);
		}
		// Remote requests happen before the popout borrows navigation state immutably.
		let gif_mode = self.gif_mode(ui);
		if gifs_tab
			&& let Some(query) = gif_mode.wanted()
			&& let Some(command) = state.request_gifs(query)
		{
			commands.push(command);
		}
		let popup_id =
			egui::Id::unique(("emoji-picker", self.reaction.map(|(message, _, _)| message)));
		let area = egui::Area::new(popup_id)
			.kind(egui::UiKind::Popup)
			.enabled(ui.is_enabled())
			.order(egui::Order::Foreground)
			.fixed_pos(egui::pos2(x, y))
			.constrain_to(bounds)
			.interactable(true)
			.show(ui.ctx(), |ui| {
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
						let (rect, _) =
							ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
						ui.style_mut().interaction.selectable_labels = false;
						const TABS: f32 = 40.0;
						const SEARCH: f32 = 54.0;
						const FOOTER: f32 = 48.0;
						const RAIL: f32 = 48.0;
						let tabs_rect =
							egui::Rect::from_min_size(rect.min, egui::vec2(width, TABS));
						let search_rect = egui::Rect::from_min_size(
							egui::pos2(rect.left(), tabs_rect.bottom()),
							egui::vec2(width, SEARCH),
						);
						let footer_rect = egui::Rect::from_min_size(
							egui::pos2(rect.left(), rect.bottom() - FOOTER),
							egui::vec2(width, FOOTER),
						);
						let body_rect = egui::Rect::from_min_max(
							egui::pos2(rect.left(), search_rect.bottom()),
							egui::pos2(rect.right(), footer_rect.top()),
						);
						let rail_rect = egui::Rect::from_min_max(
							body_rect.min,
							egui::pos2(body_rect.left() + RAIL, body_rect.bottom()),
						);
						let grid_rect = if gifs_tab || stickers_tab {
							body_rect
						} else {
							egui::Rect::from_min_max(
								egui::pos2(rail_rect.right(), body_rect.top()),
								body_rect.max,
							)
						};

						// Tab row.
						ui.scope_builder(
							egui::UiBuilder::new()
								.max_rect(tabs_rect.shrink2(egui::vec2(16.0, 0.0)))
								.layout(egui::Layout::left_to_right(egui::Align::Center)),
							|ui| {
								ui.spacing_mut().item_spacing.x = 20.0;
								let family = crate::design::semibold_family(ui.ctx());
								for (tab, label) in [
									(Tab::Gifs, "GIFs"),
									(Tab::Stickers, "Stickers"),
									(Tab::Emoji, "Emoji"),
								] {
									if self.reaction.is_some() && tab != Tab::Emoji {
										continue;
									}
									let active = self.tab == tab;
									let galley = ui.painter().layout_no_wrap(
										label.to_owned(),
										egui::FontId::new(15.0, family.clone()),
										egui::Color32::PLACEHOLDER,
									);
									let (tab_rect, response) = ui.allocate_exact_size(
										egui::vec2(galley.size().x, TABS),
										egui::Sense::click(),
									);
									let color = if active {
										colors.text_strong
									} else if response.hovered() || response.has_focus() {
										colors.text
									} else {
										colors.muted
									};
									let pos = egui::pos2(
										tab_rect.left(),
										tab_rect.center().y - galley.size().y / 2.0,
									);
									ui.painter().galley(pos, galley, color);
									if active {
										ui.painter().rect_filled(
											egui::Rect::from_min_max(
												egui::pos2(
													tab_rect.left(),
													tab_rect.bottom() - 2.0,
												),
												egui::pos2(tab_rect.right(), tab_rect.bottom()),
											),
											1,
											colors.accent,
										);
									}
									response.widget_info(|| {
										egui::WidgetInfo::selected(
											egui::Role::Button,
											true,
											active,
											label,
										)
									});
									if response.clicked() && !active {
										self.tab = tab;
										self.focus = true;
									}
								}
							},
						);
						ui.painter().hline(
							rect.x_range(),
							tabs_rect.bottom(),
							egui::Stroke::new(1.0, colors.border),
						);

						// Search row: optional back button plus the field.
						let show_back = gifs_tab
							&& (self.gif_section != GifSection::Home
								|| !self.gif_query.trim().is_empty());
						ui.scope_builder(
							egui::UiBuilder::new()
								.max_rect(search_rect.shrink2(egui::vec2(16.0, 12.0)))
								.layout(egui::Layout::left_to_right(egui::Align::Center)),
							|ui| {
								ui.spacing_mut().item_spacing.x = 8.0;
								if show_back
									&& crate::icons::button(
										ui,
										crate::icons::Icon::ArrowLeft,
										30.0,
										"Back to GIF categories",
									)
									.clicked()
								{
									self.gif_section = GifSection::Home;
									self.gif_query.clear();
									self.gif_changed_at = None;
									self.focus = true;
								}
								egui::Frame::new()
									.fill(colors.raised)
									.corner_radius(6)
									.stroke(egui::Stroke::new(1.0, colors.border))
									.inner_margin(egui::Margin::symmetric(10, 0))
									.show(ui, |ui| {
										ui.set_width(ui.available_width());
										ui.set_height(30.0);
										ui.horizontal_centered(|ui| {
											ui.spacing_mut().item_spacing.x = 8.0;
											crate::icons::inline(
												ui,
												crate::icons::Icon::Search,
												16.0,
												colors.muted,
											);
											let (text, hint, label) = if gifs_tab {
												(
													&mut self.gif_query,
													"Search KLIPY",
													"Search GIFs on KLIPY",
												)
											} else if stickers_tab {
												(
													&mut self.stickers.query,
													"Find the perfect sticker",
													"Search stickers by name",
												)
											} else {
												(
													&mut self.query,
													"Find the perfect emoji",
													"Search emoji by name",
												)
											};
											let search = ui.add(
												egui::TextEdit::singleline(text)
													.id(ui.scope_id().with("picker-search"))
													.char_limit(64)
													.frame(egui::Frame::NONE)
													.hint_text(hint)
													.desired_width(ui.available_width()),
											);
											let search = search.accessible_name(label);
											if self.focus {
												search.request_focus();
												self.focus = false;
											}
											if search.changed() {
												if gifs_tab {
													self.gif_changed_at =
														Some(ui.input(|i| i.time));
												} else if !stickers_tab {
													self.filter();
												}
											}
										});
									});
							},
						);

						if stickers_tab {
							ui.scope_builder(
								egui::UiBuilder::new()
									.max_rect(grid_rect.shrink(8.0))
									.layout(egui::Layout::top_down(egui::Align::Min)),
								|ui| {
									if let Some(error) = state.stickers.error {
										ui.label(error);
										if ui.button("Retry sticker packs").clicked()
											&& let Some(command) = state.request_sticker_packs()
										{
											commands.push(command);
										}
									}
									let image_mode = self.images();
									if let Some(sticker) = self.stickers.show(
										ui,
										state,
										avatars,
										&mut hovered_sticker,
										image_mode,
									) {
										selected = Some(if image_mode {
											Pick::Image(model::ImageShare::Sticker {
												id: sticker.id,
												format_type: sticker.format_type,
											})
										} else {
											Pick::Sticker(sticker)
										});
									}
								},
							);
						} else if gifs_tab {
							gif_action = self.gif_body(
								ui,
								grid_rect.shrink2(egui::vec2(16.0, 4.0)),
								&gif_mode,
								state,
								avatars,
								&colors,
								demo,
								&mut hovered_gif,
							);
						} else {
							// Category rail.
							ui.painter().rect_filled(rail_rect, 0, colors.base);
							ui.scope_builder(
								egui::UiBuilder::new()
									.max_rect(rail_rect.shrink2(egui::vec2(8.0, 8.0)))
									.layout(egui::Layout::top_down(egui::Align::Center)),
								|ui| {
									ui.spacing_mut().item_spacing.y = 6.0;
									let unicode = crate::icons::toggle(
										ui,
										crate::icons::Icon::Smile,
										32.0,
										self.server.is_none(),
										"Standard emoji",
									);
									if unicode.clicked() {
										self.server = None;
										self.query.clear();
										self.filter();
									}
									egui::ScrollArea::vertical()
										.id_salt("emoji-server-rail")
										.scroll_bar_visibility(
											egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
										)
										.max_height(ui.available_height())
										.show_rows(ui, 32.0, state.guilds.len(), |ui, rows| {
											for index in rows {
												let guild = &state.guilds[index];
												let active = self.server == Some(guild.id);
												let response = ui
													.push_id(guild.id, |ui| {
														let (tab, response) = ui
															.allocate_exact_size(
																egui::Vec2::splat(32.0),
																egui::Sense::click(),
															);
														if ui.is_rect_visible(tab) {
															if active
																|| response.hovered() || response
																.has_focus()
															{
																ui.painter().rect_filled(
																	tab,
																	6,
																	colors.hover,
																);
															}
															let inner = tab.shrink(3.0);
															ui.scope_builder(
																egui::UiBuilder::new()
																	.max_rect(inner),
																|ui| {
																	avatars.show_icon(
																		ui,
																		guild.icon_key(),
																		inner.width(),
																		demo,
																		&guild.name,
																	);
																},
															);
														}
														response.widget_info(|| {
															egui::WidgetInfo::selected(
																egui::Role::Button,
																true,
																active,
																&guild.name,
															)
														});
														response.on_hover_text(&guild.name)
													})
													.inner;
												if response.clicked() {
													self.server = Some(guild.id);
													self.query.clear();
													self.filter();
												}
											}
										});
								},
							);

							// Grid.
							ui.scope_builder(
								egui::UiBuilder::new()
									.max_rect(grid_rect.shrink2(egui::vec2(8.0, 8.0)))
									.layout(egui::Layout::top_down(egui::Align::Min)),
								|ui| {
									ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
									if self.reaction.is_some() && self.query.is_empty() {
										ui.label(
											crate::design::semibold(ui, "FREQUENTLY USED", 12.0)
												.color(colors.muted),
										);
										ui.horizontal_wrapped(|ui| {
											for index in self.favorites() {
												let (text, name) = standard()[index];
												let emoji = model::ReactionEmoji {
													id: None,
													name: Some(text.into()),
												};
												let response = cell(
													ui,
													crate::emoji::image(ui.ctx(), text, 32.0),
													name,
													self.can_pick(state, &emoji),
													&colors,
												);
												if response.hovered() {
													hovered = Some((
														crate::emoji::image(ui.ctx(), text, 32.0),
														text.into(),
														shortcode(name),
													));
												}
												if response.clicked() {
													selected = Some(self.pick(emoji, text.into()));
													used = Some(text);
												}
											}
										});
										ui.add_space(12.0);
									}
									let searching = !self.query.trim().is_empty();
									let guild = self
										.server
										.and_then(|id| state.guilds.iter().find(|g| g.id == id));
									let heading = if searching {
										"Search results"
									} else {
										guild.map_or("Emoji", |g| g.name.as_str())
									};
									ui.add(
										egui::Label::new(
											crate::design::semibold(
												ui,
												heading.to_uppercase(),
												12.0,
											)
											.color(colors.muted),
										)
										.truncate(),
									);
									ui.add_space(6.0);
									let columns = ((ui.available_width() - 12.0) / CELL)
										.floor()
										.clamp(1.0, 12.0) as usize;
									self.custom.update(state, self.server, &self.query);
									let custom = &self.custom;
									let unicode = if searching || self.server.is_none() {
										self.matches.as_slice()
									} else {
										&[]
									};
									let count = custom.len() + unicode.len();
									if count == 0 {
										let text = if searching {
											"No matching emoji."
										} else if guild.is_some_and(|g| g.emojis.is_none()) {
											"This server's emoji list is not loaded yet."
										} else {
											"This server has no custom emoji."
										};
										ui.label(egui::RichText::new(text).color(colors.muted));
									}
									if searching && custom.len() == CUSTOM_LIMIT {
										ui.label(
											egui::RichText::new(
												"Showing the first 1,000 custom emoji. Refine your search for more.",
											)
											.small()
											.color(colors.muted),
										);
									}
									egui::ScrollArea::vertical()
										.id_salt(("emoji-grid", channel, self.server, &self.query))
										.max_height(ui.available_height())
										.auto_shrink([false, false])
										.show_rows(
											ui,
											CELL,
											count.div_ceil(columns),
											|ui, rows| {
												for row in rows {
													ui.horizontal(|ui| {
														for index in
															(row * columns..count).take(columns)
														{
															let (
																image,
																name,
																code,
																source,
																emoji,
																text,
															) = if let Some((guild, emoji)) =
																custom.get(state, index)
															{
																(
																	avatars.custom_image(
																		ui.ctx(),
																		emoji.id,
																		32.0,
																		demo,
																	),
																	emoji.name.clone(),
																	format!(":{}:", emoji.name),
																	Some(guild),
																	model::ReactionEmoji {
																		id: Some(emoji.id),
																		name: Some(
																			emoji.name.clone(),
																		),
																	},
																	emoji.markup(),
																)
															} else {
																let (text, name) = standard()
																	[unicode[index - custom.len()]];
																(
																	crate::emoji::image(
																		ui.ctx(),
																		text,
																		32.0,
																	),
																	text.to_owned(),
																	shortcode(name),
																	None,
																	model::ReactionEmoji {
																		id: None,
																		name: Some(text.into()),
																	},
																	text.to_owned(),
																)
															};
															let unavailable = custom
																.get(state, index)
																.and_then(|(guild, custom)| {
																	state
																		.custom_emoji_unavailable_reason(
																			channel, guild.id,
																			custom,
																		)
																});
															let enabled =
																self.can_pick(state, &emoji);
															let response = ui
																.push_id(
																	(
																		source.map(|g| g.id),
																		emoji.id,
																		&name,
																	),
																	|ui| {
																		cell(
																			ui,
																			image.clone(),
																			&code,
																			enabled,
																			&colors,
																		)
																	},
																)
																.inner;
															let response = if !enabled {
																response.on_hover_text(
																	unavailable.unwrap_or(
																		"Cannot add this reaction right now",
																	),
																)
															} else {
																response
															};
															if response.hovered()
																|| response.has_focus()
															{
																hovered = Some((image, name, code));
																hovered_source =
																	source.map(|g| g.name.as_str());
															}
															if response.clicked() && enabled {
																if source.is_none() {
																	used = Some(
																		standard()[unicode
																			[index - custom.len()]]
																		.0,
																	);
																}
																selected =
																	Some(self.pick(emoji, text));
															}
														}
													});
												}
											},
										);
								},
							);
						}

						// Footer: hovered preview or a hint.
						ui.painter().rect_filled(
							footer_rect,
							egui::CornerRadius {
								nw: 0,
								ne: 0,
								sw: 8,
								se: 8,
							},
							colors.base,
						);
						ui.scope_builder(
							egui::UiBuilder::new()
								.max_rect(footer_rect.shrink2(egui::vec2(16.0, 8.0)))
								.layout(egui::Layout::left_to_right(egui::Align::Center)),
							|ui| {
								ui.spacing_mut().item_spacing.x = 12.0;
								if stickers_tab {
									if let Some((sticker, source)) = &hovered_sticker {
										avatars.sticker_image(
											ui,
											sticker,
											egui::Vec2::splat(32.0),
											demo,
										);
										ui.vertical(|ui| {
											ui.add(
												egui::Label::new(crate::design::semibold(
													ui,
													&sticker.name,
													15.0,
												))
												.truncate(),
											);
											ui.add(
												egui::Label::new(
													egui::RichText::new(source)
														.small()
														.color(colors.muted),
												)
												.truncate(),
											);
										});
									} else {
										ui.label("Hover a sticker to preview it");
									}
									return;
								}
								if gifs_tab {
									crate::icons::inline(
										ui,
										crate::icons::Icon::Gif,
										28.0,
										if hovered_gif.is_some() {
											colors.text_strong
										} else {
											colors.muted
										},
									);
									match &hovered_gif {
										Some(title) => {
											ui.label(
												crate::design::semibold(ui, title, 15.0)
													.color(colors.text_strong),
											);
										}
										None => {
											ui.label(
												egui::RichText::new(
													"Click a GIF to send it right away",
												)
												.color(colors.muted),
											);
										}
									}
									return;
								}
								match &hovered {
									Some((image, fallback, code)) => {
										let (rect, _) = ui.allocate_exact_size(
											egui::Vec2::splat(32.0),
											egui::Sense::hover(),
										);
										paint_emoji(ui, rect, image.as_ref(), fallback);
										ui.vertical(|ui| {
											ui.spacing_mut().item_spacing.y = 0.0;
											ui.add(
												egui::Label::new(
													crate::design::semibold(ui, code, 15.0)
														.color(colors.text_strong),
												)
												.truncate(),
											);
											if let Some(source) = hovered_source {
												ui.add(
													egui::Label::new(
														egui::RichText::new(source)
															.small()
															.color(colors.muted),
													)
													.truncate(),
												);
											}
										});
									}
									None => {
										crate::icons::inline(
											ui,
											crate::icons::Icon::Smile,
											28.0,
											colors.muted,
										);
										ui.label(
											egui::RichText::new("Hover an emoji to preview it")
												.color(colors.muted),
										);
									}
								}
							},
						);
					});
			});
		if self.reaction.is_some()
			&& let Some(text) = used
		{
			self.record(text);
		}
		match gif_action {
			Some(GifAction::Toggle(gif)) => {
				state.toggle_gif_favorite(&gif);
			}
			Some(GifAction::Send(url)) => selected = Some(Pick::Send(url)),
			None => {}
		}
		// Click anywhere outside the popout (except the triggers) dismisses it.
		let clicked_outside = ui.input(|i| {
			i.pointer.any_pressed()
				&& i.pointer.interact_pos().is_some_and(|pos| {
					!area.response.rect.contains(pos)
						&& !trigger.rect.contains(pos)
						&& gif_trigger.is_none_or(|trigger| !trigger.rect.contains(pos))
				})
		});
		self.open = !clicked_outside && selected.is_none();
		if !self.open {
			self.close_gifs(state, commands);
		}
		if selected.is_some() {
			trigger.request_focus();
		}
		selected
	}

	/// Which GIF view is shown; typing pauses briefly before a search fires.
	fn gif_mode(&mut self, ui: &egui::Ui) -> GifMode {
		let now = ui.input(|i| i.time);
		if let Some(at) = self.gif_changed_at {
			if now - at >= GIF_DEBOUNCE {
				self.gif_changed_at = None;
			} else {
				ui.ctx()
					.request_repaint_after(std::time::Duration::from_millis(60));
			}
		}
		let typed = self.gif_query.trim();
		if !typed.is_empty() {
			if self.gif_changed_at.is_some() {
				GifMode::Waiting
			} else {
				GifMode::Remote(Some(typed.to_owned()))
			}
		} else {
			match &self.gif_section {
				GifSection::Home => GifMode::Home,
				GifSection::Favorites => GifMode::Favorites,
				GifSection::Trending => GifMode::Remote(None),
				GifSection::Category(name) => GifMode::Remote(Some(name.clone())),
			}
		}
	}

	/// GIF tab body: category tiles, favorites, or a masonry of remote results.
	#[allow(clippy::too_many_arguments)]
	fn gif_body(
		&mut self,
		ui: &mut egui::Ui,
		rect: egui::Rect,
		mode: &GifMode,
		state: &State,
		avatars: &mut Avatars,
		colors: &crate::design::Palette,
		demo: bool,
		hovered: &mut Option<String>,
	) -> Option<GifAction> {
		let mut action = None;
		ui.scope_builder(
			egui::UiBuilder::new()
				.max_rect(rect)
				.layout(egui::Layout::top_down(egui::Align::Min)),
			|ui| {
				ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
				let heading = match mode {
					GifMode::Home | GifMode::Waiting => None,
					GifMode::Favorites => Some("Favorites".to_owned()),
					GifMode::Remote(None) => Some("Trending GIFs".to_owned()),
					GifMode::Remote(Some(query)) => Some(query.clone()),
				};
				if let Some(heading) = heading {
					ui.add_space(4.0);
					ui.label(
						crate::design::semibold(ui, heading.to_uppercase(), 12.0)
							.color(colors.muted),
					);
					ui.add_space(8.0);
				} else {
					ui.add_space(4.0);
				}
				match mode {
					GifMode::Home => {
						if let Some(section) = gif_home(ui, state, avatars, colors, demo) {
							self.gif_section = section;
						}
					}
					GifMode::Favorites => {
						if state.gifs.favorites.is_empty() {
							empty_state(
								ui,
								colors,
								crate::icons::Icon::Star,
								"No favorites yet",
								"Hover a GIF and press the star to keep it here.",
							);
						} else {
							action = gif_grid(
								ui,
								"favorites",
								&state.gifs.favorites,
								state,
								avatars,
								colors,
								demo,
								hovered,
							);
						}
					}
					GifMode::Waiting => {
						status_row(ui, colors, true, "Searching KLIPY…");
					}
					GifMode::Remote(query) => {
						let view = state.gifs.view.as_ref().filter(|view| view.query == *query);
						match view {
							Some(view) if view.loading => {
								status_row(ui, colors, true, "Loading GIFs…");
							}
							Some(view) if view.error.is_some() => {
								status_row(ui, colors, false, view.error.unwrap_or_default());
							}
							Some(view) => {
								let gifs = view
									.page
									.as_ref()
									.map(|page| page.gifs.as_slice())
									.unwrap_or_default();
								if gifs.is_empty() {
									empty_state(
										ui,
										colors,
										crate::icons::Icon::Gif,
										"No GIFs found",
										"Try a different search term.",
									);
								} else {
									action = gif_grid(
										ui,
										("results", query.as_deref()),
										gifs,
										state,
										avatars,
										colors,
										demo,
										hovered,
									);
								}
							}
							None => {
								status_row(
									ui,
									colors,
									false,
									"GIF search needs a connected session.",
								);
							}
						}
					}
				}
			},
		);
		action
	}
}

const WIDTH: f32 = 424.0;
const HEIGHT: f32 = 476.0;
const TILE_GAP: f32 = 8.0;
const TILE_HEIGHT: f32 = 92.0;

fn status_row(ui: &mut egui::Ui, colors: &crate::design::Palette, spinner: bool, text: &str) {
	ui.add_space(24.0);
	ui.vertical_centered(|ui| {
		if spinner {
			ui.add(egui::Spinner::new().size(22.0).color(colors.muted));
			ui.add_space(8.0);
		}
		ui.label(egui::RichText::new(text).color(colors.muted));
	});
}

fn empty_state(
	ui: &mut egui::Ui,
	colors: &crate::design::Palette,
	icon: crate::icons::Icon,
	title: &str,
	detail: &str,
) {
	ui.add_space(48.0);
	ui.vertical_centered(|ui| {
		let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(64.0), egui::Sense::hover());
		ui.painter()
			.circle_filled(rect.center(), 32.0, colors.raised);
		crate::icons::paint(ui.painter(), icon, rect.shrink(18.0), colors.muted);
		ui.add_space(12.0);
		ui.label(crate::design::semibold(ui, title, 16.0).color(colors.text_strong));
		ui.add_space(4.0);
		ui.label(egui::RichText::new(detail).color(colors.muted));
	});
}

/// Object-fit cover: the part of the texture that fills `rect` without distortion.
fn cover_uv(texture: [usize; 2], rect: egui::Rect) -> egui::Rect {
	let (tw, th) = (texture[0].max(1) as f32, texture[1].max(1) as f32);
	let scale = (rect.width() / tw).max(rect.height() / th);
	let (vw, vh) = (rect.width() / scale / tw, rect.height() / scale / th);
	egui::Rect::from_min_size(
		egui::pos2((1.0 - vw) / 2.0, (1.0 - vh) / 2.0),
		egui::vec2(vw, vh),
	)
}

fn paint_cover(
	ui: &egui::Ui,
	rect: egui::Rect,
	texture: (egui::TextureId, [usize; 2]),
	radius: u8,
) {
	ui.painter().add(
		egui::epaint::RectShape::filled(rect, radius, egui::Color32::WHITE)
			.with_texture(texture.0, cover_uv(texture.1, rect)),
	);
}

/// One home tile: artwork or a flat tone behind a centered icon and label.
fn tile(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	label: &str,
	icon: Option<crate::icons::Icon>,
	texture: Option<(egui::TextureId, [usize; 2])>,
	tone: usize,
	colors: &crate::design::Palette,
) -> egui::Response {
	let response = ui.interact(
		rect,
		ui.scope_id().with(("gif-tile", label)),
		egui::Sense::click(),
	);
	let lifted = response.hovered() || response.has_focus();
	if ui.is_rect_visible(rect) {
		match texture {
			Some(texture) => {
				paint_cover(ui, rect, texture, 8);
				ui.painter().rect_filled(
					rect,
					8,
					egui::Color32::from_black_alpha(if lifted { 80 } else { 128 }),
				);
			}
			None => {
				// Flat, muted tones stand in for the provider's category artwork.
				let hue = ((tone * 5) % 8) as f32 / 8.0 + 0.55;
				let mut base = egui::ecolor::Hsva::new(hue % 1.0, 0.38, 0.46, 1.0);
				if lifted {
					base.v += 0.08;
				}
				ui.painter().rect_filled(rect, 8, egui::Color32::from(base));
			}
		}
		if lifted {
			ui.painter().rect_stroke(
				rect,
				8,
				egui::Stroke::new(2.0, colors.text_strong),
				egui::StrokeKind::Inside,
			);
		}
		let family = crate::design::semibold_family(ui.ctx());
		let galley = ui.painter().layout_no_wrap(
			label.to_owned(),
			egui::FontId::new(15.0, family),
			egui::Color32::WHITE,
		);
		let icon_size = if icon.is_some() { 22.0 } else { 0.0 };
		let total = galley.size().x + icon_size + if icon.is_some() { 8.0 } else { 0.0 };
		let mut cursor = rect.center().x - total / 2.0;
		if let Some(icon) = icon {
			let icon_rect = egui::Rect::from_center_size(
				egui::pos2(cursor + icon_size / 2.0, rect.center().y),
				egui::Vec2::splat(icon_size),
			);
			crate::icons::paint(ui.painter(), icon, icon_rect, egui::Color32::WHITE);
			cursor += icon_size + 8.0;
		}
		ui.painter().galley(
			egui::pos2(cursor, rect.center().y - galley.size().y / 2.0),
			galley,
			egui::Color32::WHITE,
		);
	}
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
	response
}

/// Home: Favorites and Trending tiles, then one tile per trending category.
fn gif_home(
	ui: &mut egui::Ui,
	state: &State,
	avatars: &mut Avatars,
	colors: &crate::design::Palette,
	demo: bool,
) -> Option<GifSection> {
	let trending = state.gifs.view.as_ref().filter(|view| view.query.is_none());
	let page = trending.and_then(|view| view.page.as_ref());
	let categories: Vec<&str> = page
		.map(|page| page.categories.iter().map(String::as_str).collect())
		.unwrap_or_default();
	let favorite_art = state
		.gifs
		.favorites
		.first()
		.and_then(|gif| avatars.gif_texture(ui.ctx(), gif, demo));
	let trending_art = page
		.and_then(|page| page.gifs.first())
		.and_then(|gif| avatars.gif_texture(ui.ctx(), gif, demo));
	let mut chosen = None;
	let rows = 1 + categories.len().div_ceil(2);
	let total = rows as f32 * TILE_HEIGHT + (rows.saturating_sub(1)) as f32 * TILE_GAP + 8.0;
	egui::ScrollArea::vertical()
		.id_salt("gif-home")
		.auto_shrink([false, false])
		.show(ui, |ui| {
			let width = ui.available_width();
			let column = (width - TILE_GAP) / 2.0;
			let (area, _) = ui.allocate_exact_size(egui::vec2(width, total), egui::Sense::hover());
			let cell = |index: usize| {
				let (col, row) = (index % 2, index / 2);
				egui::Rect::from_min_size(
					egui::pos2(
						area.left() + col as f32 * (column + TILE_GAP),
						area.top() + row as f32 * (TILE_HEIGHT + TILE_GAP),
					),
					egui::vec2(column, TILE_HEIGHT),
				)
			};
			if tile(
				ui,
				cell(0),
				"Favorites",
				Some(crate::icons::Icon::StarFill),
				favorite_art,
				0,
				colors,
			)
			.clicked()
			{
				chosen = Some(GifSection::Favorites);
			}
			if tile(
				ui,
				cell(1),
				"Trending GIFs",
				Some(crate::icons::Icon::Fire),
				trending_art,
				1,
				colors,
			)
			.clicked()
			{
				chosen = Some(GifSection::Trending);
			}
			for (index, name) in categories.iter().enumerate() {
				if tile(ui, cell(index + 2), name, None, None, index + 2, colors).clicked() {
					chosen = Some(GifSection::Category((*name).to_owned()));
				}
			}
			if categories.is_empty() && trending.is_some_and(|view| view.loading) {
				let below = egui::Rect::from_min_size(
					egui::pos2(area.left(), cell(2).top()),
					egui::vec2(width, 40.0),
				);
				ui.scope_builder(
					egui::UiBuilder::new()
						.max_rect(below)
						.layout(egui::Layout::left_to_right(egui::Align::Center)),
					|ui| {
						ui.spacing_mut().item_spacing.x = 8.0;
						ui.add(egui::Spinner::new().size(16.0).color(colors.muted));
						ui.label(
							egui::RichText::new("Loading trending categories…").color(colors.muted),
						);
					},
				);
			}
		});
	chosen
}

/// Two-column masonry of static previews; the star toggles a favorite, a click sends.
#[allow(clippy::too_many_arguments)]
fn gif_grid(
	ui: &mut egui::Ui,
	salt: impl std::hash::Hash + std::fmt::Debug,
	gifs: &[model::Gif],
	state: &State,
	avatars: &mut Avatars,
	colors: &crate::design::Palette,
	demo: bool,
	hovered: &mut Option<String>,
) -> Option<GifAction> {
	let mut action = None;
	egui::ScrollArea::vertical()
		.id_salt(("gif-grid", salt))
		.auto_shrink([false, false])
		.show(ui, |ui| {
			let width = ui.available_width();
			let column = (width - TILE_GAP) / 2.0;
			let mut heights = [0.0f32; 2];
			let mut placed = Vec::with_capacity(gifs.len());
			for gif in gifs {
				let height =
					(column * gif.height as f32 / gif.width.max(1) as f32).clamp(64.0, 320.0);
				let col = if heights[1] < heights[0] { 1 } else { 0 };
				placed.push((col, heights[col], height));
				heights[col] += height + TILE_GAP;
			}
			let total = heights[0].max(heights[1]).max(1.0);
			let (area, _) = ui.allocate_exact_size(egui::vec2(width, total), egui::Sense::hover());
			for (gif, (col, y, height)) in gifs.iter().zip(placed) {
				let rect = egui::Rect::from_min_size(
					egui::pos2(
						area.left() + col as f32 * (column + TILE_GAP),
						area.top() + y,
					),
					egui::vec2(column, height),
				);
				if !ui.is_rect_visible(rect) {
					continue;
				}
				let id = ui.scope_id().with(("gif", &gif.id));
				let response = ui.interact(rect, id, egui::Sense::click());
				let star_rect = egui::Rect::from_min_size(
					egui::pos2(rect.right() - 32.0, rect.top() + 6.0),
					egui::Vec2::splat(26.0),
				);
				let favorite = state.is_gif_favorite(gif);
				let lifted = response.hovered() || response.has_focus();
				let star = if lifted || favorite {
					Some(ui.interact(star_rect, id.with("star"), egui::Sense::click()))
				} else {
					None
				};
				match avatars.gif_texture(ui.ctx(), gif, demo) {
					Some(texture) => paint_cover(ui, rect, texture, 8),
					None => {
						ui.painter().rect_filled(rect, 8, colors.raised);
						crate::icons::paint(
							ui.painter(),
							crate::icons::Icon::Gif,
							egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(22.0)),
							colors.muted,
						);
					}
				}
				if lifted {
					ui.painter().rect_stroke(
						rect,
						8,
						egui::Stroke::new(2.0, colors.text_strong),
						egui::StrokeKind::Inside,
					);
					*hovered = Some(if gif.title.is_empty() {
						"GIF".to_owned()
					} else {
						gif.title.clone()
					});
				}
				if let Some(star) = &star {
					let hot = star.hovered() || star.has_focus();
					ui.painter().rect_filled(
						star_rect,
						6,
						egui::Color32::from_black_alpha(if hot { 210 } else { 160 }),
					);
					let (icon, color) = if favorite {
						(crate::icons::Icon::StarFill, colors.warning)
					} else {
						(crate::icons::Icon::Star, egui::Color32::WHITE)
					};
					crate::icons::paint(ui.painter(), icon, star_rect.shrink(5.0), color);
					star.widget_info(|| {
						egui::WidgetInfo::selected(egui::Role::CheckBox, true, favorite, "Favorite")
					});
				}
				let label = if gif.title.is_empty() {
					"Send GIF".to_owned()
				} else {
					format!("Send GIF: {}", gif.title)
				};
				response
					.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
				if star.as_ref().is_some_and(|star| star.clicked()) {
					action = Some(GifAction::Toggle(gif.clone()));
				} else if response.clicked() && !star.as_ref().is_some_and(|s| s.hovered()) {
					action = Some(GifAction::Send(gif.url.clone()));
				}
			}
		});
	action
}

/// `:short_code:` for every bundled emoji, in `standard()` order, built once for autocomplete.
pub(crate) fn shortcodes() -> &'static [String] {
	static CODES: OnceLock<Vec<String>> = OnceLock::new();
	CODES.get_or_init(|| standard().iter().map(|(_, name)| shortcode(name)).collect())
}

/// Discord-style `:short_code:` rendered from the bundled CLDR name.
pub(crate) fn shortcode(name: &str) -> String {
	let mut code = String::with_capacity(name.len() + 2);
	code.push(':');
	let mut last_underscore = true;
	for c in name.chars() {
		if c.is_alphanumeric() {
			code.extend(c.to_lowercase());
			last_underscore = false;
		} else if !last_underscore {
			code.push('_');
			last_underscore = true;
		}
	}
	if code.ends_with('_') {
		code.pop();
	}
	code.push(':');
	code
}

fn paint_emoji(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	image: Option<&egui::Image<'static>>,
	text: &str,
) {
	if let Some(image) = image {
		image.paint_at(ui, rect);
	} else {
		ui.painter().text(
			rect.center(),
			egui::Align2::CENTER_CENTER,
			text,
			egui::FontId::proportional((rect.height() * 0.7).max(10.0)),
			ui.visuals().text_color(),
		);
	}
}

/// One grid cell: hover highlight plus the emoji image (or its name when no image exists).
fn cell(
	ui: &mut egui::Ui,
	image: Option<egui::Image<'static>>,
	name: &str,
	enabled: bool,
	colors: &crate::design::Palette,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(
		egui::Vec2::splat(CELL),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	if ui.is_rect_visible(rect) {
		if response.hovered() || response.has_focus() {
			ui.painter().rect_filled(rect, 6, colors.hover);
		}
		let inner = rect.shrink(4.0);
		match image {
			Some(image) => {
				let image = if enabled {
					image
				} else {
					image.tint(egui::Color32::from_white_alpha(96))
				};
				image.paint_at(ui, inner);
			}
			None => {
				let short: String = name.chars().take(3).collect();
				ui.painter().text(
					rect.center(),
					egui::Align2::CENTER_CENTER,
					short,
					egui::FontId::proportional(11.0),
					colors.muted,
				);
			}
		}
	}
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
	response
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	#[ignore = "release picker frame benchmark; ten warmup frames and one warmup/five measured batches"]
	fn custom_picker_frame_benchmark() {
		const FRAMES: usize = 200;
		for (label, guild_count, query, selected_server, churn) in [
			("server", 1, "", true, false),
			("search-hit", 100, "needle", false, false),
			("search-miss", 100, "missing_emoji", false, false),
			("search-many", 100, "emoji", false, false),
			("search-hit-churn", 100, "needle", false, true),
		] {
			let mut state = test_support::demo_state();
			let template = state.guilds[0].clone();
			let emoji = template.emojis.as_ref().unwrap()[0].clone();
			state.guilds = (0..guild_count)
				.map(|guild| model::Guild {
					id: Id(template.id.0 + guild),
					name: format!("Synthetic server {guild}"),
					icon: None,
					stickers: None,
					emojis: Some(
						(0..500)
							.rev()
							.map(|index| model::CustomEmoji {
								id: Id(100_000 + guild * 500 + index),
								name: if guild + 1 == guild_count && index == 499 {
									"needle".into()
								} else {
									format!("emoji_{guild}_{index}")
								},
								..emoji.clone()
							})
							.collect(),
					),
				})
				.collect();
			state.invalidate_navigation();
			let channel = state.selected.unwrap();
			let mut picker = Picker {
				open: true,
				channel: Some(channel),
				generation: state.generation,
				server: selected_server.then_some(template.id),
				query: query.into(),
				..Picker::default()
			};
			picker.filter();
			let ctx = egui::Context::default();
			let mut avatars = Avatars::default();
			let mut commands = Vec::new();
			let mut frame_number = 0;
			let mut frame = || {
				// Model accepted-event invalidation without timing the reducer itself.
				if churn {
					state.revision += 1;
				}
				frame_number += 1;
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(900.0, 700.0),
						)),
						time: Some(frame_number as f64 / 60.0),
						..Default::default()
					},
					|ui| {
						std::hint::black_box(picker.show(
							ui,
							&mut state,
							channel,
							&mut avatars,
							&mut commands,
						));
					},
				);
				std::hint::black_box(output.shapes.len());
				output.drop_without_applying_deltas();
				commands.clear();
			};
			for _ in 0..10 {
				frame();
			}
			let mut samples = Vec::with_capacity(5);
			for batch in 0..6 {
				let start = std::time::Instant::now();
				for _ in 0..FRAMES {
					frame();
				}
				let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
				if batch != 0 {
					samples.push(elapsed);
				}
			}
			assert!(picker.open);
			assert_eq!(picker.query, query);
			println!(
				"custom_picker {label}: guilds={guild_count}, emojis_per_guild=500, frames={FRAMES}, samples_ms={samples:?}"
			);
		}
	}

	#[test]
	fn custom_match_cache_tracks_catalog_scope_and_reuses_unchanged_results() {
		use client_core::{Envelope, Event};
		fn apply(state: &mut State, event: Event) {
			state.apply(Envelope {
				generation: state.generation,
				event,
			});
		}
		let mut state = test_support::demo_state();
		let guild = state.guilds[0].id;
		let mut cache = CustomMatches::default();
		assert!(cache.update(&state, Some(guild), ""));
		assert_eq!(cache.len(), state.guilds[0].emojis.as_ref().unwrap().len());
		let allocation = cache.entries.as_ptr();
		assert!(!cache.update(&state, Some(guild), ""));
		assert_eq!(cache.entries.as_ptr(), allocation);
		assert!(cache.update(&state, None, ""));
		assert_eq!(cache.len(), 0);
		assert!(cache.update(&state, None, "  NEEDLE  "));
		assert_eq!(cache.len(), 0);
		apply(
			&mut state,
			Event::GuildChanged(model::GuildPatch {
				id: guild,
				name: model::Patch::Value("Needle server".into()),
				icon: model::Patch::Absent,
			}),
		);
		assert!(cache.update(&state, None, "  NEEDLE  "));
		assert!(
			cache.len() > 0,
			"renaming a guild must invalidate cached misses"
		);
		let mut emoji = state.guilds[0].emojis.as_ref().unwrap()[0].clone();
		emoji.id = Id(99);
		emoji.name = "brand_new".into();
		emoji.animated = true;
		emoji.available = false;
		let mut other = emoji.clone();
		other.id = Id(100);
		apply(
			&mut state,
			Event::GuildEmojis {
				guild,
				emojis: vec![other, emoji.clone()],
			},
		);
		assert!(cache.update(&state, None, "  NEEDLE  "));
		assert_eq!(cache.len(), 2);
		assert_eq!(cache.get(&state, 0).unwrap().1, &emoji);
		assert_eq!(cache.get(&state, 1).unwrap().1.id, Id(100));
		assert!(cache.update(&state, None, "brand_new"));
		assert_eq!(cache.len(), 2);
		apply(
			&mut state,
			Event::GuildEmojis {
				guild,
				emojis: vec![],
			},
		);
		assert!(cache.update(&state, None, "brand_new"));
		assert_eq!(cache.len(), 0, "removed emoji must not remain selectable");
		let mut joined = state.guilds[0].clone();
		joined.id = Id(555);
		joined.stickers = None;
		joined.emojis = Some(vec![emoji.clone()]);
		apply(&mut state, Event::GuildJoined(joined));
		assert!(cache.update(&state, None, "brand_new"));
		assert_eq!(cache.get(&state, 0).unwrap().0.id, Id(555));
		let mut guilds = state.guilds.clone();
		guilds.reverse();
		let user = state.user.clone().unwrap();
		let channels = state.channels.clone();
		let generation = state.generation;
		apply(
			&mut state,
			Event::Ready {
				user,
				guilds,
				channels,
				permissions: Default::default(),
			},
		);
		assert_eq!(state.generation, generation);
		assert!(cache.update(&state, None, "brand_new"));
		assert_eq!(cache.get(&state, 0).unwrap().0.id, Id(555));
		assert_eq!(cache.get(&state, 0).unwrap().1, &emoji);
		state.apply(Envelope {
			generation: generation.wrapping_sub(1),
			event: Event::GuildEmojis {
				guild: Id(555),
				emojis: vec![],
			},
		});
		assert!(!cache.update(&state, None, "brand_new"));
		// Independent account/session identities must invalidate even if revisions coincide.
		state.user.as_mut().unwrap().id = Id(777);
		assert!(cache.update(&state, None, "brand_new"));
		state.generation += 1;
		assert!(cache.update(&state, None, "brand_new"));
		state.logout();
		assert!(cache.update(&state, None, "brand_new"));
		assert_eq!(cache.len(), 0);
	}

	#[test]
	fn custom_match_cache_bounds_results_and_retained_query_bytes() {
		let mut state = test_support::demo_state();
		let emoji = state.guilds[0].emojis.as_ref().unwrap()[0].clone();
		let mut guild = state.guilds[0].clone();
		guild.emojis = Some(
			(1..=CUSTOM_LIMIT)
				.rev()
				.map(|id| model::CustomEmoji {
					id: Id(id as u64),
					name: format!("emoji_{id}"),
					..emoji.clone()
				})
				.collect(),
		);
		let mut second = guild.clone();
		second.id = Id(guild.id.0 + 1);
		state.guilds = vec![guild, second];
		let mut cache = CustomMatches::default();
		cache.update(&state, None, "emoji");
		assert_eq!(cache.len(), CUSTOM_LIMIT);
		assert_eq!(
			std::mem::size_of_val(cache.entries.as_ref()),
			CUSTOM_LIMIT * size_of::<(usize, usize)>()
		);
		assert_eq!(cache.get(&state, 0).unwrap().1.id, Id(1));
		assert_eq!(
			cache.get(&state, CUSTOM_LIMIT - 1).unwrap().1.id,
			Id(CUSTOM_LIMIT as u64)
		);
		let query = "😀".repeat(64);
		assert!(cache.update(&state, None, &query));
		assert_eq!(cache.query.len(), 256);
		assert!(!cache.update(&state, None, &query));
		assert!(cache.update(&state, None, &"😀".repeat(65)));
		assert!(cache.query.is_empty());
		assert!(cache.key.is_none());
	}

	#[test]
	fn image_sharing_requires_enabled_plugin_and_never_changes_reactions() {
		let state = test_support::demo_state();
		let mut picker = Picker {
			channel: state.selected,
			image_sharing_enabled: true,
			..Default::default()
		};
		let emoji = model::ReactionEmoji {
			id: Some(Id(999)),
			name: Some("wave".into()),
		};
		assert!(picker.can_pick(&state, &emoji));
		assert!(matches!(
			picker.pick(emoji.clone(), "<a:wave:999>".into()),
			Pick::Image(model::ImageShare::Emoji {
				id: Id(999),
				animated: true
			})
		));
		picker.image_sharing_enabled = false;
		assert!(!picker.can_pick(&state, &emoji));
		assert!(matches!(
			picker.pick(emoji.clone(), "<a:wave:999>".into()),
			Pick::Insert(_)
		));
		picker.image_sharing_enabled = true;
		picker.reaction = Some((Id(500), egui::Rect::NOTHING, egui::Id::unique("reaction")));
		assert!(matches!(
			picker.pick(emoji, "<a:wave:999>".into()),
			Pick::React(Id(500), _)
		));
	}

	#[test]
	fn reaction_search_keyboard_selection_permissions_and_session_reset() {
		let ctx = egui::Context::default();
		crate::emoji::install(&ctx).unwrap();
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let message = Id(500);
		state.drafts.insert(channel, "Keep my draft".into());
		let mut picker = Picker::default();
		let mut avatars = Avatars::default();
		let anchor = egui::Rect::from_min_size(egui::pos2(700.0, 600.0), egui::vec2(28.0, 28.0));
		let key = |key| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		let frame = |picker: &mut Picker, state: &mut State, avatars: &mut Avatars, events| {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| picker.show_reaction(ui, state, avatars, &mut commands),
			);
			output.drop_without_applying_deltas();
			commands
		};
		for custom in [false, true] {
			state.reactions = Default::default();
			picker.open_reaction(&state, message, anchor, egui::Id::unique("synthetic-react"));
			picker.server = custom.then_some(state.guilds[0].id);
			for _ in 0..3 {
				assert!(frame(&mut picker, &mut state, &mut avatars, vec![]).is_empty());
			}
			let query = if custom { "serein_wave" } else { "rocket" };
			frame(
				&mut picker,
				&mut state,
				&mut avatars,
				vec![egui::Event::Text(query.into())],
			);
			assert_eq!(picker.query, query);
			let mut selected = false;
			for _ in 0..12 {
				frame(
					&mut picker,
					&mut state,
					&mut avatars,
					vec![key(egui::Key::Tab)],
				);
				if ctx
					.memory(|m| m.focused())
					.and_then(|id| ctx.read_response(id))
					.is_some_and(|r| r.rect.size() == egui::Vec2::splat(CELL))
				{
					let commands = frame(
						&mut picker,
						&mut state,
						&mut avatars,
						vec![key(egui::Key::Enter)],
					);
					assert_eq!(commands.len(), 1);
					assert!(
						matches!(&commands[0], Command::Reactions(client_core::reactions::Command::Set {
						message: target, emoji, ..
					}) if *target == message && emoji.id == custom.then_some(Id(9001))
						&& emoji.name.as_deref() == Some(if custom { "serein_wave" } else { "🚀" }))
					);
					selected = true;
					break;
				}
			}
			assert!(selected && !picker.open);
			assert_eq!(state.drafts[&channel], "Keep my draft");
		}
		assert_eq!(standard()[picker.favorites()[0]].0, "🚀");
		state.reactions = Default::default();
		picker.open_reaction(&state, message, anchor, egui::Id::unique("synthetic-react"));
		frame(
			&mut picker,
			&mut state,
			&mut avatars,
			vec![key(egui::Key::Escape)],
		);
		assert!(!picker.open);
		picker.open_reaction(&state, message, anchor, egui::Id::unique("synthetic-react"));
		state.gateway_connected = false;
		assert!(!picker.can_pick(
			&state,
			&model::ReactionEmoji {
				id: None,
				name: Some("🚀".into())
			}
		));
		state.selected = Some(Id(21));
		frame(&mut picker, &mut state, &mut avatars, vec![]);
		assert!(!picker.open && picker.reaction.is_none());
		assert!(!picker.frequent.is_empty());
		state.generation += 1;
		frame(&mut picker, &mut state, &mut avatars, vec![]);
		assert!(picker.frequent.is_empty());
	}

	#[test]
	fn reaction_selection_never_focuses_a_missing_accesskit_node() {
		// AccessKit's consumer panics when a tree update focuses an id that is not in the
		// node list, so every frame must keep the focused widget registered as a node.
		let ctx = egui::Context::default();
		ctx.enable_accesskit();
		crate::emoji::install(&ctx).unwrap();
		let mut state = test_support::demo_state();
		let message = Id(500);
		let mut picker = Picker::default();
		let mut avatars = Avatars::default();
		let anchor = egui::Rect::from_min_size(egui::pos2(700.0, 600.0), egui::vec2(28.0, 28.0));
		let key = |key| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		let frame = |picker: &mut Picker, state: &mut State, avatars: &mut Avatars, events| {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| picker.show_reaction(ui, state, avatars, &mut commands),
			);
			if let Some(update) = &output.platform_output.accesskit_update {
				assert!(
					update.nodes.iter().any(|(id, _)| *id == update.focus),
					"focused id {:?} is missing from the AccessKit node list",
					update.focus
				);
			}
			output.drop_without_applying_deltas();
			commands
		};
		// The real trigger is the hover-toolbar button, which is not rendered while the
		// popout covers it; a synthetic id reproduces that absent node.
		picker.open_reaction(&state, message, anchor, egui::Id::unique("synthetic-react"));
		for _ in 0..3 {
			assert!(frame(&mut picker, &mut state, &mut avatars, vec![]).is_empty());
		}
		frame(
			&mut picker,
			&mut state,
			&mut avatars,
			vec![egui::Event::Text("rocket".into())],
		);
		let mut selected = false;
		for _ in 0..12 {
			frame(
				&mut picker,
				&mut state,
				&mut avatars,
				vec![key(egui::Key::Tab)],
			);
			if ctx
				.memory(|m| m.focused())
				.and_then(|id| ctx.read_response(id))
				.is_some_and(|r| r.rect.size() == egui::Vec2::splat(CELL))
			{
				// Selecting hands focus back to the trigger, which must stay valid.
				let commands = frame(
					&mut picker,
					&mut state,
					&mut avatars,
					vec![key(egui::Key::Enter)],
				);
				assert_eq!(commands.len(), 1);
				selected = true;
				break;
			}
		}
		assert!(selected && !picker.open);
	}

	#[test]
	fn sticker_picker_search_keyboard_send_preserves_draft_and_resets_session() {
		for images in [false, true] {
			let ctx = egui::Context::default();
			crate::emoji::install(&ctx).unwrap();
			let mut state = test_support::demo_state();
			test_support::seed_stickers(&mut state);
			let channel = state.selected.unwrap();
			state.drafts.insert(channel, "Keep my draft".into());
			let mut picker = Picker {
				image_sharing_enabled: images,
				..Default::default()
			};
			picker.open_stickers(None);
			let mut avatars = Avatars::default();
			let mut frame = |picker: &mut Picker, state: &mut State, events| {
				let mut selected = None;
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(900.0, 700.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						selected = picker.show(ui, state, channel, &mut avatars, &mut Vec::new());
					},
				);
				output.drop_without_applying_deltas();
				selected
			};
			for _ in 0..3 {
				frame(&mut picker, &mut state, vec![]);
			}
			frame(
				&mut picker,
				&mut state,
				vec![egui::Event::Text("Sle".into())],
			);
			frame(
				&mut picker,
				&mut state,
				vec![egui::Event::Text("ep".into())],
			);
			assert_eq!(picker.stickers.query, "Sleep");
			let key = |key| egui::Event::Key {
				key,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			};
			let mut picked = None;
			for _ in 0..20 {
				frame(&mut picker, &mut state, vec![key(egui::Key::Tab)]);
				if ctx
					.memory(|m| m.focused())
					.and_then(|id| ctx.read_response(id))
					.is_some_and(|r| {
						r.rect.width() >= 70.0 && (r.rect.width() - r.rect.height()).abs() < 0.1
					}) {
					picked = frame(&mut picker, &mut state, vec![key(egui::Key::Enter)]);
					break;
				}
			}
			if images {
				assert!(matches!(
					picked,
					Some(Pick::Image(model::ImageShare::Sticker { id: Id(9201), .. }))
				));
			} else {
				let Some(Pick::Sticker(sticker)) = picked else {
					panic!("keyboard sticker selection");
				};
				assert_eq!(sticker.name, "Sleep");
				assert!(matches!(
					state.prepare_sticker_send(&sticker),
					Some(Command::Send {
						sticker: Some(Id(9201)),
						..
					})
				));
			}
			assert_eq!(state.drafts[&channel], "Keep my draft");
			assert!(!picker.open);
			state.generation += 1;
			picker.sync(&state, Some(channel));
			assert!(picker.stickers.query.is_empty());
		}
	}

	#[test]
	fn gif_search_keeps_focus_when_the_back_button_appears() {
		let ctx = egui::Context::default();
		crate::emoji::install(&ctx).unwrap();
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let mut picker = Picker {
			open: true,
			focus: true,
			channel: Some(channel),
			generation: state.generation,
			tab: Tab::Gifs,
			..Picker::default()
		};
		let mut avatars = Avatars::default();
		let mut frame = |picker: &mut Picker, state: &mut State, events| {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					picker.show(ui, state, channel, &mut avatars, &mut commands);
				},
			);
			output.drop_without_applying_deltas();
		};

		frame(&mut picker, &mut state, vec![]);
		frame(&mut picker, &mut state, vec![egui::Event::Text("c".into())]);
		frame(&mut picker, &mut state, vec![]);
		frame(&mut picker, &mut state, vec![egui::Event::Text("a".into())]);

		assert_eq!(picker.gif_query, "ca");
	}

	#[test]
	fn favorite_usage_is_ranked_deduplicated_and_bounded() {
		let mut picker = Picker::default();
		assert_eq!(picker.favorites().len(), 8);
		assert_eq!(standard()[picker.favorites()[0]].0, "👍");
		picker.record("🚀");
		picker.record("❤️");
		picker.record("🚀");
		assert_eq!(standard()[picker.favorites()[0]].0, "🚀");
		let unique: std::collections::BTreeSet<_> = picker.favorites().into_iter().collect();
		assert_eq!(unique.len(), 8);
		for (text, _) in standard().iter().take(100) {
			picker.record(text);
		}
		assert_eq!(picker.frequent.len(), 32);
		assert_eq!(picker.frequent.capacity(), 32);
		picker.frequent[0].1 = u32::MAX;
		picker.record(standard()[picker.frequent[0].0].0);
		assert_eq!(picker.frequent[0].1, u32::MAX);
	}

	#[test]
	fn insertion_replaces_unicode_selection_and_respects_character_and_capacity_budgets() {
		use egui::text::{CCursor, CCursorRange};
		let mut draft = "前👩🏽‍💻後".to_owned();
		let selection = Some(CCursorRange::two(CCursor::new(5), CCursor::new(1)));
		assert_eq!(insert(&mut draft, "❤️", selection, 0), Some(3));
		assert_eq!(draft, "前❤️後");
		let markup = "<a:party_blob:123456789>";
		assert_eq!(
			insert(&mut draft, markup, None, 100),
			Some(4 + markup.len())
		);
		assert_eq!(draft, format!("前❤️後{markup}"));
		let end = draft.chars().count();
		assert_eq!(
			insert(
				&mut draft,
				"😀",
				Some(CCursorRange::one(CCursor::new(usize::MAX))),
				100
			),
			Some(end + 1)
		);
		assert!(draft.ends_with("😀"));

		let mut draft = "a".repeat(client_core::MAX_CONTENT);
		assert_eq!(insert(&mut draft, "😀", None, 100), None);
		assert_eq!(draft.len(), client_core::MAX_CONTENT);
		let mut draft = String::new();
		assert_eq!(insert(&mut draft, "😀", None, 3), None);
		assert!(draft.is_empty());
		assert_eq!(insert(&mut draft, "😀", None, 4), Some(1));
		assert_eq!(draft.capacity(), 4);
		let selection = Some(CCursorRange::two(CCursor::new(0), CCursor::new(1)));
		assert_eq!(insert(&mut draft, "👍", selection, 0), Some(1));
		assert_eq!(draft, "👍");
		assert_eq!(draft.capacity(), 4);
	}

	#[test]
	fn completed_shortcode_becomes_unicode_at_the_caret() {
		let mut draft = "look :eyes: here :eyes:".to_owned();
		assert_eq!(complete_shortcode(&mut draft, 11, 0), Some(6));
		assert_eq!(draft, "look 👀 here :eyes:");
		assert_eq!(complete_shortcode(&mut draft, 18, 0), Some(13));
		assert_eq!(draft, "look 👀 here 👀");
		for literal in ["word:eyes:", "https:", "<:eyes:", ":unknown:"] {
			let mut draft = literal.to_owned();
			let cursor = draft.chars().count();
			assert_eq!(complete_shortcode(&mut draft, cursor, 0), None);
		}
	}

	#[test]
	fn palette_search_preserves_complete_sequences_and_is_bounded() {
		let atlas = include_str!("../../../assets/twemoji/index.tsv");
		let atlas: std::collections::BTreeSet<_> = atlas
			.lines()
			.map(|l| l.split_once('\t').unwrap().0)
			.collect();
		assert_eq!(standard().len(), 3953);
		assert!(NAMES.len() < 300_000);
		for (text, name) in standard() {
			assert!(atlas.contains(text.replace('\u{fe0f}', "").as_str()));
			assert!(!name.is_empty());
		}
		let mut picker = Picker {
			query: "WOMAN TECHNOLOGIST".into(),
			..Default::default()
		};
		picker.filter();
		assert!(picker.matches.iter().any(|&i| standard()[i].0 == "👩🏽‍💻"));
		picker.query = "❤️".into();
		picker.filter();
		assert!(picker.matches.iter().any(|&i| standard()[i].0 == "❤️"));
		picker.query = "not an emoji name".into();
		picker.filter();
		assert!(picker.matches.is_empty());
	}

	#[test]
	fn server_grid_only_requests_visible_images_and_resets_on_navigation() {
		let mut state = State {
			guilds: vec![model::Guild {
				stickers: None,
				id: Id(1),
				name: "Synthetic server".into(),
				icon: None,
				emojis: Some(
					(1..=1000)
						.map(|id| model::CustomEmoji {
							id: Id(id),
							name: format!("emoji_{id}"),
							animated: false,
							available: true,
							managed: false,
							roles: Some(vec![]),
						})
						.collect(),
				),
			}],
			channels: vec![model::Channel {
				id: Id(2),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic channel".into(),
				kind: 1,
				recipients: vec![],
				member_list_id: None,
				message_count: None,
				icon: None,
				last_message: None,
			}],
			..State::default()
		};
		let mut custom = CustomMatches::default();
		custom.update(&state, None, "");
		assert_eq!(custom.len(), 0);
		custom.update(&state, None, "SYNTHETIC SERVER");
		assert_eq!(custom.len(), CUSTOM_LIMIT);
		assert_eq!(custom.get(&state, 0).unwrap().1.id, Id(1));
		custom.update(&state, None, "emoji_999");
		assert_eq!(custom.get(&state, 0).unwrap().1.id, Id(999));
		let mut picker = Picker {
			open: true,
			server: Some(Id(1)),
			channel: Some(Id(2)),
			generation: state.generation,
			..Picker::default()
		};
		let mut avatars = Avatars::default();
		let mut commands = Vec::new();
		let ctx = egui::Context::default();
		for _ in 0..3 {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(640.0, 480.0),
					)),
					..Default::default()
				},
				|ui| {
					assert!(
						picker
							.show(ui, &mut state, Id(2), &mut avatars, &mut commands)
							.is_none()
					);
				},
			);
			output.textures_delta.clear();
		}
		let requests = avatars.take_requests();
		assert!(!requests.is_empty());
		assert!(
			requests.len() < 100,
			"offscreen emoji must not queue image requests"
		);
		picker.query = "old query".into();
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(640.0, 480.0),
				)),
				..Default::default()
			},
			|ui| {
				assert!(
					picker
						.show(ui, &mut state, Id(3), &mut avatars, &mut commands)
						.is_none()
				);
			},
		);
		output.textures_delta.clear();
		assert!(!picker.open && picker.server.is_none() && picker.query.is_empty());
		assert_eq!(picker.custom.len(), 0);
	}
}
