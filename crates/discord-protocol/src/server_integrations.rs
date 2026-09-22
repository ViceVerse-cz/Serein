//! Documented guild integration and authenticated webhook management responses.
use crate::{DecodeError, Timestamp, UserDto, permissions::List};
use model::{
	Id,
	server_integrations::{self as m, Application, Integration, Snapshot, Source, Webhook},
};
use serde::Deserialize;
pub const MAX_WIRE: usize = 2 * 1024 * 1024;
#[derive(Deserialize)]
struct WireApplication {
	id: Id,
	name: String,
	icon: Option<String>,
	description: String,
	bot: Option<UserDto>,
}
#[derive(Deserialize)]
struct WireIntegration {
	id: Id,
	name: String,
	#[serde(rename = "type")]
	kind: String,
	enabled: bool,
	user: Option<UserDto>,
	synced_at: Option<Timestamp>,
	role_id: Option<Id>,
	application: Option<WireApplication>,
}
impl WireIntegration {
	fn into_model(self) -> Integration {
		Integration {
			id: self.id,
			name: self.name,
			kind: self.kind,
			enabled: self.enabled,
			user: self.user.map(UserDto::into_model),
			synced_at: self.synced_at.map(|time| time.0),
			role_id: self.role_id,
			application: self.application.map(|app| Application {
				id: app.id,
				name: app.name,
				icon: app.icon,
				description: app.description,
				bot: app.bot.map(UserDto::into_model),
			}),
		}
	}
}
#[derive(Deserialize)]
struct WireSource {
	id: Id,
	name: Option<String>,
}
#[derive(Deserialize)]
struct WireWebhook {
	id: Id,
	guild_id: Id,
	channel_id: Option<Id>,
	#[serde(rename = "type")]
	kind: u8,
	name: Option<String>,
	avatar: Option<String>,
	application_id: Option<Id>,
	user: Option<UserDto>,
	source_guild: Option<WireSource>,
	source_channel: Option<WireSource>,
	// token and url intentionally have no fields: serde skips them without allocating strings.
}
impl WireWebhook {
	fn into_model(self) -> Webhook {
		Webhook {
			id: self.id,
			guild: self.guild_id,
			channel: self.channel_id,
			kind: self.kind,
			name: self.name,
			avatar: self.avatar,
			application_id: self.application_id,
			user: self.user.map(UserDto::into_model),
			source_guild: self.source_guild.map(|source| Source {
				id: source.id,
				name: source.name,
			}),
			source_channel: self.source_channel.map(|source| Source {
				id: source.id,
				name: source.name,
			}),
		}
	}
}
pub fn integrations(bytes: &[u8], guild: Id) -> Result<Snapshot, DecodeError> {
	if bytes.len() > MAX_WIRE {
		return Err(DecodeError);
	}
	let values: List<WireIntegration, { m::MAX_INTEGRATIONS }> = crate::decode(bytes)?;
	let page = Snapshot {
		guild,
		channel: None,
		integrations: Some(
			values
				.0
				.into_iter()
				.map(WireIntegration::into_model)
				.collect(),
		),
		webhooks: None,
	};
	if !page.valid() {
		return Err(DecodeError);
	}
	Ok(page)
}
pub fn webhooks(bytes: &[u8], guild: Id) -> Result<Snapshot, DecodeError> {
	if bytes.len() > MAX_WIRE {
		return Err(DecodeError);
	}
	let values: List<WireWebhook, { m::MAX_WEBHOOKS }> = crate::decode(bytes)?;
	let page = Snapshot {
		guild,
		channel: None,
		integrations: None,
		webhooks: Some(values.0.into_iter().map(WireWebhook::into_model).collect()),
	};
	if !page.valid() {
		return Err(DecodeError);
	}
	Ok(page)
}
pub fn webhook(bytes: &[u8], guild: Id) -> Result<Webhook, DecodeError> {
	if bytes.len() > 64 * 1024 {
		return Err(DecodeError);
	}
	let value: WireWebhook = crate::decode(bytes)?;
	let value = value.into_model();
	if value.guild != guild || !value.valid() {
		return Err(DecodeError);
	}
	Ok(value)
}
pub fn channel_scope(bytes: &[u8], guild: Id, channel: Id) -> Result<(), DecodeError> {
	#[derive(Deserialize)]
	struct Scope {
		id: Id,
		guild_id: Id,
		#[serde(rename = "type")]
		kind: u8,
	}
	if bytes.len() > 64 * 1024 {
		return Err(DecodeError);
	}
	let scope: Scope = crate::decode(bytes)?;
	if scope.id != channel || scope.guild_id != guild || !matches!(scope.kind, 0 | 5 | 15 | 16) {
		return Err(DecodeError);
	}
	Ok(())
}
/// Only the explicit copy request decodes a token. Borrowing prevents extra secret allocations.
pub fn webhook_url(
	bytes: &[u8],
	guild: Id,
	webhook: Id,
	channel: Id,
) -> Result<m::WebhookUrl, DecodeError> {
	#[derive(Deserialize)]
	struct CopyResponse<'a> {
		id: Id,
		guild_id: Id,
		channel_id: Id,
		#[serde(rename = "type")]
		kind: u8,
		#[serde(borrow)]
		token: &'a str,
	}
	if bytes.len() > 64 * 1024 {
		return Err(DecodeError);
	}
	let value: CopyResponse<'_> = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	if value.id != webhook
		|| value.guild_id != guild
		|| value.channel_id != channel
		|| value.kind != 1
	{
		return Err(DecodeError);
	}
	m::WebhookUrl::new(guild, webhook, channel, value.token).ok_or(DecodeError)
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;
	#[test]
	fn integrations_metadata_bounds_and_unknown_fields() {
		let item = json!({"id":"1","name":"Example","type":"discord","enabled":true,"user":{"id":"4","username":"Owner"},"application":{"id":"5","name":"Example","icon":null,"description":"Useful tools","bot":{"id":"6","username":"Bot","bot":true}}});
		let page = integrations(&serde_json::to_vec(&vec![item.clone()]).unwrap(), Id(2)).unwrap();
		let value = &page.integrations.as_ref().unwrap()[0];
		assert!(value.synced_at.is_none());
		assert_eq!(
			value.application.as_ref().unwrap().bot.as_ref().unwrap().id,
			Id(6)
		);
		assert!(page.webhooks.is_none());
		assert!(
			integrations(&serde_json::to_vec(&vec![item.clone(); 51]).unwrap(), Id(2)).is_err()
		);
		assert!(integrations(&serde_json::to_vec(&vec![item.clone(); 2]).unwrap(), Id(2)).is_err());
		let mut bad = item;
		bad["application"]["description"] = json!("x".repeat(4097));
		assert!(integrations(&serde_json::to_vec(&vec![bad]).unwrap(), Id(2)).is_err());
		assert!(webhooks(&vec![b' '; MAX_WIRE + 1], Id(2)).is_err());
		let mut page = page;
		page.integrations.as_mut().unwrap().reserve(m::MAX_BYTES);
		assert!(!page.valid());
		let result = model::server_admin::Result::Integrations(page);
		assert!(result.bytes() > m::MAX_BYTES);
		assert!(!result.valid());
	}
	#[test]
	fn integrations_webhooks_discard_secrets_and_validate_scope() {
		let item = json!({"id":"3","guild_id":"2","channel_id":"4","type":1,"name":"Hook","token":"SYNTHETIC_SECRET".repeat(3000),"url":"SYNTHETIC_EXECUTION_URL"});
		let page = webhooks(&serde_json::to_vec(&vec![item.clone()]).unwrap(), Id(2)).unwrap();
		assert!(page.bytes() < 2 * 1024);
		assert!(page.integrations.is_none());
		assert!(webhooks(&serde_json::to_vec(&vec![item.clone()]).unwrap(), Id(8)).is_err());
		assert!(webhooks(&serde_json::to_vec(&vec![item; 2]).unwrap(), Id(2)).is_err());
		let followed = webhook(br#"{"id":"7","guild_id":"2","channel_id":"4","type":2,"source_channel":{"id":"8","name":"news"}}"#, Id(2)).unwrap();
		assert!(followed.source_guild.is_none());
		assert_eq!(
			followed.source_channel.unwrap().name.as_deref(),
			Some("news")
		);
		assert!(channel_scope(br#"{"id":"4","guild_id":"9","type":0}"#, Id(2), Id(4)).is_err());
		assert!(channel_scope(br#"{"id":"4","guild_id":"2","type":11}"#, Id(2), Id(4)).is_err());
		assert!(!m::valid_webhook_name("DisCord updates"));
		assert!(!m::valid_webhook_name("\n"));
		assert!(m::valid_webhook_name("Build updates"));
	}
}

#[cfg(test)]
mod copy_tests {
	use super::*;
	#[test]
	fn webhook_url_is_scoped_bounded_and_redacted() {
		let mut value = serde_json::json!({"id":"4", "guild_id":"2", "channel_id":"3", "type":1, "token":"synthetic_Token-123", "url":"https://untrusted.invalid"});
		let parse = |value: &serde_json::Value| {
			webhook_url(value.to_string().as_bytes(), Id(2), Id(4), Id(3))
		};
		let url = parse(&value).unwrap();
		assert_eq!(
			url.expose(),
			"https://discord.com/api/webhooks/4/synthetic_Token-123"
		);
		assert_eq!(format!("{url:?}"), "WebhookUrl([REDACTED])");
		for field in ["id", "guild_id", "channel_id"] {
			let mut wrong = value.clone();
			wrong[field] = "99".into();
			assert!(parse(&wrong).is_err());
		}
		value["type"] = 2.into();
		assert!(parse(&value).is_err());
		value["type"] = 1.into();
		for token in [
			"".to_owned(),
			"x".repeat(257),
			"secret/path".into(),
			"secret?query".into(),
			"secret#fragment".into(),
			"secret\n".into(),
			"?".into(),
		] {
			value["token"] = token.into();
			assert!(parse(&value).is_err());
		}
		value.as_object_mut().unwrap().remove("token");
		assert!(parse(&value).is_err());
		assert!(webhook_url(&vec![b' '; 65537], Id(2), Id(4), Id(3)).is_err());
	}
}
