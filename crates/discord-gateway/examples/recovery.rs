//! Offline recovery check: cargo run --locked -p discord-gateway --example recovery
fn main() {
	#[cfg(debug_assertions)]
	discord_gateway::debug_recovery_check();
	println!(
		"Gateway recovery passed: interrupted backoff and waits, coalesced requests, bounded initial login (offline)."
	);
}
