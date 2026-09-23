//! People in the open conversation, as the main app lists them: gateway-paged lists keep the
//! server's own groups, thread snapshots are grouped by hoisted role, online and offline, and
//! other lists are plain rows.
use crate::Serein;
use crate::profile::{member_presence, server_tag, subtitle};
use crate::sidebar::avatar_with_presence;
use crate::theme::{color, palette};
use client_core::State;
use gpui::{prelude::*, *};
use model::{Freshness, Id, Member, MemberList, MemberSlot};
use std::ops::Range;

pub const WIDTH: f32 = 240.;
const ROW_HEIGHT: f32 = 42.;
/// Rows a paged list may expose, as the main app's cap.
const MAX_ROWS: usize = 250_000;
pub type MembersKey = ((u64, u64, u64), Option<Id>, bool, (usize, usize, usize));

pub enum MemberRow {
	Header(String),
	/// Index into `MemberList::slots`, and whether the member is online.
	Member(usize, bool),
}

impl Serein {
	/// Changes whenever the visible grouping could: a new list, window or presence sync.
	pub(crate) fn members_key(&self) -> Option<MembersKey> {
		let list = self.state.members.as_ref()?;
		Some((
			(self.state.generation, self.state.revision, list.request),
			self.state.selected,
			self.state.gateway_connected,
			(list.slots.len(), list.slot_bytes(), list.groups.len()),
		))
	}

	fn online(&self, member: &Member, guild: Option<Id>) -> bool {
		member_presence(&self.state, member, guild)
			.0
			.is_some_and(|status| matches!(status, "online" | "idle" | "dnd"))
	}

	fn group_title(&self, id: &str, guild: Option<Id>, count: Option<u64>) -> String {
		let name = match id {
			"online" => "Online".to_owned(),
			"offline" => "Offline".to_owned(),
			_ => id
				.parse::<u64>()
				.ok()
				.and_then(|role| {
					self.state
						.guild_roles(guild?)?
						.iter()
						.find(|r| r.id == Id(role))
				})
				.map_or_else(|| "Role".to_owned(), |role| role.name.clone()),
		};
		match count {
			Some(count) => format!("{name} - {count}"),
			None => name,
		}
	}

	pub(crate) fn sync_members(&mut self) {
		self.member_rows.clear();
		let Some(list) = self
			.state
			.members
			.as_ref()
			.filter(|list| Some(list.channel) == self.state.selected && !list.lazy)
		else {
			// Paged guild lists render straight from the reducer by absolute row index.
			return;
		};
		let mut rows = Vec::with_capacity(list.slots.len() + 2);
		// Lists that carry their own group headers keep the gateway's order.
		if list
			.slots
			.iter()
			.flatten()
			.any(|slot| matches!(slot, MemberSlot::Group(_)))
		{
			let mut online = true;
			for (index, slot) in list.slots.iter().enumerate() {
				match slot {
					Some(MemberSlot::Group(id)) => {
						online = id != "offline";
						let count = list
							.groups
							.iter()
							.find(|(group, _)| group == id)
							.map(|(_, count)| *count);
						rows.push(MemberRow::Header(self.group_title(id, list.guild, count)));
					}
					Some(MemberSlot::Person(_)) => rows.push(MemberRow::Member(index, online)),
					None => {}
				}
			}
			self.member_rows = rows;
			return;
		}
		let members = list
			.slots
			.iter()
			.enumerate()
			.filter_map(|(i, slot)| match slot {
				Some(MemberSlot::Person(member)) => Some((i, member)),
				_ => None,
			})
			.collect::<Vec<_>>();
		let thread = self
			.state
			.channel(list.channel)
			.is_some_and(|channel| matches!(channel.kind, 10..=12));
		if !thread {
			// Plain lists (DMs, groups, offline previews) have no headers, as in the main app.
			rows.extend(
				members
					.iter()
					.map(|(i, m)| MemberRow::Member(*i, self.online(m, list.guild))),
			);
			self.member_rows = rows;
			return;
		}
		// Thread snapshots contain people only: group them like the main app's
		// `thread_member_rows`, online by hoisted role first, then offline.
		let mut grouped = members
			.iter()
			.map(|&(i, member)| {
				let offline = !self.online(member, list.guild);
				let role = list
					.guild
					.and_then(|guild| self.state.member_roles(guild, member).0)
					.filter(|_| !offline);
				(i, offline, role)
			})
			.collect::<Vec<_>>();
		grouped.sort_by(|a, b| {
			a.1.cmp(&b.1).then_with(|| match (a.2, b.2) {
				(Some(a), Some(b)) => b.cmp_hierarchy(a),
				(Some(_), None) => std::cmp::Ordering::Less,
				(None, Some(_)) => std::cmp::Ordering::Greater,
				(None, None) => std::cmp::Ordering::Equal,
			})
		});
		for group in grouped.chunk_by(|a, b| a.1 == b.1 && a.2.map(|r| r.id) == b.2.map(|r| r.id)) {
			let name = if group[0].1 {
				"Offline"
			} else {
				group[0].2.map_or(
					"Online",
					|r| if r.name.is_empty() { "Role" } else { &r.name },
				)
			};
			rows.push(MemberRow::Header(format!("{name} - {}", group.len())));
			rows.extend(
				group
					.iter()
					.map(|(i, offline, _)| MemberRow::Member(*i, !offline)),
			);
		}
		self.member_rows = rows;
	}

