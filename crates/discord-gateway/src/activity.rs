use client_core::auth::Failure;
use discord_protocol::activity_sessions::{self, Observation};
use discord_protocol::rpc::Activity;
use model::{OwnPresence, PresenceStatus};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;

/// One replaceable game, linked Spotify playback, custom status, and last attempted values.
/// Five seconds between attempts is stricter than the documented 5 updates/20 seconds.
pub(super) struct Pending {
	current: Option<Activity>,
	spotify: Option<discord_protocol::spotify::Activity>,
	sent_spotify: Option<Option<discord_protocol::spotify::Activity>>,
	own_presence: OwnPresence,
	sent_presence: Option<OwnPresence>,
	idle_since: Option<u64>,
	sent: Option<Option<Activity>>,
	next_send: Instant,
	pub observation: Observation,
}
impl Default for Pending {
	fn default() -> Self {
		Self {
			current: None,
			spotify: None,
			sent_spotify: None,
			own_presence: OwnPresence::default(),
			sent_presence: None,
			idle_since: None,
			sent: None,
			next_send: Instant::now(),
			observation: Observation::Unconfirmed,
		}
	}
}
impl Pending {
	pub fn update_presence(&mut self, presence: &OwnPresence) -> Result<(), Failure> {
		if !presence.valid() {
			return Err(Failure::Protocol);
		}
		if self.own_presence.status != presence.status {
			self.idle_since = (presence.status == PresenceStatus::Idle).then(|| {
				SystemTime::now()
					.duration_since(UNIX_EPOCH)
					.unwrap_or_default()
					.as_millis()
					.min(u64::MAX as u128) as u64
			});
		}
		if self.own_presence != *presence {
			self.own_presence = presence.clone();
			self.observation = Observation::Unconfirmed;
		}
		Ok(())
	}
	/// Reidentifying must retain Invisible; activity publication still waits for READY.
	pub fn identify_presence(&self) -> serde_json::Value {
		serde_json::json!({"since":self.idle_since,"activities":[],"status":self.own_presence.status.wire(),"afk":false})
	}
	pub fn update(&mut self, activity: &Option<Activity>) -> Result<(), Failure> {
		if let Some(activity) = activity {
			activity.validate().map_err(|_| Failure::Protocol)?;
		}
		if self.current != *activity {
			self.current = activity.clone();
			self.observation = Observation::Unconfirmed;
		}
		Ok(())
	}
	pub fn reconnect(&mut self) {
		self.sent = None;
		self.sent_spotify = None;
		self.sent_presence = None;
		self.observation = Observation::Unconfirmed;
	}
	pub fn update_spotify(
		&mut self,
		activity: &Option<discord_protocol::spotify::Activity>,
	) -> Result<(), Failure> {
		if let Some(activity) = activity {
			activity.validate().map_err(|_| Failure::Protocol)?;
		}
		self.spotify.clone_from(activity);
		Ok(())
	}
	pub fn observe(&mut self, bytes: &[u8], session: &str) {
		if self.sent.as_ref() != Some(&self.current)
			|| self.sent_presence.as_ref() != Some(&self.own_presence)
			|| self.own_presence.status == PresenceStatus::Invisible
		{
			self.observation = Observation::Unconfirmed;
			return;
		}
		self.observation = self
			.current
			.as_ref()
			.map_or(Observation::Unconfirmed, |current| {
				activity_sessions::observe(bytes, session, current)
					.unwrap_or(Observation::Unconfirmed)
			});
	}
	pub fn deadline(&self) -> Option<Instant> {
		(self.sent.as_ref() != Some(&self.current)
			|| self.sent_spotify.as_ref() != Some(&self.spotify)
			|| self.sent_presence.as_ref() != Some(&self.own_presence))
		.then_some(self.next_send)
	}
	pub fn packet(&mut self, now: Instant) -> Option<Message> {
		if self.deadline().is_none_or(|deadline| now < deadline) {
			return None;
		}
		let mut payload = self.identify_presence();
		if self.own_presence.status != PresenceStatus::Invisible {
			let mut activities: Vec<_> = self
				.current
				.iter()
				.map(|activity| serde_json::json!(activity))
				.collect();
			if let Some(spotify) = &self.spotify {
				activities.push(serde_json::json!(spotify));
			}
			if !self.own_presence.custom_status.is_empty() {
				activities.push(
					serde_json::json!({"name":"Custom Status","type":4,"state":self.own_presence.custom_status}),
				);
			}
			payload["activities"] = serde_json::json!(activities);
		}
		if self.sent.as_ref() != Some(&self.current)
			|| self.sent_presence.as_ref() != Some(&self.own_presence)
		{
			self.observation = Observation::Unconfirmed;
		}
		self.sent = Some(self.current.clone());
		self.sent_spotify = Some(self.spotify.clone());
		self.sent_presence = Some(self.own_presence.clone());
		// Charge attempts too: a failed write may have reached Discord.
		self.next_send = now + Duration::from_secs(5);
		Some(Message::Text(
			serde_json::json!({"op": 3, "d": payload})
				.to_string()
				.into(),
		))
	}
}

