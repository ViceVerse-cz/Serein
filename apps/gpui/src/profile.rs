//! Compact profile card from data already in memory; never fetches a profile.
use crate::Serein;
use crate::sidebar::avatar_with_presence;
use crate::theme::{color, palette, tint};
use gpui::{prelude::*, *};
use model::{Id, User};

pub struct Card {
	user: User,
	guild: Option<Id>,
	/// Role ids known from the message or member list.
	roles: Vec<Id>,
	position: Point<Pixels>,
}

impl Serein {
	pub(crate) fn open_profile(
		&mut self,
		user: User,
		guild: Option<Id>,
		roles: Vec<Id>,
		position: Point<Pixels>,
		cx: &mut Context<Self>,
	) {
		self.profile = Some(Card {
			user,
			guild,
			roles,
			position,
		});
		cx.notify();
	}

	fn status_for(&self, user: Id) -> Option<&str> {
		let member = self.state.members.as_ref().and_then(|list| {
			list.slots.iter().flatten().find_map(|slot| match slot {
				model::MemberSlot::Person(member) if member.user.id == user => Some(member),
				_ => None,
			})
		});
		member
			.and_then(|member| member.status.as_deref())
			.or_else(|| {
				self.state
					.presence_for(user)
					.and_then(|presence| presence.status.as_deref())
			})
	}

	pub(crate) fn render_profile(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let card = self.profile.as_ref()?;
		let user = &card.user;
		let name = self.state.user_display_name(user).to_owned();
		let status = self.status_for(user.id);
		let custom = self.state.members.as_ref().and_then(|list| {
			list.slots.iter().flatten().find_map(|slot| match slot {
				model::MemberSlot::Person(member) if member.user.id == user.id => {
					member.custom_status.clone()
				}
				_ => None,
			})
		});
		// The live member list can know roles the message snapshot did not carry.
		let member_roles = self
			.state
			.members
			.as_ref()
			.and_then(|list| {
				list.slots.iter().flatten().find_map(|slot| match slot {
					model::MemberSlot::Person(member) if member.user.id == user.id => {
						Some(member.roles.clone())
					}
					_ => None,
				})
			})
			.unwrap_or_default();
		let mut roles = card
			.guild
			.and_then(|guild| self.state.guild_roles(guild))
			.map(|roles| {
				roles
					.iter()
					.filter(|role| card.roles.contains(&role.id) || member_roles.contains(&role.id))
					.collect::<Vec<_>>()
			})
			.unwrap_or_default();
		roles.sort_by(|a, b| b.cmp_hierarchy(a));
		roles.truncate(12);
		let mention = format!("<@{}> ", user.id);
		// Same hashed colour as the initials avatar, softened for the banner.
		let banner = tint(ui::design::fallback_avatar_color(&name), 0.55);
		Some(deferred(
			anchored()
				.position(card.position)
				.snap_to_window_with_margin(px(8.))
				.child(
					div()
						.id("profile-card")
						.occlude()
						.w(px(300.))
						.rounded(px(12.))
						.bg(color(p.base))
						.border_1()
						.border_color(color(p.border))
						.shadow_lg()
						.overflow_hidden()
						.font_family(crate::theme::FONT)
						.on_mouse_down_out(cx.listener(|this, _, _, cx| {
							this.profile = None;
							cx.notify();
						}))
						.child(div().h(px(60.)).bg(banner))
						.child(
							div().px_4().mt(px(-36.)).child(
								div()
									.p(px(4.))
									.rounded_full()
									.bg(color(p.base))
									.w(px(80.))
									.child(avatar_with_presence(
										&name,
										72.,
										Some(user),
										status,
										color(p.base),
									)),
							),
						)
						.child(
							div()
								.p_4()
								.pt_2()
								.flex()
								.flex_col()
								.gap_2()
								.child(
									div()
										.flex()
										.flex_col()
										.child(
											div()
												.text_size(px(20.))
												.font_weight(FontWeight::SEMIBOLD)
												.text_color(color(p.text_strong))
												.child(name.clone()),
										)
										.child(
											div()
												.flex()
												.gap_2()
												.items_center()
												.text_size(px(14.))
												.text_color(color(p.muted))
												.child(user.name.clone())
												.children(user.account_label().map(|label| {
													div()
														.px(px(4.))
														.rounded(px(3.))
														.bg(color(p.accent))
														.text_size(px(10.))
														.font_weight(FontWeight::SEMIBOLD)
														.text_color(color(p.accent_text))
														.child(label)
												})),
										),
								)
								.children(custom.map(|custom| {
									div()
										.p_2()
										.rounded(px(8.))
										.bg(color(p.raised))
										.text_size(px(14.))
										.child(custom)
								}))
								.when(!roles.is_empty(), |d| {
									d.child(
										div()
											.text_size(px(12.))
											.font_weight(FontWeight::SEMIBOLD)
											.text_color(color(p.muted))
											.child("ROLES"),
									)
									.child(div().flex().flex_wrap().gap_1().children(
										roles.iter().map(|role| {
											div()
												.h(px(24.))
												.px_2()
												.rounded(px(6.))
												.bg(color(p.raised))
												.flex()
												.items_center()
												.gap_1()
												.text_size(px(12.))
												.child(div().size(px(10.)).rounded_full().bg(
													if role.color == 0 {
														color(p.muted)
													} else {
														rgb(role.color)
													},
												))
												.child(role.name.clone())
										}),
									))
								})
								.child(
									div()
										.id("profile-mention")
										.h(px(34.))
										.rounded(px(8.))
										.bg(color(p.raised))
										.flex()
										.items_center()
										.justify_center()
										.cursor_pointer()
										.hover(|d| d.bg(color(p.hover)))
										.text_size(px(14.))
										.font_weight(FontWeight::MEDIUM)
										.text_color(color(p.text_strong))
										.on_click(cx.listener(move |this, _, window, cx| {
											let mention = mention.clone();
											this.composer.update(cx, |input, cx| {
												let value = format!("{}{mention}", input.value());
												input.set_value(value, cx);
											});
											let focus = this.composer.read(cx).focus_handle(cx);
											window.focus(&focus, cx);
											this.profile = None;
											cx.notify();
										}))
										.child("Mention"),
								),
						),
				),
		))
	}
}
