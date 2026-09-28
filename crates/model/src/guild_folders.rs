use crate::Id;
use std::collections::HashSet;

// Discord allows 200 joined servers, but stored layouts can keep servers that were left.
pub const MAX_FOLDERS: usize = 1000;
pub const MAX_GUILDS: usize = 1000;
pub const MAX_NAME_BYTES: usize = 400;
pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_HEAP_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
	pub id: Option<u64>,
	pub guild_ids: Vec<Id>,
	pub name: Option<String>,
	pub color: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
	pub folders: Vec<Folder>,
	pub version: u64,
}

impl Settings {
	pub fn valid(&self) -> bool {
		if self.folders.len() > MAX_FOLDERS
			|| self.heap_bytes() > MAX_HEAP_BYTES
			|| self.version > u32::MAX.into()
		{
			return false;
		}
		let mut guilds = HashSet::new();
		let mut folders = HashSet::new();
		self.folders.iter().all(|folder| {
			folder.id.is_none_or(|id| id != 0 && folders.insert(id))
				&& (folder.id.is_some() || folder.guild_ids.len() == 1)
				&& folder.color.is_none_or(|color| color <= 0xffffff)
				&& folder.name.as_ref().is_none_or(|name| {
					name.len() <= MAX_NAME_BYTES
						&& name.chars().count() <= MAX_NAME_CHARS
						&& !name.chars().any(char::is_control)
				}) && folder.guild_ids.len() <= MAX_GUILDS
				&& folder
					.guild_ids
					.iter()
					.all(|id| id.0 != 0 && guilds.insert(*id))
		}) && guilds.len() <= MAX_GUILDS
	}

	pub fn heap_bytes(&self) -> usize {
		self.folders
			.capacity()
			.saturating_mul(size_of::<Folder>())
			.saturating_add(
				self.folders
					.iter()
					.map(|folder| {
						folder
							.guild_ids
							.capacity()
							.saturating_mul(size_of::<Id>())
							.saturating_add(folder.name.as_ref().map_or(0, String::capacity))
					})
					.fold(0usize, usize::saturating_add),
			)
	}
}
