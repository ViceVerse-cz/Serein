//! Offline authoring check using the same manifest rules as package import.
use extensions::Manifest;
use std::{error::Error, fs::File, io::Read};

// Matches the standalone manifest limit in examples/extensions/pack.py.
const MAX_MANIFEST_BYTES: usize = 16 * 1024;

fn check(reader: impl Read) -> Result<Manifest, Box<dyn Error>> {
	let mut bytes = Vec::new();
	reader
		.take(MAX_MANIFEST_BYTES as u64 + 1)
		.read_to_end(&mut bytes)?;
	if bytes.len() > MAX_MANIFEST_BYTES {
		return Err("standalone manifest exceeds 16 KiB".into());
	}
	let manifest: Manifest = serde_json::from_slice(&bytes)?;
	manifest.validate()?;
	Ok(manifest)
}

fn main() -> Result<(), Box<dyn Error>> {
	let mut args = std::env::args_os().skip(1);
	let path = args.next().ok_or("usage: manifest_check <manifest.json>")?;
	if args.next().is_some() {
		return Err("usage: manifest_check <manifest.json>".into());
	}
	check(File::open(path)?)?;
	println!("Manifest valid; package contents and user grants are checked separately.");
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bounded_reader_checks_host_rules_without_running_a_plugin() {
		let source = include_bytes!("../../../examples/extensions/app-toolbox/manifest.json");
		let mut exact = source.to_vec();
		exact.resize(MAX_MANIFEST_BYTES, b' ');
		check(exact.as_slice()).unwrap();
		exact.push(b' ');
		assert!(check(exact.as_slice()).is_err());
		let mut invalid: serde_json::Value = serde_json::from_slice(source).unwrap();
		invalid["capabilities"] = serde_json::json!([]);
		assert!(check(serde_json::to_vec(&invalid).unwrap().as_slice()).is_err());
		assert!(check(b"{}".as_slice()).is_err());
		assert!(check(b"not json".as_slice()).is_err());
	}
}
