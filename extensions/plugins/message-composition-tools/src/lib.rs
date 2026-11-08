/*
 * Vencord, a Discord client mod
 * Copyright (c) 2024 Vendicated and contributors
 * Copyright (c) 2025 Vendicated and contributors
 * Copyright (c) 2026 Serein port contributors
 *
 * Portions adapted from Equicord's SplitLargeMessages (Reycko, justjxke),
 * BlockKeywords (catcraft, secp192k1) and MessageBurst (port22exposed) at
 * commit 200dba124066146a8f043fde7fc8b05e3dc5e68e.
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License version 3 or (at your
 * option) any later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */

use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	AppAction, AppInvocation, AppOutput, Element, HostEffect, Invocation, MAX_IO_BYTES,
	MessageDetailsSnapshot, Output,
};

/// The host's normal message limit, in Unicode scalar values.
const MAX_SEGMENT_CHARS: usize = 2000;
/// Bounded draft ceiling used by the existing composer tools.
const MAX_DRAFT_BYTES: usize = 8000;
const MIN_SPLIT_LIMIT: usize = 100;
const MAX_KEYWORDS: usize = 32;
const MAX_KEYWORD_BYTES: usize = 64;
const MAX_KEYWORDS_BYTES: usize = 2048;
const MIN_BURST_SEGMENTS: usize = 2;
const MAX_BURST_SEGMENTS: usize = 16;
const MAX_STORAGE_BYTES: usize = 16 * 1024;
const MAX_PANEL_SEGMENTS: usize = 12;
const MAX_PANEL_TEXT_BYTES: usize = 4000;
/// A recent-message merge must stay inside the ordinary message limits.
const MAX_MERGED_CHARS: usize = MAX_SEGMENT_CHARS;
const MAX_MERGED_BYTES: usize = MAX_DRAFT_BYTES;
const MERGE_MISSING_APP: &str =
	"Serein did not provide the current account and conversation, so no merge was proposed.";
const MERGE_MISSING_TIMELINE: &str =
	"Serein did not provide a loaded timeline for this conversation, so no merge was proposed.";
const MERGE_STALE_CONTEXT: &str = "The loaded timeline no longer belongs to the selected conversation. Reopen the tool and try again.";
const CORRUPT_STORAGE: &str =
	"Saved settings are invalid or too large. Nothing was changed; original storage was preserved.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SplitMode {
	#[default]
	Newlines,
	Spaces,
	Characters,
}

impl SplitMode {
	const ALL: [Self; 3] = [Self::Newlines, Self::Spaces, Self::Characters];

	fn label(self) -> &'static str {
		match self {
			Self::Newlines => "Newlines",
			Self::Spaces => "Spaces",
			Self::Characters => "Characters",
		}
	}

	fn from_panel(value: &str) -> Option<Self> {
		let value = value.trim();
		Self::ALL
			.into_iter()
			.find(|mode| mode.label().eq_ignore_ascii_case(value))
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BurstStyle {
	#[default]
	Lines,
	Paragraphs,
	Sentences,
}

impl BurstStyle {
	const ALL: [Self; 3] = [Self::Lines, Self::Paragraphs, Self::Sentences];

	fn label(self) -> &'static str {
		match self {
			Self::Lines => "Lines",
			Self::Paragraphs => "Paragraphs",
			Self::Sentences => "Sentences",
		}
	}

	fn from_panel(value: &str) -> Option<Self> {
		let value = value.trim();
		Self::ALL
			.into_iter()
			.find(|style| style.label().eq_ignore_ascii_case(value))
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Saved {
	split_mode: SplitMode,
	split_limit: usize,
	keywords: Vec<String>,
	keyword_case_sensitive: bool,
	burst_style: BurstStyle,
	burst_max_segments: usize,
	merge_with_attachment: bool,
	merge_use_space: bool,
}

impl Default for Saved {
	fn default() -> Self {
		Self {
			split_mode: SplitMode::Newlines,
			split_limit: MAX_SEGMENT_CHARS,
			keywords: Vec::new(),
			keyword_case_sensitive: false,
			burst_style: BurstStyle::Lines,
			burst_max_segments: 8,
			merge_with_attachment: false,
			merge_use_space: false,
		}
	}
}

#[derive(Clone, Debug)]
struct Fields {
	keywords: String,
	case_sensitive: bool,
	split_mode: SplitMode,
	split_limit: usize,
	burst_style: BurstStyle,
	burst_max_segments: usize,
	merge_with_attachment: bool,
	merge_use_space: bool,
}

impl Fields {
	fn from_saved(saved: &Saved) -> Self {
		Self {
			keywords: saved.keywords.join(", "),
			case_sensitive: saved.keyword_case_sensitive,
			split_mode: saved.split_mode,
			split_limit: saved.split_limit,
			burst_style: saved.burst_style,
			burst_max_segments: saved.burst_max_segments,
			merge_with_attachment: saved.merge_with_attachment,
			merge_use_space: saved.merge_use_space,
		}
	}

	fn from_input(input: &Invocation, saved: &Saved) -> Result<Self, &'static str> {
		let mut fields = Self::from_saved(saved);
		if let Some(value) = input.value("keywords") {
			fields.keywords = value.to_owned();
		}
		if let Some(value) = input.value("case-sensitive") {
			fields.case_sensitive = match value {
				"true" => true,
				"false" => false,
				_ => return Err("The case-sensitivity value is invalid. Nothing was saved."),
			};
		}
		if let Some(value) = input.value("split-mode") {
			fields.split_mode = SplitMode::from_panel(value)
				.ok_or("The split boundary value is invalid. Nothing was saved.")?;
		}
		if let Some(value) = input.value("split-limit") {
			let limit = value
				.parse::<usize>()
				.map_err(|_| "The split limit must be a whole number. Nothing was saved.")?;
			if !(MIN_SPLIT_LIMIT..=MAX_SEGMENT_CHARS).contains(&limit) {
				return Err(
					"The split limit must be between 100 and 2,000 characters. Nothing was saved.",
				);
			}
			fields.split_limit = limit;
		}
		if let Some(value) = input.value("burst-style") {
			fields.burst_style = BurstStyle::from_panel(value)
				.ok_or("The burst style value is invalid. Nothing was saved.")?;
		}
		if let Some(value) = input.value("burst-max-segments") {
			let segments = value.parse::<usize>().map_err(
				|_| "The burst segment count must be a whole number. Nothing was saved.",
			)?;
			if !(MIN_BURST_SEGMENTS..=MAX_BURST_SEGMENTS).contains(&segments) {
				return Err("The burst segment count must be between 2 and 16. Nothing was saved.");
			}
			fields.burst_max_segments = segments;
		}
		if let Some(value) = input.value("merge-attachment") {
			fields.merge_with_attachment = match value {
				"true" => true,
				"false" => false,
				_ => return Err("The attachment-merge value is invalid. Nothing was saved."),
			};
		}
		if let Some(value) = input.value("merge-space") {
			fields.merge_use_space = match value {
				"true" => true,
				"false" => false,
				_ => return Err("The merge-separator value is invalid. Nothing was saved."),
			};
		}
		Ok(fields)
	}
}

fn parse_keywords(raw: &str) -> Result<Vec<String>, &'static str> {
	let mut keywords: Vec<String> = Vec::new();
	for item in raw.split([',', '\n']) {
		let keyword = item.trim();
		if keyword.is_empty() {
			continue;
		}
		if keyword.len() > MAX_KEYWORD_BYTES {
			return Err("Each keyword must be at most 64 UTF-8 bytes. Nothing was saved.");
		}
		if keyword.chars().any(char::is_control) {
			return Err("Keywords cannot contain control characters. Nothing was saved.");
		}
		if !keywords.iter().any(|existing| existing.as_str() == keyword) {
			keywords.push(keyword.to_owned());
		}
	}
	if keywords.len() > MAX_KEYWORDS {
		return Err("At most 32 keywords are supported. Nothing was saved.");
	}
	if keywords.iter().map(String::len).sum::<usize>() > MAX_KEYWORDS_BYTES {
		return Err("The keyword list must be at most 2,048 UTF-8 bytes. Nothing was saved.");
	}
	Ok(keywords)
}

