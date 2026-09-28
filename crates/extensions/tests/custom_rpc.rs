//! Validate a compiled Custom RPC package through the actual offline Wasm sandbox.
use extensions::{Element, Invocation, RichPresenceUpdate, invoke, parse_package};

#[test]
fn compiled_custom_rpc_preserves_drafts_and_controls_presence() {
	let bytes = include_bytes!("../../../examples/extensions/packages/custom-rpc.serein-extension");
	let package = parse_package(bytes).expect("package validates");
	let mut input = Invocation {
		action: "open".into(),
		..Default::default()
	};
	let opened = invoke(&package, &input).expect("native editor panel validates");
	assert!(opened.rich_presence.is_none());
	assert!(opened.storage.is_none());
	input.values.extend([
		("application-id".into(), "123456789".into()),
		("name".into(), "Synthetic Custom RPC".into()),
		("details".into(), "Offline sandbox check".into()),
		("large-key".into(), "cover".into()),
		("button1-label".into(), "Example".into()),
		("button1-url".into(), "https://example.com".into()),
		("timer".into(), "Custom timestamps".into()),
		("start".into(), "2024-03-01 00:00".into()),
	]);
	input.action = "preview".into();
	let preview = invoke(&package, &input).expect("preview validates");
	assert!(preview.rich_presence.is_none() && preview.storage.is_none());
	assert!(
		preview
			.panel
			.iter()
			.any(|e| matches!(e, Element::ActivityPreview { .. }))
	);
	input.action = "apply".into();
	let applied = invoke(&package, &input).expect("apply validates");
	assert!(matches!(
		applied.rich_presence,
		Some(RichPresenceUpdate::Set { .. })
	));
	input.storage = applied.storage;
	input.values.clear();
	input.action = "activate".into();
	assert!(
		invoke(&package, &input)
			.expect("restore validates")
			.rich_presence
			.is_some()
	);
	input.action = "stop".into();
	let stopped = invoke(&package, &input).expect("stop validates");
	assert!(matches!(
		stopped.rich_presence,
		Some(RichPresenceUpdate::Clear)
	));
	input.storage = stopped.storage;
	input.action = "activate".into();
	assert!(
		invoke(&package, &input)
			.expect("stopped activation validates")
			.rich_presence
			.is_none()
	);
	input.action = "apply".into();
	input
		.values
		.insert("button1-url".into(), "javascript:alert(1)".into());
	let invalid = invoke(&package, &input).expect("invalid field stays a valid editor panel");
	assert!(invalid.rich_presence.is_none() && invalid.storage.is_none());
	println!(
		"Custom RPC: {} package bytes, {} Wasm bytes; open, preview, apply, restore, stop and invalid draft passed.",
		bytes.len(),
		package.wasm.len()
	);
}
