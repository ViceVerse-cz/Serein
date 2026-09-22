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
pub struct Override {
	pub channel_id: Id,
	#[serde(default)]
	pub mute_config: Option<MuteConfig>,
	#[serde(default)]
	pub muted: Option<bool>,
	#[serde(default)]
	pub message_notifications: Option<u8>,
}
#[derive(Deserialize)]
pub struct Overrides(
	#[serde(deserialize_with = "crate::read_state::account_entries")] pub Vec<Override>,
);
#[derive(Deserialize)]
pub struct Setting {
	// Legacy READY can use zero for private-channel settings, not just null.
	#[serde(default, deserialize_with = "crate::read_state::optional_id")]
	pub guild_id: Option<Id>,
	#[serde(default)]
	pub muted: Option<bool>,
	#[serde(default)]
	pub suppress_everyone: Option<bool>,
	#[serde(default)]
	pub suppress_roles: Option<bool>,
	#[serde(default)]
	pub hide_muted_channels: Option<bool>,
	#[serde(default)]
	pub message_notifications: Option<u8>,
	#[serde(default)]
	pub channel_overrides: Option<Overrides>,
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
