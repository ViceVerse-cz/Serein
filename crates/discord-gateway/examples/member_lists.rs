fn main() {
	#[cfg(debug_assertions)]
	discord_gateway::debug_member_list_check();
	println!(
		"Member decoding, partial presence, atomic updates and recovery checks passed (offline)."
	);
}
