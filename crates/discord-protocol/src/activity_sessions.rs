//! Unofficial self-session activity observations, never proof of visibility to every peer.
//! Reference: discord.py-self discord/state.py parse_sessions_replace and activity.py Session.
use crate::{DecodeError, rpc::Activity};
use model::Id;
use serde::{Deserialize, Deserializer, de::SeqAccess, de::Visitor};
use serde_json::value::RawValue;
use std::fmt;

const MAX_SESSIONS_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Observation {
	#[default]
	Unconfirmed,
	/// The current game is present in Discord's aggregate public activity list.
	ServerListed,
	/// Discord reports the current game only in a hidden activity list.
	ServerHidden,
	/// Discord received the game for this session; aggregate visibility is unknown.
	ServerReceived,
	/// An aggregate snapshot does not list the game publicly or explicitly as hidden.
	ServerMissing,
}

#[derive(Deserialize)]
struct Entries<'a>(#[serde(borrow, deserialize_with = "entries")] Vec<&'a RawValue>);

fn entries<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<&'de RawValue>, D::Error> {
	struct Bounded;
	impl<'de> Visitor<'de> for Bounded {
		type Value = Vec<&'de RawValue>;
		fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
			f.write_str("at most 16 session or activity objects")
		}
		fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
			let mut values = Vec::new();
			while let Some(value) = seq.next_element::<&RawValue>()? {
				if values.len() == 16 {
					return Err(serde::de::Error::custom(
						"Session observation exceeds capacity",
					));
				}
				values.push(value);
			}
			Ok(values)
		}
	}
	d.deserialize_seq(Bounded)
}

#[derive(Deserialize)]
struct Session<'a> {
	session_id: &'a str,
	#[serde(default, borrow)]
	activities: Option<&'a RawValue>,
	#[serde(default, borrow)]
	hidden_activities: Option<&'a RawValue>,
}

fn contains(bytes: &RawValue, current: &Activity) -> Result<bool, DecodeError> {
	#[derive(Deserialize)]
	struct Identity {
		#[serde(default)]
		application_id: Option<Id>,
		#[serde(rename = "type")]
		kind: u8,
		/// Only an unverified game is identified by name; the session list is already bounded.
		#[serde(default)]
		name: Option<String>,
	}
	let values: Entries<'_> = serde_json::from_str(bytes.get()).map_err(|_| DecodeError)?;
	let mut found = false;
	for value in values.0 {
		if !value.get().starts_with('{') {
			return Err(DecodeError);
		}
		let identity: Identity = serde_json::from_str(value.get()).map_err(|_| DecodeError)?;
		// A user-added game is published without an application, so Discord lists it without
		// one too; matching on the absent id alone would never confirm it.
		let application = identity.application_id.filter(|id| id.0 != 0);
		found |= identity.kind == current.kind
			&& if current.application_id.0 == 0 {
				application.is_none() && identity.name.as_deref() == Some(current.name.as_str())
			} else {
				application == Some(current.application_id)
			};
	}
	Ok(found)
}

