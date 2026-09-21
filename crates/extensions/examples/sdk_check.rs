//! Offline ABI compatibility and timing check after building the standalone SDK examples.
//! `cargo run --locked --release -p extensions --example sdk_check -- <wasm-directory>`
use extensions::{
	Element, Invocation, MAX_MODULE_BYTES, MessageEvent, MessageEventKind, Output, Package, invoke,
	parse_package,
};
use std::{
	io::Read,
	path::{Path, PathBuf},
	time::Instant,
};

fn check(name: &str, bytes: &[u8], expected: &Output) {
	let package = parse_package(bytes).expect("package is valid");
	let input = Invocation {
		action: "activate".into(),
		..Default::default()
	};
	let output = invoke(&package, &input).expect("warmup invocation succeeds");
	assert_eq!(
		serde_json::to_value(output).unwrap(),
		serde_json::to_value(expected).unwrap(),
		"{name}: activation output changed"
	);
	let mut samples = [0_u128; 5];
	for sample in &mut samples {
		let start = Instant::now();
		for _ in 0..20 {
			std::hint::black_box(invoke(&package, &input).expect("invocation succeeds"));
		}
		*sample = start.elapsed().as_nanos() / 20;
	}
	samples.sort_unstable();
	println!(
		"{name}: package_bytes={}, wasm_bytes={}, invoke_median_us={:.3} (5 samples x 20 calls; one warmup; new runtime per call; excludes package construction/parsing and process startup)",
		bytes.len(),
		package.wasm.len(),
		samples[2] as f64 / 1000.0
	);
}

fn rebuilt(manifest: &str, path: &Path) -> Result<Package, Box<dyn std::error::Error>> {
	let mut wasm = Vec::new();
	std::fs::File::open(path)?
		.take(MAX_MODULE_BYTES as u64 + 1)
		.read_to_end(&mut wasm)?;
	assert!(wasm.len() <= MAX_MODULE_BYTES, "Wasm is too large");
	Ok(Package {
		manifest: serde_json::from_str(manifest)?,
		theme: None,
		background_image: Vec::new(),
		cover_image: Vec::new(),
		wasm,
	})
}

fn check_message_counter(package: &Package) {
	package.validate().expect("counter package is valid");
	let mut input = Invocation {
		action: "message-event".into(),
		..Default::default()
	};
	for (kind, counts) in [
		(MessageEventKind::Create, [1, 0, 0]),
		(MessageEventKind::Update, [1, 1, 0]),
		(MessageEventKind::Delete, [1, 1, 1]),
	] {
		input.message_event = Some(MessageEvent {
			kind,
			channel_id: "100".into(),
			message_id: "200".into(),
			author_id: (kind == MessageEventKind::Create).then(|| "300".into()),
			content: (kind != MessageEventKind::Delete).then(|| "Synthetic message".into()),
		});
		let output = invoke(package, &input).expect("counter event succeeds");
		// The only effect is numeric storage; event IDs/content are never retained or displayed.
		assert_eq!(
			serde_json::to_value(&output).unwrap(),
			serde_json::to_value(Output {
				storage: Some(format!(
					r#"{{"create":{},"update":{},"delete":{}}}"#,
					counts[0], counts[1], counts[2]
				)),
				..Default::default()
			})
			.unwrap()
		);
		input.storage = output.storage;
	}
	input.message_event = None;
	input.action = "show".into();
	let shown = invoke(package, &input).expect("counter panel succeeds");
	assert!(shown.storage.is_none());
	assert!(
		matches!(&shown.panel[1], Element::Text { text } if text == "Created: 1\nUpdated: 1\nDeleted: 1")
	);

	input.storage = Some("invalid JSON".into());
	let shown = invoke(package, &input).expect("invalid storage is recoverable");
	assert!(shown.storage.is_none());
	assert!(
		matches!(&shown.panel[1], Element::Text { text } if text == "Saved counts are invalid. Reset counts to start again.")
	);
	input.action = "message-event".into();
	input.message_event = Some(MessageEvent {
		kind: MessageEventKind::Delete,
		channel_id: "100".into(),
		message_id: "200".into(),
		author_id: None,
		content: None,
	});
	let output = invoke(package, &input).expect("event preserves invalid storage");
	assert_eq!(
		serde_json::to_value(output).unwrap(),
		serde_json::to_value(Output::default()).unwrap()
	);
	input.message_event = None;
	input.action = "reset".into();
	let reset = invoke(package, &input).expect("explicit reset recovers storage");
	assert_eq!(
		reset.storage.as_deref(),
		Some(r#"{"create":0,"update":0,"delete":0}"#)
	);
	assert!(
		matches!(&reset.panel[1], Element::Text { text } if text == "Created: 0\nUpdated: 0\nDeleted: 0")
	);
	println!(
		"message-counter/rebuilt: wasm_bytes={}, create/update/delete, panel, invalid storage and reset passed",
		package.wasm.len()
	);
	input.action = "message-event".into();
	input.storage = reset.storage;
	input.message_event = Some(MessageEvent {
		kind: MessageEventKind::Create,
		channel_id: "100".into(),
		message_id: "200".into(),
		author_id: Some("300".into()),
		content: Some("Synthetic message".into()),
	});
	input.message_event.as_mut().unwrap().content =
		Some("\u{0000}".repeat(extensions::MAX_EVENT_CONTENT_BYTES));
	invoke(package, &input).expect("maximum escaped event content fits the sandbox");
	input.message_event.as_mut().unwrap().content = Some("Synthetic message".into());
	let _ = invoke(package, &input).unwrap();
	let mut samples = [0_u128; 5];
	for sample in &mut samples {
		let start = Instant::now();
		for _ in 0..20 {
			std::hint::black_box(invoke(package, &input).unwrap());
		}
		*sample = start.elapsed().as_nanos() / 20;
	}
	samples.sort_unstable();
	println!(
		"message-counter/create: invoke_median_us={:.3} (5 samples x 20 calls; one warmup; new runtime per call; excludes worker IO)",
		samples[2] as f64 / 1000.0
	);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let mut args = std::env::args_os().skip(1);
	let wasm_dir = PathBuf::from(args.next().expect("usage: sdk_check <wasm-directory>"));
	assert!(args.next().is_none(), "usage: sdk_check <wasm-directory>");
	for (name, committed, manifest, wasm_file, expected) in [
		(
			"message-delete-protector",
			include_bytes!(
				"../../../examples/extensions/packages/message-delete-protector.serein-extension"
			)
			.as_slice(),
			include_str!("../../../examples/extensions/message-delete-protector/manifest.json"),
			"message_delete_protector.wasm",
			Output {
				preserve_deleted_messages: true,
				..Default::default()
			},
		),
		(
			"emoji-sticker-images",
			include_bytes!(
				"../../../examples/extensions/packages/emoji-sticker-images.serein-extension"
			)
			.as_slice(),
			include_str!("../../../examples/extensions/emoji-sticker-images/manifest.json"),
			"emoji_sticker_images.wasm",
			Output {
				image_sharing: true,
				..Default::default()
			},
		),
	] {
		check(&format!("{name}/committed"), committed, &expected);
		let rebuilt = rebuilt(manifest, &wasm_dir.join(wasm_file))?;
		check(
			&format!("{name}/rebuilt"),
			&serde_json::to_vec(&rebuilt)?,
			&expected,
		);
	}
	check_message_counter(&rebuilt(
		include_str!("../../../examples/extensions/message-counter/manifest.json"),
		&wasm_dir.join("message_counter.wasm"),
	)?);
	Ok(())
}
