//! Who reacted: a paged popover from `State::request_reaction_users`, opened by right-clicking
//! a reaction chip.
use crate::Serein;
use crate::sidebar::avatar;
use crate::theme::{color, palette};
use gpui::{prelude::*, *};
use model::{Id, ReactionEmoji};

impl Serein {
	pub(crate) fn open_reactors(
		&mut self,
		message: Id,
		emoji: ReactionEmoji,
		position: Point<Pixels>,
		cx: &mut Context<Self>,
	) {
		let command = self.state.request_reaction_users(message, emoji, true);
		if command.is_none() && self.state.reactions.users.is_none() {
			self.notify_user(if self.state.demo {
				"Offline preview · reaction lists need a live connection"
			} else {
				"Who reacted is unavailable right now."
			});
		}
		self.dispatch(command);
		self.reactors_at = self.state.reactions.users.is_some().then_some(position);
		cx.notify();
	}

	fn close_reactors(&mut self, cx: &mut Context<Self>) {
		self.state.close_reaction_users();
		self.reactors_at = None;
		cx.notify();
	}

	pub(crate) fn render_reactors(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let position = self.reactors_at?;
		let details = self.state.reactions.users.as_ref().filter(|d| d.open)?;
		let label = match (&details.emoji.id, &details.emoji.name) {
			(Some(_), Some(name)) => format!(":{name}:"),
			(None, Some(name)) => name.clone(),
			_ => String::new(),
		};
		let more = !details.loading && !details.exhausted && details.error.is_none();
		Some(deferred(
			anchored()
				.position(position)
				.snap_to_window_with_margin(px(8.))
				.child(
					div()
						.id("reactors")
						.occlude()
						.w(px(260.))
						.max_h(px(360.))
						.p_2()
						.rounded(px(8.))
						.bg(color(p.base))
						.border_1()
						.border_color(color(p.border))
						.shadow_lg()
						.font_family(crate::theme::FONT)
						.flex()
						.flex_col()
						.gap_1()
						.on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_reactors(cx)))
						.child(
							div()
								.px_2()
								.pb_1()
								.flex()
								.items_center()
								.gap_2()
								.text_size(px(12.))
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(p.muted))
								.child(div().text_size(px(18.)).child(label))
								.child(format!(
									"{} {}",
									details.users.len(),
									if details.users.len() == 1 {
										"person"
									} else {
										"people"
									}
								)),
						)
						.child(
							div()
								.id("reactor-list")
								.flex_1()
								.min_h_0()
								.overflow_y_scroll()
								.flex()
								.flex_col()
								.children(details.users.iter().map(|user| {
									let name = self.state.user_display_name(user).to_owned();
									div()
										.h(px(32.))
										.px_2()
										.rounded(px(6.))
										.flex()
										.items_center()
										.gap_2()
										.hover(|d| d.bg(color(p.hover)))
										.child(avatar(&name, 22., Some(user)))
										.child(
											div()
												.flex_1()
												.min_w_0()
												.overflow_hidden()
												.whitespace_nowrap()
												.text_ellipsis()
												.text_size(px(14.))
												.text_color(color(p.text_strong))
												.child(name),
										)
								}))
								.when(details.loading, |d| {
									d.child(
										div()
											.p_2()
											.text_size(px(13.))
											.text_color(color(p.muted))
											.child("Loading…"),
									)
								})
								.children(details.error.map(|error| {
									div()
										.p_2()
										.text_size(px(13.))
										.text_color(color(p.danger))
										.child(error)
								})),
						)
						.when(more, |d| {
							d.child(
								div()
									.id("reactors-more")
									.h(px(28.))
									.rounded(px(6.))
									.flex()
									.items_center()
									.justify_center()
									.cursor_pointer()
									.text_size(px(13.))
									.text_color(color(p.link))
									.hover(|d| d.bg(color(p.hover)))
									.on_click(cx.listener(|this, _, _, cx| {
										let command = this.state.next_reaction_users_page();
										this.dispatch(command);
										cx.notify();
									}))
									.child("Load more"),
							)
						}),
				),
		))
	}
}
