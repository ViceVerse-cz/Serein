//! Split READY once, borrowing the large arrays for their independent bounded projections.
use crate::{
	ChannelDto, DecodeError, MAX_GATEWAY_WIRE, MAX_WIRE, Ready, UserDto, permissions, presence,
	read_state,
};
use model::account::Warnings;
use serde::{
	Deserialize, Deserializer,
	de::{SeqAccess, Visitor},
};
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
	#[serde(default, borrow)]
	read_state: Option<&'a RawValue>,
	#[serde(default, borrow)]
	user_guild_settings: Option<&'a RawValue>,
	#[serde(default, borrow)]
	sessions: Option<&'a RawValue>,
	#[serde(default)]
	private_channels: Vec<ChannelDto>,
	#[serde(default = "empty_array", borrow)]
	guilds: &'a RawValue,
	#[serde(default, borrow)]
	merged_members: Option<&'a RawValue>,
	#[serde(default, borrow)]
	presences: Option<&'a RawValue>,
	#[serde(default, borrow)]
	merged_presences: Option<&'a RawValue>,
}
pub(crate) fn empty_array() -> &'static RawValue {
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
	pub fn navigation(self) -> Result<(Ready, Warnings), DecodeError> {
		let mut warnings = Warnings::default();
		let guilds: Guilds = crate::decode_gateway(self.guilds.get().as_bytes())?;
		warnings.emojis = guilds.1;
		warnings.stickers = guilds.2;
		let read_state = optional(self.read_state, &mut warnings.read_state);
		let user_guild_settings = optional(self.user_guild_settings, &mut warnings.notifications)
			.filter(|settings: &crate::notifications::Snapshot| {
				let entries = match settings {
					crate::notifications::Snapshot::Versioned { entries, .. }
					| crate::notifications::Snapshot::Legacy(entries) => entries,
				};
				let count = entries.len()
					+ entries
						.iter()
						.map(|entry| {
							entry
								.channel_overrides
								.as_ref()
								.map_or(0, |overrides| overrides.0.len())
						})
						.sum::<usize>();
				let valid = count <= model::account::MAX_ENTRIES;
				warnings.notifications |= !valid;
				valid
			});
		let sessions = optional(self.sessions, &mut warnings.sessions);
		let presences = checked_presences(self.presences, &mut warnings.presence);
		let merged_presences =
			checked_merged_presences(self.merged_presences, &mut warnings.presence);
		Ok((
			Ready {
				relationships: self.relationships,
				user: self.user,
				session_id: self.session_id,
				resume_gateway_url: self.resume_gateway_url,
				users: self.users,
				read_state,
				user_guild_settings,
				sessions,
				private_channels: self.private_channels,
				guilds: guilds.0,
				presences,
				merged_presences,
			},
			warnings,
		))
	}
}

/// Supplemental optional metadata cannot invalidate an already accepted account snapshot.
pub fn supplemental(bytes: &[u8]) -> Result<(crate::ReadySupplemental, Warnings), DecodeError> {
	#[derive(Deserialize)]
	struct Supplemental<'a> {
		#[serde(default = "empty_array", borrow)]
		guilds: &'a RawValue,
		#[serde(default)]
		merged_members: Vec<Vec<crate::VoiceMemberDto>>,
		#[serde(default, borrow)]
		presences: Option<&'a RawValue>,
		#[serde(default, borrow)]
		merged_presences: Option<&'a RawValue>,
	}
	if bytes.len() > MAX_GATEWAY_WIRE {
		return Err(DecodeError);
	}
	let raw: Supplemental<'_> = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	let guilds: Guilds = crate::decode_gateway(raw.guilds.get().as_bytes())?;
	let mut warnings = Warnings {
		emojis: guilds.1,
		stickers: guilds.2,
		..Warnings::default()
	};
	let presences = checked_presences(raw.presences, &mut warnings.presence);
	let merged_presences = checked_merged_presences(raw.merged_presences, &mut warnings.presence);
	Ok((
		crate::ReadySupplemental {
			guilds: guilds.0,
			merged_members: raw.merged_members,
			presences,
			merged_presences,
		},
		warnings,
	))
}

