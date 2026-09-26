//! Inline artwork with the original wire text retained in egui's editing/undo model.
use crate::{avatars::Avatars, emoji};
use egui::{Color32, Image, text::CCursor, text::LayoutJob, text::TextFormat};
use model::User;
use std::{
	hash::{DefaultHasher, Hash, Hasher},
	ops::Range,
	sync::Arc,
};
use unicode_segmentation::UnicodeSegmentation;

struct Inline {
	source: Range<usize>,
	projected: usize,
	width: f32,
	label: Option<Arc<egui::Galley>>,
	background: Color32,
	image: Option<Image<'static>>,
}

#[derive(Default)]
pub(crate) struct Layout {
	inlines: Vec<Inline>,
	cache: Option<(u64, Arc<egui::Galley>)>,
}

impl Layout {
	#[allow(clippy::too_many_arguments)]
	pub fn galley(
		&mut self,
		ui: &egui::Ui,
		text: &str,
		width: f32,
		users: &[User],
		roles: &[model::permissions::Role],
		channels: &[model::Channel],
		mass_mentions: bool,
		avatars: &mut Avatars,
		demo: bool,
	) -> Arc<egui::Galley> {
		let mut key = DefaultHasher::new();
		text.hash(&mut key);
		width.to_bits().hash(&mut key);
		crate::fonts::revision(ui.ctx()).hash(&mut key);
		ui.ctx().pixels_per_point().to_bits().hash(&mut key);
		egui::TextStyle::Body.resolve(ui.style()).hash(&mut key);
		let colors = crate::design::palette(ui);
		colors.text.hash(&mut key);
		colors.accent.hash(&mut key);
		colors.mention_text.hash(&mut key);
		colors.mention_bg.hash(&mut key);
		colors.muted.hash(&mut key);
		avatars.revision.hash(&mut key);
		emoji::ready(ui.ctx()).hash(&mut key);
		demo.hash(&mut key);
		if text.contains("<:") || text.contains("<a:") {
			static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
			(START
				.get_or_init(std::time::Instant::now)
				.elapsed()
				.as_secs() / 60)
				.hash(&mut key);
		}
		for user in users {
			user.id.hash(&mut key);
			user.name.hash(&mut key);
		}
		for role in roles {
			role.id.hash(&mut key);
			role.name.hash(&mut key);
		}
		for (start, _) in text.match_indices("<#") {
			if let Some((id, _)) = model::channel_mention_prefix(&text[start..]) {
				channels
					.iter()
					.find(|c| c.id == id)
					.map(|c| &c.name)
					.hash(&mut key);
			}
		}
		mass_mentions.hash(&mut key);
		let key = key.finish();
		if let Some((cached, galley)) = &self.cache
			&& *cached == key
		{
			return galley.clone();
		}
		self.inlines.clear();
		let font = egui::TextStyle::Body.resolve(ui.style());
		let colors = crate::design::palette(ui);
		let format = TextFormat::simple(font.clone(), colors.text);
		let size = emoji::inline_size(ui);
		let mut job = LayoutJob::default();
		job.wrap.max_width = width;
		if text.is_empty() {
			job.append("", 0.0, format.clone());
		}
		let mut byte = 0;
		let mut source = 0;
		let mut projected = 0;
		while byte < text.len() {
			let tail = &text[byte..];
			let mut label = None;
			let mut background = colors.accent.gamma_multiply(0.12);
			let mut image = None;
			let mut artwork = false;
			let length = if let Some((id, len)) = model::user_mention_prefix(tail) {
				let name = users
					.iter()
					.find(|u| u.id == id)
					.map_or_else(|| id.to_string(), |user| user.name.clone());
				label = Some((format!("@{name}"), colors.mention_text));
				background = colors.mention_bg;
				len
			} else if let Some((id, len)) = model::role_mention_prefix(tail) {
				let name = roles
					.iter()
					.find(|role| role.id == id)
					.map_or_else(|| format!("unknown-role ({id})"), |role| role.name.clone());
				label = Some((format!("@{name}"), colors.mention_text));
				background = colors.mention_bg;
				len
			} else if let Some((id, len)) = model::channel_mention_prefix(tail) {
				let name = channels
					.iter()
					.find(|c| c.id == id && c.guild.is_some())
					.map_or_else(|| format!("unknown-channel ({id})"), |c| c.name.clone());
				label = Some((format!("#{name}"), colors.mention_text));
				background = colors.mention_bg;
				len
			} else if mass_mentions && let Some(len) = model::mass_mention_prefix(tail) {
				label = Some((tail[..len].to_owned(), colors.mention_text));
				background = colors.mention_bg;
				len
			} else if let Some((id, len)) = emoji::custom_prefix(tail) {
				image = avatars.custom_image(ui.ctx(), id, size, demo);
				// A useful name remains visible while artwork is unavailable/loading.
				if image.is_none() {
					let name = tail[..len]
						.trim_start_matches("<a:")
						.trim_start_matches("<:")
						.split(':')
						.next()
						.unwrap_or("emoji");
					label = Some((format!(":{name}:"), colors.muted));
				}
				len
			} else {
				let grapheme = tail.graphemes(true).next().expect("nonempty tail");
				image = emoji::image(ui.ctx(), grapheme, size);
				artwork = image.is_some() || emoji::lookup(grapheme).is_some();
				grapheme.len()
			};
			let label = label.map(|(text, color)| {
				let mut job = LayoutJob::simple_singleline(text, font.clone(), color);
				job.wrap.max_width = (width - 6.0).max(1.0);
				job.wrap.max_rows = 1;
				ui.fonts_mut(|f| f.layout_job(job))
			});
			let raw = &tail[..length];
			let count = raw.chars().count();
			if label.is_some() || image.is_some() || artwork {
				let slot = label.as_ref().map_or(size, |g| g.size().x + 6.0);
				// One blank glyph forms an unbroken inline object, even at a row break.
				// Expand its character slots below, so native selection/copy/undo use wire text.
				job.append(" ", 0.0, emoji::inline_format(ui, slot, size));
				self.inlines.push(Inline {
					source: source..source + count,
					projected,
					width: slot,
					label,
					background,
					image,
				});
				projected += 1;
			} else {
				job.append(raw, 0.0, format.clone());
				projected += count;
			}
			byte += length;
			source += count;
		}
		if self.inlines.is_empty() {
			let galley = ui.fonts_mut(|f| f.layout_job(job));
			self.cache = Some((key, galley.clone()));
			return galley;
		}
		let mut galley = ui.fonts_mut(|f| f.layout_job(job));
		let galley_mut = Arc::make_mut(&mut galley);
		let chars: Vec<_> = text.chars().collect();
		let mut projected = 0;
		let mut next = 0;
		for placed in &mut galley_mut.rows {
			let row = Arc::make_mut(&mut placed.row);
			let mut glyphs = Vec::with_capacity(row.glyphs.len());
			for glyph in &row.glyphs {
				if let Some(inline) = self.inlines.get(next).filter(|i| i.projected == projected) {
					let count = inline.source.len();
					for (index, chr) in chars[inline.source.clone()].iter().enumerate() {
						let mut slot = glyph.clone();
						slot.chr = *chr;
						let visual = if glyph.is_rtl {
							count - index - 1
						} else {
							index
						};
						slot.pos.x = glyph.pos.x + inline.width * visual as f32 / count as f32;
						slot.advance_width = inline.width / count as f32;
						glyphs.push(slot);
					}
					next += 1;
				} else {
					glyphs.push(glyph.clone());
				}
				projected += 1;
			}
			row.glyphs = glyphs;
			projected += usize::from(placed.ends_with_newline);
		}
		// TextEdit's galley and TextBuffer must expose exactly the same original characters.
		galley_mut.job = Arc::new(LayoutJob::simple(text.to_owned(), font, colors.text, width));
		self.cache = Some((key, galley.clone()));
		galley
	}

