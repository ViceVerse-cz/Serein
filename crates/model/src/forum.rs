//! A bounded page of active forum posts fetched on demand, mirroring the archive page budget.
use crate::{Channel, EmbedMedia, Id, Reaction, ReactionEmoji};

pub const PAGE_SIZE: usize = 25;
pub const MAX_BYTES: usize = 64 * 1024;
/// How many posts one forum may pull in before the list stops offering more.
pub const MAX_POSTS: usize = 200;

/// Discord caps a forum at 20 tags and a post at 5 applied ones.
pub const MAX_TAGS: usize = 20;
pub const MAX_APPLIED_TAGS: usize = 5;
pub const MAX_TAG_NAME: usize = 50;

/// One tag a forum or media channel offers its posts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
	pub id: Id,
	pub name: String,
	/// Only members who can manage threads may apply a moderated tag.
	pub moderated: bool,
	pub emoji_id: Option<Id>,
	/// A Unicode emoji, or the name of the custom emoji in `emoji_id`.
	pub emoji_name: Option<String>,
}

/// How a forum lays out its posts by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
	#[default]
	List,
	Gallery,
}

/// Which posts a forum lists first by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
	#[default]
	Activity,
	Created,
}

/// Forum metadata carried by a container (the tags it offers and its post defaults) or by a
/// post (the tags applied to it).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tags {
	pub available: Vec<Tag>,
	pub applied: Vec<Id>,
	/// The container requires at least one tag on every new post.
	pub required: bool,
	/// The emoji members react to posts with from the post list.
	pub reaction: Option<ReactionEmoji>,
	pub layout: Layout,
	pub sort: Sort,
	/// Unofficial `default_tag_setting`: a post must carry every selected tag, not just one.
	pub match_all: bool,
}
impl Tags {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.available.capacity() * size_of::<Tag>()
			+ self
				.available
				.iter()
				.map(|tag| {
					tag.name.capacity() + tag.emoji_name.as_ref().map_or(0, String::capacity)
				})
				.sum::<usize>()
			+ self.applied.capacity() * size_of::<Id>()
			+ self
				.reaction
				.as_ref()
				.and_then(|emoji| emoji.name.as_ref())
				.map_or(0, String::capacity)
	}
	pub fn is_empty(&self) -> bool {
		*self == Self::default()
	}
}

/// What a post card shows of its starter message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Starter {
	/// The first image, from an attachment or an embed.
	pub image: Option<EmbedMedia>,
	/// Reactions on the starter; the card shows the forum's default one or the most used.
	pub reactions: Vec<Reaction>,
}
impl Starter {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.image.as_ref().map_or(0, EmbedMedia::bytes)
			+ crate::reactions::reaction_bytes(&self.reactions)
	}
	pub fn valid(&self) -> bool {
		self.image.as_ref().is_none_or(EmbedMedia::valid)
			&& self.reactions.len() <= 20
			&& crate::reactions::valid_reactions(&self.reactions)
	}
	pub fn is_empty(&self) -> bool {
		self.image.is_none() && self.reactions.is_empty()
	}
}

pub struct Page {
	pub threads: Vec<Channel>,
	pub more: bool,
	/// The starter message of each listed post, when the service sent one.
	pub previews: Vec<(Id, Starter)>,
}
impl Page {
	pub fn bytes(&self) -> usize {
		self.threads.capacity().saturating_sub(self.threads.len()) * size_of::<Channel>()
			+ self.threads.iter().map(Channel::bytes).sum::<usize>()
			+ self.previews.capacity() * size_of::<Id>()
			+ self
				.previews
				.iter()
				.map(|(_, starter)| starter.bytes())
				.sum::<usize>()
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
			}) && self.previews.len() <= self.threads.len()
			&& self.previews.iter().all(|(id, starter)| {
				starter.valid() && self.threads.iter().any(|thread| thread.id == *id)
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
