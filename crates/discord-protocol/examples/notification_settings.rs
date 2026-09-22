//! Offline check: cargo run --locked -p discord-protocol --example notification_settings
use discord_protocol::{decode, notifications::Setting, ready};
use model::Id;
use serde_json::json;

fn main() {
	// Snowflakes exceed JavaScript's exact integer range; never pass them through a float.
	let guild = 1234567890123456789_u64;
	for (guild_id, expected) in [
		(json!(null), None),
		(json!(0), None),
		(json!("0"), None),
		(json!(guild.to_string()), Some(Id(guild))),
		(json!(guild), Some(Id(guild))),
		(json!(u64::MAX), Some(Id(u64::MAX))),
	] {
		let entries = json!([
			{"guild_id":"100","channel_overrides":[{"channel_id":"2","muted":true}]},
			{"guild_id":guild_id,"hide_muted_channels":true,
			 "channel_overrides":[{"channel_id":"3","muted":true,
			 "mute_config":{"end_time":null,"selected_time_window":-1}}]}
		]);
		for settings in [entries.clone(), json!({"entries":entries,"partial":false})] {
			let bytes = serde_json::to_vec(&json!({
				"user":{"id":"1","username":"Synthetic"},
				"session_id":"synthetic-session",
				"resume_gateway_url":"wss://gateway.discord.gg/",
				"user_guild_settings":settings
			}))
			.unwrap();
			let (startup, warnings) = ready::decode(&bytes).unwrap().navigation().unwrap();
			assert!(
				!warnings.notifications,
				"valid saved settings were discarded"
			);
			let (saved, complete) = startup.user_guild_settings.unwrap().entries();
			assert!(complete);
			assert_eq!(saved[0].guild_id, Some(Id(100)));
			assert_eq!(saved[1].guild_id, expected);
			assert_eq!(saved[1].hide_muted_channels, Some(true));
			for setting in saved {
				assert_eq!(setting.channel_overrides.unwrap().0[0].muted, Some(true));
			}
		}
	}
	for value in ["{}", r#"{"guild_id":null}"#] {
		assert_eq!(decode::<Setting>(value.as_bytes()).unwrap().guild_id, None);
	}
	for value in [
		"-1",
		"1.5",
		"1.0",
		"true",
		"[]",
		"{}",
		r#""bad""#,
		"18446744073709551616",
	] {
		assert!(decode::<Setting>(format!(r#"{{"guild_id":{value}}}"#).as_bytes()).is_err());
	}
	println!(
		"Saved mutes restore from legacy and versioned READY with string or integer guild IDs; invalid IDs stay rejected."
	);
}
