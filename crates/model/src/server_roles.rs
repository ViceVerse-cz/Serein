//! Bounded role editing snapshots; permissions outside the edit mask are preserved.
use crate::{
	Id, Patch,
	server_admin::{Members, Query},
};
pub const MAX_BYTES: usize = 512 * 1024;
pub const MAX_ROLES: usize = 512;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Colors {
	pub primary: u32,
	pub secondary: Option<u32>,
	pub tertiary: Option<u32>,
}
impl Colors {
	pub fn valid(self) -> bool {
		self.primary <= 0xff_ffff
			&& self.secondary.is_none_or(|v| v <= 0xff_ffff)
			&& self.tertiary.is_none_or(|v| v <= 0xff_ffff)
	}
	pub fn editable(self) -> bool {
		self.valid()
			&& (self.tertiary.is_none()
				|| (self.primary, self.secondary, self.tertiary)
					== (11127295, Some(16759788), Some(16761760)))
	}
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
	pub id: Id,
	pub name: String,
	pub colors: Colors,
	pub permissions: u128,
	pub position: i32,
	pub hoist: bool,
	pub mentionable: bool,
	pub managed: bool,
	pub icon: Option<String>,
	pub unicode_emoji: Option<String>,
	pub member_count: Option<u64>,
}
impl Default for Role {
	fn default() -> Self {
		Self {
			id: Id(0),
			name: "new role".into(),
			colors: Colors::default(),
			permissions: 0,
			position: 0,
			hoist: false,
			mentionable: false,
			managed: false,
			icon: None,
			unicode_emoji: None,
			member_count: None,
		}
	}
}
impl Role {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.name.capacity()
			+ self.icon.as_ref().map_or(0, String::capacity)
			+ self.unicode_emoji.as_ref().map_or(0, String::capacity)
	}
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& valid_name(&self.name)
			&& self.colors.valid()
			&& (0..=4096).contains(&self.position)
			&& self
				.icon
				.as_ref()
				.is_none_or(|value| crate::valid_avatar_hash(value))
			&& self
				.unicode_emoji
				.as_ref()
				.is_none_or(|value| valid_unicode(value))
			&& self.bytes() <= 2048
	}
	pub fn permission_role(&self) -> crate::permissions::Role {
		crate::permissions::Role {
			id: self.id,
			name: self.name.clone(),
			bits: self.permissions,
			color: self.colors.primary,
			secondary_color: self.colors.secondary,
			tertiary_color: self.colors.tertiary,
			position: self.position,
			hoist: self.hoist,
		}
	}
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
	pub guild: Id,
	pub items: Vec<Role>,
	pub features: Vec<String>,
}
impl Default for Catalog {
	fn default() -> Self {
		Self {
			guild: Id(0),
			items: vec![],
			features: vec![],
		}
	}
}
impl Catalog {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.items.iter().map(Role::bytes).sum::<usize>()
			+ self.items.capacity().saturating_sub(self.items.len()) * size_of::<Role>()
			+ self.features.capacity() * size_of::<String>()
			+ self.features.iter().map(String::capacity).sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		let mut seen = std::collections::BTreeSet::new();
		self.guild.0 != 0
			&& self.items.len() <= MAX_ROLES
			&& self.items.iter().any(|role| role.id == self.guild)
			&& self
				.items
				.iter()
				.all(|role| role.valid() && seen.insert(role.id))
			&& self.features.len() <= 256
			&& self.features.iter().all(|value| value.len() <= 128)
			&& self.bytes() <= MAX_BYTES
	}
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
	pub name: Option<String>,
	pub colors: Option<Colors>,
	pub permissions: Option<u128>,
	pub permission_mask: u128,
	pub hoist: Option<bool>,
	pub mentionable: Option<bool>,
	pub icon: Patch<String>,
	pub unicode_emoji: Patch<String>,
}
impl Edit {
	pub fn between(before: &Role, after: &Role) -> Self {
		Self {
			name: (before.name != after.name).then(|| after.name.clone()),
			colors: (before.colors != after.colors).then_some(after.colors),
			permissions: (before.permissions != after.permissions).then_some(after.permissions),
			permission_mask: before.permissions ^ after.permissions,
			hoist: (before.hoist != after.hoist).then_some(after.hoist),
			mentionable: (before.mentionable != after.mentionable).then_some(after.mentionable),
			icon: if before.icon.is_some() && after.icon.is_none() {
				Patch::Null
			} else {
				Patch::Absent
			},
			unicode_emoji: if before.unicode_emoji == after.unicode_emoji {
				Patch::Absent
			} else {
				after
					.unicode_emoji
					.clone()
					.map_or(Patch::Null, Patch::Value)
			},
		}
	}
	pub fn is_empty(&self) -> bool {
		self == &Self::default()
	}
	pub fn valid(&self) -> bool {
		self.name
			.as_ref()
			.is_none_or(|value| value.capacity() <= 400 && valid_name(value))
			&& self.colors.is_none_or(Colors::editable)
			&& (self.permissions.is_some() || self.permission_mask == 0)
			&& !matches!(
				(&self.icon, &self.unicode_emoji),
				(Patch::Value(_), Patch::Value(_))
			) && match &self.icon {
			Patch::Value(value) => {
				value.capacity() <= crate::server_settings::MAX_ICON_DATA_URI
					&& value.starts_with("data:image/png;base64,")
			}
			_ => true,
		} && match &self.unicode_emoji {
			Patch::Value(value) => value.capacity() <= 128 && valid_unicode(value),
			_ => true,
		}
	}
	pub fn only_permissions(&self) -> bool {
		self.name.is_none()
			&& self.colors.is_none()
			&& self.hoist.is_none()
			&& self.mentionable.is_none()
			&& matches!(self.icon, Patch::Absent)
			&& matches!(self.unicode_emoji, Patch::Absent)
	}
	pub fn normalize(&mut self) {
		if let Some(value) = &mut self.name {
			value.shrink_to_fit();
		}
		if let Patch::Value(value) = &mut self.icon {
			value.shrink_to_fit();
		}
		if let Patch::Value(value) = &mut self.unicode_emoji {
			value.shrink_to_fit();
		}
	}
	pub fn apply(&self, value: &mut Role) {
		if let Some(name) = &self.name {
			value.name.clone_from(name);
		}
		if let Some(colors) = self.colors {
			value.colors = colors;
		}
		if let Some(bits) = self.permissions {
			value.permissions =
				(value.permissions & !self.permission_mask) | (bits & self.permission_mask);
		}
		if let Some(hoist) = self.hoist {
			value.hoist = hoist;
		}
		if let Some(mentionable) = self.mentionable {
			value.mentionable = mentionable;
		}
		if matches!(self.icon, Patch::Null) {
			value.icon = None;
		}
		match &self.unicode_emoji {
			Patch::Value(emoji) => {
				value.unicode_emoji = Some(emoji.clone());
				value.icon = None;
			}
			Patch::Null => value.unicode_emoji = None,
			Patch::Absent => {}
		}
		if matches!(self.icon, Patch::Value(_)) {
			value.unicode_emoji = None;
		}
	}
}
#[derive(Clone)]
pub enum Action {
	Load,
	Create(Edit),
	Edit { id: Id, edit: Edit },
	Delete(Id),
	Move { id: Id, position: i32 },
	Members { role: Option<Id>, query: Query },
}
impl Action {
	pub fn write(&self) -> bool {
		!matches!(self, Self::Load | Self::Members { .. })
	}
	pub fn valid(&self) -> bool {
		match self {
			Self::Load => true,
			Self::Create(edit) => edit.valid(),
			Self::Edit { id, edit } => id.0 != 0 && edit.valid() && !edit.is_empty(),
			Self::Delete(id) => id.0 != 0,
			Self::Move { id, position } => id.0 != 0 && (1..=4096).contains(position),
			Self::Members { role, query } => role.is_none_or(|role| role.0 != 0) && query.valid(),
		}
	}
	pub fn normalize(&mut self) {
		match self {
			Self::Create(edit) | Self::Edit { edit, .. } => edit.normalize(),
			Self::Members { query, .. } => query.search.shrink_to_fit(),
			_ => {}
		}
	}
}
pub enum Result {
	Catalog {
		catalog: Catalog,
		selected: Option<Id>,
	},
	Members {
		role: Option<Id>,
		page: Members,
	},
}
impl Result {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ match self {
				Self::Catalog { catalog, .. } => catalog.bytes(),
				Self::Members { page, .. } => page.bytes(),
			}
	}
	pub fn valid(&self) -> bool {
		match self {
			Self::Catalog { catalog, selected } => {
				catalog.valid()
					&& selected.is_none_or(|id| catalog.items.iter().any(|role| role.id == id))
			}
			Self::Members { role, page } => role.is_none_or(|role| role.0 != 0) && page.valid(),
		}
	}
}
fn valid_name(value: &str) -> bool {
	!value.trim().is_empty()
		&& value.len() <= 400
		&& value.chars().count() <= 100
		&& !value.chars().any(char::is_control)
}
fn valid_unicode(value: &str) -> bool {
	!value.trim().is_empty()
		&& value.len() <= 128
		&& value.chars().count() <= 32
		&& !value.chars().any(char::is_control)
}
