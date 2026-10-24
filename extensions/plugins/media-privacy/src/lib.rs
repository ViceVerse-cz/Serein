use serein_extension_sdk::{
	Element, Invocation, MediaPrivacy, MediaPrivacyStyle, Output,
};

const STYLES: [&str; 5] = ["Blur", "Black", "Gray", "White", "Custom"];

fn style_name(config: MediaPrivacy) -> &'static str {
	if config.style == MediaPrivacyStyle::Blur {
		return "Blur";
	}
	match config.color {
		[0, 0, 0] => "Black",
		[128, 128, 128] => "Gray",
		[255, 255, 255] => "White",
		_ => "Custom",
	}
}

fn hex(color: [u8; 3]) -> String {
	format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn parse_hex(value: &str) -> Option<[u8; 3]> {
	let value = value.trim().strip_prefix('#')?;
	if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
		return None;
	}
	let channel = |pair: &str| u8::from_str_radix(pair, 16).ok();
	Some([
		channel(&value[0..2])?,
		channel(&value[2..4])?,
		channel(&value[4..6])?,
	])
}

fn clamp(value: Option<&str>, fallback: u8) -> u8 {
	value
		.and_then(|value| value.parse::<u8>().ok())
		.unwrap_or(fallback)
		.min(100)
}

fn panel(config: MediaPrivacy, saved: bool) -> Vec<Element> {
	let mut panel = vec![
		Element::Text {
			text: "Chat images, GIFs and videos stay covered until you hover. The \
				composer button beside GIF toggles masking without opening this panel."
				.into(),
		},
		Element::Checkbox {
			id: "enabled".into(),
			label: "Mask chat media".into(),
			checked: config.enabled,
		},
		Element::Checkbox {
			id: "reveal".into(),
			label: "Reveal on hover".into(),
			checked: config.reveal_on_hover,
		},
		Element::Select {
			id: "style".into(),
			label: "Effect".into(),
			options: STYLES.iter().map(|style| (*style).to_string()).collect(),
			value: style_name(config).into(),
		},
		Element::Slider {
			id: "strength".into(),
			label: "Blur strength".into(),
			min: 0,
			max: 100,
			value: i32::from(config.strength),
		},
		Element::Slider {
			id: "opacity".into(),
			label: "Mask opacity".into(),
			min: 0,
			max: 100,
			value: i32::from(config.opacity),
		},
		Element::TextInput {
			id: "color".into(),
			label: "Custom colour (#rrggbb)".into(),
			value: hex(config.color),
		},
	];
	if saved {
		panel.push(Element::Text {
			text: "Saved.".into(),
		});
	}
	panel.push(Element::Button {
		id: "settings".into(),
		label: "Save".into(),
	});
	panel
}

fn settings(input: Invocation) -> Output {
	let mut config = input.media_privacy.unwrap_or_default();
	let saved = !input.values.is_empty();
	if saved {
		if let Some(value) = input.value("enabled") {
			config.enabled = value == "true";
		}
		if let Some(value) = input.value("reveal") {
			config.reveal_on_hover = value == "true";
		}
		if let Some(value) = input.value("strength") {
			config.strength = clamp(Some(value), config.strength);
		}
		if let Some(value) = input.value("opacity") {
			config.opacity = clamp(Some(value), config.opacity);
		}
		match input.value("style") {
			Some("Blur") => config.style = MediaPrivacyStyle::Blur,
			Some("Black") => {
				config.style = MediaPrivacyStyle::Solid;
				config.color = [0, 0, 0];
			}
			Some("Gray") => {
				config.style = MediaPrivacyStyle::Solid;
				config.color = [128, 128, 128];
			}
			Some("White") => {
				config.style = MediaPrivacyStyle::Solid;
				config.color = [255, 255, 255];
			}
			Some("Custom") => {
				config.style = MediaPrivacyStyle::Solid;
				if let Some(color) = input.value("color").and_then(parse_hex) {
					config.color = color;
				}
			}
			_ => {}
		}
	}
	Output {
		media_privacy: Some(config),
		panel: panel(config, saved),
		..Default::default()
	}
}

fn handle(input: Invocation) -> Output {
	match input.action.as_str() {
		"settings" => settings(input),
		_ => Output::default(),
	}
}

serein_extension_sdk::export!(handle);
