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
const VOICE: &[KeybindAction] = &[
	KeybindAction::ToggleMute,
	KeybindAction::ToggleDeafen,
	KeybindAction::PushToTalk,
	KeybindAction::PushToMute,
];

pub(super) fn show(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<KeybindAction>,
	global_status: &str,
) {
	let colors = design::palette(ui);
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
	voice_section(ui, bindings, capturing);
	ui.add_space(10.0);
	ui.label(design::eyebrow(ui, "Global availability", colors.muted));
	design::hint(ui, global_status);

	capture(ui, bindings, capturing);
}

pub(super) fn show_voice(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<KeybindAction>,
	global_status: &str,
) {
	let colors = design::palette(ui);
	voice_section(ui, bindings, capturing);
	ui.add_space(10.0);
	ui.label(design::eyebrow(ui, "Global availability", colors.muted));
	design::hint(ui, global_status);
	capture(ui, bindings, capturing);
}

fn voice_section(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<KeybindAction>,
) {
	section(
		ui,
		"Voice",
		"Control your microphone and incoming audio during a connected call.",
		VOICE,
		bindings,
		capturing,
	);
}

#[derive(Clone, Copy)]
struct ConflictNotice {
	action: KeybindAction,
	conflicting_action: KeybindAction,
	time: f64,
}

