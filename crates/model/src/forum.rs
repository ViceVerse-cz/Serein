//! A bounded page of active forum posts fetched on demand, mirroring the archive page budget.
use crate::{Channel, Id};

pub const PAGE_SIZE: usize = 25;
pub const MAX_BYTES: usize = 64 * 1024;
/// How many posts one forum may pull in before the list stops offering more.
pub const MAX_POSTS: usize = 200;

pub struct Page {
	pub threads: Vec<Channel>,
	pub more: bool,
}
impl Page {
	pub fn bytes(&self) -> usize {
		self.threads.capacity().saturating_sub(self.threads.len()) * size_of::<Channel>()
			+ self.threads.iter().map(Channel::bytes).sum::<usize>()
	}
	pub fn valid(&self, parent: Id, guild: Id) -> bool {
		parent.0 > 0
			&& guild.0 > 0
			&& self.threads.len() <= PAGE_SIZE
			&& self.bytes() <= MAX_BYTES
			&& (!self.more || !self.threads.is_empty())
			&& self.threads.iter().enumerate().all(|(i, thread)| {
				thread.id.0 > 0
					&& thread.id != parent
					&& thread.guild == Some(guild)
					&& thread.parent_id == Some(parent)
					&& thread.name.len() <= 512
					&& matches!(thread.kind, 10 | 11)
					&& self.threads[..i].iter().all(|other| other.id != thread.id)
			})
	}
}

/// One recent page reduced to IDs and an inert latest-message preview.
pub struct Summary {
	pub messages: Vec<Id>,
	pub latest: Option<Latest>,
	pub complete: bool,
}
pub struct Latest {
	pub id: Id,
	pub channel: Id,
	pub author_id: Id,
	pub author: String,
	pub roles: Vec<Id>,
	pub webhook: bool,
	pub excerpt: String,
}
impl Summary {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.messages.capacity() * size_of::<Id>()
			+ self.latest.as_ref().map_or(0, |hit| {
				hit.author.capacity()
					+ hit.roles.capacity() * size_of::<Id>()
					+ hit.excerpt.capacity()
			})
	}
	pub fn valid(&self, channel: Id) -> bool {
		self.messages.len() <= 50
			&& self.bytes() <= 4096
			&& self.messages.iter().all(|id| id.0 > 0)
			&& self.messages.windows(2).all(|w| w[0] > w[1])
			&& (self.complete || self.messages.len() == 50)
			&& match &self.latest {
				Some(hit) => {
					self.messages.first() == Some(&hit.id)
						&& hit.channel == channel
						&& hit.author_id.0 > 0
						&& hit.author.len() <= 512
						&& hit.roles.len() <= crate::permissions::MAX_MEMBER_ROLES
						&& hit.excerpt.len() <= 1024
				}
				None => self.messages.is_empty(),
			}
	}
}
