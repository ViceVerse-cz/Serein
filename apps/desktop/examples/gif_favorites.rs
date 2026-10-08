//! Offline debug check: cargo run --locked -p serein --features demo --example gif_favorites
fn main() {
	let gifs = ui::debug_gif_favorites_check(test_support::message(1, model::Id(20)));
	use discord_protocol::gif_favorites::{decode_response, encode_patch};
	let mut current = decode_response(br#"{"settings":""}"#).expect("empty initial settings");
	assert_eq!(current.version, 0);
	for gif in &gifs {
		let patch = encode_patch(&current, gif, true).unwrap();
		let json = serde_json::json!({"settings": patch}).to_string();
		let saved = decode_response(json.as_bytes()).unwrap();
		assert!(saved.matches(gif, true).unwrap());
		assert!(current.unchanged_except(&saved, &gif.url).unwrap());
		assert!(
			saved
				.favorites()
				.unwrap()
				.iter()
				.any(|saved| saved.url == gif.url)
		);
		current = saved;
	}
	let long =
		decode_response(include_bytes!("gif_favorites_long.json")).expect("bounded long metadata");
	let patch = encode_patch(&long, &gifs[0], true).unwrap();
	let saved =
		decode_response(serde_json::json!({"settings":patch}).to_string().as_bytes()).unwrap();
	assert!(long.unchanged_except(&saved, &gifs[0].url).unwrap());
	assert_eq!(saved.favorites().unwrap().len(), 1);
	assert!(
		decode_response(br#"{"settings":"Eg=="}"#).is_err(),
		"truncation still fails"
	);
	println!(
		"GIF attachment stars, external proxies, video favorites, empty settings, round-trip preservation and bounded long records passed offline."
	);
}
