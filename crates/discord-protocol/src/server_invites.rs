//! Documented guild invite list/delete and mutable INVITES_DISABLED feature.
use crate::{DecodeError, Timestamp, UserDto, permissions::List};
use model::{
	Id,
	server_invites::{self as m, Invite, Snapshot},
};
use serde::Deserialize;
pub const MAX_WIRE: usize = 2 * 1024 * 1024;
#[derive(Deserialize)]
struct Scope {
	id: Id,
}
#[derive(Deserialize)]
struct Channel {
	id: Id,
	name: Option<String>,
}
#[derive(Deserialize)]
struct WireInvite {
	code: String,
	guild: Scope,
	channel: Option<Channel>,
	inviter: Option<UserDto>,
	uses: Option<u64>,
	max_uses: Option<u64>,
	max_age: Option<u64>,
	created_at: Option<Timestamp>,
	expires_at: Option<Timestamp>,
	temporary: Option<bool>,
	roles: Option<List<Scope, 100>>,
}
impl WireInvite {
	fn checked(self, guild: Id) -> Result<Invite, DecodeError> {
		if self.guild.id != guild || guild.0 == 0 {
			return Err(DecodeError);
		}
		let (channel, channel_name) = self
			.channel
			.map_or((None, None), |channel| (Some(channel.id), channel.name));
		let invite = Invite {
			code: self.code,
			channel,
			channel_name,
			inviter: self.inviter.map(UserDto::into_model),
			uses: self.uses,
			max_uses: self.max_uses,
			max_age: self.max_age,
			created_at: self.created_at.map(|time| time.0),
			expires_at: self.expires_at.map(|time| time.0),
			temporary: self.temporary,
			roles: self
				.roles
				.map(|roles| roles.0.into_iter().map(|role| role.id).collect()),
		};
		if !invite.valid() {
			return Err(DecodeError);
		}
		Ok(invite)
	}
}
pub fn features(bytes: &[u8], guild: Id) -> Result<Vec<String>, DecodeError> {
	#[derive(Deserialize)]
	struct Guild {
		id: Id,
		features: List<String, 256>,
	}
	if bytes.len() > MAX_WIRE {
		return Err(DecodeError);
	}
	let metadata: Guild = crate::decode(bytes)?;
	if metadata.id != guild || !m::valid_features(&metadata.features.0) {
		return Err(DecodeError);
	}
	Ok(metadata.features.0)
}
pub fn snapshot(bytes: &[u8], guild: Id, features: Vec<String>) -> Result<Snapshot, DecodeError> {
	if bytes.len() > MAX_WIRE {
		return Err(DecodeError);
	}
	let invites: List<WireInvite, 1000> = crate::decode(bytes)?;
	let value = Snapshot {
		guild,
		items: invites
			.0
			.into_iter()
			.map(|invite| invite.checked(guild))
			.collect::<Result<_, _>>()?,
		features,
	};
	if !value.valid() {
		return Err(DecodeError);
	}
	Ok(value)
}
pub fn revoked(bytes: &[u8], guild: Id, code: &str) -> Result<(), DecodeError> {
	if bytes.len() > 64 * 1024 {
		return Err(DecodeError);
	}
	let invite: WireInvite = crate::decode(bytes)?;
	if invite.checked(guild)?.code != code {
		return Err(DecodeError);
	}
	Ok(())
}
pub fn pause_body(
	mut features: Vec<String>,
	paused: bool,
) -> Result<serde_json::Value, DecodeError> {
	if !m::valid_features(&features) {
		return Err(DecodeError);
	}
	features.retain(|feature| feature != m::PAUSED_FEATURE);
	if paused {
		features.push(m::PAUSED_FEATURE.into());
	}
	if !m::valid_features(&features) {
		return Err(DecodeError);
	}
	Ok(serde_json::json!({"features":features}))
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn invites_metadata_timestamps_roles_and_payload_bounds() {
		{
			let invite = serde_json::json!({"code":"synthetic","guild":{"id":"2"},"roles":[{"id":"5","name":"Member"}],"created_at":"2026-01-01T00:00:00Z","max_age":0,"max_uses":0});
			let page = snapshot(
				&serde_json::to_vec(&vec![invite.clone()]).unwrap(),
				Id(2),
				vec![],
			)
			.unwrap();
			assert_eq!(page.items[0].roles, Some(vec![Id(5)]));
			assert_eq!(page.items[0].created_at, Some(1_767_225_600_000_000_000));
			assert_eq!(page.items[0].max_age, Some(0));
			assert!(
				snapshot(
					&serde_json::to_vec(&vec![invite; 1001]).unwrap(),
					Id(2),
					vec![]
				)
				.is_err()
			);
			assert!(
				features(
					&serde_json::to_vec(
						&serde_json::json!({"id":"2","features":["X".repeat(129)]})
					)
					.unwrap(),
					Id(2)
				)
				.is_err()
			);
			assert!(snapshot(&vec![b' '; MAX_WIRE + 1], Id(2), vec![]).is_err());
			let mut page = page;
			page.features.reserve(m::MAX_BYTES);
			assert!(!page.valid());
		}
		{
			let body =
				br#"[{"code":"abc_1","guild":{"id":"2"},"channel":{"id":"3","name":"chat"}}]"#;
			let page = snapshot(body, Id(2), vec![]).unwrap();
			assert!(page.items[0].uses.is_none() && page.items[0].inviter.is_none());
			assert!(snapshot(body, Id(4), vec![]).is_err());
			assert!(revoked(br#"{"code":"other","guild":{"id":"2"}}"#, Id(2), "abc_1").is_err());
			assert!(
				snapshot(
					br#"[{"code":"x","guild":{"id":"2"}},{"code":"x","guild":{"id":"2"}}]"#,
					Id(2),
					vec![]
				)
				.is_err()
			);
			assert!(!m::valid_code("../other"));
		}
		{
			let body = pause_body(vec!["FUTURE_FLAG".into(), "COMMUNITY".into()], true).unwrap();
			assert_eq!(
				body["features"],
				serde_json::json!(["FUTURE_FLAG", "COMMUNITY", "INVITES_DISABLED"])
			);
			let body =
				pause_body(vec!["FUTURE_FLAG".into(), "INVITES_DISABLED".into()], false).unwrap();
			assert_eq!(body["features"], serde_json::json!(["FUTURE_FLAG"]));
		}
	}
}
