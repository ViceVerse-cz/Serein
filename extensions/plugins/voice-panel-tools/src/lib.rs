/*
 * Voice Panel Tools — a safe Serein port of the volume controls in Equicord's
 * VCPanelSettings plugin.
 *
 * Upstream: Equicord/Equicord at commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e,
 * `src/equicordplugins/vcPanelSettings/index.tsx`
 * (Copyright (c) 2024 Vendicated and contributors, GPL-3.0-or-later).
 *
 * Serein adaptation copyright (c) 2026 Serein contributors.
 * Changed November 8, 2026: the patch into Discord's voice panel is replaced by
 * a native settings panel that reads the granted audio-settings snapshot and
 * proposes one bounded `set_audio_settings` change. No device list, camera
 * control, network access, microphone opening or automatic apply exists.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppAction, AppInvocation, AppOutput, AudioSettingsPatch, AudioSettingsSnapshot, Element,
	HostEffect, Output,
};

/// Upstream's output-volume slider range.
const MAX_OUTPUT_PERCENT: u16 = 200;
/// Upstream's input-volume slider range. Serein's host model permits up to 200
/// percent input gain, so a larger existing value is reported but left alone.
const MAX_INPUT_PERCENT: u16 = 100;

const OUTPUT_INPUT_ID: &str = "output-percent";
const INPUT_INPUT_ID: &str = "input-percent";

const UNAVAILABLE: &str = "Serein did not provide the current audio settings. This host is older than the audio_settings grant or the grant is unavailable, so nothing can be changed here.";
const OUT_OF_RANGE: &str = "Nothing changed: choose an output volume from 0 through 200 percent and an input volume from 0 through 100 percent.";
const NO_CHANGE: &str =
	"No volume change: the proposed values already match the current local audio settings.";

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

fn open_output(panel: Vec<Element>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		effects: Vec::new(),
	}
}

fn current(input: &AppInvocation) -> Option<&AudioSettingsSnapshot> {
	input
		.app
		.as_ref()
		.and_then(|app| app.audio_settings.as_ref())
}

/// The input slider must stay inside the host's slider bounds. An existing
/// input gain above upstream's 100 percent cap is shown as text and deliberately
/// left out of the form, so a volume-only edit cannot silently lower it.
fn input_is_editable(settings: &AudioSettingsSnapshot) -> bool {
	settings.input_percent <= MAX_INPUT_PERCENT
}

fn form_panel(settings: &AudioSettingsSnapshot, error: Option<&str>) -> Vec<Element> {
	let mut panel = vec![
		heading("Voice panel tools"),
		text(
			"These controls change the same local output and microphone gain as Serein's Voice & Video settings. Review the proposed values and press Apply; this panel never opens a microphone or joins a call.",
		),
	];
	if let Some(error) = error {
		panel.push(text(error));
	}
	panel.push(Element::Slider {
		id: OUTPUT_INPUT_ID.into(),
		label: "Output volume (0-200%)".into(),
		min: 0,
		max: i32::from(MAX_OUTPUT_PERCENT),
		value: i32::from(settings.output_percent),
	});
	if input_is_editable(settings) {
		panel.push(Element::Slider {
			id: INPUT_INPUT_ID.into(),
			label: "Input volume (0-100%)".into(),
			min: 0,
			max: i32::from(MAX_INPUT_PERCENT),
			value: i32::from(settings.input_percent),
		});
	} else {
		panel.push(text(format!(
			"The current input gain is {}%, above this panel's 0-100% range. It is left unchanged here; use Serein's Voice & Video settings for values above 100%.",
			settings.input_percent
		)));
	}
	panel.push(text(format!(
		"Current processing: {} profile, {} suppression.",
		settings.input_profile, settings.suppression
	)));
	panel.push(Element::Button {
		id: "apply".into(),
		label: "Review volume change".into(),
	});
	panel
}

fn proposal_panel(settings: &AudioSettingsSnapshot, patch: &AudioSettingsPatch) -> Vec<Element> {
	let mut panel = vec![heading("Review the volume change")];
	if let Some(output) = patch.output_percent {
		panel.push(text(format!(
			"Output volume: {}% (currently {}%).",
			output, settings.output_percent
		)));
	}
	if let Some(input) = patch.input_percent {
		panel.push(text(format!(
			"Input volume: {}% (currently {}%).",
			input, settings.input_percent
		)));
	} else if !input_is_editable(settings) {
		panel.push(text(format!(
			"Input volume stays at {}%, outside this panel's editable range.",
			settings.input_percent
		)));
	}
	panel.push(text(
		"Press Apply to write these local audio preferences. Only the listed gain values change; processing, devices, calls and microphone state are untouched.",
	));
	panel
}

fn parse_percent(input: &AppInvocation, id: &str, max: u16) -> Result<u16, ()> {
	match input.invocation.parse_value::<u16>(id) {
		Ok(Some(value)) if value <= max => Ok(value),
		_ => Err(()),
	}
}

fn handle(input: AppInvocation) -> AppOutput {
	if input.app_event.is_some() || input.message_event.is_some() {
		return AppOutput::default();
	}
	let action = input.invocation.action.as_str();
	if !matches!(action, "open" | "apply") {
		return AppOutput::default();
	}
	let Some(settings) = current(&input) else {
		return open_output(vec![heading("Voice panel tools"), text(UNAVAILABLE)]);
	};
	match action {
		"open" => open_output(form_panel(settings, None)),
		"apply" => {
			let Ok(output) = parse_percent(&input, OUTPUT_INPUT_ID, MAX_OUTPUT_PERCENT) else {
				return open_output(form_panel(settings, Some(OUT_OF_RANGE)));
			};
			let proposed_input = if input_is_editable(settings) {
				match parse_percent(&input, INPUT_INPUT_ID, MAX_INPUT_PERCENT) {
					Ok(value) => Some(value),
					Err(()) => return open_output(form_panel(settings, Some(OUT_OF_RANGE))),
				}
			} else {
				None
			};

			let mut patch = AudioSettingsPatch::default();
			let mut changed = false;
			if output != settings.output_percent {
				patch.output_percent = Some(output);
				changed = true;
			}
			if let Some(input) = proposed_input
				&& input != settings.input_percent
			{
				patch.input_percent = Some(input);
				changed = true;
			}
			if !changed {
				return open_output(vec![
					heading("Voice panel tools"),
					text(NO_CHANGE),
					text("Nothing was proposed and no setting was changed."),
				]);
			}
			let panel = proposal_panel(settings, &patch);
			AppOutput {
				output: Output {
					panel,
					..Default::default()
				},
				effects: vec![HostEffect::AppAction {
					action: AppAction::SetAudioSettings { settings: patch },
				}],
			}
		}
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::dispatch_typed;
	use serein_extension_sdk::serde_json::{self, Value, json};

	fn snapshot(input_percent: u16, output_percent: u16) -> Value {
		json!({
			"input_percent": input_percent,
			"output_percent": output_percent,
			"push_to_talk": false,
			"input_profile": "voice_isolation",
			"suppression": "rnnoise",
			"suppression_level": 2,
			"echo_cancellation": true,
			"automatic_gain": true,
			"sensitivity_db": -55
		})
	}

	fn run(values: Value, audio: Option<Value>) -> AppOutput {
		let mut invocation = json!({"action": "apply", "values": values});
		if let Some(audio) = audio {
			invocation["app"] = json!({"audio_settings": audio});
		}
		let bytes = dispatch_typed(&serde_json::to_vec(&invocation).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	fn panel_text(output: &AppOutput) -> String {
		output
			.output
			.panel
			.iter()
			.filter_map(|element| match element {
				Element::Text { text } => Some(text.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n")
	}

	fn proposed_patch(output: &AppOutput) -> &AudioSettingsPatch {
		assert_eq!(output.effects.len(), 1);
		let HostEffect::AppAction {
			action: AppAction::SetAudioSettings { settings },
		} = &output.effects[0]
		else {
			panic!("expected a set_audio_settings proposal");
		};
		settings
	}

	#[test]
	fn apply_proposes_only_the_changed_output_gain() {
		let output = run(
			json!({"output-percent": "150", "input-percent": "100"}),
			Some(snapshot(100, 100)),
		);
		let patch = proposed_patch(&output);
		assert_eq!(patch.output_percent, Some(150));
		assert!(patch.input_percent.is_none());
		assert!(!patch.open_microphone);
		assert!(panel_text(&output).contains("Output volume: 150% (currently 100%)"));
	}

	#[test]
	fn input_gain_above_the_upstream_cap_is_reported_and_never_lowered() {
		let output = run(
			json!({"output-percent": "80", "input-percent": "0"}),
			Some(snapshot(140, 100)),
		);
		let patch = proposed_patch(&output);
		assert_eq!(patch.output_percent, Some(80));
		assert!(patch.input_percent.is_none());
		assert!(panel_text(&output).contains("Input volume stays at 140%"));
	}

	#[test]
	fn out_of_range_or_missing_values_propose_nothing() {
		for values in [
			json!({"output-percent": "201", "input-percent": "50"}),
			json!({"output-percent": "100", "input-percent": "101"}),
			json!({"input-percent": "50"}),
			json!({"output-percent": "100"}),
			json!({"output-percent": "abc", "input-percent": "50"}),
		] {
			let output = run(values, Some(snapshot(100, 100)));
			assert!(output.effects.is_empty());
			assert!(panel_text(&output).contains("Nothing changed"));
		}
	}

	#[test]
	fn unchanged_gains_do_not_propose_a_write() {
		let output = run(
			json!({"output-percent": "100", "input-percent": "100"}),
			Some(snapshot(100, 100)),
		);
		assert!(output.effects.is_empty());
		assert!(panel_text(&output).contains("No volume change"));
	}

	#[test]
	fn missing_audio_snapshot_is_reported_without_a_form_or_effect() {
		let output = run(json!({"output-percent": "150"}), None);
		assert!(output.effects.is_empty());
		assert!(panel_text(&output).contains("did not provide the current audio settings"));
		assert!(
			output
				.output
				.panel
				.iter()
				.all(|element| !matches!(element, Element::Slider { .. }))
		);
	}

	#[test]
	fn open_uses_host_values_and_hides_an_above_cap_input_slider() {
		let invocation = json!({
			"action": "open",
			"app": {"audio_settings": snapshot(140, 55)}
		});
		let bytes = dispatch_typed(&serde_json::to_vec(&invocation).unwrap(), handle).unwrap();
		let output: AppOutput = serde_json::from_slice(&bytes).unwrap();
		assert!(output.effects.is_empty());
		let sliders: Vec<_> = output
			.output
			.panel
			.iter()
			.filter_map(|element| match element {
				Element::Slider { id, value, max, .. } => Some((id.as_str(), *value, *max)),
				_ => None,
			})
			.collect();
		assert_eq!(sliders, vec![(OUTPUT_INPUT_ID, 55, 200)]);
		assert!(panel_text(&output).contains("current input gain is 140%"));
	}
}
