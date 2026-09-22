//! Offline check: cargo run --locked -p ui --features demo --example thread_navigation
fn main() {
	ui::debug_thread_navigation_check(&mut test_support::demo_state());
}
