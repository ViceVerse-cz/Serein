//! Rules screening and onboarding for the current user. Unofficial user endpoints, documented by
//! discord-userdoccers (resources/guild: member verification, join requests, onboarding).
use crate::{
	DecodeError, decode,
	lossy::{Lossy, Slots, null_default},
	permissions::List,
};
use model::{
	Id,
	onboarding::{
		self as m, Answer, Features, Field, FieldKind, GuildGate, MemberGate, Outcome, Prompt,
		PromptOption, Verification,
	},
};
use serde::{
	Deserialize, Deserializer,
	de::{SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};
use std::{borrow::Cow, collections::BTreeSet};

const MAX_GUILDS: usize = model::account::MAX_ENTRIES;
const MAX_MEMBERS: usize = 4000;
pub const MAX_FORM_WIRE: usize = 512 * 1024;

fn features<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Features>, D::Error> {
	struct Flags;
	impl<'de> Visitor<'de> for Flags {
		type Value = Option<Features>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("guild features")
		}
		fn visit_unit<E>(self) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
			let mut features = Features::default();
			let mut count = 0;
			while let Some(name) = seq.next_element::<Cow<'de, str>>()? {
				count += 1;
				if count > 512 {
					return Err(serde::de::Error::custom("Guild feature capacity exceeded"));
				}
				match name.as_ref() {
					"MEMBER_VERIFICATION_GATE_ENABLED" => features.verification = true,
					"GUILD_ONBOARDING" => features.onboarding = true,
					_ => {}
				}
			}
			Ok(Some(features))
		}
	}
	d.deserialize_any(Flags)
}

#[derive(Deserialize)]
struct Identity {
	id: Id,
}
#[derive(Deserialize)]
struct Member {
	#[serde(default)]
	user: Option<Identity>,
	#[serde(default)]
	user_id: Option<Id>,
	#[serde(default, deserialize_with = "null_default")]
	pending: bool,
	#[serde(default, deserialize_with = "null_default")]
	flags: u64,
}
impl Member {
	fn gate(&self, user: Id) -> Option<MemberGate> {
		(self.user.as_ref().map(|u| u.id).or(self.user_id) == Some(user)).then_some(MemberGate {
			pending: self.pending,
			flags: self.flags,
		})
	}
}
#[derive(Default, Deserialize)]
struct Properties {
	#[serde(default, deserialize_with = "features")]
	features: Option<Features>,
}
#[derive(Deserialize)]
struct Guild {
	id: Id,
	#[serde(default, deserialize_with = "features")]
	features: Option<Features>,
	#[serde(default)]
	properties: Option<Properties>,
	#[serde(default)]
	members: Lossy<Member, MAX_MEMBERS, true>,
}
impl Guild {
	fn gate(self, extra: Vec<Member>, user: Id) -> Option<GuildGate> {
		let features = self.properties.and_then(|p| p.features).or(self.features);
		let member = self
			.members
			.items
			.iter()
			.chain(&extra)
			.find_map(|m| m.gate(user));
		(self.id.0 != 0 && (features.is_some() || member.is_some())).then_some(GuildGate {
			guild: self.id,
			features,
			member,
		})
	}
}
fn relevant(gate: &GuildGate) -> bool {
	gate.features
		.is_some_and(|f| f.verification || f.onboarding)
		|| gate.member.is_some_and(|m| m.pending)
}

