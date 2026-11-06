//! Remappable application shortcuts and the device-local global push-to-talk binding.
use crate::design;
use egui::{Event, InputState, Key, Modifiers, RichText};
use model::keybinds::{POLLED_KEYS, is_pad_button, polled_key_bit};
use model::{CustomBinding, KeyChord, KeybindAction, Keybinds};

/// Which row is recording its next key or mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureTarget {
	BuiltIn(KeybindAction),
	Custom(usize),
}

const NAVIGATION: &[KeybindAction] = &[
	KeybindAction::ShowShortcuts,
	KeybindAction::SwitchConversation,
	KeybindAction::SearchConversation,
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

/// Draws the built-in binding sections and the custom rows, sharing the capture slot so
/// only one row records at a time.
pub(super) fn show(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<CaptureTarget>,
	polled_down: u32,
	global_status: &str,
) {
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
		"Development",
		"Copy build and environment information for an issue report.",
		&[KeybindAction::CopyIssueDiagnostics],
		bindings,
		capturing,
	);
	show_voice(ui, bindings, capturing, polled_down, global_status);
	ui.add_space(10.0);
	custom_section(ui, bindings, capturing);
}

/// Draws the voice bindings, the global-keybind switch with its status, and the capture
/// row that records lone modifiers and pad buttons.
pub(super) fn show_voice(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<CaptureTarget>,
	polled_down: u32,
	global_status: &str,
) {
	let colors = design::palette(ui);
	voice_section(ui, bindings, capturing);
	ui.add_space(10.0);
	ui.label(design::eyebrow(
		ui,
		crate::i18n::translate("keybinds-show-voice-global-availability"),
		colors.muted,
	));
	design::switch(
		ui,
		"keybinds-show-voice-enable-global-keybinds",
		Some("keybinds-show-voice-use-voice-shortcuts-while-another-app-is-focused-when-off"),
		&mut bindings.global_enabled,
	);
	if bindings.global_enabled {
		design::hint(ui, global_status);
	}
	capture(ui, bindings, capturing, polled_down);
}

fn voice_section(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<CaptureTarget>,
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
	target: CaptureTarget,
	conflicting_action: KeybindAction,
	time: f64,
}

impl Default for ConflictNotice {
	fn default() -> Self {
		Self {
			target: CaptureTarget::BuiltIn(KeybindAction::ShowShortcuts),
			conflicting_action: KeybindAction::ShowShortcuts,
			time: 0.0,
		}
	}
}

/// Set on the frame a shortcut button starts capture, so the Enter or Space that
/// activated it from the keyboard is not recorded as the new binding.
#[derive(Clone, Copy, Default)]
struct CaptureStarted;

/// The native poller's down mask kept between frames, plus the side modifier whose
/// release records the binding.
#[derive(Clone, Copy, Default)]
struct PolledCapture {
	mask: u32,
	armed: u32,
}

fn polled_capture_id() -> egui::Id {
	egui::Id::unique("keybind_polled_capture")
}

/// First key set in `rising` from the polled table that is, or is not, a pad button.
fn rising_polled_key(rising: u32, pad: bool) -> Option<&'static str> {
	POLLED_KEYS
		.iter()
		.enumerate()
		.find(|(index, name)| rising & (1 << index) != 0 && is_pad_button(name) == pad)
		.map(|(_, name)| *name)
}

/// Every polled key that is a side modifier rather than a pad button.
fn side_modifier_mask() -> u32 {
	POLLED_KEYS
		.iter()
		.enumerate()
		.filter(|(_, name)| !is_pad_button(name))
		.fold(0, |mask, (index, _)| mask | (1 << index))
}

