//! Spotify Web API playback projected into an unofficial Discord listening presence.
//! Wire reference: discord.py-self/discord/activity.py Spotify; session_id is receive-only.
use crate::{
	DecodeError,
	rpc::{Assets, Timestamps},
};
use model::{ActivityImage, Id, RichActivity};
use serde::{Deserialize, Serialize};

pub const MAX_PLAYBACK_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Activity {
	name: &'static str,
	#[serde(rename = "type")]
	kind: u8,
	pub details: String,
	pub state: String,
	pub sync_id: String,
	pub timestamps: Timestamps,
	pub assets: Assets,
	party: Party,
	flags: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Party {
	id: String,
}

impl Activity {
	pub fn validate(&self) -> Result<(), DecodeError> {
		if self.name != "Spotify"
			|| self.kind != 2
			|| self.flags != 48
			|| self.details.len() > 512
			|| self.state.len() > 512
			|| !spotify_id(&self.sync_id)
			|| self
				.party
				.id
				.strip_prefix("spotify:")
				.and_then(|id| id.parse::<Id>().ok())
				.is_none()
			|| !self.display().valid()
			|| self.assets.small_image.is_some()
			|| self.assets.small_text.is_some()
			|| self
				.assets
				.large_text
				.as_ref()
				.is_some_and(|s| text(s).as_deref() != Some(s.as_str()))
			|| self
				.assets
				.large_image
				.as_ref()
				.is_some_and(|s| image(s).is_none())
			|| !matches!((self.timestamps.start, self.timestamps.end), (Some(start), Some(end)) if end > start)
		{
			return Err(DecodeError);
		}
		Ok(())
	}

	pub fn display(&self) -> RichActivity {
		RichActivity {
			kind: 2,
			name: "Spotify".into(),
			details: Some(self.details.clone()),
			state: Some(self.state.clone()),
			image: self.assets.large_image.as_deref().and_then(image),
			small_image: None,
			started_at: self.timestamps.start,
			ends_at: self.timestamps.end,
		}
	}
}

fn spotify_id(id: &str) -> bool {
	id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn image(key: &str) -> Option<ActivityImage> {
	if key.len() != 48 {
		return None;
	}
	let image = ActivityImage::Spotify(key.strip_prefix("spotify:")?.to_owned());
	image.valid().then_some(image)
}

fn text(value: &str) -> Option<String> {
	let value: String = value
		.chars()
		.filter(|c| !c.is_control())
		.take(128)
		.collect();
	let value = value.trim();
	(!value.is_empty()).then(|| value.to_owned())
}

#[derive(Deserialize)]
struct Playback<'a> {
	is_playing: bool,
	device: Option<Device>,
	progress_ms: Option<u64>,
	currently_playing_type: String,
	#[serde(borrow)]
	item: Option<&'a serde_json::value::RawValue>,
}
#[derive(Deserialize)]
struct Device {
	is_private_session: bool,
}
#[derive(Deserialize)]
struct Track {
	id: Option<String>,
	name: String,
	duration_ms: u64,
	is_local: bool,
	#[serde(rename = "type")]
	kind: String,
	artists: Vec<Artist>,
	album: Album,
}
#[derive(Deserialize)]
struct Artist {
	name: String,
}
#[derive(Deserialize)]
struct Album {
	name: String,
	images: Vec<Image>,
}
#[derive(Deserialize)]
struct Image {
	url: String,
}

/// Only ordinary public tracks are shared. Paused, private, local, ad and episode playback clears.
pub fn decode_playback(
	bytes: &[u8],
	user: Id,
	now_ms: u64,
) -> Result<Option<Activity>, DecodeError> {
	if bytes.len() > MAX_PLAYBACK_BYTES || user.0 == 0 || now_ms > model::MAX_ACTIVITY_TIMESTAMP {
		return Err(DecodeError);
	}
	let playback: Playback<'_> = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	if !playback.is_playing
		|| playback.currently_playing_type != "track"
		|| playback
			.device
			.is_none_or(|device| device.is_private_session)
	{
		return Ok(None);
	}
	let (Some(item), Some(progress)) = (playback.item, playback.progress_ms) else {
		return Ok(None);
	};
	let track: Track = serde_json::from_str(item.get()).map_err(|_| DecodeError)?;
	if track.is_local || track.kind != "track" {
		return Ok(None);
	}
	if track.artists.len() > 64
		|| track.album.images.len() > 8
		|| track.duration_ms > 24 * 60 * 60 * 1000
	{
		return Err(DecodeError);
	}
	let Some(id) = track.id.filter(|id| spotify_id(id)) else {
		return Ok(None);
	};
	if progress >= track.duration_ms {
		return Ok(None);
	}
	let start = now_ms.checked_sub(progress).ok_or(DecodeError)?;
	let end = start.checked_add(track.duration_ms).ok_or(DecodeError)?;
	let artists = track
		.artists
		.iter()
		.take(5)
		.map(|a| a.name.replace(';', ""))
		.collect::<Vec<_>>()
		.join("; ");
	let activity = Activity {
		name: "Spotify",
		kind: 2,
		flags: 48,
		details: text(&track.name).ok_or(DecodeError)?,
		state: text(&artists).ok_or(DecodeError)?,
		sync_id: id,
		timestamps: Timestamps {
			start: Some(start),
			end: Some(end),
		},
		party: Party {
			id: format!("spotify:{user}"),
		},
		assets: Assets {
			large_text: text(&track.album.name),
			large_image: track.album.images.iter().find_map(|art| {
				let id = art.url.strip_prefix("https://i.scdn.co/image/")?;
				let key = format!("spotify:{id}");
				image(&key).map(|_| key)
			}),
			..Assets::default()
		},
	};
	activity.validate()?;
	Ok(Some(activity))
}
