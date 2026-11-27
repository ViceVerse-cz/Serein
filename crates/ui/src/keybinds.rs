//! Remappable application shortcuts and the device-local global push-to-talk binding.
use crate::design;
use egui::{Event, InputState, Key, Modifiers, RichText};
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
	KeybindAction::NavigateBack,
	KeybindAction::NavigateForward,
	KeybindAction::PreviousChannel,
	KeybindAction::NextChannel,
	KeybindAction::PreviousUnreadChannel,
	KeybindAction::NextUnreadChannel,
	KeybindAction::PreviousServer,
	KeybindAction::NextServer,
];
const CHAT: &[KeybindAction] = &[
	KeybindAction::MarkChannelRead,
	KeybindAction::OpenEmojiPicker,
	KeybindAction::OpenGifPicker,
	KeybindAction::OpenStickerPicker,
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
	capturing: &mut Option<CaptureTarget>,
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
		"Chat",
		"Act on the open conversation.",
		CHAT,
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
	show_voice(ui, bindings, capturing, global_status);
	ui.add_space(10.0);
	custom_section(ui, bindings, capturing);
}

pub(super) fn show_voice(
	ui: &mut egui::Ui,
	bindings: &mut Keybinds,
	capturing: &mut Option<CaptureTarget>,
	global_status: &str,
) {
	let colors = design::palette(ui);
	voice_section(ui, bindings, capturing);
	ui.add_space(10.0);
	let mut release = bindings
		.push_to_talk_release_ms
		.min(model::keybinds::MAX_PUSH_TO_TALK_RELEASE_MS);
	design::slider_row(
		ui,
		"Push to Talk Release Delay",
		Some("Keep transmitting briefly after you let go, so the end of a word is not cut off."),
		&mut release,
		0..=model::keybinds::MAX_PUSH_TO_TALK_RELEASE_MS,
		" ms",
	);
	bindings.push_to_talk_release_ms = release;
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
	capture(ui, bindings, capturing);
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

/// Lone side-modifier recording: the key that went down alone, recorded when it comes back up.
/// egui reports side modifiers only as physical key presses, so Ctrl+K arrives as ControlLeft
/// then K: the K press disarms and the combo is recorded instead.
#[derive(Clone, Copy, Default, Debug)]
struct SideCapture {
	armed: Option<Key>,
}

fn is_side_key(key: Key) -> bool {
	model::keybinds::is_side_modifier(key.name())
}

/// Advances recording by one egui event and returns the side key to bind, if any.
fn side_step(state: &mut SideCapture, event: &Event) -> Option<&'static str> {
	match event {
		Event::Key {
			key,
			pressed: true,
			repeat: false,
			..
		} if is_side_key(*key) => {
			// A second side key overlapping the first records neither.
			state.armed = state.armed.is_none().then_some(*key);
		}
		Event::Key {
			key,
			pressed: false,
			..
		} if Some(*key) == state.armed => {
			state.armed = None;
			return Some(key.name());
		}
		Event::Key {
			key, pressed: true, ..
		} if !is_side_key(*key) => state.armed = None,
		Event::PointerButton { pressed: true, .. } | Event::WindowFocused(false) => {
			state.armed = None;
		}
		_ => {}
	}
	None
}

fn side_capture_id() -> egui::Id {
	egui::Id::unique("keybind_side_capture")
}

/// The Wayland portal cannot fire an unmodified key, so a side modifier would be a dead binding.
fn side_keys_supported() -> bool {
	!(cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some())
}

fn capture(ui: &mut egui::Ui, bindings: &mut Keybinds, capturing: &mut Option<CaptureTarget>) {
	let Some(target) = *capturing else {
		return;
	};
	if ui
		.data_mut(|data| data.remove_temp::<CaptureStarted>(capture_started_id()))
		.is_some()
	{
		ui.data_mut(|data| data.insert_temp(side_capture_id(), SideCapture::default()));
		return;
	}
	let action = match target {
		CaptureTarget::BuiltIn(action) => Some(action),
		CaptureTarget::Custom(index) => bindings.custom.get(index).map(|binding| binding.action),
	};
	let side_allowed = action.is_some_and(KeybindAction::is_global) && side_keys_supported();
	let mut side =
		ui.data_mut(|data| *data.get_temp_mut_or_default::<SideCapture>(side_capture_id()));
	let mut captured = None;
	let mut cancelled = false;
	for event in ui.input(|input| input.events.clone()) {
		if side_allowed && let Some(name) = side_step(&mut side, &event) {
			captured = Some(KeyChord::new(name, 0));
			break;
		}
		match event {
			Event::Key {
				key,
				pressed: true,
				repeat: false,
				modifiers,
				..
			} if !is_side_key(key) => {
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
					Some(name) => captured = Some(KeyChord::new(name, modifier_bits(modifiers))),
					None => cancelled = true,
				}
				break;
			}
			_ => {}
		}
	}
	ui.data_mut(|data| data.insert_temp(side_capture_id(), side));
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
		*capturing = None;
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
				side_needs_global_hint(ui, action, bindings.chord(action), bindings.global_enabled);
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