impl Default for ConflictNotice {
	fn default() -> Self {
		Self {
			action: KeybindAction::ShowShortcuts,
			conflicting_action: KeybindAction::ShowShortcuts,
			time: 0.0,
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CaptureGuard {
	action: KeybindAction,
}

impl Default for CaptureGuard {
	fn default() -> Self {
		Self {
			action: KeybindAction::ShowShortcuts,
		}
	}
}

fn capture(ui: &mut egui::Ui, bindings: &mut Keybinds, capturing: &mut Option<KeybindAction>) {
	if let Some(action) = *capturing {
		let just_started = ui.data_mut(|data| {
			data.remove_temp::<CaptureGuard>(egui::Id::unique("keybind_capture_just_started"))
		});
		if just_started.map(|g| g.action) == Some(action) {
			return;
		}
		let mut captured = None;
		let mut cancelled = false;
		for event in ui.input(|input| input.events.clone()) {
			match event {
				Event::Key {
					key,
					pressed: true,
					repeat: false,
					modifiers,
					..
				} => {
					if key == Key::Escape {
						cancelled = true;
					} else if let Some(name) = key_name(key) {
						captured = Some(KeyChord::new(name, modifier_bits(modifiers)));
					}
					break;
				}
				Event::PointerButton {
					button,
					pressed: true,
					modifiers,
					..
				} => {
					let mod_bits = modifier_bits(modifiers);
					if button == egui::PointerButton::Primary && mod_bits == 0 {
						cancelled = true;
					} else {
						let name = pointer_button_name(button);
						captured = Some(KeyChord::new(name, mod_bits));
					}
					break;
				}
				_ => {}
			}
		}
		if cancelled {
			ui.input_mut(|input| {
				input
					.events
					.retain(|event| !matches!(event, Event::PointerButton { pressed: true, .. }));
			});
			*capturing = None;
		} else if let Some(chord) = captured {
			ui.input_mut(|input| {
				input
					.events
					.retain(|event| !matches!(event, Event::PointerButton { pressed: true, .. }));
			});
			if let Some(other) = KeybindAction::ALL
				.into_iter()
				.find(|other| *other != action && chord.key != "None" && bindings.chord(*other) == &chord)
			{
				let now = ui.input(|input| input.time);
				ui.data_mut(|data| {
					data.insert_temp(
						egui::Id::unique("keybind_conflict"),
						ConflictNotice {
							action,
							conflicting_action: other,
							time: now,
						},
					);
				});
				ui.ctx().request_repaint();
				*capturing = None;
			} else {
				*bindings.chord_mut(action) = chord;
				ui.data_mut(|data| {
					data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
				});
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
	design::group(ui, title, |ui| {
		design::hint(ui, description);
		ui.add_space(4.0);
		for (index, action) in actions.iter().copied().enumerate() {
			if index > 0 {
				design::card_divider(ui);
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
	let notice: Option<ConflictNotice> =
		ui.data(|data| data.get_temp(egui::Id::unique("keybind_conflict")));
	let now = ui.input(|input| input.time);
	let mut blink_factor = 0.0f32;
	let mut fade_alpha = 0.0f32;
	let mut conflict_text = None;

	if let Some(n) = notice.filter(|n| n.action == action) {
		let elapsed = (now - n.time) as f32;
		const TOTAL_DURATION: f32 = 2.5;
		const FADE_START: f32 = 1.0;
		if elapsed < TOTAL_DURATION {
			ui.ctx().request_repaint();
			if elapsed < 0.6 {
				let pulse = (elapsed * std::f32::consts::PI * 5.0).sin().abs();
				blink_factor = pulse * pulse;
			}
			if elapsed < FADE_START {
				fade_alpha = 1.0;
			} else {
				fade_alpha =
					(1.0 - (elapsed - FADE_START) / (TOTAL_DURATION - FADE_START)).clamp(0.0, 1.0);
			}
			conflict_text = Some(format!(
				"Already bound to {}.",
				n.conflicting_action.label()
			));
		}
	}

	ui.horizontal(|ui| {
		ui.set_min_height(46.0);
		ui.allocate_ui_with_layout(
			egui::vec2((ui.available_width() - 210.0).max(0.0), 46.0),
			egui::Layout::left_to_right(egui::Align::Center),
			|ui| {
				ui.label(action.label());
				if action.is_global() {
					ui.label(RichText::new("GLOBAL").size(10.0).color(colors.accent));
				}
				if let Some(ref msg) = conflict_text.filter(|_| fade_alpha > 0.0) {
					let text_color = colors.danger.gamma_multiply(fade_alpha);
					ui.add_space(6.0);
					ui.label(RichText::new(msg).size(11.0).color(text_color));
				}
			},
		);
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			if design::text_action(ui, "Reset").clicked() {
				*bindings.chord_mut(action) = Keybinds::default().chord(action).clone();
				if *capturing == Some(action) {
					*capturing = None;
				}
				ui.data_mut(|data| {
					data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
				});
			}
			if shortcut_button(ui, bindings.chord(action), active, blink_factor).clicked() {
				*capturing = Some(action);
				ui.data_mut(|data| {
					data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
					data.insert_temp(egui::Id::unique("keybind_capture_just_started"), CaptureGuard { action });
				});
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
	if chord.key == "None" {
		return false;
	}
	if let Some(btn) = pointer_button_from_name(&chord.key) {
		let matched = input.events.iter().any(|event| {
			matches!(event, Event::PointerButton { button: event_btn, pressed: true, modifiers, .. }
				if *event_btn == btn && modifiers.matches_logically(egui_modifiers(chord.modifiers)))
		});
		if matched {
			input.events.retain(|event| {
				!matches!(event, Event::PointerButton { button: event_btn, pressed: true, .. } if *event_btn == btn)
			});
		}
		matched
	} else if let Some(key) = key_name_to_egui(&chord.key) {
		input.consume_key(egui_modifiers(chord.modifiers), key)
	} else {
		false
	}
}

pub(crate) fn pressed_exact(input: &mut InputState, chord: &KeyChord) -> bool {
	if chord.key == "None" {
		return false;
	}
	if let Some(btn) = pointer_button_from_name(&chord.key) {
		let matched = input.events.iter().any(|event| {
			matches!(event, Event::PointerButton { button: event_btn, pressed: true, modifiers, .. }
				if *event_btn == btn && modifier_bits(*modifiers) == chord.modifiers)
		});
		if matched {
			input.events.retain(|event| {
				!matches!(event, Event::PointerButton { button: event_btn, pressed: true, .. } if *event_btn == btn)
			});
		}
		matched
	} else if let Some(key) = key_name_to_egui(&chord.key) {
		let matched = input.events.iter().any(|event| {
			matches!(event, Event::Key { key: event_key, pressed: true, repeat: false, modifiers, .. }
				if *event_key == key && modifier_bits(*modifiers) == chord.modifiers)
		});
		matched && input.consume_key(egui_modifiers(chord.modifiers), key)
	} else {
		false
	}
}

pub(crate) fn down(input: &InputState, chord: &KeyChord) -> bool {
	if chord.key == "None" {
		return false;
	}
	if let Some(btn) = pointer_button_from_name(&chord.key) {
		input.pointer.button_down(btn)
			&& input
				.modifiers
				.matches_logically(egui_modifiers(chord.modifiers))
	} else if let Some(key) = key_name_to_egui(&chord.key) {
		input.key_down(key)
			&& input
				.modifiers
				.matches_logically(egui_modifiers(chord.modifiers))
	} else {
		false
	}
}

fn chord_parts(chord: &KeyChord) -> Vec<String> {
	if chord.key == "None" {
		return vec!["None".into()];
	}
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

fn shortcut_button(
	ui: &mut egui::Ui,
	chord: &KeyChord,
	active: bool,
	blink: f32,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(152.0, 34.0), egui::Sense::click());
	let fill = if active {
		colors.accent.gamma_multiply(0.18)
	} else if blink > 0.0 {
		colors.danger.gamma_multiply(0.12 + 0.18 * blink)
	} else if response.hovered() {
		colors.hover
	} else {
		colors.raised
	};
	let border_color = if active {
		colors.accent
	} else if blink > 0.0 {
		colors.danger.gamma_multiply(0.4 + 0.6 * blink)
	} else {
		colors.border
	};
	let stroke_width = if active || blink > 0.0 { 1.5 } else { 1.0 };
	ui.painter().rect(
		rect,
		6,
		fill,
		egui::Stroke::new(stroke_width, border_color),
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
		"PageDown" => "PgDn".into(),
		"PageUp" => "PgUp".into(),
		"Insert" => "Ins".into(),
		"None" => "None".into(),
		"MouseMiddle" => "Mouse 3".into(),
		"MouseExtra1" => "Mouse 4".into(),
		"MouseExtra2" => "Mouse 5".into(),
		"MouseSecondary" => "Right Click".into(),
		"MousePrimary" => "Left Click".into(),
		name if name
			.strip_prefix("Num")
			.is_some_and(|digit| digit.len() == 1) =>
		{
			name[3..].into()
		}
		other => other.to_uppercase(),
	}
}

pub fn is_mouse_button(name: &str) -> bool {
	model::keybinds::is_mouse_button(name)
}

fn pointer_button_name(button: egui::PointerButton) -> &'static str {
	match button {
		egui::PointerButton::Primary => "MousePrimary",
		egui::PointerButton::Secondary => "MouseSecondary",
		egui::PointerButton::Middle => "MouseMiddle",
		egui::PointerButton::Extra1 => "MouseExtra1",
		egui::PointerButton::Extra2 => "MouseExtra2",
	}
}

fn pointer_button_from_name(name: &str) -> Option<egui::PointerButton> {
	match name {
		"MousePrimary" => Some(egui::PointerButton::Primary),
		"MouseSecondary" => Some(egui::PointerButton::Secondary),
		"MouseMiddle" => Some(egui::PointerButton::Middle),
		"MouseExtra1" => Some(egui::PointerButton::Extra1),
		"MouseExtra2" => Some(egui::PointerButton::Extra2),
		_ => None,
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
	(Key::Insert, "Insert"),
	(Key::Delete, "Delete"),
	(Key::Home, "Home"),
	(Key::End, "End"),
	(Key::PageUp, "PageUp"),
	(Key::PageDown, "PageDown"),
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
	fn default_push_to_talk_does_not_consume_typing() {
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
		let mut down_without_consuming = false;
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
				down_without_consuming = ui.input(|input| {
					down(input, bindings.chord(KeybindAction::PushToTalk))
						&& input.events.iter().any(|event| {
							matches!(
								event,
								Event::Key {
									key: Key::V,
									pressed: true,
									..
								}
							)
						})
				})
			},
		);
		output.textures_delta.clear();
		assert!(down_without_consuming);
	}

	#[test]
	fn page_down_and_navigation_keys_are_supported() {
		assert_eq!(key_name(Key::PageDown), Some("PageDown"));
		assert_eq!(key_name(Key::PageUp), Some("PageUp"));
		assert_eq!(key_name(Key::Insert), Some("Insert"));
		assert_eq!(display_key("PageDown"), "PgDn");
		assert_eq!(display_key("PageUp"), "PgUp");
		assert_eq!(display_key("Insert"), "Ins");
		assert_eq!(key_name_to_egui("PageDown"), Some(Key::PageDown));
	}

	#[test]
	fn conflicting_keybind_leaves_binding_unchanged() {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(KeybindAction::ToggleDeafen);
		let mute_chord = bindings.chord(KeybindAction::ToggleMute).clone();
		let deafen_before = bindings.chord(KeybindAction::ToggleDeafen).clone();

		let ctx = egui::Context::default();
		let mut output = ctx.run_ui(
			egui::RawInput {
				events: vec![Event::Key {
					key: Key::M,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui_modifiers(mute_chord.modifiers),
				}],
				..Default::default()
			},
			|ui| {
				capture(ui, &mut bindings, &mut capturing);
			},
		);
		output.textures_delta.clear();

		assert_eq!(capturing, None);
		assert_eq!(bindings.chord(KeybindAction::ToggleDeafen), &deafen_before);
	}

	#[test]
	fn mouse_buttons_display_and_mapping() {
		assert_eq!(display_key("None"), "None");
		assert_eq!(display_key("MouseMiddle"), "Mouse 3");
		assert_eq!(display_key("MouseExtra1"), "Mouse 4");
		assert_eq!(display_key("MouseExtra2"), "Mouse 5");
		assert_eq!(display_key("MouseSecondary"), "Right Click");
		assert_eq!(display_key("MousePrimary"), "Left Click");

		assert!(is_mouse_button("MouseExtra1"));
		assert!(is_mouse_button("MouseExtra2"));
		assert!(is_mouse_button("MouseMiddle"));
		assert!(!is_mouse_button("None"));
		assert!(!is_mouse_button("KeyA"));

		assert_eq!(
			pointer_button_from_name("MouseExtra1"),
			Some(egui::PointerButton::Extra1)
		);
		assert_eq!(
			pointer_button_from_name("MouseExtra2"),
			Some(egui::PointerButton::Extra2)
		);
		assert_eq!(
			pointer_button_from_name("MouseMiddle"),
			Some(egui::PointerButton::Middle)
		);
		assert_eq!(
			pointer_button_from_name("MouseSecondary"),
			Some(egui::PointerButton::Secondary)
		);
		assert_eq!(
			pointer_button_from_name("MousePrimary"),
			Some(egui::PointerButton::Primary)
		);
	}

	#[test]
	fn mouse_button_capture_and_down() {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(KeybindAction::PushToTalk);

		let ctx = egui::Context::default();
		let mut output = ctx.run_ui(
			egui::RawInput {
				events: vec![Event::PointerButton {
					pos: egui::pos2(100.0, 100.0),
					button: egui::PointerButton::Extra1,
					pressed: true,
					modifiers: Modifiers::NONE,
				}],
				..Default::default()
			},
			|ui| {
				capture(ui, &mut bindings, &mut capturing);
			},
		);
		output.textures_delta.clear();

		assert_eq!(capturing, None);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("MouseExtra1", 0)
		);

		// Test down
		let mut down_detected = false;
		let mut output2 = ctx.run_ui(
			egui::RawInput {
				events: vec![Event::PointerButton {
					pos: egui::pos2(100.0, 100.0),
					button: egui::PointerButton::Extra1,
					pressed: true,
					modifiers: Modifiers::NONE,
				}],
				..Default::default()
			},
			|ui| {
				down_detected = ui.input(|input| {
					down(input, bindings.chord(KeybindAction::PushToTalk))
				});
			},
		);
		output2.textures_delta.clear();
		assert!(down_detected);
	}

	#[test]
	fn push_to_mute_default_none_and_capture() {
		let mut bindings = Keybinds::default();
		assert_eq!(
			bindings.chord(KeybindAction::PushToMute),
			&KeyChord::new("None", 0)
		);
		assert_eq!(chord_parts(bindings.chord(KeybindAction::PushToMute)), vec!["None"]);

		let mut capturing = Some(KeybindAction::PushToMute);
		let ctx = egui::Context::default();
		let mut output = ctx.run_ui(
			egui::RawInput {
				events: vec![Event::PointerButton {
					pos: egui::pos2(50.0, 50.0),
					button: egui::PointerButton::Extra2,
					pressed: true,
					modifiers: Modifiers::NONE,
				}],
				..Default::default()
			},
			|ui| {
				capture(ui, &mut bindings, &mut capturing);
			},
		);
		output.textures_delta.clear();

		assert_eq!(capturing, None);
		assert_eq!(
			bindings.chord(KeybindAction::PushToMute),
			&KeyChord::new("MouseExtra2", 0)
		);
		assert_eq!(chord_parts(bindings.chord(KeybindAction::PushToMute)), vec!["Mouse 5"]);
	}
}
