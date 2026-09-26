//! Keep source indices logical; only shaped glyph positions follow visual order.
use super::*;

pub(super) fn layout_rtl_run(
    fonts: &mut FontsImpl,
    run: &TextRun,
    text: &str,
    buffer: &harfrust::GlyphBuffer,
    metrics: &StyledMetrics,
    ctx: &mut ShapingContext,
    paragraph: &mut Paragraph,
) {
    let infos = buffer.glyph_infos();
    let positions = buffer.glyph_positions();
    let mut end = infos.len();
    // HarfRust returns RTL clusters in descending source order. Walk clusters backwards,
    // retaining the shaped glyph order *within* each cluster (base, marks, ligatures).
    while end > 0 {
        let byte_start = infos[end - 1].cluster as usize;
        let mut start = end - 1;
        while start > 0 && infos[start - 1].cluster as usize == byte_start {
            start -= 1;
        }
        let next_byte = if start > 0 {
            infos[start - 1].cluster as usize
        } else {
            text.len()
        };
        let cluster = &text[byte_start..next_byte];
        let count = cluster.chars().count();
        let width: f32 = positions[start..end]
            .iter()
            .map(|p| p.x_advance as f32 * metrics.px_scale_factor)
            .sum();
        if !ctx.is_first_glyph_in_section {
            paragraph.cursor_x_px += ctx.extra_letter_spacing * ctx.pixels_per_point;
        }
        ctx.is_first_glyph_in_section = false;
        let cell = width / count as f32;
        let mut visual_x = 0.0;
        let first_glyph = paragraph.glyphs.len();
        for (i, chr) in cluster.chars().enumerate() {
            let mut allocation = GlyphAllocation::default();
            if start + i < end {
                let info = &infos[start + i];
                let pos = &positions[start + i];
                let visual_cell = width - (i + 1) as f32 * cell;
                let OutlineGlyph {
                    allocation: mut alloc,
                    x_px,
                } = fonts.allocate_glyph(
                    run.font_key,
                    metrics,
                    &ShapedGlyph {
                        glyph_id: skrifa::GlyphId::new(info.glyph_id),
                        h_pos: visual_x + pos.x_offset as f32 * metrics.px_scale_factor,
                        is_cjk: false,
                    },
                );
                alloc.uv_rect.offset.x += (x_px as f32 - visual_cell) / ctx.pixels_per_point;
                alloc.uv_rect.offset.y -=
                    pos.y_offset as f32 * metrics.px_scale_factor / ctx.pixels_per_point;
                allocation = alloc;
                visual_x += pos.x_advance as f32 * metrics.px_scale_factor;
            }
            let mut glyph = ctx.glyph(chr, 0, cell, metrics, allocation);
            glyph.pos.x = (paragraph.cursor_x_px + i as f32 * cell) / ctx.pixels_per_point;
            glyph.is_rtl = true;
            glyph.cluster_start = i == 0;
            paragraph.glyphs.push(glyph);
        }
        // A font may expand one scalar into several glyphs. Keep their artwork on the
        // first logical slot instead of inventing source characters or dropping marks.
        if end - start > count {
            let mut extra = Vec::with_capacity(end - start - count);
            for i in start + count..end {
                let pos = &positions[i];
                let OutlineGlyph {
                    allocation: mut alloc,
                    x_px,
                } = fonts.allocate_glyph(
                    run.font_key,
                    metrics,
                    &ShapedGlyph {
                        glyph_id: skrifa::GlyphId::new(infos[i].glyph_id),
                        h_pos: visual_x + pos.x_offset as f32 * metrics.px_scale_factor,
                        is_cjk: false,
                    },
                );
                alloc.uv_rect.offset.x += (x_px as f32 - (width - cell)) / ctx.pixels_per_point;
                alloc.uv_rect.offset.y -=
                    pos.y_offset as f32 * metrics.px_scale_factor / ctx.pixels_per_point;
                extra.push(alloc.uv_rect);
                visual_x += pos.x_advance as f32 * metrics.px_scale_factor;
            }
            paragraph.glyphs[first_glyph].extra_uv_rects = Some(extra.into());
        }
        paragraph.cursor_x_px += width;
        end = start;
    }
}

pub(super) fn reorder_rows(bidi: &unicode_bidi::BidiInfo<'_>, rows: &mut [PlacedRow]) {
    let mut offset = 0;
    for placed in rows {
        let row = Arc::make_mut(&mut placed.row);
        let end = offset
            + bidi.text[offset..]
                .char_indices()
                .nth(row.glyphs.len())
                .map_or(bidi.text.len() - offset, |(i, _)| i);
        let mut column = 0;
        for paragraph in bidi
            .paragraphs
            .iter()
            .filter(|p| p.range.start < end && offset < p.range.end)
        {
            let from = offset.max(paragraph.range.start);
            let to = end.min(paragraph.range.end);
            let resolved = bidi.reordered_levels(paragraph, from..to);
            let levels: Vec<_> = bidi.text[from..to]
                .char_indices()
                .map(|(i, _)| resolved[from + i])
                .collect();
            let glyphs = &mut row.glyphs[column..column + levels.len()];
            if levels.iter().any(unicode_bidi::Level::is_rtl) {
                let widths: Vec<_> = glyphs
                    .iter()
                    .enumerate()
                    .map(|(i, g)| {
                        glyphs
                            .get(i + 1)
                            .map_or(g.advance_width, |next| next.pos.x - g.pos.x)
                    })
                    .collect();
                let mut x = glyphs.first().map_or(0.0, |g| g.pos.x);
                for i in unicode_bidi::BidiInfo::reorder_visual(&levels) {
                    glyphs[i].pos.x = x;
                    glyphs[i].is_rtl = levels[i].is_rtl();
                    x += widths[i];
                }
            }
            column += levels.len();
        }
        offset = end + usize::from(placed.ends_with_newline);
    }
}

pub(super) fn visual_glyphs(row: &Row) -> impl Iterator<Item = &Glyph> {
    let sorted = row.glyphs.iter().any(|g| g.is_rtl).then(|| {
        let mut glyphs: Vec<_> = row.glyphs.iter().collect();
        glyphs.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
        glyphs
    });
    let mut sorted = sorted.map(Vec::into_iter);
    let mut logical = row.glyphs.iter();
    std::iter::from_fn(move || match &mut sorted {
        Some(sorted) => sorted.next(),
        None => logical.next(),
    })
}