fn validate_saved(saved: &Saved) -> Result<(), &'static str> {
	if !(MIN_SPLIT_LIMIT..=MAX_SEGMENT_CHARS).contains(&saved.split_limit)
		|| !(MIN_BURST_SEGMENTS..=MAX_BURST_SEGMENTS).contains(&saved.burst_max_segments)
		|| saved.keywords.len() > MAX_KEYWORDS
		|| saved.keywords.iter().map(String::len).sum::<usize>() > MAX_KEYWORDS_BYTES
	{
		return Err(CORRUPT_STORAGE);
	}
	for keyword in &saved.keywords {
		if keyword.is_empty()
			|| keyword.len() > MAX_KEYWORD_BYTES
			|| keyword.chars().any(char::is_control)
		{
			return Err(CORRUPT_STORAGE);
		}
	}
	Ok(())
}

fn load_saved(input: &Invocation) -> Result<Saved, &'static str> {
	let Some(raw) = input.storage.as_deref() else {
		return Ok(Saved::default());
	};
	if raw.len() > MAX_STORAGE_BYTES {
		return Err(CORRUPT_STORAGE);
	}
	let saved: Saved = serde_json::from_str(raw).map_err(|_| CORRUPT_STORAGE)?;
	validate_saved(&saved)?;
	Ok(saved)
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: impl Into<String>) -> Element {
	Element::Heading { text: value.into() }
}

fn button(id: &str, label: &str) -> Element {
	Element::Button {
		id: id.into(),
		label: label.into(),
	}
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
	if value.len() <= max_bytes {
		return value;
	}
	let mut boundary = max_bytes;
	while !value.is_char_boundary(boundary) {
		boundary -= 1;
	}
	&value[..boundary]
}

