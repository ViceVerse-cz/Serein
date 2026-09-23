//! The chat header's Threads dialog, as the main app's: a name filter, the channel's active
//! threads from the session, then older threads loaded 25 at a time through
//! `client_core::archives`. Opening a thread loads its messages without joining it.
use crate::sidebar::avatar;
use crate::theme::{Icon, color, icon, palette, solid, tint};
use crate::{Serein, input, tooltip};
use gpui::{prelude::*, *};
use model::{Channel, Id, archives::Kind};

const WIDTH: f32 = 520.;
const CARD_HEIGHT: f32 = 74.;
/// The main app's dialog padding and corner radius.
const PAD: f32 = 20.;
const RADIUS: f32 = 16.;

/// "N messages · Last active 5m ago", the main app's thread summary from synced metadata.
fn activity(thread: &Channel) -> String {
	let count = match thread.message_count {
		Some(0) => "No replies yet".to_owned(),
		Some(1) => "1 message".to_owned(),
		Some(n) => format!("{n} messages"),
		None => "Thread".to_owned(),
	};
	let Some(last) = thread.last_message else {
		return count;
	};
	let now = time::OffsetDateTime::now_utc().unix_timestamp();
	format!("{count} · Last active {}", crate::forum::ago(last, now))
}

impl Serein {
	/// Toggles the dialog for `parent`, asking for its first page of older public threads.
	pub(crate) fn open_threads(&mut self, parent: Id, cx: &mut Context<Self>) {
		if self.state.archives.is_some() {
			return self.close_threads(cx);
		}
		let filter = cx.new(input::Input::new);
		filter.update(cx, |input, cx| {
			input.set_placeholder("Search for thread name".into(), cx)
		});
		cx.subscribe(&filter, |this, _, event: &input::Event, cx| match event {
			input::Event::Cancel => this.close_threads(cx),
			_ => cx.notify(),
		})
		.detach();
		self.thread_filter = Some(filter);
		let command = self.state.request_archives(parent, Kind::Public, None);
		if command.is_none() && !self.state.demo {
			self.notify_user("Threads need a live, fully synced connection.");
		}
		self.dispatch(command);
		cx.notify();
	}

	pub(crate) fn close_threads(&mut self, cx: &mut Context<Self>) {
		self.thread_filter = None;
		if self.state.archives.is_some() {
			let command = self.state.clear_archives();
			self.dispatch(Some(command));
		}
		cx.notify();
	}

	fn load_threads(
		&mut self,
		kind: Kind,
		before: Option<model::archives::Cursor>,
		cx: &mut Context<Self>,
	) {
		let Some(parent) = self.state.archives.as_ref().map(|view| view.parent) else {
			return;
		};
		let command = self.state.request_archives(parent, kind, before);
		self.dispatch(command);
		cx.notify();
	}

	/// Opens an active thread, or admits an archived one to navigation first.
	fn open_thread(&mut self, id: Id, archived: bool, cx: &mut Context<Self>) {
		if archived && self.state.admit_archived_thread(id).is_none() {
			let reason = self
				.state
				.archives
				.as_ref()
				.and_then(|view| view.error)
				.unwrap_or("This thread can't be opened right now.");
			self.notify_user(reason);
			cx.notify();
			return;
		}
		self.close_threads(cx);
		self.select(id, cx);
	}