	pub fn paint(&self, ui: &mut egui::Ui, output: &egui::text_edit::TextEditOutput) {
		// TextEdit keeps the empty hint galley on the first keystroke.
		if self
			.cache
			.as_ref()
			.is_none_or(|(_, galley)| galley.job.text != output.galley.job.text)
		{
			return;
		}
		let painter = ui.painter().with_clip_rect(output.text_clip_rect);
		for inline in &self.inlines {
			let mut cursor = CCursor::new(inline.source.start);
			cursor.prefer_next_row = true;
			let location = output.galley.layout_from_cursor(cursor);
			let row = &output.galley.rows[location.row];
			let glyphs = &row.glyphs[location.column.0..location.column.0 + inline.source.len()];
			let left = glyphs.iter().map(|g| g.pos.x).fold(f32::INFINITY, f32::min);
			let position = egui::Rect::from_min_size(
				output.galley_pos + row.pos.to_vec2() + egui::vec2(left, 0.0),
				egui::vec2(0.0, row.height()),
			);
			let rect = egui::Rect::from_min_size(
				position.min,
				egui::vec2(inline.width, position.height()),
			);
			if !rect.intersects(output.text_clip_rect) {
				continue;
			}
			if let Some(label) = &inline.label {
				painter.rect_filled(rect, 3, inline.background);
				painter.galley(
					egui::pos2(rect.left() + 3.0, rect.center().y - label.size().y / 2.0),
					label.clone(),
					Color32::PLACEHOLDER,
				);
			} else if let Some(image) = &inline.image {
				let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
				child.set_clip_rect(output.text_clip_rect);
				image.paint_at(
					&child,
					egui::Rect::from_center_size(
						rect.center(),
						image.calc_size(egui::Vec2::splat(inline.width), image.size()),
					),
				);
			}
		}
	}

