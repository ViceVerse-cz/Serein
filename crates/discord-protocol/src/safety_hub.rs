//! Read-only Safety Hub (`GET /safety-hub/@me`) decoding, as documented by Userdoccers and
//! observed in the normal official client. Unofficial and unverified; see
//! `docs/discord-compatibility.md`. Flagged message bodies and attachments are not parsed or
//! retained, and `guild_classifications` are outside this account-scoped read.
use crate::{DecodeError, collectibles::Lenient};
use model::{
	Id,
	safety_hub::{
		AccountStanding, Action, AppealEligibility, AppealStatus, Classification,
		MAX_ACTION_DESCRIPTIONS, MAX_ACTIONS, MAX_APPEAL_ELIGIBILITY, MAX_CLASSIFICATIONS,
		MAX_LINK, MAX_TEXT, Snapshot,
	},
};
use serde::Deserialize;

pub const MAX_WIRE: usize = 128 * 1024;
/// Wire rows read before the retained window is chosen.
const MAX_WIRE_CLASSIFICATIONS: usize = 64;
const MAX_WIRE_ACTIONS: usize = 32;
const MAX_WIRE_DESCRIPTIONS: usize = 16;
const MAX_WIRE_ELIGIBILITY: usize = 16;

#[derive(Deserialize)]
struct HubDto {
	#[serde(default)]
	account_standing: Option<Lenient<StandingDto>>,
	#[serde(default)]
	classifications: Vec<Lenient<ClassificationDto>>,
	#[serde(default)]
	is_appeal_eligible: bool,
	#[serde(default)]
	is_dsa_eligible: bool,
	#[serde(default)]
	appeal_eligibility: Vec<Lenient<u32>>,
}
#[derive(Deserialize)]
struct StandingDto {
	state: u32,
}
#[derive(Deserialize)]
struct ClassificationDto {
	id: Id,
	#[serde(default)]
	classification_type: u32,
	#[serde(default)]
	description: Option<Lenient<String>>,
	#[serde(default)]
	explainer_link: Option<Lenient<String>>,
	#[serde(default)]
	actions: Vec<Lenient<ActionDto>>,
	#[serde(default)]
	max_expiration_time: Option<Lenient<Instant>>,
	#[serde(default)]
	appeal_status: Option<Lenient<AppealDto>>,
	// `flagged_content` is intentionally absent: bodies and attachments never enter memory.
}
#[derive(Deserialize)]
struct ActionDto {
	id: Id,
	#[serde(default)]
	action_type: u32,
	#[serde(default)]
	descriptions: Vec<Lenient<String>>,
}
#[derive(Deserialize)]
struct AppealDto {
	status: u32,
}
/// ISO-8601 text or a Unix timestamp in seconds or milliseconds.
#[derive(Deserialize)]
#[serde(untagged)]
enum Instant {
	Text(String),
	Number(f64),
}
impl Instant {
	fn seconds(value: Lenient<Self>) -> Option<i64> {
		match value {
			Lenient::Value(Self::Text(text)) if text.len() <= 64 => {
				time::OffsetDateTime::parse(&text, &time::format_description::well_known::Rfc3339)
					.ok()
					.map(|at| at.unix_timestamp())
			}
			Lenient::Value(Self::Number(value)) if value.is_finite() && value > 0.0 => {
				Some(if value > 1e11 { value / 1000.0 } else { value } as i64)
			}
			_ => None,
		}
	}
}

