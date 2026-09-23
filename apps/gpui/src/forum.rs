//! Forum and media channels: post cards like the main app's forum view; a post opens as a thread.
use crate::Serein;
use crate::sidebar::avatar;
use crate::theme::{Icon, color, icon, palette};
use client_core::{Command, State, forum::SUMMARY_BATCH};
use gpui::{prelude::*, *};
use model::Id;

/// Every card has the same height so the list can be a `uniform_list`.
const CARD_HEIGHT: f32 = 116.;
const CARD_GAP: f32 = 8.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
	#[default]
	Activity,
	Created,
}

impl Sort {
	fn label(self) -> &'static str {
		match self {
			Sort::Activity => "Recent activity",
			Sort::Created => "Creation date",
		}
	}
}

/// Session-only forum view settings.
#[derive(Default)]
pub struct View {
	pub sort: Sort,
}

/// "5m ago" from a snowflake, as the main app's post cards.
pub(crate) fn ago(id: Id, now_unix: i64) -> String {
	let created = ((id.0 >> 22) / 1000) as i64 + 1_420_070_400;
	let seconds = now_unix.saturating_sub(created).max(0);
	match seconds {
		0..60 => "just now".to_owned(),
		60..3_600 => format!("{}m ago", seconds / 60),
		3_600..86_400 => format!("{}h ago", seconds / 3_600),
		86_400..2_592_000 => format!("{}d ago", seconds / 86_400),
		2_592_000..31_536_000 => format!("{}mo ago", seconds / 2_592_000),
		_ => format!("{}y ago", seconds / 31_536_000),
	}
}

/// Loaded posts of a forum in the chosen order.
fn sorted_posts(state: &State, forum: Id, sort: Sort) -> Vec<Id> {
	let mut posts = state.forum_posts(forum);
	if sort == Sort::Created {
		posts.sort_by_key(|post| std::cmp::Reverse(post.id));
	}
	posts.into_iter().map(|post| post.id).collect()
}

/// Offline preview: the latest-message summaries a connected session would fetch, so the
/// cards show authors and excerpts. Synthetic data only; nothing leaves the process.
pub(crate) fn seed_demo_summaries(state: &mut State, forum: Id) {
	if !state.demo {
		return;
	}
	// The reducer only requests summaries for the open forum of a live session.
	state.posts.parent = Some(forum);
	state.demo = false;
	let wanted = state
		.forum_posts(forum)
		.into_iter()
		.map(|post| post.id)
		.collect::<Vec<_>>();
	let wanted = wanted
		.into_iter()
		.filter(|id| state.needs_post_summary(*id))
		.collect::<Vec<_>>();
	const EXCERPTS: [&str; 3] = [
		"Synthetic reply: a quantised model could ship per device class.",
		"Sounds good — keep it opt-in and fully offline.",
		"Posting a synthetic benchmark table here later today.",
	];
	for (batch, ids) in wanted.chunks(SUMMARY_BATCH).enumerate() {
		let Some(Command::ForumSummaries { channels, request }) =
			state.request_post_summaries(ids.to_vec())
		else {
			continue;
		};
		let results = channels
			.into_iter()
			.enumerate()
			.map(|(i, channel)| {
				let n = batch * SUMMARY_BATCH + i;
				let latest = state.channel(channel).and_then(|post| post.last_message);
				let author = test_support::message(n as u64, channel).author;
				let summary = model::forum::Summary {
					messages: latest.into_iter().collect(),
					latest: latest.map(|id| model::forum::Latest {
						id,
						channel,
						author_id: author.id,
						author: author.name,
						roles: vec![],
						webhook: false,
						excerpt: EXCERPTS[n % EXCERPTS.len()].into(),
					}),
					complete: true,
				};
				(channel, Ok(summary))
			})
			.collect();
		state.apply_forum_summaries(request, results);
	}
	state.demo = true;
}

impl Serein {
	/// The selected channel is a forum or media container, shown instead of the chat.
	pub(crate) fn forum_visible(&self) -> bool {
		!self.friends_visible()
			&& self
				.state
				.selected
				.is_some_and(|id| self.state.is_forum(id))
	}

