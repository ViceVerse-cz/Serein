use serein_extension_sdk::{
	AppInvocation, Element, InterfaceTweak, InterfaceTweaksOutput, Invocation, Output,
};

/// The saved choice; the layout defaults to on. Corrupt storage also falls back to on and
/// is left as-is: only an explicit toggle overwrites it.
fn layout_on(inv: &Invocation) -> bool {
	match inv.storage_json::<bool>() {
		Ok(Some(on)) => on,
		Ok(None) => true,
		Err(_) => true,
	}
}

fn tweaks(on: bool) -> Vec<InterfaceTweak> {
	if on {
		vec![InterfaceTweak::VoiceOnlyLayout]
	} else {
		Vec::new()
	}
}

fn panel(text: &str, on: bool) -> Vec<Element> {
	vec![
		Element::Text { text: text.into() },
		Element::Button {
			id: "toggle".into(),
			label: if on { "Turn off" } else { "Turn on" }.into(),
		},
	]
}

fn handle(input: AppInvocation) -> InterfaceTweaksOutput {
	let invocation = &input.invocation;
	match invocation.action.as_str() {
		// Activation re-applies the saved choice; the host does not persist its storage.
		"activate" => InterfaceTweaksOutput {
			interface_tweaks: Some(tweaks(layout_on(invocation))),
			..Default::default()
		},
		"show" => {
			let on = layout_on(invocation);
			let text = if on {
			"Ventrilo Mode is on: Serein shows the slim voice-only tree of servers, voice \
			 channels and connected users. Double-click a voice channel to join it and open \
			 its call window. The button switches back to the normal layout."
			} else {
				"Ventrilo Mode is off: Serein shows its normal layout. The button switches to \
				 the slim Ventrilo-style voice-only tree."
			};
			InterfaceTweaksOutput {
				output: Output {
					panel: panel(text, on),
					..Default::default()
				},
				interface_tweaks: None,
			}
		}
		"toggle" => {
			let on = !layout_on(invocation);
			let text = if on {
				"Ventrilo Mode turned on. The slim voice-only tree is now active."
			} else {
				"Ventrilo Mode turned off. The normal layout is restored."
			};
			let mut output = Output {
				panel: panel(text, on),
				..Default::default()
			};
			// A bool always encodes; on failure the helper leaves storage unset, so the host
			// keeps the previous value.
			let _ = output.set_storage_json(&on);
			InterfaceTweaksOutput {
				output,
				interface_tweaks: Some(tweaks(on)),
			}
		}
		_ => InterfaceTweaksOutput::default(),
	}
}
serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json, serde_json::Value, serde_json::json};

	fn run(action: &str, storage: Option<&str>) -> Value {
		let input = AppInvocation {
			invocation: Invocation {
				action: action.into(),
				storage: storage.map(Into::into),
				..Default::default()
			},
			..Default::default()
		};
		let bytes = dispatch_typed(&serde_json::to_vec(&input).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	#[test]
	fn activation_without_storage_enables_layout() {
		let output = run("activate", None);
		assert_eq!(output["interface_tweaks"], json!(["voice_only_layout"]));
		assert_eq!(output["storage"], Value::Null);
		assert_eq!(output["panel"], json!([]));
	}

	#[test]
	fn activation_with_saved_off_disables_layout() {
		let output = run("activate", Some("false"));
		assert_eq!(output["interface_tweaks"], json!([]));
		assert_eq!(output["storage"], Value::Null);
	}

	#[test]
	fn toggle_from_default_saves_off_for_next_activation() {
		let toggled = run("toggle", None);
		assert_eq!(toggled["storage"], json!("false"));
		assert_eq!(toggled["interface_tweaks"], json!([]));
		assert_eq!(toggled["panel"][1]["label"], json!("Turn on"));

		let activated = run("activate", toggled["storage"].as_str());
		assert_eq!(activated["interface_tweaks"], json!([]));
	}

	#[test]
	fn corrupt_storage_defaults_to_enabled_without_overwriting() {
		let output = run("activate", Some("{"));
		assert_eq!(output["interface_tweaks"], json!(["voice_only_layout"]));
		assert_eq!(output["storage"], Value::Null);
	}
}
