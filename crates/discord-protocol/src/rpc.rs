//! Bounded, activity-only Discord IPC payloads; no authorization or account RPC commands.
use crate::DecodeError;
use model::{Id, MAX_ACTIVITY_TIMESTAMP};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_FRAME_BYTES: usize = 16 * 1024;
/// Registered keys are short, but a resolved `mp:external/...` proxy path or the source
/// URL a launcher sends before resolution needs the same room as a proxied presence image.
pub const MAX_ASSET_KEY: usize = 1024;
/// Invite codes are vanity URLs or short random codes; never a path or query string.
pub const MAX_INVITE_CODE: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Activity {
	pub name: String,
	pub application_id: Id,
	#[serde(rename = "type")]
	pub kind: u8,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub details: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub state: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub timestamps: Option<Timestamps>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub assets: Option<Assets>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ActivityFields {
	#[serde(default, rename = "type")]
	pub kind: u8,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub details: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub state: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub timestamps: Option<Timestamps>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub assets: Option<Assets>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Timestamps {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub start: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub end: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Assets {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub large_image: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub large_text: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub small_image: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub small_text: Option<String>,
}

fn text_valid(value: &str, limit: usize) -> bool {
	value.len() <= limit && !value.chars().any(char::is_control)
}

impl Activity {
	/// At most seven strings (1,152 UTF-8 bytes total) plus fixed metadata.
	pub fn validate(&self) -> Result<(), DecodeError> {
		if self.application_id.0 == 0 || self.name.trim().is_empty() || !text_valid(&self.name, 128)
		{
			return Err(DecodeError);
		}
		validate_fields(
			self.kind,
			&self.details,
			&self.state,
			&self.timestamps,
			&self.assets,
		)
	}
}

fn validate_fields(
	kind: u8,
	details: &Option<String>,
	state: &Option<String>,
	timestamps: &Option<Timestamps>,
	assets: &Option<Assets>,
) -> Result<(), DecodeError> {
	if !matches!(kind, 0 | 2 | 3 | 5)
		|| [details, state]
			.into_iter()
			.flatten()
			.any(|text| !text_valid(text, 128))
		|| timestamps.as_ref().is_some_and(|t| {
			[t.start, t.end]
				.into_iter()
				.flatten()
				.any(|v| v > MAX_ACTIVITY_TIMESTAMP)
				|| matches!((t.start, t.end), (Some(start), Some(end)) if end < start)
		}) || assets.as_ref().is_some_and(|a| {
		[&a.large_text, &a.small_text]
			.into_iter()
			.flatten()
			.any(|text| !text_valid(text, 128))
			|| [&a.large_image, &a.small_image]
				.into_iter()
				.flatten()
				.any(|text| !text_valid(text, MAX_ASSET_KEY))
	}) {
		return Err(DecodeError);
	}
	Ok(())
}

impl ActivityFields {
	pub fn into_activity(self, application_id: Id, name: String) -> Result<Activity, DecodeError> {
		let mut activity = Activity {
			name,
			application_id,
			kind: self.kind,
			details: self.details,
			state: self.state,
			timestamps: self.timestamps,
			assets: self.assets,
		};
		// Legacy RPC sends seconds; modern SDKs send milliseconds (Gateway's unit).
		if let Some(timestamps) = &mut activity.timestamps {
			for timestamp in [&mut timestamps.start, &mut timestamps.end]
				.into_iter()
				.flatten()
			{
				// ponytail: unit heuristic covers current games, but milliseconds before April
				// 1970 are ambiguous; use explicit SDK-version units if historical dates matter.
				if *timestamp < 10_000_000_000 {
					*timestamp *= 1000;
				}
			}
		}
		activity.validate()?;
		Ok(activity)
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetActivity {
	pub nonce: String,
	pub pid: u32,
	pub activity: Option<ActivityFields>,
}

fn payload(bytes: &[u8]) -> Result<Value, DecodeError> {
	if bytes.len() > MAX_FRAME_BYTES {
		return Err(DecodeError);
	}
	let value: Value = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	if !value.is_object() {
		return Err(DecodeError);
	}
	Ok(value)
}

pub fn decode_handshake(bytes: &[u8]) -> Result<Id, DecodeError> {
	let value = payload(bytes)?;
	if value.get("v").and_then(Value::as_u64) != Some(1) {
		return Err(DecodeError);
	}
	value
		.get("client_id")
		.and_then(Value::as_str)
		.ok_or(DecodeError)?
		.parse()
		.map_err(|_| DecodeError)
}

pub fn decode_command(bytes: &[u8]) -> Result<SetActivity, DecodeError> {
	let mut value = payload(bytes)?;
	if value.get("cmd").and_then(Value::as_str) != Some("SET_ACTIVITY") {
		return Err(DecodeError);
	}
	let nonce = decode_nonce(&value)?;
	let args = value
		.get_mut("args")
		.and_then(Value::as_object_mut)
		.ok_or(DecodeError)?;
	let pid = args
		.get("pid")
		.and_then(Value::as_u64)
		.and_then(|pid| u32::try_from(pid).ok())
		.filter(|pid| *pid != 0)
		.ok_or(DecodeError)?;
	// Discord's original RPC SDK omits activity when Discord_ClearPresence is called.
	let activity = args.remove("activity").unwrap_or(Value::Null);
	let activity = if activity.is_null() {
		None
	} else {
		let fields = activity.as_object().ok_or(DecodeError)?;
		for key in ["assets", "timestamps"] {
			if fields
				.get(key)
				.is_some_and(|v| !v.is_null() && !v.is_object())
			{
				return Err(DecodeError);
			}
		}
		let fields: ActivityFields = serde_json::from_value(activity).map_err(|_| DecodeError)?;
		validate_fields(
			fields.kind,
			&fields.details,
			&fields.state,
			&fields.timestamps,
			&fields.assets,
		)?;
		Some(fields)
	};
	Ok(SetActivity {
		nonce,
		pid,
		activity,
	})
}

/// Every locally supported RPC request. Unsupported commands answer with an error frame
/// instead of silently succeeding, matching Discord's own RPC surface for absent scopes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
	SetActivity(SetActivity),
	/// `INVITE_BROWSER`: the caller asks the client to show an invite. Joining stays user-confirmed.
	Invite {
		nonce: String,
		code: String,
	},
}

pub fn decode_request(bytes: &[u8]) -> Result<Request, DecodeError> {
	let value = payload(bytes)?;
	match value.get("cmd").and_then(Value::as_str) {
		Some("SET_ACTIVITY") => decode_command(bytes).map(Request::SetActivity),
		Some("INVITE_BROWSER") => {
			let nonce = decode_nonce(&value)?;
			let code = value
				.get("args")
				.and_then(Value::as_object)
				.and_then(|args| args.get("code"))
				.and_then(Value::as_str)
				.filter(|code| valid_invite_code(code))
				.ok_or(DecodeError)?
				.to_owned();
			Ok(Request::Invite { nonce, code })
		}
		_ => Err(DecodeError),
	}
}

/// Invite codes travel into an invite lookup, so keep them to the characters Discord issues.
pub fn valid_invite_code(code: &str) -> bool {
	!code.is_empty()
		&& code.len() <= MAX_INVITE_CODE
		&& code
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn decode_nonce(value: &Value) -> Result<String, DecodeError> {
	value
		.get("nonce")
		.and_then(Value::as_str)
		.filter(|nonce| !nonce.is_empty() && text_valid(nonce, 128))
		.map(str::to_owned)
		.ok_or(DecodeError)
}

/// Acknowledges that the invite was handed to the client, never that the user joined.
pub fn invite_acknowledge(nonce: &str, code: &str) -> Vec<u8> {
	json!({"cmd":"INVITE_BROWSER","evt":null,"nonce":nonce,"data":{"code":code}})
		.to_string()
		.into_bytes()
}

pub fn ready(user_id: Id, username: &str) -> Vec<u8> {
	let username: String = username
		.chars()
		.filter(|c| !c.is_control())
		.take(128)
		.collect();
	json!({"cmd":"DISPATCH","evt":"READY","data":{
		"v":1,"config":{"cdn_host":"cdn.discordapp.com","api_endpoint":"//discord.com/api","environment":"production"},
		"user":{"id":user_id,"username":username,"discriminator":"0","avatar":null}
	}}).to_string().into_bytes()
}

/// Acknowledges local acceptance, not confirmation of publication by Discord.
pub fn acknowledge(command: &SetActivity) -> Vec<u8> {
	json!({"cmd":"SET_ACTIVITY","evt":null,"nonce":command.nonce,"data":command.activity})
		.to_string()
		.into_bytes()
}

pub fn error(nonce: Option<&str>) -> Vec<u8> {
	error_response("SET_ACTIVITY", nonce)
}

/// Preserve request correlation even when a game asks for unsupported RPC capabilities.
pub fn error_for_payload(bytes: &[u8]) -> Vec<u8> {
	let value = payload(bytes).ok();
	let command = value
		.as_ref()
		.and_then(|v| v.get("cmd"))
		.and_then(Value::as_str)
		.filter(|cmd| {
			!cmd.is_empty()
				&& cmd.len() <= 64
				&& cmd.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
		})
		.unwrap_or("SET_ACTIVITY");
	let nonce = value
		.as_ref()
		.and_then(|v| v.get("nonce"))
		.and_then(Value::as_str);
	error_response(command, nonce)
}

fn error_response(command: &str, nonce: Option<&str>) -> Vec<u8> {
	let nonce = nonce.filter(|n| !n.is_empty() && text_valid(n, 128));
	json!({"cmd":command,"evt":"ERROR","nonce":nonce,
		"data":{"code":4000,"message":"Unsupported or invalid activity payload"}})
	.to_string()
	.into_bytes()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn command(activity: Value) -> Vec<u8> {
		json!({"cmd":"SET_ACTIVITY","nonce":"synthetic-1","args":{"pid":123,"activity":activity}})
			.to_string()
			.into_bytes()
	}

	#[test]
	fn rich_activity_survives_without_secrets_or_actions() {
		let bytes = command(json!({"details":"Ranked match","state":"Round 2", "type":0,
			"timestamps":{"start":1_700_000_000,"end":1_700_000_030},
			"assets":{"large_image":"map_key","large_text":"Map","small_image":"123","small_text":"Rank"},
			"secrets":{"join":"synthetic-private-secret"},"buttons":[{"label":"Join","url":"https://example.com"}],
			"name":"Spoofed name","application_id":"99"}));
		let decoded = decode_command(&bytes).unwrap();
		let ack = String::from_utf8(acknowledge(&decoded)).unwrap();
		assert!(ack.contains("synthetic-1"));
		assert!(!ack.contains("secret") && !ack.contains("buttons"));
		let activity = decoded
			.activity
			.unwrap()
			.into_activity(Id(42), "Public app name".into())
			.unwrap();
		let output = serde_json::to_value(activity).unwrap();
		assert_eq!(output["name"], "Public app name");
		assert_eq!(output["application_id"], "42");
		assert_eq!(output["details"], "Ranked match");
		assert_eq!(output["timestamps"]["start"], 1_700_000_000_000_u64);
		assert_eq!(output["assets"]["large_image"], "map_key");
		assert!(
			decode_command(&command(Value::Null))
				.unwrap()
				.activity
				.is_none()
		);
		assert!(
			decode_command(br#"{"cmd":"SET_ACTIVITY","nonce":"clear","args":{"pid":123}}"#)
				.unwrap()
				.activity
				.is_none()
		);
		assert_eq!(
			decode_handshake(br#"{"v":1,"client_id":"42"}"#).unwrap(),
			Id(42)
		);
		let ready: Value = serde_json::from_slice(&ready(Id(42), "Synthetic player")).unwrap();
		assert_eq!(ready["evt"], "READY");
		assert_eq!(ready["data"]["v"], 1);
		let millis = decode_command(&command(
			json!({"timestamps":{"start":1_700_000_000_000_u64}}),
		))
		.unwrap()
		.activity
		.unwrap()
		.into_activity(Id(42), "Modern SDK".into())
		.unwrap();
		assert_eq!(millis.timestamps.unwrap().start, Some(1_700_000_000_000));
		let error: Value = serde_json::from_slice(&error_for_payload(
			br#"{"cmd":"SUBSCRIBE","nonce":"subscribe-1","evt":"ACTIVITY_JOIN"}"#,
		))
		.unwrap();
		assert_eq!(error["cmd"], "SUBSCRIBE");
		assert_eq!(error["nonce"], "subscribe-1");
		assert_eq!(error["evt"], "ERROR");
		let error: Value = serde_json::from_slice(&error_for_payload(
			json!({"cmd":"x".repeat(1000),"nonce":"x".repeat(1000)})
				.to_string()
				.as_bytes(),
		))
		.unwrap();
		assert_eq!(error["cmd"], "SET_ACTIVITY");
		assert!(error["nonce"].is_null());
	}

	#[test]
	fn invite_requests_are_bounded_and_other_commands_stay_unsupported() {
		let Request::Invite { nonce, code } = decode_request(
			br#"{"cmd":"INVITE_BROWSER","nonce":"invite-1","args":{"code":"hTKzmak"}}"#,
		)
		.unwrap() else {
			panic!("an invite request must decode as one")
		};
		assert_eq!((nonce.as_str(), code.as_str()), ("invite-1", "hTKzmak"));
		let ack: Value = serde_json::from_slice(&invite_acknowledge(&nonce, &code)).unwrap();
		assert_eq!(ack["cmd"], "INVITE_BROWSER");
		assert_eq!(ack["nonce"], "invite-1");
		assert_eq!(ack["data"]["code"], "hTKzmak");
		assert!(matches!(
			decode_request(&command(json!({}))).unwrap(),
			Request::SetActivity(_)
		));
		for bytes in [
			br#"{"cmd":"INVITE_BROWSER","nonce":"n","args":{"code":"../secret"}}"#.as_slice(),
			br#"{"cmd":"INVITE_BROWSER","nonce":"n","args":{"code":"https://discord.gg/a"}}"#,
			br#"{"cmd":"INVITE_BROWSER","nonce":"n","args":{"code":""}}"#,
			br#"{"cmd":"INVITE_BROWSER","nonce":"n","args":{}}"#,
			br#"{"cmd":"INVITE_BROWSER","args":{"code":"hTKzmak"}}"#,
			br#"{"cmd":"AUTHORIZE","nonce":"n","args":{"scopes":["rpc"]}}"#,
			br#"{"cmd":"GUILD_TEMPLATE_BROWSER","nonce":"n","args":{"code":"hTKzmak"}}"#,
		] {
			assert!(decode_request(bytes).is_err());
		}
		assert!(!valid_invite_code(&"a".repeat(MAX_INVITE_CODE + 1)));
		assert!(valid_invite_code("wumpus-friends_1"));
	}

	#[test]
	fn rejects_malformed_unbounded_and_unsupported_payloads() {
		for bytes in [
			br#"{"v":2,"client_id":"42"}"#.as_slice(),
			br#"{"v":1,"client_id":42}"#,
			br#"{"v":1,"client_id":"0"}"#,
			br#"{"v":1,"client_id":"18446744073709551616"}"#,
			b"[]",
			b"{",
		] {
			assert!(decode_handshake(bytes).is_err());
		}
		assert!(decode_handshake(&vec![b' '; MAX_FRAME_BYTES + 1]).is_err());
		assert!(decode_command(&vec![b' '; MAX_FRAME_BYTES + 1]).is_err());
		for activity in [
			json!([]),
			json!({"type":1}),
			json!({"type":4}),
			json!({"details":"x".repeat(129)}),
			json!({"state":"é".repeat(65)}),
			json!({"details":"bad\ntext"}),
			json!({"timestamps":[]}),
			json!({"assets":[]}),
			json!({"assets":{"large_image":"x".repeat(MAX_ASSET_KEY + 1)}}),
			json!({"timestamps":{"start":u64::MAX}}),
			json!({"timestamps":{"start":-1}}),
			json!({"timestamps":{"start":10,"end":9}}),
		] {
			assert!(decode_command(&command(activity)).is_err());
		}
		for (key, value) in [
			("nonce", json!("")),
			("nonce", json!("x".repeat(129))),
			("nonce", json!(1)),
			("cmd", json!("AUTHORIZE")),
			("args", json!([])),
			("args", json!({"pid":0,"activity":null})),
			("args", json!({"pid":u64::MAX,"activity":null})),
			("args", json!({"activity":null})),
		] {
			// Replace exactly the field under test, keeping all other required fields valid.
			let mut payload: Value = serde_json::from_slice(&command(json!({}))).unwrap();
			payload[key] = value;
			assert!(decode_command(payload.to_string().as_bytes()).is_err());
		}
	}
}
