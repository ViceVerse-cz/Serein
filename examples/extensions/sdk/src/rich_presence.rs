//! An account-scoped activity contribution, retired when its extension is disabled.
use serde::{Deserialize, Serialize};

pub const MAX_CUSTOM_PRESENCE_BYTES: usize = 3072;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RichPresenceUpdate {
	Set { presence: Box<CustomRichPresence> },
	Clear,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CustomRichPresence {
	pub application_id: String,
	pub name: String,
	pub kind: RichPresenceKind,
	pub stream_url: Option<String>,
	pub details: Option<String>,
	pub details_url: Option<String>,
	pub state: Option<String>,
	pub state_url: Option<String>,
	pub large_image: Option<RichPresenceImage>,
	pub small_image: Option<RichPresenceImage>,
	pub buttons: Vec<RichPresenceButton>,
	pub party: Option<RichPresenceParty>,
	pub timer: RichPresenceTimer,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RichPresenceKind {
	#[default]
	Playing,
	Streaming,
	Listening,
	Watching,
	Competing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RichPresenceImage {
	pub key: String,
	#[serde(default)]
	pub text: Option<String>,
	#[serde(default)]
	pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RichPresenceButton {
	pub label: String,
	pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RichPresenceParty {
	pub current: u32,
	pub max: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum RichPresenceTimer {
	#[default]
	None,
	Elapsed,
	LocalDay,
	Custom {
		start: Option<u64>,
		end: Option<u64>,
	},
}

impl CustomRichPresence {
	pub fn validate(&self) -> Result<(), &'static str> {
		if self.application_id.is_empty()
			|| self.application_id.len() > 20
			|| !self.application_id.bytes().all(|c| c.is_ascii_digit())
			|| !self.application_id.parse::<u64>().is_ok_and(|id| id != 0)
		{
			return Err("Application ID must be a nonzero decimal Discord application ID");
		}
		text(&self.name, 128).map_err(
			|_| "Activity name is required and must fit 128 UTF-8 bytes without control characters",
		)?;
		for value in [&self.details, &self.state].into_iter().flatten() {
			text(value, 128)?;
		}
		for value in [&self.stream_url, &self.details_url, &self.state_url]
			.into_iter()
			.flatten()
		{
			link(value)?;
		}
		if self.kind == RichPresenceKind::Streaming && self.stream_url.is_none() {
			return Err("Streaming activities need an HTTPS stream URL");
		}
		if self.kind != RichPresenceKind::Streaming && self.stream_url.is_some() {
			return Err("Stream URL is available only for Streaming activities");
		}
		if self.details_url.is_some() && self.details.is_none() {
			return Err("Add details text before setting its link");
		}
		if self.state_url.is_some() && self.state.is_none() {
			return Err("Add state text before setting its link");
		}
		for image in [&self.large_image, &self.small_image].into_iter().flatten() {
			if image.key.starts_with("https://") {
				if image.key.len() > 1024 {
					return Err("Artwork URLs must fit 1024 UTF-8 bytes");
				}
				link(&image.key)?;
			} else {
				text(&image.key, 256)?;
				if image.key.contains([':', '/', '\\']) {
					return Err("Artwork must be an application asset key or an HTTPS image URL");
				}
			}
			if let Some(value) = &image.text {
				text(value, 128)?;
			}
			if let Some(value) = &image.url {
				link(value)?;
			}
		}
		if self.buttons.len() > 2 {
			return Err("An activity can have at most two buttons");
		}
		for button in &self.buttons {
			text(&button.label, 32).map_err(
				|_| "Button labels must contain 1 to 32 UTF-8 bytes without control characters",
			)?;
			link(&button.url)?;
		}
		if self.party.as_ref().is_some_and(|party| {
			party.current == 0 || party.max == 0 || party.current > party.max || party.max > 9999
		}) {
			return Err(
				"Party size must be 1 to 9999, with current members no greater than capacity",
			);
		}
		if let RichPresenceTimer::Custom { start, end } = self.timer {
			// Milliseconds through year 9999, safely representable by JavaScript and chrono.
			if start.is_none() && end.is_none() {
				return Err("Custom timer needs a start or end timestamp");
			}
			if start
				.into_iter()
				.chain(end)
				.any(|value| value == 0 || value > 253_402_300_799_999)
			{
				return Err(
					"Timer timestamps must be positive Unix milliseconds before year 10000",
				);
			}
			if start.zip(end).is_some_and(|(start, end)| end <= start) {
				return Err("Timer end must be later than its start");
			}
		}
		if serde_json::to_vec(self)
			.map_err(|_| "Activity could not be encoded")?
			.len() > MAX_CUSTOM_PRESENCE_BYTES
		{
			return Err(
				"Activity exceeds 3 KiB in total; shorten its links or remove optional fields",
			);
		}
		Ok(())
	}
}

fn text(value: &str, max: usize) -> Result<(), &'static str> {
	if value.len() > max {
		return Err("Activity value exceeds its size limit");
	}
	if value.trim().is_empty() || value.chars().any(char::is_control) {
		return Err("Activity value is empty or contains invalid characters");
	}
	Ok(())
}

fn link(value: &str) -> Result<(), &'static str> {
	if value.len() > 2048 {
		return Err("Links must fit 2048 UTF-8 bytes");
	}
	if !https_url(value)
		|| value.chars().any(char::is_whitespace)
		|| value.chars().any(char::is_control)
	{
		return Err(
			"Links must use HTTPS and cannot contain credentials, spaces or control characters",
		);
	}
	Ok(())
}

// The host revalidates links with its URL parser before accepting an update.
fn https_url(value: &str) -> bool {
	let Some(rest) = value.strip_prefix("https://") else {
		return false;
	};
	let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
	!authority.is_empty() && !authority.contains(['@', '\\']) && !value.contains('\\')
}

/// Opt-in presence output preserves existing SDK Output struct literals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RichPresenceOutput {
	#[serde(flatten)]
	pub output: crate::Output,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rich_presence: Option<RichPresenceUpdate>,
}
