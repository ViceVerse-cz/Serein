//! Only identity, scope and bounded status/activity metadata survive presence decoding.
use crate::DecodeError;
use model::{ActivityImage, Id, MAX_RICH_ACTIVITIES, Patch, RichActivity};
use serde::{
	Deserialize, Deserializer,
	de::{MapAccess, SeqAccess, Visitor},
};
use std::fmt;

pub struct PresenceUpdate {
	pub guild: Option<Id>,
	pub user: Id,
	pub status: Patch<String>,
	pub custom_status: Patch<String>,
	pub activities: Patch<Vec<RichActivity>>,
}

#[derive(Deserialize)]
pub struct Friends<'a>(
	#[serde(borrow, deserialize_with = "crate::read_state::entries")]
	pub  Vec<&'a serde_json::value::RawValue>,
);

#[derive(Deserialize)]
pub struct MergedPresences {
	#[serde(default)]
	pub friends: Option<Box<serde_json::value::RawValue>>,
}

#[derive(Deserialize)]
pub struct ApplicationIcon {
	pub id: Id,
	#[serde(default)]
	pub icon: Option<String>,
}

/// Activity payloads are consumed one at a time; only bounded display metadata survives.
#[derive(Default)]
pub struct Activities(pub(crate) Option<String>, pub Vec<RichActivity>);

// Derived structs also accept positional arrays; wire activities and emoji must be objects.
struct Object<T>(T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct MapOnly<T>(std::marker::PhantomData<T>);
		impl<'de, T: Deserialize<'de>> Visitor<'de> for MapOnly<T> {
			type Value = Object<T>;
			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("an activity or emoji object")
			}
			fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
				T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(Object)
			}
		}
		d.deserialize_map(MapOnly(std::marker::PhantomData))
	}
}

struct Text<const N: usize>(String);
impl<'de, const N: usize> Deserialize<'de> for Text<N> {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct Bounded<const N: usize>;
		impl<const N: usize> Visitor<'_> for Bounded<N> {
			type Value = Text<N>;
			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("bounded activity text")
			}
			fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
				if value.len() > N {
					return Err(E::custom("Activity text exceeds capacity"));
				}
				Ok(Text(value.to_owned()))
			}
		}
		d.deserialize_str(Bounded::<N>)
	}
}

