//! Soundboard routes.
//!
//! Protocol status: documented. The default-sounds route, the guild catalog routes and
//! `POST /channels/{id}/send-soundboard-sound` are published in Discord's API documentation.
//! Sending from a user session rather than an application is unverified.
use crate::{DiscordApi, Failure};
use client_core::soundboard;
use model::Id;
use wreq::Method;

/// Response budget for one guild's catalog. Names are short and the count is bounded, so this
/// leaves generous headroom over the catalog's retained byte budget.
const MAX_CATALOG_WIRE: usize = 256 * 1024;

impl DiscordApi {
	// https://docs.discord.com/developers/resources/soundboard#list-default-soundboard-sounds
	pub(crate) async fn default_soundboard_sounds(
		&self,
	) -> Result<Vec<model::SoundboardSound>, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				"/soundboard-default-sounds",
				None,
				MAX_CATALOG_WIRE,
			)
			.await?;
		discord_protocol::soundboard::default_sounds(&bytes).map_err(|_| Failure::Protocol)
	}

	// https://docs.discord.com/developers/resources/soundboard#list-guild-soundboard-sounds
	pub(crate) async fn guild_soundboard_sounds(
		&self,
		guild: Id,
	) -> Result<Vec<model::SoundboardSound>, Failure> {
		if guild.0 == 0 {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/soundboard-sounds"),
				None,
				MAX_CATALOG_WIRE,
			)
			.await?;
		discord_protocol::soundboard::guild_sounds(&bytes, guild).map_err(|_| Failure::Protocol)
	}

	/// Triggers a sound for everyone in `channel`.
	///
	/// `source_guild` is required when playing a sound that belongs to another guild, and must be
	/// omitted for a default sound.
	// https://docs.discord.com/developers/resources/soundboard#send-soundboard-sound
	pub(crate) async fn send_soundboard_sound(
		&self,
		channel: Id,
		sound: Id,
		source_guild: Option<Id>,
	) -> Result<(), Failure> {
		if channel.0 == 0 || sound.0 == 0 || source_guild.is_some_and(|id| id.0 == 0) {
			return Err(Failure::Protocol);
		}
		let mut body = serde_json::json!({ "sound_id": sound.to_string() });
		if let Some(guild) = source_guild {
			body["source_guild_id"] = serde_json::Value::String(guild.to_string());
		}
		self.request(
			Method::POST,
			&format!("/channels/{channel}/send-soundboard-sound"),
			Some(body),
		)
		.await
		.map(|_| ())
	}

	/// Uploads a new sound to a guild's catalog.
	///
	/// The server decides whether the catalog has room; this only enforces the documented byte
	/// and name limits so an obviously invalid upload does not leave the client.
	// https://docs.discord.com/developers/resources/soundboard#create-guild-soundboard-sound
	pub(crate) async fn create_soundboard_sound(
		&self,
		guild: Id,
		request: soundboard::Upload,
	) -> Result<model::SoundboardSound, Failure> {
		if guild.0 == 0 || !request.valid() {
			return Err(Failure::Protocol);
		}
		let mut body = serde_json::json!({
			"name": request.name.trim(),
			"sound": discord_protocol::soundboard::sound_data_uri(
				request.content_type,
				&request.audio,
			),
			"volume": f64::from(request.volume),
		});
		match (request.emoji_id, &request.emoji_name) {
			(Some(id), _) => body["emoji_id"] = serde_json::Value::String(id.to_string()),
			(None, Some(name)) => {
				body["emoji_name"] = serde_json::Value::String(name.clone());
			}
			(None, None) => {}
		}
		let bytes = self
			.request_limited(
				Method::POST,
				&format!("/guilds/{guild}/soundboard-sounds"),
				Some(body),
				MAX_CATALOG_WIRE,
			)
			.await?;
		discord_protocol::soundboard::guild_sound(&bytes, guild).map_err(|_| Failure::Protocol)
	}

	// https://docs.discord.com/developers/resources/soundboard#delete-guild-soundboard-sound
	pub(crate) async fn delete_soundboard_sound(
		&self,
		guild: Id,
		sound: Id,
	) -> Result<(), Failure> {
		if guild.0 == 0 || sound.0 == 0 {
			return Err(Failure::Protocol);
		}
		self.request(
			Method::DELETE,
			&format!("/guilds/{guild}/soundboard-sounds/{sound}"),
			None,
		)
		.await
		.map(|_| ())
	}
}