/// Lone side modifiers are read only by the global key poller.
fn side_needs_global_hint(
	ui: &mut egui::Ui,
	action: KeybindAction,
	chord: &KeyChord,
	global_enabled: bool,
) {
	if action.is_global() && !global_enabled && model::keybinds::is_side_modifier(&chord.key) {
		ui.label(
			RichText::new(crate::i18n::translate(
				"keybinds-side-modifier-needs-global",
			))
			.size(11.0)
			.color(design::palette(ui).muted),
		);
	}
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
				crate::design::Select::from_id_salt(("keybind-custom-action", index))
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
				// A lone side modifier only works on voice actions.
				let binding = &mut bindings.custom[index];
				if !binding.action.is_global()
					&& model::keybinds::is_side_modifier(&binding.chord.key)
				{
					binding.chord = KeyChord::new("", 0);
				}
				if action.is_global() && bindings.global_enabled {
					ui.label(
						RichText::new(crate::i18n::translate("keybinds-row-global"))
							.size(10.0)
							.color(colors.accent),
					);
				}
				side_needs_global_hint(
					ui,
					action,
					&bindings.custom[index].chord,
					bindings.global_enabled,
				);
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

/// Extra Shift still matches, so a symbol that needs Shift on some layouts keeps working. Extra
/// Alt does not: Windows reports AltGr as Ctrl+Alt, and AltGr+Q must type @ rather than run
/// a Ctrl+Q shortcut.
pub(crate) fn pressed(input: &mut InputState, chord: &KeyChord) -> bool {
	let wanted = egui_modifiers(chord.modifiers);
	let modifiers_match = |modifiers: Modifiers| {
		modifiers.matches_logically(wanted) && (wanted.alt || !modifiers.alt)
	};
	if let Some(button) = button_from_name(&chord.key) {
		return consume_button(input, button, modifiers_match);
	}
	let Some(key) = key_name_to_egui(&chord.key) else {
		return false;
	};
	let matched = input.events.iter().any(|event| {
		matches!(event, Event::Key { key: event_key, pressed: true, modifiers, .. }
			if *event_key == key && modifiers_match(*modifiers))
	});
	matched && input.consume_key(wanted, key)
}

pub(crate) fn pressed_exact(input: &mut InputState, chord: &KeyChord) -> bool {
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

pub(crate) fn down(input: &InputState, chord: &KeyChord) -> bool {
	let held = match button_from_name(&chord.key) {
		Some(button) => input.pointer.button_down(button),
		None => key_name_to_egui(&chord.key).is_some_and(|key| input.key_down(key)),
	};
	// Extra Alt never matches, as in `pressed`.
	let wanted = egui_modifiers(chord.modifiers);
	held && input.modifiers.matches_logically(wanted) && (wanted.alt || !input.modifiers.alt)
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
		"ControlLeft" if cfg!(target_os = "macos") => "Left Control".into(),
		"ControlRight" if cfg!(target_os = "macos") => "Right Control".into(),
		"AltLeft" if cfg!(target_os = "macos") => "Left Option".into(),
		"AltRight" if cfg!(target_os = "macos") => "Right Option".into(),
		"ControlLeft" => "Left Ctrl".into(),
		"ControlRight" => "Right Ctrl".into(),
		"ShiftLeft" => "Left Shift".into(),
		"ShiftRight" => "Right Shift".into(),
		"AltLeft" => "Left Alt".into(),
		"AltRight" => "Right Alt".into(),
		"MouseMiddle" => "Mouse 3".into(),
		"MouseExtra1" => "Mouse 4".into(),
		"MouseExtra2" => "Mouse 5".into(),
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
				capture(ui, &mut bindings, &mut capturing);
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
		bindings: &mut Keybinds,
		capturing: &mut Option<CaptureTarget>,
	) {
		let mut output = ctx.run_ui(
			egui::RawInput {
				events,
				..Default::default()
			},
			|ui| capture(ui, bindings, capturing),
		);
		output.textures_delta.clear();
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
				modifiers: Modifiers::COMMAND | Modifiers::SHIFT,
			}],
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(capturing, None);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new(
				"OpenBracket",
				modifier_bits(Modifiers::COMMAND | Modifiers::SHIFT)
			)
		);
		let parts = chord_parts(bindings.chord(KeybindAction::PushToTalk));
		assert_eq!(parts.last().map(String::as_str), Some("["));
	}

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
			&mut bindings,
			&mut capturing,
		);
		assert_eq!(
			chord_parts(bindings.chord(KeybindAction::PushToMute)),
			vec!["Mouse 5"]
		);
	}

	#[test]
	fn pointing_buttons_cancel_capture() {
		let mut bindings = Keybinds::default();
		let ctx = egui::Context::default();
		for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
			let mut capturing = Some(CaptureTarget::BuiltIn(KeybindAction::ToggleMute));
			capture_frame(&ctx, vec![press(button)], &mut bindings, &mut capturing);
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
		capture_frame(&ctx, vec![enter], &mut bindings, &mut capturing);
		assert_eq!(
			capturing,
			Some(CaptureTarget::BuiltIn(KeybindAction::PushToTalk))
		);
		assert_eq!(bindings, Keybinds::default());
	}

	fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
		Event::Key {
			key,
			physical_key: None,
			pressed,
			repeat: false,
			modifiers,
		}
	}

	/// Runs one frame per event list against a fresh capture of `action`.
	fn record(action: KeybindAction, frames: Vec<Vec<Event>>) -> (Keybinds, bool) {
		let mut bindings = Keybinds::default();
		let mut capturing = Some(CaptureTarget::BuiltIn(action));
		let ctx = egui::Context::default();
		for events in frames {
			capture_frame(&ctx, events, &mut bindings, &mut capturing);
		}
		(bindings, capturing.is_some())
	}

	#[test]
	fn lone_side_modifier_records_on_release() {
		let down = key(Key::ControlRight, true, Modifiers::CTRL);
		let up = key(Key::ControlRight, false, Modifiers::NONE);
		// Press and release in separate frames, and in one frame.
		for frames in [
			vec![vec![down.clone()], vec![up.clone()]],
			vec![vec![down.clone(), up.clone()]],
		] {
			let (bindings, still) = record(KeybindAction::PushToTalk, frames);
			assert!(!still);
			assert_eq!(
				bindings.chord(KeybindAction::PushToTalk),
				&KeyChord::new("ControlRight", 0)
			);
			assert_eq!(
				chord_parts(bindings.chord(KeybindAction::PushToTalk))
					.last()
					.map(String::as_str),
				Some(if cfg!(target_os = "macos") {
					"Right Control"
				} else {
					"Right Ctrl"
				})
			);
		}
		// Non-voice rows wait for a real shortcut.
		let (bindings, still) = record(KeybindAction::Bold, vec![vec![down], vec![up]]);
		assert!(still);
		assert_eq!(bindings, Keybinds::default());
	}

	#[test]
	fn side_modifier_does_not_record_with_other_input() {
		let ctrl = |pressed| {
			key(
				Key::ControlLeft,
				pressed,
				if pressed {
					Modifiers::CTRL
				} else {
					Modifiers::NONE
				},
			)
		};
		// Ctrl+K records the combo, not Left Ctrl.
		let (bindings, still) = record(
			KeybindAction::PushToTalk,
			vec![
				vec![ctrl(true)],
				vec![key(Key::K, true, Modifiers::CTRL)],
				vec![ctrl(false)],
			],
		);
		assert!(!still);
		assert_eq!(
			bindings.chord(KeybindAction::PushToTalk),
			&KeyChord::new("K", modifier_bits(Modifiers::CTRL))
		);
		let shift = |pressed| key(Key::ShiftRight, pressed, Modifiers::NONE);
		for frames in [
			// Two side keys overlapping record neither.
			vec![
				vec![ctrl(true)],
				vec![shift(true)],
				vec![shift(false)],
				vec![ctrl(false)],
			],
			// Focus moving to another app, where egui sees nothing, disarms.
			vec![
				vec![ctrl(true)],
				vec![Event::WindowFocused(false)],
				vec![ctrl(false)],
			],
			// A key already down when capture started has no press to arm it.
			vec![vec![ctrl(false)]],
		] {
			let (bindings, still) = record(KeybindAction::PushToTalk, frames);
			assert!(still);
			assert_eq!(bindings, Keybinds::default());
		}
	}
}