#[derive(Deserialize)]
struct Activity {
	#[serde(rename = "type")]
	kind: u8,
	#[serde(default)]
	name: Option<Text<4096>>,
	#[serde(default)]
	details: Option<Text<4096>>,
	#[serde(default)]
	state: Option<Text<4096>>,
	#[serde(default)]
	emoji: Option<Object<Emoji>>,
	#[serde(default)]
	application_id: Option<Text<128>>,
	#[serde(default)]
	assets: Option<Object<Assets>>,
	#[serde(default)]
	timestamps: Option<Object<Timestamps>>,
}
#[derive(Deserialize)]
struct Timestamps {
	#[serde(default)]
	start: Option<u64>,
	#[serde(default)]
	end: Option<u64>,
}
#[derive(Deserialize)]
struct Assets {
	#[serde(default)]
	large_image: Option<Text<4096>>,
	#[serde(default)]
	small_image: Option<Text<4096>>,
}
#[derive(Deserialize)]
struct Emoji {
	#[serde(default)]
	name: Option<Text<128>>,
	#[serde(default)]
	id: Option<Id>,
}
impl Activity {
	fn images(&self) -> (Option<ActivityImage>, Option<ActivityImage>) {
		let application = self
			.application_id
			.as_ref()
			.and_then(|id| id.0.parse::<Id>().ok());
		let image = |asset: &Text<4096>| {
			let image = if let Some(path) = asset.0.strip_prefix("mp:") {
				Some(ActivityImage::Proxy(path.into()))
			} else if let Some(id) = asset.0.strip_prefix("spotify:") {
				Some(ActivityImage::Spotify(id.into()))
			} else {
				Some(ActivityImage::Asset {
					application: application?,
					asset: asset.0.parse().ok()?,
				})
			};
			image.filter(ActivityImage::valid)
		};
		let assets = self.assets.as_ref().map(|assets| &assets.0);
		// A small image is a corner badge. When no large image resolves, Discord falls back
		// to the application icon rather than blowing the badge up into the artwork.
		let primary = assets
			.and_then(|assets| assets.large_image.as_ref())
			.and_then(image)
			.or_else(|| application.map(ActivityImage::Application));
		let small = assets.and_then(|assets| match &assets.small_image {
			Some(asset) => image(asset),
			None if assets.large_image.as_ref().and_then(image).is_some() => {
				application.map(ActivityImage::Application)
			}
			None => None,
		});
		let small = small.filter(|small| primary.as_ref() != Some(small));
		(primary, small)
	}
	fn rich_activity(&self) -> Option<RichActivity> {
		if !matches!(self.kind, 0..=3 | 5) {
			return None;
		}
		let normalize = |text: &Text<4096>| {
			let text: String = text
				.0
				.trim()
				.chars()
				.filter(|c| !c.is_control())
				.take(128)
				.collect();
			let text = text.trim();
			(!text.is_empty()).then(|| text.to_owned())
		};
		let (image, small_image) = self.images();
		Some(RichActivity {
			kind: self.kind,
			name: self.name.as_ref().and_then(normalize)?,
			details: self.details.as_ref().and_then(normalize),
			state: self.state.as_ref().and_then(normalize),
			image,
			small_image,
			ends_at: self.timestamps.as_ref().and_then(|timestamps| {
				let start = timestamps.0.start?;
				timestamps
					.0
					.end
					.filter(|end| *end > start && *end <= model::MAX_ACTIVITY_TIMESTAMP)
			}),
			started_at: self
				.timestamps
				.as_ref()
				.and_then(|timestamps| timestamps.0.start)
				.filter(|at| *at <= model::MAX_ACTIVITY_TIMESTAMP),
		})
	}
	fn custom_status(&self) -> Option<String> {
		let emoji = self
			.emoji
			.as_ref()
			.map(|emoji| &emoji.0)
			.filter(|e| e.id.is_none())
			.and_then(|e| e.name.as_ref())
			.map(|name| name.0.as_str())
			.filter(|name| name.chars().count() <= 8);
		let state = self.state.as_ref().map(|s| s.0.trim()).unwrap_or_default();
		let text: String = emoji
			.into_iter()
			.flat_map(str::chars)
			.chain((emoji.is_some() && !state.is_empty()).then_some(' '))
			.chain(state.chars())
			.filter(|c| !c.is_control())
			.take(128)
			.collect();
		let text = text.trim();
		(!text.is_empty()).then(|| text.to_owned())
	}
}
impl<'de> Deserialize<'de> for Activities {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct Bounded;
		impl<'de> Visitor<'de> for Bounded {
			type Value = Activities;
			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("at most 16 activity objects")
			}
			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Activities, A::Error> {
				let mut custom = None;
				let mut rich: Vec<RichActivity> = Vec::new();
				let richness = |activity: &RichActivity| {
					(
						u8::from(activity.details.is_some()) + u8::from(activity.state.is_some()),
						match &activity.image {
							Some(
								ActivityImage::Asset { .. }
								| ActivityImage::Proxy(_)
								| ActivityImage::Spotify(_),
							) => 2,
							Some(ActivityImage::Application(_)) => 1,
							None => 0,
						},
						match &activity.small_image {
							Some(
								ActivityImage::Asset { .. }
								| ActivityImage::Proxy(_)
								| ActivityImage::Spotify(_),
							) => 2,
							Some(ActivityImage::Application(_)) => 1,
							None => 0,
						},
						activity.started_at.is_some(),
					)
				};
				let mut found = false;
				let mut count = 0;
				while let Some(Object(activity)) = seq.next_element::<Object<Activity>>()? {
					if count == 16 {
						return Err(serde::de::Error::custom("Activity count exceeds capacity"));
					}
					count += 1;
					if activity.kind == 4 && !found {
						custom = activity.custom_status();
						found = true;
					} else if let Some(activity) = activity.rich_activity() {
						// ponytail: match display names; retain application identity if same-name games need separating.
						if let Some(existing) = rich.iter_mut().find(|existing| {
							activity.kind == 0
								&& existing.kind == 0 && existing
								.name
								.eq_ignore_ascii_case(&activity.name)
						}) {
							if richness(&activity) > richness(existing) {
								*existing = activity;
							}
						} else if rich.len() < MAX_RICH_ACTIVITIES {
							rich.push(activity);
						}
					}
				}
				Ok(Activities(custom, rich))
			}
		}
		d.deserialize_seq(Bounded)
	}
}

