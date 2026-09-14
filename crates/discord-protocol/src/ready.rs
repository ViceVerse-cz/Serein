//! Split READY once, borrowing the large arrays for their independent bounded projections.
use crate::{
	ChannelDto, DecodeError, MAX_GATEWAY_WIRE, MAX_WIRE, Ready, UserDto, notifications,
	permissions, presence, read_state,
};
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Deserialize)]
pub struct Envelope<'a> {
	#[serde(default)]
	relationships: Option<crate::relationships::Snapshot>,
	pub user: UserDto,
	pub session_id: String,
	pub resume_gateway_url: String,
	#[serde(default)]
	users: Vec<UserDto>,
	#[serde(default)]
	read_state: Option<read_state::Snapshot>,
	#[serde(default)]
	user_guild_settings: Option<notifications::Snapshot>,
	#[serde(default)]
	sessions: Option<notifications::Sessions>,
	#[serde(default)]
	private_channels: Vec<ChannelDto>,
	#[serde(default = "empty_array", borrow)]
	guilds: &'a RawValue,
	#[serde(default, borrow)]
	merged_members: Option<&'a RawValue>,
	#[serde(default)]
	presences: Option<Box<RawValue>>,
	#[serde(default)]
	merged_presences: Option<presence::MergedPresences>,
}
fn empty_array() -> &'static RawValue {
	serde_json::from_str("[]").expect("constant JSON array")
}
pub fn decode(bytes: &[u8]) -> Result<Envelope<'_>, DecodeError> {
	if bytes.len() > MAX_GATEWAY_WIRE {
		return Err(DecodeError);
	}
	serde_json::from_slice(bytes).map_err(|_| DecodeError)
}
impl Envelope<'_> {
	pub fn permissions(&self) -> Result<model::permissions::Snapshot, DecodeError> {
		permissions::ready_fields(
			self.guilds.get().as_bytes(),
			self.merged_members.map(|m| m.get().as_bytes()),
			self.user.id,
		)
	}
	pub fn navigation(self) -> Result<Ready, DecodeError> {
		Ok(Ready {
			relationships: self.relationships,
			user: self.user,
			session_id: self.session_id,
			resume_gateway_url: self.resume_gateway_url,
			users: self.users,
			read_state: self.read_state,
			user_guild_settings: self.user_guild_settings,
			sessions: self.sessions,
			private_channels: self.private_channels,
			guilds: crate::decode_gateway(self.guilds.get().as_bytes())?,
			presences: self.presences,
			merged_presences: self.merged_presences,
		})
	}
}

/// PASSIVE_UPDATE_V2 has independent voice, permission and read-state projections.
#[derive(Deserialize)]
pub struct PassiveEnvelope<'a> {
	#[serde(default)]
	guild_id: Option<model::Id>,
	#[serde(default)]
	updated_voice_states: Vec<crate::VoiceStateDto>,
	#[serde(default)]
	removed_voice_states: Vec<model::Id>,
	#[serde(default, deserialize_with = "read_state::entries")]
	updated_channels: Vec<read_state::LatestChannel>,
	#[serde(default = "empty_array", borrow)]
	updated_members: &'a RawValue,
}
pub fn passive(bytes: &[u8]) -> Result<PassiveEnvelope<'_>, DecodeError> {
	if bytes.len() > MAX_WIRE {
		return Err(DecodeError);
	}
	serde_json::from_slice(bytes).map_err(|_| DecodeError)
}
impl PassiveEnvelope<'_> {
	pub fn permissions(
		&self,
		user: model::Id,
	) -> Result<Option<permissions::MemberUpdate>, DecodeError> {
		permissions::passive_fields(self.guild_id, self.updated_members.get().as_bytes(), user)
	}
	pub fn voice(self) -> Result<crate::PassiveVoiceUpdate, DecodeError> {
		Ok(crate::PassiveVoiceUpdate {
			guild_id: self.guild_id,
			updated_voice_states: self.updated_voice_states,
			removed_voice_states: self.removed_voice_states,
			updated_members: crate::decode(self.updated_members.get().as_bytes())?,
			updated_channels: self.updated_channels,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::Id;

	#[test]
	fn borrowed_fields_preserve_permission_and_navigation_projections() {
		let bytes = br#"{"user":{"id":"9","username":"Synthetic"},"session_id":"synthetic","resume_gateway_url":"wss://gateway.discord.gg/","guilds":[{"id":"1","owner_id":"9","name":"Synthetic","roles":[],"channels":[{"id":"2","type":0,"name":"general","permission_overwrites":[]}]}]}"#;
		let envelope = decode(bytes).unwrap();
		assert_eq!(
			envelope.permissions().unwrap(),
			permissions::ready(bytes, Id(9)).unwrap()
		);
		let mut expected: Ready = crate::decode(bytes).unwrap();
		assert!(
			envelope.navigation().unwrap().navigation().unwrap() == expected.navigation().unwrap()
		);
		let bytes = br#"{"guild_id":"1","updated_members":[{"user":{"id":"9","username":"Synthetic"},"roles":[]}],"updated_channels":[{"id":"2","last_message_id":"3"}]}"#;
		let envelope = passive(bytes).unwrap();
		assert_eq!(
			envelope.permissions(Id(9)).unwrap(),
			permissions::passive(bytes, Id(9)).unwrap()
		);
		let voice = envelope.voice().unwrap();
		assert_eq!(voice.updated_members.len(), 1);
		assert_eq!(voice.updated_channels[0].id, Id(2));
	}
}
