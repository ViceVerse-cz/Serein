use crate::{DiscordApi, Failure};
use client_core::server_settings::Event;
use discord_protocol::server_settings::{self as wire, MAX_SETTINGS_WIRE};
use model::{
	Id, Patch,
	server_settings::{Edit, Settings},
};
use wreq::Method;

impl DiscordApi {
	async fn load_server_settings(&self, guild: Id) -> Result<Box<Settings>, Failure> {
		if guild.0 == 0 {
			return Err(Failure::Protocol);
		}
		let metadata = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}?with_counts=true"),
				None,
				MAX_SETTINGS_WIRE,
			)
			.await?;
		let profile = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/profile"),
				None,
				MAX_SETTINGS_WIRE,
			)
			.await?;
		wire::decode_settings(guild, &metadata, &profile)
			.map(Box::new)
			.map_err(|_| Failure::ProtocolAt("Server settings response was unsupported"))
	}
	pub(super) async fn server_settings(
		&self,
		guild: Id,
		request: u64,
		edit: Option<Box<Edit>>,
	) -> Event {
		let writing = edit.is_some();
		let mut result = self.update_server_settings(guild, edit.as_deref()).await;
		let refreshed = if writing
			&& result
				.as_ref()
				.is_err_and(|failure| !failure.ends_session())
		{
			match self.load_server_settings(guild).await {
				Ok(snapshot) => Some(snapshot),
				Err(failure) if failure.ends_session() => {
					result = Err(failure);
					None
				}
				Err(_) => None,
			}
		} else {
			None
		};
		Event {
			guild,
			request,
			result,
			refreshed,
		}
	}
	async fn update_server_settings(
		&self,
		guild: Id,
		edit: Option<&Edit>,
	) -> Result<Box<Settings>, Failure> {
		if edit.is_some_and(|edit| !edit.valid() || edit.is_empty()) {
			return Err(Failure::Protocol);
		}
		let latest = self.load_server_settings(guild).await?;
		let Some(edit) = edit else {
			return Ok(latest);
		};
		let (profile, metadata) =
			wire::encode_edit(edit, &latest).map_err(|_| Failure::Protocol)?;
		let mut written = false;
		for (suffix, body) in [("/profile", profile), ("", metadata)] {
			if body.as_object().is_some_and(|object| object.is_empty()) {
				continue;
			}
			// The two routes are not transactional. A later failure must preserve the draft
			// and refetch both resources, never resend the already completed write.
			let bytes = self
				.request_limited(
					Method::PATCH,
					&format!("/guilds/{guild}{suffix}"),
					Some(body),
					MAX_SETTINGS_WIRE,
				)
				.await
				.map_err(|failure| {
					if failure.ends_session() {
						failure
					} else if written {
						Failure::ProtocolAt(
							"Some settings may have saved; review the refreshed server before retrying",
						)
					} else if failure == Failure::Capacity {
						Failure::Ambiguous
					} else {
						failure
					}
				})?;
			written = true;
			wire::confirm_guild(&bytes, guild).map_err(|_| Failure::Ambiguous)?;
		}
		let saved = self.load_server_settings(guild).await.map_err(|failure| {
			if failure.ends_session() {
				failure
			} else {
				Failure::Ambiguous
			}
		})?;
		if !matches_edit(edit, &saved) {
			return Err(Failure::ProtocolAt(
				"Server settings changed while saving; review the refreshed values",
			));
		}
		Ok(saved)
	}
}
fn matches_edit(edit: &Edit, saved: &Settings) -> bool {
	let mut expected = saved.clone();
	edit.apply(&mut expected);
	let icon_matches = match &edit.icon {
		Patch::Absent => true,
		Patch::Null => saved.icon.is_none(),
		Patch::Value(_) => saved.icon.is_some(),
	};
	// Feature order is not meaningful; Edit::apply normalizes just the activity pair.
	expected.features.clone_from(&saved.features);
	icon_matches && expected == *saved
}
