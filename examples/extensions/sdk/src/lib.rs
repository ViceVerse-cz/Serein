//! Minimal author-side ABI. This crate belongs inside the sandbox, not in the host.
//!
//! Test handlers locally through the same JSON path as the Wasm exports:
//! ```
//! use serein_extension_sdk::{dispatch, Invocation, Output, serde_json};
//! fn handle(input: Invocation) -> Output {
//!     Output { replacement: input.composer.map(|text| text.to_uppercase()), ..Default::default() }
//! }
//! let bytes = dispatch(br#"{"action":"upper","composer":"hello"}"#, handle)?;
//! let output: Output = serde_json::from_slice(&bytes)?;
//! assert_eq!(output.replacement.as_deref(), Some("HELLO"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
use serde::{Deserialize, Serialize};
pub use serde_json;
use std::{collections::BTreeMap, fmt, io, str::FromStr};
mod discovery;
pub use discovery::*;
mod conversation_activity;
pub use conversation_activity::*;
mod message_content;
pub use message_content::*;
mod forum_data;
pub use forum_data::*;
mod channel_metadata;
pub use channel_metadata::*;
mod member_details;
pub use member_details::*;
mod app;
pub use app::*;
mod extended;
pub use extended::*;
mod manifest;
pub use manifest::*;

/// Maximum serialized size of either ABI buffer, in UTF-8 bytes.
pub const MAX_IO_BYTES: usize = 256 * 1024;
/// Maximum message-event content size, in UTF-8 bytes.
pub const MAX_EVENT_CONTENT_BYTES: usize = 16 * 1024;
/// Maximum distinct capabilities requested by one manifest.
pub const MAX_CAPABILITIES: usize = 64;
/// Version of the unchanged Wasm buffer and JSON contract.
pub const API_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Opt-in message-event context, preserving the original `Invocation` struct literal API.
/// Use this input type with `export!(handler)` and `dispatch_typed`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventInvocation {
	#[serde(flatten)]
	pub invocation: Invocation,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub message_event: Option<MessageEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageEventKind {
	Create,
	Update,
	Delete,
}

/// Bounded, live message metadata. Delete events contain IDs only; updates may be partial.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageEvent {
	pub kind: MessageEventKind,
	pub channel_id: String,
	pub message_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub author_id: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub content: Option<String>,
}

impl Invocation {
	/// Read a panel value without allocating. Missing values stay distinct from empty strings.
	pub fn value(&self, id: &str) -> Option<&str> {
		self.values.get(id).map(String::as_str)
	}

	/// Parse a checkbox, slider or other typed value; malformed values are errors, not defaults.
	pub fn parse_value<T: FromStr>(&self, id: &str) -> Result<Option<T>, T::Err> {
		self.value(id).map(str::parse).transpose()
	}

	/// Decode JSON stored by the plugin. Absent storage is `None`; corrupt storage is an error.
	pub fn storage_json<T: serde::de::DeserializeOwned>(
		&self,
	) -> Result<Option<T>, serde_json::Error> {
		self.storage
			.as_deref()
			.map(serde_json::from_str)
			.transpose()
	}
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Output {
	#[serde(skip_serializing_if = "is_false")]
	pub image_sharing: bool,
	#[serde(skip_serializing_if = "is_false")]
	pub preserve_deleted_messages: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub appearance: Option<Theme>,
	pub replacement: Option<String>,
	pub panel: Vec<Element>,
	pub storage: Option<String>,
}

fn is_false(value: &bool) -> bool {
	!value
}

impl Output {
	/// Save JSON using the existing opaque storage field. On failure, keep the previous value.
	/// The complete response (including JSON escaping) must still fit `MAX_IO_BYTES`.
	pub fn set_storage_json<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
		let bytes = encode(value)?;
		// serde_json always emits valid UTF-8.
		self.storage = Some(String::from_utf8(bytes).expect("JSON is UTF-8"));
		Ok(())
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Local SDK errors. The Wasm ABI still reports failure with a zero-length response.
#[derive(Debug)]
pub enum Error {
	InputTooLarge,
	InvalidInput(serde_json::Error),
	OutputTooLarge,
	InvalidOutput(serde_json::Error),
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::InputTooLarge => "extension input exceeds 256 KiB",
			Self::InvalidInput(_) => "invalid extension input JSON",
			Self::OutputTooLarge => "extension output exceeds 256 KiB",
			Self::InvalidOutput(_) => "extension output could not be serialized",
		})
	}
}

