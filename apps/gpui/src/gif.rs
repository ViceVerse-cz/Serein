//! GIF picker for the composer's GIF button, following the main app's GIFs tab: trending and
//! category tiles, a searched two-column masonry of still previews (first frame only), and a
//! click that sends the GIF's provider link as its own message. Results come from
//! `client_core::gifs`; the offline preview answers with synthetic pages and drawn stills.
use crate::theme::{Icon, color, icon, palette, solid};
use crate::{Serein, input};
use gpui::{prelude::*, *};
use std::cell::Cell;
use std::time::{Duration, Instant};

const WIDTH: f32 = 424.;
const HEIGHT: f32 = 476.;
const PAD: f32 = 12.;
const TILE_GAP: f32 = 8.;
const TILE_HEIGHT: f32 = 92.;
/// Typing pauses this long before a search is sent, as in the main app.
const DEBOUNCE: Duration = Duration::from_millis(300);

thread_local! {
	/// When a click outside closed the picker, so that click on the GIF button does not reopen it.
	static CLOSED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

#[derive(Clone, PartialEq, Debug)]
pub enum Section {
	Home,
	Favorites,
	Trending,
	Category(String),
}

pub struct Picker {
	position: Point<Pixels>,
	search: Entity<input::Input>,
	section: Section,
	/// Last keystroke, while a search waits for the debounce.
	typed_at: Option<Instant>,
}

/// What the body shows for the current query and section.
#[derive(PartialEq, Debug)]
enum Mode {
	Home,
	Favorites,
	Waiting,
	Remote(Option<String>),
}

fn mode(query: &str, section: &Section, typing: bool) -> Mode {
	let query = query.trim();
	if !query.is_empty() {
		return if typing {
			Mode::Waiting
		} else {
			Mode::Remote(Some(query.to_owned()))
		};
	}
	match section {
		Section::Home => Mode::Home,
		Section::Favorites => Mode::Favorites,
		Section::Trending => Mode::Remote(None),
		Section::Category(name) => Mode::Remote(Some(name.clone())),
	}
}

/// Two-column masonry: each tile keeps its GIF's aspect (64 to 320 tall) in the shorter column.
/// Returns `(column, top, height)` per GIF and the total height.
fn masonry(gifs: &[model::Gif], column: f32) -> (Vec<(usize, f32, f32)>, f32) {
	let mut heights = [0f32; 2];
	let placed = gifs
		.iter()
		.map(|gif| {
			let height = (column * gif.height as f32 / gif.width.max(1) as f32).clamp(64., 320.);
			let col = usize::from(heights[1] < heights[0]);
			let top = heights[col];
			heights[col] += height + TILE_GAP;
			(col, top, height)
		})
		.collect();
	(placed, (heights[0].max(heights[1]) - TILE_GAP).max(0.))
}

impl Serein {
	/// Opens the picker above `position`, focused on its search box, and loads trending GIFs.
	pub(crate) fn open_gif_picker(
		&mut self,
		position: Point<Pixels>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		if self.gif_picker.is_some() {
			return self.close_gif_picker(Some(window), cx);
		}
		if CLOSED_AT
			.get()
			.is_some_and(|at| at.elapsed() < Duration::from_millis(250))
		{
			return;
		}
		self.close_emoji_picker(None, cx);
		let search = cx.new(input::Input::new);
		search.update(cx, |input, cx| {
			input.set_placeholder("Search KLIPY".into(), cx)
		});
		cx.subscribe_in(
			&search,
			window,
			|this, _, event: &input::Event, window, cx| match event {
				input::Event::Changed => this.gif_query_changed(cx),
				input::Event::Cancel => this.close_gif_picker(Some(window), cx),
				_ => {}
			},
		)
		.detach();
		cx.subscribe(&search, |this, _, _: &input::Submit, cx| {
			if let Some(picker) = &mut this.gif_picker {
				picker.typed_at = None;
			}
			this.request_gif_page(cx);
		})
		.detach();
		let focus = search.read(cx).focus_handle(cx);
		window.focus(&focus, cx);
		self.gif_picker = Some(Picker {
			position,
			search,
			section: Section::Home,
			typed_at: None,
		});
		// Trending categories fill the home tiles.
		let command = self.state.request_gifs(None);
		self.dispatch(command);
		cx.notify();
	}

	/// Offline preview: the picker open on `section` ("", "favorites", "trending" or a query),
	/// with five synthetic favorites, as the main app's `--demo-gifs`.
	pub(crate) fn preview_gif_picker(
		&mut self,
		section: &str,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let favorites = test_support::gif_page(None)
			.gifs
			.into_iter()
			.skip(2)
			.take(5)
			.collect();
		self.state.restore_gif_favorites(favorites);
		let size = window.viewport_size();
		let at = point(size.width - px(300.), size.height - px(60.));
		self.open_gif_picker(at, window, cx);
		match section {
			"" => {}
			"favorites" => self.open_gif_section(Section::Favorites, cx),
			"trending" => self.open_gif_section(Section::Trending, cx),
			query => {
				if let Some(picker) = &self.gif_picker {
					picker
						.search
						.update(cx, |input, cx| input.set_value(query.to_owned(), cx));
				}
				if let Some(picker) = &mut self.gif_picker {
					picker.typed_at = None;
				}
				self.request_gif_page(cx);
			}
		}
	}

	pub(crate) fn close_gif_picker(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
		if self.gif_picker.take().is_none() {
			return;
		}
		let command = self.state.clear_gifs();
		self.dispatch(command);
		if let Some(window) = window {
			let focus = self.composer.read(cx).focus_handle(cx);
			window.focus(&focus, cx);
		}
		cx.notify();
	}

	fn gif_query_changed(&mut self, cx: &mut Context<Self>) {
		let Some(picker) = &mut self.gif_picker else {
			return;
		};
		let at = Instant::now();
		picker.typed_at = Some(at);
		cx.spawn(async move |this, cx| {
			cx.background_executor().timer(DEBOUNCE).await;
			let _ = this.update(cx, |this, cx| {
				if let Some(picker) = &mut this.gif_picker
					&& picker.typed_at == Some(at)
				{
					picker.typed_at = None;
					this.request_gif_page(cx);
				}
			});
		})
		.detach();
		cx.notify();
	}

	/// Asks for the page the current query or section shows; loaded pages are reused.
	fn request_gif_page(&mut self, cx: &mut Context<Self>) {
		let Some(picker) = &self.gif_picker else {
			return;
		};
		let query = picker.search.read(cx).value().to_owned();
		if let Mode::Remote(query) = mode(&query, &picker.section, false) {
			let command = self.state.request_gifs(query.as_deref());
			if command.is_none() && !self.state.demo && !self.state.can_browse_gifs() {
				self.notify_user("GIF search needs a connected session.");
			}
			self.dispatch(command);
		}
		cx.notify();
	}

	fn open_gif_section(&mut self, section: Section, cx: &mut Context<Self>) {
		if let Some(picker) = &mut self.gif_picker {
			picker.section = section;
		}
		self.request_gif_page(cx);
	}

	/// Sends the GIF's link as its own message; the typed draft stays in the composer.
	fn send_gif(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
		let Some(channel) = self.state.selected else {
			return;
		};
		if self.uploads.has_files() {
			self.notify_user("Send or remove the attached files before sending a GIF.");
			cx.notify();
			return;
		}
		if !self.save_draft(cx) {
			return;
		}
		let saved = self.state.drafts.insert(channel, url);
		let command = self.state.prepare_send();
		match saved {
			Some(saved) => {
				self.state.drafts.insert(channel, saved);
			}
			None => {
				self.state.drafts.remove(&channel);
			}
		}
		if command.is_none() {
			self.notify_user("Sending is unavailable with the current connection or permissions");
		} else {
			self.state.reply = None;
		}
		self.dispatch(command);
		self.close_gif_picker(Some(window), cx);
		self.sync_rows();
		self.messages.scroll_to_end();
		cx.notify();
	}

	/// Home tile: provider artwork dimmed under a white label, or a flat muted tone.
	fn gif_tile(
		&self,
		index: usize,
		label: String,
		glyph: Option<Icon>,
		art: Option<std::sync::Arc<RenderImage>>,
		section: Section,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let hue = (((index * 5) % 8) as f32 / 8. + 0.55) % 1.;
		let flat = egui::ecolor::Hsva::new(hue, 0.38, 0.46, 1.0);
		let dim = art.is_some();
		div()
			.id(("gif-tile", index))
			.relative()
			.h(px(TILE_HEIGHT))
			.rounded(px(8.))
			.overflow_hidden()
			.cursor_pointer()
			.bg(color(egui::Color32::from(flat)))
			.on_click(cx.listener(move |this, _, _, cx| this.open_gif_section(section.clone(), cx)))
			.children(art.map(|art| {
				img(art)
					.absolute()
					.inset_0()
					.size_full()
					.object_fit(ObjectFit::Cover)
			}))
			.child(
				div()
					.absolute()
					.inset_0()
					.rounded(px(8.))
					.border_2()
					.border_color(gpui::transparent_black())
					// Artwork is dimmed under the label, a little less on hover.
					.when(dim, |d| d.bg(crate::theme::tint(egui::Color32::BLACK, 0.5)))
					// One hover style per element: GPUI panics on a second.
					.hover(move |d| {
						let d = d.border_color(color(p.text_strong));
						if dim {
							d.bg(crate::theme::tint(egui::Color32::BLACK, 0.31))
						} else {
							d
						}
					})
					.flex()
					.items_center()
					.justify_center()
					.gap(px(8.))
					.text_size(px(15.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(white())
					.children(glyph.map(|glyph| icon(glyph, px(22.), white())))
					.child(label),
			)
			.into_any_element()
	}

	fn gif_home(&self, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let trending = self
			.state
			.gifs
			.view
			.as_ref()
			.filter(|view| view.query.is_none());
		let page = trending.and_then(|view| view.page.as_ref());
		let still = crate::images::gif_still;
		let mut tiles = vec![
			self.gif_tile(
				0,
				"Favorites".into(),
				Some(Icon::StarFill),
				self.state.gifs.favorites.first().and_then(still),
				Section::Favorites,
				cx,
			),
			self.gif_tile(
				1,
				"Trending GIFs".into(),
				Some(Icon::Fire),
				page.and_then(|page| page.gifs.first()).and_then(still),
				Section::Trending,
				cx,
			),
		];
		for (index, category) in page
			.map_or(&[][..], |page| &page.categories)
			.iter()
			.enumerate()
		{
			let art = category
				.preview
				.as_deref()
				.and_then(crate::images::gif_category);
			tiles.push(self.gif_tile(
				index + 2,
				category.name.clone(),
				None,
				art,
				Section::Category(category.name.clone()),
				cx,
			));
		}
		let loading = trending.is_some_and(|view| view.loading) && page.is_none();
		let mut rows = Vec::new();
		let mut tiles = tiles.into_iter();
		while let Some(first) = tiles.next() {
			let second = tiles.next();
			rows.push(
				div()
					.flex()
					.gap(px(TILE_GAP))
					.child(div().flex_1().child(first))
					.child(div().flex_1().children(second)),
			);
		}
		div()
			.flex()
			.flex_col()
			.gap(px(TILE_GAP))
			.children(rows)
			.when(loading, |d| {
				d.child(
					div()
						.text_color(color(p.muted))
						.child("Loading trending categories…"),
				)
			})
			.into_any_element()
	}

	fn gif_grid(&self, gifs: &[model::Gif], cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let column = (WIDTH - 2. * PAD - TILE_GAP) / 2.;
		let (placed, total) = masonry(gifs, column);
		div()
			.relative()
			.w_full()
			.h(px(total))
			.children(gifs.iter().zip(placed).enumerate().map(
				|(index, (gif, (col, top, height)))| {
					let url = gif.url.clone();
					let favorite = self.state.is_gif_favorite(gif);
					let group: SharedString = format!("gif-{index}").into();
					let toggled = gif.clone();
					let title = if gif.title.is_empty() {
						"Send GIF".to_owned()
					} else {
						format!("Send GIF: {}", gif.title)
					};
					div()
						.id(("gif", index))
						.group(group.clone())
						.absolute()
						.left(px(col as f32 * (column + TILE_GAP)))
						.top(px(top))
						.w(px(column))
						.h(px(height))
						.rounded(px(8.))
						.overflow_hidden()
						.bg(color(p.raised))
						.cursor_pointer()
						.flex()
						.items_center()
						.justify_center()
						.tooltip(crate::tooltip(title))
						.on_click(cx.listener(move |this, _, window, cx| {
							this.send_gif(url.clone(), window, cx)
						}))
						.child(match crate::images::gif_still(gif) {
							Some(still) => img(still)
								.size_full()
								.rounded(px(8.))
								.object_fit(ObjectFit::Cover)
								.into_any_element(),
							None => icon(Icon::Gif, px(22.), color(p.muted)).into_any_element(),
						})
						// The hover ring sits above the picture.
						.child(
							div()
								.absolute()
								.inset_0()
								.rounded(px(8.))
								.border_2()
								.border_color(gpui::transparent_black())
								.hover(|d| d.border_color(color(p.text_strong))),
						)
						// The main app's favourite star: always on favourites, on hover otherwise.
						.child(
							div()
								.id(("gif-star", index))
								.absolute()
								.top(px(6.))
								.right(px(6.))
								.size(px(26.))
								.rounded(px(6.))
								.bg(crate::theme::tint(egui::Color32::BLACK, 0.63))
								.hover(|d| d.bg(crate::theme::tint(egui::Color32::BLACK, 0.82)))
								.flex()
								.items_center()
								.justify_center()
								.when(!favorite, |d| {
									d.invisible().group_hover(group, |s| s.visible())
								})
								.tooltip(crate::tooltip(if favorite {
									"Remove from GIF favorites"
								} else {
									"Save to GIF favorites"
								}))
								.on_click(cx.listener(move |this, _, _, cx| {
									cx.stop_propagation();
									this.state.toggle_gif_favorite(&toggled);
									cx.notify();
								}))
								.child(if favorite {
									icon(Icon::StarFill, px(16.), color(p.warning))
								} else {
									icon(Icon::Star, px(16.), white())
								}),
						)
				},
			))
			.into_any_element()
	}

	pub(crate) fn render_gif_picker(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let picker = self.gif_picker.as_ref()?;
		let query = picker.search.read(cx).value().to_owned();
		let mode = mode(&query, &picker.section, picker.typed_at.is_some());
		let status = |text: &str| {
			div()
				.pt(px(24.))
				.flex()
				.justify_center()
				.text_color(color(p.muted))
				.child(text.to_owned())
		};
		let heading = match &mode {
			Mode::Home | Mode::Waiting => None,
			Mode::Favorites => Some("Favorites".to_owned()),
			Mode::Remote(None) => Some("Trending GIFs".to_owned()),
			Mode::Remote(Some(query)) => Some(query.clone()),
		};
		let body = match &mode {
			Mode::Home => self.gif_home(cx),
			Mode::Waiting => status("Searching KLIPY…").into_any_element(),
			Mode::Favorites if self.state.gifs.favorites.is_empty() => {
				status("No favorites yet").into_any_element()
			}
			Mode::Favorites => {
				let favorites = self.state.gifs.favorites.clone();
				self.gif_grid(&favorites, cx)
			}
			Mode::Remote(query) => {
				match self
					.state
					.gifs
					.view
					.as_ref()
					.filter(|view| view.query == *query)
				{
					Some(view) if view.loading => status("Loading GIFs…").into_any_element(),
					Some(view) if view.error.is_some() => {
						status(view.error.unwrap_or_default()).into_any_element()
					}
					Some(view) => {
						let gifs = view
							.page
							.as_ref()
							.map(|page| page.gifs.clone())
							.unwrap_or_default();
						if gifs.is_empty() {
							status("No GIFs found").into_any_element()
						} else {
							self.gif_grid(&gifs, cx)
						}
					}
					None => status("GIF search needs a connected session.").into_any_element(),
				}
			}
		};
		let back = !matches!(mode, Mode::Home) && query.trim().is_empty();
		Some(deferred(
			anchored()
				.anchor(Anchor::BottomRight)
				.position(picker.position)
				.snap_to_window_with_margin(px(8.))
				.child(
					div()
						.id("gif-picker")
						.occlude()
						.w(px(WIDTH))
						.h(px(HEIGHT))
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
							CLOSED_AT.set(Some(Instant::now()));
							this.close_gif_picker(None, cx);
						}))
						// The main app's tabs; stickers are not offered here, Emoji opens its picker.
						.child(
							div()
								.px(px(PAD + 6.))
								.pt(px(6.))
								.flex()
								.gap(px(20.))
								.border_b_1()
								.border_color(color(p.border))
								.text_size(px(16.))
								.font_weight(FontWeight::SEMIBOLD)
								.child(
									div()
										.py(px(8.))
										.border_b_2()
										.border_color(color(p.accent))
										.text_color(color(p.text_strong))
										.child("GIFs"),
								)
								.child(
									div()
										.id("gif-emoji-tab")
										.py(px(8.))
										.cursor_pointer()
										.text_color(color(p.muted))
										.hover(|d| d.text_color(color(p.text_strong)))
										.on_click(cx.listener(|this, _, window, cx| {
											let at = this.gif_picker.as_ref().map(|p| p.position);
											this.close_gif_picker(None, cx);
											if let Some(at) = at {
												let target = crate::emoji::Target::Composer;
												this.open_emoji_picker(target, at, window, cx);
											}
										}))
										.child("Emoji"),
								),
						)
						.child(
							div()
								.p(px(PAD))
								.pb(px(8.))
								.flex()
								.items_center()
								.gap(px(6.))
								.when(back, |d| {
									d.child(
										crate::chat::tool(
											"gif-back",
											Icon::ArrowLeft,
											28.,
											false,
											true,
											"Back",
										)
										.on_click(cx.listener(|this, _, _, cx| {
											this.open_gif_section(Section::Home, cx)
										})),
									)
								})
								.child(
									div()
										.flex_1()
										.h(px(36.))
										.px_2()
										.rounded(px(6.))
										.bg(color(p.raised))
										.flex()
										.items_center()
										.gap_2()
										.child(icon(Icon::Search, px(16.), color(p.muted)))
										.child(
											div().flex_1().min_w_0().child(picker.search.clone()),
										),
								),
						)
						.child(
							div()
								.id("gif-body")
								.flex_1()
								.min_h_0()
								.overflow_y_scroll()
								.px(px(PAD))
								.pb(px(PAD))
								.children(heading.map(|heading| {
									div()
										.pt_1()
										.pb(px(8.))
										.text_size(px(12.))
										.font_weight(FontWeight::SEMIBOLD)
										.text_color(color(p.muted))
										.child(heading.to_uppercase())
								}))
								.child(body),
						)
						.child(
							div()
								.h(px(48.))
								.flex_none()
								.px(px(PAD + 6.))
								.flex()
								.items_center()
								.gap(px(10.))
								.border_t_1()
								.border_color(color(p.border))
								.bg(solid(p.sidebar))
								.child(icon(Icon::Gif, px(26.), color(p.muted)))
								.child(
									div()
										.text_size(px(15.))
										.text_color(color(p.muted))
										.child("Click a GIF to send it right away"),
								),
						),
				),
		))
	}
}

