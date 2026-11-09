//! Marks drawn after a person's name when plugins switch on the matching interface tweaks:
//! their username beside a differing display name, the server owner's crown, a staff tag
//! from their roles' permissions and the kinds of client they are online from.
use crate::icons::{self, Icon};
use client_core::State;
use extensions::InterfaceTweak;
use model::Id;

/// What the enabled tweaks would draw after one person's name; empty when nothing applies.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Marks {
	/// The account's username, when it differs from the name shown.
	pub username: Option<String>,
	/// Whether the person owns the server the name is shown in.
	pub owner: bool,
	/// A staff tag from the person's roles in that server, as a translation key.
	pub tag: Option<&'static str>,
	/// Client kinds the person is online from, in desktop, web, mobile order.
	pub platforms: Vec<Icon>,
}

impl Marks {
	/// Collects the marks for `user` shown as `shown` in `guild` with server `roles`,
	/// following the tweaks that enabled plugins switched on.
	pub(crate) fn of(
		ctx: &egui::Context,
		state: &State,
		user: &model::User,
		shown: &str,
		(guild, roles): (Option<Id>, &[Id]),
	) -> Self {
		let on = |tweak| crate::interface_tweaks::on(ctx, tweak);
		let mut marks = Self::default();
		if user.webhook || user.id.0 == 0 {
			return marks;
		}
		if on(InterfaceTweak::UsernameBesideName)
			&& !user.name.is_empty()
			&& !shown.eq_ignore_ascii_case(&user.name)
		{
			marks.username = Some(user.name.clone());
		}
		marks.owner = on(InterfaceTweak::OwnerCrown)
			&& guild.is_some_and(|guild| {
				state
					.permissions
					.guilds
					.get(&guild)
					.and_then(|metadata| metadata.owner)
					== Some(user.id)
			});
		if on(InterfaceTweak::StaffTags)
			&& !marks.owner
			&& let Some(guild) = guild
		{
			marks.tag = staff_tag(state, guild, roles);
		}
		if on(InterfaceTweak::PlatformIndicators) {
			let (status, _, _, clients) = crate::profiles::presence(state, user.id, guild);
			if status.is_some_and(|status| status != "offline") {
				for (online, icon) in [
					(clients.desktop, Icon::Television),
					(clients.web, Icon::Globe),
					(clients.mobile, Icon::DeviceMobile),
				] {
					if online {
						marks.platforms.push(icon);
					}
				}
			}
		}
		marks
	}

	/// Whether there is nothing to draw.
	pub(crate) fn is_empty(&self) -> bool {
		self.username.is_none() && !self.owner && self.tag.is_none() && self.platforms.is_empty()
	}

	/// Width to keep free after the name so the marks fit; a long username is truncated
	/// to its share.
	pub(crate) fn reserve(&self) -> f32 {
		let icons = self.platforms.len() + usize::from(self.owner);
		icons as f32 * 19.0
			+ if self.username.is_some() { 72.0 } else { 0.0 }
			+ if self.tag.is_some() { 44.0 } else { 0.0 }
	}
	pub(crate) fn show(&self, ui: &mut egui::Ui) {
		if self.is_empty() {
			return;
		}
		let colors = crate::design::palette(ui);
		if let Some(username) = &self.username {
			ui.add(
				egui::Label::new(egui::RichText::new(username).size(12.0).color(colors.muted))
					.truncate()
					.selectable(false),
			);
		}
		if self.owner {
			let rect = icons::inline(ui, Icon::Crown, 14.0, colors.warning);
			ui.interact(
				rect,
				ui.scope_id().with("owner-crown"),
				egui::Sense::hover(),
			)
			.on_hover_text(crate::i18n::translate("name-marks-server-owner"));
		}
		if let Some(tag) = self.tag {
			egui::Frame::new()
				.fill(colors.raised)
				.corner_radius(3)
				.inner_margin(egui::Margin::symmetric(4, 0))
				.show(ui, |ui| {
					ui.add(
						egui::Label::new(
							egui::RichText::new(crate::i18n::translate(tag))
								.size(10.0)
								.strong()
								.color(colors.text),
						)
						.selectable(false),
					);
				});
		}
		for icon in &self.platforms {
			let rect = icons::inline(ui, *icon, 14.0, colors.muted);
			let key = match icon {
				Icon::Television => "name-marks-online-desktop",
				Icon::Globe => "name-marks-online-web",
				_ => "name-marks-online-mobile",
			};
			ui.interact(
				rect,
				ui.scope_id().with(("platform", key)),
				egui::Sense::hover(),
			)
			.on_hover_text(crate::i18n::translate(key));
		}
	}
}

