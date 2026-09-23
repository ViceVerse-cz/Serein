//! ⌘K quick switcher, ranked by `ui::switcher_choices` exactly like the main app.
use crate::sidebar::avatar;
use crate::theme::{Icon, color, icon, palette, tint};
use crate::{Serein, input};
use gpui::{prelude::*, *};

pub struct Switcher {
	pub(crate) input: Entity<input::Input>,
	choices: Vec<ui::SwitcherChoice>,
	selected: usize,
	/// Focus before opening, restored on close.
	restore: Option<FocusHandle>,
}

impl Serein {
	pub(crate) fn toggle_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		if self.switcher.is_some() {
			return self.close_switcher(window, cx);
		}
		if self.state.user.is_none() {
			return;
		}
		let input = cx.new(input::Input::new);
		input.update(cx, |input, cx| {
			input.set_placeholder("Where would you like to go?".into(), cx);
			input.set_picking(true);
		});
		cx.subscribe_in(
			&input,
			window,
			|this, _, event: &input::Event, window, cx| match event {
				input::Event::Changed => this.refresh_switcher(cx),
				input::Event::Pick(input::Pick::Up) => this.move_switcher(-1, cx),
				input::Event::Pick(input::Pick::Down) => this.move_switcher(1, cx),
				input::Event::Pick(input::Pick::Accept) => {
					let index = this.switcher.as_ref().map_or(0, |s| s.selected);
					this.choose_switcher(index, window, cx);
				}
				input::Event::Pick(input::Pick::Close) | input::Event::Cancel => {
					this.close_switcher(window, cx)
				}
				input::Event::EditLast => {}
			},
		)
		.detach();
		let restore = window.focused(cx);
		let focus = input.read(cx).focus_handle(cx);
		window.focus(&focus, cx);
		self.switcher = Some(Switcher {
			input,
			choices: ui::switcher_choices(&self.state, ""),
			selected: 0,
			restore,
		});
		cx.notify();
	}

	pub(crate) fn refresh_switcher(&mut self, cx: &mut Context<Self>) {
		let Some(switcher) = &mut self.switcher else {
			return;
		};
		let query = switcher.input.read(cx).value().to_owned();
		switcher.choices = ui::switcher_choices(&self.state, &query);
		switcher.selected = 0;
		cx.notify();
	}

	fn move_switcher(&mut self, step: isize, cx: &mut Context<Self>) {
		if let Some(switcher) = &mut self.switcher
			&& !switcher.choices.is_empty()
		{
			let count = switcher.choices.len() as isize;
			switcher.selected = (switcher.selected as isize + step).rem_euclid(count) as usize;
			cx.notify();
		}
	}

	pub(crate) fn close_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		if let Some(switcher) = self.switcher.take()
			&& let Some(restore) = switcher.restore
		{
			window.focus(&restore, cx);
		}
		cx.notify();
	}

	fn choose_switcher(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
		let Some(choice) = self
			.switcher
			.as_ref()
			.and_then(|switcher| switcher.choices.get(index))
		else {
			return;
		};
		let channel = choice.channel;
		let voice = choice.voice;
		self.switcher = None;
		match channel {
			Some(_) if voice => self.notify_user("Voice channels open in the main Serein app."),
			Some(id) => {
				self.select(id, cx);
				let focus = self.composer.read(cx).focus_handle(cx);
				window.focus(&focus, cx);
			}
			None => self.notify_user("Start a new direct message from the main Serein app."),
		}
		cx.notify();
	}

	pub(crate) fn render_switcher(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let switcher = self.switcher.as_ref()?;
		let rows = switcher
			.choices
			.iter()
			.enumerate()
			.map(|(index, choice)| {
				let selected = index == switcher.selected;
				let glyph = match (&choice.user, choice.voice, choice.guild) {
					(Some(user), _, _) => avatar(&choice.name, 24., Some(user)).into_any_element(),
					(None, true, _) => {
						icon(Icon::Speaker, px(20.), color(p.muted)).into_any_element()
					}
					(None, false, true) => {
						icon(Icon::Hash, px(20.), color(p.muted)).into_any_element()
					}
					(None, false, false) => {
						icon(Icon::Users, px(20.), color(p.muted)).into_any_element()
					}
				};
				div()
					.id(("switcher-choice", index))
					.h(px(40.))
					.px_3()
					.rounded(px(6.))
					.flex()
					.items_center()
					.gap_3()
					.cursor_pointer()
					.when(selected, |d| d.bg(color(p.selected)))
					.hover(|d| d.bg(color(p.hover)))
					.on_click(cx.listener(move |this, _, window, cx| {
						this.choose_switcher(index, window, cx)
					}))
					.child(glyph)
					.child(
						div()
							.flex_none()
							.max_w(px(260.))
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(15.))
							.font_weight(FontWeight::MEDIUM)
							.text_color(color(p.text_strong))
							.child(choice.name.clone()),
					)
					.child(
						div()
							.flex_1()
							.min_w_0()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(13.))
							.text_color(color(p.muted))
							.child(choice.scope.clone()),
					)
					.when(choice.current, |d| {
						d.child(
							div()
								.px(px(6.))
								.rounded(px(4.))
								.bg(color(p.accent))
								.text_size(px(10.))
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(p.accent_text))
								.child("CURRENT"),
						)
					})
					.into_any_element()
			})
			.collect::<Vec<_>>();
		let empty = rows.is_empty();
		Some(deferred(
			div()
				.id("switcher-backdrop")
				.occlude()
				.absolute()
				.inset_0()
				.bg(tint(egui::Color32::BLACK, 0.55))
				.flex()
				.justify_center()
				.pt(px(120.))
				.font_family(crate::theme::FONT)
				.on_mouse_down(
					MouseButton::Left,
					cx.listener(|this, _, window, cx| this.close_switcher(window, cx)),
				)
				.child(
					div()
						.id("switcher")
						.w(px(560.))
						.max_h(px(520.))
						.p_4()
						.rounded(px(12.))
						.bg(color(p.base))
						.border_1()
						.border_color(color(p.border))
						.shadow_lg()
						.flex()
						.flex_col()
						.gap_3()
						.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
						.child(
							div()
								.h(px(48.))
								.px_3()
								.rounded(px(8.))
								.bg(color(p.raised))
								.flex()
								.items_center()
								.gap_2()
								.child(icon(Icon::Search, px(18.), color(p.muted)))
								.child(div().flex_1().child(switcher.input.clone())),
						)
						.child(
							div()
								.text_size(px(12.))
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(p.muted))
								.child(if empty {
									"NO MATCHES"
								} else {
									"CONVERSATIONS AND FRIENDS"
								}),
						)
						.child(
							div()
								.id("switcher-results")
								.flex_1()
								.min_h_0()
								.overflow_y_scroll()
								.flex()
								.flex_col()
								.children(rows)
								.when(empty, |d| {
									d.child(
										div()
											.p_3()
											.text_size(px(14.))
											.text_color(color(p.muted))
											.child("Try a channel, server or person name."),
									)
								}),
						)
						.child(
							div()
								.flex()
								.gap_4()
								.text_size(px(12.))
								.text_color(color(p.muted))
								.child("↑↓ choose")
								.child("↵ open")
								.child("Esc close"),
						),
				),
		))
	}
}
