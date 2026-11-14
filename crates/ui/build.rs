use std::{env, fs, path::PathBuf};

fn main() {
	let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../assets/icons");
	let directory = root.join("svg");
	// Watch the parent too so adding the optional SVG directory invalidates the map.
	println!("cargo:rerun-if-changed={}", root.display());
	if directory.exists() {
		println!("cargo:rerun-if-changed={}", directory.display());
	}
	println!(
		"cargo:rerun-if-changed={}",
		root.join("index.tsv").display()
	);
	let index = fs::read_to_string(root.join("index.tsv")).expect("bundled icon index");
	let mut generated = String::from("const SVG_SOURCES: &[(&str, &[u8])] = &[\n");
	let mut bytes = 0;
	for line in index.lines() {
		let (name, _) = line.split_once('\t').expect("bundled icon index entry");
		let path = directory.join(format!("{name}.svg"));
		if !path.exists() {
			continue;
		}
		println!("cargo:rerun-if-changed={}", path.display());
		let size = fs::metadata(&path).expect("bundled SVG metadata").len();
		assert!(size <= 256 * 1024, "bundled icon SVG exceeds 256 KiB");
		bytes += size;
		assert!(bytes <= 1024 * 1024, "bundled icon SVGs exceed 1 MiB");
		generated.push_str(&format!(
			"({name:?}, include_bytes!({:?})),\n",
			path.to_str().expect("UTF-8 icon path")
		));
	}
	generated.push_str("];\n");
	let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("icon_sources.rs");
	fs::write(output, generated).expect("write bundled SVG source map");
}
