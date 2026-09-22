//! UI-neutral session entities. No filesystem or network dependencies.
pub mod account;
pub mod application_commands;
mod image_sharing;
pub use image_sharing::ImageShare;
pub mod archives;
mod channel_preferences;
pub mod keybinds;
pub mod messaging_permissions;
pub mod notification_preferences;
pub mod voice_settings;
pub use channel_preferences::{ChannelPreferences, PreferenceEdit, Shortcut};
pub use keybinds::{KeyChord, KeybindAction, Keybinds};
pub mod forum;
pub mod gifs;
mod graphics;
pub use graphics::GpuPreference;
pub mod guild_folders;
pub mod permissions;
mod reading_preferences;
pub mod server_admin;
pub mod server_audit_log;
pub mod server_integrations;
pub mod server_invites;
pub mod server_roles;
pub mod server_settings;
pub use reading_preferences::ReadingPreferences;
mod profile;
mod system_messages;
pub use profile::*;
pub use system_messages::{Segment, SystemMessage};
mod attachments;
pub use attachments::*;
mod components;
pub use components::*;
mod embeds;
pub use embeds::*;
mod extra_content;
pub use extra_content::{ExtraContent, ExtraContentPatch};
mod mentions;
pub use mentions::*;
mod stickers;
pub use stickers::*;
mod reactions;
pub use reactions::*;
mod search;
pub use gifs::*;
pub use search::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub u64);
impl fmt::Display for Id {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.fmt(f)
	}
}
impl FromStr for Id {
	type Err = &'static str;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
			return Err("Invalid Discord ID");
		}
		s.parse::<u64>()
			.ok()
			.filter(|v| *v != 0)
			.map(Self)
			.ok_or("Invalid Discord ID")
	}
}
impl Serialize for Id {
	fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_str(&self.to_string())
	}
}
impl<'de> Deserialize<'de> for Id {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct IdVisitor;
		impl serde::de::Visitor<'_> for IdVisitor {
			type Value = Id;
			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				f.write_str("a Discord ID string")
			}
			fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Id, E> {
				value.parse().map_err(E::custom)
			}
		}
		d.deserialize_str(IdVisitor)
	}
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
	#[serde(default)]
	pub kind: AccountKind,
	/// Set only for message authors with a service-provided webhook_id.
	#[serde(default)]
	pub webhook: bool,
	pub id: Id,
	pub name: String,
	pub avatar: Option<String>,
	pub discriminator: u16,
	/// Service-supplied server identity displayed beside this user's name.
	#[serde(default, skip_serializing)]
	pub primary_guild: Option<Box<ClanTag>>,
}
impl User {
	pub fn account_label(&self) -> Option<&'static str> {
		match (self.kind, self.webhook) {
			(AccountKind::App, _) => Some("APP"),
			(_, true) => Some("WEBHOOK"),
			(AccountKind::Bot, _) => Some("BOT"),
			_ => None,
		}
	}
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity()
			+ self.avatar.as_ref().map_or(0, String::capacity)
			+ self.primary_guild.as_ref().map_or(0, |guild| {
				std::mem::size_of::<ClanTag>()
					+ guild.tag.capacity()
					+ guild.badge.as_ref().map_or(0, String::capacity)
			})
	}
	pub fn avatar_key(&self) -> String {
		if let Some(hash) = self
			.avatar
			.as_deref()
			.filter(|hash| valid_avatar_hash(hash))
		{
			format!("{}-{hash}", self.id)
		} else {
			let index = if self.discriminator == 0 {
				(self.id.0 >> 22) % 6
			} else {
				u64::from(self.discriminator % 5)
			};
			format!("default-{index}")
		}
	}
	pub fn avatar_url(&self) -> String {
		let key = self.avatar_key();
		if let Some(index) = key.strip_prefix("default-") {
			format!("https://cdn.discordapp.com/embed/avatars/{index}.png")
		} else {
			let (_, hash) = key.split_once('-').expect("avatar key");
			let ext = if hash.starts_with("a_") { "gif" } else { "png" };
			format!(
				"https://cdn.discordapp.com/avatars/{}/{hash}.{ext}?size=128",
				self.id
			)
		}
	}
}
/// Explicit service metadata, never inferred from a name or a failed profile request.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccountKind {
	#[default]
	Human = 0,
	Bot = 1,
	App = 2,
}
/// Locally remembered account for the switcher: identity only, never a token.
/// Tokens stay in the OS credential store under their own per-account entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedAccount {
	pub id: Id,
	pub name: String,
	/// Global display name, when the account has one.
	pub display: Option<String>,
	pub avatar: Option<String>,
	pub discriminator: u16,
	/// Whether the OS credential store holds this account's own entry. Read-only output of
	/// the roster: writes go through `set_account_token`, so an identity refresh cannot
	/// claim a token exists. Keeps the client from rewriting an entry it already wrote,
	/// which on macOS is an access-controlled operation that can prompt for the keychain.
	pub has_token: bool,
}
/// Bounded roster: enough for people juggling alternates, small enough to stay readable.
pub const MAX_SAVED_ACCOUNTS: usize = 8;
impl SavedAccount {
	pub fn is_valid(&self) -> bool {
		self.id.0 != 0
			&& (1..=64).contains(&self.name.len())
			&& self
				.display
				.as_ref()
				.is_none_or(|display| (1..=64).contains(&display.len()))
			&& self.avatar.as_deref().is_none_or(valid_avatar_hash)
			&& self.discriminator <= 9999
	}
	/// What the switcher shows: display name when set, otherwise the username.
	pub fn label(&self) -> &str {
		self.display
			.as_deref()
			.filter(|display| !display.is_empty())
			.unwrap_or(&self.name)
	}
	/// Avatar lookups and name rows reuse the ordinary user widgets.
	pub fn user(&self) -> User {
		User {
			kind: AccountKind::Human,
			webhook: false,
			id: self.id,
			name: self.name.clone(),
			avatar: self.avatar.clone(),
			discriminator: self.discriminator,
			primary_guild: None,
		}
	}
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity()
			+ self.display.as_ref().map_or(0, String::capacity)
			+ self.avatar.as_ref().map_or(0, String::capacity)
	}
}
pub fn valid_avatar_hash(hash: &str) -> bool {
	let hash = hash.strip_prefix("a_").unwrap_or(hash);
	hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}
