//! Declarative, capability-gated customization of the client's start (connecting) screen.
//!
//! A plugin with the `start_screen` grant returns a [`StartScreen`] from an activation or
//! panel result. The host keeps the last accepted value on the device and shows it on the
//! next start screen, before any account data loads. The plugin draws nothing: the host
//! renders the text and an optional spinning model through one ordered-dither (Bayer)
//! raster. Disabling the plugin, or returning `start_screen: null`, restores the default.

use serde::{Deserialize, Serialize};

/// Maximum UTF-8 bytes of the title.
pub const MAX_START_TITLE_BYTES: usize = 64;
/// Maximum UTF-8 bytes of the subtitle.
pub const MAX_START_SUBTITLE_BYTES: usize = 128;
/// Maximum model vertices.
pub const MAX_START_MODEL_VERTICES: usize = 4096;
/// Maximum model triangles.
pub const MAX_START_MODEL_TRIANGLES: usize = 8192;
/// Largest absolute vertex coordinate; the host fits the model to its frame.
pub const MAX_START_MODEL_COORDINATE: f32 = 1_000.0;
/// Largest absolute spin speed in degrees per second.
pub const MAX_START_SPIN_DEGREES: f32 = 360.0;

/// How the host draws the title and subtitle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartTextStyle {
	/// The client's normal typography.
	#[default]
	Plain,
	/// A blocky pixel font whose cells are filled through the Bayer dither raster.
	Dither,
}

/// A small triangle mesh the host spins, lights and prints through the dither raster.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StartModel {
	/// Vertex positions `[x, y, z]`, y up. The host centers and scales them to fit.
	pub vertices: Vec<[f32; 3]>,
	/// Counter-clockwise triangles as indices into `vertices`.
	pub triangles: Vec<[u16; 3]>,
	/// Fill color as `#RRGGBB`; omitted uses the theme accent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub color: Option<String>,
	/// Yaw speed in degrees per second, -360 through 360; 0 holds still.
	#[serde(default = "default_spin")]
	pub spin: f32,
	/// Fixed pitch toward the viewer in degrees, -90 through 90.
	#[serde(default = "default_pitch")]
	pub pitch: f32,
}

fn default_spin() -> f32 {
	12.0
}

fn default_pitch() -> f32 {
	20.0
}

/// The start screen a plugin asks for. A `title` replaces both default lines (the welcome
/// title and the connection stage); a `subtitle` adds a second line, or replaces only the
/// stage line when there is no title. Without either, the host's own copy stays.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StartScreen {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub subtitle: Option<String>,
	#[serde(default)]
	pub text_style: StartTextStyle,
	/// Hide the client logo above the title.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub hide_logo: bool,
	/// Show the start screen in a compact splash window; the previous size, position and
	/// maximized state are restored when the screen ends.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub compact: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<StartModel>,
}

fn color(value: &str) -> bool {
	value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn text(value: &Option<String>, limit: usize) -> bool {
	value.as_ref().is_none_or(|text| {
		!text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
	})
}

impl StartScreen {
	/// Bound every field so the host can render the screen without further checks.
	pub fn validate(&self) -> Result<(), &'static str> {
		if !text(&self.title, MAX_START_TITLE_BYTES) {
			return Err("start screen title must be 1 through 64 bytes without controls");
		}
		if !text(&self.subtitle, MAX_START_SUBTITLE_BYTES) {
			return Err("start screen subtitle must be 1 through 128 bytes without controls");
		}
		let Some(model) = &self.model else {
			return Ok(());
		};
		if model.vertices.is_empty() || model.vertices.len() > MAX_START_MODEL_VERTICES {
			return Err("start screen model needs 1 through 4096 vertices");
		}
		if model.triangles.is_empty() || model.triangles.len() > MAX_START_MODEL_TRIANGLES {
			return Err("start screen model needs 1 through 8192 triangles");
		}
		if model
			.vertices
			.iter()
			.flatten()
			.any(|value| !value.is_finite() || value.abs() > MAX_START_MODEL_COORDINATE)
		{
			return Err("start screen model coordinates must be finite and within 1000");
		}
		if model
			.triangles
			.iter()
			.flatten()
			.any(|index| usize::from(*index) >= model.vertices.len())
		{
			return Err("start screen model triangle index out of range");
		}
		if model.color.as_deref().is_some_and(|value| !color(value)) {
			return Err("start screen model color must be #RRGGBB");
		}
		if !model.spin.is_finite() || model.spin.abs() > MAX_START_SPIN_DEGREES {
			return Err("start screen model spin must be within 360 degrees per second");
		}
		if !model.pitch.is_finite() || model.pitch.abs() > 90.0 {
			return Err("start screen model pitch must be within 90 degrees");
		}
		Ok(())
	}
}

