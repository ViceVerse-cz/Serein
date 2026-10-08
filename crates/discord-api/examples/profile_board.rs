//! Offline debug check: cargo run --locked -p discord-api --example profile_board
use discord_protocol::profile::{apply_board_games, decode_profile};
use model::{Id, ProfileGameWidgetKind};

fn main() {
	#[cfg(debug_assertions)]
	discord_api::debug_profile_board_request_check();
	let mut profile = decode_profile(
		br#"{"user":{"id":"1","username":"synthetic"},"user_profile":{"bio":"About"},"widgets":[{"data":{"type":"future_widget"}},{"data":{"type":"favorite_games","games":[{"game_id":"2","tags":["open_to_play"]}]}},{"data":{"type":"current_games","games":[{"game_id":"3"}]}}]}"#,
		None,
		true,
	).unwrap();
	let board = profile.board.as_ref().unwrap();
	assert_eq!(board.len(), 2);
	assert_eq!(board[0].kind, ProfileGameWidgetKind::Favorite);
	assert_eq!(board[1].games.len(), 1);
	apply_board_games(
		&mut profile,
		br#"[{"id":"2","name":"Synthetic game","cover_image_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"id":"3","name":"Unrequested game"}]"#,
		&[Id(2)],
	).unwrap();
	let board = profile.board.as_ref().unwrap();
	assert_eq!(board[0].games[0].name.as_deref(), Some("Synthetic game"));
	assert_eq!(
		board[0].games[0].cover_key().as_deref(),
		Some("application-icon-2-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
	);
	assert!(board[1].games[0].name.is_none());
	apply_board_games(
		&mut profile,
		br#"[{"id":"2","name":"Current game","media":{"cover":{"type":"hash","value":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}},{"id":"3","name":"Proxied game","media":{"cover":{"type":"url","value":"https://images-ext-1.discordapp.net/external/aaaaaaaaaaaaaaaa/https/example.com/cover.jpg"},"icon":{"type":"url","value":"https://cdn.discordapp.com/app-icons/3/cccccccccccccccccccccccccccccccc.png?size=128"}}}]"#,
		&[Id(2), Id(3)],
	).unwrap();
	let board = profile.board.as_ref().unwrap();
	assert_eq!(board[0].games[0].name.as_deref(), Some("Current game"));
	assert_eq!(
		board[0].games[0].cover_key().as_deref(),
		Some("application-icon-2-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
	);
	assert_eq!(
		board[1].games[0].cover_key().as_deref(),
		Some(
			"game:https://images-ext-1.discordapp.net/external/aaaaaaaaaaaaaaaa/https/example.com/cover.jpg"
		)
	);
	assert_eq!(
		board[1].games[0].icon_key().as_deref(),
		Some("application-icon-3-cccccccccccccccccccccccccccccccc")
	);
	for value in [
		"http://127.0.0.1/cover.png",
		"https://images-ext-1.discordapp.net.evil.test/external/a/https/b/c",
		"https://cdn.discordapp.com/app-icons/4/cccccccccccccccccccccccccccccccc.png",
	] {
		let bytes = serde_json::json!([{"id":"3","name":"Safe name","media":{"cover":{"type":"url","value":value}}}]).to_string();
		apply_board_games(&mut profile, bytes.as_bytes(), &[Id(3)]).unwrap();
		assert!(
			profile.board.as_ref().unwrap()[1].games[0]
				.cover_key()
				.is_none()
		);
	}

	assert!(profile.valid());
	assert!(apply_board_games(&mut profile, &vec![0; 256 * 1024 + 1], &[Id(2)]).is_err());
	let absent = decode_profile(
		br#"{"user":{"id":"1","username":"synthetic"}}"#,
		None,
		false,
	)
	.unwrap();
	assert!(absent.board.is_none());
	let oversized = serde_json::json!({"user":{"id":"1","username":"synthetic"},"user_profile":{"bio":"Editable"},"widgets":[{"data":{"type":"favorite_games","games":[{"game_id":"2","comment":"x".repeat(300),"tags":["a","b","c","d"]}]}}]});
	let capped = decode_profile(oversized.to_string().as_bytes(), None, false).unwrap();
	assert!(
		!capped.limited,
		"optional Board limits must not disable profile editing"
	);
	assert_eq!(
		capped.board.unwrap()[0].games[0]
			.comment
			.as_ref()
			.unwrap()
			.len(),
		256
	);
	println!(
		"Profile Board repeated query keys, current/legacy artwork, URL rejection, requested IDs and wire bounds passed."
	);
}