/// Borrows a bounded projection; no session IDs, game details or raw payloads are retained.
pub fn observe(
	bytes: &[u8],
	current_session: &str,
	current: &Activity,
) -> Result<Observation, DecodeError> {
	if bytes.len() > MAX_SESSIONS_BYTES {
		return Err(DecodeError);
	}
	let values: Entries<'_> = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	let mut aggregate = None;
	let mut own = None;
	for value in values.0 {
		if !value.get().starts_with('{') {
			return Err(DecodeError);
		}
		let session: Session<'_> = serde_json::from_str(value.get()).map_err(|_| DecodeError)?;
		if session.session_id.is_empty() || session.session_id.len() > 2048 {
			return Err(DecodeError);
		}
		let target = if session.session_id == "all" {
			&mut aggregate
		} else if session.session_id == current_session {
			&mut own
		} else {
			continue;
		};
		if target.is_some() {
			return Err(DecodeError);
		}
		*target = Some(session);
	}
	let is_aggregate = aggregate.is_some();
	let Some(session) = aggregate.or(own) else {
		return Ok(Observation::Unconfirmed);
	};
	let public = session
		.activities
		.map(|v| contains(v, current))
		.transpose()?;
	let hidden = session
		.hidden_activities
		.map(|v| contains(v, current))
		.transpose()?;
	Ok(if public == Some(true) {
		if is_aggregate {
			Observation::ServerListed
		} else {
			Observation::ServerReceived
		}
	} else if hidden == Some(true) {
		Observation::ServerHidden
	} else if is_aggregate && public == Some(false) {
		Observation::ServerMissing
	} else {
		Observation::Unconfirmed
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::{Value, json};

	#[test]
	fn aggregate_and_own_sessions_distinguish_received_hidden_and_missing() {
		let game = crate::rpc::ActivityFields::default()
			.into_activity(Id(42), "osu!".into())
			.unwrap();
		let game_wire = json!({"application_id":"42","type":0});
		let observe_value = |v| observe(&serde_json::to_vec(&v).unwrap(), "own", &game).unwrap();
		assert_eq!(
			observe_value(json!([{"session_id":"other","activities":[game_wire]}])),
			Observation::Unconfirmed
		);
		assert_eq!(
			observe_value(json!([{"session_id":"own","activities":[game_wire]}])),
			Observation::ServerReceived
		);
		assert_eq!(
			observe_value(
				json!([{"session_id":"all","activities":[game_wire],"hidden_activities":[]}])
			),
			Observation::ServerListed
		);
		assert_eq!(
			observe_value(
				json!([{"session_id":"all","activities":[],"hidden_activities":[game_wire]}])
			),
			Observation::ServerHidden
		);
		assert_eq!(
			observe_value(
				json!([{"session_id":"own","activities":[game_wire]},{"session_id":"all","activities":[],"hidden_activities":[]}])
			),
			Observation::ServerMissing
		);
		assert_eq!(
			observe_value(json!([{"session_id":"all","activities":[]}])),
			Observation::ServerMissing
		);
		assert_eq!(
			observe_value(json!([{"session_id":"all"}])),
			Observation::Unconfirmed
		);
		assert!(observe(&vec![b' '; MAX_SESSIONS_BYTES + 1], "own", &game).is_err());
		assert!(
			observe(
				&serde_json::to_vec(&vec![json!({"session_id":"other"}); 17]).unwrap(),
				"own",
				&game
			)
			.is_err()
		);
		assert!(
			observe(
				&serde_json::to_vec(&json!([{"session_id":"all","activities":vec![game_wire;17]}]))
					.unwrap(),
				"own",
				&game
			)
			.is_err()
		);
		// An unverified game is listed without an application id and is matched by its name.
		let unverified = crate::rpc::ActivityFields::default()
			.into_activity(Id(0), "My game".into())
			.unwrap();
		let listed = |activity: Value| {
			observe(
				&serde_json::to_vec(
					&json!([{"session_id":"all","activities":[activity],"hidden_activities":[]}]),
				)
				.unwrap(),
				"own",
				&unverified,
			)
			.unwrap()
		};
		assert_eq!(
			listed(json!({"name":"My game","type":0,"created_at":1})),
			Observation::ServerListed
		);
		assert_eq!(
			listed(json!({"name":"Other game","type":0})),
			Observation::ServerMissing
		);
		assert_eq!(
			listed(json!({"name":"My game","type":0,"application_id":"42"})),
			Observation::ServerMissing
		);
		assert_eq!(
			observe_value(json!([{"session_id":"all","activities":[{"name":"osu!","type":0}]}])),
			Observation::ServerMissing
		);
		for malformed in [
			br#"[{"session_id":"all"},{"session_id":"all"}]"#.as_slice(),
			br#"[{"session_id":"all","activities":[["42",0]]}]"#,
		] {
			assert!(observe(malformed, "own", &game).is_err());
		}
	}
}
