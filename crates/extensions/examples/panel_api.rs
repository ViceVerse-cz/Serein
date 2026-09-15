//! Offline validation check: cargo run --locked -p extensions --example panel_api
use extensions::{Invocation, Manifest, Output};
use serde_json::json;

fn main() {
	let manifest: Manifest = serde_json::from_value(json!({
		"api_version": 1, "id": "panel-check", "name": "Offline panel check",
		"version": "1.0.0", "author": "Serein contributors", "license": "MIT",
		"source": "https://example.com/source", "kind": "plugin",
		"actions": [{"id": "apply", "label": "Apply", "surface": "panel"}]
	}))
	.unwrap();
	manifest.validate().unwrap();
	let input = Invocation {
		action: "apply".into(),
		..Default::default()
	};
	input.validate(&manifest).unwrap();
	let panel = json!([
		{"type": "heading", "text": "Appearance"},
		{"type": "separator"},
		{"type": "select", "id": "density", "label": "Density",
		 "options": ["Compact", "Comfortable"], "value": "Comfortable"},
		{"type": "slider", "id": "size", "label": "Text size", "min": 10, "max": 28, "value": 16},
		{"type": "button", "id": "apply", "label": "Apply"}
	]);
	let check = |panel| {
		serde_json::from_value::<Output>(json!({"panel": panel}))
			.unwrap()
			.validate(&manifest, &input)
	};
	check(panel.clone()).unwrap();
	for (index, field, value) in [
		(0, "text", json!("x".repeat(129))),
		(2, "id", json!("invalid id")),
		(2, "label", json!("x".repeat(129))),
		(2, "options", json!([])),
		(2, "options", json!(["Comfortable", "Comfortable"])),
		(2, "options", json!(["x".repeat(129), "Comfortable"])),
		(2, "options", json!(["bad\nlabel", "Comfortable"])),
		(
			2,
			"options",
			json!((0..33).map(|n| n.to_string()).collect::<Vec<_>>()),
		),
		(2, "value", json!("Unknown")),
		(3, "id", json!("density")),
		(3, "min", json!(28)),
		(3, "max", json!(9)),
		(3, "value", json!(29)),
	] {
		let mut invalid = panel.clone();
		invalid[index][field] = value;
		assert!(
			check(invalid).is_err(),
			"accepted invalid element {index} field {field}"
		);
	}
	// The actual sandbox enforces appearance grants before the host can apply a result.
	let mut appearance_manifest = manifest.clone();
	appearance_manifest.actions[0].surface = extensions::Surface::Activation;
	let response =
		r##"{"appearance":{"dark":{"colors":{"accent":"#5865f2"}},"style":{"body_size":18}}}"##;
	let data: String = response.bytes().map(|b| format!("\\{b:02x}")).collect();
	let module = wat::parse_str(format!(
		r#"(module (memory (export "memory") 1 256)
        (data (i32.const 32768) "{data}")
        (func (export "serein_alloc") (param i32) (result i32) (i32.const 0))
        (func (export "serein_invoke") (param i32 i32) (result i64) (i64.const {})))"#,
		(32768_u64 << 32) | response.len() as u64
	))
	.unwrap();
	let mut package = extensions::Package {
		manifest: appearance_manifest,
		theme: None,
		background_image: Vec::new(),
		cover_image: Vec::new(),
		wasm: module,
	};
	assert!(matches!(
		extensions::invoke(&package, &input),
		Err(extensions::Error::Capability)
	));
	package
		.manifest
		.capabilities
		.push(extensions::Capability::Appearance);
	let result = extensions::invoke(&package, &input).unwrap();
	assert_eq!(result.appearance.unwrap().style.body_size, Some(18));

	println!("Panel API check passed: native control schema and bounded validation.");
}