fn panel(saved: &Saved, fields: &Fields, notice: &str) -> Output {
	let mut elements = vec![
		heading("Message Composition Tools"),
		text(
			"Local draft tools. Split and burst show a bounded review panel and never send messages. Keyword filtering changes the draft only after Apply.",
		),
	];
	if !notice.is_empty() {
		elements.push(text(notice));
	}
	elements.extend([
		heading("Split"),
		Element::Select {
			id: "split-mode".into(),
			label: "Preferred boundary".into(),
			options: SplitMode::ALL
				.iter()
				.map(|mode| mode.label().into())
				.collect(),
			value: fields.split_mode.label().into(),
		},
		Element::Slider {
			id: "split-limit".into(),
			label: "Characters per segment".into(),
			min: MIN_SPLIT_LIMIT as i32,
			max: MAX_SEGMENT_CHARS as i32,
			value: fields.split_limit as i32,
		},
		text(
			"The split action displays up to 12 Unicode-safe segments for review. It does not send or schedule messages.",
		),
		heading("Keyword filter"),
		Element::TextInput {
			id: "keywords".into(),
			label: "Comma-separated literal keywords".into(),
			value: truncate_utf8(&fields.keywords, MAX_KEYWORDS_BYTES).into(),
		},
		Element::Checkbox {
			id: "case-sensitive".into(),
			label: "Match keywords case-sensitively".into(),
			checked: fields.case_sensitive,
		},
		text(
			"Filtering removes whole lines that contain a keyword. It is literal matching, not regular expressions, and never blocks incoming messages.",
		),
		heading("Burst review"),
		Element::Select {
			id: "burst-style".into(),
			label: "Segment style".into(),
			options: BurstStyle::ALL
				.iter()
				.map(|style| style.label().into())
				.collect(),
			value: fields.burst_style.label().into(),
		},
		Element::Slider {
			id: "burst-max-segments".into(),
			label: "Maximum burst segments".into(),
			min: MIN_BURST_SEGMENTS as i32,
			max: MAX_BURST_SEGMENTS as i32,
			value: fields.burst_max_segments as i32,
		},
		text(
			"Burst review disperses a draft into candidate messages; it never sends. The burst-merge composer action separately proposes editing your latest loaded own message, and only after Apply.",
		),
		heading("Burst merge"),
		Element::Checkbox {
			id: "merge-attachment".into(),
			label: "Allow merging onto a message with attachments".into(),
			checked: fields.merge_with_attachment,
		},
		Element::Checkbox {
			id: "merge-space".into(),
			label: "Join merged text with a space instead of a new line".into(),
			checked: fields.merge_use_space,
		},
		text(
			"Merging is an explicit foreground proposal. Serein has no plugin clock or send hook, so the upstream time window and reply-bar check are not simulated.",
		),
		Element::Row {
			children: vec![button("save", "Save configuration")],
		},
	]);
	if saved.keywords.is_empty() {
		elements.push(text("No keywords are configured."));
	} else {
		elements.push(text(format!(
			"{} keyword(s) configured.",
			saved.keywords.len()
		)));
	}
	Output {
		panel: elements,
		..Default::default()
	}
}

fn save_saved(saved: &Saved, fields: &Fields, notice: &str) -> Output {
	let encoded = match serde_json::to_string(saved) {
		Ok(encoded) => encoded,
		Err(_) => {
			return panel(
				saved,
				fields,
				"Could not encode settings. Nothing was saved.",
			);
		}
	};
	if encoded.len() > MAX_STORAGE_BYTES {
		return panel(
			saved,
			fields,
			"The settings exceed the 16 KiB storage limit. Nothing was saved.",
		);
	}
	let mut output = panel(saved, fields, notice);
	output.storage = Some(encoded);
	match serde_json::to_vec(&output) {
		Ok(bytes) if bytes.len() <= MAX_IO_BYTES => output,
		_ => panel(
			saved,
			fields,
			"The saved response is too large. Nothing was saved.",
		),
	}
}

fn handle_panel(input: &Invocation) -> Output {
	let saved = match load_saved(input) {
		Ok(saved) => saved,
		Err(error) => {
			return panel(
				&Saved::default(),
				&Fields::from_saved(&Saved::default()),
				error,
			);
		}
	};
	let fields = match Fields::from_input(input, &saved) {
		Ok(fields) => fields,
		Err(error) => return panel(&saved, &Fields::from_saved(&saved), error),
	};
	match input.action.as_str() {
		"open" => panel(&saved, &fields, ""),
		"save" => {
			let keywords = match parse_keywords(&fields.keywords) {
				Ok(keywords) => keywords,
				Err(error) => return panel(&saved, &fields, error),
			};
			let updated = Saved {
				split_mode: fields.split_mode,
				split_limit: fields.split_limit,
				keywords,
				keyword_case_sensitive: fields.case_sensitive,
				burst_style: fields.burst_style,
				burst_max_segments: fields.burst_max_segments,
				merge_with_attachment: fields.merge_with_attachment,
				merge_use_space: fields.merge_use_space,
			};
			if let Err(error) = validate_saved(&updated) {
				return panel(&saved, &fields, error);
			}
			let saved_fields = Fields::from_saved(&updated);
			save_saved(
				&updated,
				&saved_fields,
				"Configuration saved. Split and burst actions only display review segments; they never send messages.",
			)
		}
		_ => Output::default(),
	}
}

fn byte_index_after_chars(content: &str, characters: usize) -> usize {
	content
		.char_indices()
		.nth(characters)
		.map_or(content.len(), |(index, _)| index)
}

fn find_split_index(content: &str, limit: usize, mode: SplitMode) -> usize {
	let hard = byte_index_after_chars(content, limit);
	if mode == SplitMode::Characters || hard == 0 {
		return hard.max(1).min(content.len());
	}
	let prefix = &content[..hard];
	let boundary = if mode == SplitMode::Newlines {
		'\n'
	} else {
		' '
	};
	if let Some(index) = prefix.rfind(boundary)
		&& index > 0
	{
		return index + boundary.len_utf8();
	}
	if mode == SplitMode::Newlines
		&& let Some(index) = prefix.rfind(' ')
		&& index > 0
	{
		return index + 1;
	}
	hard.max(1).min(content.len())
}

/// Split on Unicode scalar boundaries. The caller bounds the draft and limit, so
/// each chunk is at most 2,000 scalars and therefore at most 8,000 UTF-8 bytes.
fn split_draft(draft: &str, mode: SplitMode, limit: usize) -> Vec<String> {
	let limit = limit.max(1);
	let mut chunks = Vec::new();
	let mut remaining = draft;
	while remaining.chars().count() > limit {
		let split = find_split_index(remaining, limit, mode);
		chunks.push(remaining[..split].to_owned());
		remaining = &remaining[split..];
	}
	if !remaining.is_empty() {
		chunks.push(remaining.to_owned());
	}
	chunks
}

