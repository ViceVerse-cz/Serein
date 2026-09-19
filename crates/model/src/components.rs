//! Bounded service component metadata. URLs never authorize fetching or navigation.
use crate::Id;
use serde::{Deserialize, Serialize};

pub const MAX_COMPONENTS: usize = 40;
pub const MAX_COMPONENT_BYTES: usize = 128 * 1024;
pub const MAX_COMPONENT_DEPTH: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentEmoji {
	pub id: Option<Id>,
	pub name: Option<String>,
	pub animated: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentOption {
	pub label: String,
	pub value: String,
	pub description: Option<String>,
	pub emoji: Option<ComponentEmoji>,
	pub default: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComponentDefaultValue {
	pub id: Id,
	#[serde(rename = "type")]
	pub kind: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentMedia {
	pub url: String,
	pub proxy_url: Option<String>,
	pub height: Option<u32>,
	pub width: Option<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentMediaItem {
	pub media: ComponentMedia,
	pub description: Option<String>,
	pub spoiler: bool,
}
/// Unknown numeric kinds retain a visible fallback without preserving arbitrary JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, remote = "Self")]
pub struct Component {
	#[serde(rename = "type")]
	pub kind: u8,
	pub id: u32,
	pub custom_id: Option<String>,
	pub label: Option<String>,
	pub description: Option<String>,
	pub content: Option<String>,
	pub placeholder: Option<String>,
	pub url: Option<String>,
	#[serde(deserialize_with = "component_value")]
	pub value: Option<String>,
	#[serde(deserialize_with = "bounded_list")]
	pub values: Vec<String>,
	#[serde(deserialize_with = "bounded_list")]
	pub file_types: Vec<String>,
	pub checked: Option<bool>,
	pub default: bool,
	pub style: Option<u8>,
	pub disabled: bool,
	#[serde(default = "required_by_default")]
	pub required: bool,
	pub spoiler: bool,
	pub min_values: Option<u16>,
	pub max_values: Option<u16>,
	pub min_length: Option<u16>,
	pub max_length: Option<u16>,
	pub emoji: Option<ComponentEmoji>,
	#[serde(deserialize_with = "bounded_list")]
	pub options: Vec<ComponentOption>,
	#[serde(deserialize_with = "bounded_list")]
	pub default_values: Vec<ComponentDefaultValue>,
	#[serde(deserialize_with = "bounded_list")]
	pub channel_types: Vec<u8>,
	#[serde(deserialize_with = "bounded_list")]
	pub components: Vec<Component>,
	pub accessory: Option<Box<Component>>,
	pub component: Option<Box<Component>>,
	pub media: Option<ComponentMedia>,
	pub file: Option<ComponentMedia>,
	#[serde(deserialize_with = "bounded_list")]
	pub items: Vec<ComponentMediaItem>,
	pub sku_id: Option<Id>,
	pub accent_color: Option<u32>,
	pub divider: Option<bool>,
	pub spacing: Option<u8>,
}
// Validate each completed subtree during decoding, so a large input cannot allocate an
// unbounded number of nodes before the final list-level validation runs.
impl<'de> Deserialize<'de> for Component {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		let component = Self::deserialize(d)?;
		let mut count = 0;
		if !valid_component(&component, 1, &mut count) || component.bytes() > MAX_COMPONENT_BYTES {
			return Err(serde::de::Error::custom("Component exceeds capacity"));
		}
		Ok(component)
	}
}
impl Serialize for Component {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		Self::serialize(self, s)
	}
}
fn bounded_list<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
	d: D,
) -> Result<Vec<T>, D::Error> {
	struct List<T>(std::marker::PhantomData<T>);
	impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for List<T> {
		type Value = Vec<T>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("a bounded component array")
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
			let mut values = Vec::new();
			while let Some(value) = seq.next_element()? {
				if values.len() == MAX_COMPONENTS {
					return Err(serde::de::Error::custom("Too many component entries"));
				}
				values.push(value);
			}
			Ok(values.into_boxed_slice().into_vec())
		}
	}
	d.deserialize_seq(List(std::marker::PhantomData))
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentList(pub Vec<Component>);
impl<'de> Deserialize<'de> for ComponentList {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		let components = bounded_list(d)?;
		if !valid_components(&components) {
			return Err(serde::de::Error::custom("Components exceed capacity"));
		}
		Ok(Self(components))
	}
}
fn text_bytes(text: &Option<String>) -> usize {
	text.as_ref().map_or(0, String::capacity)
}
impl ComponentEmoji {
	fn bytes(&self) -> usize {
		text_bytes(&self.name)
	}
	fn valid(&self) -> bool {
		bounded_text(&self.name, 128) && self.id.is_none_or(|id| id.0 != 0)
	}
}
impl ComponentMedia {
	fn bytes(&self) -> usize {
		self.url.capacity() + text_bytes(&self.proxy_url)
	}
	fn valid(&self) -> bool {
		self.url.len() <= 4096 && bounded_text(&self.proxy_url, 4096)
	}
}
impl Component {
	/// Discord file filters match filename extensions, never sniffed contents.
	pub fn accepts_file(&self, filename: &str) -> bool {
		if self.file_types.is_empty() {
			return true;
		}
		let Some((_, extension)) = filename.rsplit_once('.') else {
			return false;
		};
		self.file_types.iter().any(|kind| {
			if let Some(expected) = kind.strip_prefix('.') {
				return extension.eq_ignore_ascii_case(expected);
			}
			let extensions: &[&str] = match kind.as_str() {
				"image" => &[
					"png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "tif", "tiff", "svg",
					"ico", "heic", "heif",
				],
				"video" => &[
					"mp4", "webm", "mov", "mkv", "avi", "m4v", "mpeg", "mpg", "3gp", "ogv",
				],
				"audio" => &[
					"mp3", "wav", "ogg", "opus", "flac", "m4a", "aac", "wma", "aiff", "aif",
				],
				_ => &[],
			};
			extensions
				.iter()
				.any(|expected| extension.eq_ignore_ascii_case(expected))
		})
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ [
				&self.custom_id,
				&self.label,
				&self.description,
				&self.content,
				&self.placeholder,
				&self.url,
				&self.value,
			]
			.into_iter()
			.map(text_bytes)
			.sum::<usize>()
			+ self.emoji.as_ref().map_or(0, ComponentEmoji::bytes)
			+ self.options.capacity() * size_of::<ComponentOption>()
			+ self
				.options
				.iter()
				.map(|o| {
					o.label.capacity()
						+ o.value.capacity()
						+ text_bytes(&o.description)
						+ o.emoji.as_ref().map_or(0, ComponentEmoji::bytes)
				})
				.sum::<usize>()
			+ self.default_values.capacity() * size_of::<ComponentDefaultValue>()
			+ self
				.default_values
				.iter()
				.map(|d| d.kind.capacity())
				.sum::<usize>()
			+ self.file_types.capacity() * size_of::<String>()
			+ self.file_types.iter().map(String::capacity).sum::<usize>()
			+ self.values.capacity() * size_of::<String>()
			+ self.values.iter().map(String::capacity).sum::<usize>()
			+ self.channel_types.capacity()
			+ component_bytes(&self.components)
			+ self.accessory.as_ref().map_or(0, |c| c.bytes())
			+ self.component.as_ref().map_or(0, |c| c.bytes())
			+ self.media.as_ref().map_or(0, ComponentMedia::bytes)
			+ self.file.as_ref().map_or(0, ComponentMedia::bytes)
			+ self.items.capacity() * size_of::<ComponentMediaItem>()
			+ self
				.items
				.iter()
				.map(|i| i.media.bytes() + text_bytes(&i.description))
				.sum::<usize>()
	}
	pub fn find(&self, custom_id: &str) -> Option<&Self> {
		if self.custom_id.as_deref() == Some(custom_id) {
			return Some(self);
		}
		self.components
			.iter()
			.chain(self.accessory.as_deref())
			.chain(self.component.as_deref())
			.find_map(|c| c.find(custom_id))
	}
}
pub fn component_bytes(components: &Vec<Component>) -> usize {
	components.capacity().saturating_sub(components.len()) * size_of::<Component>()
		+ components.iter().map(Component::bytes).sum::<usize>()
}
fn bounded_text(text: &Option<String>, maximum: usize) -> bool {
	text.as_ref().is_none_or(|text| text.len() <= maximum)
}
fn option_text(text: &str) -> bool {
	text.len() <= 400 && text.chars().count() <= 100
}
fn valid_component(c: &Component, depth: usize, count: &mut usize) -> bool {
	*count += 1;
	depth <= MAX_COMPONENT_DEPTH
		&& *count <= MAX_COMPONENTS
		&& c.file_types.len() <= 10
		&& c.file_types.iter().all(|kind| {
			kind.len() <= 32
				&& !kind.is_empty()
				&& kind.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
		}) && c.values.len() <= 25
		&& c.values.iter().all(|v| option_text(v))
		&& c.components.len() <= MAX_COMPONENTS
		&& c.options.len() <= 25
		&& c.default_values.len() <= 25
		&& c.items.len() <= 10
		&& c.channel_types.len() <= 40
		&& c.custom_id.as_ref().is_none_or(|s| option_text(s))
		&& bounded_text(&c.content, 16 * 1024)
		&& bounded_text(&c.value, 16 * 1024)
		&& bounded_text(&c.label, 1024)
		&& bounded_text(&c.description, 4096)
		&& bounded_text(&c.placeholder, 600)
		&& bounded_text(&c.url, 4096)
		&& c.emoji.as_ref().is_none_or(ComponentEmoji::valid)
		&& c.options.iter().all(|o| {
			option_text(&o.label)
				&& option_text(&o.value)
				&& o.description.as_ref().is_none_or(|s| option_text(s))
				&& o.emoji.as_ref().is_none_or(ComponentEmoji::valid)
		}) && c
		.default_values
		.iter()
		.all(|v| v.id.0 != 0 && v.kind.len() <= 32)
		&& c.media.as_ref().is_none_or(ComponentMedia::valid)
		&& c.file.as_ref().is_none_or(ComponentMedia::valid)
		&& c.items
			.iter()
			.all(|i| i.media.valid() && bounded_text(&i.description, 4096))
		&& c.components
			.iter()
			.chain(c.accessory.as_deref())
			.chain(c.component.as_deref())
			.all(|c| valid_component(c, depth + 1, count))
}
pub fn valid_components(components: &Vec<Component>) -> bool {
	let mut count = 0;
	components.len() <= MAX_COMPONENTS
		&& components.iter().all(|c| valid_component(c, 1, &mut count))
		&& component_bytes(components) <= MAX_COMPONENT_BYTES
}