fn values<T>(list: Vec<Lenient<T>>) -> impl Iterator<Item = T> {
	list.into_iter().filter_map(|item| match item {
		Lenient::Value(value) => Some(value),
		Lenient::Invalid(_) => None,
	})
}
fn text(value: Option<Lenient<String>>) -> String {
	match value {
		Some(Lenient::Value(value)) => value
			.chars()
			.filter(|c| !c.is_control())
			.take(MAX_TEXT)
			.collect(),
		_ => String::new(),
	}
}
fn link(value: Option<Lenient<String>>) -> Option<String> {
	let Lenient::Value(value) = value? else {
		return None;
	};
	if value.len() > MAX_LINK || value.chars().any(char::is_control) {
		return None;
	}
	let url = url::Url::parse(&value).ok()?;
	(url.scheme() == "https"
		&& url.host_str().is_some()
		&& url.username().is_empty()
		&& url.password().is_none())
	.then(|| url.to_string())
}
fn action(dto: ActionDto) -> Option<Action> {
	let mut descriptions = Vec::with_capacity(MAX_ACTION_DESCRIPTIONS);
	for value in values(dto.descriptions).take(MAX_WIRE_DESCRIPTIONS) {
		descriptions.push(
			value
				.chars()
				.filter(|c| !c.is_control())
				.take(MAX_TEXT)
				.collect::<String>(),
		);
		if descriptions.len() == MAX_ACTION_DESCRIPTIONS {
			break;
		}
	}
	let action = Action {
		id: dto.id,
		action_type: dto.action_type,
		descriptions,
	};
	action.valid().then_some(action)
}
fn classification(dto: ClassificationDto) -> Option<Classification> {
	let mut actions = Vec::with_capacity(MAX_ACTIONS);
	for item in values(dto.actions).take(MAX_WIRE_ACTIONS) {
		if let Some(action) = action(item) {
			actions.push(action);
			if actions.len() == MAX_ACTIONS {
				break;
			}
		}
	}
	let value = Classification {
		id: dto.id,
		classification_type: dto.classification_type,
		description: text(dto.description),
		explainer_link: link(dto.explainer_link),
		actions,
		max_expiration_time: dto.max_expiration_time.and_then(Instant::seconds),
		appeal_status: match dto.appeal_status {
			Some(Lenient::Value(AppealDto { status })) => AppealStatus::parse(status),
			_ => None,
		},
	};
	value.valid().then_some(value)
}

