//! Offline check: cargo run --locked -p ui --example member_voice_status
#[cfg(debug_assertions)]
fn main() {
	ui::debug_member_voice_status_check(
		test_support::voice_demo_state(),
		test_support::existing_call_demo_state(),
	);
}

#[cfg(not(debug_assertions))]
fn main() {
	println!("Run this offline check without --release.");
}