#[derive(Clone)]
pub struct InvitePreview {
	pub guild: Id,
	pub embed: Embed,
}
impl InvitePreview {
	pub fn bytes(&self) -> usize {
		size_of::<Self>() - size_of::<Embed>() + self.embed.bytes()
	}
}
#[derive(Clone, PartialEq, Eq)]
pub struct Guild {
	pub stickers: Option<Vec<Sticker>>,
	pub emojis: Option<Vec<CustomEmoji>>,
	pub id: Id,
	pub name: String,
	pub icon: Option<String>,
}
impl Guild {
	pub fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ self.name.capacity()
			+ self.icon.as_ref().map_or(0, String::capacity)
			+ self.emojis.as_ref().map_or(0, custom_emoji_bytes)
			+ self.stickers.as_ref().map_or(0, sticker_bytes)
	}
	pub fn icon_key(&self) -> Option<String> {
		self.icon
			.as_deref()
			.filter(|hash| valid_avatar_hash(hash))
			.map(|hash| format!("guild-{}-{hash}", self.id))
	}
}
#[derive(Clone)]
pub struct GuildPatch {
	pub id: Id,
	pub name: Patch<String>,
	pub icon: Patch<String>,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Channel {
	/// Group DM icon hash; absent for groups using the default icon.
	pub icon: Option<String>,
	pub last_message: Option<Id>,
	pub id: Id,
	pub guild: Option<Id>,
	pub parent_id: Option<Id>,
	pub position: i32,
	pub name: String,
	pub kind: u8,
	pub recipients: Vec<User>,
	/// Unofficial service member-list identity; absent when permission metadata is missing.
	pub member_list_id: Option<String>,
	/// Thread reply count reported by the service; None for non-threads or unknown.
	pub message_count: Option<u32>,
}
impl Channel {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.name.capacity()
			+ self.icon.as_ref().map_or(0, String::capacity)
			+ self.member_list_id.as_ref().map_or(0, String::capacity)
			+ self.recipients.capacity() * size_of::<User>()
			+ self.recipients.iter().map(User::heap_bytes).sum::<usize>()
	}
	pub fn supports_text(&self) -> bool {
		matches!(self.kind, 0..=3 | 5 | 10..=12)
	}
}
#[derive(Clone)]
pub struct ChannelPatch {
	pub icon: Patch<String>,
	pub last_message: Patch<Id>,
	pub id: Id,
	pub name: Patch<String>,
	pub parent_id: Patch<Id>,
	pub position: Patch<i32>,
	pub kind: Patch<u8>,
	pub message_count: Patch<u32>,
}
/// The command invocation that produced an application response message.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interaction {
	pub user: User,
	/// Bounded command name without the leading slash; empty when the service omitted it.
	#[serde(default)]
	pub command: String,
}
impl Interaction {
	pub fn heap_bytes(&self) -> usize {
		size_of::<Self>() + self.user.heap_bytes() + self.command.capacity()
	}
}
#[derive(Clone, PartialEq, Eq)]
pub struct Message {
	pub sticker_items: Vec<Sticker>,
	/// Original outer message flags, retained for interaction submissions.
	pub flags: u64,
	pub ephemeral: bool,
	pub components: Vec<Component>,
	pub application_id: Option<Id>,
	/// Last known counts. None means they have not been loaded yet.
	pub reactions: Option<Vec<Reaction>>,
	pub id: Id,
	pub channel: Id,
	pub author: User,
	/// Role membership supplied with this message.
	pub author_roles: Vec<Id>,
	/// Guild nickname supplied with this message.
	pub author_nick: Option<String>,
	pub content: String,
	pub mentions: Vec<User>,
	/// Session-only service notification metadata; never inferred from message text.
	pub mention_roles: Vec<Id>,
	pub mention_everyone: bool,
	pub suppress_notifications: bool,
	pub edited: bool,
	pub edited_at: Option<i128>,
	pub revision: u64,
	pub nonce: Option<String>,
	pub reply_to: Option<Id>,
	/// Discord message type; 255 denotes an unknown legacy cached type.
	pub kind: u8,
	/// The service explicitly returned a null referenced message, not an unresolved preview.
	pub reply_deleted: bool,
	/// Body is the immutable snapshot attached to a forwarded message.
	pub forwarded: bool,
	/// The application command invocation this message answers.
	pub interaction: Option<Box<Interaction>>,
	pub unsupported: bool,
	pub extra_content: ExtraContent,
	pub embeds: Vec<Embed>,
	pub embeds_suppressed: bool,
	pub attachments: Vec<Attachment>,
}
impl Message {
	/// A plain-text description, separate from the original service content.
	pub fn system_summary(&self) -> Option<String> {
		self.system_message().map(|system| system.summary())
	}

