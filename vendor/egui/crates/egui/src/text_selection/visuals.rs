use std::sync::Arc;

use emath::Pos2;
use epaint::{
    Stroke,
    text::{
        CharIndex, Row,
        cursor::{CCursor, LayoutCursor},
    },
};

use crate::{
    Galley, Painter, Rect, Ui, Visuals, pos2, text_selection::text_cursor_state::cursor_rect, vec2,
};

use super::CCursorRange;

#[derive(Clone, Debug)]
pub struct RowVertexIndices {
    pub row: usize,
    pub vertex_indices: [u32; 6],
}

/// Adds text selection rectangles to the galley.
pub fn paint_text_selection(
    galley: &mut Arc<Galley>,
    visuals: &Visuals,
    cursor_range: &CCursorRange,
    mut new_vertex_indices: Option<&mut Vec<RowVertexIndices>>,
) {
    if cursor_range.is_empty() {
        return;
    }

    // We need to modify the galley (add text selection painting to it),
    // and so we need to clone it if it is shared:
    let galley: &mut Galley = Arc::make_mut(galley);

    let background_color = visuals.selection.bg_fill;
    let text_color = visuals.selection.stroke.color;

    let [min, max] = cursor_range.sorted_cursors();
    let min = galley.layout_from_cursor(min);
    let max = galley.layout_from_cursor(max);

    for ri in min.row..=max.row {
        let placed_row = &mut galley.rows[ri];
        let row = Arc::make_mut(&mut placed_row.row);

        let newline_size = if ri != max.row && placed_row.ends_with_newline {
            row.height() / 2.0 // visualize that we select the newline
        } else {
            0.0
        };
        let rects = selection_rects(row, ri, min, max, newline_size);
        let mesh = &mut row.visuals.mesh;

        if !row.glyphs.is_empty() {
            // Change color of the selected text:
            let first_glyph_index = if ri == min.row { min.column.0 } else { 0 };
            let last_glyph_index = if ri == max.row {
                max.column.0
            } else {
                row.glyphs.len()
            };

            let first_vertex_index = row
                .glyphs
                .get(first_glyph_index)
                .map_or(row.visuals.glyph_vertex_range.end, |g| g.first_vertex as _);
            let last_vertex_index = row
                .glyphs
                .get(last_glyph_index)
                .map_or(row.visuals.glyph_vertex_range.end, |g| g.first_vertex as _);

            for vi in first_vertex_index..last_vertex_index {
                mesh.vertices[vi].color = text_color;
            }
        }

        // Time to insert the selection rectangle into the row mesh.
        // It should be on top (after) of any background in the galley,
        // but behind (before) any glyphs. The row visuals has this information:
        let glyph_index_start = row.visuals.glyph_index_start;

        // Append all selection rectangles, then move their triangles behind the glyphs.
        let num_indices_before = mesh.indices.len();
        for rect in rects {
            mesh.add_colored_rect(rect, background_color);
            if let Some(new_vertex_indices) = &mut new_vertex_indices {
                new_vertex_indices.push(RowVertexIndices {
                    row: ri,
                    vertex_indices: mesh.indices[mesh.indices.len() - 6..]
                        .try_into()
                        .expect("a rectangle has six indices"),
                });
            }
        }
        let num_selection_indices = mesh.indices.len() - num_indices_before;
        mesh.indices[glyph_index_start..].rotate_right(num_selection_indices);
        row.visuals.mesh_bounds = mesh.calc_bounds();
    }
}

