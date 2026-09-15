//! Offline check: cargo run --locked -p extensions --example theme_import -- <theme files>
use std::io::Read;

fn main() {
	let mut document: serde_json::Value =
		serde_json::from_slice(include_bytes!("../../../extensions/ocean.serein-extension"))
			.unwrap();
	document["manifest"]["id"] = "Mixed-Case-Theme".into();
	let bytes = serde_json::to_vec(&document).unwrap();
	let package = extensions::parse_package(&bytes).unwrap();
	assert_eq!(package.manifest.id, "mixed-case-theme");
	package.validate().unwrap();
	for id in ["../escape", "CON", "A/B", "", "Thème"] {
		document["manifest"]["id"] = id.into();
		assert!(extensions::parse_package(&serde_json::to_vec(&document).unwrap()).is_err());
	}
	for path in std::env::args_os().skip(1) {
		let mut bytes = Vec::new();
		std::fs::File::open(path)
			.unwrap()
			.take(extensions::MAX_PACKAGE_BYTES as u64 + 1)
			.read_to_end(&mut bytes)
			.unwrap();
		let package = extensions::parse_package(&bytes).expect("theme import must validate");
		assert_eq!(package.manifest.kind, extensions::ExtensionKind::Theme);
		package.validate().unwrap();
		println!(
			"Imported theme: {} ({})",
			package.manifest.name, package.manifest.id
		);
	}
	println!("Theme import and path-safety debug check passed.");
}
