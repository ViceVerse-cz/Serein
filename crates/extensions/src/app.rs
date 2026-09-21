use serde::{Deserialize, Serialize};

pub const MAX_APP_SNAPSHOT_BYTES: usize = 64 * 1024;
pub const MAX_APP_CHANNELS: usize = 100;
pub const MAX_APP_MESSAGES: usize = 50;
pub const MAX_APP_MEMBERS: usize = 100;
pub const MAX_APP_PRESENCES: usize = 100;
pub const MAX_VOICE_PARTICIPANTS: usize = 64;
pub const MAX_HOST_EFFECTS: usize = 1;
pub const MAX_HOST_EFFECT_BYTES: usize = 8 * 1024;

/// Each group is present only when granted and available; partial lists say so explicitly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSnapshot {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub context: Option<AppContextSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub channels: Option<ChannelDirectorySnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub timeline: Option<TimelineSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub members: Option<MembersSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub presence: Option<PresenceSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub voice: Option<VoiceSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub read_state: Option<ReadSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub settings: Option<LocalSettingsSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppContextSnapshot {
	pub connected: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user: Option<UserSnapshot>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel: Option<ChannelSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSnapshot {
	pub id: String,
	pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSnapshot {
	pub id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub guild_id: Option<String>,
	pub name: String,
	pub kind: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelDirectorySnapshot {
	pub items: Vec<ChannelSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineSnapshot {
	pub channel_id: String,
	pub messages: Vec<MessageSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSnapshot {
	pub id: String,
	pub author: UserSnapshot,
	pub content: String,
	pub attachment_count: u16,
	pub edited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembersSnapshot {
	pub channel_id: String,
	pub items: Vec<UserSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceSnapshot {
	pub items: Vec<PresenceEntry>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceEntry {
	pub user_id: String,
	pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel_id: Option<String>,
	pub phase: String,
	pub muted: bool,
	pub deafened: bool,
	pub camera: bool,
	pub streaming: bool,
	pub participants: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel_id: Option<String>,
	pub unread: Option<bool>,
	pub mentions: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSettingsSnapshot {
	pub zoom_percent: u16,
	pub sidebar_width: u16,
	pub show_members: bool,
	pub animate_gifs: bool,
	pub hide_media_links: bool,
}

/// Omitted preferences keep their current values when the user approves the proposal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalSettingsPatch {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub zoom_percent: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sidebar_width: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub show_members: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub animate_gifs: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub hide_media_links: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEventKind {
	Ready,
	Navigation,
	Connection,
	Context,
	Voice,
	Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppView {
	Friends,
	Search,
	Pins,
	Members,
	Threads,
	Settings,
	Account,
	ProfileSettings,
	Appearance,
	MessagingPermissions,
	Notifications,
	Activity,
	Keybinds,
	Storage,
	Updates,
	Extensions,
	Themes,
	VoiceSettings,
}

/// One local host proposal per response, applied only after explicit user confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostEffect {
	Navigate {
		channel_id: String,
	},
	Home,
	OpenView {
		view: AppView,
	},
	OpenProfile {
		user_id: String,
	},
	JumpToMessage {
		channel_id: String,
		message_id: String,
	},
	Search {
		query: String,
	},
	Notice {
		text: String,
	},
	CopyText {
		text: String,
	},
	SetVoice {
		muted: bool,
		deafened: bool,
	},
	LeaveVoice,
	SetLocalSettings {
		settings: LocalSettingsPatch,
	},
}
use crate::{Capability, Error, MAX_EVENT_CONTENT_BYTES, Manifest};
use std::{collections::BTreeSet, io};

fn grant(manifest: &Manifest, capability: Capability) -> Result<(), Error> {
	manifest
		.capabilities
		.contains(&capability)
		.then_some(())
		.ok_or(Error::Capability)
}

pub(crate) fn entity_id(id: &str) -> Result<(), Error> {
	if id.len() > 20
		|| !id.bytes().all(|byte| byte.is_ascii_digit())
		|| !id.parse::<u64>().is_ok_and(|value| value != 0)
	{
		return Err(Error::Invalid);
	}
	Ok(())
}

fn label(value: &str, limit: usize) -> Result<(), Error> {
	if value.len() > limit {
		return Err(Error::Limit);
	}
	if value.is_empty() || value.chars().any(char::is_control) {
		return Err(Error::Invalid);
	}
	Ok(())
}

fn user(user: &UserSnapshot) -> Result<(), Error> {
	entity_id(&user.id)?;
	label(&user.name, 256)
}

fn channel(channel: &ChannelSnapshot) -> Result<(), Error> {
	entity_id(&channel.id)?;
	if let Some(guild) = &channel.guild_id {
		entity_id(guild)?;
	}
	label(&channel.name, 256)
}

fn ids<'a>(items: impl Iterator<Item = &'a str>, limit: usize) -> Result<(), Error> {
	let mut seen = BTreeSet::new();
	for id in items {
		if seen.len() == limit {
			return Err(Error::Limit);
		}
		entity_id(id)?;
		if !seen.insert(id) {
			return Err(Error::Invalid);
		}
	}
	Ok(())
}

/// Counts serialized bytes without allocating an oversized JSON buffer.
fn bounded_bytes(value: &(impl Serialize + ?Sized), limit: usize) -> Result<usize, Error> {
	struct Counter {
		used: usize,
		limit: usize,
	}
	impl io::Write for Counter {
		fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
			if bytes.len() > self.limit - self.used {
				return Err(io::ErrorKind::WriteZero.into());
			}
			self.used += bytes.len();
			Ok(bytes.len())
		}
		fn flush(&mut self) -> io::Result<()> {
			Ok(())
		}
	}
	let mut counter = Counter { used: 0, limit };
	serde_json::to_writer(&mut counter, value).map_err(|error| {
		if error.io_error_kind() == Some(io::ErrorKind::WriteZero) {
			Error::Limit
		} else {
			Error::Invalid
		}
	})?;
	Ok(counter.used)
}

impl AppSnapshot {
	/// Exact serialized size, rejecting snapshots above the 64 KiB wire budget.
	pub fn bytes(&self) -> Result<usize, Error> {
		bounded_bytes(self, MAX_APP_SNAPSHOT_BYTES)
	}

	pub fn validate(&self, manifest: &Manifest) -> Result<(), Error> {
		for (present, capability) in [
			(self.context.is_some(), Capability::AppContext),
			(self.channels.is_some(), Capability::ChannelDirectory),
			(self.timeline.is_some(), Capability::Timeline),
			(self.members.is_some(), Capability::Members),
			(self.presence.is_some(), Capability::Presence),
			(self.voice.is_some(), Capability::VoiceState),
			(self.read_state.is_some(), Capability::ReadState),
			(self.settings.is_some(), Capability::LocalSettings),
		] {
			if present {
				grant(manifest, capability)?;
			}
		}
		if let Some(context) = &self.context {
			if let Some(value) = &context.user {
				user(value)?;
			}
			if let Some(value) = &context.channel {
				channel(value)?;
			}
		}
		if let Some(directory) = &self.channels {
			ids(
				directory.items.iter().map(|item| item.id.as_str()),
				MAX_APP_CHANNELS,
			)?;
			for item in &directory.items {
				channel(item)?;
			}
		}
		if let Some(timeline) = &self.timeline {
			entity_id(&timeline.channel_id)?;
			ids(
				timeline.messages.iter().map(|item| item.id.as_str()),
				MAX_APP_MESSAGES,
			)?;
			for message in &timeline.messages {
				user(&message.author)?;
				if message.content.len() > MAX_EVENT_CONTENT_BYTES {
					return Err(Error::Limit);
				}
			}
		}
		if let Some(members) = &self.members {
			entity_id(&members.channel_id)?;
			ids(
				members.items.iter().map(|item| item.id.as_str()),
				MAX_APP_MEMBERS,
			)?;
			for member in &members.items {
				user(member)?;
			}
		}
		if let Some(presence) = &self.presence {
			ids(
				presence.items.iter().map(|item| item.user_id.as_str()),
				MAX_APP_PRESENCES,
			)?;
			for item in &presence.items {
				label(&item.status, 32)?;
			}
		}
		if let Some(voice) = &self.voice {
			if let Some(channel) = &voice.channel_id {
				entity_id(channel)?;
			}
			label(&voice.phase, 64)?;
			ids(
				voice.participants.iter().map(String::as_str),
				MAX_VOICE_PARTICIPANTS,
			)?;
		}
		if let Some(read_state) = &self.read_state
			&& let Some(channel) = &read_state.channel_id
		{
			entity_id(channel)?;
		}
		if let Some(settings) = &self.settings {
			settings.validate()?;
		}
		self.bytes().map(|_| ())
	}
}

impl LocalSettingsSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		if !(80..=150).contains(&self.zoom_percent) || !(190..=360).contains(&self.sidebar_width) {
			return Err(Error::Invalid);
		}
		Ok(())
	}
}

impl LocalSettingsPatch {
	pub fn validate(&self) -> Result<(), Error> {
		if self == &Self::default()
			|| self
				.zoom_percent
				.is_some_and(|value| !(80..=150).contains(&value))
			|| self
				.sidebar_width
				.is_some_and(|value| !(190..=360).contains(&value))
		{
			return Err(Error::Invalid);
		}
		Ok(())
	}
}

impl HostEffect {
	pub fn required_capability(&self) -> Capability {
		match self {
			Self::Navigate { .. }
			| Self::Home
			| Self::OpenView { .. }
			| Self::OpenProfile { .. }
			| Self::JumpToMessage { .. }
			| Self::Search { .. } => Capability::Navigation,
			Self::Notice { .. } => Capability::LocalNotices,
			Self::CopyText { .. } => Capability::ClipboardWrite,
			Self::SetVoice { .. } | Self::LeaveVoice => Capability::VoiceControl,
			Self::SetLocalSettings { .. } => Capability::LocalSettings,
		}
	}

	pub fn validate(&self, manifest: &Manifest) -> Result<(), Error> {
		grant(manifest, self.required_capability())?;
		match self {
			Self::Navigate { channel_id } => entity_id(channel_id)?,
			Self::OpenProfile { user_id } => entity_id(user_id)?,
			Self::JumpToMessage {
				channel_id,
				message_id,
			} => {
				entity_id(channel_id)?;
				entity_id(message_id)?;
			}
			Self::Search { query } => label(query, 256)?,
			Self::Notice { text } => {
				if text.len() > 1024 {
					return Err(Error::Limit);
				}
				if text.trim().is_empty() {
					return Err(Error::Invalid);
				}
			}
			Self::CopyText { text } => {
				if text.len() > 4096 {
					return Err(Error::Limit);
				}
			}
			Self::SetLocalSettings { settings } => settings.validate()?,
			Self::Home | Self::OpenView { .. } | Self::SetVoice { .. } | Self::LeaveVoice => {}
		}
		bounded_bytes(self, MAX_HOST_EFFECT_BYTES).map(|_| ())
	}
}

pub(crate) fn validate_effects(effects: &[HostEffect], manifest: &Manifest) -> Result<(), Error> {
	if effects.len() > MAX_HOST_EFFECTS {
		return Err(Error::Limit);
	}
	for effect in effects {
		effect.validate(manifest)?;
	}
	bounded_bytes(effects, MAX_HOST_EFFECT_BYTES).map(|_| ())
}
