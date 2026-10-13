use serein_extension_sdk::{Element, ImageCompression, Invocation, Output};

fn valid(policy: ImageCompression) -> bool {
	(256..=2048).contains(&policy.max_edge) && (40..=95).contains(&policy.jpeg_quality)
}

fn handle(input: Invocation) -> Output {
	let saved = input
		.storage_json::<ImageCompression>()
		.ok()
		.flatten()
		.filter(|value| valid(*value))
		.unwrap_or(ImageCompression {
			max_edge: 1280,
			jpeg_quality: 80,
		});
	if input.action == "enable" {
		return Output {
			image_compression: Some(saved),
			..Default::default()
		};
	}
	if !matches!(input.action.as_str(), "configure" | "apply") {
		return Output::default();
	}
	let mut output = Output::default();
	let mut policy = saved;
	let notice = if input.action == "apply" {
		let changed = input
			.value("edge")
			.and_then(|value| value.parse().ok())
			.zip(input.value("quality").and_then(|value| value.parse().ok()))
			.map(|(max_edge, jpeg_quality)| ImageCompression {
				max_edge,
				jpeg_quality,
			})
			.filter(|policy| valid(*policy));
		if let Some(changed) = changed {
			if output.set_storage_json(&changed).is_ok() {
				policy = changed;
				output.image_compression = Some(changed);
				"Applied to photos selected next."
			} else {
				"Could not save these settings."
			}
		} else {
			"Enter an edge from 256 to 2048 and quality from 40 to 95."
		}
	} else {
		"Disable this plugin to keep newly selected originals."
	};
	output.panel = vec![
        Element::Heading { text: "Photo compression".into() },
        Element::Text { text: "Selected JPEGs and opaque static PNGs may become smaller JPEGs. Files on disk stay unchanged. Transparent, animated, unsupported or large files keep their original. Videos are not compressed.".into() },
        Element::TextInput { id: "edge".into(), label: "Longest edge (256–2048 pixels)".into(), value: policy.max_edge.to_string() },
        Element::TextInput { id: "quality".into(), label: "JPEG quality (40–95)".into(), value: policy.jpeg_quality.to_string() },
        Element::Button { id: "apply".into(), label: "Apply to next photo selections".into() },
        Element::Text { text: notice.into() },
    ];
	output
}

serein_extension_sdk::export!(handle);
