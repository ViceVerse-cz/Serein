//! Device-local keyboard bindings shared by the settings UI and native host.

const MAX_KEY_NAME_BYTES: usize = 24;

/// Keys only the native input poller can report, by name. The index is the bit in its down mask.
pub const POLLED_KEYS: [&str; 20] = [
	"ControlLeft",
	"ControlRight",
	"ShiftLeft",
	"ShiftRight",
	"AltLeft",
	"AltRight",
	"PadSouth",
	"PadEast",
	"PadWest",
	"PadNorth",
	"PadUp",
	"PadDown",
	"PadLeft",
	"PadRight",
	"PadL1",
	"PadR1",
	"PadL2",
	"PadR2",
	"PadL3",
	"PadR3",
];

/// The down-mask bit for a polled key, or `None` for any other key name.
pub fn polled_key_bit(name: &str) -> Option<u32> {
	POLLED_KEYS
		.iter()
		.position(|key| *key == name)
		.map(|index| 1 << index)
}

/// Whether `name` is a controller button from [`POLLED_KEYS`].
pub fn is_pad_button(name: &str) -> bool {
	name.starts_with("Pad") && polled_key_bit(name).is_some()
}

/// Polled keys are lone voice keys, so they stay off every non-global action.
fn accepts_polled_key(action: KeybindAction, chord: &KeyChord) -> bool {
	action.is_global() || polled_key_bit(&chord.key).is_none()
}

pub const PRIMARY: u8 = 1;
pub const SHIFT: u8 = 2;
pub const ALT: u8 = 4;
pub const CTRL: u8 = 8;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct KeyChord {
	/// An egui logical key name, for example `K`, `Slash`, or `ArrowUp`.
	pub key: String,
	/// A bounded bitset of [`PRIMARY`], [`SHIFT`], [`ALT`], and [`CTRL`].
	pub modifiers: u8,
}

impl Default for KeyChord {
	fn default() -> Self {
		Self {
			key: "V".into(),
			modifiers: 0,
		}
	}
}

impl KeyChord {
	pub fn new(key: &str, modifiers: u8) -> Self {
		Self {
			key: key.into(),
			modifiers,
		}
	}

	/// Whether the chord can be stored: a bounded ASCII key name, only known modifier
	/// bits, and no unmodified key the native poller already owns.
	pub fn is_valid(&self) -> bool {
		!self.key.is_empty()
			&& self.key.len() <= MAX_KEY_NAME_BYTES
			&& self.key.bytes().all(|byte| byte.is_ascii_alphanumeric())
			&& self.modifiers & !(PRIMARY | SHIFT | ALT | CTRL) == 0
			&& (self.modifiers == 0 || polled_key_bit(&self.key).is_none())
	}
}

/// Mouse buttons a binding may use. Left and right click stay reserved for ordinary pointing.
pub fn is_mouse_button(name: &str) -> bool {
	matches!(name, "MouseMiddle" | "MouseExtra1" | "MouseExtra2")
}

/// Upper bound on user-defined extra bindings, keeping saved preferences small.
pub const MAX_CUSTOM_BINDINGS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeybindAction {
	ShowShortcuts,
	SwitchConversation,
	SearchConversation,
	CloseOverlay,
	SendMessage,
	InsertNewLine,
	EditLastMessage,
	Bold,
	Italic,
	Underline,
	Strikethrough,
	InlineCode,
	CodeBlock,
	Spoiler,
	PushToTalk,
	PushToMute,
	ToggleMute,
	ToggleDeafen,
	CopyIssueDiagnostics,
}

impl KeybindAction {
	pub const ALL: [Self; 19] = [
		Self::ShowShortcuts,
		Self::SwitchConversation,
		Self::SearchConversation,
		Self::CloseOverlay,
		Self::SendMessage,
		Self::InsertNewLine,
		Self::EditLastMessage,
		Self::Bold,
		Self::Italic,
		Self::Underline,
		Self::Strikethrough,
		Self::InlineCode,
		Self::CodeBlock,
		Self::Spoiler,
		Self::PushToTalk,
		Self::PushToMute,
		Self::ToggleMute,
		Self::ToggleDeafen,
		Self::CopyIssueDiagnostics,
	];

