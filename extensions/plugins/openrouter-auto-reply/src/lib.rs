use serein_extension_sdk::{Element, Invocation, OpenrouterAutoReplyOutput, Output};

fn handle(input: Invocation) -> OpenrouterAutoReplyOutput {
	match input.action.as_str() {
		"activate" => OpenrouterAutoReplyOutput { openrouter_auto_reply: true, ..Default::default() },
		"open" => OpenrouterAutoReplyOutput { output: Output {
			panel: vec![Element::Text { text: "Configure your connection and chosen DMs below. Settings are managed by Serein; no API key or message text is given to this plugin.".into() }],
			..Default::default()
		}, ..Default::default() },
		_ => OpenrouterAutoReplyOutput::default(),
	}
}
serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn activation_exposes_settings_without_starting_network_work() {
		assert!(
			handle(Invocation {
				action: "activate".into(),
				..Default::default()
			})
			.openrouter_auto_reply
		);
		let open = handle(Invocation {
			action: "open".into(),
			..Default::default()
		});
		assert!(
			!open.output.panel.is_empty()
				&& !open.openrouter_auto_reply
				&& open.output.storage.is_none()
		);
	}
}