fn checked_merged_presences(
	raw: Option<&RawValue>,
	warning: &mut bool,
) -> Option<presence::MergedPresences> {
	#[derive(Deserialize)]
	struct Merged<'a> {
		#[serde(default, borrow)]
		friends: Option<&'a RawValue>,
	}
	optional::<Merged<'_>>(raw, warning).map(|merged| presence::MergedPresences {
		friends: checked_presences(merged.friends, warning),
	})
}

fn optional<'a, T: Deserialize<'a>>(raw: Option<&'a RawValue>, warning: &mut bool) -> Option<T> {
	raw.and_then(|raw| match serde_json::from_str(raw.get()) {
		Ok(value) => Some(value),
		Err(_) => {
			*warning = true;
			None
		}
	})
}

fn checked_presences(raw: Option<&RawValue>, warning: &mut bool) -> Option<Box<RawValue>> {
	raw.and_then(|raw| {
		let valid = serde_json::from_str::<presence::Friends<'_>>(raw.get()).is_ok_and(|friends| {
			friends
				.0
				.iter()
				.all(|friend| presence::decode(friend.get().as_bytes()).is_ok())
		});
		if valid {
			Some(raw.to_owned())
		} else {
			*warning = true;
			None
		}
	})
}

// Only READY tolerates unavailable optional catalogs. Ordinary guild responses remain strict.
#[derive(Deserialize)]
struct Guild<'a> {
	#[serde(default, borrow)]
	stickers: Option<&'a RawValue>,
	id: model::Id,
	#[serde(default, borrow)]
	emojis: Option<&'a RawValue>,
	#[serde(default)]
	properties: Option<crate::GuildProperties>,
	#[serde(default)]
	icon: Option<String>,
	#[serde(default)]
	name: String,
	#[serde(default, deserialize_with = "crate::threads::list")]
	channels: Vec<ChannelDto>,
	#[serde(default, deserialize_with = "crate::threads::list")]
	threads: Vec<ChannelDto>,
	#[serde(default)]
	roles: Vec<crate::RoleDto>,
	#[serde(default)]
	voice_states: Vec<crate::VoiceStateDto>,
	#[serde(default)]
	members: Vec<crate::VoiceMemberDto>,
}
struct Guilds(Vec<crate::GuildDto>, bool, bool);
impl<'de> Deserialize<'de> for Guilds {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct GuildVisitor;
		impl<'de> Visitor<'de> for GuildVisitor {
			type Value = Guilds;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("bounded READY guilds")
			}
			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Guilds, A::Error> {
				let mut guilds = Vec::new();
				let mut unavailable = false;
				let mut stickers_unavailable = false;
				let mut entries = 0usize;
				while let Some(guild) = seq.next_element::<Guild<'de>>()? {
					entries += 1 + guild.channels.len() + guild.threads.len();
					if entries > model::account::MAX_ENTRIES {
						return Err(serde::de::Error::custom(
							"Account navigation capacity exceeded",
						));
					}
					guilds.push(crate::GuildDto {
						id: guild.id,
						emojis: optional(guild.emojis, &mut unavailable),
						properties: guild.properties,
						icon: guild.icon,
						name: guild.name,
						channels: guild.channels,
						stickers: optional::<crate::stickers::Catalog>(
							guild.stickers,
							&mut stickers_unavailable,
						)
						.and_then(|catalog| {
							match crate::stickers::guild_catalog(catalog.0, guild.id) {
								Ok(stickers) => Some(model::StickerList(stickers)),
								Err(_) => {
									stickers_unavailable = true;
									None
								}
							}
						}),
						threads: guild.threads,
						roles: guild.roles,
						voice_states: guild.voice_states,
						members: guild.members,
					});
				}
				Ok(Guilds(guilds, unavailable, stickers_unavailable))
			}
		}
		d.deserialize_seq(GuildVisitor)
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
	#[serde(default, deserialize_with = "read_state::account_entries")]
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
	use serde_json::json;

	fn fixture() -> serde_json::Value {
		json!({"user":{"id":"9","username":"Synthetic"},"session_id":"synthetic","resume_gateway_url":"wss://gateway.discord.gg/","guilds":[{"id":"1","owner_id":"9","name":"Synthetic","roles":[],"channels":[{"id":"2","type":0,"name":"general","permission_overwrites":[]}]}]})
	}

	#[test]
	fn optional_failures_preserve_valid_navigation_and_unknown_metadata() {
		let mut payload = fixture();
		payload["read_state"] = json!({"entries":[{"id":"2","last_message_id":true}]});
		payload["user_guild_settings"] = json!({"entries":[{"guild_id":"1","muted":"invalid"}]});
		payload["sessions"] = json!([{"status":false}]);
		payload["presences"] = json!([{"user_id":"3","activities":false}]);
		payload["merged_presences"] = json!({"friends":false});
		payload["guilds"][0]["emojis"] = json!([{"id":"4","name":"invalid emoji name"}]);
		payload["guilds"]
			.as_array_mut()
			.unwrap()
			.push(json!({"id":"5","emojis":[]}));
		let bytes = serde_json::to_vec(&payload).unwrap();
		let envelope = decode(&bytes).unwrap();
		assert_eq!(envelope.permissions().unwrap().channels.len(), 1);
		let (mut ready, warnings) = envelope.navigation().unwrap();
		assert_eq!(
			warnings,
			Warnings {
				read_state: true,
				notifications: true,
				sessions: true,
				presence: true,
				emojis: true,
				stickers: false,
			}
		);
		assert!(
			ready.read_state.is_none()
				&& ready.user_guild_settings.is_none()
				&& ready.sessions.is_none()
		);
		assert!(ready.presences.is_none());
		assert!(ready.merged_presences.unwrap().friends.is_none());
		ready.merged_presences = None;
		let (guilds, channels) = ready.navigation().unwrap();
		assert_eq!(channels.len(), 1);
		assert!(guilds[0].emojis.is_none());
		assert_eq!(guilds[1].emojis, Some(vec![]));
		assert!(
			crate::decode::<crate::GuildDto>(&serde_json::to_vec(&payload["guilds"][0]).unwrap())
				.is_err()
		);
		let supplemental_payload =
			json!({"guilds":[payload["guilds"][0].clone()],"merged_presences":false});
		let (extra, warnings) =
			supplemental(&serde_json::to_vec(&supplemental_payload).unwrap()).unwrap();
		assert!(warnings.emojis && warnings.presence);
		assert_eq!(extra.guilds.len(), 1);
		assert!(extra.guilds[0].emojis.is_none() && extra.merged_presences.is_none());
		assert!(
			supplemental(br#"{"guilds":[{"id":"1","voice_states":[{"user_id":false}]}]}"#).is_err()
		);
	}

	#[test]
	fn invalid_optional_stickers_preserve_ready_and_have_their_own_warning() {
		for stickers in [
			json!(true),
			json!([{"id":"4","name":null,"format_type":1}]),
			json!([{"id":"4","name":"Wave","format_type":1,"guild_id":"8"}]),
			json!([{"id":"4","name":"Wave","format_type":1,"pack_id":"8"}]),
		] {
			let mut payload = fixture();
			payload["guilds"][0]["stickers"] = stickers;
			let bytes = serde_json::to_vec(&payload).unwrap();
			let (mut ready, warnings) = decode(&bytes).unwrap().navigation().unwrap();
			assert!(warnings.stickers);
			assert!(!warnings.emojis);
			let (guilds, channels) = ready.navigation().unwrap();
			assert_eq!(channels.len(), 1);
			assert!(guilds[0].stickers.is_none());
		}
		let mut payload = fixture();
		payload["guilds"][0]["stickers"] = json!([{"id":"4","name":"Wave","format_type":1}]);
		let bytes = serde_json::to_vec(&payload).unwrap();
		let (mut ready, warnings) = decode(&bytes).unwrap().navigation().unwrap();
		assert!(!warnings.stickers);
		assert_eq!(
			ready.navigation().unwrap().0[0].stickers.as_ref().unwrap()[0].guild_id,
			Some(Id(1))
		);
	}

	#[test]
	fn large_accounts_cross_old_navigation_permission_and_read_limits() {
		for guild_count in [70, 96, 200] {
			let mut payload = fixture();
			payload["guilds"] = json!((1..=guild_count).map(|guild| json!({
				"id":guild.to_string(),"name":"Synthetic", "owner_id":"9",
				"roles":[{"id":guild.to_string(),"permissions":"1024"}],
				"channels":(1..=100).map(|channel| json!({"id":(guild*1000+channel).to_string(),"type":0,"name":"general","permission_overwrites":[]})).collect::<Vec<_>>()
			})).collect::<Vec<_>>());
			payload["read_state"] = json!({"entries":(1..=4001).map(|id| json!({"id":id.to_string(),"last_message_id":"7"})).collect::<Vec<_>>()});
			payload["user_guild_settings"] = json!({"entries":[{"guild_id":"1","muted":false,"channel_overrides":(1..=4001).map(|id| json!({"channel_id":id.to_string(),"muted":false})).collect::<Vec<_>>()}]});
			payload["sessions"] = json!([{"status":"online"}]);
			payload["presences"] = json!([{"user_id":"3","status":"online"}]);
			let bytes = serde_json::to_vec(&payload).unwrap();
			let envelope = decode(&bytes).unwrap();
			let permissions = envelope.permissions().unwrap();
			assert_eq!(permissions.channels.len(), guild_count * 100);
			let (mut ready, warnings) = envelope.navigation().unwrap();
			assert_eq!(warnings, Warnings::default());
			assert_eq!(ready.read_state.as_ref().unwrap().entries.len(), 4001);
			assert_eq!(ready.sessions.as_ref().unwrap().dnd(), Some(false));
			let (guilds, channels) = ready.navigation().unwrap();
			assert_eq!(guilds.len(), guild_count);
			assert_eq!(channels.len(), guild_count * 100);
		}
	}

	#[test]
	fn essential_failures_and_navigation_capacity_remain_strict() {
		let mut payload = fixture();
		payload["guilds"][0]["roles"] = json!([{"id":"1","permissions":"invalid"}]);
		let bytes = serde_json::to_vec(&payload).unwrap();
		assert!(decode(&bytes).unwrap().permissions().is_err());
		for fault in [
			json!({"id":"1","channels":[{"id":"2","type":0},{"id":"2","type":0}]}),
			json!({"id":"1","channels":[{"id":"2","guild_id":"3","type":0}]}),
			json!({"id":"1","voice_states":[{"user_id":false}]}),
		] {
			let mut payload = fixture();
			payload["guilds"] = json!([fault]);
			let bytes = serde_json::to_vec(&payload).unwrap();
			let result = decode(&bytes).unwrap().navigation();
			assert!(result.is_err() || result.unwrap().0.navigation().is_err());
		}
		for count in [model::account::MAX_ENTRIES - 1, model::account::MAX_ENTRIES] {
			let mut ready: Ready = crate::decode(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
			ready.guilds[0].channels = (0..count)
				.map(|index| ChannelDto {
					id: Id(index as u64 + 100),
					guild_id: Some(Id(1)),
					name: None,
					kind: 0,
					icon: None,
					flags: 1 << 17,
					last_message_id: None,
					parent_id: None,
					position: 0,
					recipients: Vec::new(),
					permission_overwrites: None,
					message_count: None,
					is_message_request: false,
					is_spam: false,
				})
				.collect();
			assert_eq!(
				ready.navigation().is_ok(),
				count < model::account::MAX_ENTRIES
			);
		}
	}

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
			envelope.navigation().unwrap().0.navigation().unwrap()
				== expected.navigation().unwrap()
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
