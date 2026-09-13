//! Fresh version-guarded account preference writes, confirmed from the response.
use crate::{DiscordApi, Failure};
use discord_protocol::messaging_permissions::{self as wire, MAX_SETTINGS_RESPONSE};
use model::messaging_permissions::{Change, Snapshot};
use reqwest::Method;
impl DiscordApi {
	pub async fn account_messaging_permissions(
		&self,
		change: Option<Change>,
	) -> Result<Snapshot, Failure> {
		if change.as_ref().is_some_and(|v| !v.valid()) {
			return Err(Failure::Protocol);
		}
		let unavailable =
			Failure::ProtocolAt("Discord messaging permissions are unavailable or unsupported");
		let unconfirmed =
			Failure::ProtocolAt("Messaging permissions were not confirmed; reload before retrying");
		let bytes = self
			.request_limited(
				Method::GET,
				"/users/@me/settings-proto/1",
				None,
				MAX_SETTINGS_RESPONSE,
			)
			.await?;
		let current = wire::decode_response(&bytes).map_err(|_| unavailable)?;
		let Some(change) = change else {
			return Ok(current.snapshot);
		};
		let mut wanted = current.snapshot.clone();
		change.apply(&mut wanted);
		if !wanted.valid() {
			return Err(Failure::Protocol);
		}
		if wanted == current.snapshot {
			return Ok(wanted);
		}
		let patch = wire::encode_patch(&current, &change).map_err(|_| Failure::Protocol)?;
		let bytes = self
			.request_limited(
				Method::PATCH,
				"/users/@me/settings-proto/1",
				Some(serde_json::json!({"settings":patch,"required_data_version":current.version})),
				MAX_SETTINGS_RESPONSE,
			)
			.await?;
		let saved = wire::decode_response(&bytes).map_err(|_| unconfirmed)?;
		if saved.snapshot != wanted {
			return Err(unconfirmed);
		}
		Ok(saved.snapshot)
	}
}
