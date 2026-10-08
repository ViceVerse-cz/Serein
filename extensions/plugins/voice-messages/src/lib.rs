use serein_extension_sdk::{
	Element, Invocation, VoiceMessagesConfig, VoiceMessagesOutput, WaveformStyle,
};

fn panel(settings: VoiceMessagesConfig, notice: &str) -> Vec<Element> {
	let mut elements = vec![
		Element::Heading { text: "Voice Messages".into() },
		Element::Text {
			text: "Choose Record voice message in the composer's + menu, review the recording, then choose Send. This plugin never receives audio or starts the microphone.".into(),
		},
		Element::Slider {
			id: "duration".into(), label: "Maximum recording duration (seconds)".into(),
			min: 5, max: 120, value: i32::from(settings.max_duration_seconds),
		},
		Element::Select {
			id: "waveform".into(), label: "Waveform style".into(),
			options: vec!["Bars".into(), "Line".into()],
			value: match settings.waveform_style { WaveformStyle::Bars => "Bars", WaveformStyle::Line => "Line" }.into(),
		},
		Element::Checkbox {
			id: "suppression".into(), label: "Noise suppression".into(),
			checked: settings.noise_suppression,
		},
		Element::Text { text: "Recordings are limited to 120 seconds and 8 MiB. Settings apply to new recordings; saving does not capture or send audio.".into() },
		Element::Button { id: "save".into(), label: "Save voice message settings".into() },
	];
	if !notice.is_empty() {
		elements.push(Element::Text {
			text: notice.into(),
		});
	}
	elements
}

fn settings_from_values(input: &Invocation) -> Result<VoiceMessagesConfig, &'static str> {
	let max_duration_seconds = input
		.parse_value::<u16>("duration")
		.map_err(|_| "Choose a duration between 5 and 120 seconds.")?
		.ok_or("Choose a duration between 5 and 120 seconds.")?;
	let noise_suppression = input
		.parse_value::<bool>("suppression")
		.map_err(|_| "Choose whether noise suppression is enabled.")?
		.ok_or("Choose whether noise suppression is enabled.")?;
	let waveform_style = match input.value("waveform") {
		Some("Bars") => WaveformStyle::Bars,
		Some("Line") => WaveformStyle::Line,
		_ => return Err("Choose Bars or Line waveform style."),
	};
	let settings = VoiceMessagesConfig {
		max_duration_seconds,
		waveform_style,
		noise_suppression,
	};
	settings.validate()?;
	Ok(settings)
}

fn handle(input: Invocation) -> VoiceMessagesOutput {
	if !matches!(input.action.as_str(), "activate" | "open" | "save") {
		return VoiceMessagesOutput::default();
	}
	let stored = input.storage_json::<VoiceMessagesConfig>();
	let corrupt = stored.is_err()
		|| stored
			.as_ref()
			.ok()
			.and_then(|value| value.as_ref())
			.is_some_and(|settings| settings.validate().is_err());
	let saved = stored
		.ok()
		.flatten()
		.filter(|settings| settings.validate().is_ok())
		.unwrap_or_default();
	if input.action == "activate" {
		// Invalid saved settings must not silently authorize a different recorder configuration.
		return VoiceMessagesOutput {
			voice_messages: (!corrupt).then_some(saved),
			..Default::default()
		};
	}
	let mut output = VoiceMessagesOutput::default();
	let mut shown = saved;
	let mut notice = if corrupt {
		"Saved settings are invalid. Review the defaults and Save to repair them."
	} else {
		""
	};
	if input.action == "save" {
		match settings_from_values(&input) {
			Err(error) => notice = error,
			Ok(settings) => {
				if output.output.set_storage_json(&settings).is_ok() {
					shown = settings;
					output.voice_messages = Some(settings);
					notice = "Saved. These settings apply to new recordings.";
				} else {
					notice = "Could not save these settings. Nothing was applied.";
				}
			}
		}
	}
	output.output.panel = panel(shown, notice);
	output
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	fn save_input() -> Invocation {
		Invocation {
			action: "save".into(),
			values: [
				("duration".into(), "30".into()),
				("waveform".into(), "Line".into()),
				("suppression".into(), "false".into()),
			]
			.into(),
			..Default::default()
		}
	}

	#[test]
	fn settings_save_and_activation_restore_but_open_is_passive() {
		let input = save_input();
		let bytes = dispatch_typed(&serde_json::to_vec(&input).unwrap(), handle).unwrap();
		let output: VoiceMessagesOutput = serde_json::from_slice(&bytes).unwrap();
		let expected = VoiceMessagesConfig {
			max_duration_seconds: 30,
			waveform_style: WaveformStyle::Line,
			noise_suppression: false,
		};
		assert_eq!(output.voice_messages, Some(expected));
		let storage = output.output.storage.unwrap();
		let opened = handle(Invocation {
			action: "open".into(),
			storage: Some(storage.clone()),
			..Default::default()
		});
		assert!(opened.voice_messages.is_none() && opened.output.storage.is_none());
		assert!(
			opened
				.output
				.panel
				.iter()
				.any(|element| matches!(element, Element::Slider { value: 30, .. }))
		);
		let activated = handle(Invocation {
			action: "activate".into(),
			storage: Some(storage),
			..Default::default()
		});
		assert_eq!(activated.voice_messages, Some(expected));
		assert!(activated.output.panel.is_empty() && activated.output.storage.is_none());
	}

	#[test]
	fn invalid_or_missing_form_values_never_apply_or_overwrite_storage() {
		for (key, value) in [
			("duration", "4"),
			("duration", "121"),
			("duration", "65536"),
			("duration", "NaN"),
			("waveform", "unknown"),
			("suppression", "yes"),
		] {
			let mut input = save_input();
			input.values.insert(key.into(), value.into());
			let output = handle(input);
			assert!(output.voice_messages.is_none() && output.output.storage.is_none());
		}
		for key in ["duration", "waveform", "suppression"] {
			let mut input = save_input();
			input.values.remove(key);
			let output = handle(input);
			assert!(output.voice_messages.is_none() && output.output.storage.is_none());
		}
	}

	#[test]
	fn corrupt_saved_settings_fail_closed_until_explicit_repair() {
		for storage in [
			"broken",
			r#"{"max_duration_seconds":121,"waveform_style":"bars","noise_suppression":true}"#,
		] {
			let output = handle(Invocation {
				action: "activate".into(),
				storage: Some(storage.into()),
				..Default::default()
			});
			assert!(output.voice_messages.is_none() && output.output.storage.is_none());
			let mut input = save_input();
			input.storage = Some(storage.into());
			let output = handle(input);
			assert!(output.voice_messages.is_some() && output.output.storage.is_some());
		}
	}
}