	pub const fn label(self) -> &'static str {
		match self {
			Self::ShowShortcuts => "Show Keyboard Shortcuts List",
			Self::SwitchConversation => "Switch Conversation",
			Self::SearchConversation => "Search Current Conversation",
			Self::CloseOverlay => "Close Settings or Dialog",
			Self::SendMessage => "Send Message",
			Self::InsertNewLine => "Insert New Line",
			Self::EditLastMessage => "Edit Last Editable Message",
			Self::Bold => "Bold",
			Self::Italic => "Italic",
			Self::Underline => "Underline",
			Self::Strikethrough => "Strikethrough",
			Self::InlineCode => "Inline Code",
			Self::CodeBlock => "Code Block",
			Self::Spoiler => "Spoiler",
			Self::PushToTalk => "Push to Talk",
			Self::PushToMute => "Push to Mute",
			Self::ToggleMute => "Toggle Mute",
			Self::ToggleDeafen => "Toggle Deafen",
			Self::CopyIssueDiagnostics => "Copy Issue Diagnostics",
		}
	}

	/// Actions that may stay unassigned, stored as an empty key.
	pub const fn is_optional(self) -> bool {
		matches!(self, Self::PushToMute | Self::CopyIssueDiagnostics)
	}

	pub const fn is_global(self) -> bool {
		matches!(
			self,
			Self::PushToTalk | Self::PushToMute | Self::ToggleMute | Self::ToggleDeafen
		)
	}
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Keybinds {
	pub global_enabled: bool,
	pub show_shortcuts: KeyChord,
	pub switch_conversation: KeyChord,
	pub search_conversation: KeyChord,
	pub close_overlay: KeyChord,
	pub send_message: KeyChord,
	pub insert_new_line: KeyChord,
	pub edit_last_message: KeyChord,
	pub bold: KeyChord,
	pub italic: KeyChord,
	pub underline: KeyChord,
	pub strikethrough: KeyChord,
	pub inline_code: KeyChord,
	pub code_block: KeyChord,
	pub spoiler: KeyChord,
	pub push_to_talk: KeyChord,
	/// Unassigned by default, like [`Self::copy_issue_diagnostics`].
	pub push_to_mute: KeyChord,
	pub toggle_mute: KeyChord,
	pub toggle_deafen: KeyChord,
	/// Unassigned by default; the empty key is valid only for optional actions.
	pub copy_issue_diagnostics: KeyChord,
	/// Extra user-defined bindings for any action; unassigned rows are not saved.
	#[serde(
		default,
		deserialize_with = "deserialize_custom",
		serialize_with = "serialize_custom"
	)]
	pub custom: Vec<CustomBinding>,
}

/// One extra binding on top of the fixed row for [`KeybindAction`].
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CustomBinding {
	pub action: KeybindAction,
	/// The empty key marks a row the user has not recorded yet.
	pub chord: KeyChord,
}

/// Unassigned rows only exist while the user is picking a key, so they never reach the file.
fn serialize_custom<S>(custom: &[CustomBinding], serializer: S) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	serializer.collect_seq(
		custom
			.iter()
			.filter(|binding| !binding.chord.key.is_empty()),
	)
}

/// Drops a row naming an action this build does not have instead of failing every preference.
fn deserialize_custom<'de, D>(deserializer: D) -> Result<Vec<CustomBinding>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	#[derive(serde::Deserialize)]
	#[serde(untagged)]
	enum Entry {
		Known(CustomBinding),
		Dropped(serde::de::IgnoredAny),
	}
	let entries = <Vec<Entry> as serde::Deserialize>::deserialize(deserializer)?;
	Ok(entries
		.into_iter()
		.filter_map(|entry| match entry {
			Entry::Known(binding) => Some(binding),
			Entry::Dropped(_) => None,
		})
		.collect())
}

impl Default for Keybinds {
	fn default() -> Self {
		Self {
			global_enabled: true,
			show_shortcuts: KeyChord::new("Slash", PRIMARY),
			switch_conversation: KeyChord::new("K", PRIMARY),
			search_conversation: KeyChord::new("F", PRIMARY),
			close_overlay: KeyChord::new("Escape", 0),
			send_message: KeyChord::new("Enter", 0),
			insert_new_line: KeyChord::new("Enter", SHIFT),
			edit_last_message: KeyChord::new("ArrowUp", 0),
			bold: KeyChord::new("B", PRIMARY),
			italic: KeyChord::new("I", PRIMARY),
			underline: KeyChord::new("U", PRIMARY),
			strikethrough: KeyChord::new("X", PRIMARY | SHIFT),
			inline_code: KeyChord::new("E", PRIMARY),
			code_block: KeyChord::new("C", PRIMARY | SHIFT),
			spoiler: KeyChord::new("P", PRIMARY | SHIFT),
			push_to_talk: KeyChord::new("V", 0),
			push_to_mute: KeyChord::new("", 0),
			toggle_mute: KeyChord::new("M", PRIMARY | SHIFT),
			toggle_deafen: KeyChord::new("D", PRIMARY | SHIFT),
			copy_issue_diagnostics: KeyChord::new("", 0),
			custom: Vec::new(),
		}
	}
}

