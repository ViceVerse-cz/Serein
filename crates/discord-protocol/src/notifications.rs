//! Unofficial normal-user settings/session payloads; unknown guild preferences disable OS alerts.
use model::Id;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct MuteConfig {
	#[serde(default)]
	pub end_time: Option<crate::Timestamp>,
}
impl MuteConfig {
	pub fn until(&self) -> Option<i64> {
		self.end_time
			.as_ref()
			.and_then(|at| i64::try_from(at.0 / 1_000_000_000).ok())
	}
}
#[derive(Deserialize)]
#[serde(from = "OverrideWire")]
pub struct Override {
	pub channel_id: Id,
	pub mute_config: Option<MuteConfig>,
	pub muted: Option<bool>,
	pub message_notifications: Option<u8>,
	absent: u8,
}
#[derive(Deserialize)]
struct OverrideWire {
	channel_id: Id,
	#[serde(default)]
	mute_config: Option<MuteConfig>,
	#[serde(default)]
	muted: model::Patch<bool>,
	#[serde(default)]
	message_notifications: model::Patch<u8>,
}
impl From<OverrideWire> for Override {
	fn from(wire: OverrideWire) -> Self {
		let mut absent = 0;
		let muted = match wire.muted {
			model::Patch::Value(value) => Some(value),
			model::Patch::Null => None,
			model::Patch::Absent => {
				absent |= 1;
				None
			}
		};
		let message_notifications = match wire.message_notifications {
			model::Patch::Value(value) => Some(value),
			model::Patch::Null => None,
			model::Patch::Absent => {
				absent |= 2;
				None
			}
		};
		Self {
			channel_id: wire.channel_id,
			mute_config: wire.mute_config,
			muted,
			message_notifications,
			absent,
		}
	}
}
#[derive(Deserialize)]
pub struct Overrides(
	#[serde(deserialize_with = "crate::read_state::account_entries")] pub Vec<Override>,
);
fn present_overrides<'de, D: serde::Deserializer<'de>>(
	d: D,
) -> Result<Option<Overrides>, D::Error> {
	Overrides::deserialize(d).map(Some)
}
pub struct Setting {
	pub guild_id: Option<Id>,
	pub mute_config: Option<MuteConfig>,
	pub muted: Option<bool>,
	pub suppress_everyone: Option<bool>,
	pub suppress_roles: Option<bool>,
	pub hide_muted_channels: Option<bool>,
	pub message_notifications: Option<u8>,
	pub channel_overrides: Option<Overrides>,
	absent: u8,
}
impl<'de> Deserialize<'de> for Setting {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		struct Wire {
			#[serde(default, deserialize_with = "crate::read_state::optional_id")]
			guild_id: Option<Id>,
			#[serde(default)]
			mute_config: Option<MuteConfig>,
			#[serde(default)]
			muted: model::Patch<bool>,
			#[serde(default)]
			suppress_everyone: model::Patch<bool>,
			#[serde(default)]
			suppress_roles: model::Patch<bool>,
			#[serde(default)]
			hide_muted_channels: model::Patch<bool>,
			#[serde(default)]
			message_notifications: model::Patch<u8>,
			#[serde(default, deserialize_with = "present_overrides")]
			channel_overrides: Option<Overrides>,
		}
		fn value<T>(patch: model::Patch<T>, bit: u8, absent: &mut u8) -> Option<T> {
			match patch {
				model::Patch::Absent => {
					*absent |= bit;
					None
				}
				model::Patch::Null => None,
				model::Patch::Value(value) => Some(value),
			}
		}
		let wire = Wire::deserialize(deserializer)?;
		let mut absent = 0;
		Ok(Self {
			guild_id: wire.guild_id,
			mute_config: wire.mute_config,
			muted: value(wire.muted, 1, &mut absent),
			suppress_everyone: value(wire.suppress_everyone, 2, &mut absent),
			suppress_roles: value(wire.suppress_roles, 4, &mut absent),
			hide_muted_channels: value(wire.hide_muted_channels, 8, &mut absent),
			message_notifications: value(wire.message_notifications, 16, &mut absent),
			channel_overrides: wire.channel_overrides,
			absent,
		})
	}
}
impl Setting {
	/// Gateway objects are full settings records. Only omitted fields use service defaults;
	/// explicit null stays unknown. REST acknowledgements deliberately skip this conversion.
	pub fn with_defaults(mut self) -> Self {
		if self.absent & 1 != 0 {
			self.muted = Some(false);
		}
		if self.absent & 2 != 0 {
			self.suppress_everyone = Some(false);
		}
		if self.absent & 4 != 0 {
			self.suppress_roles = Some(false);
		}
		if self.absent & 8 != 0 {
			self.hide_muted_channels = Some(false);
		}
		if self.absent & 16 != 0 {
			self.message_notifications = Some(3);
		}
		if let Some(overrides) = &mut self.channel_overrides {
			for row in &mut overrides.0 {
				if row.absent & 1 != 0 {
					row.muted = Some(false);
				}
				if row.absent & 2 != 0 {
					row.message_notifications = Some(3);
				}
			}
		}
		self
	}
}
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Snapshot {
	Versioned {
		#[serde(deserialize_with = "crate::read_state::account_entries")]
		entries: Vec<Setting>,
		#[serde(default)]
		partial: bool,
	},
	Legacy(#[serde(deserialize_with = "crate::read_state::account_entries")] Vec<Setting>),
}
impl Snapshot {
	pub fn entries(self) -> (Vec<Setting>, bool) {
		match self {
			Self::Versioned { entries, partial } => (entries, !partial),
			Self::Legacy(entries) => (entries, true),
		}
	}
}
#[derive(Deserialize)]
pub struct Session {
	pub status: String,
}
#[derive(Deserialize)]
pub struct Sessions(#[serde(deserialize_with = "crate::read_state::entries")] pub Vec<Session>);
impl Sessions {
	pub fn dnd(&self) -> Option<bool> {
		if self.0.iter().any(|s| s.status == "dnd") {
			return Some(true);
		}
		(!self.0.is_empty()
			&& self.0.iter().all(|s| {
				matches!(
					s.status.as_str(),
					"online" | "idle" | "offline" | "invisible"
				)
			}))
		.then_some(false)
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn defaults_apply_only_to_omitted_gateway_fields_not_rest_confirmation() {
		let sparse: Setting = crate::decode(br#"{"guild_id":"1"}"#).unwrap();
		assert_eq!(sparse.muted, None);
		assert_eq!(sparse.message_notifications, None);
		let defaults = sparse.with_defaults();
		assert_eq!(defaults.muted, Some(false));
		assert_eq!(defaults.message_notifications, Some(3));
		assert_eq!(defaults.suppress_everyone, Some(false));
		let unknown: Setting =
			crate::decode(br#"{"guild_id":"1","muted":null,"message_notifications":null}"#)
				.unwrap();
		let unknown = unknown.with_defaults();
		assert_eq!(unknown.muted, None);
		assert_eq!(unknown.message_notifications, None);
		assert!(crate::decode::<Setting>(br#"{"guild_id":"1","channel_overrides":null}"#).is_err());
	}

	#[test]
	fn mention_suppression_settings_preserve_unknown_false_and_true() {
		for (fields, everyone, roles) in [
			(r#""#, None, None),
			(
				r#", "suppress_everyone":null,"suppress_roles":null"#,
				None,
				None,
			),
			(
				r#", "suppress_everyone":false,"suppress_roles":true"#,
				Some(false),
				Some(true),
			),
			(
				r#", "suppress_everyone":true,"suppress_roles":false"#,
				Some(true),
				Some(false),
			),
		] {
			let setting: Setting =
				crate::decode(format!(r#"{{"guild_id":"1"{fields}}}"#).as_bytes()).unwrap();
			assert_eq!(
				(setting.suppress_everyone, setting.suppress_roles),
				(everyone, roles)
			);
		}
		assert!(crate::decode::<Setting>(br#"{"guild_id":"1","suppress_roles":"false"}"#).is_err());
		assert!(crate::decode::<Setting>(br#"{"guild_id":"1","suppress_everyone":0}"#).is_err());
	}
	#[test]
	fn bounded_preferences_and_unknown_presence_fail_closed() {
		let snapshot: Snapshot = crate::decode(br#"{"entries":[{"guild_id":null,"muted":false,"message_notifications":0,"channel_overrides":[{"channel_id":"2","muted":true,"message_notifications":2}]}],"partial":false}"#).unwrap();
		let (settings, complete) = snapshot.entries();
		assert!(complete);
		assert_eq!(
			settings[0].channel_overrides.as_ref().unwrap().0[0].channel_id,
			Id(2)
		);
		assert_eq!(
			settings[0].channel_overrides.as_ref().unwrap().0[0].muted,
			Some(true)
		);
		assert_eq!(
			crate::decode::<Sessions>(br#"[{"status":"online"},{"status":"dnd"}]"#)
				.unwrap()
				.dnd(),
			Some(true)
		);
		assert_eq!(
			crate::decode::<Sessions>(br#"[{"status":"new-status"}]"#)
				.unwrap()
				.dnd(),
			None
		);
		assert_eq!(crate::decode::<Sessions>(br#"[]"#).unwrap().dnd(), None);
		let oversized = serde_json::json!({"entries":vec![serde_json::json!({"guild_id":null});model::account::MAX_ENTRIES+1]});
		assert!(crate::decode::<Snapshot>(&serde_json::to_vec(&oversized).unwrap()).is_err());
	}
}