#[cfg(test)]
mod tests {
	use super::{Mode, Section, TILE_GAP, masonry, mode};

	#[test]
	fn typing_waits_then_searches_and_sections_pick_their_page() {
		assert_eq!(mode("", &Section::Home, false), Mode::Home);
		assert_eq!(mode(" wave ", &Section::Home, true), Mode::Waiting);
		assert_eq!(
			mode(" wave ", &Section::Favorites, false),
			Mode::Remote(Some("wave".into()))
		);
		assert_eq!(mode("", &Section::Trending, false), Mode::Remote(None));
		assert_eq!(
			mode("", &Section::Category("Agree".into()), false),
			Mode::Remote(Some("Agree".into()))
		);
	}

	#[test]
	fn masonry_fills_the_shorter_column_and_clamps_heights() {
		let gifs = test_support::gif_page(None).gifs;
		let (placed, total) = masonry(&gifs, 196.);
		assert_eq!(placed.len(), gifs.len());
		assert_eq!((placed[0].0, placed[1].0), (0, 1));
		assert!(placed.iter().all(|(_, _, h)| (64. ..=320.).contains(h)));
		for col in 0..2 {
			let bottom = placed
				.iter()
				.filter(|(c, _, _)| *c == col)
				.map(|(_, top, h)| top + h)
				.fold(0f32, f32::max);
			assert!(bottom <= total + 0.01);
		}
		assert!(total > 0. && TILE_GAP > 0.);
	}
}
