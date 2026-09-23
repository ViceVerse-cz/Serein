//! Channel search and pinned messages in the right-hand panel, driven by `client_core::search`.
use crate::sidebar::avatar;
use crate::theme::{Icon, color, icon, palette};
use crate::{Serein, input, tooltip};
use gpui::{prelude::*, *};
use model::Id;

pub const WIDTH: f32 = 360.;

impl Serein {
	pub(crate) fn search_open(&self) -> bool {
		self.state
			.search
			.as_ref()
			.is_some_and(|view| Some(view.channel) == self.state.selected)
	}

	pub(crate) fn run_search(&mut self, cx: &mut Context<Self>) {
		let query = self.search_input.read(cx).value().trim().to_owned();
		if query.is_empty() {
			return self.close_search(cx);
		}
		let command = self.state.request_search(query, None);
		if command.is_none() && !self.state.demo {
			self.notify_user("Search needs a live, fully synced connection.");
		}
		self.dispatch(command);
		cx.notify();
	}

	pub(crate) fn toggle_pins(&mut self, cx: &mut Context<Self>) {
		if self.state.search.as_ref().is_some_and(|view| view.pins) {
			return self.close_search(cx);
		}
		let command = self.state.request_pins();
		if command.is_none() && !self.state.demo {
			self.notify_user("Pinned messages need a live, fully synced connection.");
		}
		self.dispatch(command);
		cx.notify();
	}

	pub(crate) fn close_search(&mut self, cx: &mut Context<Self>) {
		if self.state.search.is_some() {
			let command = self.state.clear_search();
			self.dispatch(Some(command));
		}
		cx.notify();
	}

	fn more_results(&mut self, cx: &mut Context<Self>) {
		let Some(view) = self.state.search.as_ref() else {
			return;
		};
		let command = if view.pins {
			self.state.request_older_pins()
		} else {
			let before = view
				.page
				.as_ref()
				.and_then(|page| page.hits.last())
				.map(|hit| hit.id);
			let query = view.query.clone();
			before.and_then(|before| self.state.request_search(query, Some(before)))
		};
		self.dispatch(command);
		cx.notify();
	}

	fn open_hit(&mut self, message: Id, cx: &mut Context<Self>) {
		let command = self.state.open_search_hit(message);
		if command.is_none() && !self.state.demo {
			self.notify_user("This result can no longer be opened.");
		}
		self.dispatch(command);
		self.sync_rows();
		if let Some(index) = self.rows.iter().position(|id| *id == message) {
			self.messages.scroll_to_reveal_item(index);
		}
		cx.notify();
	}

	/// Search pill for the chat header; Enter searches, Escape clears.
	/// The main app's 144px pill, widening to 240px while a search is shown.
	pub(crate) fn search_box(&self) -> impl IntoElement {
		let p = palette();
		let searching = self.search_open() && self.state.search.as_ref().is_some_and(|v| !v.pins);
		div()
			.w(px(if searching { 240. } else { 144. }))
			.h(px(28.))
			.pl(px(10.))
			.pr(px(6.))
			.rounded(px(6.))
			.bg(color(p.raised))
			.flex()
			.items_center()
			.gap_1()
			.text_size(px(13.))
			.child(div().flex_1().min_w_0().child(self.search_input.clone()))
			.child(icon(Icon::Search, px(16.), color(p.muted)))
	}