impl Keybinds {
	pub fn chord(&self, action: KeybindAction) -> &KeyChord {
		match action {
			KeybindAction::ShowShortcuts => &self.show_shortcuts,
			KeybindAction::SwitchConversation => &self.switch_conversation,
			KeybindAction::SearchConversation => &self.search_conversation,
			KeybindAction::CloseOverlay => &self.close_overlay,
			KeybindAction::SendMessage => &self.send_message,
			KeybindAction::InsertNewLine => &self.insert_new_line,
			KeybindAction::EditLastMessage => &self.edit_last_message,
			KeybindAction::Bold => &self.bold,
			KeybindAction::Italic => &self.italic,
			KeybindAction::Underline => &self.underline,
			KeybindAction::Strikethrough => &self.strikethrough,
			KeybindAction::InlineCode => &self.inline_code,
			KeybindAction::CodeBlock => &self.code_block,
			KeybindAction::Spoiler => &self.spoiler,
			KeybindAction::PushToTalk => &self.push_to_talk,
			KeybindAction::PushToMute => &self.push_to_mute,
			KeybindAction::ToggleMute => &self.toggle_mute,
			KeybindAction::ToggleDeafen => &self.toggle_deafen,
			KeybindAction::CopyIssueDiagnostics => &self.copy_issue_diagnostics,
		}
	}

	pub fn chord_mut(&mut self, action: KeybindAction) -> &mut KeyChord {
		match action {
			KeybindAction::ShowShortcuts => &mut self.show_shortcuts,
			KeybindAction::SwitchConversation => &mut self.switch_conversation,
			KeybindAction::SearchConversation => &mut self.search_conversation,
			KeybindAction::CloseOverlay => &mut self.close_overlay,
			KeybindAction::SendMessage => &mut self.send_message,
			KeybindAction::InsertNewLine => &mut self.insert_new_line,
			KeybindAction::EditLastMessage => &mut self.edit_last_message,
			KeybindAction::Bold => &mut self.bold,
			KeybindAction::Italic => &mut self.italic,
			KeybindAction::Underline => &mut self.underline,
			KeybindAction::Strikethrough => &mut self.strikethrough,
			KeybindAction::InlineCode => &mut self.inline_code,
			KeybindAction::CodeBlock => &mut self.code_block,
			KeybindAction::Spoiler => &mut self.spoiler,
			KeybindAction::PushToTalk => &mut self.push_to_talk,
			KeybindAction::PushToMute => &mut self.push_to_mute,
			KeybindAction::ToggleMute => &mut self.toggle_mute,
			KeybindAction::ToggleDeafen => &mut self.toggle_deafen,
			KeybindAction::CopyIssueDiagnostics => &mut self.copy_issue_diagnostics,
		}
	}

	/// The built-in chord when assigned, then every custom chord for `action`.
	pub fn chords(&self, action: KeybindAction) -> impl Iterator<Item = &KeyChord> {
		std::iter::once(self.chord(action))
			.filter(|chord| chord.is_valid())
			.chain(
				self.custom
					.iter()
					.filter(move |binding| binding.action == action)
					.map(|binding| &binding.chord)
					.filter(|chord| chord.is_valid()),
			)
	}

	/// First action already bound to `chord`, ignoring one built-in action and one custom row.
	pub fn conflict(
		&self,
		chord: &KeyChord,
		ignore_builtin: Option<KeybindAction>,
		ignore_custom: Option<usize>,
	) -> Option<KeybindAction> {
		KeybindAction::ALL
			.into_iter()
			.filter(|action| Some(*action) != ignore_builtin)
			.find(|action| self.chord(*action) == chord)
			.or_else(|| {
				self.custom
					.iter()
					.enumerate()
					.find(|(index, binding)| {
						Some(*index) != ignore_custom && binding.chord == *chord
					})
					.map(|(_, binding)| binding.action)
			})
	}