/// Opt-in wrapper preserving original SDK `Output` struct literals and compiled plugins.
/// `start_screen: Some` replaces this plugin's start screen; an explicit JSON `null` clears
/// it; omitting the field leaves it unchanged.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StartScreenOutput {
	#[serde(flatten)]
	pub output: crate::Output,
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		deserialize_with = "present"
	)]
	pub start_screen: Option<Option<StartScreen>>,
}

/// Distinguish an explicit `null` (`Some(None)`) from an omitted field (`None`).
fn present<'de, D>(deserializer: D) -> Result<Option<Option<StartScreen>>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	Option::<StartScreen>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cube() -> StartScreen {
		StartScreen {
			title: Some("Loading".into()),
			subtitle: None,
			text_style: StartTextStyle::Dither,
			hide_logo: true,
			compact: true,
			model: Some(StartModel {
				vertices: vec![
					[-1.0, -1.0, -1.0],
					[1.0, -1.0, -1.0],
					[1.0, 1.0, -1.0],
					[-1.0, 1.0, -1.0],
					[-1.0, -1.0, 1.0],
					[1.0, -1.0, 1.0],
					[1.0, 1.0, 1.0],
					[-1.0, 1.0, 1.0],
				],
				triangles: vec![[0, 1, 2], [0, 2, 3]],
				color: Some("#33aa55".into()),
				spin: 12.0,
				pitch: 20.0,
			}),
		}
	}

	#[test]
	fn bounds_are_enforced() {
		let mut screen = cube();
		assert!(screen.validate().is_ok());
		let model = screen.model.as_mut().unwrap();
		model.triangles[0] = [0, 1, 8];
		assert!(
			screen.validate().is_err(),
			"triangle index out of range rejected"
		);
		let model = screen.model.as_mut().unwrap();
		model.triangles[0] = [0, 1, 2];
		model.vertices[0] = [f32::NAN, 0.0, 0.0];
		assert!(screen.validate().is_err(), "NaN coordinate rejected");
		screen.model.as_mut().unwrap().vertices[0] = [1001.0, 0.0, 0.0];
		assert!(screen.validate().is_err(), "oversized coordinate rejected");
		let model = screen.model.as_mut().unwrap();
		model.vertices[0] = [0.0; 3];
		model.color = Some("33aa55".into());
		assert!(screen.validate().is_err(), "color without # rejected");
		screen.model.as_mut().unwrap().color = Some("#33aa5g".into());
		assert!(screen.validate().is_err(), "non-hex color rejected");
		let model = screen.model.as_mut().unwrap();
		model.color = None;
		model.vertices = vec![[0.0; 3]; MAX_START_MODEL_VERTICES + 1];
		model.triangles = vec![[0, 0, 0]];
		assert!(screen.validate().is_err(), "oversized vertex list rejected");
		screen.model.as_mut().unwrap().vertices.truncate(1);
		assert!(screen.validate().is_ok());
		screen.title = Some("x".repeat(MAX_START_TITLE_BYTES + 1));
		assert!(screen.validate().is_err(), "oversized title rejected");
		screen.title = Some("\u{7}".into());
		assert!(screen.validate().is_err(), "control character rejected");
	}

	#[test]
	fn output_distinguishes_omitted_null_and_value() {
		let screen = cube();
		let omitted: StartScreenOutput = serde_json::from_str("{}").unwrap();
		assert_eq!(omitted.start_screen, None);
		let cleared: StartScreenOutput = serde_json::from_str("{\"start_screen\":null}").unwrap();
		assert_eq!(cleared.start_screen, Some(None));
		let value = serde_json::to_string(&StartScreenOutput {
			output: crate::Output::default(),
			start_screen: Some(Some(screen.clone())),
		})
		.unwrap();
		let parsed: StartScreenOutput = serde_json::from_str(&value).unwrap();
		assert_eq!(parsed.start_screen, Some(Some(screen)));
	}
}
