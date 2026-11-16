//! Right-click menu for editable text: cut, copy, paste and select all, like Discord's.
use egui::text::{CCursor, CCursorRange};

fn focus_key() -> egui::Id {
	egui::Id::unique("text-menu-pending-focus")
}

/// Popup close can clear focus after its owner has already rendered this pass.
/// Retain one target for the following frame, while respecting a new interaction.
pub(crate) fn restore_focus(ctx: &egui::Context, id: egui::Id) {
	let Some((target, frame)) = ctx.data(|data| data.get_temp::<(egui::Id, u64)>(focus_key()))
	else {
		return;
	};
	let current = ctx.cumulative_frame_nr();
	if frame < current || (frame == current && target == id) {
		ctx.data_mut(|data| data.remove::<(egui::Id, u64)>(focus_key()));
	}
	if frame != current || target != id {
		return;
	}
	let available = ctx.memory(|memory| {
		memory.focused().is_none_or(|focused| focused == id) && memory.top_modal_layer().is_none()
	});
	let moved = ctx.input(|input| {
		input.events.iter().any(|event| {
			matches!(
				event,
				egui::Event::PointerButton { pressed: true, .. }
					| egui::Event::Key {
						key: egui::Key::Tab | egui::Key::Escape,
						pressed: true,
						..
					}
			)
		})
	});
	if available && !moved && !egui::Popup::is_any_open(ctx) {
		ctx.memory_mut(|memory| {
			memory.request_focus(id);
			memory.move_focus(egui::FocusDirection::None);
		});
	}
}

pub(crate) fn begin_paste_undo(ctx: &egui::Context, id: egui::Id, text: &str) -> bool {
	let pasted = ctx.memory(|memory| memory.has_focus(id))
		&& ctx.input(|input| {
			input
				.events
				.iter()
				.any(|event| matches!(event, egui::Event::Paste(_)))
		});
	if pasted {
		undo_checkpoint(ctx, id, text);
	}
	pasted
}

pub(crate) fn undo_checkpoint(ctx: &egui::Context, id: egui::Id, text: &str) {
	let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
	let cursor = state
		.cursor
		.char_range()
		.unwrap_or_else(|| CCursorRange::one(CCursor::end_of_str(text)));
	let value = (cursor, text.to_owned());
	let mut undoer = state.undoer();
	undoer.feed_state(ctx.input(|input| input.time), &value);
	undoer.add_undo(&value);
	state.set_undoer(undoer);
	state.store(ctx, id);
}

pub(crate) fn selection_for_menu(ctx: &egui::Context, id: egui::Id) -> Option<CCursorRange> {
	restore_focus(ctx, id);
	(ctx.input(|input| input.pointer.button_pressed(egui::PointerButton::Secondary))
		|| ctx
			.read_response(id)
			.is_some_and(|response| response.context_menu_opened()))
	.then(|| egui::TextEdit::load_state(ctx, id)?.cursor.char_range())
	.flatten()
	.filter(|range| !range.is_empty())
}

/// Shows the menu for the text edit `id` holding `text`. Cut marks the response changed;
/// Paste returns true so the caller can read the system clipboard, which also carries files and
/// images copied in other apps.
pub(crate) fn show(
	response: &mut egui::Response,
	id: egui::Id,
	text: &mut String,
	paste_enabled: bool,
	selection: Option<CCursorRange>,
) -> bool {
	// TextEdit clears selections on secondary presses and when focus moves to the menu.
	if (response.contains_pointer() || response.context_menu_opened())
		&& let Some(selection) = selection
	{
		let mut state = egui::TextEdit::load_state(&response.ctx, id).unwrap_or_default();
		state.cursor.set_char_range(Some(selection));
		state.store(&response.ctx, id);
	}
	let mut paste = false;
	let mut changed = false;
	let mut refocus = false;
	response.context_menu(|ui| {
		let ctx = ui.ctx().clone();
		let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
		let selected = state
			.cursor
			.char_range()
			.map(|range| {
				let [start, end] = range.sorted_cursors();
				byte(text, start.index.0)..byte(text, end.index.0)
			})
			.filter(|range| !range.is_empty());
		let item = |ui: &mut egui::Ui, key: &str, enabled: bool| {
			ui.add_enabled(enabled, egui::Button::new(crate::i18n::translate(key)))
				.clicked()
		};
		if item(ui, "text-menu-cut", selected.is_some())
			&& let Some(range) = selected.clone()
		{
			ctx.copy_text(text[range.clone()].to_owned());
			undo_checkpoint(&ctx, id, text);
			state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
			let start = text[..range.start].chars().count();
			text.replace_range(range, "");
			changed = true;
			state
				.cursor
				.set_char_range(Some(CCursorRange::one(CCursor::new(start))));
			state.clone().store(&ctx, id);
			undo_checkpoint(&ctx, id, text);
			refocus = true;
			ui.close();
		}
		if item(ui, "text-menu-copy", selected.is_some())
			&& let Some(range) = selected.clone()
		{
			ctx.copy_text(text[range].to_owned());
			refocus = true;
			ui.close();
		}
		if item(ui, "text-menu-paste", paste_enabled) {
			refocus = true;
			paste = true;
			ui.close();
		}
		ui.separator();
		if item(ui, "text-menu-select-all", !text.is_empty()) {
			state.cursor.set_char_range(Some(CCursorRange::two(
				CCursor::new(0),
				CCursor::end_of_str(text),
			)));
			state.clone().store(&ctx, id);
			refocus = true;
			ui.close();
		}
	});
	if refocus {
		response.ctx.memory_mut(|memory| memory.request_focus(id));
		let frame = response.ctx.cumulative_frame_nr() + 1;
		response
			.ctx
			.data_mut(|data| data.insert_temp(focus_key(), (id, frame)));
		response.ctx.request_repaint();
	}
	if changed {
		response.mark_changed();
	}
	paste
}

