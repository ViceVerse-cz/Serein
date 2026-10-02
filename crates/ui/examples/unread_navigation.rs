//! Offline check: cargo run --locked -p ui --features demo --example unread_navigation
fn main() {
	ui::debug_unread_navigation_check(&mut test_support::demo_state());
}