fn contains_keyword(line: &str, keyword: &str, case_sensitive: bool) -> bool {
	if keyword.is_empty() {
		return false;
	}
	if case_sensitive {
		line.contains(keyword)
	} else {
		line.to_lowercase().contains(&keyword.to_lowercase())
	}
}

fn filter_keywords(draft: &str, keywords: &[String], case_sensitive: bool) -> (String, usize) {
	let mut output = String::with_capacity(draft.len());
	let mut removed = 0;
	for line in draft.split_inclusive('\n') {
		if keywords
			.iter()
			.any(|keyword| contains_keyword(line, keyword, case_sensitive))
		{
			removed += 1;
		} else {
			output.push_str(line);
		}
	}
	(output, removed)
}

fn line_segments(draft: &str) -> Vec<String> {
	draft
		.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty())
		.map(str::to_owned)
		.collect()
}

fn paragraph_segments(draft: &str) -> Vec<String> {
	let mut segments = Vec::new();
	let mut current = Vec::new();
	for line in draft.lines() {
		if line.trim().is_empty() {
			if !current.is_empty() {
				segments.push(current.join("\n"));
				current.clear();
			}
		} else {
			current.push(line.trim_end().to_owned());
		}
	}
	if !current.is_empty() {
		segments.push(current.join("\n"));
	}
	segments
}

fn sentence_segments(draft: &str) -> Vec<String> {
	let mut segments = Vec::new();
	let mut start = 0;
	for (index, character) in draft.char_indices() {
		if !matches!(character, '.' | '!' | '?') {
			continue;
		}
		let end = index + character.len_utf8();
		let next = draft[end..].chars().next();
		if next.is_none_or(char::is_whitespace) {
			let segment = draft[start..end].trim();
			if !segment.is_empty() {
				segments.push(segment.to_owned());
			}
			start = end;
		}
	}
	let rest = draft[start..].trim();
	if !rest.is_empty() {
		segments.push(rest.to_owned());
	}
	segments
}

fn burst_segments(
	draft: &str,
	style: BurstStyle,
	limit: usize,
	max_segments: usize,
) -> (Vec<String>, usize) {
	let base = match style {
		BurstStyle::Lines => line_segments(draft),
		BurstStyle::Paragraphs => paragraph_segments(draft),
		BurstStyle::Sentences => sentence_segments(draft),
	};
	let fallback = match style {
		BurstStyle::Sentences => SplitMode::Spaces,
		BurstStyle::Lines | BurstStyle::Paragraphs => SplitMode::Newlines,
	};
	let mut segments = Vec::new();
	for segment in base {
		segments.extend(split_draft(&segment, fallback, limit));
	}
	let total = segments.len();
	segments.truncate(max_segments);
	(segments, total)
}

fn push_panel_text(panel: &mut Vec<Element>, value: &str) {
	if value.is_empty() {
		panel.push(text("(empty segment)"));
		return;
	}
	let mut remaining = value;
	while !remaining.is_empty() {
		let mut end = remaining.len().min(MAX_PANEL_TEXT_BYTES);
		while !remaining.is_char_boundary(end) {
			end -= 1;
		}
		if end == 0 {
			end = remaining.chars().next().map_or(0, char::len_utf8);
		}
		panel.push(text(&remaining[..end]));
		remaining = &remaining[end..];
	}
}

fn segment_panel(
	title: &str,
	intro: String,
	note: &str,
	segments: &[String],
	total: usize,
) -> Output {
	let shown = segments.len().min(MAX_PANEL_SEGMENTS);
	let mut panel = vec![heading(title), text(intro), text(note)];
	for (index, segment) in segments.iter().take(shown).enumerate() {
		panel.push(heading(format!(
			"Segment {} ({} chars, {} bytes)",
			index + 1,
			segment.chars().count(),
			segment.len()
		)));
		push_panel_text(&mut panel, segment);
	}
	if total > shown {
		panel.push(text(format!(
			"{} additional segment(s) are not shown by the bounded review panel.",
			total - shown
		)));
	}
	Output {
		panel,
		..Default::default()
	}
}

fn composer_error(error: &str) -> Output {
	Output {
		panel: vec![text(error)],
		..Default::default()
	}
}

fn composer_split(draft: &str, saved: &Saved) -> Output {
	if draft.is_empty() {
		return composer_error("The draft is empty; there is nothing to split.");
	}
	if draft.len() > MAX_DRAFT_BYTES {
		return composer_error(
			"The draft exceeds 8,000 UTF-8 bytes. The draft was left unchanged.",
		);
	}
	let chunks = split_draft(draft, saved.split_mode, saved.split_limit);
	if chunks.len() <= 1 {
		return Output {
			panel: vec![
				heading("Split review"),
				text(format!(
					"The draft already fits in one message ({} chars, {} bytes). No messages were sent.",
					draft.chars().count(),
					draft.len()
				)),
			],
			..Default::default()
		};
	}
	segment_panel(
		"Split review",
		format!(
			"{} segments at the {} boundary, at most {} characters each. No messages were sent or scheduled.",
			chunks.len(),
			saved.split_mode.label().to_lowercase(),
			saved.split_limit
		),
		"Each segment below is Unicode-safe. Review and copy segments manually; this action does not send a message burst.",
		&chunks,
		chunks.len(),
	)
}