	/// Keep mouse/arrow selection endpoints outside encoded inline objects.
	pub fn snap_cursor(&self, output: &mut egui::text_edit::TextEditOutput, ctx: &egui::Context) {
		let Some(mut range) = output.cursor_range else {
			return;
		};
		let original = range;
		let direction = ctx.input(|i| {
			if i.key_pressed(egui::Key::ArrowLeft) {
				-1
			} else if i.key_pressed(egui::Key::ArrowRight) {
				1
			} else {
				0
			}
		});
		for cursor in [&mut range.primary, &mut range.secondary] {
			for inline in &self.inlines {
				let index = cursor.index.0;
				if inline.source.start < index && index < inline.source.end {
					cursor.index = if direction < 0
						|| (direction == 0
							&& index - inline.source.start < inline.source.end - index)
					{
						inline.source.start.into()
					} else {
						inline.source.end.into()
					};
					break;
				}
			}
		}
		if range != original {
			output.cursor_range = Some(range);
			output.state.cursor.set_char_range(Some(range));
			output.state.clone().store(ctx, output.response.id);
		}
	}

	/// Let the native editor delete a selected token, preserving its normal undo history.
	pub fn select_deleted_inline(&self, ctx: &egui::Context, id: egui::Id) {
		if !ctx.memory(|m| m.has_focus(id)) {
			return;
		}
		let Some(mut state) = egui::text_edit::TextEditState::load(ctx, id) else {
			return;
		};
		let Some(range) = state.cursor.char_range().filter(|r| r.is_empty()) else {
			return;
		};
		let (back, forward) = ctx.input(|i| {
			(
				i.key_pressed(egui::Key::Backspace),
				i.key_pressed(egui::Key::Delete),
			)
		});
		if let Some(inline) = self.inlines.iter().find(|i| {
			(back && i.source.end == range.primary.index.0)
				|| (forward && i.source.start == range.primary.index.0)
		}) {
			state
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::two(
					CCursor::new(inline.source.start),
					CCursor::new(inline.source.end),
				)));
			state.store(ctx, id);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn users() -> Vec<User> {
		vec![User {
			id: model::Id(42),
			name: "Zoë".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		}]
	}