impl std::error::Error for Error {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::InvalidInput(error) | Self::InvalidOutput(error) => Some(error),
			_ => None,
		}
	}
}

/// Run the same bounded JSON path used by `export!`, without raw pointers or a Wasm runtime.
/// This checks wire encoding and byte limits; the host still validates capabilities and panels.
pub fn dispatch(
	input: &[u8],
	handler: impl FnOnce(Invocation) -> Output,
) -> Result<Vec<u8>, Error> {
	dispatch_typed(input, handler)
}

/// Bounded JSON dispatch for additive context types such as `EventInvocation`.
/// The host validates event capabilities, payloads and allowed effects before/after execution.
pub fn dispatch_typed<I: serde::de::DeserializeOwned, O: Serialize>(
	input: &[u8],
	handler: impl FnOnce(I) -> O,
) -> Result<Vec<u8>, Error> {
	if input.len() > MAX_IO_BYTES {
		return Err(Error::InputTooLarge);
	}
	let input = serde_json::from_slice(input).map_err(Error::InvalidInput)?;
	encode(&handler(input))
}

fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, Error> {
	struct Buffer(Vec<u8>);
	impl io::Write for Buffer {
		fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
			if bytes.len() > MAX_IO_BYTES - self.0.len() {
				return Err(io::ErrorKind::WriteZero.into());
			}
			let required = self.0.len() + bytes.len();
			if required > self.0.capacity() {
				let capacity = required.max(self.0.capacity() * 2).min(MAX_IO_BYTES);
				self.0.reserve_exact(capacity - self.0.len());
			}
			self.0.extend_from_slice(bytes);
			Ok(bytes.len())
		}

		fn flush(&mut self) -> io::Result<()> {
			Ok(())
		}
	}
	let mut buffer = Buffer(Vec::new());
	serde_json::to_writer(&mut buffer, value).map_err(|error| {
		if error.io_error_kind() == Some(io::ErrorKind::WriteZero) {
			Error::OutputTooLarge
		} else {
			Error::InvalidOutput(error)
		}
	})?;
	Ok(buffer.0)
}

/// Export a typed handler as Serein ABI version 1, including `fn(Invocation) -> Output`.
/// Each invocation gets a fresh instance, so buffers are reclaimed when it finishes.
#[macro_export]
macro_rules! export {
	($handler:path) => {
		#[unsafe(no_mangle)]
		pub extern "C" fn serein_alloc(length: u32) -> u32 {
			if length == 0 || length as usize > $crate::MAX_IO_BYTES {
				return 0;
			}
			let buffer = ::std::vec![0_u8; length as usize].into_boxed_slice();
			::std::boxed::Box::into_raw(buffer) as *mut u8 as u32
		}

		/// # Safety
		/// The Serein host supplies the pointer returned by `serein_alloc` and its allocated length.
		#[unsafe(no_mangle)]
		pub unsafe extern "C" fn serein_invoke(pointer: u32, length: u32) -> u64 {
			if pointer == 0 || length == 0 || length as usize > $crate::MAX_IO_BYTES {
				return 0;
			}
			// SAFETY: the host ABI owns this allocation and writes exactly `length` bytes.
			let input =
				unsafe { ::std::slice::from_raw_parts(pointer as *const u8, length as usize) };
			let Ok(output) = $crate::dispatch_typed(input, $handler) else {
				return 0;
			};
			let length = output.len() as u64;
			let pointer = ::std::boxed::Box::into_raw(output.into_boxed_slice()) as *mut u8 as u32;
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
		Self {
			top_bar: 85,
			server_list: 85,
			channel_list: 85,
			message_list: 75,
			member_list: 85,
			composer: 90,
		}
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
