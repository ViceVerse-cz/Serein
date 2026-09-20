fn main() {
	assert!(std::env::args().any(|arg| arg == "--demo"), "pass --demo");
	#[cfg(debug_assertions)]
	ui::debug_role_mentions_check(&mut test_support::demo_state());
	println!(
		"Role and thread mentions: autocomplete, wire IDs, guild scope, chat rendering and composer checks passed."
	);
}