	/// Load the first page of posts (live sessions) when a forum opens.
	pub(crate) fn open_forum(&mut self, forum: Id) {
		if self.state.demo {
			seed_demo_summaries(&mut self.state, forum);
		}
		let command = self.state.request_forum_posts(forum, false);
		self.dispatch(command);
	}

	fn load_more_posts(&mut self, forum: Id, cx: &mut Context<Self>) {
		self.state.posts.error = None;
		let more = self.state.posts.loaded > 0;
		let command = self.state.request_forum_posts(forum, more);
		self.dispatch(command);
		cx.notify();
	}

	fn footer_visible(&self, forum: Id) -> bool {
		let posts = &self.state.posts;
		posts.parent == Some(forum) && (posts.loading || posts.error.is_some() || posts.more)
	}

	pub(crate) fn render_forum(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let Some(forum) = self.state.selected else {
			return div().flex_1().into_any_element();
		};
		let name = self
			.state
			.channel(forum)
			.map(crate::channel_label)
			.unwrap_or_default();
		let posts = self.state.forum_posts(forum).len();
		let items = posts + usize::from(self.footer_visible(forum));
		let loading = self.state.posts.parent == Some(forum) && self.state.posts.loading;
		let sort = self.forum.sort;
		div()
			.flex_1()
			.min_w_0()
			.h_full()
			.bg(color(p.chat))
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
					.gap(px(8.))
					.child(icon(Icon::Forum, px(22.), color(p.muted)))
					.child(
						div()
							.flex_none()
							.max_w(px(360.))
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(16.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.text_strong))
							.child(name),
					)
					.child(
						div()
							.flex_1()
							.min_w_0()
							.text_size(px(13.))
							.text_color(color(p.muted))
							.child(format!("{posts} post{}", if posts == 1 { "" } else { "s" })),
					)
					.child(
						self.icon_button(
							"members-toggle",
							Icon::Users,
							self.members_open,
							"Show people",
						)
						.on_click(cx.listener(|this, _, _, cx| {
							this.members_open = !this.members_open;
							cx.notify();
						})),
					),
			)
			.child(
				div()
					.flex_none()
					.px_4()
					.pt_3()
					.pb_2()
					.flex()
					.items_center()
					.gap(px(6.))
					.child(
						div()
							.mr_1()
							.text_size(px(12.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.muted))
							.child("SORT BY"),
					)
					.children([Sort::Activity, Sort::Created].map(|option| {
						let active = option == sort;
						div()
							.id(option.label())
							.h(px(26.))
							.px_3()
							.flex()
							.items_center()
							.rounded(px(13.))
							.border_1()
							.border_color(color(if active { p.accent } else { p.border }))
							.when(active, |d| d.bg(crate::theme::tint(p.accent, 0.16)))
							.text_size(px(13.))
							.font_weight(FontWeight::MEDIUM)
							.text_color(color(if active { p.text_strong } else { p.muted }))
							.cursor_pointer()
							.hover(|d| d.text_color(color(p.text_strong)))
							.on_click(cx.listener(move |this, _, _, cx| {
								this.forum.sort = option;
								cx.notify();
							}))
							.child(option.label())
					})),
			)
			.child(
				div()
					.flex_1()
					.min_h_0()
					.when(items == 0, |d| {
						d.flex()
							.flex_col()
							.items_center()
							.pt(px(48.))
							.gap_1()
							.child(
								div()
									.text_size(px(16.))
									.font_weight(FontWeight::SEMIBOLD)
									.text_color(color(p.text_strong))
									.child(if loading {
										"Loading posts…"
									} else {
										"No posts loaded"
									}),
							)
							.when(!loading, |d| {
								d.child(div().text_color(color(p.muted)).child(
									"Nothing is posted here yet; archived posts open in the main Serein app.",
								))
							})
					})
					.when(items > 0, |d| {
						d.child(
							uniform_list(
								("forum-posts", forum.0),
								items,
								cx.processor(move |this, range, window, cx| {
									this.forum_items(forum, range, window, cx)
								}),
							)
							.size_full(),
						)
					}),
			)
			.into_any_element()
	}

	fn forum_items(
		&mut self,
		forum: Id,
		range: std::ops::Range<usize>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) -> Vec<AnyElement> {
		let posts = sorted_posts(&self.state, forum, self.forum.sort);
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0, |elapsed| elapsed.as_secs() as i64);
		// Summaries are fetched for the cards on screen, a few at a time, like the main app.
		let wanted = posts
			.get(range.start.min(posts.len())..range.end.min(posts.len()))
			.unwrap_or_default()
			.iter()
			.copied()
			.filter(|id| self.state.needs_post_summary(*id))
			.take(SUMMARY_BATCH)
			.collect::<Vec<_>>();
		if !wanted.is_empty() {
			cx.defer_in(window, move |this, _, cx| {
				let command = this.state.request_post_summaries(wanted);
				if command.is_some() {
					this.dispatch(command);
					cx.notify();
				}
			});
		}
		range
			.map(|ix| match posts.get(ix) {
				Some(post) => self.post_card(*post, now, cx),
				None => self.posts_footer(forum, cx),
			})
			.collect()
	}

	fn post_card(&self, id: Id, now: i64, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let Some(post) = self.state.channel(id) else {
			return div().h(px(CARD_HEIGHT)).into_any_element();
		};
		let unread = self.state.post_unread(post);
		let latest = self
			.state
			.post_summary(id)
			.and_then(|summary| summary.latest.as_ref());
		let new_label = unread.then(|| match self.state.post_new_count(post) {
			Some((count, exact)) if count > 0 => {
				format!("({count}{} New)", if exact { "" } else { "+" })
			}
			_ => "(New)".to_owned(),
		});
		let latest_row = match latest {
			Some(latest) => {
				let author_color = self
					.state
					.forum_author_color(id, latest.author_id, latest.webhook, &latest.roles)
					.map_or(color(p.text_strong), |rgb| {
						color(ui::design::role_name_color(rgb, p.raised, p.text_strong))
					});
				let excerpt = if latest.excerpt.trim().is_empty() {
					"Attachment or non-text message".to_owned()
				} else {
					latest.excerpt.clone()
				};
				div()
					.h(px(22.))
					.flex()
					.items_center()
					.gap(px(6.))
					.min_w_0()
					.child(avatar(&latest.author, 20., None))
					.child(
						div()
							.flex_none()
							.text_size(px(14.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(author_color)
							.child(format!("{}:", latest.author)),
					)
					.child(
						div()
							.flex_1()
							.min_w_0()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(14.))
							.text_color(color(p.text))
							.child(excerpt),
					)
			}
			None => div()
				.h(px(22.))
				.flex()
				.items_center()
				.text_size(px(14.))
				.text_color(color(p.muted))
				.child("Latest message unavailable"),
		};
		let meta = div()
			.h(px(20.))
			.flex()
			.items_center()
			.gap(px(6.))
			.text_size(px(13.))
			.text_color(color(p.muted))
			.children(post.message_count.map(|count| {
				div()
					.flex()
					.items_center()
					.gap(px(4.))
					.child(icon(Icon::Chats, px(16.), color(p.muted)))
					.child(
						div()
							.font_weight(FontWeight::MEDIUM)
							.text_color(color(p.text))
							.child(count.to_string()),
					)
			}))
			.children(new_label.map(|label| {
				div()
					.font_weight(FontWeight::MEDIUM)
					.text_color(color(p.accent))
					.child(label)
			}))
			.child("·")
			.child(ago(post.last_message.unwrap_or(post.id), now));
		let title = div()
			.h(px(24.))
			.flex()
			.items_center()
			.gap(px(8.))
			.min_w_0()
			.when(unread, |d| {
				d.child(
					div()
						.size(px(8.))
						.flex_none()
						.rounded_full()
						.bg(color(p.text_strong)),
				)
			})
			.child(
				div()
					.flex_1()
					.min_w_0()
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.text_size(px(16.))
					.font_weight(if unread {
						FontWeight::SEMIBOLD
					} else {
						FontWeight::MEDIUM
					})
					.text_color(color(if unread { p.text_strong } else { p.muted }))
					.child(post.name.clone()),
			);
		div()
			.h(px(CARD_HEIGHT))
			.px_4()
			.pb(px(CARD_GAP))
			.child(
				div()
					.id(("post", id.0))
					.size_full()
					.px_4()
					.py(px(12.))
					.rounded(px(8.))
					.bg(color(p.raised))
					.border_1()
					.border_color(color(p.border))
					.flex()
					.flex_col()
					.justify_center()
					.gap(px(6.))
					.cursor_pointer()
					.focusable()
					.tab_stop(true)
					.hover(|d| d.bg(color(p.hover)))
					.focus(|d| d.border_color(color(p.text_strong)))
					.on_click(cx.listener(move |this, _, _, cx| this.select(id, cx)))
					.child(title)
					.child(latest_row)
					.child(meta),
			)
			.into_any_element()
	}

	/// Loading, error and "Load more posts" line under the cards (one uniform row).
	fn posts_footer(&self, forum: Id, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let posts = &self.state.posts;
		let row = div()
			.h(px(CARD_HEIGHT))
			.px_4()
			.pt_1()
			.flex()
			.items_start()
			.gap_3()
			.text_size(px(13.));
		let action = |label: &'static str| {
			div()
				.id("posts-more")
				.cursor_pointer()
				.font_weight(FontWeight::MEDIUM)
				.text_color(color(p.link))
				.hover(|d| d.underline())
				.on_click(cx.listener(move |this, _, _, cx| this.load_more_posts(forum, cx)))
				.child(label)
		};
		if posts.loading {
			row.text_color(color(p.muted))
				.child("Loading posts…")
				.into_any_element()
		} else if let Some(error) = posts.error {
			row.child(div().text_color(color(p.danger)).child(error))
				.when(self.state.can_load_posts(forum), |d| {
					d.child(action("Retry"))
				})
				.into_any_element()
		} else {
			row.child(action("Load more posts")).into_any_element()
		}
	}
}

