//! Offline sandbox timing: `cargo run --locked --release -p extensions --example benchmark`.
//! Pass a local API Proxy package to check its Wasm activation/Open/Apply and timing.
//! Measures wall time, not process memory or native UI frame time.
use extensions::{Invocation, invoke, parse_package};
use std::time::Instant;

fn median_us(mut measurements: Vec<u128>) -> u128 {
	measurements.sort_unstable();
	measurements[measurements.len() / 2]
}

fn measure(name: &str, bytes: &[u8], invocation: Invocation) {
	let package = parse_package(bytes).expect("synthetic package is valid");
	std::hint::black_box(invoke(&package, &invocation).expect("warmup succeeds"));
	let mut cold = Vec::new();
	for _ in 0..5 {
		let start = Instant::now();
		let package = parse_package(bytes).expect("synthetic package is valid");
		std::hint::black_box(invoke(&package, &invocation).expect("invocation succeeds"));
		cold.push(start.elapsed().as_micros());
	}
	let mut repeated = Vec::new();
	for _ in 0..100 {
		let start = Instant::now();
		std::hint::black_box(invoke(&package, &invocation).expect("invocation succeeds"));
		repeated.push(start.elapsed().as_micros());
	}

	println!(
		"{name}: package_bytes={}, wasm_bytes={}, parse_plus_invoke_median_us={} (n=5), invoke_median_us={} (n=100; new runtime each call)",
		bytes.len(),
		package.wasm.len(),
		median_us(cold),
		median_us(repeated)
	);
}

fn main() {
	if let Some(path) = std::env::args_os().nth(1) {
		use std::io::Read;
		let mut bytes = Vec::new();
		std::fs::File::open(path)
			.expect("plugin file")
			.take(extensions::MAX_PACKAGE_BYTES as u64 + 1)
			.read_to_end(&mut bytes)
			.expect("bounded plugin read");
		let package = parse_package(&bytes).expect("API Proxy package");
		let route = extensions::ApiProxyConfig::Url {
			url: "http://127.0.0.1:8080".into(),
		};
		let input = Invocation {
			action: "activate".into(),
			storage: Some(serde_json::to_string(&route).unwrap()),
			..Default::default()
		};
		assert_eq!(invoke(&package, &input).unwrap().api_proxy, Some(route));
		let opened = invoke(
			&package,
			&Invocation {
				action: "open".into(),
				..input.clone()
			},
		)
		.unwrap();
		assert!(opened.api_proxy.is_none() && opened.storage.is_none());
		let mut apply = Invocation {
			action: "apply".into(),
			..input.clone()
		};
		apply.values.insert("mode".into(), "Direct".into());
		let applied = invoke(&package, &apply).unwrap();
		assert_eq!(applied.api_proxy, Some(extensions::ApiProxyConfig::Direct));
		assert!(applied.storage.is_some());
		measure("api-proxy/activation", &bytes, input);
	}
	measure(
		"message-delete-protector",
		include_bytes!(
			"../../../community-extensions/plugins/packages/message-delete-protector.serein-extension"
		),
		Invocation {
			action: "activate".into(),
			..Default::default()
		},
	);
}