	pub(crate) fn render_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let view = self.state.search.as_ref();
		let pins = view.is_some_and(|view| view.pins);
		let title = match view {
			Some(view) if view.pins => "Pinned messages".to_owned(),
			Some(view) => match view.page.as_ref() {
				Some(page) => format!("{} results for “{}”", page.total, view.query),
				None => format!("Searching “{}”", view.query),
			},
			None => String::new(),
		};
		let hits = view
			.and_then(|view| view.page.as_ref())
			.map(|page| page.hits.as_slice())
			.unwrap_or_default();
		let more = view.is_some_and(|view| {
			!view.loading
				&& view.page.as_ref().is_some_and(|page| {
					if view.pins {
						page.pin_cursor.is_some()
					} else {
						(page.hits.len() as u64) < page.total && !page.hits.is_empty()
					}
				})
		});
		div()
			.id("search-panel")
			.w(px(WIDTH))
			.h_full()
			.flex_none()
			.bg(color(p.sidebar))
			.flex()
			.flex_col()
			.child(
				div()
					.h(px(48.))
					.flex_none()
					.px_4()
					.border_b_1()
					.border_color(color(p.border))
					.flex()
					.items_center()
					.gap_2()
					.child(icon(
						if pins { Icon::Pin } else { Icon::Search },
						px(18.),
						color(p.muted),
					))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.text_strong))
							.child(title),
					)
					.child(
						self.icon_button("close-search", Icon::Close, false, "Close")
							.on_click(cx.listener(|this, _, _, cx| this.close_search(cx))),
					),
			)
			.child(
				div()
					.id("search-hits")
					.flex_1()
					.min_h_0()
					.overflow_y_scroll()
					.p_2()
					.flex()
					.flex_col()
					.gap_2()
					.children(view.and_then(|view| {
						let note = if view.loading {
							Some("Loading…")
						} else if let Some(error) = view.error {
							Some(error)
						} else if hits.is_empty() {
							Some(if view.pins {
								"This conversation has no pinned messages."
							} else {
								"No results found."
							})
						} else {
							None
						};
						note.map(|note| div().p_2().text_color(color(p.muted)).child(note))
					}))
					.children(hits.iter().map(|hit| {
						let id = hit.id;
						let name = self.state.user_display_name(&hit.author).to_owned();
						let excerpt: String = hit.excerpt.chars().take(400).collect();
						div()
							.id(("hit", id.0))
							.p_3()
							.rounded(px(8.))
							.bg(color(p.chat))
							.border_1()
							.border_color(color(p.border))
							.cursor_pointer()
							.hover(|d| d.border_color(color(p.muted)))
							.tooltip(tooltip("Jump to message"))
							.on_click(cx.listener(move |this, _, _, cx| this.open_hit(id, cx)))
							.flex()
							.gap_3()
							.child(avatar(&name, 32., Some(&hit.author)))
							.child(
								div()
									.flex_1()
									.min_w_0()
									.flex()
									.flex_col()
									.gap_1()
									.child(
										div()
											.flex()
											.gap_2()
											.items_center()
											.child(
												div()
													.font_weight(FontWeight::MEDIUM)
													.text_color(color(p.text_strong))
													.child(name),
											)
											.child(
												div()
													.text_size(px(12.))
													.text_color(color(p.muted))
													.child(crate::chat::short_date(id)),
											),
									)
									.child(div().text_size(px(14.)).child(excerpt))
									.when(!hit.attachments.is_empty(), |d| {
										d.child(
											div()
												.text_size(px(12.))
												.text_color(color(p.muted))
												.child(format!(
													"{} attachment{}",
													hit.attachments.len(),
													if hit.attachments.len() == 1 {
														""
													} else {
														"s"
													}
												)),
										)
									}),
							)
					}))
					.when(more, |d| {
						d.child(
							self.button("more-results", "Load more", false)
								.justify_center()
								.on_click(cx.listener(|this, _, _, cx| this.more_results(cx))),
						)
					}),
			)
	}

	/// Offline fixture: search and pins over the loaded synthetic timeline only.
	pub(crate) fn demo_search(&self, command: &client_core::Command) -> Option<client_core::Event> {
		use client_core::{Command, Event, search::Outcome};
		let hit = |message: &model::Message| model::SearchHit {
			id: message.id,
			channel: message.channel,
			author: message.author.clone(),
			excerpt: message.display_text().chars().take(200).collect(),
			attachments: message.attachments.clone(),
			embeds: vec![],
		};
		match command {
			Command::Search {
				channel,
				query,
				request,
				before,
				..
			} => {
				let needle = query.to_lowercase();
				let hits = self
					.state
					.timeline
					.iter()
					.rev()
					.filter(|m| m.channel == *channel && before.is_none_or(|b| m.id < b))
					.filter(|m| m.content.to_lowercase().contains(&needle))
					.take(25)
					.map(hit)
					.collect::<Vec<_>>();
				Some(Event::Search {
					channel: *channel,
					request: *request,
					result: Ok(Outcome::Page(model::SearchPage {
						total: hits.len() as u64,
						hits,
						partial: false,
						pin_cursor: None,
					})),
				})
			}
			Command::Pins {
				channel, request, ..
			} => Some(Event::Search {
				channel: *channel,
				request: *request,
				result: Ok(Outcome::Pins(model::SearchPage {
					hits: self
						.state
						.timeline
						.iter()
						.filter(|m| m.channel == *channel)
						.take(2)
						.map(hit)
						.collect(),
					total: 0,
					partial: false,
					pin_cursor: None,
				})),
			}),
			// Synthetic GIF results; previews are drawn locally.
			Command::Gifs { query, request } => Some(Event::Gifs {
				request: *request,
				result: Ok(test_support::gif_page(query.as_deref())),
			}),
			// The main app's synthetic archived threads: three per page, then one older page.
			Command::Archives {
				parent,
				guild,
				kind,
				before,
				request,
			} => {
				use model::archives::{Cursor, Page};
				let offset = parent.0.saturating_mul(10_000).saturating_add(match kind {
					model::archives::Kind::Public => 0,
					model::archives::Kind::Private => 1_000,
					model::archives::Kind::JoinedPrivate => 2_000,
				});
				let ids = (if before.is_none() {
					[900, 850, 800]
				} else {
					[700, 650, 600]
				})
				.map(|id| offset.saturating_add(id));
				let announcement = self
					.state
					.channels
					.iter()
					.any(|c| c.id == *parent && c.kind == 5);
				let threads = ids
					.into_iter()
					.map(|id| model::Channel {
						id: Id(id),
						guild: Some(*guild),
						parent_id: Some(*parent),
						position: 0,
						name: format!("Synthetic archived thread {id}"),
						icon: None,
						kind: match (kind, announcement) {
							(model::archives::Kind::Public, true) => 10,
							(model::archives::Kind::Public, false) => 11,
							_ => 12,
						},
						recipients: vec![],
						last_message: None,
						member_list_id: None,
						message_count: None,
					})
					.collect();
				Some(Event::Archives {
					parent: *parent,
					request: *request,
					result: Ok(Page {
						threads,
						next: before.is_none().then_some(
							if *kind == model::archives::Kind::JoinedPrivate {
								Cursor::Id(Id(ids[2]))
							} else {
								Cursor::Time(1_788_998_400_000_000_000)
							},
						),
					}),
				})
			}
			_ => None,
		}
	}

	pub(crate) fn search_input_event(&mut self, event: &input::Event, cx: &mut Context<Self>) {
		if matches!(event, input::Event::Cancel) {
			self.search_input
				.update(cx, |input, cx| input.set_value(String::new(), cx));
			self.close_search(cx);
		}
	}
}