	/// Styled runs describing a service-generated message, or `None` for user content.
	pub fn system_message(&self) -> Option<SystemMessage> {
		system_messages::describe(self)
	}

	/// Whether the service, not a user, generated this message.
	pub fn is_system(&self) -> bool {
		system_messages::is_system(self.kind)
	}

	pub fn display_text(&self) -> std::borrow::Cow<'_, str> {
		match self.system_message() {
			Some(system) if system.content_shown || self.content.is_empty() => {
				system.summary().into()
			}
			Some(system) => format!("{}\n{}", system.summary(), self.content).into(),
			None => self.content.as_str().into(),
		}
	}

	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.reactions.as_ref().map_or(0, |r| {
				reaction_bytes(r) + r.capacity().saturating_sub(r.len()) * size_of::<Reaction>()
			}) + self.content.capacity()
			+ self.author.heap_bytes()
			+ self.interaction.as_ref().map_or(0, |i| i.heap_bytes())
			+ self.author_nick.as_ref().map_or(0, String::capacity)
			+ self.author_roles.capacity() * size_of::<Id>()
			+ mention_bytes(&self.mentions)
			+ self.mention_roles.capacity() * size_of::<Id>()
			+ self.nonce.as_ref().map_or(0, String::capacity)
			+ attachment_bytes(&self.attachments)
			+ self
				.attachments
				.capacity()
				.saturating_sub(self.attachments.len())
				* size_of::<Attachment>()
			+ sticker_bytes(&self.sticker_items)
			+ component_bytes(&self.components)
			+ embed_bytes(&self.embeds)
			+ self.embeds.capacity().saturating_sub(self.embeds.len()) * size_of::<Embed>()
	}
}
pub const MAX_MENTION_ROLES: usize = 100;
pub fn valid_mention_roles(roles: &Vec<Id>) -> bool {
	roles.len() <= MAX_MENTION_ROLES
		&& roles.capacity() <= MAX_MENTION_ROLES
		&& roles
			.iter()
			.enumerate()
			.all(|(index, id)| id.0 != 0 && !roles[..index].contains(id))
}
/// Missing differs from explicit null in partial service updates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Patch<T> {
	#[default]
	Absent,
	Null,
	Value(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		Ok(Option::<T>::deserialize(d)?.map_or(Self::Null, Self::Value))
	}
}
#[derive(Clone)]
pub struct MessagePatch {
	pub sticker_items: Patch<Vec<Sticker>>,
	pub flags: Patch<u64>,
	pub components: Patch<Vec<Component>>,
	pub application_id: Patch<Id>,
	pub extra_content: ExtraContentPatch,
	pub reactions: Patch<Vec<Reaction>>,
	pub id: Id,
	pub channel: Id,
	pub content: Patch<String>,
	pub mentions: Patch<Vec<User>>,
	pub edited: Patch<i128>,
	pub embeds: Patch<Vec<Embed>>,
	pub embeds_suppressed: Patch<bool>,
	pub attachments: Patch<Vec<Attachment>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
	Loading,
	Fresh,
	Stale,
	Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
	Sending,
	Confirmed,
	Rejected,
	Ambiguous,
}