	#[test]
	fn mention_highlights_follow_theme_and_preserve_wire_text() {
		let ctx = egui::Context::default();
		let mut layout = Layout::default();
		let mut avatars = Avatars::default();
		let mut text = "<@42> <@!43> @everyone @here <:missing:9001>".to_owned();
		for dark in [true, false] {
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			for width in [80.0, 300.0] {
				let output = ctx.run_ui(Default::default(), |ui| {
					let colors = crate::design::palette(ui);
					let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _| {
						layout.galley(
							ui,
							buffer.as_str(),
							width,
							&users(),
							&[],
							&[],
							true,
							&mut avatars,
							false,
						)
					};
					let edit = egui::TextEdit::multiline(&mut text)
						.layouter(&mut layouter)
						.show(ui);
					layout.paint(ui, &edit);
					assert_eq!(edit.galley.job.text, text);
					assert_eq!(layout.inlines.len(), 5);
					for (inline, label) in layout.inlines.iter().zip([
						"@Zoë",
						"@43",
						"@everyone",
						"@here",
						":missing:",
					]) {
						let mention = label.starts_with('@');
						let galley = inline.label.as_ref().unwrap();
						assert_eq!(galley.job.text, label);
						assert_eq!(
							galley.job.sections[0].format.color,
							if mention {
								colors.mention_text
							} else {
								colors.muted
							}
						);
						assert_eq!(
							inline.background,
							if mention {
								colors.mention_bg
							} else {
								colors.accent.gamma_multiply(0.12)
							}
						);
					}
				});
				let colors = crate::design::palette_for(&ctx);
				assert_eq!(
					output
						.shapes
						.iter()
						.filter(|s| matches!(&s.shape,
					egui::Shape::Rect(rect) if rect.fill == colors.mention_bg))
						.count(),
					4
				);
				output.drop_without_applying_deltas();
			}
		}
	}

	#[test]
	fn emoji_slots_stay_large_and_never_use_font_artwork_while_loading() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		let mut layout = Layout::default();
		let text = "😀👩🏽‍💻❤️🇨🇿";
		let mut cold_size = None;
		for ready in [false, true] {
			if ready {
				emoji::install(&ctx).unwrap();
			}
			let output = ctx.run_ui(Default::default(), |ui| {
				let galley =
					layout.galley(ui, text, 65.0, &[], &[], &[], false, &mut avatars, true);
				assert_eq!(galley.job.text, text);
				assert_eq!(layout.inlines.len(), 4);
				for inline in &layout.inlines {
					assert_eq!(inline.width, emoji::inline_size(ui));
					assert!(inline.width > egui::TextStyle::Body.resolve(ui.style()).size + 5.0);
					assert_eq!(inline.image.is_some(), ready);
				}
				assert!(galley.rows.iter().all(|row| {
					row.visuals
						.mesh
						.vertices
						.iter()
						.all(|v| v.color == Color32::TRANSPARENT)
				}));
				if let Some(size) = cold_size {
					assert_eq!(galley.size(), size);
				} else {
					cold_size = Some(galley.size());
				}
			});
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn first_emoji_input_waits_for_its_own_galley_then_keeps_artwork() {
		let ctx = egui::Context::default();
		emoji::install(&ctx).unwrap();
		let mut avatars = Avatars::default();
		let mut layout = Layout::default();
		let mut text = String::new();
		let id = egui::Id::unique("emoji-first-input");
		ctx.memory_mut(|m| m.request_focus(id));
		for (frame, events) in [
			vec![egui::Event::Text("😀".into())],
			vec![],
			vec![egui::Event::Text("❤️".into())],
		]
		.into_iter()
		.enumerate()
		{
			let output = ctx.run_ui(
				egui::RawInput {
					events,
					..Default::default()
				},
				|ui| {
					let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width| {
						layout.galley(
							ui,
							buffer.as_str(),
							width,
							&[],
							&[],
							&[],
							false,
							&mut avatars,
							true,
						)
					};
					let edit = egui::TextEdit::multiline(&mut text)
						.id(id)
						.hint_text("Message")
						.layouter(&mut layouter)
						.show(ui);
					layout.paint(ui, &edit);
				},
			);
			let images = output
				.shapes
				.iter()
				.filter(
					|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.brush.is_some()),
				)
				.count();
			assert_eq!(
				images, frame,
				"first input must not paint against the empty hint galley"
			);
			output.drop_without_applying_deltas();
		}
		assert_eq!(text, "😀❤️");
	}

