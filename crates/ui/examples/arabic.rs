//! Offline check: cargo run --locked -p ui --features demo --example arabic
fn main() {
	#[cfg(debug_assertions)]
	ui::debug_arabic_check();
	#[cfg(not(debug_assertions))]
	panic!("Run this check in the debug profile.");
}