fn composer_burst(draft: &str, saved: &Saved) -> Output {
	if draft.is_empty() {
		return composer_error("The draft is empty; there is nothing to disperse.");
	}
	if draft.len() > MAX_DRAFT_BYTES {
		return composer_error(
			"The draft exceeds 8,000 UTF-8 bytes. The draft was left unchanged.",
		);
	}
	let (segments, total) = burst_segments(
		draft,
		saved.burst_style,
		saved.split_limit,
		saved.burst_max_segments,
	);
	if segments.is_empty() {
		return Output {
			panel: vec![
				heading("Burst review"),
				text("No nonblank burst segments were found. Nothing was changed."),
			],
			..Default::default()
		};
	}
	segment_panel(
		"Burst review",
		format!(
			"{} segment(s) using the {} style, at most {} per the saved setting. This is a review only.",
			total,
			saved.burst_style.label().to_lowercase(),
			saved.burst_max_segments
		),
		"MessageBurst's time-window merge and edit of an already-sent message are not available to this sandboxed plugin. Nothing is merged, edited, repeated or sent.",
		&segments,
		total,
	)
}

/// Canonical nonzero decimal snowflake IDs only; malformed IDs are never guessed.
fn canonical_id(value: &str) -> bool {
	let Ok(id) = value.parse::<u64>() else {
		return false;
	};
	id != 0
		&& value.len() <= 20
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value == id.to_string()
}

fn is_ordinary(kind: u8) -> bool {
	matches!(kind, 0 | 19)
}

struct MergeScope<'a> {
	channel_id: &'a str,
	own_id: &'a str,
	timeline: &'a serein_extension_sdk::TimelineSnapshot,
	details: &'a MessageDetailsSnapshot,
	last_message_id: &'a str,
}

fn merge_scope(input: &AppInvocation) -> Result<MergeScope<'_>, &'static str> {
	let app = input.app.as_ref().ok_or(MERGE_MISSING_APP)?;
	let context = app.context.as_ref().ok_or(MERGE_MISSING_APP)?;
	if !context.connected {
		return Err(MERGE_MISSING_APP);
	}
	let user = context.user.as_ref().ok_or(MERGE_MISSING_APP)?;
	let channel = context.channel.as_ref().ok_or(MERGE_MISSING_APP)?;
	if !canonical_id(&user.id) || !canonical_id(&channel.id) {
		return Err(MERGE_STALE_CONTEXT);
	}
	let timeline = app.timeline.as_ref().ok_or(MERGE_MISSING_TIMELINE)?;
	if timeline.channel_id != channel.id || !canonical_id(&timeline.channel_id) {
		return Err(MERGE_STALE_CONTEXT);
	}
	let details = app.message_details.as_ref().ok_or(
		"Serein did not provide loaded message metadata, so the target cannot be verified.",
	)?;
	if details.channel_id != channel.id || !canonical_id(&details.channel_id) {
		return Err(MERGE_STALE_CONTEXT);
	}
	let channel_details = app.channel_details.as_ref().ok_or(
		"Serein did not provide loaded channel details, so it cannot verify that nobody spoke after this message.",
	)?;
	if channel_details.channel.id != channel.id {
		return Err(MERGE_STALE_CONTEXT);
	}
	let last_message_id = channel_details.last_message_id.as_deref().ok_or(
		"Serein did not report the conversation's latest message, so no merge was proposed.",
	)?;
	if !canonical_id(last_message_id) {
		return Err(MERGE_STALE_CONTEXT);
	}
	Ok(MergeScope {
		channel_id: &channel.id,
		own_id: &user.id,
		timeline,
		details,
		last_message_id,
	})
}

fn merge_output(panel: Vec<Element>, effect: Option<HostEffect>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		effects: effect.into_iter().collect(),
	}
}

fn merge_error(message: &str) -> AppOutput {
	merge_output(vec![heading("Burst merge"), text(message)], None)
}

/// Explicit previous-message merge. Upstream edits during a send hook inside a
/// short time window; this sandbox has no send hook or clock, so the user must
/// invoke the composer action and confirm the exact edit.
fn composer_burst_merge(input: &AppInvocation, draft: &str, saved: &Saved) -> AppOutput {
	if draft.is_empty() {
		return merge_error("The draft is empty; there is nothing to merge.");
	}
	if draft.len() > MAX_DRAFT_BYTES {
		return merge_error("The draft exceeds 8,000 UTF-8 bytes. The draft was left unchanged.");
	}
	let scope = match merge_scope(input) {
		Ok(scope) => scope,
		Err(error) => return merge_error(error),
	};
	let Some(last) = scope.timeline.messages.last() else {
		return merge_error(
			"No message is present in the loaded window, so no merge was proposed.",
		);
	};
	if !canonical_id(&last.id) || last.author.id != scope.own_id {
		return merge_error(
			"The latest loaded message was not authored by the current account. Nothing was merged.",
		);
	}
	if last.id != scope.last_message_id {
		return merge_error(
			"A newer message exists in this conversation, so the loaded message is not your latest. Nothing was merged.",
		);
	}
	let Some(detail) = scope.details.items.iter().find(|item| item.id == last.id) else {
		return merge_error(
			"The latest loaded message has no matching metadata window. Nothing was merged.",
		);
	};
	if !is_ordinary(detail.kind) {
		return merge_error(
			"The latest loaded message is not an ordinary message. Nothing was merged.",
		);
	}
	if !detail.attachments.is_empty() && !saved.merge_with_attachment {
		return merge_error(
			"The latest loaded message has attachments and attachment merging is disabled. Nothing was merged.",
		);
	}
	let separator = if saved.merge_use_space { " " } else { "\n" };
	let merged = if last.content.is_empty() {
		draft.to_owned()
	} else {
		format!("{}{separator}{draft}", last.content)
	};
	if merged.trim().is_empty() {
		return merge_error("The merged text would be empty. Nothing was merged.");
	}
	if merged.chars().count() > MAX_MERGED_CHARS || merged.len() > MAX_MERGED_BYTES {
		return merge_error(
			"The merged message would exceed 2,000 characters or 8,000 UTF-8 bytes. Nothing was merged.",
		);
	}
	let mut panel = vec![
		heading("Burst merge"),
		text(format!(
			"Proposes replacing your latest loaded message {} with the previous text plus this draft.",
			last.id
		)),
		text(
			"Serein requires the explicit Apply action. The draft is not cleared, sent or scheduled automatically.",
		),
	];
	push_panel_text(&mut panel, &merged);
	merge_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::EditMessage {
				channel_id: scope.channel_id.to_owned(),
				message_id: last.id.clone(),
				content: merged,
			},
		}),
	)
}