	#[test]
	fn nonsquare_emoji_keep_aspect_in_messages_and_composer() {
		for dimensions in [[64, 32], [32, 64]] {
			let ctx = egui::Context::default();
			let texture = ctx.load_texture(
				"synthetic-emoji",
				egui::ColorImage::filled(dimensions, Color32::WHITE),
				Default::default(),
			);
			let image = Image::new(&texture).fit_to_exact_size(egui::Vec2::splat(24.0));
			let mut layout = Layout::default();
			let mut avatars = Avatars::default();
			let mut text = "<:serein_leaf:9001>".to_owned();
			let message = crate::markdown::Formatted::parse(&text);
			let mut output = ctx.run_ui(Default::default(), |ui| {
				ui.style_mut()
					.text_styles
					.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
				// The timeline resolves custom artwork through the avatar cache.
				avatars.custom_image(ui.ctx(), model::Id(9001), 24.0, false);
				avatars.accept(
					ui.ctx(),
					"emoji-9001".into(),
					Some(egui::ColorImage::filled(dimensions, Color32::WHITE)),
				);
				message.show_with_images(
					ui,
					&mut None,
					&[],
					None,
					&mut crate::profiles::ProfileSession::default(),
					(&mut avatars, false, &[]),
				);
				let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width| {
					layout.galley(
						ui,
						buffer.as_str(),
						width,
						&[],
						&[],
						&[],
						false,
						&mut avatars,
						true,
					)
				};
				let edit = egui::TextEdit::multiline(&mut text)
					.layouter(&mut layouter)
					.show(ui);
				layout.inlines[0].image = Some(image.clone());
				layout.paint(ui, &edit);
			});
			output.textures_delta.clear();
			let sizes: Vec<_> = output
				.shapes
				.iter()
				.filter_map(|shape| match &shape.shape {
					egui::Shape::Rect(rect)
						if rect.fill_texture_id() != egui::TextureId::default() =>
					{
						Some(rect.rect.size())
					}
					_ => None,
				})
				.collect();
			assert_eq!(sizes.len(), 2);
			for size in sizes {
				assert!(
					(size.x / size.y - dimensions[0] as f32 / dimensions[1] as f32).abs() < 0.01
				);
				assert!(size.x <= 24.0 && size.y <= 24.0);
			}
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn artwork_keeps_wire_characters_and_cursor_geometry_across_wrapped_lines() {
		let ctx = egui::Context::default();
		emoji::install(&ctx).unwrap();
		let text = "Hi <@42> 👩🏽‍💻 <:serein_leaf:9001>\nagain <@!42> ❤️";
		let mut avatars = Avatars::default();
		let mut layout = Layout::default();
		let output = ctx.run_ui(Default::default(), |ui| {
			let galley = layout.galley(
				ui,
				text,
				130.0,
				&users(),
				&[],
				&[],
				false,
				&mut avatars,
				true,
			);
			assert_eq!(galley.job.text, text);
			let mut reconstructed = String::new();
			for row in &galley.rows {
				reconstructed.extend(row.glyphs.iter().map(|g| g.chr));
				if row.ends_with_newline {
					reconstructed.push('\n');
				}
				assert!(row.glyphs.iter().all(|g| g.pos.x >= -0.1));
				assert!(row.glyphs.windows(2).all(|g| g[0].pos.x <= g[1].pos.x));
			}
			assert_eq!(reconstructed, text);
			assert_eq!(layout.inlines.len(), 5);
			assert_eq!(
				layout.inlines.iter().filter(|i| i.image.is_some()).count(),
				3
			);
			assert_eq!(layout.inlines[0].label.as_ref().unwrap().job.text, "@Zoë");
			assert_eq!(galley.end().index.0, text.chars().count());
			for inline in &layout.inlines {
				let mut cursor = CCursor::new(inline.source.start);
				cursor.prefer_next_row = true;
				let start = galley.pos_from_cursor(cursor);
				cursor.index = inline.source.end.into();
				cursor.prefer_next_row = false;
				let end = galley.pos_from_cursor(cursor);
				assert!(
					(start.top() - end.top()).abs() < 0.1,
					"inline must not wrap internally"
				);
				assert!((end.left() - start.left() - inline.width).abs() < 1.0);
			}
		});
		output.drop_without_applying_deltas();
		assert!(avatars.take_requests().is_empty());
	}

	#[test]
	fn native_delete_undo_copy_and_ime_keep_wire_mentions() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		let mut layout = Layout::default();
		let mut text = "hi <@42>!".to_owned();
		let id = egui::Id::unique("rich-editor-test");
		let mut frame = |time, events: Vec<egui::Event>, cursor: Option<usize>| {
			if let Some(cursor) = cursor {
				ctx.memory_mut(|m| m.request_focus(id));
				let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap_or_default();
				state
					.cursor
					.set_char_range(Some(egui::text::CCursorRange::one(CCursor::new(cursor))));
				state.store(&ctx, id);
			}
			let copying = events
				.iter()
				.any(|event| matches!(event, egui::Event::Copy));
			let composing = events
				.iter()
				.any(|event| matches!(event, egui::Event::Ime(_)));
			let output = ctx.run_ui(
				egui::RawInput {
					time: Some(time),
					events,
					..Default::default()
				},
				|ui| {
					layout.galley(
						ui,
						&text,
						300.0,
						&users(),
						&[],
						&[],
						false,
						&mut avatars,
						true,
					);
					layout.select_deleted_inline(&ctx, id);
					let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width| {
						layout.galley(
							ui,
							buffer.as_str(),
							width,
							&users(),
							&[],
							&[],
							false,
							&mut avatars,
							true,
						)
					};
					let mut edit = egui::TextEdit::multiline(&mut text)
						.id(id)
						.layouter(&mut layouter)
						.show(ui);
					if !composing {
						layout.snap_cursor(&mut edit, &ctx);
					}
					layout.paint(ui, &edit);
				},
			);
			if copying {
				assert!(output.platform_output.commands.iter().any(|command| {
					matches!(command, egui::OutputCommand::CopyText(copied) if copied == &text)
				}));
			}
			output.drop_without_applying_deltas();
			text.clone()
		};
		let key = |key, modifiers| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers,
		};
		assert_eq!(frame(0.0, vec![], Some(8)), "hi <@42>!");
		assert_eq!(
			frame(
				1.0,
				vec![key(egui::Key::Backspace, egui::Modifiers::NONE)],
				None
			),
			"hi !"
		);
		assert_eq!(
			frame(2.0, vec![key(egui::Key::Z, egui::Modifiers::COMMAND)], None),
			"hi <@42>!"
		);
		frame(
			2.1,
			vec![key(egui::Key::ArrowLeft, egui::Modifiers::NONE)],
			Some(8),
		);
		assert_eq!(
			egui::text_edit::TextEditState::load(&ctx, id)
				.unwrap()
				.cursor
				.char_range()
				.unwrap()
				.primary
				.index
				.0,
			3
		);
		frame(
			2.2,
			vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)],
			None,
		);
		assert_eq!(
			egui::text_edit::TextEditState::load(&ctx, id)
				.unwrap()
				.cursor
				.char_range()
				.unwrap()
				.primary
				.index
				.0,
			8
		);
		assert_eq!(
			frame(3.0, vec![egui::Event::Text("č".into())], Some(8)),
			"hi <@42>č!"
		);
		assert_eq!(
			frame(
				4.0,
				vec![egui::Event::Ime(egui::ImeEvent::Preedit {
					text: "ni".into(),
					active_range_chars: None,
				})],
				Some(9)
			),
			"hi <@42>čni!"
		);
		assert_eq!(
			frame(
				5.0,
				vec![egui::Event::Ime(egui::ImeEvent::Commit("你".into()))],
				None
			),
			"hi <@42>č你!"
		);
		assert_eq!(
			frame(
				6.0,
				vec![
					key(egui::Key::A, egui::Modifiers::COMMAND),
					egui::Event::Copy
				],
				None
			),
			"hi <@42>č你!"
		);
	}
}

