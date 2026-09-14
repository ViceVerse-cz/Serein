//! Discord wire DTOs. JSON values never become application state.
pub mod activity_sessions;
pub mod activity_sharing;
pub mod archives;
mod attachments;
mod embeds;
mod extra_content;
pub mod forum;
pub mod gifs;
pub mod group_actions;
pub mod guild_folders;
pub mod invites;
pub mod messaging_permissions;
pub mod notification_settings;
pub mod notifications;
pub mod permissions;
pub mod pins;
pub mod presence;
pub mod profile;
mod reactions;
pub mod read_state;
pub mod ready;
pub mod relationships;
pub mod rpc;
pub mod search;
pub mod server_admin;
pub mod server_audit_log;
pub mod server_integrations;
pub mod server_invites;
pub mod server_roles;
pub mod server_settings;
pub mod stream;
pub mod threads;
pub mod typing;
use attachments::AttachmentList;
use embeds::EmbedList;
use model::{Channel, Guild, Id, Message, MessagePatch, Patch, User};
pub use reactions::{GuildEmojisUpdate, ReactionDelta, ReactionEmojiTarget, ReactionTarget};
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Deserialize, Debug, PartialEq, Eq)]
#[serde(try_from = "String")]
pub struct Timestamp(i128);
impl TryFrom<String> for Timestamp {
	type Error = &'static str;
	fn try_from(value: String) -> Result<Self, Self::Error> {
		time::OffsetDateTime::parse(&value, &time::format_description::well_known::Rfc3339)
			.map(|t| Self(t.unix_timestamp_nanos()))
			.map_err(|_| "Invalid service timestamp")
	}
}
pub const MAX_WIRE: usize = 4 * 1024 * 1024;
/// A normal account's READY carries every joined server's channels, roles, emojis and settings,
/// and legitimately exceeds the per-event/REST bound. Retained projections keep their own limits.
pub const MAX_GATEWAY_WIRE: usize = 64 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
#[error("Unsupported or oversized Discord payload")]
pub struct DecodeError;
pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
	decode_limited(bytes, MAX_WIRE)
}
/// Whole Gateway packets and login snapshots (READY, READY_SUPPLEMENTAL).
pub fn decode_gateway<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, DecodeError> {
	decode_limited(bytes, MAX_GATEWAY_WIRE)
}
fn decode_limited<T: serde::de::DeserializeOwned>(
	bytes: &[u8],
	limit: usize,
) -> Result<T, DecodeError> {
	if bytes.len() > limit {
		return Err(DecodeError);
	}
	serde_json::from_slice(bytes).map_err(|_| DecodeError)
}
#[derive(Clone, Deserialize)]
pub struct UserDto {
	pub id: Id,
	pub username: String,
	#[serde(default)]
	pub global_name: Option<String>,
	#[serde(default)]
	pub bot: bool,
	#[serde(default)]
	pub avatar: Option<String>,
	#[serde(default)]
	pub discriminator: String,
}
impl UserDto {
	pub fn into_model(self) -> User {
		User {
			kind: if self.bot {
				model::AccountKind::Bot
			} else {
				model::AccountKind::Human
			},
			id: self.id,
			name: self
				.global_name
				.unwrap_or(self.username)
				.chars()
				.take(128)
				.collect(),
			avatar: self.avatar.filter(|hash| model::valid_avatar_hash(hash)),
			webhook: false,
			discriminator: self
				.discriminator
				.parse::<u16>()
				.ok()
				.filter(|n| *n <= 9999)
				.unwrap_or(0),
		}
	}
}
#[derive(Deserialize)]
pub struct ChannelDto {
	#[serde(default)]
	pub icon: Option<String>,
	#[serde(default)]
	pub flags: u64,
	#[serde(default)]
	pub last_message_id: Option<Id>,
	pub id: Id,
	#[serde(default)]
	pub guild_id: Option<Id>,
	#[serde(default)]
	pub parent_id: Option<Id>,
	#[serde(default)]
	pub position: i32,
	#[serde(default)]
	pub name: Option<String>,
	#[serde(rename = "type")]
	pub kind: u8,
	#[serde(default)]
	pub recipients: Vec<UserDto>,
	#[serde(default)]
	pub permission_overwrites: Option<Vec<Overwrite>>,
	#[serde(default)]
	pub message_count: Option<u32>,
}
impl ChannelDto {
	pub fn is_obfuscated(&self) -> bool {
		self.flags & (1 << 17) != 0
	}

	pub fn into_model(self) -> Channel {
		let recipients: Vec<_> = self
			.recipients
			.into_iter()
			.take(64)
			.map(UserDto::into_model)
			.collect();
		Channel {
			icon: self.icon.filter(|hash| model::valid_avatar_hash(hash)),
			last_message: self.last_message_id,
			id: self.id,
			guild: self.guild_id,
			parent_id: self.parent_id,
			position: self.position,
			name: self.name.unwrap_or_else(|| {
				recipients
					.iter()
					.map(|u| u.name.as_str())
					.collect::<Vec<_>>()
					.join(", ")
			}),
			kind: self.kind,
			recipients,
			member_list_id: None,
			message_count: self.message_count,
		}
	}
}
#[derive(Deserialize)]
pub struct ChannelPatchDto {
	#[serde(default)]
	pub icon: Patch<String>,
	#[serde(default)]
	pub last_message_id: Patch<Id>,
	pub id: Id,
	#[serde(default)]
	pub name: Patch<String>,
	#[serde(default)]
	pub parent_id: Patch<Id>,
	#[serde(default)]
	pub position: Patch<i32>,
	#[serde(rename = "type", default)]
	pub kind: Patch<u8>,
	#[serde(default)]
	pub permission_overwrites: Patch<Vec<Overwrite>>,
	#[serde(default)]
	pub flags: Patch<u64>,
	#[serde(default)]
	pub message_count: Patch<u32>,
}
impl ChannelPatchDto {
	pub fn is_obfuscated(&self) -> bool {
		matches!(self.flags, Patch::Value(flags) if flags & (1 << 17) != 0)
	}