pub const MAX_RICH_ACTIVITIES: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PresenceStatus {
	#[default]
	Online,
	Idle,
	DoNotDisturb,
	Invisible,
}
impl PresenceStatus {
	pub const ALL: [Self; 4] = [
		Self::Online,
		Self::Idle,
		Self::DoNotDisturb,
		Self::Invisible,
	];
	pub fn wire(self) -> &'static str {
		match self {
			Self::Online => "online",
			Self::Idle => "idle",
			Self::DoNotDisturb => "dnd",
			Self::Invisible => "invisible",
		}
	}
	pub fn label(self) -> &'static str {
		match self {
			Self::Online => "Online",
			Self::Idle => "Idle",
			Self::DoNotDisturb => "Do Not Disturb",
			Self::Invisible => "Invisible",
		}
	}
	pub fn parse(wire: &str) -> Option<Self> {
		match wire {
			"online" => Some(Self::Online),
			"idle" => Some(Self::Idle),
			"dnd" => Some(Self::DoNotDisturb),
			"invisible" => Some(Self::Invisible),
			_ => None,
		}
	}
}

/// This account's chosen status. Discord settings are authoritative. A local row is only the fallback when that read fails.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnPresence {
	pub status: PresenceStatus,
	pub custom_status: String,
	/// Custom-status clear deadline, unix milliseconds. The gateway presence opcode does not carry it.
	pub expires_at_ms: Option<u64>,
}
impl OwnPresence {
	pub fn valid(&self) -> bool {
		self.custom_status.is_empty() || valid_presence_text(&self.custom_status)
	}
}