/// Records the next chord into the active capture target. Escape wins over a same-frame
/// rise, side modifiers bind on release, and pad buttons bind on press.
fn capture(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<CaptureTarget>,
	polled_down: u32,
) {
	let Some(target) = *capturing else {
		return;
	};
	let action = match target {
		CaptureTarget::BuiltIn(action) => Some(action),
		CaptureTarget::Custom(index) => bindings.custom.get(index).map(|binding| binding.action),
	};
	let mut polled: PolledCapture = ui
		.data(|data| data.get_temp(polled_capture_id()))
		.unwrap_or_default();
	let previous = std::mem::replace(&mut polled.mask, polled_down);
	let started = ui
		.data_mut(|data| data.remove_temp::<CaptureStarted>(capture_started_id()))
		.is_some();
	// A side modifier armed by an earlier capture must not record into this one.
	if started {
		polled.armed = 0;
	}
	// Escape wins over a pad button or side modifier that rose in the same frame, so
	// abandoning a capture can never record one of them instead.
	let escaped = ui.input(|input| {
		input.events.iter().any(|event| {
			matches!(
				event,
				Event::Key {
					key: Key::Escape,
					pressed: true,
					repeat: false,
					..
				}
			)
		})
	});
	let mut captured = None;
	// Only global actions take lone side modifiers or pad buttons. The native poller
	// dispatches them, so egui never sees a press that the loop below could record.
	// Presses made in another app never record; the mask still tracks them above, so
	// they do not replay on refocus.
	if escaped || !ui.input(|input| input.focused) {
		polled.armed = 0;
	} else if action.is_some_and(KeybindAction::is_global) {
		let rising = polled_down & !previous;
		let falling = previous & !polled_down;
		if let Some(name) = rising_polled_key(rising, true) {
			captured = Some(KeyChord::new(name, 0));
		} else {
			let side = side_modifier_mask();
			// Two side modifiers at once is a chord Serein cannot bind, so never arm
			// one while another is down and drop it the moment another joins.
			if polled.armed & side != 0 && polled_down & side & !polled.armed != 0 {
				polled.armed = 0;
			} else if polled.armed == 0
				&& let Some(name) = rising_polled_key(rising, false)
			{
				let bit = polled_key_bit(name).unwrap_or_default();
				if polled_down & side & !bit == 0 {
					polled.armed = bit;
				}
			}
			let pressed = ui.input(|input| {
				input.events.iter().any(|event| {
					matches!(
						event,
						Event::Key { pressed: true, .. }
							| Event::PointerButton { pressed: true, .. }
					)
				})
			});
			if pressed {
				// A real key or button wins: the normal capture path records it below.
				polled.armed = 0;
			} else if polled.armed & falling != 0 {
				captured = POLLED_KEYS
					.get(polled.armed.trailing_zeros() as usize)
					.map(|name| KeyChord::new(name, 0));
				polled.armed = 0;
			}
		}
	} else {
		polled.armed = 0;
	}
	if captured.is_some() {
		polled.armed = 0;
	}
	ui.data_mut(|data| data.insert_temp(polled_capture_id(), polled));
	if started {
		return;
	}
	let mut cancelled = escaped;
	if captured.is_none() && !cancelled {
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
				// Left and right click stay reserved for pointing, so either one cancels.
				Event::PointerButton {
					button,
					pressed: true,
					modifiers,
					..
				} => {
					match button_name(button) {
						Some(name) => {
							captured = Some(KeyChord::new(name, modifier_bits(modifiers)))
						}
						None => cancelled = true,
					}
					break;
				}
				_ => {}
			}
		}
	}
	if cancelled || captured.is_some() {
		// The press that ended capture must not also act on the rest of this frame.
		ui.input_mut(|input| {
			input
				.events
				.retain(|event| !matches!(event, Event::PointerButton { pressed: true, .. }));
		});
	}
	if cancelled {
		*capturing = None;
	} else if let Some(chord) = captured {
		record(ui, bindings, target, chord);
		*capturing = None;
	}
}

/// Applies `chord` to the capturing row, or shows the conflict notice instead.
fn record(ui: &mut egui::Ui, bindings: &mut Keybinds, target: CaptureTarget, chord: KeyChord) {
	let (ignore_builtin, ignore_custom) = match target {
		CaptureTarget::BuiltIn(action) => (Some(action), None),
		CaptureTarget::Custom(index) => (None, Some(index)),
	};
	if let Some(other) = bindings.conflict(&chord, ignore_builtin, ignore_custom) {
		let now = ui.input(|input| input.time);
		ui.data_mut(|data| {
			data.insert_temp(
				egui::Id::unique("keybind_conflict"),
				ConflictNotice {
					target,
					conflicting_action: other,
					time: now,
				},
			);
		});
		ui.ctx().request_repaint();
	} else {
		match target {
			CaptureTarget::BuiltIn(action) => *bindings.chord_mut(action) = chord,
			CaptureTarget::Custom(index) => {
				if let Some(binding) = bindings.custom.get_mut(index) {
					binding.chord = chord;
				}
			}
		}
		ui.data_mut(|data| {
			data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
		});
	}
}

fn section(
	ui: &mut egui::Ui,
	title: &str,
	description: &str,
	actions: &[KeybindAction],
	bindings: &mut Keybinds,
	capturing: &mut Option<CaptureTarget>,
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
	capturing: &mut Option<CaptureTarget>,
) {
	let colors = design::palette(ui);
	let target = CaptureTarget::BuiltIn(action);
	let active = *capturing == Some(target);
	let (blink_factor, fade_alpha, conflict_text) = conflict_visual(ui, target);

	ui.horizontal(|ui| {
		ui.set_min_height(46.0);
		ui.allocate_ui_with_layout(
			egui::vec2((ui.available_width() - 210.0).max(0.0), 46.0),
			egui::Layout::left_to_right(egui::Align::Center),
			|ui| {
				ui.label(action.label());
				if action.is_global() && bindings.global_enabled {
					ui.label(
						RichText::new(crate::i18n::translate("keybinds-row-global"))
							.size(10.0)
							.color(colors.accent),
					);
				}
				if let Some(msg) = &conflict_text {
					let text_color = colors.danger.gamma_multiply(fade_alpha);
					ui.add_space(6.0);
					ui.label(RichText::new(msg).size(11.0).color(text_color));
				}
			},
		);
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			if design::text_action(ui, &crate::i18n::translate("keybinds-row-reset")).clicked() {
				*bindings.chord_mut(action) = Keybinds::default().chord(action).clone();
				if *capturing == Some(target) {
					*capturing = None;
				}
				ui.data_mut(|data| {
					data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
				});
			}
			if shortcut_button(ui, bindings.chord(action), active, blink_factor).clicked() {
				*capturing = Some(target);
				ui.data_mut(|data| {
					data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
					data.insert_temp(capture_started_id(), CaptureStarted);
				});
			}
		});
	});
}