	/// The open conversation's list, when it is a gateway-paged guild list.
	fn lazy_list(&self) -> Option<&MemberList> {
		self.state
			.members
			.as_ref()
			.filter(|list| list.lazy && Some(list.channel) == self.state.selected)
	}

	pub(crate) fn render_members(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let list = self
			.state
			.members
			.as_ref()
			.filter(|list| Some(list.channel) == self.state.selected);
		let (count, has_people) = match list {
			Some(list) if list.lazy => (
				lazy_row_count(list),
				self.state.members_cached() || list.slots.iter().any(Option::is_some),
			),
			_ => (
				self.member_rows.len(),
				self.member_rows
					.iter()
					.any(|row| matches!(row, MemberRow::Member(..))),
			),
		};
		let note = match list.map(|list| list.freshness) {
			None => Some("Choose a conversation to see its people."),
			Some(Freshness::Loading) if !has_people => Some("Loading people…"),
			Some(Freshness::Loading) => None,
			Some(Freshness::Stale) => Some("Awaiting member sync"),
			Some(Freshness::Unavailable) => Some("Member list unavailable"),
			Some(Freshness::Fresh) if !has_people => Some("No people returned for this view."),
			Some(Freshness::Fresh) => None,
		};
		// A new request (another list) starts at the top; paging keeps the scroll position.
		let key = list.map_or(0, |list| list.request);
		div()
			.w(px(WIDTH))
			.h_full()
			.flex_none()
			.bg(color(p.sidebar))
			.pt_2()
			.flex()
			.flex_col()
			.children(note.map(|note| {
				div()
					.px_4()
					.py_2()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child(note)
			}))
			.when(count > 0 && has_people, |d| {
				d.child(
					div().flex_1().min_h_0().px_2().child(
						uniform_list(
							("members", key),
							count,
							cx.processor(|this, range, window, cx| {
								this.member_items(range, window, cx)
							}),
						)
						.size_full(),
					),
				)
			})
	}

	/// Rows for the visible range; a paged list asks the gateway for the chunks on screen.
	fn member_items(
		&mut self,
		range: Range<usize>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) -> Vec<AnyElement> {
		let Some(list) = self.lazy_list() else {
			return range
				.map(|ix| match self.member_rows.get(ix) {
					Some(MemberRow::Header(title)) => header_row(title.clone()),
					Some(MemberRow::Member(index, online)) => {
						let list = self.state.members.as_ref();
						match list.and_then(|list| list.slots.get(*index)) {
							Some(Some(MemberSlot::Person(member))) => {
								self.person_row(member, list.and_then(|l| l.guild), *online, cx)
							}
							_ => pending_row(),
						}
					}
					None => pending_row(),
				})
				.collect();
		};
		// GPUI measures the first row alone before laying out the visible range; only the
		// real viewport moves the subscription, the way egui's `member_rows` does.
		if !(range.start == 0 && range.len() <= 1) {
			let (first, last) = (range.start, range.end.saturating_sub(1));
			cx.defer_in(window, move |this, _, _| {
				let command = this.state.focus_member_ranges(first, last);
				this.dispatch(command);
			});
		}
		let guild = list.guild;
		range
			.map(|index| match lazy_slot(&self.state, list, index) {
				Some(MemberSlot::Group(id)) => {
					let count = list
						.groups
						.iter()
						.find(|(group, _)| group == id)
						.map(|(_, count)| *count);
					header_row(self.group_title(id, guild, count))
				}
				Some(MemberSlot::Person(member)) => {
					self.person_row(member, guild, self.online(member, guild), cx)
				}
				None => pending_row(),
			})
			.collect()
	}