/// A service image reference, never permission to fetch an arbitrary external URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivityImage {
	Asset { application: Id, asset: Id },
	Proxy(String),
	Spotify(String),
	Application(Id),
}
impl ActivityImage {
	pub fn valid(&self) -> bool {
		match self {
			Self::Asset { application, asset } => application.0 != 0 && asset.0 != 0,
			Self::Application(id) => id.0 != 0,
			Self::Spotify(id) => id.len() == 40 && id.bytes().all(|b| b.is_ascii_hexdigit()),
			Self::Proxy(path) => {
				!path.is_empty()
					&& path.len() <= 1024
					&& !path.starts_with('/')
					&& !path
						.chars()
						.any(|c| c.is_control() || c.is_whitespace() || c == '\\')
					&& !path.split('/').any(|part| matches!(part, "." | ".."))
					&& !path.as_bytes().windows(3).any(|part| {
						part.eq_ignore_ascii_case(b"%2e")
							|| part.eq_ignore_ascii_case(b"%2f")
							|| part.eq_ignore_ascii_case(b"%5c")
					})
			}
		}
	}
	pub fn heap_bytes(&self) -> usize {
		match self {
			Self::Proxy(path) | Self::Spotify(path) => path.capacity(),
			_ => 0,
		}
	}
	pub fn key(&self) -> String {
		match self {
			Self::Asset { application, asset } => format!("activity-{application}-{asset}"),
			Self::Proxy(path) => format!("embed:https://media.discordapp.net/{path}"),
			Self::Application(id) => format!("app-icon-{id}"),
			Self::Spotify(id) => format!("spotify-{id}"),
		}
	}
}

/// Bounded activity metadata. Secrets and actions are never retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichActivity {
	pub kind: u8,
	pub name: String,
	pub details: Option<String>,
	pub state: Option<String>,
	pub image: Option<ActivityImage>,
	pub small_image: Option<ActivityImage>,
	/// Unix milliseconds, as supplied by the activity producer.
	pub started_at: Option<u64>,
	/// Track end in Unix milliseconds; absent when duration is unknown.
	pub ends_at: Option<u64>,
}
pub const MAX_ACTIVITY_TIMESTAMP: u64 = 9_007_199_254_740_991;
impl RichActivity {
	pub fn valid(&self) -> bool {
		matches!(self.kind, 0..=3 | 5)
			&& valid_presence_text(&self.name)
			&& self.details.as_deref().is_none_or(valid_presence_text)
			&& self.state.as_deref().is_none_or(valid_presence_text)
			&& self.image.as_ref().is_none_or(ActivityImage::valid)
			&& self.small_image.as_ref().is_none_or(ActivityImage::valid)
			&& self
				.started_at
				.is_none_or(|at| at <= MAX_ACTIVITY_TIMESTAMP)
			&& self.ends_at.is_none_or(|end| {
				end <= MAX_ACTIVITY_TIMESTAMP && self.started_at.is_some_and(|start| end > start)
			})
	}
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity()
			+ self.details.as_ref().map_or(0, String::capacity)
			+ self.state.as_ref().map_or(0, String::capacity)
			+ self.image.as_ref().map_or(0, ActivityImage::heap_bytes)
			+ self
				.small_image
				.as_ref()
				.map_or(0, ActivityImage::heap_bytes)
	}
	pub fn summary(&self) -> String {
		let verb = match self.kind {
			0 => "Playing",
			1 => "Streaming",
			2 => "Listening to",
			3 => "Watching",
			5 => "Competing in",
			_ => return self.name.clone(),
		};
		format!("{verb} {}", self.name)
	}
}

fn valid_presence_text(text: &str) -> bool {
	!text.is_empty()
		&& text.len() <= 512
		&& text.chars().count() <= 128
		&& text.trim() == text
		&& !text.chars().any(char::is_control)
}

/// Complete, bounded presence values for an already-loaded user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberPresence {
	pub user: Id,
	pub status: Option<String>,
	pub custom_status: Option<String>,
	pub activities: Vec<RichActivity>,
}

impl MemberPresence {
	pub fn valid(&self) -> bool {
		self.user.0 != 0
			&& self
				.status
				.as_deref()
				.is_none_or(|status| matches!(status, "online" | "idle" | "dnd" | "offline"))
			&& self
				.custom_status
				.as_deref()
				.is_none_or(valid_presence_text)
			&& self.activities.len() <= MAX_RICH_ACTIVITIES
			&& self.activities.iter().all(RichActivity::valid)
	}
	pub fn heap_bytes(&self) -> usize {
		self.status.as_ref().map_or(0, String::capacity)
			+ self.custom_status.as_ref().map_or(0, String::capacity)
			+ self.activities.capacity() * size_of::<RichActivity>()
			+ self
				.activities
				.iter()
				.map(RichActivity::heap_bytes)
				.sum::<usize>()
	}
}

