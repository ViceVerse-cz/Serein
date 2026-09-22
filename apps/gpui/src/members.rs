//! People in the open conversation, grouped like the main app: hoisted role, online, offline.
use crate::Serein;
use crate::sidebar::avatar_with_presence;
use crate::theme::{color, palette};
use gpui::{prelude::*, *};
use model::{Freshness, Id, MemberSlot};

pub const WIDTH: f32 = 240.;
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

	fn presence<'a>(&'a self, member: &'a model::Member, guild: Option<Id>) -> Option<&'a str> {
		let list_fresh = self
			.state
			.members
			.as_ref()
			.is_some_and(|list| list.guild == guild && list.freshness == Freshness::Fresh);
		if guild.is_some() && list_fresh && (self.state.demo || self.state.gateway_connected) {
			member.status.as_deref()
		} else {
			self.state
				.presence_for(member.user.id)
				.and_then(|presence| presence.status.as_deref())
		}
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
			Some(count) => format!("{name} — {count}"),
			None => name,
		}
	}

	pub(crate) fn sync_members(&mut self) {
		self.member_rows.clear();
		let Some(list) = self
			.state
			.members
			.as_ref()
			.filter(|list| Some(list.channel) == self.state.selected)
		else {
			return;
		};
		let mut rows = Vec::with_capacity(list.slots.len() + 2);
		// Guild lists arrive in the gateway's own order, headers included (first window only).
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
		let online = |member: &model::Member| {
			self.presence(member, list.guild)
				.is_some_and(|status| matches!(status, "online" | "idle" | "dnd"))
		};
		if let Some(guild) = list.guild {
			// Thread snapshots contain people only: group them like the main app.
			let (online_members, offline): (Vec<_>, Vec<_>) =
				members.iter().copied().partition(|(_, m)| online(m));
			let mut online_members = online_members
				.into_iter()
				.map(|member| (member, self.state.member_roles(guild, member.1).0))
				.collect::<Vec<_>>();
			online_members.sort_by(|a, b| match (a.1, b.1) {
				(Some(a), Some(b)) => b.cmp_hierarchy(a),
				(Some(_), None) => std::cmp::Ordering::Less,
				(None, Some(_)) => std::cmp::Ordering::Greater,
				(None, None) => std::cmp::Ordering::Equal,
			});
			for group in online_members.chunk_by(|a, b| a.1.map(|r| r.id) == b.1.map(|r| r.id)) {
				let name =
					group[0].1.map_or(
						"Online",
						|r| if r.name.is_empty() { "Role" } else { &r.name },
					);
				rows.push(MemberRow::Header(format!("{name} — {}", group.len())));
				rows.extend(group.iter().map(|(m, _)| MemberRow::Member(m.0, true)));
			}
			if !offline.is_empty() {
				rows.push(MemberRow::Header(format!("Offline — {}", offline.len())));
				rows.extend(offline.iter().map(|m| MemberRow::Member(m.0, false)));
			}
		} else {
			rows.push(MemberRow::Header(format!("Members — {}", members.len())));
			rows.extend(members.iter().map(|m| MemberRow::Member(m.0, online(m.1))));
		}
		self.member_rows = rows;
	}

	pub(crate) fn render_members(&self, _cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let list = self
			.state
			.members
			.as_ref()
			.filter(|list| Some(list.channel) == self.state.selected);
		let note = match list.map(|list| list.freshness) {
			None => Some("Choose a conversation to see its people."),
			Some(Freshness::Loading) => Some("Loading people…"),
			Some(Freshness::Stale) => Some("Awaiting member sync"),
			Some(Freshness::Unavailable) => Some("Member list unavailable"),
			Some(Freshness::Fresh) if self.member_rows.len() <= 1 => {
				Some("No people returned for this view.")
			}
			Some(Freshness::Fresh) => None,
		};
		div()
			.id("members")
			.w(px(WIDTH))
			.h_full()
			.flex_none()
			.bg(color(p.sidebar))
			.overflow_y_scroll()
			.p_2()
			.flex()
			.flex_col()
			.children(note.map(|note| {
				div()
					.p_2()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child(note)
			}))
			.children(self.member_rows.iter().enumerate().map(|(ix, row)| {
				match row {
					MemberRow::Header(title) => div()
						.pt(px(if ix == 0 { 8. } else { 16. }))
						.pb_1()
						.px_2()
						.text_size(px(12.))
						.font_weight(FontWeight::MEDIUM)
						.text_color(color(p.muted))
						.child(title.clone())
						.into_any_element(),
					MemberRow::Member(index, online) => {
						self.member_row(*index, *online).into_any_element()
					}
				}
			}))
	}

	fn member_row(&self, index: usize, online: bool) -> impl IntoElement {
		let p = palette();
		let Some((list, member)) =
			self.state
				.members
				.as_ref()
				.and_then(|list| match list.slots.get(index)? {
					Some(MemberSlot::Person(member)) => Some((list, member)),
					_ => None,
				})
		else {
			return div();
		};
		let name = member.nick.as_deref().unwrap_or(&member.user.name);
		let status = self.presence(member, list.guild);
		let name_color = match list.guild {
			Some(guild) if online => self
				.state
				.member_roles(guild, member)
				.1
				.map(|role| color(ui::design::role_name_color(role.color, p.sidebar, p.text)))
				.unwrap_or(color(p.text)),
			_ if online => color(p.text),
			_ => color(p.muted),
		};
		let subtitle = member.custom_status.clone().or_else(|| {
			member
				.activities
				.first()
				.map(|activity| format!("Playing {}", activity.name))
		});
		div()
			.h(px(42.))
			.flex_none()
			.px_2()
			.rounded(px(6.))
			.flex()
			.items_center()
			.gap(px(12.))
			.when(!online, |d| d.opacity(0.6))
			.hover(|d| d.bg(color(p.hover)).opacity(1.))
			.child(avatar_with_presence(
				name,
				32.,
				status.or(Some("offline")),
				color(p.sidebar),
			))
			.child(
				div()
					.flex_1()
					.min_w_0()
					.flex()
					.flex_col()
					.child(
						div()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(15.))
							.font_weight(FontWeight::MEDIUM)
							.text_color(name_color)
							.child(name.to_owned()),
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
			)
	}
}