/// READY: only servers that can gate a member are kept; everything else is implicitly open.
pub fn ready(guilds: &[u8], merged: Option<&[u8]>, user: Id) -> Vec<GuildGate> {
	type Merged = Slots<Lossy<Member, MAX_MEMBERS, true>, MAX_GUILDS>;
	let Ok(guilds) = serde_json::from_slice::<Slots<Guild, MAX_GUILDS>>(guilds) else {
		return Vec::new();
	};
	let mut merged = merged
		.and_then(|bytes| serde_json::from_slice::<Merged>(bytes).ok())
		.filter(|rows| rows.items.len() == guilds.items.len())
		.map(|rows| rows.items)
		.unwrap_or_default()
		.into_iter();
	let mut seen = BTreeSet::new();
	guilds
		.items
		.into_iter()
		.filter_map(|guild| {
			let extra = merged.next().flatten().map(|m| m.items).unwrap_or_default();
			guild?.gate(extra, user)
		})
		.filter(|gate| relevant(gate) && seen.insert(gate.guild))
		.collect()
}
/// GUILD_CREATE and GUILD_UPDATE; a payload without members only refreshes features.
pub fn guild(bytes: &[u8], user: Id) -> Option<GuildGate> {
	decode::<Guild>(bytes).ok()?.gate(Vec::new(), user)
}
/// GUILD_MEMBER_UPDATE for the current user.
pub fn member(bytes: &[u8], user: Id) -> Option<GuildGate> {
	#[derive(Deserialize)]
	struct Update {
		guild_id: Id,
		#[serde(flatten)]
		member: Member,
	}
	let update = decode::<Update>(bytes).ok()?;
	(update.guild_id.0 != 0)
		.then(|| update.member.gate(user))
		.flatten()
		.map(|member| GuildGate {
			guild: update.guild_id,
			features: None,
			member: Some(member),
		})
}

fn text(value: Option<String>, max: usize) -> Option<String> {
	let value: String = value?
		.chars()
		.filter(|c| !c.is_control() || *c == '\n')
		.take(max)
		.collect();
	let value = value.trim();
	(!value.is_empty()).then(|| value.to_owned())
}
#[derive(Deserialize)]
struct WireField {
	field_type: String,
	#[serde(default, deserialize_with = "null_default")]
	label: String,
	#[serde(default)]
	description: Option<String>,
	#[serde(default)]
	placeholder: Option<String>,
	#[serde(default, deserialize_with = "null_default")]
	values: List<String, { m::MAX_RULES }>,
	#[serde(default, deserialize_with = "null_default")]
	choices: List<String, { m::MAX_CHOICES }>,
	#[serde(default)]
	required: bool,
}
#[derive(Deserialize)]
struct WireVerification {
	#[serde(default)]
	version: Option<String>,
	#[serde(default)]
	description: Option<String>,
	#[serde(default, deserialize_with = "null_default")]
	form_fields: List<WireField, { m::MAX_FIELDS }>,
}
/// `GET /guilds/{id}/member-verification`; no questions means nothing to screen.
pub fn verification(bytes: &[u8]) -> Result<Option<Verification>, DecodeError> {
	if bytes.len() > MAX_FORM_WIRE {
		return Err(DecodeError);
	}
	let wire: WireVerification = decode(bytes)?;
	let fields: Vec<Field> = wire
		.form_fields
		.0
		.into_iter()
		.map(|f| Field {
			kind: match f.field_type.as_str() {
				"TERMS" => FieldKind::Terms,
				"TEXT_INPUT" => FieldKind::TextInput,
				"PARAGRAPH" => FieldKind::Paragraph,
				"MULTIPLE_CHOICE" => FieldKind::MultipleChoice,
				_ => FieldKind::Unknown,
			},
			wire_kind: f.field_type.chars().take(64).collect(),
			label: text(Some(f.label), 300).unwrap_or_default(),
			description: text(f.description, 300),
			placeholder: text(f.placeholder, 300),
			rules: f
				.values
				.0
				.into_iter()
				.filter_map(|r| text(Some(r), 300))
				.collect(),
			choices: f
				.choices
				.0
				.into_iter()
				.map(|c| text(Some(c), 150).unwrap_or_default())
				.collect(),
			required: f.required,
		})
		.collect();
	if fields.is_empty() {
		return Ok(None);
	}
	Ok(Some(Verification {
		version: wire.version.filter(|v| v.len() <= 64),
		description: text(wire.description, 300),
		fields,
	}))
}