	fn person_row(
		&self,
		member: &Member,
		guild: Option<Id>,
		online: bool,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let name = member
			.nick
			.as_deref()
			.filter(|nick| !nick.is_empty())
			.unwrap_or_else(|| self.state.user_display_name(&member.user))
			.to_owned();
		let (status, custom, activities) = member_presence(&self.state, member, guild);
		let subtitle = subtitle(custom, activities);
		// Online names take the top coloured role, like the main app; offline names are muted.
		let role_color = guild
			.filter(|_| online)
			.and_then(|guild| self.state.member_roles(guild, member).1)
			.map(|role| role.color);
		let name_color = |background| match role_color {
			Some(rgb) => color(ui::design::role_name_color(rgb, background, p.text)),
			None if online => color(p.text),
			None => color(p.muted),
		};
		let (idle_color, hover_color) = (name_color(p.sidebar), name_color(p.hover));
		let group: SharedString = format!("member-{}", member.user.id).into();
		let (user, roles) = (member.user.clone(), member.roles.clone());
		div()
			.id(("member", member.user.id.0))
			.group(group.clone())
			.w_full()
			.h(px(ROW_HEIGHT))
			.flex_none()
			.overflow_hidden()
			.py(px(1.))
			.child(
				div()
					.size_full()
					.px_2()
					.rounded(px(6.))
					.flex()
					.items_center()
					.gap(px(12.))
					.cursor_pointer()
					.group_hover(group.clone(), |d| d.bg(color(p.hover)))
					.child(avatar_with_presence(
						&name,
						32.,
						Some(&member.user),
						status,
						color(p.sidebar),
					))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.flex()
							.flex_col()
							.gap(px(1.))
							.child(
								div()
									.h(px(18.))
									.min_w_0()
									.flex()
									.items_center()
									.gap(px(5.))
									.child(
										div()
											.min_w_0()
											.overflow_hidden()
											.whitespace_nowrap()
											.text_ellipsis()
											.text_size(px(15.))
											.font_weight(FontWeight::MEDIUM)
											.text_color(idle_color)
											.group_hover(group, |d| d.text_color(hover_color))
											.child(name),
									)
									.children(member.user.account_label().map(|label| {
										div()
											.flex_none()
											.h(px(15.))
											.px(px(4.))
											.rounded(px(3.))
											.bg(color(p.accent))
											.flex()
											.items_center()
											.text_size(px(10.))
											.font_weight(FontWeight::SEMIBOLD)
											.text_color(color(p.accent_text))
											.child(label)
									}))
									.children(
										member
											.user
											.primary_guild
											.as_deref()
											.map(|tag| server_tag(tag, self.state.demo)),
									),
							)
							.children(subtitle.map(|subtitle| {
								div()
									.overflow_hidden()
									.whitespace_nowrap()
									.text_ellipsis()
									.text_size(px(12.))
									.text_color(color(p.muted))
									.child(subtitle)
							})),
					),
			)
			.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
				this.open_profile(user.clone(), guild, roles.clone(), event.position(), cx)
			}))
			.into_any_element()
	}
}

/// Rows a paged list scrolls through: the reported total (group headers included), never less
/// than the delivered window, capped like the main app.
fn lazy_row_count(list: &MemberList) -> usize {
	let window = list.start.saturating_add(list.slots.len()) as u64;
	list.total.max(window).min(MAX_ROWS as u64) as usize
}

/// One absolute row of a paged list: the cached chunks first, then the current window.
fn lazy_slot<'a>(state: &'a State, list: &'a MemberList, index: usize) -> Option<&'a MemberSlot> {
	state.member_slot(index).or_else(|| {
		list.slots
			.get(index.checked_sub(list.start)?)
			.and_then(Option::as_ref)
	})
}

fn header_row(title: String) -> AnyElement {
	let p = palette();
	div()
		.h(px(ROW_HEIGHT))
		.px_2()
		.pb(px(6.))
		.flex()
		.items_end()
		.overflow_hidden()
		.whitespace_nowrap()
		.text_ellipsis()
		.text_size(px(12.))
		.font_weight(FontWeight::MEDIUM)
		.text_color(color(p.muted))
		.child(title)
		.into_any_element()
}

/// A row the gateway has not delivered yet.
fn pending_row() -> AnyElement {
	let p = palette();
	div()
		.h(px(ROW_HEIGHT))
		.px_2()
		.flex()
		.items_center()
		.gap(px(12.))
		.child(
			div()
				.size(px(32.))
				.flex_none()
				.rounded_full()
				.bg(color(p.hover)),
		)
		.child(
			div()
				.h(px(10.))
				.w(px(96.))
				.rounded(px(5.))
				.bg(color(p.hover)),
		)
		.into_any_element()
}

#[cfg(test)]
mod tests {
	// Not a glob import: `gpui::*` would shadow the built-in `#[test]` attribute.
	use super::{MAX_ROWS, lazy_row_count, lazy_slot};
	use client_core::State;
	use model::{Freshness, Id, MemberList, MemberSlot};

	fn list(start: usize, slots: usize, total: u64) -> MemberList {
		MemberList {
			guild: Some(Id(10)),
			channel: Id(20),
			request: 1,
			start,
			slots: (0..slots)
				.map(|i| Some(MemberSlot::Group(format!("g{}", start + i))))
				.collect(),
			total,
			lazy: true,
			freshness: Freshness::Fresh,
			groups: vec![],
			ranges: vec![[start, start + 99]],
		}
	}

	#[test]
	fn paged_lists_scroll_through_the_reported_total() {
		assert_eq!(lazy_row_count(&list(0, 100, 5_000)), 5_000);
		// A window past a stale total still shows every delivered row.
		assert_eq!(lazy_row_count(&list(100, 100, 150)), 200);
		assert_eq!(lazy_row_count(&list(0, 0, u64::MAX)), MAX_ROWS);
	}

	#[test]
	fn rows_resolve_by_absolute_index_in_the_window() {
		let state = State::default();
		let list = list(200, 100, 1_000);
		let group = |index| match lazy_slot(&state, &list, index) {
			Some(MemberSlot::Group(id)) => Some(id.clone()),
			_ => None,
		};
		assert_eq!(group(250).as_deref(), Some("g250"));
		assert_eq!(group(199), None);
		assert_eq!(group(300), None);
	}
}