fn required_by_default() -> bool {
	true
}

// Checkbox response values are booleans; schema values for text inputs are strings.
fn component_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
	#[derive(Deserialize)]
	#[serde(untagged)]
	enum Value {
		Text(String),
		Checked(bool),
	}
	Ok(Option::<Value>::deserialize(d)?.map(|v| match v {
		Value::Text(s) => s,
		Value::Checked(b) => b.to_string(),
	}))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn component_tree_limits_count_depth_and_owned_bytes() {
		let button = Component {
			kind: 2,
			custom_id: Some("synthetic".into()),
			..Component::default()
		};
		assert!(valid_components(&vec![button.clone()]));
		let mut with_slack = Vec::with_capacity(64);
		with_slack.resize(40, button.clone());
		assert!(valid_components(&with_slack));
		let mut selected = Component {
			kind: 3,
			..Default::default()
		};
		for n in 0..25 {
			selected.values.push(n.to_string());
		}
		assert!(valid_components(&vec![selected]));
		assert!(!valid_components(&vec![button.clone(); MAX_COMPONENTS + 1]));
		let mut nested = button;
		for _ in 0..MAX_COMPONENT_DEPTH {
			nested = Component {
				kind: 17,
				components: vec![nested],
				..Component::default()
			};
		}
		assert!(!valid_components(&vec![nested]));
		assert!(!valid_components(&vec![Component {
			content: Some("x".repeat(MAX_COMPONENT_BYTES)),
			..Component::default()
		}]));
	}
}
