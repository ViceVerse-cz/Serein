use extensions::{
	Action, Capability, Error, ExtensionKind, Invocation, Manifest, Output, Surface,
	VoiceMessagesConfig, WaveformStyle,
};

fn manifest(surface: Surface) -> Manifest {
	Manifest {
		api_version: 1,
		id: "voice-messages".into(),
		name: "Voice Messages".into(),
		version: "1".into(),
		author: "test".into(),
		license: "MIT".into(),
		source: "https://example.org/source".into(),
		kind: ExtensionKind::Plugin,
		capabilities: vec![Capability::VoiceMessages],
		actions: vec![Action {
			id: "configure".into(),
			label: "Configure".into(),
			surface,
		}],
	}
}

#[test]
fn config_bounds_and_sdk_wire_shape_agree() {
	for seconds in [0, 4, 5, 120, 121, u16::MAX] {
		for waveform_style in [WaveformStyle::Bars, WaveformStyle::Line] {
			let host = VoiceMessagesConfig {
				max_duration_seconds: seconds,
				waveform_style,
				noise_suppression: true,
			};
			let sdk: serein_extension_sdk::VoiceMessagesConfig =
				serde_json::from_value(serde_json::to_value(host).unwrap()).unwrap();
			assert_eq!(host.validate().is_ok(), (5..=120).contains(&seconds));
			assert_eq!(host.validate().is_ok(), sdk.validate().is_ok());
		}
	}
	for text in [
		r#"{"max_duration_seconds":120,"waveform_style":"bars","noise_suppression":true,"audio":[]}"#,
		r#"{"max_duration_seconds":120,"waveform_style":"unknown","noise_suppression":true}"#,
		r#"{"max_duration_seconds":120,"waveform_style":"bars"}"#,
	] {
		assert!(serde_json::from_str::<VoiceMessagesConfig>(text).is_err());
		assert!(serde_json::from_str::<serein_extension_sdk::VoiceMessagesConfig>(text).is_err());
	}
}

#[test]
fn native_configuration_requires_opt_in_and_panel_or_activation() {
	let sdk = serein_extension_sdk::VoiceMessagesOutput {
		voice_messages: Some(serein_extension_sdk::VoiceMessagesConfig::default()),
		..Default::default()
	};
	let host: Output = serde_json::from_value(serde_json::to_value(sdk).unwrap()).unwrap();
	let input = Invocation {
		action: "configure".into(),
		..Default::default()
	};
	for surface in [Surface::Panel, Surface::Activation] {
		let mut manifest = manifest(surface);
		host.validate(&manifest, &input).unwrap();
		manifest.capabilities.clear();
		assert!(matches!(
			host.validate(&manifest, &input),
			Err(Error::Capability)
		));
	}
	for surface in [
		Surface::Message,
		Surface::Composer,
		Surface::MessageEvent,
		Surface::AppEvent,
		Surface::Tick,
	] {
		assert!(matches!(
			host.validate(&manifest(surface), &input),
			Err(Error::Capability)
		));
	}
	assert!(
		serde_json::from_str::<Output>("{}")
			.unwrap()
			.voice_messages
			.is_none()
	);
}

#[test]
fn committed_plugin_runs_only_inside_bounded_sandbox_and_returns_settings() {
	let package = extensions::parse_package(include_bytes!(
		"../../../extensions/plugins/packages/voice-messages.serein-extension"
	))
	.unwrap();
	let mut input = Invocation {
		action: "activate".into(),
		..Default::default()
	};
	let activated = extensions::invoke(&package, &input).unwrap();
	assert_eq!(
		activated.voice_messages,
		Some(VoiceMessagesConfig::default())
	);
	assert!(
		activated.panel.is_empty() && activated.storage.is_none() && activated.effects.is_empty()
	);
	input.action = "open".into();
	let opened = extensions::invoke(&package, &input).unwrap();
	assert!(
		!opened.panel.is_empty() && opened.voice_messages.is_none() && opened.storage.is_none()
	);
	input.action = "save".into();
	input.values = [
		("duration".into(), "5".into()),
		("waveform".into(), "Line".into()),
		("suppression".into(), "false".into()),
	]
	.into();
	let saved = extensions::invoke(&package, &input).unwrap();
	assert_eq!(
		saved.voice_messages,
		Some(VoiceMessagesConfig {
			max_duration_seconds: 5,
			waveform_style: WaveformStyle::Line,
			noise_suppression: false,
		})
	);
	input.storage = saved.storage;
	input.action = "activate".into();
	input.values.clear();
	assert_eq!(
		extensions::invoke(&package, &input).unwrap().voice_messages,
		saved.voice_messages
	);
	input.action = "save".into();
	input.values = [
		("duration".into(), "121".into()),
		("waveform".into(), "Line".into()),
		("suppression".into(), "false".into()),
	]
	.into();
	let invalid = extensions::invoke(&package, &input).unwrap();
	assert!(invalid.voice_messages.is_none() && invalid.storage.is_none());
	input.action = "activate".into();
	input.values.clear();
	input.storage = Some("broken".into());
	assert!(
		extensions::invoke(&package, &input)
			.unwrap()
			.voice_messages
			.is_none()
	);
}