#[derive(Deserialize)]
struct Identity {
	id: Id,
}

#[derive(Deserialize)]
struct PresenceDto {
	#[serde(default)]
	guild_id: Option<Id>,
	#[serde(default)]
	user: Option<Identity>,
	#[serde(default)]
	user_id: Option<Id>,
	#[serde(default)]
	status: Patch<String>,
	#[serde(default)]
	activities: Patch<Activities>,
}

pub fn decode(bytes: &[u8]) -> Result<PresenceUpdate, DecodeError> {
	let presence: PresenceDto = crate::decode(bytes)?;
	let embedded = presence.user.map(|user| user.id);
	if embedded.zip(presence.user_id).is_some_and(|(a, b)| a != b) {
		return Err(DecodeError);
	}
	let user = embedded
		.or(presence.user_id)
		.filter(|id| id.0 != 0)
		.ok_or(DecodeError)?;
	let status = match presence.status {
		Patch::Value(status)
			if matches!(status.as_str(), "online" | "idle" | "dnd" | "offline") =>
		{
			Patch::Value(status)
		}
		Patch::Absent => Patch::Absent,
		_ => Patch::Null,
	};
	let (custom_status, activities) = match presence.activities {
		Patch::Absent => (Patch::Absent, Patch::Absent),
		Patch::Null => (Patch::Null, Patch::Null),
		Patch::Value(Activities(custom, rich)) => {
			(custom.map_or(Patch::Null, Patch::Value), Patch::Value(rich))
		}
	};
	Ok(PresenceUpdate {
		guild: presence.guild_id,
		user,
		status,
		custom_status,
		activities,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn update(activities: &str) -> Result<PresenceUpdate, DecodeError> {
		decode(format!(r#"{{"user":{{"id":"2"}},"activities":{activities}}}"#).as_bytes())
	}

	#[test]
	fn activity_artwork_selection_and_snapshot_updates_agree() {
		for (fields, image) in [
			(
				r#""application_id":"10","assets":{"large_image":"20","small_image":"30"}"#,
				Some(ActivityImage::Asset {
					application: Id(10),
					asset: Id(20),
				}),
			),
			// A badge alone must not become the artwork; the application icon does.
			(
				r#""application_id":"10","assets":{"small_image":"30"}"#,
				Some(ActivityImage::Application(Id(10))),
			),
			(
				r#""application_id":"10""#,
				Some(ActivityImage::Application(Id(10))),
			),
			(
				r#""assets":{"large_image":"mp:external/synthetic-hash-01/https/example.com/art.png"}"#,
				Some(ActivityImage::Proxy(
					"external/synthetic-hash-01/https/example.com/art.png".into(),
				)),
			),
			(
				r#""application_id":"10","assets":{"large_image":"https://example.com/raw.png","small_image":"30"}"#,
				Some(ActivityImage::Application(Id(10))),
			),
			(r#""assets":{"large_image":"20"}"#, None),
			(
				r#""assets":{"large_image":"https://example.com/raw.png"}"#,
				None,
			),
			(
				r#""application_id":"0","assets":{"large_image":"20"}"#,
				None,
			),
			(
				r#""application_id":"10","assets":{"large_image":"0"}"#,
				Some(ActivityImage::Application(Id(10))),
			),
			(r#""application_id":null,"assets":null"#, None),
		] {
			let wire = format!(r#"[{{"type":0,"name":"Synthetic",{fields}}}]"#);
			let Patch::Value(activities) = update(&wire).unwrap().activities else {
				panic!()
			};
			assert_eq!(activities[0].image, image, "{fields}");
			assert!(activities[0].valid());
			let snapshot: crate::PresenceDto =
				crate::decode(format!(r#"{{"status":"online","activities":{wire}}}"#).as_bytes())
					.unwrap();
			assert_eq!(snapshot.activities.1, activities);
		}
	}

	#[test]
	fn activity_badges_and_start_timestamps_are_retained_and_bounded() {
		for (assets, small_image) in [
			(
				serde_json::json!({"large_image":"20","small_image":"30"}),
				Some(ActivityImage::Asset {
					application: Id(10),
					asset: Id(30),
				}),
			),
			(
				serde_json::json!({"large_image":"20"}),
				Some(ActivityImage::Application(Id(10))),
			),
			(
				serde_json::json!({"small_image":"30"}),
				Some(ActivityImage::Asset {
					application: Id(10),
					asset: Id(30),
				}),
			),
			(
				serde_json::json!({"large_image":"20","small_image":"20"}),
				None,
			),
			(
				serde_json::json!({"large_image":"20","small_image":"mp:external/small/https/example.com/icon.png"}),
				Some(ActivityImage::Proxy(
					"external/small/https/example.com/icon.png".into(),
				)),
			),
			(
				serde_json::json!({"large_image":"20","small_image":"mp:external/../secret"}),
				None,
			),
			(serde_json::json!({"large_image":"invalid"}), None),
		] {
			let wire = serde_json::json!([{"type":0,"name":"Synthetic","application_id":"10","assets":assets,"timestamps":{"start":1_700_000_000_000_u64}}]).to_string();
			let Patch::Value(activities) = update(&wire).unwrap().activities else {
				panic!()
			};
			assert_eq!(activities[0].small_image, small_image, "{assets}");
			assert_eq!(activities[0].started_at, Some(1_700_000_000_000));
			assert!(activities[0].valid());
		}
		for (start, expected) in [
			(0, Some(0)),
			(
				model::MAX_ACTIVITY_TIMESTAMP,
				Some(model::MAX_ACTIVITY_TIMESTAMP),
			),
			(model::MAX_ACTIVITY_TIMESTAMP + 1, None),
		] {
			let wire =
				serde_json::json!([{"type":0,"name":"Synthetic","timestamps":{"start":start}}])
					.to_string();
			let Patch::Value(activities) = update(&wire).unwrap().activities else {
				panic!()
			};
			assert_eq!(activities[0].started_at, expected);
		}
		for timestamps in [r#"{"start":-1}"#, r#"{"start":"1"}"#, "[]"] {
			assert!(
				update(&format!(
					r#"[{{"type":0,"name":"Synthetic","timestamps":{timestamps}}}]"#
				))
				.is_err()
			);
		}
	}

	#[test]
	fn invalid_activity_proxy_paths_fall_back_without_losing_text() {
		for path in [
			"",
			"/external/a",
			"../a",
			"external/../a",
			"external/./a",
			"external/%2e%2E/a",
			"external/%2fa",
			"external/%5Ca",
			"external/\\a",
			"external/\na",
			&"x".repeat(1025),
		] {
			let wire = serde_json::json!([{"type":0,"name":"Synthetic","application_id":"10","assets":{"large_image":format!("mp:{path}")}}]).to_string();
			let Patch::Value(activities) = update(&wire).unwrap().activities else {
				panic!()
			};
			assert_eq!(
				activities[0].image,
				Some(ActivityImage::Application(Id(10))),
				"{path}"
			);
			assert_eq!(activities[0].name, "Synthetic");
		}
		let wire = serde_json::json!([{"type":0,"name":"Synthetic","assets":{"large_image":format!("mp:{}", "x".repeat(1024))}}]).to_string();
		let Patch::Value(activities) = update(&wire).unwrap().activities else {
			panic!()
		};
		assert!(
			matches!(&activities[0].image, Some(ActivityImage::Proxy(path)) if path.len() == 1024)
		);
		for field in ["large_image", "small_image"] {
			let wire = serde_json::json!([{"type":0,"name":"Synthetic","assets":{field:"x".repeat(4097)}}]).to_string();
			assert!(update(&wire).is_err());
		}
	}

	#[test]
	fn rich_activities_share_snapshot_normalization_types_and_patch_semantics() {
		assert_eq!(
			decode(br#"{"user":{"id":"2"}}"#).unwrap().activities,
			Patch::Absent
		);
		assert_eq!(update("null").unwrap().activities, Patch::Null);
		assert_eq!(update("[]").unwrap().activities, Patch::Value(vec![]));
		for (kind, summary) in [
			(0, "Playing Synthetic"),
			(1, "Streaming Synthetic"),
			(2, "Listening to Synthetic"),
			(3, "Watching Synthetic"),
			(5, "Competing in Synthetic"),
		] {
			let wire = format!(
				r#"[{{"type":{kind},"name":" \nSynthetic\t ","details":" Level\n 2 ","state":" \u0000 "}},{{"type":4,"state":"Custom"}}]"#
			);
			let Patch::Value(activities) = update(&wire).unwrap().activities else {
				panic!()
			};
			assert_eq!(
				activities,
				vec![RichActivity {
					kind,
					name: "Synthetic".into(),
					details: Some("Level 2".into()),
					state: None,
					image: None,
					small_image: None,
					ends_at: None,
					started_at: None,
				}]
			);
			assert!(activities[0].valid());
			assert_eq!(activities[0].summary(), summary);
			let snapshot: crate::MemberItem = crate::decode(format!(r#"{{"member":{{"user":{{"id":"2","username":"Synthetic"}},"presence":{{"status":"online","activities":{wire}}}}}}}"#).as_bytes()).unwrap();
			let member = snapshot.into_model().unwrap();
			assert_eq!(member.activities, activities);
			assert_eq!(member.custom_status.as_deref(), Some("Custom"));
			assert!(member.valid());
		}
		for wire in [
			r#"[{"type":6,"name":"Future"}]"#,
			r#"[{"type":0,"name":" \n "}]"#,
			r#"[{"type":0}]"#,
		] {
			assert_eq!(update(wire).unwrap().activities, Patch::Value(vec![]));
		}
	}

	#[test]
	fn matching_games_keep_the_richer_entry_in_snapshots_and_updates() {
		let basic = serde_json::json!({"type":0,"name":" osu! ","application_id":"10"});
		let detailed = serde_json::json!({"type":0,"name":"OSU!","state":"Idle","application_id":"20","assets":{"large_image":"30"}});
		for pair in [
			[basic.clone(), detailed.clone()],
			[detailed.clone(), basic.clone()],
		] {
			let wire = serde_json::json!([
				pair[0], {"type":0,"name":"Terraria"},
				{"type":2,"name":"osu!"}, {"type":0,"name":"Dota 2"}, pair[1]
			])
			.to_string();
			let Patch::Value(activities) = update(&wire).unwrap().activities else {
				panic!()
			};
			assert_eq!(activities.len(), MAX_RICH_ACTIVITIES);
			assert_eq!(activities[0].state.as_deref(), Some("Idle"));
			assert_eq!(
				activities[0].image,
				Some(ActivityImage::Asset {
					application: Id(20),
					asset: Id(30)
				})
			);
			assert_eq!(activities[1].name, "Terraria");
			assert_eq!(activities[2].kind, 2);
			let snapshot: crate::MemberItem = crate::decode(format!(r#"{{"member":{{"user":{{"id":"2","username":"Synthetic"}},"presence":{{"status":"online","activities":{wire}}}}}}}"#).as_bytes()).unwrap();
			assert_eq!(snapshot.into_model().unwrap().activities, activities);
		}
		let mut artwork_only = detailed;
		artwork_only.as_object_mut().unwrap().remove("state");
		let Patch::Value(activities) =
			update(&serde_json::json!([basic, artwork_only]).to_string())
				.unwrap()
				.activities
		else {
			panic!()
		};
		assert_eq!(activities.len(), 1);
		assert!(matches!(
			activities[0].image,
			Some(ActivityImage::Asset { .. })
		));
	}

	#[test]
	fn matching_games_prefer_explicit_badges_then_start_timestamps() {
		let basic = serde_json::json!({"type":0,"name":"Synthetic","application_id":"10","assets":{"large_image":"20"}});
		let mut badge = basic.clone();
		badge["assets"]["small_image"] = "30".into();
		let mut timed = badge.clone();
		timed["timestamps"] = serde_json::json!({"start":1_700_000_000_000_u64});
		for (less, more) in [(basic, badge.clone()), (badge, timed)] {
			for pair in [[&less, &more], [&more, &less]] {
				let Patch::Value(actual) = update(&serde_json::json!(pair).to_string())
					.unwrap()
					.activities
				else {
					panic!()
				};
				let expected = update(&serde_json::json!([more]).to_string())
					.unwrap()
					.activities;
				assert_eq!(Patch::Value(actual), expected);
			}
		}
	}

	#[test]
	fn rich_activity_retention_and_wire_fields_are_bounded() {
		let text = "🌙".repeat(1024);
		let wire = format!(r#"[{{"type":0,"name":"{text}","details":"{text}","state":"{text}"}}]"#);
		let Patch::Value(activities) = update(&wire).unwrap().activities else {
			panic!()
		};
		let activity = &activities[0];
		for text in [
			&activity.name,
			activity.details.as_ref().unwrap(),
			activity.state.as_ref().unwrap(),
		] {
			assert_eq!(text.len(), 512);
			assert_eq!(text.chars().count(), 128);
		}
		assert!(activity.valid());
		for field in ["name", "details", "state"] {
			let wire = format!(r#"[{{"type":0,"{field}":"{}"}}]"#, "x".repeat(4097));
			assert!(update(&wire).is_err());
			assert!(update(&format!(r#"[{{"type":0,"{field}":42}}]"#)).is_err());
		}
		let wire = format!(
			"[{}]",
			(0..16)
				.map(|index| format!(r#"{{"type":0,"name":"Game {index}"}}"#))
				.collect::<Vec<_>>()
				.join(",")
		);
		let Patch::Value(activities) = update(&wire).unwrap().activities else {
			panic!()
		};
		assert_eq!(activities.len(), MAX_RICH_ACTIVITIES);
		assert_eq!(activities.last().unwrap().name, "Game 3");
	}

	#[test]
	fn offline_member_snapshot_cannot_retain_activity_or_custom_status() {
		let snapshot: crate::MemberItem = crate::decode(br#"{"member":{"user":{"id":"2","username":"Synthetic"},"presence":{"status":"offline","activities":[{"type":0,"name":"Old game"},{"type":4,"state":"Old custom status"}]}}}"#).unwrap();
		let member = snapshot.into_model().unwrap();
		assert_eq!(member.status.as_deref(), Some("offline"));
		assert!(member.custom_status.is_none());
		assert!(member.activities.is_empty());
	}

	#[test]
	fn custom_status_preserves_absence_and_clears_explicit_missing_activity() {
		for wire in [
			r#"{"user":{"id":"2"}}"#,
			r#"{"user":{"id":"2"},"status":"offline"}"#,
		] {
			assert_eq!(
				decode(wire.as_bytes()).unwrap().custom_status,
				Patch::Absent
			);
		}
		for activities in [
			"null",
			"[]",
			r#"[{"type":0,"state":"Not a custom status"}]"#,
			r#"[{"type":4}]"#,
			r#"[{"type":4,"state":null,"emoji":null}]"#,
			r#"[{"type":4,"state":" \n\t "}]"#,
			r#"[{"type":4,"emoji":{"name":"custom","id":"123"}}]"#,
		] {
			let presence = update(activities).unwrap();
			assert_eq!(presence.custom_status, Patch::Null, "{activities}");
			assert_eq!(presence.status, Patch::Absent);
		}
	}

	#[test]
	fn snapshots_and_updates_share_bounded_unicode_custom_status_normalization() {
		for (activities, expected) in [
			(
				r#"[{"type":0,"state":"Ignored"},{"type":4,"state":" Rest\ning \t","emoji":{"name":"\ud83c\udf19","id":null}}]"#,
				Some("🌙 Resting"),
			),
			(
				r#"[{"type":4,"emoji":{"name":"\ud83c\udf19"}}]"#,
				Some("🌙"),
			),
			(
				r#"[{"type":4,"state":"Text","emoji":{"name":"custom","id":"123"}}]"#,
				Some("Text"),
			),
			(
				r#"[{"type":4,"state":"Text","emoji":{"name":"123456789"}}]"#,
				Some("Text"),
			),
			(
				r#"[{"type":4,"state":"First"},{"type":4,"state":"Second"}]"#,
				Some("First"),
			),
			(r#"[{"type":4},{"type":4,"state":"Second"}]"#, None),
		] {
			let snapshot: crate::PresenceDto = crate::decode(
				format!(r#"{{"status":"online","activities":{activities}}}"#).as_bytes(),
			)
			.unwrap();
			assert_eq!(snapshot.custom_status().as_deref(), expected);
			assert_eq!(
				update(activities).unwrap().custom_status,
				expected.map_or(Patch::Null, |s| Patch::Value(s.into()))
			);
		}
		let activities = format!(r#"[{{"type":4,"state":"{}"}}]"#, "🌙".repeat(1024));
		let Patch::Value(text) = update(&activities).unwrap().custom_status else {
			panic!()
		};
		assert_eq!(text.chars().count(), 128);
		assert_eq!(text.len(), 512);
		let snapshot: crate::PresenceDto =
			crate::decode(format!(r#"{{"status":"online","activities":{activities}}}"#).as_bytes())
				.unwrap();
		assert_eq!(snapshot.custom_status().as_deref(), Some(text.as_str()));
		// Secrets and unsupported assets are ignored instead of surviving in application state.
		let rich = format!(
			r#"[{{"type":0,"secrets":{{"join":"{}"}},"assets":{{"large_image":"unused"}}}},{{"type":4,"state":"Visible"}}]"#,
			"synthetic".repeat(4096)
		);
		assert_eq!(
			update(&rich).unwrap().custom_status,
			Patch::Value("Visible".into())
		);
	}

	#[test]
	fn filtering_and_truncation_leave_only_valid_trimmed_custom_statuses() {
		let truncated = format!(r#"[{{"type":4,"state":"{} trailing"}}]"#, "x".repeat(127));
		for (activities, expected) in [
			(r#"[{"type":4,"emoji":{"name":" \t "}}]"#.to_owned(), None),
			(
				r#"[{"type":4,"emoji":{"name":" \t "},"state":"Visible"}]"#.to_owned(),
				Some("Visible".to_owned()),
			),
			(
				r#"[{"type":4,"state":"\u0000  Visible  \u0000"}]"#.to_owned(),
				Some("Visible".to_owned()),
			),
			(truncated, Some("x".repeat(127))),
		] {
			let decoded = update(&activities).unwrap();
			assert_eq!(
				decoded.custom_status,
				expected.clone().map_or(Patch::Null, Patch::Value)
			);
			let snapshot: crate::PresenceDto = crate::decode(
				format!(r#"{{"status":"online","activities":{activities}}}"#).as_bytes(),
			)
			.unwrap();
			assert_eq!(snapshot.custom_status(), expected);
			assert!(
				model::MemberPresence {
					user: Id(2),
					status: None,
					custom_status: snapshot.custom_status(),
					activities: snapshot.activities.1,
				}
				.valid()
			);
		}
	}

	#[test]
	fn malformed_activity_shapes_and_resource_overflows_are_rejected() {
		for activities in [
			"42",
			"true",
			r#""text""#,
			"{}",
			"[null]",
			"[[]]",
			r#"[[4,"Array status",null]]"#,
			"[42]",
			"[{}]",
			r#"[{"type":"4"}]"#,
			r#"[{"type":256}]"#,
			r#"[{"type":4,"state":42}]"#,
			r#"[{"type":4,"emoji":[]}]"#,
			r#"[{"type":4,"emoji":["Array emoji",null]}]"#,
			r#"[{"type":4,"emoji":{"name":42}}]"#,
			r#"[{"type":4,"emoji":{"id":"0"}}]"#,
			r#"[{"type":4,"emoji":{"id":123}}]"#,
			r#"[{"type":4,"state":"First","state":"Second"}]"#,
		] {
			assert!(update(activities).is_err(), "{activities}");
			assert!(
				crate::decode::<crate::PresenceDto>(
					format!(r#"{{"status":"online","activities":{activities}}}"#).as_bytes(),
				)
				.is_err(),
				"{activities}"
			);
		}
		assert!(decode(br#"{"user":{"id":"2"},"activities":[],"activities":null}"#).is_err());
		let sixteen = format!("[{}]", [r#"{"type":0}"#; 16].join(","));
		assert_eq!(update(&sixteen).unwrap().custom_status, Patch::Null);
		for activities in [
			format!("[{}]", [r#"{"type":0}"#; 17].join(",")),
			format!(r#"[{{"type":4,"state":"{}"}}]"#, "x".repeat(4097)),
			format!(r#"[{{"type":4,"emoji":{{"name":"{}"}}}}]"#, "x".repeat(129)),
		] {
			assert!(update(&activities).is_err());
			assert!(
				crate::decode::<crate::PresenceDto>(
					format!(r#"{{"status":"online","activities":{activities}}}"#).as_bytes(),
				)
				.is_err()
			);
		}
	}

	#[test]
	fn partial_identity_and_status_patches_do_not_retain_unrelated_presence_data() {
		for status in ["online", "idle", "dnd", "offline"] {
			let bytes = format!(
				r#"{{"guild_id":"1","user":{{"id":"2"}},"status":"{status}","activities":[{{"type":0,"name":"Synthetic"}}],"client_status":{{"desktop":"dnd"}}}}"#
			);
			let presence = decode(bytes.as_bytes()).unwrap();
			assert_eq!(presence.guild, Some(Id(1)));
			assert_eq!(presence.user, Id(2));
			assert_eq!(presence.status, Patch::Value(status.into()));
		}
		let presence = decode(br#"{"user":{"id":"2","username":"Ignored"}}"#).unwrap();
		assert_eq!(presence.guild, None);
		assert_eq!(presence.status, Patch::Absent);
		assert_eq!(
			decode(br#"{"guild_id":null,"user":{"id":"2"},"status":null}"#)
				.unwrap()
				.status,
			Patch::Null
		);
		for status in ["invisible", "future-status", "ONLINE", ""] {
			let bytes = format!(r#"{{"user":{{"id":"2"}},"status":"{status}"}}"#);
			assert_eq!(decode(bytes.as_bytes()).unwrap().status, Patch::Null);
		}
		let huge = format!(
			r#"{{"user":{{"id":"2"}},"status":"{}"}}"#,
			"x".repeat(128 * 1024)
		);
		assert_eq!(decode(huge.as_bytes()).unwrap().status, Patch::Null);
		let oversized = format!(
			r#"{{"user":{{"id":"2"}},"status":"{}"}}"#,
			"x".repeat(crate::MAX_WIRE)
		);
		assert!(decode(oversized.as_bytes()).is_err());
	}

	#[test]
	fn malformed_identity_scope_or_status_is_rejected() {
		for bytes in [
			r#"{"user":{}}"#,
			r#"{"user":{"id":"0"}}"#,
			r#"{"user":{"id":2}}"#,
			r#"{"user":{"id":"18446744073709551616"}}"#,
			r#"{"user":{"id":"000000000000000000002"}}"#,
			r#"{"guild_id":"0","user":{"id":"2"}}"#,
			r#"{"guild_id":"bad","user":{"id":"2"}}"#,
			r#"{"guild_id":1,"user":{"id":"2"}}"#,
			r#"{"guild_id":"1","user":null}"#,
			r#"{"user":{"id":"2"},"status":42}"#,
			r#"{"user":{"id":"2"},"status":{"online":null}}"#,
			r#"{"user":{"id":"2"},"status":"online","status":"idle"}"#,
		] {
			assert!(decode(bytes.as_bytes()).is_err());
		}
		assert_eq!(
			decode(br#"{"user":{"id":"18446744073709551615"}}"#)
				.unwrap()
				.user,
			Id(u64::MAX)
		);
	}
}