/// "Admin" for roles that administer or manage the server, "Mod" for roles that moderate
/// messages or members, from the roles' permissions and the server's everyone role.
fn staff_tag(state: &State, guild: Id, roles: &[Id]) -> Option<&'static str> {
	use model::permissions as p;
	/// Discord's Ban Members permission bit.
	const BAN_MEMBERS: u128 = 1 << 2;
	let all = state.permissions.guilds.get(&guild)?.roles.as_deref()?;
	let bits = all
		.iter()
		.filter(|role| role.id == guild || roles.contains(&role.id))
		.fold(0, |bits, role| bits | role.bits);
	if bits & (p::ADMINISTRATOR | p::MANAGE_GUILD) != 0 {
		Some("name-marks-tag-admin")
	} else if bits & (p::MANAGE_MESSAGES | p::KICK_MEMBERS | BAN_MEMBERS | p::MODERATE_MEMBERS) != 0
	{
		Some("name-marks-tag-moderator")
	} else {
		None
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Checks that each tweak adds only its own mark, and only when it applies: a username
	/// that differs from the shown name, the owner of the shown server, and the clients of
	/// someone online.
	#[test]
	fn marks_follow_their_tweaks_and_the_person() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut user = test_support::message(1, Id(22)).author;
		user.name = "synthetic_handle".into();
		state.direct_presences.push(model::MemberPresence {
			user: user.id,
			status: Some("online".into()),
			custom_status: None,
			activities: vec![],
			clients: model::ClientPlatforms {
				desktop: true,
				mobile: true,
				..Default::default()
			},
		});
		let guild = *state.permissions.guilds.keys().next().expect("demo server");
		state.permissions.guilds.get_mut(&guild).unwrap().owner = Some(user.id);
		let tweaks = |on: &[InterfaceTweak]| {
			crate::interface_tweaks::publish(
				&ctx,
				&std::sync::Arc::new(on.iter().copied().collect()),
			);
		};
		tweaks(&[]);
		assert!(Marks::of(&ctx, &state, &user, "Nickname", (Some(guild), &[])).is_empty());
		tweaks(InterfaceTweak::ALL);
		assert_eq!(
			Marks::of(&ctx, &state, &user, "Nickname", (Some(guild), &[])),
			Marks {
				username: Some("synthetic_handle".into()),
				owner: true,
				tag: None,
				platforms: vec![Icon::Television, Icon::DeviceMobile],
			}
		);
		// The username repeats nothing, the crown needs the owner's server, and someone
		// offline shows no clients.
		state.direct_presences[0].status = Some("offline".into());
		assert!(Marks::of(&ctx, &state, &user, "Synthetic_Handle", (None, &[])).is_empty());
		tweaks(&[InterfaceTweak::OwnerCrown]);
		let mut other = user.clone();
		other.id = Id(user.id.0 + 1);
		assert!(!Marks::of(&ctx, &state, &other, "Nickname", (Some(guild), &[])).owner);
		other.webhook = true;
		other.id = user.id;
		assert!(Marks::of(&ctx, &state, &other, "Nickname", (Some(guild), &[])).is_empty());

		// Staff tags follow role permissions, including the everyone role, and give way
		// to the owner's crown.
		tweaks(&[InterfaceTweak::StaffTags]);
		let roles = state
			.permissions
			.guilds
			.get_mut(&guild)
			.unwrap()
			.roles
			.get_or_insert_default();
		roles.retain(|role| role.id != guild);
		for (id, bits) in [
			(guild, 0),
			(Id(501), model::permissions::MODERATE_MEMBERS),
			(Id(502), model::permissions::ADMINISTRATOR),
		] {
			roles.push(model::permissions::Role {
				id,
				bits,
				name: String::new(),
				color: 0,
				secondary_color: None,
				tertiary_color: None,
				position: 1,
				hoist: false,
				managed: false,
			});
		}
		let mut member = user.clone();
		member.id = Id(user.id.0 + 7);
		let tag = |roles: &[Id], member: &model::User| {
			Marks::of(&ctx, &state, member, "Nickname", (Some(guild), roles)).tag
		};
		assert_eq!(tag(&[], &member), None);
		assert_eq!(tag(&[Id(501)], &member), Some("name-marks-tag-moderator"));
		assert_eq!(
			tag(&[Id(501), Id(502)], &member),
			Some("name-marks-tag-admin")
		);
		assert_eq!(tag(&[Id(502)], &user), Some("name-marks-tag-admin"));
		tweaks(&[InterfaceTweak::StaffTags, InterfaceTweak::OwnerCrown]);
		assert_eq!(
			tag(&[Id(502)], &user),
			None,
			"the owner shows the crown instead"
		);
		assert_eq!(tag(&[Id(502)], &member), Some("name-marks-tag-admin"));
	}
}