	pub fn into_model(self) -> model::ChannelPatch {
		model::ChannelPatch {
			icon: match self.icon {
				Patch::Value(hash) if model::valid_avatar_hash(&hash) => Patch::Value(hash),
				Patch::Value(_) | Patch::Null => Patch::Null,
				Patch::Absent => Patch::Absent,
			},
			last_message: self.last_message_id,
			id: self.id,
			name: self.name,
			parent_id: self.parent_id,
			position: self.position,
			kind: self.kind,
			message_count: self.message_count,
		}
	}
}
#[cfg(test)]
mod channel_tests {
	use super::*;
	#[test]
	fn group_icon_hash_and_patches_keep_absent_null_and_value_distinct() {
		let channel = decode::<ChannelDto>(
			br#"{"id":"3","type":3,"icon":"0123456789abcdef0123456789abcdef"}"#,
		)
		.unwrap()
		.into_model();
		assert_eq!(
			channel.icon.as_deref(),
			Some("0123456789abcdef0123456789abcdef")
		);
		assert!(matches!(
			decode::<ChannelPatchDto>(br#"{"id":"3"}"#)
				.unwrap()
				.into_model()
				.icon,
			Patch::Absent
		));
		assert!(matches!(
			decode::<ChannelPatchDto>(br#"{"id":"3","icon":null}"#)
				.unwrap()
				.into_model()
				.icon,
			Patch::Null
		));
		assert!(matches!(
			decode::<ChannelPatchDto>(br#"{"id":"3","icon":"../invalid"}"#)
				.unwrap()
				.into_model()
				.icon,
			Patch::Null
		));
	}
	#[test]
	fn ready_omits_obfuscated_channels_and_their_threads_without_inventing_child_permissions() {
		let mut ready: Ready = decode(br#"{
            "user":{"id":"9","username":"Synthetic"},"session_id":"synthetic",
            "resume_gateway_url":"wss://gateway.discord.gg", "private_channels":[{"id":"8","type":1}],
            "guilds":[{"id":"1","name":"Synthetic","channels":[
                {"id":"2","type":0,"flags":131072,"name":"not-a-placeholder"},
                {"id":"7","type":4,"flags":131072,"name":"hidden category"},
                {"id":"5","type":0,"parent_id":"7","name":"___hidden___"}
            ],"threads":[
                {"id":"3","type":11,"parent_id":"2","name":"Hidden parent's thread"},
                {"id":"4","type":11,"parent_id":"5","name":"Visible thread"},
                {"id":"6","type":11,"parent_id":"5","flags":131072}
            ]}]}"#).unwrap();
		let (guilds, channels) = ready.navigation().unwrap();
		assert_eq!(guilds.len(), 1);
		assert_eq!(
			channels.iter().map(|c| c.id).collect::<Vec<_>>(),
			vec![Id(8), Id(5), Id(4)]
		);
		assert_eq!(
			channels[1].name, "___hidden___",
			"Names never imply visibility"
		);
		let flags = decode::<ChannelDto>(br#"{"id":"3","type":0,"flags":16}"#).unwrap();
		assert!(!flags.is_obfuscated());
		let payload = serde_json::json!({"user":{"id":"9","username":"Synthetic"},"session_id":"s","resume_gateway_url":"wss://gateway.discord.gg","guilds":[{"id":"1","channels":[{"id":"2","type":0,"flags":131072},{"id":"2","type":0}]}]});
		let mut ready: Ready = decode(&serde_json::to_vec(&payload).unwrap()).unwrap();
		assert!(
			ready.navigation().is_err(),
			"Filtering must not bypass duplicate identity validation"
		);
	}

	#[test]
	fn category_metadata_and_partial_channel_updates() {
		let channel = decode::<ChannelDto>(
			br#"{"id":"3","guild_id":"1","type":0,"name":"general","position":12,"parent_id":"2"}"#,
		)
		.unwrap()
		.into_model();
		assert_eq!(channel.parent_id, Some(Id(2)));
		assert_eq!(channel.position, 12);
		assert!(channel.supports_text());
		let category =
			decode::<ChannelDto>(br#"{"id":"2","guild_id":"1","type":4,"name":"Category"}"#)
				.unwrap()
				.into_model();
		assert!(!category.supports_text());
		let patch = decode::<ChannelPatchDto>(br#"{"id":"3","position":0,"parent_id":null}"#)
			.unwrap()
			.into_model();
		assert_eq!(patch.parent_id, Patch::Null);
		assert_eq!(patch.position, Patch::Value(0));
		assert_eq!(patch.name, Patch::Absent);
		assert_eq!(patch.kind, Patch::Absent);
		assert_eq!(
			decode::<ChannelPatchDto>(br#"{"id":"3"}"#)
				.unwrap()
				.into_model()
				.parent_id,
			Patch::Absent
		);
		assert!(decode::<ChannelDto>(br#"{"id":"3","type":0,"position":4294967296}"#).is_err());
	}
}
#[derive(Deserialize)]
pub struct GuildDto {
	#[serde(default)]
	pub emojis: Option<reactions::CustomEmojiList>,
	pub id: Id,
	#[serde(default)]
	pub properties: Option<GuildProperties>,
	#[serde(default)]
	pub icon: Option<String>,
	#[serde(default)]
	pub name: String,
	#[serde(default)]
	pub channels: Vec<ChannelDto>,
	#[serde(default, deserialize_with = "threads::list")]
	pub threads: Vec<ChannelDto>,
	#[serde(default)]
	pub roles: Vec<RoleDto>,
	#[serde(default)]
	pub voice_states: Vec<VoiceStateDto>,
	#[serde(default)]
	pub members: Vec<VoiceMemberDto>,
}
#[derive(Deserialize)]
pub struct GuildProperties {
	#[serde(default)]
	pub name: Patch<String>,
	#[serde(default)]
	pub icon: Patch<String>,
}
#[derive(Deserialize)]
pub struct GuildPatchDto {
	pub id: Id,
	#[serde(flatten)]
	pub properties: GuildProperties,
}
impl GuildPatchDto {
	pub fn into_model(self) -> model::GuildPatch {
		model::GuildPatch {
			id: self.id,
			name: self.properties.name,
			icon: self.properties.icon,
		}
	}
}
#[derive(Deserialize)]
pub struct Ready {
	#[serde(default)]
	pub relationships: Option<relationships::Snapshot>,
	#[serde(default)]
	pub presences: Option<Box<RawValue>>,
	#[serde(default)]
	pub merged_presences: Option<presence::MergedPresences>,
	#[serde(default)]
	pub users: Vec<UserDto>,
	#[serde(default)]
	pub read_state: Option<read_state::Snapshot>,
	#[serde(default)]
	pub user_guild_settings: Option<notifications::Snapshot>,
	#[serde(default)]
	pub sessions: Option<notifications::Sessions>,
	pub user: UserDto,
	pub session_id: String,
	pub resume_gateway_url: String,
	#[serde(default)]
	pub guilds: Vec<GuildDto>,
	#[serde(default)]
	pub private_channels: Vec<ChannelDto>,
}
impl Ready {
	pub fn navigation(&mut self) -> Result<(Vec<Guild>, Vec<Channel>), DecodeError> {
		let incoming = self.private_channels.len()
			+ self.guilds.len()
			+ self
				.guilds
				.iter()
				.map(|g| g.channels.len() + g.threads.len())
				.sum::<usize>();
		if incoming > threads::MAX_ITEMS {
			return Err(DecodeError);
		}
		let mut guild_ids = std::collections::BTreeSet::new();
		if self.user.id.0 == 0
			|| self.guilds.iter().any(|g| {
				g.id.0 == 0
					|| !guild_ids.insert(g.id)
					|| g.channels
						.iter()
						.chain(&g.threads)
						.any(|c| c.guild_id.is_some_and(|id| id != g.id))
			}) {
			return Err(DecodeError);
		}
		let mut ids = std::collections::BTreeSet::new();
		if self
			.private_channels
			.iter()
			.chain(
				self.guilds
					.iter()
					.flat_map(|g| g.channels.iter().chain(&g.threads)),
			)
			.any(|c| c.id.0 == 0 || !ids.insert(c.id))
		{
			return Err(DecodeError);
		}
		drop(ids);
		let mut channels: Vec<_> = std::mem::take(&mut self.private_channels)
			.into_iter()
			.filter(|c| !c.is_obfuscated())
			.map(ChannelDto::into_model)
			.collect();
		let guilds = std::mem::take(&mut self.guilds)
			.into_iter()
			.map(|mut g| {
				// Normal-user READY may wrap guild identity in `properties`; public bot objects
				// are flat. The nested values override only fields actually present there.
				if let Some(properties) = g.properties {
					match properties.name {
						Patch::Value(name) => g.name = name,
						Patch::Null => g.name.clear(),
						Patch::Absent => {}
					}
					match properties.icon {
						Patch::Value(icon) => g.icon = Some(icon),
						Patch::Null => g.icon = None,
						Patch::Absent => {}
					}
				}
				let everyone = g
					.roles
					.iter()
					.find(|r| r.id == g.id)
					.and_then(|r| r.permissions.parse::<u128>().ok());
				let hidden: std::collections::BTreeSet<_> = g
					.channels
					.iter()
					.filter(|c| c.is_obfuscated())
					.map(|c| c.id)
					.collect();
				channels.extend(
					g.channels
						.into_iter()
						.filter(|c| {
							!c.is_obfuscated()
								&& !(matches!(c.kind, 10..=12)
									&& c.parent_id.is_some_and(|id| hidden.contains(&id)))
						})
						.map(|mut c| {
							c.guild_id = Some(g.id);
							let list_id = everyone.and_then(|permissions| {
								c.permission_overwrites
									.as_ref()
									.and_then(|o| member_list_id(permissions, o))
							});
							let mut channel = c.into_model();
							channel.member_list_id = list_id;
							channel
						}),
				);
				for thread in g.threads {
					if thread.is_obfuscated()
						|| thread.parent_id.is_some_and(|id| hidden.contains(&id))
					{
						continue;
					}
					channels.push(threads::into_thread(thread, g.id)?);
				}
				Ok(Guild {
					emojis: g.emojis.map(|emojis| emojis.0),
					id: g.id,
					name: g.name.chars().take(128).collect(),
					icon: g.icon.filter(|hash| model::valid_avatar_hash(hash)),
				})
			})
			.collect::<Result<Vec<_>, DecodeError>>()?;
		let bytes = channels.iter().map(Channel::bytes).sum::<usize>()
			+ guilds.iter().map(Guild::bytes).sum::<usize>();
		if channels.len() + guilds.len() > threads::MAX_ITEMS || bytes > model::account::MAX_BYTES {
			return Err(DecodeError);
		}
		Ok((guilds, channels))
	}
}
#[derive(Deserialize, Default)]
pub struct MentionList(#[serde(deserialize_with = "model::deserialize_mentions")] pub Vec<UserDto>);
fn mention_roles<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Id>, D::Error> {
	struct Roles;
	impl<'de> serde::de::Visitor<'de> for Roles {
		type Value = Vec<Id>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("at most 100 unique role IDs")
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(
			self,
			mut seq: A,
		) -> Result<Self::Value, A::Error> {
			let mut roles = Vec::new();
			while let Some(role) = seq.next_element::<Id>()? {
				if roles.len() == model::MAX_MENTION_ROLES || roles.contains(&role) {
					return Err(serde::de::Error::custom(
						"Invalid or excessive role mentions",
					));
				}
				roles.push(role);
			}
			Ok(roles.into_boxed_slice().into_vec())
		}
	}
	d.deserialize_seq(Roles)
}
#[derive(Deserialize)]
pub struct MessageDto {
	#[serde(default)]
	pub application_id: Option<Id>,
	#[serde(default)]
	pub webhook_id: Option<Id>,
	#[serde(default)]
	pub poll: Option<extra_content::Object>,
	#[serde(default)]
	pub sticker_items: Option<extra_content::Array>,
	#[serde(default)]
	pub stickers: Option<extra_content::Array>,
	#[serde(default)]
	pub components: Option<extra_content::Array>,
	#[serde(default)]
	pub reactions: reactions::ReactionList,
	pub id: Id,
	pub channel_id: Id,
	pub author: UserDto,
	#[serde(default)]
	pub content: String,
	#[serde(default)]
	pub mentions: MentionList,
	#[serde(default, deserialize_with = "mention_roles")]
	pub mention_roles: Vec<Id>,
	#[serde(default)]
	pub mention_everyone: bool,
	#[serde(default)]
	pub edited_timestamp: Option<Timestamp>,
	#[serde(default)]
	pub nonce: Option<Nonce>,
	#[serde(default)]
	pub message_reference: Option<Reference>,
	#[serde(default)]
	pub message_snapshots: Snapshots,
	#[serde(default)]
	pub referenced_message: model::Patch<extra_content::Object>,
	#[serde(default)]
	pub attachments: AttachmentList,
	#[serde(default)]
	pub embeds: EmbedList,
	#[serde(default)]
	pub flags: u64,
	#[serde(rename = "type", default)]
	pub kind: u8,
}
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Nonce {
	Text(String),
	Number(u64),
}
#[derive(Deserialize)]
pub struct Reference {
	pub message_id: Option<Id>,
	pub channel_id: Option<Id>,
	#[serde(rename = "type", default)]
	pub kind: u64,
}
#[derive(Default)]
pub struct Snapshots(Option<Snapshot>);
impl<'de> Deserialize<'de> for Snapshots {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct Visitor;
		impl<'de> serde::de::Visitor<'de> for Visitor {
			type Value = Snapshots;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("a bounded message snapshot array")
			}
			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut seq: A,
			) -> Result<Snapshots, A::Error> {
				let first = seq.next_element::<Snapshot>()?;
				let mut extra = false;
				for _ in 0..100 {
					if seq.next_element::<serde::de::IgnoredAny>()?.is_none() {
						return Ok(Snapshots(if extra { None } else { first }));
					}
					extra = true;
				}
				Err(serde::de::Error::custom("Too many message snapshots"))
			}
		}
		d.deserialize_seq(Visitor)
	}
}
#[derive(Deserialize)]
struct Snapshot {
	message: SnapshotBody,
}
#[derive(Deserialize)]
struct SnapshotBody {
	#[serde(rename = "type")]
	kind: u8,
	#[serde(default)]
	content: String,
	#[serde(default)]
	attachments: AttachmentList,
	#[serde(default)]
	embeds: EmbedList,
	#[serde(default)]
	flags: u64,
	#[serde(default)]
	poll: Option<extra_content::Object>,
	#[serde(default)]
	sticker_items: Option<extra_content::Array>,
	#[serde(default)]
	stickers: Option<extra_content::Array>,
	#[serde(default)]
	components: Option<extra_content::Array>,
}
impl MessageDto {
	pub fn into_model(mut self) -> Message {
		let snapshot = self.message_snapshots.0.take().filter(|snapshot| {
			self.kind == 0
				&& self.message_reference.as_ref().is_some_and(|r| r.kind == 1)
				&& matches!(snapshot.message.kind, 0 | 19 | 20)
				&& snapshot.message.content.len() <= 64 * 1024
		});
		let forwarded = snapshot.is_some();
		let mut snapshot_flags = None;
		if let Some(Snapshot { message }) = snapshot {
			self.content = message.content;
			self.attachments = message.attachments;
			self.embeds = message.embeds;
			self.poll = message.poll;
			self.sticker_items = message.sticker_items;
			self.stickers = message.stickers;
			self.components = message.components;
			snapshot_flags = Some(message.flags);
		}
		// Reply navigation is confined to this conversation. Crossposts and
		// incomplete/unknown references without a supported snapshot retain the ordinary unsupported-content fallback.
		let reply_to = self
			.message_reference
			.as_ref()
			.filter(|reference| {
				matches!(self.kind, 19 | 23)
					&& self.flags & (1 << 1) == 0
					&& reference.kind == 0
					&& reference.channel_id == Some(self.channel_id)
			})
			.and_then(|reference| reference.message_id)
			.filter(|id| id.0 > 0 && id.0 < self.id.0);
		let unsupported_reference =
			self.message_reference.is_some() && reply_to.is_none() && !forwarded;
		let reply_deleted =
			reply_to.is_some() && matches!(self.referenced_message, model::Patch::Null);
		let mut author = self.author.into_model();
		author.webhook = self.webhook_id.is_some();
		if self.application_id.is_some()
			&& (author.webhook || author.kind == model::AccountKind::Bot)
		{
			author.kind = model::AccountKind::App;
		}
		Message {
			extra_content: model::ExtraContent {
				poll: self.poll.is_some(),
				sticker_items: self.sticker_items.is_some_and(|a| a.0),
				stickers: self.stickers.is_some_and(|a| a.0),
				components: self.components.is_some_and(|a| a.0),
				components_v2: snapshot_flags.unwrap_or(self.flags) & (1 << 15) != 0,
			},
			reactions: Some(self.reactions.0),
			id: self.id,
			channel: self.channel_id,
			author,
			content: self.content,
			mention_roles: self.mention_roles,
			mention_everyone: self.mention_everyone,
			suppress_notifications: self.flags & (1 << 12) != 0,
			mentions: self
				.mentions
				.0
				.into_iter()
				.map(UserDto::into_model)
				.collect(),
			edited: self.edited_timestamp.is_some(),
			edited_at: self.edited_timestamp.map(|t| t.0),
			revision: 0,
			nonce: self.nonce.map(|n| match n {
				Nonce::Text(s) => s,
				Nonce::Number(n) => n.to_string(),
			}),
			kind: self.kind,
			reply_to,
			reply_deleted,
			forwarded,
			unsupported: !matches!(self.kind, 0 | 19 | 20 | 23) || unsupported_reference,
			attachments: self.attachments.0,
			embeds: embeds::bounded(self.embeds.0),
			embeds_suppressed: snapshot_flags.unwrap_or(self.flags) & 4 != 0,
		}
	}
}
#[derive(Deserialize)]
pub struct PatchDto {
	#[serde(default)]
	pub poll: Patch<extra_content::Object>,
	#[serde(default)]
	pub sticker_items: Patch<extra_content::Array>,
	#[serde(default)]
	pub stickers: Patch<extra_content::Array>,
	#[serde(default)]
	pub components: Patch<extra_content::Array>,
	#[serde(default)]
	pub reactions: Patch<reactions::ReactionList>,
	pub id: Id,
	pub channel_id: Id,
	#[serde(default)]
	pub content: Patch<String>,
	#[serde(default)]
	pub mentions: Patch<MentionList>,
	#[serde(default)]
	pub edited_timestamp: Patch<Timestamp>,
	#[serde(default)]
	pub embeds: Patch<EmbedList>,
	#[serde(default)]
	pub attachments: Patch<AttachmentList>,
	#[serde(default)]
	pub flags: Patch<u64>,
}
impl PatchDto {
	pub fn into_model(self) -> MessagePatch {
		MessagePatch {
			extra_content: model::ExtraContentPatch {
				poll: extra_content::object_patch(self.poll),
				sticker_items: extra_content::array_patch(self.sticker_items),
				stickers: extra_content::array_patch(self.stickers),
				components: extra_content::array_patch(self.components),
				components_v2: match &self.flags {
					Patch::Absent => Patch::Absent,
					Patch::Null => Patch::Null,
					Patch::Value(flags) => Patch::Value(flags & (1 << 15) != 0),
				},
			},
			reactions: match self.reactions {
				Patch::Absent => Patch::Absent,
				Patch::Null => Patch::Null,
				Patch::Value(list) => Patch::Value(list.0),
			},
			id: self.id,
			channel: self.channel_id,
			content: self.content,
			mentions: match self.mentions {
				Patch::Absent => Patch::Absent,
				Patch::Null => Patch::Null,
				Patch::Value(users) => {
					Patch::Value(users.0.into_iter().map(UserDto::into_model).collect())
				}
			},
			attachments: match self.attachments {
				Patch::Absent => Patch::Absent,
				Patch::Null => Patch::Null,
				Patch::Value(values) => Patch::Value(values.0),
			},
			embeds: match self.embeds {
				Patch::Absent => Patch::Absent,
				Patch::Null => Patch::Null,
				Patch::Value(values) => Patch::Value(embeds::bounded(values.0)),
			},
			embeds_suppressed: match self.flags {
				Patch::Absent => Patch::Absent,
				Patch::Null => Patch::Null,
				Patch::Value(flags) => Patch::Value(flags & 4 != 0),
			},
			edited: match self.edited_timestamp {
				Patch::Absent => Patch::Absent,
				Patch::Null => Patch::Null,
				Patch::Value(t) => Patch::Value(t.0),
			},
		}
	}
}
#[derive(Deserialize)]
pub struct Deleted {
	pub id: Id,
	pub channel_id: Id,
}
#[derive(Deserialize)]
pub struct BulkDeleted {
	pub ids: Vec<Id>,
	pub channel_id: Id,
}
#[derive(Deserialize)]
pub struct GatewayPacket {
	pub op: u8,
	#[serde(default)]
	pub s: Option<u64>,
	#[serde(default)]
	pub t: Option<String>,
	pub d: Box<RawValue>,
}
#[derive(Deserialize)]
pub struct Hello {
	pub heartbeat_interval: u64,
}
#[derive(Deserialize)]
pub struct GatewayLocation {
	pub url: String,
}
#[derive(Deserialize, Default)]
pub struct ErrorBody {
	#[serde(default)]
	pub code: Option<u64>,
	#[serde(default)]
	pub retry_after: Option<f64>,
	#[serde(default)]
	pub global: bool,
	#[serde(default)]
	pub captcha_key: Option<Box<RawValue>>,
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn webhook_authors_require_explicit_message_metadata() {
		let mut wire = serde_json::json!({"id":"100","channel_id":"2","author":{"id":"3","username":"Synthetic webhook","bot":true},"content":"webhook"});
		let read =
			|wire: &serde_json::Value| decode::<MessageDto>(&serde_json::to_vec(wire).unwrap());
		assert!(!read(&wire).unwrap().into_model().author.webhook);
		wire["webhook_id"] = serde_json::json!("3");
		let author = read(&wire).unwrap().into_model().author;
		assert!(author.webhook);
		assert_eq!(author.name, "Synthetic webhook");
		wire["webhook_id"] = serde_json::Value::Null;
		assert!(!read(&wire).unwrap().into_model().author.webhook);
		wire["webhook_id"] = serde_json::json!("invalid");
		assert!(read(&wire).is_err());
	}
	#[test]
	fn account_badges_follow_bot_and_application_provenance() {
		let mut wire = serde_json::json!({"id":"100","channel_id":"2","author":{"id":"3","username":"BOT WEBHOOK APP"},"content":"synthetic"});
		let label = |wire: &serde_json::Value| {
			decode::<MessageDto>(&serde_json::to_vec(wire).unwrap())
				.unwrap()
				.into_model()
				.author
				.account_label()
		};
		assert_eq!(label(&wire), None, "Names are not account metadata");
		wire["author"]["bot"] = serde_json::json!(true);
		assert_eq!(label(&wire), Some("BOT"));
		wire["webhook_id"] = serde_json::json!("3");
		assert_eq!(label(&wire), Some("WEBHOOK"));
		wire["application_id"] = serde_json::json!("4");
		assert_eq!(label(&wire), Some("APP"));
		wire["webhook_id"] = serde_json::Value::Null;
		assert_eq!(label(&wire), Some("APP"));
		wire["author"]["bot"] = serde_json::json!(false);
		assert_eq!(
			label(&wire),
			None,
			"Human rich-presence/game messages are not apps"
		);
		let user: UserDto =
			decode(br#"{"id":"3","username":"Synthetic member","bot":true}"#).unwrap();
		assert_eq!(user.into_model().account_label(), Some("BOT"));
	}
	#[test]
	fn notification_metadata_is_service_derived_and_role_mentions_are_bounded() {
		let wire = || {
			serde_json::json!({
				"id":"100", "channel_id":"2", "author":{"id":"3","username":"Synthetic"},
				"content":"@everyone <@&4>",
			})
		};
		let read =
			|value: &serde_json::Value| decode::<MessageDto>(&serde_json::to_vec(value).unwrap());
		let plain = read(&wire()).unwrap().into_model();
		assert!(plain.mention_roles.is_empty());
		assert!(!plain.mention_everyone && !plain.suppress_notifications);
		let mut value = wire();
		value["mention_roles"] = serde_json::json!(["4", "5"]);
		value["mention_everyone"] = true.into();
		value["flags"] = (4096 | 4 | 32768).into();
		let message = read(&value).unwrap().into_model();
		assert_eq!(message.mention_roles, vec![Id(4), Id(5)]);
		assert!(message.mention_everyone && message.suppress_notifications);
		assert!(message.embeds_suppressed && message.extra_content.components_v2);
		assert!(model::valid_mention_roles(&message.mention_roles));
		let bytes = message.bytes();
		let role_bytes = message.mention_roles.capacity() * size_of::<Id>();
		let mut without_roles = message;
		without_roles.mention_roles = Vec::new();
		assert_eq!(bytes - without_roles.bytes(), role_bytes);
		value["mention_roles"] = (1..=100)
			.map(|id| id.to_string())
			.collect::<Vec<_>>()
			.into();
		assert!(model::valid_mention_roles(
			&read(&value).unwrap().into_model().mention_roles
		));
		for roles in [
			serde_json::json!(null),
			serde_json::json!({}),
			serde_json::json!(["0"]),
			serde_json::json!([1]),
			serde_json::json!(["4", "4"]),
			serde_json::json!((1..=101).map(|id| id.to_string()).collect::<Vec<_>>()),
		] {
			value["mention_roles"] = roles;
			assert!(read(&value).is_err());
		}
		value["mention_roles"] = serde_json::json!([]);
		value["mention_everyone"] = serde_json::json!("true");
		assert!(read(&value).is_err());
	}
	#[test]
	fn forwarded_snapshot_uses_bounded_audio_body_and_outer_identity() {
		let mut wire = serde_json::json!({
			"id":"100", "channel_id":"2", "author":{"id":"3","username":"Forwarder"},
			"type":0, "content":"", "message_reference":{"type":1,"channel_id":"99","message_id":"50"},
			"message_snapshots":[{"message":{"type":0,"content":"Frozen text", "mention_everyone":true,
				"attachments":[{"id":"60","filename":"Samsung.mp3","size":3850000,"content_type":"audio/mpeg", "url":"https://cdn.discordapp.com/attachments/99/60/Samsung.mp3"}],
				"embeds":[{"type":"rich","title":"Snapshot embed"}]}}]
		});
		let read = |wire: &serde_json::Value| {
			decode::<MessageDto>(&serde_json::to_vec(wire).unwrap())
				.unwrap()
				.into_model()
		};
		let message = read(&wire);
		assert!(message.forwarded && !message.unsupported);
		assert_eq!(message.content, "Frozen text");
		assert_eq!(message.author.id, Id(3));
		assert_eq!(message.channel, Id(2));
		assert_eq!(message.id, Id(100));
		assert!(message.reply_to.is_none() && !message.mention_everyone);
		assert!(message.attachments[0].is_audio());
		assert_eq!(message.embeds[0].title.as_deref(), Some("Snapshot embed"));
		let snapshot = wire["message_snapshots"][0].clone();
		for snapshots in [
			serde_json::json!([]),
			serde_json::json!([snapshot.clone(), snapshot]),
		] {
			wire["message_snapshots"] = snapshots;
			let message = read(&wire);
			assert!(!message.forwarded && message.unsupported && message.attachments.is_empty());
		}
	}
	#[test]
	fn reply_references_require_same_channel_and_distinguish_deleted_from_unknown() {
		let wire = || {
			serde_json::json!({
				"id":"100", "channel_id":"2", "author":{"id":"3","username":"Synthetic"},
				"type":19, "message_reference":{"message_id":"50","channel_id":"2"}
			})
		};
		let read = |value: serde_json::Value| {
			decode::<MessageDto>(&serde_json::to_vec(&value).unwrap())
				.unwrap()
				.into_model()
		};
		let unknown = read(wire());
		assert_eq!(unknown.reply_to, Some(Id(50)));
		assert!(!unknown.reply_deleted && !unknown.unsupported);
		for kind in [19, 23] {
			let mut value = wire();
			value["type"] = kind.into();
			value["referenced_message"] = serde_json::Value::Null;
			let deleted = read(value);
			assert_eq!(deleted.reply_to, Some(Id(50)));
			assert!(deleted.reply_deleted && !deleted.unsupported);
		}
		let mut value = wire();
		// Nested bodies are deliberately discarded, never rendered or cached as another message.
		value["referenced_message"] = serde_json::json!({"content":"nested secret marker", "referenced_message":{"content":"nested"}});
		let resolved = read(value);
		assert!(!resolved.reply_deleted);
		assert!(resolved.content.is_empty());
		for invalid in 0..9 {
			let mut value = wire();
			value["referenced_message"] = serde_json::Value::Null;
			match invalid {
				0 => {
					value["message_reference"]["channel_id"] = "9".into();
				}
				1 => {
					value["message_reference"]
						.as_object_mut()
						.unwrap()
						.remove("channel_id");
				}
				2 => {
					value["message_reference"]["type"] = 1.into();
				}
				3 => {
					value["message_reference"]["type"] = 999.into();
				}
				4 => {
					value["message_reference"]["message_id"] = "0".into();
					assert!(decode::<MessageDto>(&serde_json::to_vec(&value).unwrap()).is_err());
					continue;
				}
				5 => {
					value["message_reference"]["message_id"] = "100".into();
				}
				6 => {
					value["message_reference"]["message_id"] = "101".into();
				}
				7 => {
					value["type"] = 21.into();
				}
				_ => {
					value["flags"] = 2.into();
				}
			}
			let message = read(value);
			assert!(message.reply_to.is_none() && !message.reply_deleted && message.unsupported);
		}
		for invalid in [
			serde_json::json!([]),
			serde_json::json!("invalid"),
			serde_json::Value::Object(
				(0..65)
					.map(|i| (i.to_string(), serde_json::Value::Null))
					.collect(),
			),
		] {
			let mut value = wire();
			value["referenced_message"] = invalid;
			assert!(decode::<MessageDto>(&serde_json::to_vec(&value).unwrap()).is_err());
		}
	}
	#[test]
	fn system_types_keep_original_content_and_describe_known_events() {
		let wire = |kind| {
			serde_json::json!({
				"id":"1", "channel_id":"2", "author":{"id":"3", "username":"Robin"},
				"type":kind, "content":"original text", "mentions":[{"id":"4", "username":"Casey"}]
			})
		};
		for kind in [
			1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 17, 18, 21, 22, 24, 25, 26, 27, 28,
			29, 30, 31, 32, 36, 37, 38, 39, 44, 46, 55, 58, 59, 60, 61, 62, 65, 67,
		] {
			let message = decode::<MessageDto>(&serde_json::to_vec(&wire(kind)).unwrap())
				.unwrap()
				.into_model();
			assert_eq!(message.kind, kind);
			assert_eq!(message.content, "original text");
			assert!(message.system_summary().is_some(), "type {kind}");
			if matches!(kind, 4 | 18) {
				// New channel/thread names are shown inline instead of as a second line.
				assert!(message.display_text().ends_with(": original text"));
			} else {
				assert!(message.display_text().ends_with("\noriginal text"));
			}
		}
		for kind in [0, 19, 20, 23, 222, 255] {
			let message = decode::<MessageDto>(&serde_json::to_vec(&wire(kind)).unwrap())
				.unwrap()
				.into_model();
			assert_eq!(message.kind, kind);
			assert!(message.system_summary().is_none());
			assert_eq!(message.display_text(), "original text");
			assert_eq!(message.unsupported, matches!(kind, 222 | 255));
		}
		let mut message = decode::<MessageDto>(&serde_json::to_vec(&wire(7)).unwrap())
			.unwrap()
			.into_model();
		message.content.clear();
		assert_eq!(message.display_text(), "Welcome, Robin! Joined the server.");
		message.kind = 1;
		assert_eq!(
			message.system_summary().unwrap(),
			"Robin added Casey to the conversation."
		);
		message.kind = 2;
		message.mentions[0] = message.author.clone();
		assert_eq!(
			message.system_summary().unwrap(),
			"Robin left the conversation."
		);
		message.mentions.clear();
		assert_eq!(
			message.system_summary().unwrap(),
			"Robin removed a member from the conversation."
		);
		message.author.name = "界".repeat(1000);
		assert!(message.system_summary().unwrap().chars().count() < 150);
		let mut invalid = wire(0);
		invalid["type"] = serde_json::json!(256);
		assert!(decode::<MessageDto>(&serde_json::to_vec(&invalid).unwrap()).is_err());
		invalid["type"] = serde_json::json!(-1);
		assert!(decode::<MessageDto>(&serde_json::to_vec(&invalid).unwrap()).is_err());
	}
	#[test]
	fn bounded_message_mentions_and_patch_presence() {
		let message=decode::<MessageDto>(br#"{"id":"1","channel_id":"2","author":{"id":"3","username":"author"},"content":"<@4>","mentions":[{"id":"4","username":"user","global_name":"Display name"}]}"#).unwrap().into_model();
		assert_eq!(message.mentions[0].id, Id(4));
		assert_eq!(message.mentions[0].name, "Display name");
		assert!(matches!(
			decode::<PatchDto>(br#"{"id":"1","channel_id":"2"}"#)
				.unwrap()
				.into_model()
				.mentions,
			Patch::Absent
		));
		assert!(matches!(
			decode::<PatchDto>(br#"{"id":"1","channel_id":"2","mentions":null}"#)
				.unwrap()
				.into_model()
				.mentions,
			Patch::Null
		));
		let large = serde_json::json!({"id":"1","channel_id":"2","mentions":(1..=101).map(|id|serde_json::json!({"id":id.to_string(),"username":"synthetic"})).collect::<Vec<_>>()});
		assert!(decode::<PatchDto>(&serde_json::to_vec(&large).unwrap()).is_err());
	}
	#[test]
	fn precision_patches_and_hostile_payloads() {
		let id: Id = decode(br#""18446744073709551615""#).unwrap();
		assert_eq!(id.0, u64::MAX);
		assert!(decode::<Id>(b"123").is_err());
		let patch: PatchDto = decode(br#"{"id":"1","channel_id":"2","content":null}"#).unwrap();
		assert_eq!(patch.content, Patch::Null);
		assert_eq!(patch.edited_timestamp, Patch::Absent);
		assert!(decode::<GatewayPacket>(b"{").is_err());
		assert!(decode::<GatewayPacket>(&vec![b' '; MAX_WIRE + 1]).is_err());
		assert!(decode::<serde_json::Value>(&[vec![b'['; 200], vec![b']'; 200]].concat()).is_err());
	}
}

#[derive(Deserialize)]
pub struct RoleDto {
	pub id: Id,
	pub permissions: String,
}
#[derive(Deserialize)]
pub struct Overwrite {
	pub id: Id,
	pub allow: String,
	pub deny: String,
}
fn member_list_id(everyone: u128, overwrites: &[Overwrite]) -> Option<String> {
	if overwrites.len() > model::permissions::MAX_OVERWRITES {
		return None;
	}
	let overwrites = overwrites
		.iter()
		.map(|o| {
			Some(model::permissions::Overwrite {
				id: o.id,
				kind: 0, // List identity uses IDs and view bits, regardless of overwrite kind.
				allow: o.allow.parse::<u128>().ok()?,
				deny: o.deny.parse::<u128>().ok()?,
			})
		})
		.collect::<Option<Vec<_>>>()?;
	model::permissions::member_list_id(everyone, &overwrites)
}
#[derive(Deserialize)]
pub struct MemberDto {
	#[serde(default, deserialize_with = "permissions::member_roles")]
	pub roles: Vec<Id>,
	pub user: UserDto,
	#[serde(default)]
	pub nick: Option<String>,
	#[serde(default)]
	pub presence: Option<PresenceDto>,
}
#[derive(Deserialize)]
pub struct PresenceDto {
	pub status: String,
	#[serde(default)]
	pub activities: presence::Activities,
}
impl PresenceDto {
	/// The same bounded custom-status normalization is used for snapshots and updates.
	pub fn custom_status(&self) -> Option<String> {
		self.activities.0.clone()
	}
}
#[derive(Deserialize)]
#[serde(untagged)]
pub enum MemberItem {
	Member { member: Box<MemberDto> },
	Group { group: MemberGroup },
}
#[derive(Deserialize)]
pub struct MemberGroup {
	pub id: String,
}
impl MemberItem {
	pub fn into_model(self) -> Option<model::Member> {
		match self {
			Self::Group { .. } => None,
			Self::Member { member: mut m } => Some(model::Member {
				roles: m.roles,
				user: m.user.into_model(),
				nick: m.nick.map(|n| n.chars().take(128).collect()),
				custom_status: m
					.presence
					.as_ref()
					.filter(|p| p.status != "offline")
					.and_then(PresenceDto::custom_status),
				activities: m
					.presence
					.as_mut()
					.filter(|p| p.status != "offline")
					.map_or_else(Vec::new, |p| std::mem::take(&mut p.activities.1)),
				status: m.presence.and_then(|p| match p.status.as_str() {
					"online" | "idle" | "dnd" | "offline" => Some(p.status),
					_ => None,
				}),
			}),
		}
	}
}
#[derive(Deserialize)]
#[serde(tag = "op")]
pub enum MemberOp {
	#[serde(rename = "SYNC")]
	Sync {
		range: [usize; 2],
		items: Vec<MemberItem>,
	},
	#[serde(rename = "INVALIDATE")]
	Invalidate { range: [usize; 2] },
	#[serde(rename = "UPDATE")]
	Update { index: usize, item: MemberItem },
	#[serde(rename = "INSERT")]
	Insert { index: usize, item: MemberItem },
	#[serde(rename = "DELETE")]
	Delete { index: usize },
}
#[derive(Deserialize)]
pub struct MemberUpdate {
	pub guild_id: Id,
	pub id: String,
	pub member_count: u64,
	pub ops: Vec<MemberOp>,
}

#[cfg(test)]
mod member_tests {
	use super::*;
	#[test]
	fn member_roles_are_retained_sorted_and_bounded_before_list_admission() {
		let member: MemberItem =
			decode(br#"{"member":{"user":{"id":"5","username":"Synthetic"},"roles":["12","11"]}}"#)
				.unwrap();
		let mut member = member.into_model().unwrap();
		assert_eq!(member.roles, vec![Id(11), Id(12)]);
		let bytes = member.bytes();
		member.roles.reserve(100);
		assert!(member.bytes() >= bytes + 100 * size_of::<Id>());
		for roles in [
			serde_json::json!(["0"]),
			serde_json::json!(["11", "11"]),
			serde_json::json!((1..=513).map(|id| id.to_string()).collect::<Vec<_>>()),
		] {
			let value = serde_json::json!({"member":{"user":{"id":"5","username":"Synthetic"},"roles":roles}});
			assert!(decode::<MemberItem>(&serde_json::to_vec(&value).unwrap()).is_err());
		}
	}

	#[test]
	fn avatars_recipients_and_permission_scoped_list_ids() {
		let mut nested: Ready = decode(br#"{"user":{"id":"1","username":"Synthetic"},"session_id":"synthetic","resume_gateway_url":"wss://gateway.discord.gg","guilds":[{"id":"2","properties":{"name":"Nested","icon":"0123456789abcdef0123456789abcdef"}},{"id":"3","name":"Flat fallback","icon":"0123456789abcdef0123456789abcdef","properties":{"icon":null}}]}"#).unwrap();
		let (nested_guilds, _) = nested.navigation().unwrap();
		assert_eq!(nested_guilds[0].name, "Nested");
		assert!(nested_guilds[0].icon_key().is_some());
		assert_eq!(nested_guilds[1].name, "Flat fallback");
		assert!(nested_guilds[1].icon.is_none());
		let patch = decode::<GuildPatchDto>(br#"{"id":"2","icon":null}"#)
			.unwrap()
			.into_model();
		assert_eq!(patch.name, Patch::Absent);
		assert_eq!(patch.icon, Patch::Null);
		let patch = decode::<GuildPatchDto>(br#"{"id":"2","name":"Renamed"}"#)
			.unwrap()
			.into_model();
		assert_eq!(patch.name, Patch::Value("Renamed".into()));
		assert_eq!(patch.icon, Patch::Absent);
		let mut ready: Ready = decode(br#"{"user":{"id":"1","username":"Synthetic"},"session_id":"synthetic","resume_gateway_url":"wss://gateway.discord.gg","guilds":[{"id":"2","name":"Server","icon":"a_0123456789abcdef0123456789abcdef"},{"id":"3","name":"Missing icon","icon":"../../invalid"}]}"#).unwrap();
		let (guilds, _) = ready.navigation().unwrap();
		assert_eq!(
			guilds[0].icon_key().as_deref(),
			Some("guild-2-a_0123456789abcdef0123456789abcdef")
		);
		assert!(guilds[1].icon_key().is_none());
		let user: UserDto = decode(
			br#"{"id":"4194304","username":"Name","avatar":"../../invalid","discriminator":"0"}"#,
		)
		.unwrap();
		let user = user.into_model();
		assert!(user.avatar.is_none());
		assert_eq!(user.avatar_key(), "default-1");
		assert_eq!(
			user.avatar_url(),
			"https://cdn.discordapp.com/embed/avatars/1.png"
		);
		let member: MemberItem = decode(br#"{"member":{"user":{"id":"5","username":"Presence"},"presence":{"status":"idle","activities":[{"type":0,"name":"Game","state":"ignored"},{"type":4,"name":"Custom Status","state":" semifluent in computerspeak ","emoji":{"name":"\ud83c\udf19","id":null}}]}}}"#).unwrap();
		let member = member.into_model().unwrap();
		assert_eq!(member.status.as_deref(), Some("idle"));
		assert_eq!(
			member.custom_status.as_deref(),
			Some("🌙 semifluent in computerspeak")
		);
		let custom_emoji: MemberItem = decode(br#"{"member":{"user":{"id":"5","username":"Presence"},"presence":{"status":"online","activities":[{"type":4,"emoji":{"name":"serein_wave","id":"9001"}}]}}}"#).unwrap();
		assert!(custom_emoji.into_model().unwrap().custom_status.is_none());
		let long: MemberItem = decode(format!(r#"{{"member":{{"user":{{"id":"5","username":"P"}},"presence":{{"status":"dnd","activities":[{{"type":4,"state":"{}"}}]}}}}}}"#, "x".repeat(400)).as_bytes()).unwrap();
		assert_eq!(
			long.into_model()
				.unwrap()
				.custom_status
				.unwrap()
				.chars()
				.count(),
			128
		);
		assert_eq!(member_list_id(1024, &[]), Some("everyone".into()));
		assert_eq!(member_list_id(0, &[]), Some("0".into()));
		assert_eq!(
			member_list_id(
				1024 | (1 << 100),
				&[Overwrite {
					id: Id(5),
					allow: (1u128 << 100).to_string(),
					deny: "0".into(),
				}]
			),
			Some("everyone".into())
		);
		let deny = Overwrite {
			id: Id(5),
			allow: "0".into(),
			deny: "1024".into(),
		};
		assert_ne!(member_list_id(1024, &[deny]), Some("everyone".into()));
		assert!(
			member_list_id(
				1024,
				&[Overwrite {
					id: Id(5),
					allow: "0".into(),
					deny: "invalid".into()
				}]
			)
			.is_none()
		);
		let channel:ChannelDto=decode(br#"{"id":"1","type":1,"recipients":[{"id":"2","username":"Person","avatar":"a_0123456789abcdef0123456789abcdef","discriminator":"1337"}]}"#).unwrap();
		let channel = channel.into_model();
		assert_eq!(channel.recipients.len(), 1);
		assert_eq!(
			channel.recipients[0].avatar_key(),
			"2-a_0123456789abcdef0123456789abcdef"
		);
	}
}

#[derive(Deserialize)]
pub struct MemberIdentity {
	pub user: UserIdentity,
}
#[derive(Deserialize)]
pub struct UserIdentity {
	pub id: Id,
}

#[derive(Deserialize)]
pub struct RecipientAdded {
	pub channel_id: Id,
	pub user: UserDto,
}
#[derive(Deserialize)]
pub struct RecipientRemoved {
	pub channel_id: Id,
	pub user: UserIdentity,
}

/// Normal-user DM call dispatches; public developer guild voice docs do not cover CALL_*.
#[derive(Deserialize)]
pub struct CallDto {
	pub channel_id: Id,
	#[serde(default)]
	pub ringing: Option<Vec<Id>>,
	#[serde(default)]
	pub voice_states: Option<Vec<VoiceStateDto>>,
	#[serde(default)]
	pub unavailable: bool,
}
#[derive(Deserialize)]
pub struct VoiceStateDto {
	#[serde(default)]
	pub guild_id: Option<Id>,
	pub channel_id: Option<Id>,
	pub user_id: Id,
	#[serde(default)]
	pub session_id: Option<String>,
	#[serde(default)]
	pub self_mute: bool,
	#[serde(default)]
	pub self_deaf: bool,
	#[serde(default)]
	pub mute: bool,
	#[serde(default)]
	pub deaf: bool,
	#[serde(default)]
	pub suppress: bool,
	#[serde(default)]
	pub self_video: bool,
	#[serde(default)]
	pub self_stream: bool,
	#[serde(default)]
	pub member: Option<VoiceMemberDto>,
}
#[derive(Deserialize)]
pub struct VoiceServerDto {
	#[serde(default)]
	pub guild_id: Option<Id>,
	#[serde(default)]
	pub channel_id: Option<Id>,
	pub token: String,
	pub endpoint: Option<String>,
}

/// READY_SUPPLEMENTAL member identities can reference the READY users array.
#[derive(Deserialize)]
pub struct VoiceMemberDto {
	#[serde(default)]
	pub user: Option<UserDto>,
	#[serde(default)]
	pub user_id: Option<Id>,
	#[serde(default)]
	pub nick: Option<String>,
}
#[derive(Deserialize)]
pub struct ReadySupplemental {
	#[serde(default)]
	pub presences: Option<Box<RawValue>>,
	#[serde(default)]
	pub merged_presences: Option<presence::MergedPresences>,
	#[serde(default)]
	pub guilds: Vec<GuildDto>,
	#[serde(default)]
	pub merged_members: Vec<Vec<VoiceMemberDto>>,
}
#[derive(Deserialize)]
pub struct PassiveVoiceUpdate {
	#[serde(default, deserialize_with = "read_state::account_entries")]
	pub updated_channels: Vec<read_state::LatestChannel>,
	#[serde(default)]
	pub guild_id: Option<Id>,
	#[serde(default)]
	pub updated_voice_states: Vec<VoiceStateDto>,
	#[serde(default)]
	pub removed_voice_states: Vec<Id>,
	#[serde(default)]
	pub updated_members: Vec<VoiceMemberDto>,
}
