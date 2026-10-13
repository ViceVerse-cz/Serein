use extensions::{
	Action, Capability, Error, ExtensionKind, ImageCompression, Invocation, Manifest, Output,
	Surface,
};

#[test]
fn packaged_photo_plugin_activates_and_applies_only_valid_panel_settings() {
	let package = extensions::parse_package(include_bytes!(
		"../../../extensions/plugins/packages/photo-compression.serein-extension"
	))
	.unwrap();
	let mut input = Invocation {
		action: "enable".into(),
		..Default::default()
	};
	let output = extensions::invoke(&package, &input).unwrap();
	assert_eq!(
		output.image_compression,
		Some(ImageCompression {
			max_edge: 1280,
			jpeg_quality: 80
		})
	);
	input.action = "configure".into();
	let output = extensions::invoke(&package, &input).unwrap();
	assert!(!output.panel.is_empty() && output.image_compression.is_none());
	input.action = "apply".into();
	input.values.insert("edge".into(), "512".into());
	input.values.insert("quality".into(), "70".into());
	let output = extensions::invoke(&package, &input).unwrap();
	assert_eq!(
		output.image_compression,
		Some(ImageCompression {
			max_edge: 512,
			jpeg_quality: 70
		})
	);
	input.storage = output.storage;
	input.values.insert("quality".into(), "255".into());
	let output = extensions::invoke(&package, &input).unwrap();
	assert!(output.image_compression.is_none() && output.storage.is_none());
	input.action = "enable".into();
	assert_eq!(
		extensions::invoke(&package, &input)
			.unwrap()
			.image_compression,
		Some(ImageCompression {
			max_edge: 512,
			jpeg_quality: 70
		})
	);
}

#[test]
fn photo_policy_requires_explicit_capability_and_foreground_configuration() {
	let mut manifest = Manifest {
		api_version: 1,
		id: "photo-compression".into(),
		name: "Photo compression".into(),
		version: "1".into(),
		author: "test".into(),
		license: "MIT".into(),
		source: "https://example.org/source".into(),
		kind: ExtensionKind::Plugin,
		capabilities: vec![],
		actions: vec![Action {
			id: "configure".into(),
			label: "Configure".into(),
			surface: Surface::Activation,
		}],
	};
	let input = Invocation {
		action: "configure".into(),
		..Default::default()
	};
	let mut output = Output {
		image_compression: Some(ImageCompression {
			max_edge: 1280,
			jpeg_quality: 80,
		}),
		..Default::default()
	};
	assert!(matches!(
		output.validate(&manifest, &input),
		Err(Error::Capability)
	));
	manifest.capabilities.push(Capability::ImageCompression);
	assert!(output.validate(&manifest, &input).is_ok());
	let sdk = serein_extension_sdk::Output {
		image_compression: Some(serein_extension_sdk::ImageCompression {
			max_edge: 1280,
			jpeg_quality: 80,
		}),
		..Default::default()
	};
	let decoded: Output = serde_json::from_slice(&serde_json::to_vec(&sdk).unwrap()).unwrap();
	assert_eq!(decoded.image_compression, output.image_compression);
	for surface in [
		Surface::Message,
		Surface::Composer,
		Surface::MessageEvent,
		Surface::AppEvent,
		Surface::Tick,
	] {
		manifest.actions[0].surface = surface;
		assert!(matches!(
			output.validate(&manifest, &input),
			Err(Error::Capability)
		));
	}
	manifest.actions[0].surface = Surface::Panel;
	for (max_edge, jpeg_quality) in [(255, 80), (2049, 80), (1280, 39), (1280, 96)] {
		output.image_compression = Some(ImageCompression {
			max_edge,
			jpeg_quality,
		});
		assert!(matches!(
			output.validate(&manifest, &input),
			Err(Error::Invalid)
		));
	}
}
