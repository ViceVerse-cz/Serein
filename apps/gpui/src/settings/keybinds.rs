//! Keybinds: the shortcuts this frontend binds, grouped like the main app's page, with the
//! chords read from GPUI's keymap so the list cannot drift from the real bindings.
//! Rebinding and voice shortcuts stay in the main Serein app.
use super::kit;
use crate::theme::{color, palette};
use crate::{OpenSettings, Serein, ToggleSwitcher, input};
use gpui::{prelude::*, *};

/// Where a row's chords come from: a bound action, or a key handled directly by a view.
enum Source {
	Action(Box<dyn Action>),
	Key(&'static str),
}

struct Shortcut {
	label: &'static str,
	detail: Option<&'static str>,
	source: Source,
}

fn action(label: &'static str, action: impl Action) -> Shortcut {
	Shortcut {
		label,
		detail: None,
		source: Source::Action(Box::new(action)),
	}
}

fn groups() -> [(&'static str, &'static str, Vec<Shortcut>); 4] {
	[
		(
			"Navigation",
			"Move around Serein without reaching for the mouse.",
			vec![
				action("Switch Conversation", ToggleSwitcher),
				action("Open Settings", OpenSettings),
				Shortcut {
					label: "Close Settings or Dialog",
					detail: None,
					source: Source::Key("escape"),
				},
			],
		),
		(
			"Messages",
			"Composer shortcuts are only active while you are writing.",
			vec![
				action("Send Message", input::Submit),
				action("Insert New Line", input::Newline),
				Shortcut {
					label: "Edit Last Editable Message",
					detail: Some("While the composer is empty."),
					source: Source::Action(Box::new(input::Up)),
				},
				Shortcut {
					label: "Cancel Reply",
					detail: Some("Also closes suggestions and stops editing."),
					source: Source::Action(Box::new(input::Cancel)),
				},
			],
		),
		(
			"Text Formatting",
			"Apply or remove formatting in the composer.",
			vec![
				action("Bold", input::FormatBold),
				action("Italic", input::FormatItalic),
				action("Underline", input::FormatUnderline),
				action("Strikethrough", input::FormatStrike),
				action("Inline Code", input::FormatCode),
				action("Code Block", input::FormatCodeBlock),
				action("Spoiler", input::FormatSpoiler),
			],
		),
		(
			"Window",
			"Application shortcuts from the menu bar.",
			vec![
				action("Minimize", crate::Minimize),
				action("Hide Serein", crate::Hide),
				action("Hide Others", crate::HideOthers),
				action("Quit Serein", crate::Quit),
			],
		),
	]
}

impl Serein {
	pub(super) fn settings_keybinds(&mut self, cx: &mut Context<Self>) -> Div {
		let keymap = cx.key_bindings();
		let keymap = keymap.borrow();
		let sections = groups().map(|(title, description, shortcuts)| {
			let rows = shortcuts
				.iter()
				.enumerate()
				.flat_map(|(index, shortcut)| {
					let chords = match &shortcut.source {
						Source::Action(action) => {
							chords(keymap.bindings_for_action(action.as_ref()).filter_map(
								|binding| match binding.keystrokes() {
									[single] => Some(single.inner().clone()),
									_ => None,
								},
							))
						}
						Source::Key(key) => Keystroke::parse(key).into_iter().collect(),
					};
					let divider = (index > 0).then(|| kit::divider().into_any_element());
					divider.into_iter().chain(std::iter::once(
						shortcut_row(shortcut.label, shortcut.detail, &chords).into_any_element(),
					))
				})
				.collect::<Vec<_>>();
			kit::group(
				title,
				kit::card()
					.child(div().pb_1().child(kit::hint(description)))
					.children(rows),
			)
		});
		div()
			.flex()
			.flex_col()
			.gap_3()
			.children(sections)
			.child(kit::group(
				"Custom keybinds",
				kit::card()
					.child(kit::notice(
						kit::Level::Info,
						"These shortcuts are fixed in this preview. Custom keybinds are edited in the main Serein app and are not read here.",
					))
					.child(kit::hint(
						"Voice shortcuts (Toggle Mute, Toggle Deafen, Push to Talk) and the keyboard shortcuts list are only in the main app; this preview does not join calls.",
					)),
			))
	}
}

/// Unique chords in binding order.
fn chords(keystrokes: impl Iterator<Item = Keystroke>) -> Vec<Keystroke> {
	let mut unique = Vec::<Keystroke>::new();
	for keystroke in keystrokes {
		if !unique.contains(&keystroke) {
			unique.push(keystroke);
		}
	}
	unique
}

/// Action label on the left, keycaps on the right; alternative chords are joined by "or".
fn shortcut_row(label: &str, detail: Option<&str>, chords: &[Keystroke]) -> Div {
	let p = palette();
	let mut caps = Vec::new();
	for (index, chord) in chords.iter().enumerate() {
		if index > 0 {
			caps.push(
				div()
					.px_1()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child("or")
					.into_any_element(),
			);
		}
		caps.push(
			div()
				.flex()
				.gap_1()
				.children(chord_parts(chord).into_iter().map(keycap))
				.into_any_element(),
		);
	}
	kit::row(
		label,
		detail,
		div()
			.min_w(px(152.))
			.h(px(34.))
			.px_2()
			.rounded(px(6.))
			.bg(color(p.raised))
			.border_1()
			.border_color(color(p.border))
			.flex()
			.items_center()
			.justify_center()
			.gap_1()
			.children(caps),
	)
	.min_h(px(46.))
}

/// One key face with the darker edge underneath, like the main app's shortcut chips.
fn keycap(label: String) -> Div {
	let p = palette();
	div()
		.min_w(px(24.))
		.rounded(px(4.))
		.bg(color(p.border))
		.pb(px(2.))
		.child(
			div()
				.h(px(22.))
				.px(px(7.))
				.rounded(px(4.))
				.bg(color(p.chat))
				.border_1()
				.border_color(color(p.border))
				.flex()
				.items_center()
				.justify_center()
				.text_size(px(11.))
				.font_weight(FontWeight::MEDIUM)
				.text_color(color(p.text_strong))
				.child(label),
		)
}

/// Modifier and key labels in the platform's order: glyphs on macOS, words elsewhere.
fn chord_parts(keystroke: &Keystroke) -> Vec<String> {
	let mac = cfg!(target_os = "macos");
	let modifiers = keystroke.modifiers;
	let mut parts = Vec::new();
	for (on, glyph, word) in [
		(modifiers.control, "⌃", "Ctrl"),
		(modifiers.alt, "⌥", "Alt"),
		(modifiers.shift, "⇧", "Shift"),
		(
			modifiers.platform,
			"⌘",
			if cfg!(target_os = "windows") {
				"Win"
			} else {
				"Super"
			},
		),
	] {
		if on {
			parts.push(if mac { glyph } else { word }.to_owned());
		}
	}
	parts.push(key_label(&keystroke.key));
	parts
}

fn key_label(key: &str) -> String {
	match key {
		"enter" => "↵".into(),
		"escape" => "Esc".into(),
		"up" => "↑".into(),
		"down" => "↓".into(),
		"left" => "←".into(),
		"right" => "→".into(),
		"space" => "Space".into(),
		"backspace" => "⌫".into(),
		"tab" => "Tab".into(),
		other => other.to_uppercase(),
	}
}

#[cfg(test)]
mod tests {
	use super::{chord_parts, chords, key_label};
	use gpui::Keystroke;

	#[test]
	fn chords_show_platform_modifiers_and_readable_keys() {
		let bold = Keystroke::parse("secondary-shift-x").unwrap();
		let parts = chord_parts(&bold);
		if cfg!(target_os = "macos") {
			assert_eq!(parts, ["⇧", "⌘", "X"]);
		} else {
			assert_eq!(parts, ["Ctrl", "Shift", "X"]);
		}
		assert_eq!(key_label("enter"), "↵");
		assert_eq!(key_label("escape"), "Esc");
		assert_eq!(key_label(","), ",");
		let k = Keystroke::parse("cmd-k").unwrap();
		let unique =
			chords([k.clone(), k.clone(), Keystroke::parse("ctrl-k").unwrap()].into_iter());
		assert_eq!(unique.len(), 2);
		assert_eq!(unique[0], k);
	}
}