/// Only the active member pane is retained; group rows and unloaded slots remain None.
#[derive(Clone)]
pub struct Member {
	pub roles: Vec<Id>,
	pub user: User,
	pub nick: Option<String>,
	pub status: Option<String>,
	/// Custom status text with any unicode emoji; bounded, never a rich activity.
	pub custom_status: Option<String>,
	pub activities: Vec<RichActivity>,
}
impl Member {
	pub fn valid(&self) -> bool {
		self.user.id.0 != 0
			&& self
				.status
				.as_deref()
				.is_none_or(|status| matches!(status, "online" | "idle" | "dnd" | "offline"))
			&& self
				.custom_status
				.as_deref()
				.is_none_or(valid_presence_text)
			&& self.activities.len() <= MAX_RICH_ACTIVITIES
			&& self.activities.iter().all(RichActivity::valid)
	}
	/// Drops presence details this client cannot show, keeping the member row itself.
	pub fn sanitize_presence(&mut self) {
		if self
			.status
			.as_deref()
			.is_some_and(|status| !matches!(status, "online" | "idle" | "dnd" | "offline"))
		{
			self.status = None;
		}
		if self
			.custom_status
			.as_deref()
			.is_some_and(|text| !valid_presence_text(text))
		{
			self.custom_status = None;
		}
		self.activities.retain(RichActivity::valid);
		self.activities.truncate(MAX_RICH_ACTIVITIES);
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.roles.capacity() * size_of::<Id>()
			+ self.user.heap_bytes()
			+ self.nick.as_ref().map_or(0, String::capacity)
			+ self.status.as_ref().map_or(0, String::capacity)
			+ self.custom_status.as_ref().map_or(0, String::capacity)
			+ self.activities.capacity() * size_of::<RichActivity>()
			+ self
				.activities
				.iter()
				.map(RichActivity::heap_bytes)
				.sum::<usize>()
	}
}
#[derive(Clone)]
pub enum MemberSlot {
	Person(Member),
	/// Gateway group id: role snowflake, "online", or "offline".
	Group(String),
}

impl MemberSlot {
	pub fn bytes(&self) -> usize {
		match self {
			Self::Person(member) => member.bytes(),
			Self::Group(id) => id.capacity(),
		}
	}
}

#[derive(Clone)]
pub struct MemberList {
	pub guild: Option<Id>,
	pub channel: Id,
	pub request: u64,
	/// Absolute index of `slots[0]`.
	pub start: usize,
	/// Contiguous window. None is a hole. At most 200 entries.
	pub slots: Vec<Option<MemberSlot>>,
	pub total: u64,
	/// Guild channel lazy list. Scrollbar length is `total`. DMs and threads are false and scroll `slots.len()`.
	pub lazy: bool,
	pub freshness: Freshness,
	/// id -> count from the update's top-level groups array. At most MAX_ROLES + 2.
	pub groups: Vec<(String, u64)>,
	/// Ranges last requested for a lazy guild list.
	pub ranges: Vec<[usize; 2]>,
}

impl MemberList {
	pub fn slot_bytes(&self) -> usize {
		self.slots.iter().flatten().map(MemberSlot::bytes).sum()
	}
}

#[cfg(test)]
mod presence_tests {
	use super::*;

	#[test]
	fn own_presence_bounds_unicode_and_allows_explicit_clear() {
		let mut presence = OwnPresence::default();
		assert!(presence.valid());
		assert_eq!(
			PresenceStatus::ALL.map(PresenceStatus::wire),
			["online", "idle", "dnd", "invisible"]
		);
		for text in [
			" x".into(),
			"x ".into(),
			"line\nfeed".into(),
			"\0".into(),
			"x".repeat(129),
			"🦀".repeat(129),
		] {
			presence.custom_status = text;
			assert!(!presence.valid());
		}
		presence.custom_status = "🦀".repeat(128);
		assert!(presence.valid());
		assert_eq!(presence.custom_status.len(), 512);
		presence.custom_status.clear();
		assert!(presence.valid());
	}