	fn thread_card(&self, thread: &Channel, archived: bool, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let id = thread.id;
		// A thread shares its id with its starter message, which the open channel may hold.
		let starter = self.state.timeline.get(thread.id).map(|starter| {
			let name = self.state.message_author_name(starter).to_owned();
			let tone = self
				.state
				.message_author_color(starter)
				.map_or(p.text_strong, |rgb| {
					ui::design::role_name_color(rgb, p.raised, p.text)
				});
			div()
				.flex()
				.items_center()
				.gap(px(6.))
				.flex_none()
				.child(avatar(&name, 18., Some(&starter.author)))
				.child(div().text_color(color(p.muted)).child("Started by"))
				.child(
					div()
						.font_weight(FontWeight::MEDIUM)
						.text_color(color(tone))
						.child(name),
				)
				.child(div().text_color(color(p.muted)).child("•"))
		});
		div()
			.id(("thread-card", id.0))
			.flex_none()
			.h(px(CARD_HEIGHT))
			.px(px(16.))
			.py(px(12.))
			.rounded(px(8.))
			.bg(color(p.raised))
			.border_1()
			.border_color(color(p.border))
			.hover(|d| d.bg(color(p.hover)).border_color(color(p.accent)))
			.cursor_pointer()
			.flex()
			.flex_col()
			.gap(px(6.))
			.tooltip(tooltip(format!("Open thread “{}”", thread.name)))
			.on_click(cx.listener(move |this, _, _, cx| this.open_thread(id, archived, cx)))
			.child(
				div()
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.text_size(px(15.5))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.text_strong))
					.child(thread.name.clone()),
			)
			.child(
				div()
					.flex()
					.items_center()
					.gap(px(6.))
					.min_w_0()
					.text_size(px(13.))
					.children(starter)
					.child(
						div()
							.min_w_0()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_color(color(p.muted))
							.child(activity(thread)),
					),
			)
			.into_any_element()
	}

	/// The dialog while `state.archives` is open: centred over a dimmed window.
	pub(crate) fn render_threads(
		&self,
		window: &mut Window,
		cx: &mut Context<Self>,
	) -> Option<AnyElement> {
		let p = palette();
		let view = self.state.archives.as_ref()?;
		let allowed = self.state.can_archive(view.parent, view.kind);
		let private = self.state.channel(view.parent).is_some_and(|c| c.kind == 0);
		let filter = self
			.thread_filter
			.as_ref()
			.map(|input| input.read(cx).value().trim().to_lowercase())
			.unwrap_or_default();
		let matches =
			|thread: &Channel| filter.is_empty() || thread.name.to_lowercase().contains(&filter);
		let active_all = self.state.active_threads(view.parent);
		let active = active_all
			.iter()
			.filter(|thread| matches(thread))
			.map(|thread| self.thread_card(thread, false, cx))
			.collect::<Vec<_>>();
		let older = view
			.page
			.iter()
			.flat_map(|page| &page.threads)
			.filter(|thread| matches(thread))
			.map(|thread| self.thread_card(thread, true, cx))
			.collect::<Vec<_>>();
		let next = view.page.as_ref().and_then(|page| page.next);
		let (kind, loading, error, before) = (view.kind, view.loading, view.error, view.before);
		let section = |label: String| {
			div()
				.pb(px(6.))
				.text_size(px(12.))
				.font_weight(FontWeight::SEMIBOLD)
				.text_color(color(p.muted))
				.child(label)
		};
		let hint = |text: &'static str| {
			div()
				.text_size(px(13.))
				.text_color(color(p.muted))
				.child(text)
		};
		// The main app's selectable buttons (kinds) and plain buttons (Reload, Older, Retry).
		let button =
			|id: &'static str, label: &'static str, selected: Option<bool>, enabled: bool| {
				let framed = selected.is_none();
				let on = selected == Some(true);
				div()
					.id(id)
					.h(px(32.))
					.px(px(12.))
					.rounded(px(8.))
					.flex()
					.items_center()
					.text_size(px(14.))
					.font_weight(FontWeight::MEDIUM)
					.when(framed, |d| d.bg(color(p.raised)))
					.when(on, |d| d.bg(tint(p.accent, 0.35)))
					.text_color(if enabled {
						color(if on { p.text_strong } else { p.text })
					} else {
						tint(p.muted, 0.5)
					})
					.when(enabled && !on, |d| {
						d.cursor_pointer().hover(|d| d.bg(color(p.hover)))
					})
					.child(label)
			};
		let kinds = [
			(Kind::Public, "threads-public", "Public"),
			(Kind::JoinedPrivate, "threads-joined", "Joined private"),
			(Kind::Private, "threads-private", "Private"),
		];
		let viewport = window.viewport_size();
		let body_height = (viewport.height - px(220.)).clamp(px(120.), px(620.));
		let active_count = active.len();
		let dialog = div()
			.id("threads-dialog")
			.w(px(WIDTH).min(viewport.width - px(32.)))
			.rounded(px(RADIUS))
			.bg(solid(p.chat))
			.border_1()
			.border_color(color(p.border))
			.shadow_lg()
			.flex()
			.flex_col()
			.gap(px(8.))
			// Clicks inside never reach the backdrop.
			.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
			.child(
				div()
					.pl(px(PAD))
					.pr(px(PAD - 4.))
					.pt(px(PAD))
					.pb(px(4.))
					.flex()
					.items_center()
					.gap(px(6.))
					.child(
						div()
							.size(px(30.))
							.flex()
							.items_center()
							.justify_center()
							.child(icon(Icon::Thread, px(24.), color(p.muted))),
					)
					.child(
						div()
							.flex_1()
							.text_size(px(19.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.text_strong))
							.child("Threads"),
					)
					.child(
						crate::chat::tool("close-threads", Icon::Close, 30., false, true, "Close dialog (Esc)")
							.on_click(cx.listener(|this, _, _, cx| this.close_threads(cx))),
					),
			)
			.child(
				div().px(px(PAD)).pb(px(6.)).children(self.thread_filter.clone().map(|input| {
					div()
						.px(px(10.))
						.py(px(6.))
						.rounded(px(8.))
						.bg(solid(p.sidebar))
						.border_1()
						.border_color(color(p.border))
						.flex()
						.items_center()
						.gap(px(8.))
						.child(icon(Icon::Search, px(16.), color(p.muted)))
						.child(div().flex_1().min_w_0().child(input))
				})),
			)
			.child(
				div()
					.id("threads-body")
					.max_h(body_height)
					.overflow_y_scroll()
					.px(px(PAD))
					.flex()
					.flex_col()
					.gap(px(8.))
					.when(!active_all.is_empty(), |d| {
						d.child(section(format!("{active_count} ACTIVE THREADS")))
							.when(active_count == 0, |d| {
								d.child(hint("No active thread matches this search."))
							})
							.children(active)
							.child(div().h(px(4.)))
					})
					.child(section("OLDER THREADS".into()))
					.child(
						div()
							.flex()
							.flex_wrap()
							.gap(px(6.))
							.children(
								kinds
									.into_iter()
									.filter(|(k, _, _)| *k == Kind::Public || private)
									.map(|(k, id, label)| {
										let enabled = allowed && !loading;
										button(id, label, Some(k == kind), enabled).when(
											enabled && k != kind,
											|d| {
												d.on_click(cx.listener(move |this, _, _, cx| {
													this.load_threads(k, None, cx)
												}))
											},
										)
									}),
							)
							.child({
								let enabled = allowed && !loading;
								button("threads-reload", "Reload", None, enabled).when(
									enabled,
									|d| {
										d.on_click(cx.listener(move |this, _, _, cx| {
											this.load_threads(kind, None, cx)
										}))
									},
								)
							})
							.when(error.is_some(), |d| {
								let enabled = allowed && !loading;
								d.child(button("threads-retry", "Retry", None, enabled).when(
									enabled,
									|d| {
										d.on_click(cx.listener(move |this, _, _, cx| {
											this.load_threads(kind, before, cx)
										}))
									},
								))
							})
							.when_some(next.filter(|_| error.is_none()), |d, next| {
								let enabled = allowed && !loading;
								d.child(button("threads-older", "Older", None, enabled).when(
									enabled,
									|d| {
										d.on_click(cx.listener(move |this, _, _, cx| {
											this.load_threads(kind, Some(next), cx)
										}))
									},
								))
							}),
					)
					.when(kind == Kind::Private, |d| {
						d.child(hint("Private archives require permission from the service."))
					})
					.when(!allowed, |d| {
						d.child(hint(
							"Archives are unavailable while disconnected or without channel access.",
						))
					})
					.children(error.map(|error| {
						div()
							.text_size(px(13.))
							.text_color(color(p.danger))
							.child(error)
					}))
					.when(loading, |d| d.child(hint("Loading older threads…")))
					.when(!loading && view.page.is_some() && older.is_empty(), |d| {
						d.child(hint("No older threads returned."))
					})
					.children(older)
					.when(!loading && view.page.is_some() && next.is_none(), |d| {
						d.child(hint("No older threads reported by the service."))
					})
					.child(hint(
						"Active threads come from the session; older threads load 25 at a time. Opening loads messages without joining.",
					)),
			)
			// The main app's footer strip with the dialog's Close action.
			.child(
				div()
					.mt(px(PAD - 8.))
					.px(px(PAD))
					.py(px(16.))
					.rounded_b(px(RADIUS))
					.bg(solid(p.sidebar))
					.border_t_1()
					.border_color(color(p.border))
					.flex()
					.justify_end()
					.child(
						div()
							.id("threads-close")
							.h(px(38.))
							.px(px(16.))
							.rounded(px(8.))
							.flex()
							.items_center()
							.cursor_pointer()
							.hover(|d| d.bg(color(p.hover)))
							.text_size(px(14.))
							.font_weight(FontWeight::MEDIUM)
							.text_color(color(p.text_strong))
							.on_click(cx.listener(|this, _, _, cx| this.close_threads(cx)))
							.child("Close"),
					),
			);
		Some(
			deferred(
				div()
					.id("threads-backdrop")
					.occlude()
					.absolute()
					.inset_0()
					.bg(hsla(0., 0., 0., 150. / 255.))
					.flex()
					.items_center()
					.justify_center()
					.font_family(crate::theme::FONT)
					.on_mouse_down(
						MouseButton::Left,
						cx.listener(|this, _, _, cx| this.close_threads(cx)),
					)
					.child(dialog),
			)
			.with_priority(1)
			.into_any_element(),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::activity;
	use model::{Channel, Id};

	#[test]
	fn thread_summaries_count_messages_like_the_main_app() {
		let mut thread = Channel {
			id: Id(9),
			guild: Some(Id(1)),
			parent_id: Some(Id(2)),
			kind: 11,
			position: 0,
			name: "Synthetic thread".into(),
			recipients: vec![],
			last_message: None,
			member_list_id: None,
			message_count: Some(0),
			icon: None,
		};
		assert_eq!(activity(&thread), "No replies yet");
		thread.message_count = Some(3);
		assert_eq!(activity(&thread), "3 messages");
		thread.message_count = None;
		thread.last_message = Some(Id(1 << 22));
		assert!(activity(&thread).starts_with("Thread · Last active "));
	}
}