#[derive(Deserialize)]
struct WireEmoji {
	#[serde(default)]
	id: Option<Id>,
	#[serde(default)]
	name: Option<String>,
}
#[derive(Deserialize)]
struct WireOption {
	id: Id,
	#[serde(default, deserialize_with = "null_default")]
	title: String,
	#[serde(default)]
	description: Option<String>,
	#[serde(default)]
	emoji: Option<WireEmoji>,
}
#[derive(Deserialize)]
struct WirePrompt {
	id: Id,
	#[serde(default, deserialize_with = "null_default")]
	title: String,
	#[serde(default)]
	single_select: bool,
	#[serde(default)]
	required: bool,
	#[serde(default = "yes")]
	in_onboarding: bool,
	#[serde(default, deserialize_with = "null_default")]
	options: List<WireOption, { m::MAX_OPTIONS }>,
}
fn yes() -> bool {
	true
}
#[derive(Deserialize)]
struct WireOnboarding {
	#[serde(default)]
	enabled: bool,
	#[serde(default, deserialize_with = "null_default")]
	prompts: List<WirePrompt, { m::MAX_PROMPTS }>,
	#[serde(default, deserialize_with = "null_default")]
	responses: List<Id, 750>,
}
/// `GET /guilds/{id}/onboarding`; disabled onboarding or no onboarding prompts means nothing to ask.
pub fn onboarding(bytes: &[u8]) -> Result<Option<m::Onboarding>, DecodeError> {
	if bytes.len() > MAX_FORM_WIRE {
		return Err(DecodeError);
	}
	let wire: WireOnboarding = decode(bytes)?;
	if !wire.enabled {
		return Ok(None);
	}
	let prompts: Vec<Prompt> = wire
		.prompts
		.0
		.into_iter()
		.filter(|p| p.in_onboarding && p.id.0 != 0)
		.map(|p| Prompt {
			id: p.id,
			title: text(Some(p.title), 100).unwrap_or_default(),
			single_select: p.single_select,
			required: p.required,
			options: p
				.options
				.0
				.into_iter()
				.filter(|o| o.id.0 != 0)
				.map(|o| PromptOption {
					id: o.id,
					title: text(Some(o.title), 50).unwrap_or_default(),
					description: text(o.description, 100),
					emoji: o
						.emoji
						.filter(|e| e.id.is_none())
						.and_then(|e| text(e.name, 32)),
				})
				.collect(),
		})
		.filter(|p| !p.options.is_empty())
		.collect();
	if prompts.is_empty() {
		return Ok(None);
	}
	Ok(Some(m::Onboarding {
		prompts,
		responses: wire.responses.0,
	}))
}