#[cfg(debug_assertions)]
pub fn debug_spotify_check() {
	use serde_json::{Value, json};
	let now_ms = 1_800_000_000_000_u64;
	let wire = json!({
		"is_playing":true,"device":{"is_private_session":false},
		"progress_ms":30_000,"currently_playing_type":"track",
		"item":{"id":"0123456789abcdefghijkl","type":"track","is_local":false,
			"name":"Synthetic track","duration_ms":180_000,
			"artists":[{"name":"Synthetic artist"}],
			"album":{"name":"Synthetic album","images":[{"url":
				"https://i.scdn.co/image/ab67616d0000b2730123456789abcdef01234567"}]}}
	});
	let decode = |wire: &Value| {
		discord_protocol::spotify::decode_playback(
			&serde_json::to_vec(wire).unwrap(),
			model::Id(1),
			now_ms,
		)
	};
	let spotify = decode(&wire).unwrap().unwrap();
	assert!(spotify.display().valid());
	assert!(spotify.display().image.is_some());
	assert_eq!(spotify.timestamps.end, Some(now_ms + 150_000));
	for (pointer, value) in [
		("/is_playing", json!(false)),
		("/device/is_private_session", json!(true)),
		("/device", Value::Null),
		("/item/is_local", json!(true)),
		("/currently_playing_type", json!("ad")),
		("/item/id", json!("../invalid")),
		("/progress_ms", json!(180_000)),
	] {
		let mut hidden = wire.clone();
		*hidden.pointer_mut(pointer).unwrap() = value;
		assert!(decode(&hidden).unwrap().is_none(), "{pointer}");
	}
	assert!(
		discord_protocol::spotify::decode_playback(
			&vec![b' '; discord_protocol::spotify::MAX_PLAYBACK_BYTES + 1],
			model::Id(1),
			now_ms,
		)
		.is_err()
	);
	let mut pending = Pending::default();
	let game = discord_protocol::rpc::ActivityFields::default()
		.into_activity(model::Id(42), "Synthetic game".into())
		.unwrap();
	pending.update(&Some(game)).unwrap();
	let mut presence = OwnPresence {
		custom_status: "Synthetic status".into(),
		..Default::default()
	};
	pending.update_presence(&presence).unwrap();
	pending.update_spotify(&Some(spotify.clone())).unwrap();
	let now = Instant::now();
	let packet =
		|message: Message| serde_json::from_str::<Value>(message.to_text().unwrap()).unwrap();
	let sent = packet(pending.packet(now).unwrap());
	assert_eq!(sent["d"]["activities"].as_array().unwrap().len(), 3);
	assert_eq!(sent["d"]["activities"][1]["sync_id"], spotify.sync_id);
	assert_eq!(sent["d"]["activities"][1]["party"]["id"], "spotify:1");
	assert_eq!(sent["d"]["activities"][1]["flags"], 48);
	assert_eq!(sent["d"]["activities"][2]["type"], 4);
	pending.update_spotify(&None).unwrap();
	assert!(pending.packet(now + Duration::from_secs(4)).is_none());
	let cleared = packet(pending.packet(now + Duration::from_secs(5)).unwrap());
	assert_eq!(cleared["d"]["activities"].as_array().unwrap().len(), 2);
	assert_eq!(cleared["d"]["activities"][0]["name"], "Synthetic game");
	pending.update_spotify(&Some(spotify)).unwrap();
	presence.status = PresenceStatus::Invisible;
	pending.update_presence(&presence).unwrap();
	assert_eq!(
		packet(pending.packet(now + Duration::from_secs(10)).unwrap())["d"]["activities"],
		json!([])
	);
	presence.status = PresenceStatus::Online;
	pending.update_presence(&presence).unwrap();
	pending.reconnect();
	assert!(pending.packet(now + Duration::from_secs(14)).is_none());
	assert_eq!(
		packet(pending.packet(now + Duration::from_secs(15)).unwrap())["d"]["activities"]
			.as_array()
			.unwrap()
			.len(),
		3
	);
	println!(
		"Spotify playback decoding, privacy, publication, clearing and reconnect passed (offline)."
	);
}

