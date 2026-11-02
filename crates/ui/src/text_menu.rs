//! Right-click menu for editable text: cut, copy, paste and select all, like Discord's.
use egui::text::{CCursor, CCursorRange};

pub(crate) fn selection_for_menu(ctx: &egui::Context, id: egui::Id) -> Option<CCursorRange> {
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
			let start = text[..range.start].chars().count();
			text.replace_range(range, "");
			changed = true;
			state
				.cursor
				.set_char_range(Some(CCursorRange::one(CCursor::new(start))));
			state.clone().store(&ctx, id);
			ctx.memory_mut(|memory| memory.request_focus(id));
			ui.close();
		}
		if item(ui, "text-menu-copy", selected.is_some())
			&& let Some(range) = selected.clone()
		{
			ctx.copy_text(text[range].to_owned());
			ui.close();
		}
		if item(ui, "text-menu-paste", paste_enabled) {
			ctx.memory_mut(|memory| memory.request_focus(id));
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
			ctx.memory_mut(|memory| memory.request_focus(id));
			ui.close();
		}
	});
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