/// `PUT /guilds/{id}/requests/@me` body: every field echoed back with its response.
pub fn join_request_body(form: &Verification, answers: &[Answer]) -> Value {
	let fields: Vec<Value> = form
		.fields
		.iter()
		.zip(answers)
		.map(|(field, answer)| {
			let mut row = Map::new();
			row.insert("field_type".into(), json!(field.wire_kind));
			row.insert("label".into(), json!(field.label));
			row.insert("required".into(), json!(field.required));
			row.insert("description".into(), json!(field.description));
			if !field.rules.is_empty() {
				row.insert("values".into(), json!(field.rules));
			}
			if !field.choices.is_empty() {
				row.insert("choices".into(), json!(field.choices));
			}
			if field.placeholder.is_some() {
				row.insert("placeholder".into(), json!(field.placeholder));
			}
			let response = match answer {
				Answer::Terms(agreed) => json!(agreed),
				Answer::Text(text) if !text.trim().is_empty() => json!(text.trim()),
				Answer::Choice(Some(index)) => json!(index),
				_ => Value::Null,
			};
			row.insert("response".into(), response);
			Value::Object(row)
		})
		.collect();
	json!({ "version": form.version, "form_fields": fields })
}
/// `POST /guilds/{id}/onboarding-responses` body; every shown prompt and option counts as seen.
pub fn onboarding_body(form: &m::Onboarding, chosen: &[Id], now_ms: u64) -> Value {
	let prompts: Map<String, Value> = form
		.prompts
		.iter()
		.map(|p| (p.id.to_string(), json!(now_ms)))
		.collect();
	let options: Map<String, Value> = form
		.prompts
		.iter()
		.flat_map(|p| &p.options)
		.map(|o| (o.id.to_string(), json!(now_ms)))
		.collect();
	json!({
		"onboarding_responses": chosen.iter().map(Id::to_string).collect::<Vec<_>>(),
		"onboarding_prompts_seen": prompts,
		"onboarding_responses_seen": options,
	})
}
/// Partial guild join request returned by the submit call.
pub fn join_request(bytes: &[u8]) -> Result<Outcome, DecodeError> {
	#[derive(Deserialize)]
	struct Request {
		#[serde(default)]
		application_status: Option<String>,
	}
	let request: Request = decode(bytes)?;
	Ok(match request.application_status.as_deref() {
		Some("APPROVED") | None => Outcome::Approved,
		Some("REJECTED") => Outcome::Rejected,
		_ => Outcome::Submitted,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gates_come_from_properties_and_merged_self_member_rows() {
		let guilds = br#"[
			{"id":"1","properties":{"features":["MEMBER_VERIFICATION_GATE_ENABLED"]}},
			{"id":"2","features":["COMMUNITY"]},
			{"id":"3","features":["GUILD_ONBOARDING"]}
		]"#;
		let merged = br#"[
			[{"user_id":"9","pending":true,"flags":0}],
			[{"user_id":"9","pending":false}],
			[{"user_id":"8","pending":true},{"user_id":"9","flags":2}]
		]"#;
		let gates = ready(guilds, Some(merged), Id(9));
		assert_eq!(gates.len(), 2);
		assert_eq!(
			gates[0].member,
			Some(MemberGate {
				pending: true,
				flags: 0
			})
		);
		assert_eq!(gates[1].member.map(|m| m.flags), Some(2));
		let update = member(
			br#"{"guild_id":"1","user":{"id":"9"},"pending":false}"#,
			Id(9),
		);
		assert_eq!(
			update.and_then(|g| g.member).map(|m| m.pending),
			Some(false)
		);
		assert!(member(br#"{"guild_id":"1","user":{"id":"8"}}"#, Id(9)).is_none());
	}

	#[test]
	fn forms_decode_and_echo_answers() {
		let form = verification(br#"{"version":"2026-01-01T00:00:00+00:00","description":"Hi","form_fields":[{"field_type":"TERMS","label":"Read the rules","values":["Be kind","No spam"],"required":true,"description":null,"automations":null}]}"#).unwrap().unwrap();
		assert_eq!(form.fields[0].kind, FieldKind::Terms);
		assert_eq!(form.fields[0].rules.len(), 2);
		let body = join_request_body(&form, &[Answer::Terms(true)]);
		assert_eq!(body["form_fields"][0]["response"], json!(true));
		assert_eq!(body["version"], json!("2026-01-01T00:00:00+00:00"));
		assert!(
			verification(br#"{"version":null,"form_fields":[]}"#)
				.unwrap()
				.is_none()
		);
		let onboarding = onboarding(br#"{"guild_id":"1","enabled":true,"mode":0,"default_channel_ids":["5"],"prompts":[
			{"id":"2","type":0,"title":"Games","single_select":false,"required":true,"in_onboarding":true,"options":[{"id":"3","title":"Chess","description":null,"emoji":{"id":null,"name":"\u265f"},"role_ids":["4"],"channel_ids":[]}]},
			{"id":"6","type":0,"title":"Later","single_select":false,"required":false,"in_onboarding":false,"options":[{"id":"7","title":"X"}]}
		],"responses":[]}"#).unwrap().unwrap();
		assert_eq!(onboarding.prompts.len(), 1);
		assert_eq!(onboarding.prompts[0].options[0].emoji.as_deref(), Some("♟"));
		let body = onboarding_body(&onboarding, &[Id(3)], 5);
		assert_eq!(body["onboarding_responses"], json!(["3"]));
		assert_eq!(body["onboarding_responses_seen"]["3"], json!(5));
		assert_eq!(
			join_request(br#"{"application_status":"SUBMITTED"}"#).unwrap(),
			Outcome::Submitted
		);
	}
}