	#[test]
	fn rich_presence_validates_retained_fields_and_accounts_for_allocations() {
		let activity = RichActivity {
			kind: 0,
			name: "Synthetic".into(),
			details: Some("Level 2".into()),
			state: Some("In a party".into()),
			image: None,
			small_image: None,
			ends_at: None,
			started_at: None,
		};
		let mut presence = MemberPresence {
			user: Id(2),
			status: None,
			custom_status: None,
			activities: vec![activity.clone(); MAX_RICH_ACTIVITIES],
		};
		assert!(presence.valid());
		assert_eq!(
			presence.heap_bytes(),
			presence.activities.capacity() * size_of::<RichActivity>()
				+ presence
					.activities
					.iter()
					.map(RichActivity::heap_bytes)
					.sum::<usize>()
		);
		presence.activities.push(activity.clone());
		assert!(!presence.valid());
		for text in [
			"",
			" padded",
			"control\n",
			&"x".repeat(129),
			&"🌙".repeat(129),
		] {
			let mut invalid = activity.clone();
			invalid.name = text.into();
			assert!(!invalid.valid());
			invalid.name = activity.name.clone();
			invalid.details = Some(text.into());
			assert!(!invalid.valid());
			invalid.details = None;
			invalid.state = Some(text.into());
			assert!(!invalid.valid());
		}
		for kind in [4, 6, 255] {
			assert!(
				!RichActivity {
					kind,
					..activity.clone()
				}
				.valid()
			);
		}
		let mut allocated = activity;
		allocated.name.reserve(100);
		let mut path = String::from("external/synthetic-hash-01/https/example.com/art.png");
		path.reserve(2048);
		allocated.image = Some(ActivityImage::Proxy(path));
		let mut small_path = String::from("external/synthetic-small/https/example.com/badge.png");
		small_path.reserve(1024);
		allocated.small_image = Some(ActivityImage::Proxy(small_path));
		assert!(allocated.valid());
		assert_eq!(
			allocated.heap_bytes(),
			allocated.name.capacity()
				+ allocated.details.as_ref().unwrap().capacity()
				+ allocated.state.as_ref().unwrap().capacity()
				+ allocated.image.as_ref().unwrap().heap_bytes()
				+ allocated.small_image.as_ref().unwrap().heap_bytes()
		);
		allocated.started_at = Some(MAX_ACTIVITY_TIMESTAMP);
		assert!(allocated.valid());
		allocated.started_at = Some(MAX_ACTIVITY_TIMESTAMP + 1);
		assert!(!allocated.valid());
		allocated.started_at = None;
		allocated.small_image = Some(ActivityImage::Proxy("external/../secret".into()));
		assert!(!allocated.valid());
	}

	#[test]
	fn activity_image_keys_preserve_only_bounded_service_references() {
		for (image, key) in [
			(
				ActivityImage::Asset {
					application: Id(10),
					asset: Id(20),
				},
				"activity-10-20",
			),
			(ActivityImage::Application(Id(10)), "app-icon-10"),
			(
				ActivityImage::Proxy("external/synthetic-hash-01/https/example.com/art.png".into()),
				"embed:https://media.discordapp.net/external/synthetic-hash-01/https/example.com/art.png",
			),
		] {
			assert!(image.valid());
			assert_eq!(image.key(), key);
		}
		assert!(!ActivityImage::Application(Id(0)).valid());
		assert!(
			!ActivityImage::Asset {
				application: Id(1),
				asset: Id(0)
			}
			.valid()
		);
		assert!(!ActivityImage::Proxy("external/../secret".into()).valid());
		assert!(!ActivityImage::Proxy("x".repeat(1025)).valid());
	}
}

#[cfg(test)]
mod notification_metadata_tests {
	use super::*;
	#[test]
	fn role_mentions_bound_identity_count_and_reserved_allocation() {
		assert!(valid_mention_roles(&Vec::new()));
		assert!(valid_mention_roles(&(1..=100).map(Id).collect()));
		assert!(!valid_mention_roles(&vec![Id(0)]));
		assert!(!valid_mention_roles(&vec![Id(1), Id(1)]));
		assert!(!valid_mention_roles(&(1..=101).map(Id).collect()));
		assert!(!valid_mention_roles(&Vec::with_capacity(101)));
	}
}
