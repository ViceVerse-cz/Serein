//! Offline ABI compatibility and timing check after building the standalone SDK examples.
//! `cargo run --locked --release -p extensions --example sdk_check -- <wasm-directory>`
use extensions::{Invocation, MAX_MODULE_BYTES, Output, Package, invoke, parse_package};
use std::{io::Read, path::PathBuf, time::Instant};

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
		let mut wasm = Vec::new();
		std::fs::File::open(wasm_dir.join(wasm_file))?
			.take(MAX_MODULE_BYTES as u64 + 1)
			.read_to_end(&mut wasm)?;
		assert!(wasm.len() <= MAX_MODULE_BYTES, "{name}: Wasm is too large");
		let rebuilt = Package {
			manifest: serde_json::from_str(manifest)?,
			theme: None,
			background_image: Vec::new(),
			cover_image: Vec::new(),
			wasm,
		};
		check(
			&format!("{name}/rebuilt"),
			&serde_json::to_vec(&rebuilt)?,
			&expected,
		);
	}
	Ok(())
}
