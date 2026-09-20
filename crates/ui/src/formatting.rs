//! Composer keyboard formatting: Cmd/Ctrl shortcuts wrap or unwrap the selection with markdown.
use egui::text::{CCursor, CCursorRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
	Bold,
	Italic,
	Underline,
	Strikethrough,
	Code,
	CodeBlock,
	Spoiler,
}

impl Style {
	fn markers(self) -> (&'static str, &'static str) {
		match self {
			Self::Bold => ("**", "**"),
			Self::Italic => ("*", "*"),
			Self::Underline => ("__", "__"),
			Self::Strikethrough => ("~~", "~~"),
			Self::Code => ("`", "`"),
			Self::CodeBlock => ("```\n", "\n```"),
			Self::Spoiler => ("||", "||"),
		}
	}

	/// Consumes the first matching shortcut this frame. Shift combinations are checked first
	/// because a plain `Cmd+letter` match would otherwise also swallow them.
	pub fn consume(ctx: &egui::Context, bindings: &model::Keybinds) -> Option<Self> {
		const SHORTCUTS: [(model::KeybindAction, Style); 7] = [
			(model::KeybindAction::Strikethrough, Style::Strikethrough),
			(model::KeybindAction::CodeBlock, Style::CodeBlock),
			(model::KeybindAction::Spoiler, Style::Spoiler),
			(model::KeybindAction::Bold, Style::Bold),
			(model::KeybindAction::Italic, Style::Italic),
			(model::KeybindAction::Underline, Style::Underline),
			(model::KeybindAction::InlineCode, Style::Code),
		];
		ctx.input_mut(|input| {
			SHORTCUTS
				.iter()
				.find(|(action, _)| crate::keybinds::pressed(input, bindings.chord(*action)))
				.map(|(_, style)| *style)
		})
	}
}

fn char_to_byte(text: &str, index: usize) -> usize {
	text.char_indices()
		.nth(index)
		.map_or(text.len(), |(byte, _)| byte)
}

/// Wraps the selection (or inserts an empty pair at the caret) and returns the new selection.
/// A selection that is already wrapped, or sits exactly inside the markers, is unwrapped instead.
pub fn apply(
	draft: &mut String,
	style: Style,
	range: Option<CCursorRange>,
	remaining: usize,
) -> Option<CCursorRange> {
	let (prefix, suffix) = style.markers();
	let count = draft.chars().count();
	let (start, end) = range.map_or((count, count), |range| {
		let range = range.as_sorted_char_range();
		(range.start.0.min(count), range.end.0.min(count))
	});
	let (prefix_len, suffix_len) = (prefix.chars().count(), suffix.chars().count());
	let selected: String = draft.chars().skip(start).take(end - start).collect();

	// Toggle off: `**text**` selected, or `text` selected with the markers just outside.
	if let Some(inner) = selected
		.strip_prefix(prefix)
		.and_then(|rest| rest.strip_suffix(suffix))
		&& !selected.is_empty()
	{
		let inner = inner.to_owned();
		let (from, to) = (char_to_byte(draft, start), char_to_byte(draft, end));
		draft.replace_range(from..to, &inner);
		return Some(CCursorRange::two(
			CCursor::new(start),
			CCursor::new(start + inner.chars().count()),
		));
	}
	if start >= prefix_len && end + suffix_len <= count {
		let before: String = draft
			.chars()
			.skip(start - prefix_len)
			.take(prefix_len)
			.collect();
		let after: String = draft.chars().skip(end).take(suffix_len).collect();
		if before == prefix && after == suffix {
			let from = char_to_byte(draft, start - prefix_len);
			let to = char_to_byte(draft, end + suffix_len);
			draft.replace_range(from..to, &selected);
			return Some(CCursorRange::two(
				CCursor::new(start - prefix_len),
				CCursor::new(end - prefix_len),
			));
		}
	}

	let wrapped = format!("{prefix}{selected}{suffix}");
	let range = Some(CCursorRange::two(CCursor::new(start), CCursor::new(end)));
	crate::emoji_picker::insert(draft, &wrapped, range, remaining)?;
	let inner_start = start + prefix_len;
	Some(CCursorRange::two(
		CCursor::new(inner_start),
		CCursor::new(inner_start + (end - start)),
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn selection(start: usize, end: usize) -> Option<CCursorRange> {
		Some(CCursorRange::two(CCursor::new(start), CCursor::new(end)))
	}

	#[test]
	fn wrapping_toggles_and_keeps_the_inner_selection() {
		let mut draft = "say hi there".to_owned();
		let range = apply(&mut draft, Style::Bold, selection(4, 6), 100).unwrap();
		assert_eq!(draft, "say **hi** there");
		assert_eq!(range.as_sorted_char_range().start.0, 6);
		assert_eq!(range.as_sorted_char_range().end.0, 8);
		let range = apply(&mut draft, Style::Bold, Some(range), 100).unwrap();
		assert_eq!(draft, "say hi there");
		assert_eq!(range.as_sorted_char_range().start.0, 4);
		assert_eq!(range.as_sorted_char_range().end.0, 6);
		let mut draft = "**hi**".to_owned();
		assert!(apply(&mut draft, Style::Bold, selection(0, 6), 100).is_some());
		assert_eq!(draft, "hi");
	}

	#[test]
	fn empty_caret_inserts_markers_and_code_blocks_span_lines() {
		let mut draft = "ab".to_owned();
		let range = apply(&mut draft, Style::Italic, selection(1, 1), 100).unwrap();
		assert_eq!(draft, "a**b");
		assert!(range.is_empty());
		assert_eq!(range.primary.index.0, 2);
		let mut draft = "console.log(\"hi\")".to_owned();
		let end = draft.chars().count();
		let range = apply(&mut draft, Style::CodeBlock, selection(0, end), 100).unwrap();
		assert_eq!(draft, "```\nconsole.log(\"hi\")\n```");
		assert_eq!(range.as_sorted_char_range().start.0, 4);
		assert_eq!(range.as_sorted_char_range().end.0, 4 + end);
	}

	#[test]
	fn budget_overflow_leaves_the_draft_unchanged() {
		let mut draft = "x".to_owned();
		draft.shrink_to_fit();
		assert!(apply(&mut draft, Style::Bold, selection(0, 1), 0).is_none());
		assert_eq!(draft, "x");
	}
}