/// A logical selection can occupy several disjoint visual spans in a bidi row.
fn selection_rects(
    row: &Row,
    ri: usize,
    min: LayoutCursor,
    max: LayoutCursor,
    newline_size: f32,
) -> impl Iterator<Item = Rect> + use<> {
    if !row.glyphs.iter().any(|glyph| glyph.is_rtl) {
        let left = if ri == min.row {
            row.x_offset(min.column)
        } else {
            0.0
        };
        let right = if ri == max.row {
            row.x_offset(max.column)
        } else {
            row.size.x + newline_size
        };
        return Some(Rect::from_min_max(pos2(left, 0.0), pos2(right, row.size.y)))
            .into_iter()
            .chain(Vec::new());
    }

    let start = if ri == min.row { min.column.0 } else { 0 };
    let end = if ri == max.row {
        max.column.0
    } else {
        row.glyphs.len()
    };
    let mut rects: Vec<_> = row.glyphs[start..end]
        .iter()
        .map(|glyph| Rect::from_min_max(pos2(glyph.pos.x, 0.0), pos2(glyph.max_x(), row.size.y)))
        .collect();
    if newline_size > 0.0 {
        let x = row.x_offset(CharIndex(row.glyphs.len()));
        let extent = if row.glyphs.last().is_some_and(|glyph| glyph.is_rtl) {
            x - newline_size
        } else {
            x + newline_size
        };
        rects.push(Rect::from_min_max(
            pos2(x.min(extent), 0.0),
            pos2(x.max(extent), row.size.y),
        ));
    }
    rects.sort_unstable_by(|a, b| a.min.x.total_cmp(&b.min.x));
    let mut count = 0;
    for i in 0..rects.len() {
        if count > 0 && rects[i].min.x <= rects[count - 1].max.x {
            rects[count - 1].max.x = rects[count - 1].max.x.max(rects[i].max.x);
        } else {
            rects[count] = rects[i];
            count += 1;
        }
    }
    rects.truncate(count);
    None.into_iter().chain(rects)
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn paint_ime_preedit_text_visuals(
    pos: Pos2,
    ui: &Ui,
    painter: &Painter,
    galley: &Arc<Galley>,
    row_height: f32,
    preedit_range: core::ops::Range<CCursor>,
    mut relative_active_range: Option<core::ops::Range<CCursor>>,
    time_since_last_interaction: f64,
) {
    /// Instead of implementing [`PartialOrd`] and [`Ord`] for [`CCursor`] to
    /// make [`std::ops::Range::is_empty`] available, we use this helper
    /// function instead.
    ///
    /// These traits are intentionally not implemented because
    /// [`CCursor::prefer_next_row`] makes it difficult to define a clear
    /// ordering between two [`CCursor`]s.
    fn is_cursor_range_empty(range: &core::ops::Range<CCursor>) -> bool {
        range.start.index == range.end.index
    }

    if is_cursor_range_empty(&preedit_range) {
        return;
    }

    if let Some(relative_active_range) = &mut relative_active_range
        && relative_active_range.end.index > preedit_range.end.index - preedit_range.start.index
    {
        relative_active_range.end.index = preedit_range.end.index - preedit_range.start.index;
    }

    let visuals = ui.visuals();
    let active_underline_stroke = visuals.ime_composition.active_underline_stroke;
    let inactive_underline_stroke = visuals.ime_composition.inactive_underline_stroke;

    if let Some(relative_active_range) = &relative_active_range
        && !is_cursor_range_empty(relative_active_range)
    {
        if relative_active_range.start.index > CharIndex::ZERO {
            paint_underlines(
                pos,
                painter,
                galley,
                galley.layout_from_cursor(preedit_range.start),
                galley.layout_from_cursor(preedit_range.start + relative_active_range.start.index),
                inactive_underline_stroke,
            );
        }

        paint_underlines(
            pos,
            painter,
            galley,
            galley.layout_from_cursor(preedit_range.start + relative_active_range.start.index),
            galley.layout_from_cursor(preedit_range.start + relative_active_range.end.index),
            active_underline_stroke,
        );

        if !is_cursor_range_empty(
            &(relative_active_range.end..(preedit_range.end - preedit_range.start.index)),
        ) {
            paint_underlines(
                pos,
                painter,
                galley,
                galley.layout_from_cursor(preedit_range.start + relative_active_range.end.index),
                galley.layout_from_cursor(preedit_range.end),
                inactive_underline_stroke,
            );
        }
    } else {
        paint_underlines(
            pos,
            painter,
            galley,
            galley.layout_from_cursor(preedit_range.start),
            galley.layout_from_cursor(preedit_range.end),
            inactive_underline_stroke,
        );
    }

    if let Some(relative_active_range) = relative_active_range
        && is_cursor_range_empty(&relative_active_range)
    {
        let active_cursor = preedit_range.start + relative_active_range.start.index;
        let cursor_rect = cursor_rect(galley, &active_cursor, row_height);

        paint_text_cursor(
            ui,
            painter,
            cursor_rect.translate(pos.to_vec2()),
            time_since_last_interaction,
        );
    }
}

fn paint_underlines(
    pos: Pos2,
    painter: &Painter,
    galley: &Arc<Galley>,
    min: LayoutCursor,
    max: LayoutCursor,
    stroke: Stroke,
) {
    for ri in min.row..=max.row {
        let placed_row = &galley.rows[ri];
        let row = &placed_row.row;

        let offset = pos + placed_row.pos.to_vec2();
        for rect in selection_rects(row, ri, min, max, 0.0) {
            painter.line_segment(
                [
                    offset + rect.left_bottom().to_vec2(),
                    offset + rect.right_bottom().to_vec2(),
                ],
                stroke,
            );
        }
    }
}

/// Paint one end of the selection, e.g. the primary cursor.
///
/// This will never blink.
pub fn paint_cursor_end(painter: &Painter, visuals: &Visuals, cursor_rect: Rect) {
    let stroke = visuals.text_cursor.stroke;

    let top = cursor_rect.center_top();
    let bottom = cursor_rect.center_bottom();

    painter.line_segment([top, bottom], stroke);

    if false {
        // Roof/floor:
        let extrusion = 3.0;
        let width = 1.0;
        painter.line_segment(
            [top - vec2(extrusion, 0.0), top + vec2(extrusion, 0.0)],
            (width, stroke.color),
        );
        painter.line_segment(
            [bottom - vec2(extrusion, 0.0), bottom + vec2(extrusion, 0.0)],
            (width, stroke.color),
        );
    }
}

/// Paint one end of the selection, e.g. the primary cursor, with blinking (if enabled).
pub fn paint_text_cursor(
    ui: &Ui,
    painter: &Painter,
    primary_cursor_rect: Rect,
    time_since_last_interaction: f64,
) {
    if ui.visuals().text_cursor.blink {
        let on_duration = ui.visuals().text_cursor.on_duration;
        let off_duration = ui.visuals().text_cursor.off_duration;
        let total_duration = on_duration + off_duration;

        let time_in_cycle = (time_since_last_interaction % (total_duration as f64)) as f32;

        let wake_in = if time_in_cycle < on_duration {
            // Cursor is visible
            paint_cursor_end(painter, ui.visuals(), primary_cursor_rect);
            on_duration - time_in_cycle
        } else {
            // Cursor is not visible
            total_duration - time_in_cycle
        };

        ui.request_repaint_after_secs(wake_in);
    } else {
        paint_cursor_end(painter, ui.visuals(), primary_cursor_rect);
    }
}