#[cfg(test)]
mod tests {
	use super::*;
	use discord_protocol::rpc::{ActivityFields, Assets, Timestamps};
	use model::Id;

	fn game(name: &str) -> Activity {
		ActivityFields {
			details: Some("Ranked match".into()),
			state: Some("Round 2".into()),
			timestamps: Some(Timestamps {
				start: Some(1_700_000_000),
				end: None,
			}),
			assets: Some(Assets {
				large_image: Some("123".into()),
				..Default::default()
			}),
			..Default::default()
		}
		.into_activity(Id(42), name.into())
		.unwrap()
	}

	#[test]
	fn own_presence_preserves_games_clears_and_restores_after_invisible() {
		let mut pending = Pending::default();
		let now = Instant::now();
		let packet = |frame: Message| -> serde_json::Value {
			serde_json::from_str(frame.to_text().unwrap()).unwrap()
		};
		let mut own = OwnPresence {
			status: PresenceStatus::Idle,
			custom_status: "Taking a break".into(),
			expires_at_ms: None,
		};
		pending.update_presence(&own).unwrap();
		pending.update(&Some(game("osu!"))).unwrap();
		let first = packet(pending.packet(now).unwrap());
		assert_eq!(first["d"]["status"], "idle");
		assert!(first["d"]["since"].as_u64().is_some_and(|since| since > 0));
		assert_eq!(first["d"]["activities"][0]["name"], "osu!");
		assert_eq!(
			first["d"]["activities"][1],
			serde_json::json!({"type":4,"name":"Custom Status","state":"Taking a break"})
		);
		pending.update(&Some(game("Minecraft"))).unwrap();
		let next = packet(pending.packet(now + Duration::from_secs(5)).unwrap());
		assert_eq!(first["d"]["since"], next["d"]["since"]);
		own.status = PresenceStatus::Invisible;
		pending.update_presence(&own).unwrap();
		assert_eq!(pending.identify_presence()["status"], "invisible");
		assert!(pending.packet(now + Duration::from_secs(9)).is_none());
		let hidden = packet(pending.packet(now + Duration::from_secs(10)).unwrap());
		assert_eq!(hidden["d"]["activities"], serde_json::json!([]));
		assert_eq!(hidden["d"]["since"], serde_json::Value::Null);
		pending.reconnect();
		assert_eq!(pending.identify_presence()["status"], "invisible");
		assert!(pending.packet(now + Duration::from_secs(14)).is_none());
		pending.packet(now + Duration::from_secs(15)).unwrap();
		own.status = PresenceStatus::DoNotDisturb;
		pending.update_presence(&own).unwrap();
		let restored = packet(pending.packet(now + Duration::from_secs(20)).unwrap());
		assert_eq!(restored["d"]["status"], "dnd");
		assert_eq!(restored["d"]["activities"][0]["name"], "Minecraft");
		assert_eq!(restored["d"]["activities"][1]["state"], "Taking a break");
		let invalid = OwnPresence {
			custom_status: "x".repeat(129),
			..own.clone()
		};
		assert_eq!(pending.update_presence(&invalid), Err(Failure::Protocol));
		assert!(pending.deadline().is_none());
		own.custom_status.clear();
		pending.update_presence(&own).unwrap();
		let cleared = packet(pending.packet(now + Duration::from_secs(25)).unwrap());
		assert_eq!(cleared["d"]["activities"].as_array().unwrap().len(), 1);
		assert_eq!(cleared["d"]["activities"][0]["name"], "Minecraft");
		own.custom_status = "Still here".into();
		pending.update_presence(&own).unwrap();
		pending.update(&None).unwrap();
		let custom_only = packet(pending.packet(now + Duration::from_secs(30)).unwrap());
		assert_eq!(
			custom_only["d"]["activities"],
			serde_json::json!([{"type":4,"name":"Custom Status","state":"Still here"}])
		);
	}

