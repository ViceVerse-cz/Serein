//! Device-local keyboard bindings shared by the settings UI and native host.

const MAX_KEY_NAME_BYTES: usize = 24;

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

	pub fn is_valid(&self) -> bool {
		!self.key.is_empty()
			&& self.key.len() <= MAX_KEY_NAME_BYTES
			&& self.key.bytes().all(|byte| byte.is_ascii_alphanumeric())
			&& self.modifiers & !(PRIMARY | SHIFT | ALT | CTRL) == 0
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
	ToggleMute,
	ToggleDeafen,
	CopyIssueDiagnostics,
}

impl KeybindAction {
	pub const ALL: [Self; 18] = [
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
			Self::ToggleMute => "Toggle Mute",
			Self::ToggleDeafen => "Toggle Deafen",
			Self::CopyIssueDiagnostics => "Copy Issue Diagnostics",
		}
	}

	pub const fn is_global(self) -> bool {
		matches!(
			self,
			Self::PushToTalk | Self::ToggleMute | Self::ToggleDeafen
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
	pub toggle_mute: KeyChord,
	pub toggle_deafen: KeyChord,
	/// Unassigned by default; the empty key is valid only for this optional action.
	pub copy_issue_diagnostics: KeyChord,
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
			toggle_mute: KeyChord::new("M", PRIMARY | SHIFT),
			toggle_deafen: KeyChord::new("D", PRIMARY | SHIFT),
			copy_issue_diagnostics: KeyChord::new("", 0),
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
			KeybindAction::ToggleMute => &mut self.toggle_mute,
			KeybindAction::ToggleDeafen => &mut self.toggle_deafen,
			KeybindAction::CopyIssueDiagnostics => &mut self.copy_issue_diagnostics,
		}
	}

	pub fn is_valid(&self) -> bool {
		KeybindAction::ALL.into_iter().all(|action| {
			let chord = self.chord(action);
			chord.is_valid()
				|| (action == KeybindAction::CopyIssueDiagnostics
					&& chord.key.is_empty()
					&& chord.modifiers == 0)
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
		bindings.copy_issue_diagnostics.modifiers = CTRL;
		assert!(!bindings.is_valid());
		bindings.copy_issue_diagnostics = KeyChord::new("D", CTRL | SHIFT);
		assert!(bindings.is_valid());
		bindings.send_message = KeyChord::new("", 0);
		assert!(!bindings.is_valid());
	}
}
