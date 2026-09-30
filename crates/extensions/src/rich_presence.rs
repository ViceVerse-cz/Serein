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
	pub fn validate(&self) -> Result<(), crate::Error> {
		use crate::Error;
		if self.application_id.is_empty()
			|| self.application_id.len() > 20
			|| !self.application_id.bytes().all(|c| c.is_ascii_digit())
			|| !self.application_id.parse::<u64>().is_ok_and(|id| id != 0)
		{
			return Err(Error::Invalid);
		}
		text(&self.name, 128)?;
		for value in [&self.details, &self.state].into_iter().flatten() {
			text(value, 128)?;
		}
		for value in [&self.stream_url, &self.details_url, &self.state_url]
			.into_iter()
			.flatten()
		{
			link(value)?;
		}
		if (self.kind == RichPresenceKind::Streaming) != self.stream_url.is_some()
			|| self.details_url.is_some() && self.details.is_none()
			|| self.state_url.is_some() && self.state.is_none()
		{
			return Err(Error::Invalid);
		}
		for image in [&self.large_image, &self.small_image].into_iter().flatten() {
			if image.key.starts_with("https://") {
				if image.key.len() > 1024 {
					return Err(Error::Limit);
				}
				link(&image.key)?;
			} else {
				text(&image.key, 256)?;
				if image.key.contains([':', '/', '\\']) {
					return Err(Error::Invalid);
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
			return Err(Error::Limit);
		}
		for button in &self.buttons {
			text(&button.label, 32)?;
			link(&button.url)?;
		}
		if self.party.as_ref().is_some_and(|party| {
			party.current == 0 || party.max == 0 || party.current > party.max || party.max > 9999
		}) {
			return Err(Error::Invalid);
		}
		if let RichPresenceTimer::Custom { start, end } = self.timer {
			// Milliseconds through year 9999, safely representable by JavaScript and chrono.
			if start.is_none() && end.is_none()
				|| start
					.into_iter()
					.chain(end)
					.any(|value| value == 0 || value > 253_402_300_799_999)
				|| start.zip(end).is_some_and(|(start, end)| end <= start)
			{
				return Err(Error::Invalid);
			}
		}
		if serde_json::to_vec(self).map_err(|_| Error::Invalid)?.len() > MAX_CUSTOM_PRESENCE_BYTES {
			return Err(Error::Limit);
		}
		Ok(())
	}
}

fn text(value: &str, max: usize) -> Result<(), crate::Error> {
	if value.len() > max {
		return Err(crate::Error::Limit);
	}
	if value.trim().is_empty() || value.chars().any(char::is_control) {
		return Err(crate::Error::Invalid);
	}
	Ok(())
}

fn link(value: &str) -> Result<(), crate::Error> {
	if value.len() > 2048 {
		return Err(crate::Error::Limit);
	}
	if !crate::valid_https_url(value)
		|| value.contains('\\')
		|| value.chars().any(char::is_whitespace)
		|| value.chars().any(char::is_control)
	{
		return Err(crate::Error::Invalid);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Action, Capability, ExtensionKind, Invocation, Manifest, Output, Surface};

	fn presence() -> CustomRichPresence {
		CustomRichPresence {
			application_id: "123456789".into(),
			name: "Building Serein".into(),
			..Default::default()
		}
	}

	#[test]
	fn rich_presence_total_budget_includes_all_fields_and_matches_sdk() {
		let mut activity = presence();
		activity.buttons = vec![
			RichPresenceButton {
				label: "Project".into(),
				url: "https://example.com/".into(),
			};
			2
		];
		let remaining = MAX_CUSTOM_PRESENCE_BYTES - serde_json::to_vec(&activity).unwrap().len();
		activity.buttons[0].url.push_str(&"a".repeat(remaining / 2));
		activity.buttons[1]
			.url
			.push_str(&"a".repeat(remaining - remaining / 2));
		assert_eq!(
			serde_json::to_vec(&activity).unwrap().len(),
			MAX_CUSTOM_PRESENCE_BYTES
		);
		let sdk: serein_extension_sdk::CustomRichPresence =
			serde_json::from_value(serde_json::to_value(&activity).unwrap()).unwrap();
		assert!(activity.validate().is_ok());
		assert!(sdk.validate().is_ok());
		activity.buttons[1].url.push('a');
		let sdk: serein_extension_sdk::CustomRichPresence =
			serde_json::from_value(serde_json::to_value(&activity).unwrap()).unwrap();
		assert!(matches!(activity.validate(), Err(crate::Error::Limit)));
		assert!(sdk.validate().unwrap_err().contains("3 KiB"));
	}

	#[test]
	fn rich_presence_bounds_links_and_timer_are_validated() {
		let mut activity = presence();
		assert!(activity.validate().is_ok());
		for value in ["", "0", "+123", "18446744073709551616"] {
			activity.application_id = value.into();
			assert!(activity.validate().is_err(), "{value}");
		}
		activity = presence();
		activity.details = Some("Details".into());
		for value in [
			"http://example.com",
			"https://user:pass@example.com",
			"https://example.com/\n",
			"https://example.com\\foo",
		] {
			activity.details_url = Some(value.into());
			assert!(activity.validate().is_err(), "{value:?}");
		}
		activity.details_url = Some("https://example.com/details".into());
		activity.large_image = Some(RichPresenceImage {
			key: "https://example.com/image.png".into(),
			text: Some("Artwork".into()),
			url: None,
		});
		activity.buttons = vec![
			RichPresenceButton {
				label: "View project".into(),
				url: "https://example.com".into()
			};
			2
		];
		activity.party = Some(RichPresenceParty { current: 1, max: 4 });
		activity.timer = RichPresenceTimer::Custom {
			start: Some(1_700_000_000_000),
			end: Some(1_700_000_001_000),
		};
		assert!(activity.validate().is_ok());
		activity.buttons.push(activity.buttons[0].clone());
		assert!(activity.validate().is_err());
		activity.buttons.pop();
		activity.timer = RichPresenceTimer::Custom {
			start: Some(1000),
			end: Some(1000),
		};
		assert!(activity.validate().is_err());
		activity.timer = RichPresenceTimer::None;
		activity.name = "a".repeat(129);
		assert!(activity.validate().is_err());
	}

	#[test]
	fn rich_presence_is_opt_in_foreground_or_activation_and_sdk_wire_compatible() {
		let mut manifest = Manifest {
			api_version: crate::API_VERSION,
			id: "custom-rpc".into(),
			name: "Custom RPC".into(),
			version: "1.0.0".into(),
			author: "Serein".into(),
			license: "MIT".into(),
			source: "https://example.com".into(),
			kind: ExtensionKind::Plugin,
			capabilities: vec![Capability::RichPresence],
			actions: vec![Action {
				id: "apply".into(),
				label: "Apply".into(),
				surface: Surface::Panel,
			}],
		};
		let input = Invocation {
			action: "apply".into(),
			..Default::default()
		};
		let sdk_presence =
			serde_json::from_value(serde_json::to_value(presence()).unwrap()).unwrap();
		let sdk = serein_extension_sdk::RichPresenceOutput {
			rich_presence: Some(serein_extension_sdk::RichPresenceUpdate::Set {
				presence: sdk_presence,
			}),
			..Default::default()
		};
		let output: Output = serde_json::from_slice(&serde_json::to_vec(&sdk).unwrap()).unwrap();
		assert!(output.validate(&manifest, &input).is_ok());
		manifest.actions[0].surface = Surface::Activation;
		assert!(output.validate(&manifest, &input).is_ok());
		for surface in [
			Surface::Message,
			Surface::Composer,
			Surface::MessageEvent,
			Surface::AppEvent,
		] {
			manifest.actions[0].surface = surface;
			assert!(output.validate(&manifest, &input).is_err());
		}
		manifest.actions[0].surface = Surface::Panel;
		manifest.capabilities.clear();
		assert!(output.validate(&manifest, &input).is_err());
		let clear = Output {
			rich_presence: Some(RichPresenceUpdate::Clear),
			..Default::default()
		};
		assert!(clear.validate(&manifest, &input).is_err());
		manifest.capabilities.push(Capability::RichPresence);
		assert!(clear.validate(&manifest, &input).is_ok());
	}
}
