//! Right-click menu for editable text: cut, copy, paste and select all, like Discord's.
use crate::icons::{self, Icon};
use egui::text::{CCursor, CCursorRange};

fn refocus_id(id: egui::Id) -> egui::Id {
	id.with("text-menu-refocus")
}

/// Call before showing the text edit `id`. Returns the selection a menu opening this frame keeps.
pub(crate) fn selection_for_menu(ctx: &egui::Context, id: egui::Id) -> Option<CCursorRange> {
	// The click on a menu item lands outside the field, which surrenders its focus that frame,
	// so the field takes focus back here, before it is shown again.
	if ctx
		.data_mut(|data| data.remove_temp::<bool>(refocus_id(id)))
		.is_some()
	{
		ctx.memory_mut(|memory| memory.request_focus(id));
	}
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
		// The glyph follows the row's text colour, so it dims when disabled.
		let item = |ui: &mut egui::Ui, icon: Icon, key: &str, enabled: bool| {
			let button = egui::Button::new((
				egui::Atom::paint(egui::Vec2::splat(16.0), move |ui, args| {
					icons::paint(ui.painter(), icon, args.rect, args.fallback_text_color);
				}),
				crate::i18n::translate(key),
			));
			ui.add_enabled(enabled, button).clicked()
		};
		let refocus = |ui: &mut egui::Ui| {
			ctx.memory_mut(|memory| memory.request_focus(id));
			ctx.data_mut(|data| data.insert_temp(refocus_id(id), true));
			ui.close();
		};
		if item(ui, Icon::Cut, "text-menu-cut", selected.is_some())
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
			refocus(ui);
		}
		if item(ui, Icon::Copy, "text-menu-copy", selected.is_some())
			&& let Some(range) = selected.clone()
		{
			ctx.copy_text(text[range].to_owned());
			ui.close();
		}
		if item(ui, Icon::Paste, "text-menu-paste", paste_enabled) {
			paste = true;
			refocus(ui);
		}
		ui.separator();
		if item(
			ui,
			Icon::SelectAll,
			"text-menu-select-all",
			!text.is_empty(),
		) {
			state.cursor.set_char_range(Some(CCursorRange::two(
				CCursor::new(0),
				CCursor::end_of_str(text),
			)));
			state.clone().store(&ctx, id);
			refocus(ui);
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