	#[test]
	fn server_observations_do_not_confirm_unsent_changed_or_cleared_games() {
		let mut pending = Pending::default();
		let listed = br#"[{"session_id":"all","activities":[{"application_id":"42","type":0}]}]"#;
		pending.update(&Some(game("osu!"))).unwrap();
		pending.observe(listed, "own");
		assert_eq!(pending.observation, Observation::Unconfirmed);
		pending.packet(Instant::now()).unwrap();
		pending.observe(listed, "own");
		assert_eq!(pending.observation, Observation::ServerListed);
		let mut changed = game("osu!");
		changed.details = Some("Next map".into());
		pending.update(&Some(changed)).unwrap();
		pending.observe(listed, "own");
		assert_eq!(pending.observation, Observation::Unconfirmed);
		pending.update(&None).unwrap();
		pending.observe(listed, "own");
		assert_eq!(pending.observation, Observation::Unconfirmed);
	}

	#[test]
	fn activity_coalesces_clears_reconnects_and_bounds_utf8() {
		let mut pending = Pending::default();
		let now = Instant::now();
		let packet = |frame: Message| -> serde_json::Value {
			serde_json::from_str(frame.to_text().unwrap()).unwrap()
		};
		pending.update(&Some(game("osu!"))).unwrap();
		assert_eq!(
			packet(pending.packet(now).unwrap()),
			serde_json::json!({"op":3,"d":{
				"since":null,"activities":[{"name":"osu!","type":0,"application_id":"42",
					"details":"Ranked match","state":"Round 2","timestamps":{"start":1_700_000_000_000_u64},
					"assets":{"large_image":"123"}}],"status":"online","afk":false
			}})
		);
		assert!(pending.deadline().is_none());
		pending.update(&Some(game("Intermediate"))).unwrap();
		pending.update(&None).unwrap();
		assert!(pending.packet(now + Duration::from_millis(4999)).is_none());
		assert_eq!(
			packet(pending.packet(now + Duration::from_secs(5)).unwrap())["d"]["activities"],
			serde_json::json!([])
		);
		assert!(pending.deadline().is_none());
		pending.reconnect();
		assert!(pending.packet(now + Duration::from_secs(9)).is_none());
		assert_eq!(
			packet(pending.packet(now + Duration::from_secs(10)).unwrap())["d"]["activities"],
			serde_json::json!([])
		);
		for name in [
			"x".repeat(129),
			"é".repeat(65),
			"game\n".into(),
			"\0".into(),
			" ".into(),
		] {
			let mut activity = game("Valid");
			activity.name = name;
			assert_eq!(pending.update(&Some(activity)), Err(Failure::Protocol));
		}
		pending.update(&Some(game(&"é".repeat(64)))).unwrap();
		assert!(pending.packet(now + Duration::from_secs(14)).is_none());
		assert!(pending.packet(now + Duration::from_secs(15)).is_some());
		assert!(pending.deadline().is_none());
		let mut changed = game(&"é".repeat(64));
		changed.details = Some("Next match".into());
		pending.update(&Some(changed.clone())).unwrap();
		changed.state = Some("x".repeat(129));
		assert_eq!(pending.update(&Some(changed)), Err(Failure::Protocol));
		assert_eq!(
			packet(pending.packet(now + Duration::from_secs(20)).unwrap())["d"]["activities"][0]["details"],
			"Next match"
		);
	}
}