#[cfg(test)]
mod tests {
	// Not a glob import: `gpui::*` would shadow the built-in `#[test]` attribute.
	use super::{Sort, ago, seed_demo_summaries, sorted_posts};
	use model::Id;

	fn snowflake(unix: i64) -> Id {
		Id((((unix - 1_420_070_400) * 1000) as u64) << 22)
	}

	#[test]
	fn activity_age_uses_the_main_app_units() {
		let now = 1_800_000_000;
		assert_eq!(ago(snowflake(now - 5), now), "just now");
		assert_eq!(ago(snowflake(now - 300), now), "5m ago");
		assert_eq!(ago(snowflake(now - 7_200), now), "2h ago");
		assert_eq!(ago(snowflake(now - 3 * 86_400), now), "3d ago");
		assert_eq!(ago(snowflake(now + 60), now), "just now");
	}

	#[test]
	fn demo_forum_lists_posts_with_synthetic_summaries() {
		let mut state = test_support::chat_demo_state();
		let forum = Id(26);
		assert!(state.is_forum(forum));
		let _ = state.select(forum);
		seed_demo_summaries(&mut state, forum);
		assert!(state.demo);
		let posts = sorted_posts(&state, forum, Sort::Activity);
		assert_eq!(posts.len(), 3);
		// Newest activity first; creation order puts the highest ID first.
		assert_eq!(posts[0], Id(42));
		assert_eq!(sorted_posts(&state, forum, Sort::Created)[0], Id(42));
		for post in posts {
			let latest = state
				.post_summary(post)
				.and_then(|summary| summary.latest.as_ref())
				.expect("seeded summary");
			assert!(!latest.excerpt.is_empty());
		}
		// The offline preview never asks the service for posts.
		assert!(state.request_forum_posts(forum, false).is_none());
	}
}
