use emath::TSTransform;
use epaint::text::{CharIndex, PlacedRow};

use crate::{Context, Galley, Id};

use super::{CCursorRange, text_cursor_state::is_word_char};

/// AccessKit's `word_starts` uses `u8` indices, so text runs cannot exceed this length.
pub(crate) const MAX_CHARS_PER_TEXT_RUN: usize = 255;

/// Convert a (row, column) layout cursor position to a text run node ID and character index,
/// accounting for rows that are split into multiple text runs.
fn text_run_position(parent_id: Id, row: usize, column: CharIndex) -> accesskit::TextPosition {
    let column = column.0;
    // When column lands exactly on a chunk boundary (e.g., 255), it refers to
    // the end of the previous chunk, not the start of a new one.
    let chunk_index = if column > 0 && column.is_multiple_of(MAX_CHARS_PER_TEXT_RUN) {
        column / MAX_CHARS_PER_TEXT_RUN - 1
    } else {
        column / MAX_CHARS_PER_TEXT_RUN
    };
    let character_index = column - chunk_index * MAX_CHARS_PER_TEXT_RUN;
    accesskit::TextPosition {
        node: parent_id.with(row).with(chunk_index).accesskit_id(),
        character_index,
    }
}

/// Update accesskit with the current text state.
///
/// This adds the [`accesskit::Role::TextRun`] children of the widget, and the
/// current text selection. It never touches the role of the widget itself: that
/// is set from the [`crate::WidgetInfo`] the widget reports, so a link stays a
/// link and a button stays a button.
pub fn update_accesskit_for_text_widget(
    ctx: &Context,
    widget_id: Id,
    cursor_range: Option<CCursorRange>,
    global_from_galley: TSTransform,
    galley: &Galley,
) {
    let parent_id = ctx.accesskit_node_builder(widget_id, |builder| {
        let parent_id = widget_id;

        if let Some(cursor_range) = &cursor_range {
            let anchor = galley.layout_from_cursor(cursor_range.secondary);
            let focus = galley.layout_from_cursor(cursor_range.primary);
            builder.set_text_selection(accesskit::TextSelection {
                anchor: text_run_position(parent_id, anchor.row, anchor.column),
                focus: text_run_position(parent_id, focus.row, focus.column),
            });
        }

        parent_id
    });

    let Some(parent_id) = parent_id else {
        return;
    };

    let mut prev_row_ended_with_newline = true;

    for (row_index, row) in galley.rows.iter().enumerate() {
        let glyph_count = row.glyphs.len();
        let mut value = String::with_capacity(glyph_count);
        let mut character_lengths = Vec::<u8>::with_capacity(glyph_count);
        let mut word_starts = Vec::<usize>::new();
        // For soft-wrapped continuation rows, treat the start as a word
        // boundary so the first word character gets a `word_starts` entry.
        // Paragraph-starting runs (first row or after a newline) get an
        // implicit word start from AccessKit, so they don't need this.
        let mut was_at_word_end = !prev_row_ended_with_newline;

        for glyph in &row.glyphs {
            let is_word_char = is_word_char(glyph.chr);
            if is_word_char && was_at_word_end {
                word_starts.push(character_lengths.len());
            }
            was_at_word_end = !is_word_char;
            let old_len = value.len();
            value.push(glyph.chr);
            character_lengths.push((value.len() - old_len) as _);
        }

        if row.ends_with_newline {
            value.push('\n');
            character_lengths.push(1);
        }

        let total_chars = character_lengths.len();

        if total_chars <= MAX_CHARS_PER_TEXT_RUN {
            let run_id = parent_id.with(row_index).with(0usize);
            ctx.register_accesskit_parent(run_id, parent_id);

            ctx.accesskit_node_builder(run_id, |builder| {
                builder.set_role(accesskit::Role::TextRun);
                // TODO(mwcampbell): Set more node fields for the row
                // once AccessKit adapters expose text formatting info.

                set_run_geometry(builder, row, 0..total_chars, global_from_galley);
                builder.set_value(value);
                builder.set_character_lengths(character_lengths);

                let chunk_word_starts: Vec<u8> = word_starts.iter().map(|&ws| ws as u8).collect();
                builder.set_word_starts(chunk_word_starts);
            });
        } else {
            let num_chunks = total_chars.div_ceil(MAX_CHARS_PER_TEXT_RUN);
            let mut byte_offset = 0usize;

            for chunk_idx in 0..num_chunks {
                let char_start = chunk_idx * MAX_CHARS_PER_TEXT_RUN;
                let char_end = (char_start + MAX_CHARS_PER_TEXT_RUN).min(total_chars);

                let byte_start = byte_offset;
                let chunk_byte_len: usize = character_lengths[char_start..char_end]
                    .iter()
                    .map(|&l| l as usize)
                    .sum();
                let byte_end = byte_start + chunk_byte_len;
                byte_offset = byte_end;

                let run_id = parent_id.with(row_index).with(chunk_idx);
                ctx.register_accesskit_parent(run_id, parent_id);

                ctx.accesskit_node_builder(run_id, |builder| {
                    builder.set_role(accesskit::Role::TextRun);
                    // TODO(mwcampbell): Set more node fields for the row
                    // once AccessKit adapters expose text formatting info.

                    if chunk_idx > 0 {
                        let prev_id = parent_id.with(row_index).with(chunk_idx - 1);
                        builder.set_previous_on_line(prev_id.accesskit_id());
                    }
                    if chunk_idx + 1 < num_chunks {
                        let next_id = parent_id.with(row_index).with(chunk_idx + 1);
                        builder.set_next_on_line(next_id.accesskit_id());
                    }

                    set_run_geometry(builder, row, char_start..char_end, global_from_galley);
                    builder.set_value(value[byte_start..byte_end].to_owned());
                    builder.set_character_lengths(character_lengths[char_start..char_end].to_vec());

                    let chunk_word_starts: Vec<u8> = word_starts
                        .iter()
                        .filter(|&&ws| ws >= char_start && ws < char_end)
                        .map(|&ws| (ws - char_start) as u8)
                        .collect();
                    builder.set_word_starts(chunk_word_starts);
                });
            }
        }

        prev_row_ended_with_newline = row.ends_with_newline;
    }
}

