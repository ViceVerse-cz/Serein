//! Remappable application shortcuts and the device-local global push-to-talk binding.
use crate::design;
use egui::{Event, InputState, Key, Modifiers, RichText};
use model::{KeyChord, KeybindAction, Keybinds};

const NAVIGATION: &[KeybindAction] = &[
	KeybindAction::ShowShortcuts,
	KeybindAction::SwitchConversation,
	KeybindAction::CloseOverlay,
];
const MESSAGES: &[KeybindAction] = &[
	KeybindAction::SendMessage,
	KeybindAction::InsertNewLine,
	KeybindAction::EditLastMessage,
];
const FORMATTING: &[KeybindAction] = &[
	KeybindAction::Bold,
	KeybindAction::Italic,
	KeybindAction::Underline,
	KeybindAction::Strikethrough,
	KeybindAction::InlineCode,
	KeybindAction::CodeBlock,
	KeybindAction::Spoiler,
];

pub(super) fn show(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<KeybindAction>,
	global_status: &str,
) {
	let colors = design::palette(ui);
	ui.heading("Custom Keybinds");
	ui.label(
		"Make Serein feel like yours. Click any shortcut, then press the key combination you want.",
	);
	design::card(ui, |ui| {
		ui.horizontal(|ui| {
			ui.label(RichText::new("⌨").size(20.0).color(colors.accent));
			ui.vertical(|ui| {
				ui.label(design::semibold(
					ui,
					"Shortcuts are saved on this device",
					14.0,
				));
				ui.weak(
					"Application shortcuts work while Serein is focused. Push to Talk can also work globally.",
				);
			});
		});
	});

	ui.add_space(18.0);
	section(
		ui,
		"Navigation",
		"Move around Serein without reaching for the mouse.",
		NAVIGATION,
		bindings,
		capturing,
	);
	section(
		ui,
		"Messages",
		"Composer shortcuts are only active while you are writing.",
		MESSAGES,
		bindings,
		capturing,
	);
	section(
		ui,
		"Text Formatting",
		"Apply or remove formatting in the composer.",
		FORMATTING,
		bindings,
		capturing,
	);
	section(
		ui,
		"Voice",
		"Control your microphone and incoming audio during a connected call.",
		&[
			KeybindAction::ToggleMute,
			KeybindAction::ToggleDeafen,
			KeybindAction::PushToTalk,
		],
		bindings,
		capturing,
	);
	ui.add_space(10.0);
	ui.label(
		RichText::new("GLOBAL AVAILABILITY")
			.size(11.0)
			.color(colors.muted),
	);
	ui.label(RichText::new(global_status).color(colors.muted));

	if let Some(action) = *capturing {
		let mut captured = None;
		let mut cancelled = false;
		for event in ui.input(|input| input.events.clone()) {
			if let Event::Key {
				key,
				pressed: true,
				repeat: false,
				modifiers,
				..
			} = event
			{
				if key == Key::Escape {
					cancelled = true;
				} else if let Some(name) = key_name(key) {
					captured = Some(KeyChord::new(name, modifier_bits(modifiers)));
				}
				break;
			}
		}
		if cancelled {
			*capturing = None;
		} else if let Some(chord) = captured {
			if let Some(other) = KeybindAction::ALL
				.into_iter()
				.find(|other| *other != action && bindings.chord(*other) == &chord)
			{
				ui.colored_label(
					colors.danger,
					format!("That shortcut is already used by {}.", other.label()),
				);
			} else {
				*bindings.chord_mut(action) = chord;
				*capturing = None;
			}
		}
	}
}

fn section(
	ui: &mut egui::Ui,
	title: &str,
	description: &str,
	actions: &[KeybindAction],
	bindings: &mut Keybinds,
	capturing: &mut Option<KeybindAction>,
) {
	ui.add_space(14.0);
	ui.label(design::semibold(ui, title, 18.0));
	ui.weak(description);
	design::card(ui, |ui| {
		for (index, action) in actions.iter().copied().enumerate() {
			if index > 0 {
				ui.separator();
			}
			row(ui, action, bindings, capturing);
		}
	});
}

fn row(
	ui: &mut egui::Ui,
	action: KeybindAction,
	bindings: &mut Keybinds,
	capturing: &mut Option<KeybindAction>,
) {
	let colors = design::palette(ui);
	let active = *capturing == Some(action);
	ui.horizontal(|ui| {
		ui.set_min_height(46.0);
		ui.allocate_ui_with_layout(
			egui::vec2((ui.available_width() - 190.0).max(0.0), 46.0),
			egui::Layout::left_to_right(egui::Align::Center),
			|ui| {
				ui.label(action.label());
				if action.is_global() {
					ui.label(RichText::new("GLOBAL").size(10.0).color(colors.accent));
				}
			},
		);
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			if ui.small_button("Reset").clicked() {
				*bindings.chord_mut(action) = Keybinds::default().chord(action).clone();
				if *capturing == Some(action) {
					*capturing = None;
				}
			}
			if shortcut_button(ui, bindings.chord(action), active).clicked() {
				*capturing = Some(action);
			}
		});
	});
}

