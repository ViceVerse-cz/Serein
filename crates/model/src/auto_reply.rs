//! Bounded host settings for private OpenRouter auto replies.
use crate::Id;
use serde::{Deserialize, Serialize};

pub const MAX_TARGETS: usize = 32;
pub const MAX_PROMPT_BYTES: usize = 1024;
pub const MAX_MESSAGE_BYTES: usize = 8000;
pub const PREFIX: &str = "[Auto reply] ";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
	pub model: String,
	pub instructions: String,
	pub channels: Vec<Id>,
}

impl Config {
	pub fn validate(&self) -> Result<(), &'static str> {
		if self.model.is_empty()
			|| self.model.len() > 128
			|| !self
				.model
				.bytes()
				.all(|c| c.is_ascii_alphanumeric() || b"/._:-".contains(&c))
		{
			return Err("Enter an OpenRouter model ID (up to 128 bytes).");
		}
		if self.instructions.len() > MAX_PROMPT_BYTES {
			return Err("Reply instructions must fit 1,024 UTF-8 bytes.");
		}
		if self.channels.is_empty()
			|| self.channels.len() > MAX_TARGETS
			|| self
				.channels
				.iter()
				.enumerate()
				.any(|(i, id)| id.0 == 0 || self.channels[..i].contains(id))
		{
			return Err("Choose between 1 and 32 different private conversations.");
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn settings_reject_unbounded_or_ambiguous_targets_and_text() {
		let valid = Config {
			model: "provider/model:free".into(),
			instructions: "Be concise".into(),
			channels: vec![Id(20)],
		};
		assert!(valid.validate().is_ok());
		for channels in [
			vec![],
			vec![Id(0)],
			vec![Id(20), Id(20)],
			(1..=33).map(Id).collect(),
		] {
			assert!(
				Config {
					channels,
					..valid.clone()
				}
				.validate()
				.is_err()
			);
		}
		for model in ["", "provider/model\n", &"x".repeat(129)] {
			assert!(
				Config {
					model: model.into(),
					..valid.clone()
				}
				.validate()
				.is_err()
			);
		}
		assert!(
			Config {
				instructions: "x".repeat(MAX_PROMPT_BYTES + 1),
				..valid
			}
			.validate()
			.is_err()
		);
	}
}
