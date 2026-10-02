//! Offline check: cargo run --locked -p ui --example direct_message_navigation
fn main() {
	let mut state = test_support::demo_state();
	let dm = state.channels.iter().find(|c| c.kind == 1).unwrap().id;
	let guild = state.guilds[0].id;
	state.select(dm);
	state.select_guild(guild);
	state.open_messages();
	assert_eq!(state.selected, Some(dm));
	state.open_home();
	state.select_guild(guild);
	state.open_messages();
	assert_eq!(state.selected, None);
	state.select(dm);
	state.select_guild(guild);
	state.channels.retain(|c| c.id != dm);
	state.open_messages();
	assert_eq!(state.selected, None);
	state.logout();
	assert_eq!(state.last_viewed_dm, None);
}