	/// Whether every built-in and custom binding is storable: each is a valid chord the
	/// action accepts, or an empty chord for an optional action; custom rows also stay
	/// within the cap.
	pub fn is_valid(&self) -> bool {
		KeybindAction::ALL.into_iter().all(|action| {
			let chord = self.chord(action);
			(chord.is_valid() && accepts_polled_key(action, chord))
				|| (action.is_optional() && chord.key.is_empty() && chord.modifiers == 0)
		}) && self.custom.len() <= MAX_CUSTOM_BINDINGS
			&& self.custom.iter().all(|binding| {
				// A row the user added but has not recorded yet is not an invalid preference.
				(binding.chord.is_valid() && accepts_polled_key(binding.action, &binding.chord))
					|| (binding.chord.key.is_empty() && binding.chord.modifiers == 0)
			})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn diagnostics_is_optional_without_relaxing_other_binding_validation() {
		let mut bindings = Keybinds::default();
		assert!(bindings.is_valid());
		assert!(bindings.copy_issue_diagnostics.key.is_empty());
		assert!(!KeybindAction::CopyIssueDiagnostics.is_global());
		assert!(bindings.push_to_mute.key.is_empty());
		bindings.push_to_mute = KeyChord::new("MouseExtra1", 0);
		assert!(bindings.is_valid());
		bindings.copy_issue_diagnostics.modifiers = CTRL;
		assert!(!bindings.is_valid());
		bindings.copy_issue_diagnostics = KeyChord::new("D", CTRL | SHIFT);
		assert!(bindings.is_valid());
		bindings.send_message = KeyChord::new("", 0);
		assert!(!bindings.is_valid());
	}

	#[test]
	fn polled_keys_are_lone_voice_keys() {
		assert_eq!(POLLED_KEYS.len(), 20);
		assert_eq!(polled_key_bit("ControlLeft"), Some(1));
		assert_eq!(polled_key_bit("PadR3"), Some(1 << 19));
		assert_eq!(polled_key_bit("V"), None);
		assert!(is_pad_button("PadSouth"));
		assert!(!is_pad_button("ControlRight"));
		assert!(!is_pad_button("PadUnknown"));

		let mut bindings = Keybinds {
			push_to_talk: KeyChord::new("ControlRight", 0),
			..Default::default()
		};
		assert!(bindings.is_valid());
		bindings.push_to_talk = KeyChord::new("ControlRight", SHIFT);
		assert!(!bindings.is_valid());
		bindings.push_to_talk = KeyChord::new("V", 0);
		bindings.bold = KeyChord::new("PadSouth", 0);
		assert!(!bindings.is_valid());
		bindings.bold = KeyChord::new("B", PRIMARY);
		bindings.custom = vec![CustomBinding {
			action: KeybindAction::Bold,
			chord: KeyChord::new("PadSouth", 0),
		}];
		assert!(!bindings.is_valid());
		bindings.custom[0].action = KeybindAction::PushToTalk;
		assert!(bindings.is_valid());
	}

	#[test]
	fn custom_bindings_extend_actions_and_skip_unrecorded_rows() {
		let mut bindings = Keybinds {
			custom: vec![
				CustomBinding {
					action: KeybindAction::PushToTalk,
					chord: KeyChord::new("Y", 0),
				},
				CustomBinding {
					action: KeybindAction::PushToTalk,
					chord: KeyChord::new("", 0),
				},
				CustomBinding {
					action: KeybindAction::Bold,
					chord: KeyChord::new("G", CTRL),
				},
			],
			..Default::default()
		};
		assert!(bindings.is_valid());
		assert_eq!(
			bindings
				.chords(KeybindAction::PushToTalk)
				.cloned()
				.collect::<Vec<_>>(),
			vec![KeyChord::new("V", 0), KeyChord::new("Y", 0)]
		);
		assert_eq!(
			bindings.conflict(&KeyChord::new("Y", 0), None, None),
			Some(KeybindAction::PushToTalk)
		);
		assert_eq!(
			bindings.conflict(&KeyChord::new("Y", 0), None, Some(0)),
			None
		);
		assert_eq!(
			bindings.conflict(
				&KeyChord::new("V", 0),
				Some(KeybindAction::PushToTalk),
				None
			),
			None
		);
		bindings.custom = (0..=MAX_CUSTOM_BINDINGS)
			.map(|index| CustomBinding {
				action: KeybindAction::Bold,
				chord: KeyChord::new(&format!("G{index}"), 0),
			})
			.collect();
		assert!(!bindings.is_valid());
	}
}