/// Final card: extra bindings the user adds on top of the fixed rows above.
fn custom_section(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<CaptureTarget>,
) {
	let remove = design::group(
		ui,
		&crate::i18n::translate("keybinds-custom-section-custom-bindings"),
		|ui| {
			design::hint(
				ui,
				&crate::i18n::translate(
					"keybinds-custom-section-add-extra-shortcuts-for-any-action-each-key-or-mouse-button-combination-can-only-be-used-once",
				),
			);
			ui.add_space(4.0);
			let mut remove = None;
			for index in 0..bindings.custom.len() {
				if index > 0 {
					design::card_divider(ui);
				}
				if custom_row(ui, index, bindings, capturing) {
					remove = Some(index);
				}
			}
			if !bindings.custom.is_empty() {
				ui.add_space(8.0);
			}
			let at_limit = bindings.custom.len() >= model::keybinds::MAX_CUSTOM_BINDINGS;
			ui.add_enabled_ui(!at_limit, |ui| {
				if design::secondary_button(
					ui,
					&crate::i18n::translate("keybinds-custom-section-add-binding"),
				)
				.clicked()
				{
					bindings.custom.push(CustomBinding {
						action: KeybindAction::ALL[0],
						chord: KeyChord::new("", 0),
					});
					ui.data_mut(|data| {
						data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
					});
				}
			});
			remove
		},
	);
	if let Some(index) = remove {
		bindings.custom.remove(index);
		*capturing = match *capturing {
			Some(CaptureTarget::Custom(captured)) if captured == index => None,
			Some(CaptureTarget::Custom(captured)) if captured > index => {
				Some(CaptureTarget::Custom(captured - 1))
			}
			other => other,
		};
		ui.data_mut(|data| {
			data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
		});
	}
}

/// One custom binding row: action picker, chord button and the conflict or global hint.
/// Returns whether the row asked to be removed.
fn custom_row(
	ui: &mut egui::Ui,
	index: usize,
	bindings: &mut Keybinds,
	capturing: &mut Option<CaptureTarget>,
) -> bool {
	let colors = design::palette(ui);
	let target = CaptureTarget::Custom(index);
	let active = *capturing == Some(target);
	let action = bindings.custom[index].action;
	let (blink_factor, fade_alpha, conflict_text) = conflict_visual(ui, target);
	let mut remove = false;

	ui.horizontal(|ui| {
		ui.set_min_height(46.0);
		ui.allocate_ui_with_layout(
			egui::vec2((ui.available_width() - 210.0).max(0.0), 46.0),
			egui::Layout::left_to_right(egui::Align::Center),
			|ui| {
				egui::ComboBox::from_id_salt(("keybind-custom-action", index))
					.width((ui.available_width() - 12.0).clamp(80.0, 210.0))
					.selected_text(action.label())
					.show_ui(ui, |ui| {
						for candidate in KeybindAction::ALL {
							ui.selectable_value(
								&mut bindings.custom[index].action,
								candidate,
								candidate.label(),
							);
						}
					});
				// A pad button or lone modifier recorded for a voice action cannot stay
				// once the row switches to a text action; clearing it keeps the prefs valid.
				normalize_custom_binding(&mut bindings.custom[index]);
				let action = bindings.custom[index].action;
				if action.is_global() && bindings.global_enabled {
					ui.label(
						RichText::new(crate::i18n::translate("keybinds-row-global"))
							.size(10.0)
							.color(colors.accent),
					);
				}
				if let Some(msg) = &conflict_text {
					let text_color = colors.danger.gamma_multiply(fade_alpha);
					ui.add_space(6.0);
					ui.label(RichText::new(msg).size(11.0).color(text_color));
				}
			},
		);
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			if design::text_action(ui, &crate::i18n::translate("keybinds-custom-row-remove"))
				.clicked()
			{
				remove = true;
			}
			if shortcut_button(ui, &bindings.custom[index].chord, active, blink_factor).clicked() {
				*capturing = Some(target);
				ui.data_mut(|data| {
					data.remove_temp::<ConflictNotice>(egui::Id::unique("keybind_conflict"));
					data.insert_temp(capture_started_id(), CaptureStarted);
				});
			}
		});
	});
	remove
}

