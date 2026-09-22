//! Bounded presence detection, without retaining unrendered message payloads.
use serde::{
	Deserialize, Deserializer,
	de::{IgnoredAny, MapAccess, Visitor},
};
use std::fmt;

const MAX_FIELDS: usize = 64;

pub struct Object;
impl<'de> Deserialize<'de> for Object {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct Presence;
		impl<'de> Visitor<'de> for Presence {
			type Value = Object;
			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				f.write_str("a bounded content object")
			}
			fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Object, A::Error> {
				let mut count = 0;
				while map.next_key::<IgnoredAny>()?.is_some() {
					if count == MAX_FIELDS {
						return Err(serde::de::Error::custom("Content object exceeds capacity"));
					}
					map.next_value::<IgnoredAny>()?;
					count += 1;
				}
				Ok(Object)
			}
		}
		d.deserialize_map(Presence)
	}
}

pub fn object_patch(value: model::Patch<Object>) -> model::Patch<bool> {
	match value {
		model::Patch::Absent => model::Patch::Absent,
		model::Patch::Null => model::Patch::Null,
		model::Patch::Value(_) => model::Patch::Value(true),
	}
}
#[cfg(test)]
mod tests {
	use crate::{MessageDto, PatchDto, decode};
	use model::Patch;

	fn wire(fields: &str) -> String {
		format!(
			r#"{{"id":"1","channel_id":"2","author":{{"id":"3","username":"Synthetic"}}{fields}}}"#
		)
	}

	#[test]
	fn normal_messages_retain_presence_without_retaining_payloads() {
		for (fields, bits) in [
			("", 0),
			(r#", "poll":{}"#, 1),
			(
				r#", "sticker_items":[{"id":"4","name":"Wave","format_type":1}]"#,
				2,
			),
			(
				r#", "stickers":[{"id":"5","name":"Synthetic","format_type":1}]"#,
				4,
			),
			(
				r#", "components":[{"type":1,"components":[{"type":2}]}]"#,
				8,
			),
			(r#", "flags":32768"#, 16),
			(
				r#", "poll":null,"sticker_items":[],"stickers":null,"components":[]"#,
				0,
			),
		] {
			let message = decode::<MessageDto>(wire(fields).as_bytes())
				.unwrap()
				.into_model();
			assert!(!message.unsupported);
			assert_eq!(message.extra_content.bits(), bits);
			assert!(message.content.is_empty());
		}
		let short = decode::<MessageDto>(wire(r#", "poll":{}"#).as_bytes())
			.unwrap()
			.into_model();
		let fields = format!(
			r#", "poll":{{"question":{{"text":"{}"}}}}"#,
			"x".repeat(64 * 1024)
		);
		let large = decode::<MessageDto>(wire(&fields).as_bytes())
			.unwrap()
			.into_model();
		assert_eq!(short.bytes(), large.bytes());
		let system = decode::<MessageDto>(wire(r#", "type":7,"poll":{},"flags":32772"#).as_bytes())
			.unwrap()
			.into_model();
		assert!(system.unsupported);
		assert!(system.embeds_suppressed);
		assert_eq!(system.extra_content.bits(), 17);
	}

	#[test]
	fn field_patches_preserve_absence_and_clear_only_explicit_sources() {
		let mut message = decode::<MessageDto>(
			wire(
				r#", "poll":{},"sticker_items":[{"id":"4","name":"Wave","format_type":1}],"stickers":[{"id":"5","name":"Synthetic","format_type":1}],"components":[{}],"flags":32772"#,
			)
			.as_bytes(),
		)
		.unwrap()
		.into_model();
		let first =
			decode::<PatchDto>(wire(r#", "poll":null,"components":[],"flags":4"#).as_bytes())
				.unwrap()
				.into_model();
		assert!(matches!(first.extra_content.poll, Patch::Null));
		assert!(matches!(first.extra_content.sticker_items, Patch::Absent));
		assert!(matches!(
			first.extra_content.components,
			Patch::Value(false)
		));
		assert!(matches!(first.embeds_suppressed, Patch::Value(true)));
		first.extra_content.apply(&mut message.extra_content);
		assert_eq!(message.extra_content.bits(), 2 | 4);
		let mut pending = first.extra_content;
		let second =
			decode::<PatchDto>(wire(r#", "poll":{},"sticker_items":null,"flags":null"#).as_bytes())
				.unwrap()
				.into_model();
		assert!(matches!(second.extra_content.components_v2, Patch::Null));
		assert!(matches!(second.embeds_suppressed, Patch::Null));
		pending.merge(&second.extra_content);
		let unrelated = decode::<PatchDto>(wire(r#", "content":"changed""#).as_bytes())
			.unwrap()
			.into_model();
		assert!(matches!(
			unrelated.extra_content.components_v2,
			Patch::Absent
		));
		pending.merge(&unrelated.extra_content);
		pending.apply(&mut message.extra_content);
		assert_eq!(message.extra_content.bits(), 1 | 4);
	}

	#[test]
	fn malformed_shapes_and_local_wire_limits_are_rejected() {
		for fields in [
			r#", "poll":[]"#,
			r#", "poll":true"#,
			r#", "components":{}"#,
			r#", "components":[null]"#,
			r#", "stickers":["payload"]"#,
			r#", "sticker_items":1"#,
		] {
			let bytes = wire(fields);
			assert!(decode::<MessageDto>(bytes.as_bytes()).is_err());
			assert!(decode::<PatchDto>(bytes.as_bytes()).is_err());
		}
		for count in [40, 41] {
			let fields = format!(r#", "components":[{}]"#, vec!["{}"; count].join(","));
			let bytes = wire(&fields);
			assert_eq!(decode::<MessageDto>(bytes.as_bytes()).is_ok(), count == 40);
			assert_eq!(decode::<PatchDto>(bytes.as_bytes()).is_ok(), count == 40);
		}
		for count in [64, 65] {
			let fields = (0..count)
				.map(|n| format!(r#""field{n}":null"#))
				.collect::<Vec<_>>()
				.join(",");
			let bytes = wire(&format!(r#", "poll":{{{fields}}}"#));
			assert_eq!(decode::<MessageDto>(bytes.as_bytes()).is_ok(), count == 64);
			assert_eq!(decode::<PatchDto>(bytes.as_bytes()).is_ok(), count == 64);
		}
		let bytes = wire(&format!(
			r#", "poll":{{"payload":"{}"}}"#,
			"x".repeat(crate::MAX_WIRE)
		));
		assert!(decode::<MessageDto>(bytes.as_bytes()).is_err());
		assert!(decode::<PatchDto>(bytes.as_bytes()).is_err());
	}
}

#[cfg(test)]
mod component_bounds {
	#[test]
	fn preserves_defaults_unknown_types_and_bounds_each_subtree() {
		let parsed: model::ComponentList = crate::decode(br#"[{"type":18,"component":{"type":4,"custom_id":"text"}},{"type":23,"default":true},{"type":250}]"#).unwrap();
		assert!(parsed.0[0].component.as_ref().unwrap().required);
		assert!(parsed.0[1].default);
		assert_eq!(parsed.0[2].kind, 250);
		let mut component = serde_json::json!({"type":2});
		for _ in 0..model::MAX_COMPONENT_DEPTH {
			component = serde_json::json!({"type":17,"components":[component]});
		}
		assert!(
			crate::decode::<model::ComponentList>(
				serde_json::to_string(&vec![component]).unwrap().as_bytes()
			)
			.is_err()
		);
	}
}