/// Bounded decode. A malformed row is dropped; a malformed or unknown standing becomes
/// unavailable rather than `AllGood`.
pub fn decode_hub(bytes: &[u8]) -> Result<Snapshot, DecodeError> {
	if bytes.len() > MAX_WIRE {
		return Err(DecodeError);
	}
	let dto: HubDto = crate::decode(bytes)?;
	let standing = match dto.account_standing {
		Some(Lenient::Value(StandingDto { state })) => AccountStanding::parse(state),
		_ => None,
	};
	let mut classifications = Vec::with_capacity(MAX_CLASSIFICATIONS);
	for item in dto
		.classifications
		.into_iter()
		.take(MAX_WIRE_CLASSIFICATIONS)
	{
		let Lenient::Value(value) = item else {
			continue;
		};
		let Some(value) = classification(value) else {
			continue;
		};
		classifications.push(value);
		if classifications.len() == MAX_CLASSIFICATIONS {
			break;
		}
	}
	let mut appeal_eligibility = Vec::with_capacity(MAX_APPEAL_ELIGIBILITY);
	for value in values(dto.appeal_eligibility).take(MAX_WIRE_ELIGIBILITY) {
		let Some(value) = AppealEligibility::parse(value) else {
			continue;
		};
		if appeal_eligibility.len() < MAX_APPEAL_ELIGIBILITY && !appeal_eligibility.contains(&value)
		{
			appeal_eligibility.push(value);
		}
	}
	let snapshot = Snapshot {
		standing,
		classifications,
		is_appeal_eligible: dto.is_appeal_eligible,
		is_dsa_eligible: dto.is_dsa_eligible,
		appeal_eligibility,
	};
	if snapshot.valid() {
		Ok(snapshot)
	} else {
		Err(DecodeError)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn standing_is_typed_and_unknown_or_missing_is_unavailable() {
		for (state, expected) in [
			(100, Some(AccountStanding::AllGood)),
			(200, Some(AccountStanding::Limited)),
			(300, Some(AccountStanding::VeryLimited)),
			(400, Some(AccountStanding::AtRisk)),
			(500, Some(AccountStanding::Suspended)),
			(999, None),
		] {
			let bytes = format!(r#"{{"account_standing":{{"state":{state}}}}}"#).into_bytes();
			assert_eq!(decode_hub(&bytes).unwrap().standing, expected);
		}
		assert_eq!(
			decode_hub(br#"{"classifications":[]}"#).unwrap().standing,
			None
		);
		assert_eq!(
			decode_hub(br#"{"account_standing":{"state":"100"}}"#)
				.unwrap()
				.standing,
			None
		);
	}

	#[test]
	fn sparse_fields_are_bounded_and_flagged_content_is_never_retained() {
		let mut flagged = String::new();
		for _ in 0..4096 {
			flagged.push_str("private flagged message body ");
		}
		let body = serde_json::json!({
			"account_standing": {"state": 500},
			"is_appeal_eligible": true,
			"is_dsa_eligible": false,
			"appeal_eligibility": [1, 2, 2, 3, 4],
			"username": "ignored",
			"guild_classifications": [{"id": "999", "classification_type": 3030}],
			"classifications": [{
				"id": "123456789012345678",
				"classification_type": 3030,
				"description": "Synthetic violation",
				"explainer_link": "https://support.discord.com/hc/articles/1",
				"max_expiration_time": "2026-01-02T03:04:05+00:00",
				"appeal_status": {"status": 1},
				"flagged_content": [{
					"type": "message",
					"id": "42",
					"content": flagged,
					"attachments": [{"url": "https://cdn.discordapp.com/private.png"}]
				}],
				"actions": [{
					"id": "98",
					"action_type": 4,
					"descriptions": ["Warning"]
				}]
			}]
		})
		.to_string();
		let snapshot = decode_hub(body.as_bytes()).unwrap();
		assert_eq!(snapshot.standing, Some(AccountStanding::Suspended));
		assert!(snapshot.is_appeal_eligible);
		assert_eq!(
			snapshot.appeal_eligibility,
			[
				AppealEligibility::Dsa,
				AppealEligibility::InApp,
				AppealEligibility::AgeVerification
			]
		);
		assert_eq!(snapshot.classifications.len(), 1);
		let classification = &snapshot.classifications[0];
		assert_eq!(classification.id, Id(123456789012345678));
		assert_eq!(
			classification.appeal_status,
			Some(AppealStatus::ReviewPending)
		);
		assert!(classification.max_expiration_time.is_some());
		assert_eq!(classification.actions[0].descriptions, ["Warning"]);
		assert!(!format!("{snapshot:?}").contains("private flagged message body"));
		assert!(snapshot.bytes() < model::safety_hub::MAX_BYTES);
	}

	#[test]
	fn invalid_links_and_rows_are_dropped_without_rejecting_the_response() {
		let body = serde_json::json!({
			"account_standing": {"state": 200},
			"classifications": [
				{"id": "0", "description": "zero id"},
				{"id": "7", "description": "http link", "explainer_link": "http://example.com/why"},
				{"id": "8", "description": "userinfo", "explainer_link": "https://user:pass@example.com/why"},
				{"id": "9", "description": "normal", "explainer_link": "https://example.com/why"},
				{"id": "10", "description": "bad action", "actions": [{"id": "0"}]}
			]
		})
		.to_string();
		let snapshot = decode_hub(body.as_bytes()).unwrap();
		let ids: Vec<_> = snapshot.classifications.iter().map(|c| c.id.0).collect();
		assert_eq!(ids, [7, 8, 9, 10]);
		assert!(snapshot.classifications[0].explainer_link.is_none());
		assert!(snapshot.classifications[1].explainer_link.is_none());
		assert_eq!(
			snapshot.classifications[2].explainer_link.as_deref(),
			Some("https://example.com/why")
		);
		assert!(snapshot.classifications[3].actions.is_empty());
	}

	#[test]
	fn oversized_or_malformed_payloads_fail_closed() {
		assert!(decode_hub(&vec![b' '; MAX_WIRE + 1]).is_err());
		assert!(decode_hub(b"{").is_err());
		let huge = serde_json::json!({
			"account_standing": {"state": 100},
			"classifications": (0..MAX_CLASSIFICATIONS + 10)
				.map(|i| serde_json::json!({"id": (i + 1).to_string(), "description": "x".repeat(MAX_TEXT + 20)}))
				.collect::<Vec<_>>()
		})
		.to_string();
		let snapshot = decode_hub(huge.as_bytes()).unwrap();
		assert!(snapshot.classifications.len() <= MAX_CLASSIFICATIONS);
		assert!(snapshot.valid());
	}
}
