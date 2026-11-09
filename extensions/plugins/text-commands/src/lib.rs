/*
 * Vencord, a modification for Discord's desktop app
 * Copyright (c) 2022 Vendicated and contributors
 * Copyright (c) 2023 Vendicated and contributors
 * Copyright (c) 2026 Serein port contributors
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use serde::{Deserialize, Serialize};
use serein_extension_sdk::{Element, Invocation, MAX_IO_BYTES, Output};
use std::collections::{BTreeMap, BTreeSet};

const MAX_RULES: usize = 8;
const MAX_TAGS: usize = 8;
const MAX_RULE_FIELD_BYTES: usize = 4096;
const MAX_TEMPLATE_BYTES: usize = 4096;
const MAX_STORAGE_BYTES: usize = 64 * 1024;
const MAX_DRAFT_BYTES: usize = 8000;
const MAX_TAG_ARGUMENTS: usize = 64;
const CORRUPT_STORAGE: &str =
	"Saved settings are invalid or too large. Nothing was changed; original storage was preserved.";
const STORAGE_LIMIT: &str = "The settings exceed the 64 KiB storage limit. Nothing was saved.";
const DRAFT_LIMIT: &str =
	"The draft or transformed result exceeds 8,000 UTF-8 bytes. The draft was left unchanged.";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Saved {
	rules: Vec<Rule>,
	tags: Vec<Tag>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
	find: String,
	replace: String,
	only_if_includes: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tag {
	name: String,
	message: String,
}

#[derive(Clone, Debug, Default)]
struct Fields {
	find: String,
	replace: String,
	condition: String,
	name: String,
	message: String,
	rule_selection: String,
	tag_selection: String,
}

impl Fields {
	fn from_input(input: &Invocation, saved: &Saved) -> Self {
		if input.action == "open" && input.values.is_empty() {
			let mut fields = Self::default();
			if let Some(rule) = saved.rules.first() {
				fields.find = rule.find.clone();
				fields.replace = rule.replace.clone();
				fields.condition = rule.only_if_includes.clone();
			}
			if let Some(tag) = saved.tags.first() {
				fields.name = tag.name.clone();
				fields.message = tag.message.clone();
			}
			fields.normalize_selections(saved);
			return fields;
		}
		let value = |id: &str| input.value(id).unwrap_or_default().to_owned();
		let mut fields = Self {
			find: value("find"),
			replace: value("replace"),
			condition: value("condition"),
			name: value("name"),
			message: value("message"),
			rule_selection: value("rule-selection"),
			tag_selection: value("tag-selection"),
		};
		fields.normalize_selections(saved);
		fields
	}

	fn normalize_selections(&mut self, saved: &Saved) {
		if self.rule_selection.is_empty() && !saved.rules.is_empty() {
			self.rule_selection = rule_option(0, &saved.rules[0]);
		}
		if self.tag_selection.is_empty() && !saved.tags.is_empty() {
			self.tag_selection = tag_option(0, &saved.tags[0]);
		}
	}
}

/// The host rejects a whole panel when any select option exceeds this many UTF-8 bytes.
const MAX_OPTION_BYTES: usize = 128;

fn rule_option(index: usize, rule: &Rule) -> String {
	bounded_option(format!(
		"{}. Find: `{}`; replace: `{}`; condition: `{}`",
		index + 1,
		preview(&rule.find, 32),
		preview(&rule.replace, 32),
		preview(&rule.only_if_includes, 24),
	))
}

fn tag_option(index: usize, tag: &Tag) -> String {
	bounded_option(format!(
		"{}. /{}: {}",
		index + 1,
		tag.name,
		preview(&tag.message, MAX_OPTION_BYTES)
	))
}

/// Clips a complete option, numbered prefix included, to the host's limit. The prefix stays
/// intact, so options remain unique and `selection_index` still finds the entry.
fn bounded_option(option: String) -> String {
	if option.len() <= MAX_OPTION_BYTES {
		return option;
	}
	let clipped = truncate_utf8(&option, MAX_OPTION_BYTES - '…'.len_utf8());
	format!("{clipped}…")
}

fn selection_index(value: &str) -> Option<usize> {
	let digits = value.split_once('.').map_or(value, |(prefix, _)| prefix);
	digits
		.trim()
		.parse::<usize>()
		.ok()
		.and_then(|index| index.checked_sub(1))
}

fn selected_option(options: &[String], value: &str) -> String {
	if let Some(index) = selection_index(value)
		&& let Some(option) = options.get(index)
	{
		return option.clone();
	}
	options
		.iter()
		.find(|option| option.as_str() == value)
		.or_else(|| options.first())
		.cloned()
		.unwrap_or_default()
}

fn valid_tag_name(name: &str) -> bool {
	let bytes = name.as_bytes();
	(1..=32).contains(&bytes.len())
		&& bytes[0].is_ascii_lowercase()
		&& bytes
			.iter()
			.skip(1)
			.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn validate_rule(rule: &Rule) -> Result<(), &'static str> {
	if rule.find.trim().is_empty() {
		return Err("Find must not be blank. Nothing was saved.");
	}
	if rule.find.len() > MAX_RULE_FIELD_BYTES
		|| rule.replace.len() > MAX_RULE_FIELD_BYTES
		|| rule.only_if_includes.len() > MAX_RULE_FIELD_BYTES
	{
		return Err("Each rule field must be at most 4,096 UTF-8 bytes. Nothing was saved.");
	}
	Ok(())
}

fn validate_tag(tag: &Tag) -> Result<(), &'static str> {
	if !valid_tag_name(&tag.name) {
		return Err(
			"Tag names use 1-32 lowercase ASCII letters, digits or hyphens and start with a letter. Nothing was saved.",
		);
	}
	if tag.message.trim().is_empty() {
		return Err("Tag message must not be empty. Nothing was saved.");
	}
	if tag.message.len() > MAX_TEMPLATE_BYTES {
		return Err("Tag message must be at most 4,096 UTF-8 bytes. Nothing was saved.");
	}
	Ok(())
}

fn validate_saved(saved: &Saved) -> Result<(), &'static str> {
	if saved.rules.len() > MAX_RULES || saved.tags.len() > MAX_TAGS {
		return Err(CORRUPT_STORAGE);
	}
	let mut rule_names = BTreeSet::new();
	for rule in &saved.rules {
		if validate_rule(rule).is_err() || !rule_names.insert(rule.find.as_str()) {
			return Err(CORRUPT_STORAGE);
		}
	}
	let mut tag_names = BTreeSet::new();
	for tag in &saved.tags {
		if validate_tag(tag).is_err() || !tag_names.insert(tag.name.as_str()) {
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

fn input(id: &str, label: &str, value: &str) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: truncate_utf8(value, MAX_RULE_FIELD_BYTES).into(),
	}
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

fn preview(value: &str, max_bytes: usize) -> String {
	let clean: String = value
		.chars()
		.map(|character| {
			if character.is_control() {
				' '
			} else {
				character
			}
		})
		.collect();
	let clipped = truncate_utf8(&clean, max_bytes);
	if clipped.len() == clean.len() {
		clean
	} else {
		format!("{clipped}…")
	}
}

fn panel(saved: &Saved, fields: &Fields, notice: &str) -> Output {
	let mut elements = vec![
		heading("Text Commands"),
		text(
			"Local, literal replacements and single-pass tags. Composer results require Apply and never send automatically.",
		),
	];
	if !notice.is_empty() {
		elements.push(text(notice));
	}
	elements.extend([
		heading(format!("Replacement rules ({}/{MAX_RULES})", saved.rules.len())),
		text("Find is the exact rule name used for replacement and deletion. Saving a duplicate updates its existing position."),
		input("find", "Find (rule name)", &fields.find),
		input("replace", "Replace with (\\n makes a newline)", &fields.replace),
		input("condition", "Only if includes (optional)", &fields.condition),
		Element::Row {
			children: vec![
				button("save-rule", "Save replacement rule"),
				button("delete-rule", "Delete rule named by Find"),
			],
		},
	]);
	if !saved.rules.is_empty() {
		let options: Vec<String> = saved
			.rules
			.iter()
			.enumerate()
			.map(|(index, rule)| rule_option(index, rule))
			.collect();
		elements.extend([
			Element::Select {
				id: "rule-selection".into(),
				label: "Saved replacement rules".into(),
				value: selected_option(&options, &fields.rule_selection),
				options,
			},
			button("load-rule", "Load selected rule into fields"),
		]);
	}
	elements.extend([
		heading(format!("Tags ({}/{MAX_TAGS})", saved.tags.len())),
		text("Names determine deterministic replacement and deletion. Use {{arg}} or {{arg = default}} in the message."),
		input("name", "Tag name", &fields.name),
		input("message", "Tag message template", &fields.message),
		Element::Row {
			children: vec![
				button("save-tag", "Save tag"),
				button("delete-tag", "Delete tag named above"),
			],
		},
	]);
	if !saved.tags.is_empty() {
		let options: Vec<String> = saved
			.tags
			.iter()
			.enumerate()
			.map(|(index, tag)| tag_option(index, tag))
			.collect();
		elements.extend([
			Element::Select {
				id: "tag-selection".into(),
				label: "Saved tags".into(),
				value: selected_option(&options, &fields.tag_selection),
				options,
			},
			button("load-tag", "Load selected tag into fields"),
		]);
	}
	Output {
		panel: elements,
		..Default::default()
	}
}

fn save_saved(previous: &Saved, saved: &Saved, fields: &Fields, notice: &str) -> Output {
	let encoded = match serde_json::to_string(saved) {
		Ok(encoded) => encoded,
		Err(_) => {
			return panel(
				previous,
				fields,
				"Could not encode settings. Nothing was saved.",
			);
		}
	};
	if encoded.len() > MAX_STORAGE_BYTES {
		return panel(previous, fields, STORAGE_LIMIT);
	}
	let mut output = panel(saved, fields, notice);
	output.storage = Some(encoded);
	match serde_json::to_vec(&output) {
		Ok(bytes) if bytes.len() <= MAX_IO_BYTES => output,
		_ => panel(
			previous,
			fields,
			"The saved response is too large. Nothing was saved.",
		),
	}
}

fn handle_panel(input: &Invocation) -> Output {
	let mut saved = match load_saved(input) {
		Ok(saved) => saved,
		Err(error) => return panel(&Saved::default(), &Fields::default(), error),
	};
	let mut fields = Fields::from_input(input, &saved);
	match input.action.as_str() {
		"open" => panel(&saved, &fields, ""),
		"save-rule" => {
			let rule = Rule {
				find: fields.find.clone(),
				replace: fields.replace.clone(),
				only_if_includes: fields.condition.clone(),
			};
			if let Err(error) = validate_rule(&rule) {
				return panel(&saved, &fields, error);
			}
			let previous = saved.clone();
			if let Some(existing) = saved.rules.iter_mut().find(|item| item.find == rule.find) {
				*existing = rule;
			} else if saved.rules.len() < MAX_RULES {
				saved.rules.push(rule);
			} else {
				return panel(
					&saved,
					&fields,
					"Eight replacement rules already exist: delete one or replace an existing Find. Nothing was saved.",
				);
			}
			save_saved(&previous, &saved, &fields, "Replacement rule saved.")
		}
		"delete-rule" => {
			let Some(index) = saved.rules.iter().position(|rule| rule.find == fields.find) else {
				return panel(
					&saved,
					&fields,
					"No replacement rule has that exact Find value. Storage was not changed.",
				);
			};
			let previous = saved.clone();
			saved.rules.remove(index);
			save_saved(&previous, &saved, &fields, "Replacement rule deleted.")
		}
		"save-tag" => {
			let tag = Tag {
				name: fields.name.trim().into(),
				message: fields.message.clone(),
			};
			if let Err(error) = validate_tag(&tag) {
				return panel(&saved, &fields, error);
			}
			fields.name = tag.name.clone();
			let previous = saved.clone();
			if let Some(existing) = saved.tags.iter_mut().find(|item| item.name == tag.name) {
				*existing = tag;
			} else if saved.tags.len() < MAX_TAGS {
				saved.tags.push(tag);
			} else {
				return panel(
					&saved,
					&fields,
					"Eight tags already exist: delete one or replace an existing name. Nothing was saved.",
				);
			}
			save_saved(&previous, &saved, &fields, "Tag saved.")
		}
		"delete-tag" => {
			let name = fields.name.trim();
			let Some(index) = saved.tags.iter().position(|tag| tag.name == name) else {
				return panel(
					&saved,
					&fields,
					"No tag has that exact name. Storage was not changed.",
				);
			};
			let previous = saved.clone();
			saved.tags.remove(index);
			save_saved(&previous, &saved, &fields, "Tag deleted.")
		}
		"load-rule" => {
			let Some(index) = selection_index(&fields.rule_selection) else {
				return panel(&saved, &fields, "No saved replacement rule is selected.");
			};
			let Some(rule) = saved.rules.get(index) else {
				return panel(&saved, &fields, "No saved replacement rule is selected.");
			};
			fields.find = rule.find.clone();
			fields.replace = rule.replace.clone();
			fields.condition = rule.only_if_includes.clone();
			panel(
				&saved,
				&fields,
				"Loaded the selected replacement rule into the fields.",
			)
		}
		"load-tag" => {
			let Some(index) = selection_index(&fields.tag_selection) else {
				return panel(&saved, &fields, "No saved tag is selected.");
			};
			let Some(tag) = saved.tags.get(index) else {
				return panel(&saved, &fields, "No saved tag is selected.");
			};
			fields.name = tag.name.clone();
			fields.message = tag.message.clone();
			panel(&saved, &fields, "Loaded the selected tag into the fields.")
		}
		_ => Output::default(),
	}
}

fn decode_newlines(value: &str) -> String {
	value.replace("\\n", "\n")
}

fn push_bounded(output: &mut String, part: &str) -> Result<(), String> {
	if part.len() > MAX_DRAFT_BYTES - output.len() {
		return Err(DRAFT_LIMIT.into());
	}
	output.push_str(part);
	Ok(())
}

/// Push text after decoding literal `\n` sequences, checking the decoded byte
/// length without first allocating an encoded intermediate. A trailing backslash
/// is carried into the next part so decoding matches the concatenated template.
fn push_decoded_bounded(
	output: &mut String,
	part: &str,
	pending_backslash: &mut bool,
) -> Result<(), String> {
	let mut rest = part;
	if *pending_backslash {
		match rest.chars().next() {
			Some('n') => {
				push_bounded(output, "\n")?;
				rest = &rest[1..];
			}
			Some(_) => push_bounded(output, "\\")?,
			None => return Ok(()),
		}
		*pending_backslash = false;
	}
	while let Some(index) = rest.find("\\n") {
		push_bounded(output, &rest[..index])?;
		push_bounded(output, "\n")?;
		rest = &rest[index + 2..];
	}
	*pending_backslash = rest.ends_with('\\');
	if *pending_backslash {
		rest = &rest[..rest.len() - 1];
	}
	push_bounded(output, rest)
}

fn replace_literal(content: &str, find: &str, replace: &str) -> Result<String, String> {
	if find.is_empty() {
		return Ok(content.into());
	}
	let mut result = String::new();
	let mut copied = 0;
	for (index, _) in content.match_indices(find) {
		push_bounded(&mut result, &content[copied..index])?;
		push_bounded(&mut result, replace)?;
		copied = index + find.len();
	}
	push_bounded(&mut result, &content[copied..])?;
	Ok(result)
}

fn apply_replacements(draft: &str, saved: &Saved) -> Result<String, String> {
	if draft.len() > MAX_DRAFT_BYTES {
		return Err(DRAFT_LIMIT.into());
	}
	let mut content = draft.to_owned();
	for rule in &saved.rules {
		if !rule.only_if_includes.is_empty() && !content.contains(&rule.only_if_includes) {
			continue;
		}
		let replacement = decode_newlines(&rule.replace);
		content = replace_literal(&content, &rule.find, &replacement)?;
	}
	Ok(content)
}

fn command_parts(draft: &str) -> Option<(&str, &str)> {
	let body = draft.strip_prefix('/')?;
	let (name, arguments) = body.split_once(' ').unwrap_or((body, ""));
	let arguments = if arguments.trim().is_empty() {
		""
	} else {
		arguments
	};
	valid_tag_name(name).then_some((name, arguments))
}

fn next_placeholder<'a>(template: &'a str, cursor: &mut usize) -> Option<(usize, usize, &'a str)> {
	let open = *cursor + template[*cursor..].find("{{")?;
	let inner_start = open + 2;
	let close = inner_start + template[inner_start..].find("}}")?;
	*cursor = close + 2;
	Some((open, close + 2, &template[inner_start..close]))
}

fn parse_placeholder(inner: &str) -> Option<(String, Option<String>)> {
	let (name, default) = inner
		.split_once('=')
		.map_or((inner, None), |(name, default)| {
			(name, Some(default.trim()))
		});
	let name = name.trim();
	(!name.is_empty()).then(|| (name.into(), default.map(str::to_owned)))
}

fn placeholder_definitions(
	template: &str,
) -> Result<BTreeMap<String, Option<String>>, &'static str> {
	let mut definitions = BTreeMap::new();
	let mut cursor = 0;
	while let Some((_, _, inner)) = next_placeholder(template, &mut cursor) {
		let Some((name, default)) = parse_placeholder(inner) else {
			continue;
		};
		if !definitions.contains_key(&name) {
			if definitions.len() == MAX_TAG_ARGUMENTS {
				return Err(
					"A tag may use at most 64 distinct arguments. The draft was left unchanged.",
				);
			}
			definitions.insert(name, default);
		}
	}
	Ok(definitions)
}

fn next_argument_start(
	tail: &str,
	definitions: &BTreeMap<String, Option<String>>,
	arguments: &BTreeMap<String, String>,
	current: &str,
) -> Option<usize> {
	for (index, character) in tail.char_indices() {
		if character != ' ' {
			continue;
		}
		let candidate = tail[index + 1..].trim_start_matches(' ');
		let Some(colon) = candidate.find(':') else {
			continue;
		};
		let name = &candidate[..colon];
		if name.is_empty() || name == current || arguments.contains_key(name) {
			continue;
		}
		if definitions.contains_key(name) {
			return Some(index);
		}
	}
	None
}

fn parse_arguments(
	mut rest: &str,
	definitions: &BTreeMap<String, Option<String>>,
) -> Result<BTreeMap<String, String>, &'static str> {
	let mut arguments = BTreeMap::new();
	while !rest.is_empty() {
		let Some(colon) = rest.find(':') else {
			return Err("Use key:value tag arguments. The draft was left unchanged.");
		};
		let name = &rest[..colon];
		if name.is_empty() {
			return Err(
				"Tag argument names must immediately precede ':'. The draft was left unchanged.",
			);
		}
		if !definitions.contains_key(name) {
			return Err("The tag does not define that argument. The draft was left unchanged.");
		}
		let tail = &rest[colon + 1..];
		let end = next_argument_start(tail, definitions, &arguments, name).unwrap_or(tail.len());
		arguments.insert(name.into(), tail[..end].trim().into());
		if end == tail.len() {
			break;
		}
		rest = tail[end..].trim_start_matches(' ');
	}
	Ok(arguments)
}

fn render_tag(
	tag: &Tag,
	definitions: &BTreeMap<String, Option<String>>,
	arguments: &BTreeMap<String, String>,
) -> Result<String, String> {
	for (name, default) in definitions {
		if default.is_none() && !arguments.contains_key(name) {
			return Err(format!(
				"Missing required argument `{}` for /{}. The draft was left unchanged.",
				preview(name, 160),
				tag.name,
			));
		}
	}
	let mut result = String::new();
	let mut pending_backslash = false;
	let mut cursor = 0;
	let mut copied = 0;
	while let Some((open, end, inner)) = next_placeholder(&tag.message, &mut cursor) {
		push_decoded_bounded(
			&mut result,
			&tag.message[copied..open],
			&mut pending_backslash,
		)?;
		if let Some((name, _)) = parse_placeholder(inner) {
			let value = arguments
				.get(&name)
				.or_else(|| definitions.get(&name).and_then(|default| default.as_ref()));
			if let Some(value) = value {
				push_decoded_bounded(&mut result, value, &mut pending_backslash)?;
			}
		} else {
			push_decoded_bounded(&mut result, &tag.message[open..end], &mut pending_backslash)?;
		}
		copied = end;
	}
	push_decoded_bounded(&mut result, &tag.message[copied..], &mut pending_backslash)?;
	if pending_backslash {
		push_bounded(&mut result, "\\")?;
	}
	if result.trim().is_empty() {
		return Err("Tag expansion is empty. The draft was left unchanged.".into());
	}
	Ok(result)
}

fn expand_saved_tag(draft: &str, saved: &Saved) -> Result<Option<String>, String> {
	let Some((name, raw_arguments)) = command_parts(draft) else {
		return Ok(None);
	};
	let Some(tag) = saved.tags.iter().find(|tag| tag.name == name) else {
		return Ok(None);
	};
	if draft.len() > MAX_DRAFT_BYTES {
		return Err(DRAFT_LIMIT.into());
	}
	let definitions = placeholder_definitions(&tag.message).map_err(str::to_owned)?;
	let arguments = parse_arguments(raw_arguments, &definitions).map_err(str::to_owned)?;
	render_tag(tag, &definitions, &arguments).map(Some)
}

fn composer_error(error: String) -> Output {
	Output {
		panel: vec![text(error)],
		..Default::default()
	}
}

fn handle(input: Invocation) -> Output {
	match input.action.as_str() {
		"open" | "save-rule" | "delete-rule" | "save-tag" | "delete-tag" | "load-rule"
		| "load-tag" => handle_panel(&input),
		"apply-replacements" | "expand-tag" => {
			let Some(draft) = input.composer.as_deref() else {
				return Output::default();
			};
			let saved = match load_saved(&input) {
				Ok(saved) => saved,
				Err(error) => return composer_error(error.into()),
			};
			let result = if input.action == "apply-replacements" {
				apply_replacements(draft, &saved).map(Some)
			} else {
				expand_saved_tag(draft, &saved)
			};
			match result {
				Ok(Some(replacement)) => Output {
					replacement: Some(replacement),
					..Default::default()
				},
				Ok(None) => Output::default(),
				Err(error) => composer_error(error),
			}
		}
		_ => Output::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;

	fn rule(find: &str, replace: &str) -> Rule {
		Rule {
			find: find.into(),
			replace: replace.into(),
			only_if_includes: String::new(),
		}
	}

	fn invoke(action: &str, storage: &Saved, composer: &str) -> Output {
		handle(Invocation {
			action: action.into(),
			composer: Some(composer.into()),
			storage: Some(serde_json::to_string(storage).unwrap()),
			..Default::default()
		})
	}

	fn invoke_panel(action: &str, storage: Option<&Saved>, values: &[(&str, &str)]) -> Output {
		handle(Invocation {
			action: action.into(),
			storage: storage.map(|saved| serde_json::to_string(saved).unwrap()),
			values: values
				.iter()
				.map(|(id, value)| ((*id).into(), (*value).into()))
				.collect(),
			..Default::default()
		})
	}

	fn input_value<'a>(output: &'a Output, id: &str) -> Option<&'a str> {
		output.panel.iter().find_map(|element| match element {
			Element::TextInput {
				id: element_id,
				value,
				..
			} if element_id == id => Some(value.as_str()),
			_ => None,
		})
	}

	fn option_count(output: &Output, id: &str) -> Option<usize> {
		output.panel.iter().find_map(|element| match element {
			Element::Select {
				id: element_id,
				options,
				..
			} if element_id == id => Some(options.len()),
			_ => None,
		})
	}

	/// The host rejects the whole panel, and with it any storage update, when one select
	/// option exceeds its byte limit.
	fn assert_options_fit_host(output: &Output) {
		for element in &output.panel {
			if let Element::Select { options, .. } = element {
				for option in options {
					assert!(
						option.len() <= MAX_OPTION_BYTES,
						"{} bytes: {option}",
						option.len()
					);
				}
			}
		}
	}

	#[test]
	fn literal_rules_run_sequentially_and_check_current_conditions() {
		let mut second = rule("b", "c");
		second.only_if_includes = "b".into();
		let saved = Saved {
			rules: vec![rule("a", "b"), second],
			tags: vec![],
		};
		let output = invoke("apply-replacements", &saved, "a a");
		assert_eq!(output.replacement.as_deref(), Some("c c"));
	}

	#[test]
	fn replacement_escapes_become_real_newlines() {
		let saved = Saved {
			rules: vec![rule("brb", "be right\\nback")],
			tags: vec![],
		};
		let output = invoke("apply-replacements", &saved, "brb");
		assert_eq!(output.replacement.as_deref(), Some("be right\nback"));
	}

	#[test]
	fn corrupt_storage_is_preserved_and_not_proposed() {
		let output = handle(Invocation {
			action: "save-rule".into(),
			storage: Some("{broken".into()),
			..Default::default()
		});
		assert!(output.storage.is_none());
		assert!(output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("invalid or too large")
		)));
		let output = handle(Invocation {
			action: "apply-replacements".into(),
			composer: Some("unchanged".into()),
			storage: Some("{broken".into()),
			..Default::default()
		});
		assert!(output.replacement.is_none());
		assert!(output.storage.is_none());
	}

	#[test]
	fn tag_defaults_first_duplicate_and_unicode_values_expand() {
		let saved = Saved {
			rules: vec![],
			tags: vec![Tag {
				name: "greet".into(),
				message: "Hello {{ name = world }} / {{name = ignored}}\\n🦀".into(),
			}],
		};
		let output = invoke("expand-tag", &saved, "/greet");
		assert_eq!(
			output.replacement.as_deref(),
			Some("Hello world / world\n🦀")
		);
		let output = invoke("expand-tag", &saved, "/greet name:Zoë 潘");
		assert_eq!(
			output.replacement.as_deref(),
			Some("Hello Zoë 潘 / Zoë 潘\n🦀")
		);
	}

	#[test]
	fn missing_required_argument_blocks_tag_replacement() {
		let saved = Saved {
			rules: vec![],
			tags: vec![Tag {
				name: "greet".into(),
				message: "Hi {{name}}, {{mood = fine}}".into(),
			}],
		};
		let output = invoke("expand-tag", &saved, "/greet mood:great");
		assert!(output.replacement.is_none());
		assert!(output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("Missing required argument `name`")
		)));
	}

	#[test]
	fn duplicate_rule_identity_updates_in_place_and_invalid_fields_are_rejected() {
		let saved = Saved {
			rules: vec![rule("brb", "old")],
			tags: vec![],
		};
		let output = invoke_panel(
			"save-rule",
			Some(&saved),
			&[("find", "brb"), ("replace", "new"), ("condition", "")],
		);
		let stored: Saved = serde_json::from_str(output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(stored.rules, vec![rule("brb", "new")]);

		let rejected = invoke_panel(
			"save-rule",
			Some(&saved),
			&[("find", "   "), ("replace", "new"), ("condition", "")],
		);
		assert!(rejected.storage.is_none());
		assert!(rejected.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("Find must not be blank")
		)));
	}

	#[test]
	fn a_condition_removed_by_an_earlier_rule_skips_the_later_rule() {
		let mut second = rule("b", "c");
		second.only_if_includes = "a".into();
		let saved = Saved {
			rules: vec![rule("a", "x"), second],
			tags: vec![],
		};
		let output = invoke("apply-replacements", &saved, "a b");
		assert_eq!(output.replacement.as_deref(), Some("x b"));
	}

	#[test]
	fn storage_limit_failure_renders_previous_rules_and_keeps_entered_fields() {
		let rules = (0..7)
			.map(|index| Rule {
				find: format!("{index}{}", "a".repeat(2900)),
				replace: "b".repeat(2900),
				only_if_includes: "c".repeat(2900),
			})
			.collect();
		let saved = Saved {
			rules,
			tags: vec![],
		};
		let find = format!("new{}", "d".repeat(4000));
		let replace = "e".repeat(4000);
		let condition = "f".repeat(4000);
		let output = invoke_panel(
			"save-rule",
			Some(&saved),
			&[
				("find", find.as_str()),
				("replace", replace.as_str()),
				("condition", condition.as_str()),
			],
		);
		assert!(output.storage.is_none());
		assert_eq!(option_count(&output, "rule-selection"), Some(7));
		assert!(output.panel.iter().any(|element| matches!(
			element,
			Element::Heading { text } if text.contains("(7/8)")
		)));
		assert_eq!(input_value(&output, "find"), Some(find.as_str()));
		assert_eq!(input_value(&output, "replace"), Some(replace.as_str()));
	}

	#[test]
	fn loading_a_later_saved_entry_uses_full_values_and_keeps_earlier_entries() {
		let long_find = format!("find-{}", "é".repeat(200));
		let long_message = format!("Hello {{{{name}}}} {}", "y".repeat(300));
		let saved = Saved {
			rules: vec![rule("first", "one"), rule(&long_find, "two")],
			tags: vec![
				Tag {
					name: "first".into(),
					message: "one".into(),
				},
				Tag {
					name: "second".into(),
					message: long_message.clone(),
				},
			],
		};
		let rule_selection = rule_option(1, &saved.rules[1]);
		let loaded_rule = invoke_panel(
			"load-rule",
			Some(&saved),
			&[("rule-selection", rule_selection.as_str())],
		);
		assert_eq!(input_value(&loaded_rule, "find"), Some(long_find.as_str()));
		assert_eq!(option_count(&loaded_rule, "rule-selection"), Some(2));
		assert_options_fit_host(&loaded_rule);

		let tag_selection = tag_option(1, &saved.tags[1]);
		let loaded_tag = invoke_panel(
			"load-tag",
			Some(&saved),
			&[("tag-selection", tag_selection.as_str())],
		);
		assert_eq!(
			input_value(&loaded_tag, "message"),
			Some(long_message.as_str())
		);
		assert_eq!(input_value(&loaded_tag, "name"), Some("second"));
		assert_options_fit_host(&loaded_tag);
		assert_eq!(option_count(&loaded_tag, "tag-selection"), Some(2));
	}

	#[test]
	fn spaced_placeholder_names_expand_and_missing_values_do_not_replace() {
		let saved = Saved {
			rules: vec![],
			tags: vec![Tag {
				name: "greet".into(),
				message: "Hello {{first name}}".into(),
			}],
		};
		let output = invoke("expand-tag", &saved, "/greet first name:Ada");
		assert_eq!(output.replacement.as_deref(), Some("Hello Ada"));

		let missing = invoke("expand-tag", &saved, "/greet");
		assert!(missing.replacement.is_none());
		assert!(missing.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("Missing required argument `first name`")
		)));
	}

	#[test]
	fn tag_output_limit_uses_decoded_bytes_and_trailing_space_is_no_arguments() {
		let saved = Saved {
			rules: vec![],
			tags: vec![Tag {
				name: "greet".into(),
				message: "{{x}}{{x}}{{x}}{{x}}{{x}}{{x}}".into(),
			}],
		};
		let argument = "a\\n".repeat(500);
		let output = invoke("expand-tag", &saved, &format!("/greet x:{argument}"));
		let replacement = output.replacement.expect("decoded output fits the limit");
		assert_eq!(replacement.len(), 6000);
		assert!(replacement.starts_with("a\n"));

		let defaulted = Saved {
			rules: vec![],
			tags: vec![Tag {
				name: "greet".into(),
				message: "Hello {{name = world}}".into(),
			}],
		};
		let output = invoke("expand-tag", &defaulted, "/greet ");
		assert_eq!(output.replacement.as_deref(), Some("Hello world"));
	}

	#[test]
	fn noncommands_and_unknown_tags_remain_untouched() {
		let saved = Saved {
			rules: vec![],
			tags: vec![Tag {
				name: "greet".into(),
				message: "Hello".into(),
			}],
		};
		for draft in ["say /greet", "/unknown name:Ada", "/Greet"] {
			let output = invoke("expand-tag", &saved, draft);
			assert!(output.replacement.is_none(), "{draft}");
			assert!(output.panel.is_empty(), "{draft}");
			assert!(output.storage.is_none(), "{draft}");
		}
	}
}
