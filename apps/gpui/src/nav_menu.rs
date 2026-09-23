//! Right-click menus for channel rows, rail conversations and server tiles. Reads go through the
//! `client_core::State` reducer; copies only put public IDs and links on the clipboard.
use crate::Serein;
use crate::theme::{Icon, color, icon, palette, solid};
use gpui::{prelude::*, *};
use model::Id;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
	Channel(Id),
	Guild(Id),
}

pub struct Menu {
	target: Target,
	position: Point<Pixels>,
	focus: FocusHandle,
}

/// Discord's shareable link for a channel; direct messages use the `@me` scope.
pub fn channel_link(guild: Option<Id>, channel: Id) -> String {
	match guild {
		Some(guild) => format!("https://discord.com/channels/{guild}/{channel}"),
		None => format!("https://discord.com/channels/@me/{channel}"),
	}
}

type Action = Box<dyn Fn(&mut Serein, &mut Context<Serein>)>;

impl Serein {
	pub(crate) fn open_nav_menu(
		&mut self,
		target: Target,
		position: Point<Pixels>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let focus = cx.focus_handle();
		window.focus(&focus, cx);
		self.navigation.menu = Some(Menu {
			target,
			position,
			focus,
		});
		cx.notify();
	}

	fn close_nav_menu(&mut self, cx: &mut Context<Self>) {
		if self.navigation.menu.take().is_some() {
			cx.notify();
		}
	}

	fn copy(&mut self, text: String, notice: &'static str, cx: &mut Context<Self>) {
		cx.write_to_clipboard(ClipboardItem::new_string(text));
		self.notify_user(notice);
	}

	fn menu_items(&self, target: Target) -> Vec<Option<(Icon, &'static str, bool, Action)>> {
		match target {
			Target::Channel(id) => {
				let guild = self.state.channel(id).and_then(|c| c.guild);
				vec![
					Some((
						Icon::Check,
						"Mark As Read",
						self.state.can_mark_channel_read(id),
						Box::new(move |this: &mut Serein, _: &mut Context<Serein>| {
							let command = this.state.prepare_mark_channel_read(id);
							this.dispatch(command);
						}),
					)),
					None,
					Some((
						Icon::Link,
						"Copy Link",
						true,
						Box::new(move |this: &mut Serein, cx: &mut Context<Serein>| {
							this.copy(channel_link(guild, id), "Link copied", cx)
						}),
					)),
					Some((
						Icon::Copy,
						"Copy Channel ID",
						true,
						Box::new(move |this: &mut Serein, cx: &mut Context<Serein>| {
							this.copy(id.to_string(), "Channel ID copied", cx)
						}),
					)),
				]
			}
			Target::Guild(id) => vec![
				Some((
					Icon::Check,
					"Mark As Read",
					self.state.can_mark_guild_read(id),
					Box::new(move |this: &mut Serein, _: &mut Context<Serein>| {
						let command = this.state.prepare_mark_guild_read(id);
						this.dispatch(command);
					}),
				)),
				None,
				Some((
					Icon::Copy,
					"Copy Server ID",
					true,
					Box::new(move |this: &mut Serein, cx: &mut Context<Serein>| {
						this.copy(id.to_string(), "Server ID copied", cx)
					}),
				)),
			],
		}
	}

	/// The open menu, anchored at the pointer and kept inside the window.
	pub(crate) fn render_nav_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
		let p = palette();
		let menu = self.navigation.menu.as_ref()?;
		let mut rows = Vec::new();
		for (index, item) in self.menu_items(menu.target).into_iter().enumerate() {
			let Some((glyph, label, enabled, action)) = item else {
				rows.push(
					div()
						.h(px(1.))
						.mx(px(4.))
						.my(px(4.))
						.bg(color(p.border))
						.into_any_element(),
				);
				continue;
			};
			rows.push(
				div()
					.id(("nav-menu-item", index))
					.h(px(32.))
					.px(px(8.))
					.rounded(px(4.))
					.flex()
					.items_center()
					.gap(px(8.))
					.text_size(px(14.))
					.font_weight(FontWeight::MEDIUM)
					.when(enabled, |d| {
						d.cursor_pointer()
							.text_color(color(p.text))
							.hover(|d| d.bg(color(p.accent)).text_color(color(p.accent_text)))
							.on_click(cx.listener(move |this, _, _, cx| {
								this.navigation.menu = None;
								action(this, cx);
								cx.notify();
							}))
					})
					.when(!enabled, |d| d.text_color(color(p.muted)).opacity(0.6))
					.child(div().flex_1().child(label))
					.child(icon(glyph, px(16.), color(p.muted)))
					.into_any_element(),
			);
		}
		Some(
			deferred(
				anchored()
					.position(menu.position)
					.offset(point(px(2.), px(2.)))
					.snap_to_window_with_margin(px(8.))
					.child(
						div()
							.id("nav-menu")
							.track_focus(&menu.focus)
							.occlude()
							.w(px(220.))
							.p(px(6.))
							.flex()
							.flex_col()
							.rounded(px(8.))
							.bg(solid(p.chat))
							.border_1()
							.border_color(color(p.border))
							.shadow_lg()
							.font_family(crate::theme::FONT)
							.on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
								if event.keystroke.key == "escape" {
									this.close_nav_menu(cx);
									cx.stop_propagation();
								}
							}))
							.on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
								this.close_nav_menu(cx)
							}))
							.children(rows),
					),
			)
			.with_priority(1)
			.into_any_element(),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::channel_link;
	use model::Id;

	#[test]
	fn links_use_the_guild_or_the_direct_message_scope() {
		assert_eq!(
			channel_link(Some(Id(10)), Id(21)),
			"https://discord.com/channels/10/21"
		);
		assert_eq!(
			channel_link(None, Id(22)),
			"https://discord.com/channels/@me/22"
		);
	}
}