fn composer_filter(draft: &str, saved: &Saved) -> Output {
	if saved.keywords.is_empty() {
		return Output {
			panel: vec![
				heading("Keyword filter"),
				text(
					"No keywords are configured. Open the panel to add literal keywords. The draft was left unchanged.",
				),
			],
			..Default::default()
		};
	}
	if draft.len() > MAX_DRAFT_BYTES {
		return composer_error(
			"The draft exceeds 8,000 UTF-8 bytes. The draft was left unchanged.",
		);
	}
	let (filtered, removed) = filter_keywords(draft, &saved.keywords, saved.keyword_case_sensitive);
	if removed == 0 {
		return Output {
			panel: vec![
				heading("Keyword filter"),
				text("No draft line contained a configured keyword. The draft was left unchanged."),
			],
			..Default::default()
		};
	}
	if filtered.trim().is_empty() {
		return Output {
			panel: vec![
				heading("Keyword filter"),
				text(format!(
					"All {removed} matching line(s) contained a keyword. The draft was left unchanged instead of clearing it."
				)),
			],
			..Default::default()
		};
	}
	if filtered.chars().count() > MAX_SEGMENT_CHARS {
		return Output {
			panel: vec![
				heading("Keyword filter"),
				text(format!(
					"The remaining draft has {} characters, above the 2,000-character message limit. The draft was left unchanged.",
					filtered.chars().count()
				)),
			],
			..Default::default()
		};
	}
	Output {
		replacement: Some(filtered),
		panel: vec![
			heading("Keyword filter"),
			text(format!(
				"Removed {removed} line(s) containing configured keywords. Review the proposed draft and choose Apply; nothing is sent automatically."
			)),
		],
		..Default::default()
	}
}