#[cfg(all(feature = "demo", debug_assertions))]
pub fn debug_arabic_check() {
	let ctx = egui::Context::default();
	crate::fonts::install(&ctx);
	crate::design::apply(&ctx);
	let mut layout = Layout::default();
	let mut avatars = Avatars::default();
	const ARABIC: &str = "هذا الخط معكوس";
	ctx.run_ui(Default::default(), |ui| {
		for source in [
			ARABIC,
			"مرحبا English 123!",
			"English مرحبا 123!",
			"مَرْحَبًا لا لأ",
			"مرحبا 😀 <@42> world",
			"مرحبا\nEnglish 123\nالعربية",
		] {
			for width in [500.0, 85.0] {
				let galley =
					layout.galley(ui, source, width, &[], &[], &[], false, &mut avatars, true);
				assert_eq!(galley.job.text, source);
				let text: String = galley
					.rows
					.iter()
					.flat_map(|row| {
						row.glyphs
							.iter()
							.map(|g| g.chr)
							.chain(row.ends_with_newline.then_some('\n'))
					})
					.collect();
				assert_eq!(text, source, "one source scalar per editing slot");
				assert_eq!(galley.end().index.0, source.chars().count());
				for row in &galley.rows {
					assert!(
						row.glyphs
							.iter()
							.all(|g| g.pos.x.is_finite() && g.advance_width >= 0.0)
					);
				}
				if source == ARABIC && width == 500.0 {
					let row = &galley.rows[0];
					let mut visual = row.glyphs.clone();
					visual.sort_by(|a, b| a.pos.x.total_cmp(&b.pos.x));
					assert_eq!(
						visual.iter().map(|g| g.chr).collect::<String>(),
						"سوكعم طخلا اذه"
					);
					let start = galley.pos_from_cursor(CCursor::new(0)).center();
					let end = galley.pos_from_cursor(galley.end()).center();
					assert!(start.x > end.x);
					assert_eq!(galley.cursor_from_pos(start.to_vec2()).index.0, 0);
					assert_eq!(
						galley.cursor_from_pos(end.to_vec2()).index.0,
						ARABIC.chars().count()
					);
					assert_eq!(
						galley.cursor_left_one_character(&CCursor::new(0)).index.0,
						1
					);
				}
				let mut selected = galley.clone();
				egui::text_selection::visuals::paint_text_selection(
					&mut selected,
					ui.visuals(),
					&egui::text::CCursorRange::two(CCursor::new(0), galley.end()),
					None,
				);
				assert!(selected.rows.iter().all(|row| row.visuals.mesh.is_valid()));
			}
		}
	})
	.drop_without_applying_deltas();

	ctx.run_ui(Default::default(), |ui| {
		let joined = layout.galley(ui, "بَ", 1.0, &[], &[], &[], false, &mut avatars, true);
		assert_eq!(
			joined.rows.len(),
			1,
			"wrapping must not split a base/mark cluster"
		);
		let mixed = layout.galley(
			ui,
			"abc ابج def",
			500.0,
			&[],
			&[],
			&[],
			false,
			&mut avatars,
			true,
		);
		let row = &mixed.rows[0];
		for (i, glyph) in row.glyphs.iter().enumerate() {
			if glyph.advance_width <= 0.0 {
				continue;
			}
			for fraction in [0.1, 0.9] {
				let point = egui::vec2(
					row.pos.x + glyph.pos.x + fraction * glyph.advance_width,
					row.pos.y + row.height() / 2.0,
				);
				let cursor = mixed.cursor_from_pos(point);
				let leading = (fraction < 0.5) != glyph.is_rtl;
				assert_eq!(
					cursor.index.0,
					i + usize::from(!leading),
					"mixed-direction glyph hit"
				);
				let expected_x = if fraction < 0.5 {
					glyph.pos.x
				} else {
					glyph.max_x()
				};
				// Kerning and pixel snapping can overlap adjacent caret cells by a fraction of a pixel.
				assert!(
					(mixed.pos_from_cursor(cursor).center().x - row.pos.x - expected_x).abs()
						< 1.0,
					"caret retains boundary affinity: char {i} {:?}, fraction {fraction}, cursor {cursor:?}, actual {}, expected {expected_x}", glyph.chr, mixed.pos_from_cursor(cursor).center().x - row.pos.x
				);
			}
		}
		let single = ui.fonts_mut(|fonts| {
			fonts.layout_job(LayoutJob::simple_singleline(
				"مرحبا\nالعربية".into(),
				egui::FontId::proportional(15.0),
				Color32::WHITE,
			))
		});
		assert_eq!(single.rows.len(), 1);
		assert!(single.rows[0].glyphs[0].pos.x > single.rows[0].glyphs[4].pos.x);
	})
	.drop_without_applying_deltas();
	let painted = ctx.run_ui(Default::default(), |ui| {
		crate::markdown::Formatted::parse(ARABIC).show_with_images(
			ui,
			&mut None,
			&[],
			None,
			&mut crate::profiles::ProfileSession::default(),
			(&mut avatars, true, &[]),
		)
	});
	assert!(painted.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == ARABIC && text.galley.rows[0].glyphs[0].is_rtl)), "message renderer retains logical Arabic source");
	painted.drop_without_applying_deltas();

	let id = egui::Id::unique("arabic-debug-edit");
	let mut text = String::new();
	ctx.memory_mut(|m| m.request_focus(id));
	for events in [
		vec![egui::Event::Text(ARABIC.into())],
		vec![egui::Event::Copy],
		vec![egui::Event::Text("!".into())],
	] {
		let copying = events.iter().any(|e| matches!(e, egui::Event::Copy));
		if copying {
			let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
			state
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::two(
					CCursor::new(0),
					CCursor::new(text.chars().count()),
				)));
			state.store(&ctx, id);
		}
		let output = ctx.run_ui(
			egui::RawInput {
				events,
				..Default::default()
			},
			|ui| {
				let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width| {
					layout.galley(
						ui,
						buffer.as_str(),
						width,
						&[],
						&[],
						&[],
						false,
						&mut avatars,
						true,
					)
				};
				egui::TextEdit::multiline(&mut text)
					.id(id)
					.layouter(&mut layouter)
					.show(ui);
			},
		);
		if copying {
			assert!(
				output
					.platform_output
					.commands
					.iter()
					.any(|c| matches!(c, egui::OutputCommand::CopyText(s) if s == ARABIC))
			);
		}
		output.drop_without_applying_deltas();
	}
	assert_eq!(
		text, "!",
		"selection replacement uses logical source indices"
	);
	println!(
		"Arabic debug check passed: visual order, wrapping, marks, mixed text, inline objects, cursor hit testing, copy and editing."
	);
}