/// AccessKit measures character positions from the edge indicated by the run direction.
fn set_run_geometry(
    builder: &mut accesskit::Node,
    row: &PlacedRow,
    range: core::ops::Range<usize>,
    global_from_galley: TSTransform,
) {
    let glyphs = &row.glyphs[range.start.min(row.glyphs.len())..range.end.min(row.glyphs.len())];
    let is_rtl = glyphs
        .first()
        .or_else(|| row.glyphs.last())
        .is_some_and(|glyph| glyph.is_rtl);
    let mut positions = Vec::with_capacity(range.len());
    let mut widths = Vec::with_capacity(range.len());
    let mut left = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    for index in range {
        let (x, width) = row.glyphs.get(index).map_or_else(
            || (row.x_offset(CharIndex(row.glyphs.len())), 0.0),
            |glyph| (glyph.pos.x, glyph.advance_width),
        );
        positions.push(x);
        widths.push(width);
        left = left.min(x);
        right = right.max(x + width);
    }
    if positions.is_empty() {
        left = 0.0;
        right = row.size.x;
    }
    let rect = global_from_galley
        * emath::Rect::from_min_max(
            row.pos + emath::vec2(left, 0.0),
            row.pos + emath::vec2(right, row.size.y),
        );
    builder.set_bounds(accesskit::Rect {
        x0: rect.min.x.into(),
        y0: rect.min.y.into(),
        x1: rect.max.x.into(),
        y1: rect.max.y.into(),
    });

    if glyphs.iter().any(|glyph| glyph.is_rtl != is_rtl) {
        // ponytail: fixed logical chunks cannot express mixed directions; split
        // chunks by bidi runs if mixed-text accessibility geometry is needed.
        builder.clear_text_direction();
        builder.clear_character_positions();
        builder.clear_character_widths();
        return;
    }
    builder.set_text_direction(if is_rtl {
        accesskit::TextDirection::RightToLeft
    } else {
        accesskit::TextDirection::LeftToRight
    });
    for (position, width) in positions.iter_mut().zip(&mut widths) {
        *position = if is_rtl {
            right - (*position + *width)
        } else {
            *position - left
        } * global_from_galley.scaling;
        *width *= global_from_galley.scaling;
    }
    builder.set_character_positions(positions);
    builder.set_character_widths(widths);
}
