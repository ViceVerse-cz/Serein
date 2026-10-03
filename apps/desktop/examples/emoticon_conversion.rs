//! Offline check: cargo run --locked -p serein --example emoticon_conversion
#[cfg(debug_assertions)]
fn main() {
	ui::debug_emoticon_conversion_check(test_support::demo_state);
	let legacy: local_store::AppPreferences = serde_json::from_str("{}").unwrap();
	assert!(!legacy.convert_emoticons);
	let path =
		std::env::temp_dir().join(format!("serein-emoticons-{}.sqlite3", std::process::id()));
	{
		let store = local_store::LocalStore::open(&path).unwrap();
		let mut preferences = store.app_preferences().unwrap();
		preferences.convert_emoticons = true;
		store.save_app_preferences(&preferences).unwrap();
	}
	{
		let store = local_store::LocalStore::open(&path).unwrap();
		assert!(store.app_preferences().unwrap().convert_emoticons);
		let mut preferences = store.app_preferences().unwrap();
		preferences.convert_emoticons = false;
		store.save_app_preferences(&preferences).unwrap();
	}
	assert!(
		!local_store::LocalStore::open(&path)
			.unwrap()
			.app_preferences()
			.unwrap()
			.convert_emoticons
	);
	std::fs::remove_file(path).unwrap();
	println!(
		"Emoticon debug check passed: composer on/off, protected code/links and SQLite reopen. Synthetic/offline only."
	);
}

#[cfg(not(debug_assertions))]
fn main() {
	println!("Run this offline check without --release.");
}