fn handle(input: AppInvocation) -> AppOutput {
	let invocation = &input.invocation;
	if matches!(invocation.action.as_str(), "open" | "save") {
		return AppOutput {
			output: handle_panel(invocation),
			effects: Vec::new(),
		};
	}
	let Some(draft) = invocation.composer.as_deref() else {
		return AppOutput::default();
	};
	let saved = match load_saved(invocation) {
		Ok(saved) => saved,
		Err(error) => {
			return AppOutput {
				output: composer_error(error),
				effects: Vec::new(),
			};
		}
	};
	let output = match invocation.action.as_str() {
		"split" => composer_split(draft, &saved),
		"filter-keywords" => composer_filter(draft, &saved),
		"burst-disperse" => composer_burst(draft, &saved),
		"burst-merge" => return composer_burst_merge(&input, draft, &saved),
		_ => Output::default(),
	};
	AppOutput {
		output,
		effects: Vec::new(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{
		AppContextSnapshot, AppSnapshot, AttachmentSnapshot, ChannelDetailsSnapshot,
		ChannelSnapshot, MessageDetailSnapshot, MessageSnapshot, TimelineSnapshot, UserSnapshot,
	};

	fn storage(saved: &Saved) -> String {
		serde_json::to_string(saved).unwrap()
	}

	fn composer(action: &str, draft: &str, saved: &Saved) -> Output {
		run(Invocation {
			action: action.into(),
			composer: Some(draft.into()),
			storage: Some(storage(saved)),
			..Default::default()
		})
		.output
	}

	fn run(invocation: Invocation) -> AppOutput {
		handle(AppInvocation {
			invocation,
			..Default::default()
		})
	}

	fn user(id: u64) -> UserSnapshot {
		UserSnapshot {
			id: id.to_string(),
			name: format!("user {id}"),
		}
	}

	fn message(id: u64, author: u64, content: &str, attachments: u16) -> MessageSnapshot {
		MessageSnapshot {
			id: id.to_string(),
			author: user(author),
			content: content.into(),
			attachment_count: attachments,
			edited: false,
		}
	}

	fn detail(id: u64, kind: u8, attachments: usize) -> MessageDetailSnapshot {
		MessageDetailSnapshot {
			id: id.to_string(),
			kind,
			reply_to: None,
			mention_ids: Vec::new(),
			mentions_truncated: false,
			mention_everyone: false,
			attachments: (0..attachments)
				.map(|index| AttachmentSnapshot {
					id: (id + index as u64).to_string(),
					filename: "file.bin".into(),
					size: 1,
					content_type: None,
					spoiler: false,
				})
				.collect(),
			attachments_truncated: false,
			reactions: None,
			reactions_truncated: false,
		}
	}

	fn merge_input(draft: &str, saved: &Saved, last: MessageSnapshot) -> AppInvocation {
		AppInvocation {
			invocation: Invocation {
				action: "burst-merge".into(),
				composer: Some(draft.into()),
				storage: Some(storage(saved)),
				..Default::default()
			},
			app: Some(AppSnapshot {
				context: Some(AppContextSnapshot {
					connected: true,
					user: Some(user(1)),
					channel: Some(ChannelSnapshot {
						id: "20".into(),
						guild_id: Some("2".into()),
						name: "general".into(),
						kind: 0,
					}),
				}),
				timeline: Some(TimelineSnapshot {
					channel_id: "20".into(),
					messages: vec![last.clone()],
					truncated: false,
				}),
				message_details: Some(MessageDetailsSnapshot {
					channel_id: "20".into(),
					items: vec![detail(
						last.id.parse().unwrap(),
						0,
						usize::from(last.attachment_count),
					)],
					truncated: false,
				}),
				channel_details: Some(ChannelDetailsSnapshot {
					channel: ChannelSnapshot {
						id: "20".into(),
						guild_id: Some("2".into()),
						name: "general".into(),
						kind: 0,
					},
					parent_id: None,
					position: 0,
					last_message_id: Some(last.id.clone()),
					message_count: None,
					recipients: Vec::new(),
					recipients_truncated: false,
					can_send: true,
					can_read_history: true,
				}),
				..Default::default()
			}),
			..Default::default()
		}
	}

	fn merged_edit(output: &AppOutput) -> Option<(&str, &str, &str)> {
		match output.effects.as_slice() {
			[
				HostEffect::AppAction {
					action:
						AppAction::EditMessage {
							channel_id,
							message_id,
							content,
						},
				},
			] => Some((channel_id.as_str(), message_id.as_str(), content.as_str())),
			_ => None,
		}
	}

	fn has_text(output: &Output, needle: &str) -> bool {
		output
			.panel
			.iter()
			.any(|element| matches!(element, Element::Text { text } if text.contains(needle)))
	}

	#[test]
	fn splitting_is_unicode_safe_and_prefers_natural_boundaries() {
		let draft = format!("{}🦀{}", "é".repeat(2500), "x".repeat(100));
		let chunks = split_draft(&draft, SplitMode::Characters, MAX_SEGMENT_CHARS);
		assert_eq!(chunks.concat(), draft);
		assert!(chunks.len() >= 2);
		assert!(
			chunks
				.iter()
				.all(|chunk| chunk.chars().count() <= MAX_SEGMENT_CHARS
					&& chunk.len() <= MAX_DRAFT_BYTES)
		);

		assert_eq!(
			split_draft("hello\nworld", SplitMode::Newlines, 8),
			vec!["hello\n".to_owned(), "world".to_owned()]
		);
		assert_eq!(
			split_draft("alpha beta gamma", SplitMode::Spaces, 12),
			vec!["alpha beta ".to_owned(), "gamma".to_owned()]
		);
	}

	#[test]
	fn split_and_burst_are_review_only_and_never_propose_a_send() {
		let saved = Saved {
			split_limit: 100,
			burst_max_segments: 4,
			..Default::default()
		};
		let draft = "a".repeat(250);
		let split = composer("split", &draft, &saved);
		assert!(split.replacement.is_none());
		assert!(split.storage.is_none());
		assert!(has_text(&split, "No messages were sent"));
		assert!(split.panel.iter().any(|element| matches!(
			element,
			Element::Heading { text } if text.starts_with("Segment ")
		)));

		let burst = composer("burst-disperse", "one\ntwo\nthree\nfour\nfive", &saved);
		assert!(burst.replacement.is_none());
		assert!(burst.storage.is_none());
		assert!(has_text(
			&burst,
			"Nothing is merged, edited, repeated or sent"
		));
		assert!(has_text(&burst, "1 additional segment"));

		let oversized = composer("split", &"x".repeat(MAX_DRAFT_BYTES + 1), &saved);
		assert!(oversized.replacement.is_none());
		assert!(has_text(&oversized, "8,000 UTF-8 bytes"));
		for output in [&split, &burst, &oversized] {
			let json = serde_json::to_string(output).unwrap();
			assert!(!json.contains("effects"));
			assert!(!json.contains("message_send"));
		}
	}

	#[test]
	fn keyword_filter_removes_whole_lines_and_preserves_crlf() {
		let saved = Saved {
			keywords: vec!["spam".into()],
			..Default::default()
		};
		let output = composer(
			"filter-keywords",
			"keep\r\nSpam here\r\nkeep two\r\n",
			&saved,
		);
		assert_eq!(output.replacement.as_deref(), Some("keep\r\nkeep two\r\n"));
		assert!(output.storage.is_none());
		assert!(has_text(&output, "Removed 1 line"));
	}

	#[test]
	fn malformed_and_out_of_range_storage_is_preserved() {
		let corrupt = [
			"{broken".to_owned(),
			storage(&Saved {
				split_limit: 0,
				..Default::default()
			}),
			storage(&Saved {
				keywords: vec!["x".repeat(MAX_KEYWORD_BYTES + 1)],
				..Default::default()
			}),
		];
		for raw in corrupt {
			let output = run(Invocation {
				action: "open".into(),
				storage: Some(raw.clone()),
				..Default::default()
			})
			.output;
			assert!(output.storage.is_none());
			assert!(has_text(&output, "invalid or too large"));

			let output = run(Invocation {
				action: "split".into(),
				composer: Some("unchanged".into()),
				storage: Some(raw),
				..Default::default()
			})
			.output;
			assert!(output.replacement.is_none());
			assert!(output.storage.is_none());
			assert!(has_text(&output, "invalid or too large"));
		}
	}

	#[test]
	fn saving_rejects_corrupt_storage_and_roundtrips_bounded_settings() {
		let corrupt = run(Invocation {
			action: "save".into(),
			storage: Some("{broken".into()),
			..Default::default()
		})
		.output;
		assert!(corrupt.storage.is_none());

		let mut values = std::collections::BTreeMap::new();
		for (id, value) in [
			("keywords", "spam, ads"),
			("case-sensitive", "true"),
			("split-mode", "Spaces"),
			("split-limit", "1200"),
			("burst-style", "Paragraphs"),
			("burst-max-segments", "4"),
			("merge-attachment", "true"),
			("merge-space", "true"),
		] {
			values.insert(id.to_owned(), value.to_owned());
		}
		let output = run(Invocation {
			action: "save".into(),
			values,
			..Default::default()
		})
		.output;
		let saved: Saved = serde_json::from_str(output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(saved.split_mode, SplitMode::Spaces);
		assert_eq!(saved.split_limit, 1200);
		assert_eq!(saved.keywords, vec!["spam".to_owned(), "ads".to_owned()]);
		assert!(saved.keyword_case_sensitive);
		assert_eq!(saved.burst_style, BurstStyle::Paragraphs);
		assert_eq!(saved.burst_max_segments, 4);
		assert!(saved.merge_with_attachment);
		assert!(saved.merge_use_space);
	}

	#[test]
	fn filtering_without_keywords_or_matches_is_a_noop() {
		let no_keywords = composer("filter-keywords", "keep me", &Saved::default());
		assert!(no_keywords.replacement.is_none());
		assert!(no_keywords.storage.is_none());

		let no_match = composer(
			"filter-keywords",
			"keep me",
			&Saved {
				keywords: vec!["absent".into()],
				..Default::default()
			},
		);
		assert!(no_match.replacement.is_none());
		assert!(has_text(&no_match, "left unchanged"));
	}

	#[test]
	fn filtering_never_proposes_an_over_limit_or_cleared_draft() {
		let saved = Saved {
			keywords: vec!["drop".into()],
			..Default::default()
		};
		let over_limit = composer(
			"filter-keywords",
			&format!("drop\n{}", "x".repeat(MAX_SEGMENT_CHARS + 1)),
			&saved,
		);
		assert!(over_limit.replacement.is_none());
		assert!(has_text(&over_limit, "above the 2,000-character"));

		let all_filtered = composer("filter-keywords", "drop\r\ndrop", &saved);
		assert!(all_filtered.replacement.is_none());
		assert!(has_text(&all_filtered, "instead of clearing"));
	}

	#[test]
	fn burst_merge_proposes_one_exact_edit_and_never_clears_or_sends() {
		let newline = handle(merge_input(
			"world",
			&Saved::default(),
			message(101, 1, "hello", 0),
		));
		assert_eq!(merged_edit(&newline), Some(("20", "101", "hello\nworld")));
		assert!(newline.output.replacement.is_none());
		assert!(newline.output.storage.is_none());

		let space = handle(merge_input(
			"world",
			&Saved {
				merge_use_space: true,
				..Default::default()
			},
			message(101, 1, "hello", 0),
		));
		assert_eq!(merged_edit(&space), Some(("20", "101", "hello world")));

		// An attachment-only previous message becomes just the draft rather than
		// starting the merged body with a separator.
		let empty = handle(merge_input(
			"world",
			&Saved::default(),
			message(101, 1, "", 0),
		));
		assert_eq!(merged_edit(&empty), Some(("20", "101", "world")));
	}

	#[test]
	fn burst_merge_rejects_foreign_attachment_nonordinary_and_over_limit_targets() {
		let foreign = handle(merge_input(
			"world",
			&Saved::default(),
			message(101, 3, "hello", 0),
		));
		assert!(merged_edit(&foreign).is_none());
		assert!(has_text(
			&foreign.output,
			"not authored by the current account"
		));

		let attachment = handle(merge_input(
			"world",
			&Saved::default(),
			message(101, 1, "hello", 1),
		));
		assert!(merged_edit(&attachment).is_none());
		assert!(has_text(
			&attachment.output,
			"attachment merging is disabled"
		));

		let allowed = handle(merge_input(
			"world",
			&Saved {
				merge_with_attachment: true,
				..Default::default()
			},
			message(101, 1, "hello", 1),
		));
		assert_eq!(merged_edit(&allowed), Some(("20", "101", "hello\nworld")));

		let mut system = merge_input("world", &Saved::default(), message(101, 1, "hello", 0));
		system
			.app
			.as_mut()
			.unwrap()
			.message_details
			.as_mut()
			.unwrap()
			.items[0]
			.kind = 7;
		assert!(merged_edit(&handle(system)).is_none());

		let over = handle(merge_input(
			&"x".repeat(MAX_MERGED_CHARS),
			&Saved::default(),
			message(101, 1, "hello", 0),
		));
		assert!(merged_edit(&over).is_none());
		assert!(has_text(&over.output, "2,000 characters"));

		let mut stale = merge_input("world", &Saved::default(), message(101, 1, "hello", 0));
		stale
			.app
			.as_mut()
			.unwrap()
			.timeline
			.as_mut()
			.unwrap()
			.channel_id = "21".into();
		assert!(merged_edit(&handle(stale)).is_none());

		let mut newer = merge_input("world", &Saved::default(), message(101, 1, "hello", 0));
		newer
			.app
			.as_mut()
			.unwrap()
			.channel_details
			.as_mut()
			.unwrap()
			.last_message_id = Some("102".into());
		assert!(merged_edit(&handle(newer)).is_none());
	}
}