fn byte(text: &str, chars: usize) -> usize {
	text.char_indices()
		.nth(chars)
		.map_or(text.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
	#[test]
	fn pending_menu_focus_expires_and_yields_to_new_navigation() {
		for interaction in 0..5 {
			let ctx = egui::Context::default();
			let target = egui::Id::unique("synthetic-composer");
			let other = egui::Id::unique("synthetic-search");
			let frame = |events, show: &mut dyn FnMut()| {
				let output = ctx.run_ui(
					egui::RawInput {
						events,
						..Default::default()
					},
					|_| show(),
				);
				output.drop_without_applying_deltas();
			};
			frame(vec![], &mut || {
				let due = ctx.cumulative_frame_nr() + 1;
				ctx.data_mut(|data| data.insert_temp(super::focus_key(), (target, due)));
			});
			let events = match interaction {
				0 => vec![egui::Event::PointerButton {
					pos: egui::pos2(30.0, 30.0),
					button: egui::PointerButton::Primary,
					pressed: true,
					modifiers: Default::default(),
				}],
				1 => vec![egui::Event::Key {
					key: egui::Key::Tab,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: Default::default(),
				}],
				2 => vec![egui::Event::Key {
					key: egui::Key::Escape,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: Default::default(),
				}],
				_ => vec![],
			};
			frame(events, &mut || {
				if interaction == 3 {
					ctx.memory_mut(|memory| memory.request_focus(other));
				}
				if interaction != 4 {
					super::restore_focus(&ctx, target);
				}
				assert!(!ctx.memory(|memory| memory.has_focus(target)));
				if interaction == 3 {
					assert!(ctx.memory(|memory| memory.has_focus(other)));
				}
			});
			frame(vec![], &mut || {
				super::restore_focus(&ctx, target);
				assert!(
					!ctx.memory(|memory| memory.has_focus(target)),
					"A discarded or expired request cannot refocus later"
				);
				assert!(
					ctx.data(|data| data.get_temp::<(egui::Id, u64)>(super::focus_key()))
						.is_none()
				);
			});
		}
	}

	#[test]
	fn pending_menu_focus_preserves_macos_word_navigation() {
		let ctx = egui::Context::default();
		ctx.set_os(egui::os::OperatingSystem::Mac);
		let id = egui::Id::unique("synthetic-word-navigation");
		let mut text = "one two".to_owned();
		ctx.run_ui(egui::RawInput::default(), |ui| {
			egui::TextEdit::singleline(&mut text).id(id).show(ui);
			let mut state = egui::TextEdit::load_state(&ctx, id).unwrap();
			state
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::one(
					egui::text::CCursor::end_of_str(&text),
				)));
			state.store(&ctx, id);
			ctx.memory_mut(|memory| memory.surrender_focus(id));
			let due = ctx.cumulative_frame_nr() + 1;
			ctx.data_mut(|data| data.insert_temp(super::focus_key(), (id, due)));
		})
		.drop_without_applying_deltas();
		ctx.run_ui(
			egui::RawInput {
				events: vec![
					egui::Event::Key {
						key: egui::Key::ArrowLeft,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::ALT,
					},
					egui::Event::Text("X".into()),
				],
				..Default::default()
			},
			|ui| {
				super::restore_focus(&ctx, id);
				egui::TextEdit::singleline(&mut text).id(id).show(ui);
				assert!(ctx.memory(|memory| memory.has_focus(id)));
			},
		)
		.drop_without_applying_deltas();
		assert_eq!(text, "one Xtwo");
	}

	#[test]
	fn char_cursors_map_to_byte_offsets() {
		assert_eq!(super::byte("héllo", 2), 3);
		assert_eq!(super::byte("héllo", 9), 6);
		assert_eq!(super::byte("", 0), 0);
	}
}