fn modifier_bits(modifiers: Modifiers) -> u8 {
	let mut bits = 0;
	if modifiers.command {
		bits |= model::keybinds::PRIMARY;
	}
	if modifiers.shift {
		bits |= model::keybinds::SHIFT;
	}
	if modifiers.alt {
		bits |= model::keybinds::ALT;
	}
	if modifiers.ctrl && !modifiers.command {
		bits |= model::keybinds::CTRL;
	}
	bits
}

fn egui_modifiers(bits: u8) -> Modifiers {
	Modifiers {
		alt: bits & model::keybinds::ALT != 0,
		ctrl: bits & model::keybinds::CTRL != 0,
		shift: bits & model::keybinds::SHIFT != 0,
		mac_cmd: false,
		command: bits & model::keybinds::PRIMARY != 0,
	}
}

pub(crate) fn pressed(input: &mut InputState, chord: &KeyChord) -> bool {
	key_name_to_egui(&chord.key)
		.is_some_and(|key| input.consume_key(egui_modifiers(chord.modifiers), key))
}

pub(crate) fn pressed_exact(input: &mut InputState, chord: &KeyChord) -> bool {
	let Some(key) = key_name_to_egui(&chord.key) else {
		return false;
	};
	let matched = input.events.iter().any(|event| {
		matches!(event, Event::Key { key: event_key, pressed: true, repeat: false, modifiers, .. }
			if *event_key == key && modifier_bits(*modifiers) == chord.modifiers)
	});
	matched && input.consume_key(egui_modifiers(chord.modifiers), key)
}

pub(crate) fn down(input: &InputState, chord: &KeyChord) -> bool {
	key_name_to_egui(&chord.key).is_some_and(|key| {
		input.key_down(key)
			&& input
				.modifiers
				.matches_logically(egui_modifiers(chord.modifiers))
	})
}

fn chord_parts(chord: &KeyChord) -> Vec<String> {
	let mut parts: Vec<String> = Vec::new();
	if chord.modifiers & model::keybinds::PRIMARY != 0 {
		parts.push(
			if cfg!(target_os = "macos") {
				"⌘"
			} else {
				"Ctrl"
			}
			.into(),
		);
	}
	if chord.modifiers & model::keybinds::CTRL != 0 {
		parts.push("Ctrl".into());
	}
	if chord.modifiers & model::keybinds::ALT != 0 {
		parts.push(
			if cfg!(target_os = "macos") {
				"⌥"
			} else {
				"Alt"
			}
			.into(),
		);
	}
	if chord.modifiers & model::keybinds::SHIFT != 0 {
		parts.push("Shift".into());
	}
	parts.push(display_key(&chord.key));
	parts
}

fn shortcut_button(ui: &mut egui::Ui, chord: &KeyChord, active: bool) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(152.0, 34.0), egui::Sense::click());
	let fill = if active {
		colors.accent.gamma_multiply(0.18)
	} else if response.hovered() {
		colors.hover
	} else {
		colors.raised
	};
	ui.painter().rect(
		rect,
		6,
		fill,
		egui::Stroke::new(1.0, if active { colors.accent } else { colors.border }),
		egui::StrokeKind::Inside,
	);
	if active {
		ui.painter().text(
			rect.center(),
			egui::Align2::CENTER_CENTER,
			"Press keys…",
			egui::FontId::new(12.0, design::medium_family(ui.ctx())),
			colors.text_strong,
		);
		return response;
	}

	let font = egui::FontId::new(11.0, design::medium_family(ui.ctx()));
	let labels = chord_parts(chord);
	let widths: Vec<f32> = labels
		.iter()
		.map(|label| {
			ui.painter()
				.layout_no_wrap(label.clone(), font.clone(), colors.text_strong)
				.size()
				.x + 14.0
		})
		.collect();
	let total = widths.iter().sum::<f32>() + (widths.len().saturating_sub(1) as f32 * 4.0);
	let mut x = rect.center().x - total * 0.5;
	for (label, width) in labels.iter().zip(widths) {
		let key_rect = egui::Rect::from_min_size(
			egui::pos2(x, rect.center().y - 12.0),
			egui::vec2(width, 24.0),
		);
		ui.painter().rect_filled(
			egui::Rect::from_min_max(
				key_rect.left_top(),
				key_rect.right_bottom() + egui::vec2(0.0, 2.0),
			),
			4,
			colors.border,
		);
		let face = key_rect.translate(egui::vec2(0.0, -1.0));
		ui.painter().rect(
			face,
			4,
			colors.chat,
			egui::Stroke::new(1.0, colors.border),
			egui::StrokeKind::Inside,
		);
		let galley = ui
			.painter()
			.layout_no_wrap(label.clone(), font.clone(), colors.text_strong);
		ui.painter().galley(
			face.center() - galley.size() * 0.5,
			galley,
			colors.text_strong,
		);
		x += width + 4.0;
	}
	response
}

