//! Minimal author-side ABI. This crate belongs inside the sandbox, not in the host.
use serde::{Deserialize, Serialize};
pub use serde_json;
use std::collections::BTreeMap;

#[derive(Default, Deserialize)]
pub struct Invocation {
	pub action: String,
	#[serde(default)]
	pub selected_message: Option<String>,
	#[serde(default)]
	pub composer: Option<String>,
	#[serde(default)]
	pub storage: Option<String>,
	#[serde(default)]
	pub values: BTreeMap<String, String>,
}

#[derive(Default, Serialize)]
pub struct Output {
	pub image_sharing: bool,
	pub preserve_deleted_messages: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub appearance: Option<Theme>,
	pub replacement: Option<String>,
	pub panel: Vec<Element>,
	pub storage: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Element {
	Text {
		text: String,
	},
	Heading {
		text: String,
	},
	Separator,
	Row {
		children: Vec<Element>,
	},
	Button {
		id: String,
		label: String,
	},
	TextInput {
		id: String,
		label: String,
		value: String,
	},
	Checkbox {
		id: String,
		label: String,
		checked: bool,
	},
	Select {
		id: String,
		label: String,
		options: Vec<String>,
		value: String,
	},
	Slider {
		id: String,
		label: String,
		min: i32,
		max: i32,
		value: i32,
	},
}

/// Export a plain `fn(Invocation) -> Output` as Serein ABI version 1.
/// Each invocation gets a fresh instance, so buffers are reclaimed when it finishes.
#[macro_export]
macro_rules! export {
	($handler:path) => {
		#[unsafe(no_mangle)]
		pub extern "C" fn serein_alloc(length: u32) -> u32 {
			if length > 256 * 1024 {
				return 0;
			}
			let buffer = vec![0_u8; length as usize].into_boxed_slice();
			Box::into_raw(buffer) as *mut u8 as u32
		}

		/// # Safety
		/// The Serein host supplies the pointer returned by `serein_alloc` and its allocated length.
		#[unsafe(no_mangle)]
		pub unsafe extern "C" fn serein_invoke(pointer: u32, length: u32) -> u64 {
			if length > 256 * 1024 {
				return 0;
			}
			// SAFETY: the host ABI owns this allocation and writes exactly `length` bytes.
			let input =
				unsafe { std::slice::from_raw_parts(pointer as *const u8, length as usize) };
			let Ok(input) = $crate::serde_json::from_slice::<$crate::Invocation>(input) else {
				return 0;
			};
			let Ok(output) = $crate::serde_json::to_vec(&$handler(input)) else {
				return 0;
			};
			if output.len() > 256 * 1024 {
				return 0;
			}
			let length = output.len() as u64;
			let pointer = Box::into_raw(output.into_boxed_slice()) as *mut u8 as u32;
			((pointer as u64) << 32) | length
		}
	};
}

// Declarative appearance values; the host validates token names and numeric bounds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemePalette {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub background: Option<Background>,
	#[serde(default)]
	pub colors: BTreeMap<String, String>,
	#[serde(default)]
	pub backdrop: Option<[String; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Background {
	pub opacity: u8,
	pub fit: BackgroundFit,
	pub target: BackgroundTarget,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sections: Option<SectionOpacity>,
}
impl Default for Background {
	fn default() -> Self {
		Self {
			opacity: 25,
			fit: BackgroundFit::Cover,
			target: BackgroundTarget::Window,
			sections: None,
		}
	}
}
/// Surface coverage over one continuous window image; text and controls stay opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SectionOpacity {
 pub top_bar: u8,
 pub server_list: u8,
 pub channel_list: u8,
 pub message_list: u8,
 pub member_list: u8,
 pub composer: u8,
}
impl Default for SectionOpacity {
 fn default() -> Self {
  Self { top_bar: 85, server_list: 85, channel_list: 85, message_list: 75, member_list: 85, composer: 90 }
 }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundFit {
	#[default]
	Cover,
	Contain,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTarget {
	#[default]
	Window,
	Chat,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
	#[serde(default)]
	pub light: ThemePalette,
	#[serde(default)]
	pub dark: ThemePalette,
	#[serde(default)]
	pub style: ThemeStyle,
}

/// Native control metrics in logical pixels. Omitted fields inherit the active theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeStyle {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub transparency_blur: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub transparency: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub blur: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub transparent_all: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub body_size: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub heading_size: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub button_size: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub small_size: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub monospace_size: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub item_spacing: Option<[u8; 2]>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub button_padding: Option<[u8; 2]>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub control_height: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub widget_radius: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub window_radius: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub menu_radius: Option<u8>,
}
