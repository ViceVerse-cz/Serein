use extensions::{Capability, Invocation, Manifest, Output, Surface};

#[test]
fn auto_reply_activation_requires_its_separate_grant_and_preserves_legacy_wire() {
	let mut manifest: Manifest = serde_json::from_str(include_str!(
		"../../../extensions/plugins/openrouter-auto-reply/manifest.json"
	))
	.unwrap();
	manifest.validate().unwrap();
	let input = Invocation {
		action: "activate".into(),
		..Default::default()
	};
	let sdk = serein_extension_sdk::OpenrouterAutoReplyOutput {
		openrouter_auto_reply: true,
		..Default::default()
	};
	let host: Output = serde_json::from_value(serde_json::to_value(sdk).unwrap()).unwrap();
	host.validate(&manifest, &input).unwrap();
	manifest.capabilities.clear();
	assert!(host.validate(&manifest, &input).is_err());
	manifest.capabilities.push(Capability::OpenrouterAutoReply);
	for surface in [
		Surface::Panel,
		Surface::MessageEvent,
		Surface::AppEvent,
		Surface::Tick,
		Surface::Composer,
		Surface::Message,
	] {
		manifest.actions[1].surface = surface;
		assert!(host.validate(&manifest, &input).is_err());
	}
	let wire = serde_json::to_value(serein_extension_sdk::Output::default()).unwrap();
	assert!(wire.get("openrouter_auto_reply").is_none());
	assert!(
		!serde_json::from_value::<Output>(wire)
			.unwrap()
			.openrouter_auto_reply
	);
}

#[test]
fn committed_plugin_runs_in_the_sandbox() {
	let package = extensions::parse_package(include_bytes!(
		"../../../extensions/plugins/packages/openrouter-auto-reply.serein-extension"
	))
	.unwrap();
	let output = extensions::invoke(
		&package,
		&Invocation {
			action: "activate".into(),
			..Default::default()
		},
	)
	.unwrap();
	assert!(output.openrouter_auto_reply && output.storage.is_none());
	assert!(
		!extensions::invoke(
			&package,
			&Invocation {
				action: "open".into(),
				..Default::default()
			}
		)
		.unwrap()
		.panel
		.is_empty()
	);
}