fn display_key(name: &str) -> String {
	match name {
		"ArrowUp" => "↑".into(),
		"ArrowDown" => "↓".into(),
		"ArrowLeft" => "←".into(),
		"ArrowRight" => "→".into(),
		"Escape" => "Esc".into(),
		"Enter" => "↵".into(),
		"Slash" => "/".into(),
		name if name
			.strip_prefix("Num")
			.is_some_and(|digit| digit.len() == 1) =>
		{
			name[3..].into()
		}
		other => other.to_uppercase(),
	}
}

fn key_name(key: Key) -> Option<&'static str> {
	KEYS.iter()
		.find(|(candidate, _)| *candidate == key)
		.map(|(_, name)| *name)
}

fn key_name_to_egui(name: &str) -> Option<Key> {
	KEYS.iter()
		.find(|(_, candidate)| *candidate == name)
		.map(|(key, _)| *key)
}

const KEYS: &[(Key, &str)] = &[
	(Key::ArrowDown, "ArrowDown"),
	(Key::ArrowLeft, "ArrowLeft"),
	(Key::ArrowRight, "ArrowRight"),
	(Key::ArrowUp, "ArrowUp"),
	(Key::Escape, "Escape"),
	(Key::Tab, "Tab"),
	(Key::Backspace, "Backspace"),
	(Key::Enter, "Enter"),
	(Key::Space, "Space"),
	(Key::Delete, "Delete"),
	(Key::Home, "Home"),
	(Key::End, "End"),
	(Key::Slash, "Slash"),
	(Key::Backtick, "Backtick"),
	(Key::Minus, "Minus"),
	(Key::Equals, "Equals"),
	(Key::Comma, "Comma"),
	(Key::Period, "Period"),
	(Key::Num0, "Num0"),
	(Key::Num1, "Num1"),
	(Key::Num2, "Num2"),
	(Key::Num3, "Num3"),
	(Key::Num4, "Num4"),
	(Key::Num5, "Num5"),
	(Key::Num6, "Num6"),
	(Key::Num7, "Num7"),
	(Key::Num8, "Num8"),
	(Key::Num9, "Num9"),
	(Key::A, "A"),
	(Key::B, "B"),
	(Key::C, "C"),
	(Key::D, "D"),
	(Key::E, "E"),
	(Key::F, "F"),
	(Key::G, "G"),
	(Key::H, "H"),
	(Key::I, "I"),
	(Key::J, "J"),
	(Key::K, "K"),
	(Key::L, "L"),
	(Key::M, "M"),
	(Key::N, "N"),
	(Key::O, "O"),
	(Key::P, "P"),
	(Key::Q, "Q"),
	(Key::R, "R"),
	(Key::S, "S"),
	(Key::T, "T"),
	(Key::U, "U"),
	(Key::V, "V"),
	(Key::W, "W"),
	(Key::X, "X"),
	(Key::Y, "Y"),
	(Key::Z, "Z"),
	(Key::F1, "F1"),
	(Key::F2, "F2"),
	(Key::F3, "F3"),
	(Key::F4, "F4"),
	(Key::F5, "F5"),
	(Key::F6, "F6"),
	(Key::F7, "F7"),
	(Key::F8, "F8"),
	(Key::F9, "F9"),
	(Key::F10, "F10"),
	(Key::F11, "F11"),
	(Key::F12, "F12"),
];

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_bindings_round_trip_and_match() {
		let bindings = Keybinds::default();
		assert!(bindings.is_valid());
		assert_eq!(
			chord_parts(bindings.chord(KeybindAction::SwitchConversation)).join(" + "),
			if cfg!(target_os = "macos") {
				"⌘ + K"
			} else {
				"Ctrl + K"
			}
		);
		let ctx = egui::Context::default();
		let mut matched = false;
		let mut output = ctx.run_ui(
			egui::RawInput {
				events: vec![Event::Key {
					key: Key::V,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: Modifiers::NONE,
				}],
				..Default::default()
			},
			|ui| {
				matched =
					ui.input_mut(|input| pressed(input, bindings.chord(KeybindAction::PushToTalk)))
			},
		);
		output.textures_delta.clear();
		assert!(matched);
	}
}