/// Drops a recorded pad button or lone modifier from a custom row whose action no
/// longer accepts it, so the row reads as unrecorded instead of invalidating the prefs.
fn normalize_custom_binding(binding: &mut CustomBinding) {
	if !binding.action.is_global() && polled_key_bit(&binding.chord.key).is_some() {
		binding.chord = KeyChord::new("", 0);
	}
}

/// Blink strength, fade alpha and the conflict message for the row being captured.
fn conflict_visual(ui: &egui::Ui, target: CaptureTarget) -> (f32, f32, Option<String>) {
	let notice: Option<ConflictNotice> =
		ui.data(|data| data.get_temp(egui::Id::unique("keybind_conflict")));
	let Some(notice) = notice.filter(|notice| notice.target == target) else {
		return (0.0, 0.0, None);
	};
	let elapsed = (ui.input(|input| input.time) - notice.time) as f32;
	const TOTAL_DURATION: f32 = 2.5;
	const FADE_START: f32 = 1.0;
	if elapsed >= TOTAL_DURATION {
		return (0.0, 0.0, None);
	}
	ui.ctx().request_repaint();
	let blink_factor = if elapsed < 0.6 {
		let pulse = (elapsed * std::f32::consts::PI * 5.0).sin().abs();
		pulse * pulse
	} else {
		0.0
	};
	let fade_alpha = if elapsed < FADE_START {
		1.0
	} else {
		(1.0 - (elapsed - FADE_START) / (TOTAL_DURATION - FADE_START)).clamp(0.0, 1.0)
	};
	(
		blink_factor,
		fade_alpha,
		Some(format!(
			"Already bound to {}.",
			notice.conflicting_action.label()
		)),
	)
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

/// Consumes this frame's press of `chord` for a focused window, matching modifiers
/// logically. Polled keys belong to the native reader, so they never match here.
pub(crate) fn pressed(input: &mut InputState, chord: &KeyChord) -> bool {
	// The native poller owns these, so egui must never consume them as shortcuts.
	if polled_key_bit(&chord.key).is_some() {
		return false;
	}
	if let Some(button) = button_from_name(&chord.key) {
		let wanted = egui_modifiers(chord.modifiers);
		consume_button(input, button, |modifiers| {
			modifiers.matches_logically(wanted)
		})
	} else {
		key_name_to_egui(&chord.key)
			.is_some_and(|key| input.consume_key(egui_modifiers(chord.modifiers), key))
	}
}

/// Like [`pressed`], but every modifier must match exactly, so a registered global chord
/// stays distinct from its focused variant.
pub(crate) fn pressed_exact(input: &mut InputState, chord: &KeyChord) -> bool {
	if polled_key_bit(&chord.key).is_some() {
		return false;
	}
	if let Some(button) = button_from_name(&chord.key) {
		return consume_button(input, button, |modifiers| {
			modifier_bits(modifiers) == chord.modifiers
		});
	}
	let Some(key) = key_name_to_egui(&chord.key) else {
		return false;
	};
	let matched = input.events.iter().any(|event| {
		matches!(event, Event::Key { key: event_key, pressed: true, repeat: false, modifiers, .. }
			if *event_key == key && modifier_bits(*modifiers) == chord.modifiers)
	});
	matched && input.consume_key(egui_modifiers(chord.modifiers), key)
}

/// Removes this frame's presses of `button` when one matches, like `consume_key`.
fn consume_button(
	input: &mut InputState,
	button: egui::PointerButton,
	matches: impl Fn(Modifiers) -> bool,
) -> bool {
	let matched = input.events.iter().any(|event| {
		matches!(event, Event::PointerButton { button: pressed_button, pressed: true, modifiers, .. }
			if *pressed_button == button && matches(*modifiers))
	});
	if matched {
		input.events.retain(|event| {
			!matches!(event, Event::PointerButton { button: pressed_button, pressed: true, .. }
				if *pressed_button == button)
		});
	}
	matched
}

/// Whether `chord` is held right now in the focused window; polled keys are native-only.
pub(crate) fn down(input: &InputState, chord: &KeyChord) -> bool {
	if polled_key_bit(&chord.key).is_some() {
		return false;
	}
	let held = match button_from_name(&chord.key) {
		Some(button) => input.pointer.button_down(button),
		None => key_name_to_egui(&chord.key).is_some_and(|key| input.key_down(key)),
	};
	held && input
		.modifiers
		.matches_logically(egui_modifiers(chord.modifiers))
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

/// The clickable chord button: accent while recording, a pulsing danger tint for a
/// conflict, and the chord labels centred inside.
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

/// Key name as the settings UI shows it, using macOS spelling for modifiers on macOS.
fn display_key(name: &str) -> String {
	// macOS names the modifier keys the way its own shortcuts UI does.
	if cfg!(target_os = "macos") {
		match name {
			"ControlLeft" => return "Left Control".into(),
			"ControlRight" => return "Right Control".into(),
			"AltLeft" => return "Left Option".into(),
			"AltRight" => return "Right Option".into(),
			_ => {}
		}
	}
	match name {
		"" => "Unassigned".into(),
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
		"MediaPlayPause" => "Play / Pause".into(),
		"MediaTrackNext" => "Next Track".into(),
		"MediaTrackPrevious" => "Previous Track".into(),
		"MediaStop" => "Media Stop".into(),
		"AudioVolumeMute" => "Volume Mute".into(),
		"AudioVolumeDown" => "Volume Down".into(),
		"AudioVolumeUp" => "Volume Up".into(),
		"CapsLock" => "Caps Lock".into(),
		"NumLock" => "Num Lock".into(),
		"ScrollLock" => "Scroll Lock".into(),
		"PrintScreen" => "PrtSc".into(),
		"Menu" => "Menu".into(),
		"Fn" => "Fn".into(),
		"MouseMiddle" => "Mouse 3".into(),
		"MouseExtra1" => "Mouse 4".into(),
		"MouseExtra2" => "Mouse 5".into(),
		"ControlLeft" => "Left Ctrl".into(),
		"ControlRight" => "Right Ctrl".into(),
		"ShiftLeft" => "Left Shift".into(),
		"ShiftRight" => "Right Shift".into(),
		"AltLeft" => "Left Alt".into(),
		"AltRight" => "Right Alt".into(),
		"PadSouth" => "Cross".into(),
		"PadEast" => "Circle".into(),
		"PadWest" => "Square".into(),
		"PadNorth" => "Triangle".into(),
		"PadUp" => "D-pad Up".into(),
		"PadDown" => "D-pad Down".into(),
		"PadLeft" => "D-pad Left".into(),
		"PadRight" => "D-pad Right".into(),
		"PadL1" => "L1".into(),
		"PadR1" => "R1".into(),
		"PadL2" => "L2".into(),
		"PadR2" => "R2".into(),
		"PadL3" => "L3".into(),
		"PadR3" => "R3".into(),
		"OpenBracket" => "[".into(),
		"CloseBracket" => "]".into(),
		"OpenCurlyBracket" => "{".into(),
		"CloseCurlyBracket" => "}".into(),
		"Backslash" => "\\".into(),
		"IntlBackslash" => "\\".into(),
		"Semicolon" => ";".into(),
		"Colon" => ":".into(),
		"Quote" => "'".into(),
		"Backtick" => "`".into(),
		"Minus" => "-".into(),
		"Equals" => "=".into(),
		"Comma" => ",".into(),
		"Period" => ".".into(),
		"Plus" => "+".into(),
		"Pipe" => "|".into(),
		"Questionmark" => "?".into(),
		"Exclamationmark" => "!".into(),
		name if name
			.strip_prefix("Num")
			.is_some_and(|digit| digit.len() == 1) =>
		{
			name[3..].into()
		}
		other => other.to_uppercase(),
	}
}

fn capture_started_id() -> egui::Id {
	egui::Id::unique("keybind_capture_started")
}

/// Bindable mouse buttons, named for [`model::keybinds::is_mouse_button`].
const BUTTONS: &[(egui::PointerButton, &str)] = &[
	(egui::PointerButton::Middle, "MouseMiddle"),
	(egui::PointerButton::Extra1, "MouseExtra1"),
	(egui::PointerButton::Extra2, "MouseExtra2"),
];

fn button_name(button: egui::PointerButton) -> Option<&'static str> {
	BUTTONS
		.iter()
		.find(|(candidate, _)| *candidate == button)
		.map(|(_, name)| *name)
}

pub(crate) fn button_from_name(name: &str) -> Option<egui::PointerButton> {
	BUTTONS
		.iter()
		.find(|(_, candidate)| *candidate == name)
		.map(|(button, _)| *button)
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
	(Key::Colon, "Colon"),
	(Key::Semicolon, "Semicolon"),
	(Key::Backslash, "Backslash"),
	(Key::IntlBackslash, "IntlBackslash"),
	(Key::Pipe, "Pipe"),
	(Key::Questionmark, "Questionmark"),
	(Key::Exclamationmark, "Exclamationmark"),
	(Key::OpenBracket, "OpenBracket"),
	(Key::CloseBracket, "CloseBracket"),
	(Key::OpenCurlyBracket, "OpenCurlyBracket"),
	(Key::CloseCurlyBracket, "CloseCurlyBracket"),
	(Key::Plus, "Plus"),
	(Key::Quote, "Quote"),
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
	(Key::F13, "F13"),
	(Key::F14, "F14"),
	(Key::F15, "F15"),
	(Key::F16, "F16"),
	(Key::F17, "F17"),
	(Key::F18, "F18"),
	(Key::F19, "F19"),
	(Key::F20, "F20"),
	(Key::F21, "F21"),
	(Key::F22, "F22"),
	(Key::F23, "F23"),
	(Key::F24, "F24"),
	(Key::BrowserBack, "BrowserBack"),
	(Key::BrowserForward, "BrowserForward"),
	(Key::BrowserRefresh, "BrowserRefresh"),
	(Key::BrowserSearch, "BrowserSearch"),
	(Key::BrowserHome, "BrowserHome"),
	(Key::BrowserFavorites, "BrowserFavorites"),
	(Key::BrowserStop, "BrowserStop"),
	(Key::MediaPlayPause, "MediaPlayPause"),
	(Key::MediaTrackNext, "MediaTrackNext"),
	(Key::MediaTrackPrevious, "MediaTrackPrevious"),
	(Key::MediaStop, "MediaStop"),
	(Key::AudioVolumeMute, "AudioVolumeMute"),
	(Key::AudioVolumeDown, "AudioVolumeDown"),
	(Key::AudioVolumeUp, "AudioVolumeUp"),
	(Key::LaunchMail, "LaunchMail"),
	(Key::LaunchApp1, "LaunchApp1"),
	(Key::LaunchApp2, "LaunchApp2"),
	(Key::CapsLock, "CapsLock"),
	(Key::NumLock, "NumLock"),
	(Key::ScrollLock, "ScrollLock"),
	(Key::PrintScreen, "PrintScreen"),
	(Key::Pause, "Pause"),
	(Key::Menu, "Menu"),
	(Key::Fn, "Fn"),
	(Key::Eject, "Eject"),
	(Key::Help, "Help"),
	(Key::Power, "Power"),
	(Key::Sleep, "Sleep"),
	(Key::Clear, "Clear"),
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
	fn conflicting_keybind_leaves_binding_unchanged() {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::ToggleDeafen));
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
				capture(ui, &mut bindings, &mut capturing, 0);
			},
		);
		output.textures_delta.clear();

		assert_eq!(capturing, None);
		assert_eq!(bindings.chord(KeybindAction::ToggleDeafen), &deafen_before);
	}

	fn press(button: egui::PointerButton) -> Event {
		Event::PointerButton {
			pos: egui::pos2(100.0, 100.0),
			button,
			pressed: true,
			modifiers: Modifiers::NONE,
		}
	}

	fn capture_frame(
		ctx: &egui::Context,
		events: Vec<Event>,
		polled_down: u32,
		bindings: &mut Keybinds,
		capturing: &mut Option<CaptureTarget>,
	) {
		let mut output = ctx.run_ui(
			egui::RawInput {
				events,
				..Default::default()
			},
			|ui| capture(ui, bindings, capturing, polled_down),
		);
		output.textures_delta.clear();
	}

	fn polled_bit(name: &str) -> u32 {
		model::keybinds::polled_key_bit(name).unwrap()
	}

	#[test]
	fn lone_right_ctrl_records_on_release() {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let ctx = egui::Context::default();
		let right_ctrl = polled_bit("ControlRight");
		capture_frame(&ctx, vec![], right_ctrl, &mut bindings, &mut capturing);
		assert_eq!(
			capturing,
			Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk))
		);
		capture_frame(&ctx, vec![], 0, &mut bindings, &mut capturing);
		assert_eq!(capturing, None);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("ControlRight", 0)
		);
	}

	/// Two side modifiers rising together never arm a lone-modifier binding.
	#[test]
	fn overlapping_modifiers_record_nothing() {
		let mut bindings = Keybinds::default();
		let before = bindings.chord(KeybindAction::PushToTalk).clone();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let ctx = egui::Context::default();
		let right_ctrl = polled_bit("ControlRight");
		let left_shift = polled_bit("ShiftLeft");
		for down in [right_ctrl, right_ctrl | left_shift, right_ctrl, 0] {
			capture_frame(&ctx, vec![], down, &mut bindings, &mut capturing);
		}
		assert_eq!(
			capturing,
			Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk))
		);
		assert_eq!(bindings.chord(KeybindAction::PushToTalk), &before);
		// Releasing everything and starting over records the lone modifier normally.
		capture_frame(&ctx, vec![], right_ctrl, &mut bindings, &mut capturing);
		capture_frame(&ctx, vec![], 0, &mut bindings, &mut capturing);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("ControlRight", 0)
		);
	}

	#[test]
	fn escape_wins_over_a_pad_button_rising_in_the_same_frame() {
		let mut bindings = Keybinds::default();
		let before = bindings.chord(KeybindAction::PushToTalk).clone();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let ctx = egui::Context::default();
		capture_frame(
			&ctx,
			vec![Event::Key {
				key: Key::Escape,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: Modifiers::NONE,
			}],
			polled_bit("PadSouth"),
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(capturing, None);
		assert_eq!(bindings.chord(KeybindAction::PushToTalk), &before);
	}

	#[test]
	fn side_modifiers_rising_together_never_arm() {
		let mut bindings = Keybinds::default();
		let before = bindings.chord(KeybindAction::PushToTalk).clone();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let ctx = egui::Context::default();
		let both = polled_bit("ControlRight") | polled_bit("ShiftLeft");
		capture_frame(&ctx, vec![], both, &mut bindings, &mut capturing);
		capture_frame(&ctx, vec![], 0, &mut bindings, &mut capturing);
		assert_eq!(
			capturing,
			Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk))
		);
		assert_eq!(bindings.chord(KeybindAction::PushToTalk), &before);
	}

	/// A modifier armed by a cancelled capture cannot record into the next one.
	#[test]
	fn new_capture_ignores_a_modifier_armed_by_the_last_one() {
		let mut bindings = Keybinds::default();
		let before = bindings.chord(KeybindAction::ToggleMute).clone();
		let ctx = egui::Context::default();
		let right_ctrl = polled_bit("ControlRight");
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		capture_frame(&ctx, vec![], right_ctrl, &mut bindings, &mut capturing);
		// Leaving the page ends that capture while Right Ctrl is still held.
		capturing = Some(CaptureTarget::BuiltIn(KeybindAction::ToggleMute));
		ctx.data_mut(|data| data.insert_temp(capture_started_id(), CaptureStarted));
		capture_frame(&ctx, vec![], right_ctrl, &mut bindings, &mut capturing);
		capture_frame(&ctx, vec![], 0, &mut bindings, &mut capturing);
		assert_eq!(bindings.chord(KeybindAction::ToggleMute), &before);
	}

	#[test]
	fn pad_press_in_another_app_does_not_record() {
		let mut bindings = Keybinds::default();
		let before = bindings.chord(KeybindAction::ToggleMute).clone();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::ToggleMute));
		let ctx = egui::Context::default();
		let south = polled_bit("PadSouth");
		let mut output = ctx.run_ui(
			egui::RawInput {
				focused: false,
				..Default::default()
			},
			|ui| capture(ui, &mut bindings, &mut capturing, south),
		);
		output.textures_delta.clear();
		// Still held on refocus: not a new press, so nothing records.
		capture_frame(&ctx, vec![], south, &mut bindings, &mut capturing);
		assert_eq!(bindings.chord(KeybindAction::ToggleMute), &before);
		assert!(capturing.is_some());
	}

	#[test]
	fn ctrl_combo_still_records_combo() {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let ctx = egui::Context::default();
		let right_ctrl = polled_bit("ControlRight");
		capture_frame(&ctx, vec![], right_ctrl, &mut bindings, &mut capturing);
		capture_frame(
			&ctx,
			vec![Event::Key {
				key: Key::K,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: Modifiers::CTRL,
			}],
			right_ctrl,
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(capturing, None);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("K", model::keybinds::CTRL)
		);
		capture_frame(&ctx, vec![], 0, &mut bindings, &mut capturing);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("K", model::keybinds::CTRL)
		);
	}

	#[test]
	fn unmapped_key_press_disarms_lone_modifier() {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let ctx = egui::Context::default();
		let right_ctrl = polled_bit("ControlRight");
		capture_frame(&ctx, vec![], right_ctrl, &mut bindings, &mut capturing);
		// Copy has no name in the binding table, so it neither records nor ends capture;
		// it must still disarm the side modifier before it is released.
		capture_frame(
			&ctx,
			vec![Event::Key {
				key: Key::Copy,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: Modifiers::NONE,
			}],
			right_ctrl,
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(
			capturing,
			Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk))
		);
		capture_frame(&ctx, vec![], 0, &mut bindings, &mut capturing);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("V", 0)
		);
	}

	#[test]
	fn custom_row_action_change_clears_polled_chord() {
		let mut bindings = Keybinds {
			custom: vec![CustomBinding {
				action: KeybindAction::PushToTalk,
				chord: KeyChord::new("PadSouth", 0),
			}],
			..Default::default()
		};
		assert!(bindings.is_valid());
		bindings.custom[0].action = KeybindAction::Bold;
		normalize_custom_binding(&mut bindings.custom[0]);
		assert_eq!(bindings.custom[0].chord, KeyChord::new("", 0));
		assert!(bindings.is_valid());
	}

	#[test]
	fn polled_keys_ignored_for_text_rows() {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::Bold));
		let ctx = egui::Context::default();
		let right_ctrl = polled_bit("ControlRight");
		capture_frame(&ctx, vec![], right_ctrl, &mut bindings, &mut capturing);
		capture_frame(&ctx, vec![], 0, &mut bindings, &mut capturing);
		assert_eq!(
			bindings.chord(KeybindAction::Bold),
			&KeyChord::new("B", model::keybinds::PRIMARY)
		);
		assert_eq!(capturing, Some(CaptureTarget::BuiltIn(KeybindAction::Bold)));
	}

	#[test]
	fn pad_button_records_on_press() {
		let mut bindings = Keybinds {
			custom: vec![CustomBinding {
				action: KeybindAction::PushToTalk,
				chord: KeyChord::new("", 0),
			}],
			..Default::default()
		};
		let mut capturing = Some(CaptureTarget::Custom(0));
		let ctx = egui::Context::default();
		capture_frame(
			&ctx,
			vec![],
			polled_bit("PadSouth"),
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(capturing, None);
		assert_eq!(bindings.custom[0].chord, KeyChord::new("PadSouth", 0));
	}

	#[test]
	fn held_ptt_modifier_does_not_consume_shortcut() {
		let bindings = Keybinds {
			push_to_talk: KeyChord::new("ControlRight", 0),
			..Default::default()
		};
		let ctx = egui::Context::default();
		let mut shortcut = false;
		let mut ptt = false;
		let mut output = ctx.run_ui(
			egui::RawInput {
				events: vec![Event::Key {
					key: Key::C,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: Modifiers::CTRL,
				}],
				..Default::default()
			},
			|ui| {
				ui.input_mut(|input| {
					ptt = pressed(input, bindings.chord(KeybindAction::PushToTalk));
					shortcut = pressed(input, &KeyChord::new("C", model::keybinds::CTRL));
				});
			},
		);
		output.textures_delta.clear();
		assert!(!ptt);
		assert!(shortcut);
	}

	#[test]
	fn symbol_keys_bind_and_display() {
		assert_eq!(display_key("OpenBracket"), "[");
		assert_eq!(display_key("CloseBracket"), "]");
		assert_eq!(display_key("Backslash"), "\\");
		assert_eq!(display_key("Semicolon"), ";");
		assert_eq!(display_key("Quote"), "'");
		assert_eq!(display_key("Minus"), "-");
		assert_eq!(display_key("Slash"), "/");

		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let ctx = egui::Context::default();
		capture_frame(
			&ctx,
			vec![Event::Key {
				key: Key::OpenBracket,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: Modifiers::COMMAND,
			}],
			0,
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(capturing, None);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("OpenBracket", modifier_bits(Modifiers::COMMAND))
		);
		let parts = chord_parts(bindings.chord(KeybindAction::PushToTalk));
		assert_eq!(parts.last().map(String::as_str), Some("["));
	}

	/// A bound mouse button feeds the hold paths instead of being swallowed as navigation.
	#[test]
	fn mouse_buttons_bind_and_hold() {
		assert_eq!(display_key("MouseMiddle"), "Mouse 3");
		assert_eq!(display_key("MouseExtra1"), "Mouse 4");
		assert_eq!(display_key("MouseExtra2"), "Mouse 5");
		for (button, name) in BUTTONS {
			assert!(model::keybinds::is_mouse_button(name));
			assert_eq!(button_from_name(name), Some(*button));
		}

		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let ctx = egui::Context::default();
		capture_frame(
			&ctx,
			vec![press(egui::PointerButton::Extra1)],
			0,
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(capturing, None);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("MouseExtra1", 0)
		);

		let mut held = false;
		let mut output = ctx.run_ui(
			egui::RawInput {
				events: vec![press(egui::PointerButton::Extra1)],
				..Default::default()
			},
			|ui| held = ui.input(|input| down(input, bindings.chord(KeybindAction::PushToTalk))),
		);
		output.textures_delta.clear();
		assert!(held);
	}

	/// Push-to-mute ships unassigned, so no default hold can mute a call by accident.
	#[test]
	fn push_to_mute_starts_unassigned() {
		let mut bindings = Keybinds::default();
		assert_eq!(
			chord_parts(bindings.chord(KeybindAction::PushToMute)),
			vec!["Unassigned"]
		);
		let ctx = egui::Context::default();
		let mut pressed_unassigned = true;
		let mut output = ctx.run_ui(
			egui::RawInput {
				events: vec![press(egui::PointerButton::Middle)],
				..Default::default()
			},
			|ui| {
				pressed_unassigned = ui.input_mut(|input| {
					pressed(input, bindings.chord(KeybindAction::PushToMute))
						|| down(input, bindings.chord(KeybindAction::PushToMute))
				});
			},
		);
		output.textures_delta.clear();
		assert!(!pressed_unassigned);

		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToMute));
		capture_frame(
			&ctx,
			vec![press(egui::PointerButton::Extra2)],
			0,
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(
			chord_parts(bindings.chord(KeybindAction::PushToMute)),
			vec!["Mouse 5"]
		);
	}

	/// Left and right click cancel capture instead of being recorded as a binding.
	#[test]
	fn pointing_buttons_cancel_capture() {
		let mut bindings = Keybinds::default();
		let ctx = egui::Context::default();
		for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
			let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::ToggleMute));
			capture_frame(&ctx, vec![press(button)], 0, &mut bindings, &mut capturing);
			assert_eq!(capturing, None);
		}
		assert_eq!(bindings, Keybinds::default());
	}

	#[test]
	fn activating_key_is_not_captured() {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk));
		let enter = Event::Key {
			key: Key::Enter,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: Modifiers::NONE,
		};
		let ctx = egui::Context::default();
		ctx.data_mut(|data| data.insert_temp(capture_started_id(), CaptureStarted));
		capture_frame(&ctx, vec![enter], 0, &mut bindings, &mut capturing);
		assert_eq!(
			capturing,
			Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk))
		);
		assert_eq!(bindings, Keybinds::default());
	}
}
