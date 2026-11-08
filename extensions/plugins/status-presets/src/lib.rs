/*
 * Status presets — a safe Serein port of Equicord's StatusPresets and
 * CustomStatusTimeouts, plus the status portion of StatusWhileActive.
 *
 * Upstream: Equicord/Equicord at commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e.
 * Serein adaptation copyright (c) 2026 Serein contributors.
 * Changed November 8, 2026: bounded local presets replace patched Discord
 * menus; Apply is always an explicit AppAction proposal reviewed by the host.
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License version 3 or (at your
 * option) any later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	AppAction, AppOutput, Element, HostEffect, Invocation, Output, OwnPresencePatch, serde_json,
};

/// Hard preset count cap. The panel, storage and delete/apply lists share it.
const MAX_PRESETS: usize = 32;
/// The whole encoded storage value stays below this many UTF-8 bytes.
const MAX_STORAGE_BYTES: usize = 48 * 1024;
const MAX_NAME_CHARS: usize = 32;
/// Panel select options are limited to 128 UTF-8 bytes by the host.
const MAX_NAME_BYTES: usize = 128;
/// Host-enforced custom-status limits: 128 characters and 512 UTF-8 bytes.
const MAX_STATUS_CHARS: usize = 128;
const MAX_STATUS_BYTES: usize = 512;
/// Host-enforced presence expiry bound: one day.
const MAX_CLEAR_AFTER_SECONDS: u32 = 86_400;
const STORAGE_VERSION: u8 = 1;
const STATUS_VALUES: [&str; 4] = ["online", "idle", "dnd", "invisible"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preset {
	name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	status: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	custom_status: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	clear_after_seconds: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
	version: u8,
	#[serde(default)]
	presets: Vec<Preset>,
}

#[derive(Clone, Debug)]
enum StorageState {
	Empty,
	Valid(Stored),
	/// The saved value is kept exactly as received and never rewritten implicitly.
	Corrupt,
}

impl StorageState {
	fn presets(&self) -> &[Preset] {
		match self {
			Self::Valid(stored) => &stored.presets,
			Self::Empty | Self::Corrupt => &[],
		}
	}

	fn is_corrupt(&self) -> bool {
		matches!(self, Self::Corrupt)
	}
}

#[derive(Clone, Debug, Default)]
struct SaveDraft {
	name: String,
	status: String,
	text: String,
	clear_text: bool,
	clear_after_seconds: String,
}

impl SaveDraft {
	fn from_invocation(input: &Invocation) -> Self {
		let status = input.value("status").unwrap_or("Unchanged");
		Self {
			name: input.value("name").unwrap_or_default().to_owned(),
			status: if status.is_empty() {
				"Unchanged".to_owned()
			} else {
				status.to_owned()
			},
			text: input.value("text").unwrap_or_default().to_owned(),
			clear_text: input
				.parse_value::<bool>("clear-text")
				.ok()
				.flatten()
				.unwrap_or(false),
			clear_after_seconds: input
				.value("clear-after-seconds")
				.unwrap_or_default()
				.to_owned(),
		}
	}
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

fn button(id: &str, label: &str) -> Element {
	Element::Button {
		id: id.into(),
		label: label.into(),
	}
}

fn input(id: &str, label: &str, value: &str) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: value.into(),
	}
}

/// Shorten list text without splitting a character; only the panel display is
/// shortened, the stored preset keeps the complete validated text.
fn preview(value: &str, max_chars: usize) -> String {
	let mut result: String = value.chars().take(max_chars).collect();
	if value.chars().count() > max_chars {
		result.push('…');
	}
	result
}

fn preset_line(preset: &Preset) -> String {
	let mut line = preset.name.clone();
	if let Some(status) = &preset.status {
		line.push_str(" · status ");
		line.push_str(status);
	}
	match preset.custom_status.as_deref() {
		Some("") => line.push_str(" · clears text"),
		Some(value) => {
			line.push_str(" · \"");
			line.push_str(&preview(value, 40));
			line.push('"');
		}
		None => {}
	}
	if let Some(seconds) = preset.clear_after_seconds {
		if seconds == 0 {
			line.push_str(" · removes expiry");
		} else {
			line.push_str(&format!(" · clears after {seconds}s"));
		}
	}
	line
}

fn panel(
	state: &StorageState,
	draft: &SaveDraft,
	selected: &str,
	notice: Option<&str>,
	error: Option<&str>,
) -> Vec<Element> {
	let mut elements = vec![
		heading("Status presets"),
		text(
			"Save small local presets for your own status. Applying one proposes a single account change; nothing is written until you review and Apply it.",
		),
	];
	if state.is_corrupt() {
		elements.push(text(
			"Saved preset data could not be read and has been left unchanged. Saving a valid preset explicitly replaces the damaged value; it is never reset automatically.",
		));
	}
	if let Some(error) = error {
		elements.push(text(format!("Nothing changed: {error}")));
	}
	if let Some(notice) = notice {
		elements.push(text(notice));
	}
	elements.push(Element::Separator);
	elements.push(heading("Saved presets"));
	let presets = state.presets();
	if presets.is_empty() {
		elements.push(text("No presets are saved."));
	} else {
		for preset in presets {
			elements.push(text(preset_line(preset)));
		}
	}
	elements.push(Element::Separator);
	elements.push(heading("Save a preset"));
	elements.push(input("name", "Preset name", &draft.name));
	elements.push(Element::Select {
		id: "status".into(),
		label: "Status (Unchanged keeps the current status)".into(),
		options: vec![
			"Unchanged".into(),
			"online".into(),
			"idle".into(),
			"dnd".into(),
			"invisible".into(),
		],
		value: match parse_status(&draft.status) {
			Ok(Some(status)) => status,
			_ => "Unchanged".into(),
		},
	});
	elements.push(input(
		"text",
		"Custom status text (max 128 characters / 512 bytes)",
		&draft.text,
	));
	elements.push(Element::Checkbox {
		id: "clear-text".into(),
		label: "Clear custom status text instead of keeping or replacing it".into(),
		checked: draft.clear_text,
	});
	elements.push(input(
		"clear-after-seconds",
		"Expiry seconds (blank keeps, 0 removes, max 86400)",
		&draft.clear_after_seconds,
	));
	elements.push(text(
		"Omitted fields are preserved. Zero removes the existing expiry. Positive values start at Apply time. An empty custom status clears the text and its expiry.",
	));
	elements.push(button("save", "Save preset"));
	elements.push(Element::Separator);
	elements.push(heading("Apply or delete"));
	if presets.is_empty() {
		elements.push(text("Save a preset before applying or deleting one."));
	} else {
		elements.push(Element::Select {
			id: "preset".into(),
			label: "Selected preset".into(),
			options: presets.iter().map(|preset| preset.name.clone()).collect(),
			value: if presets.iter().any(|preset| preset.name == selected) {
				selected.into()
			} else {
				presets[0].name.clone()
			},
		});
		elements.push(Element::Row {
			children: vec![
				button("apply", "Apply selected preset"),
				button("delete", "Delete selected preset"),
			],
		});
		elements.push(text(
			"Apply opens the host's proposal review. The plugin never changes presence, starts timers, or retries in the background.",
		));
	}
	elements
}

fn response(panel: Vec<Element>, storage: Option<String>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			storage,
			..Default::default()
		},
		..Default::default()
	}
}

fn load_storage(raw: Option<&str>) -> StorageState {
	let Some(raw) = raw else {
		return StorageState::Empty;
	};
	if raw.len() > MAX_STORAGE_BYTES {
		return StorageState::Corrupt;
	}
	match serde_json::from_str::<Stored>(raw) {
		Ok(stored) if valid_stored(&stored) => StorageState::Valid(stored),
		_ => StorageState::Corrupt,
	}
}

fn valid_name(value: &str) -> bool {
	!value.is_empty()
		&& value.trim() == value
		&& value.chars().count() <= MAX_NAME_CHARS
		&& value.len() <= MAX_NAME_BYTES
		&& !value.chars().any(char::is_control)
}

fn valid_status_text(value: &str) -> bool {
	value.chars().count() <= MAX_STATUS_CHARS
		&& value.len() <= MAX_STATUS_BYTES
		&& (value.is_empty() || value.trim() == value)
		&& !value.chars().any(char::is_control)
}

fn valid_status(value: &str) -> bool {
	STATUS_VALUES.contains(&value)
}

fn valid_preset(preset: &Preset) -> bool {
	valid_name(&preset.name)
		&& preset.status.as_deref().is_none_or(valid_status)
		&& preset
			.custom_status
			.as_deref()
			.is_none_or(valid_status_text)
		&& preset
			.clear_after_seconds
			.is_none_or(|seconds| seconds <= MAX_CLEAR_AFTER_SECONDS)
		&& (preset.status.is_some()
			|| preset.custom_status.is_some()
			|| preset.clear_after_seconds.is_some())
}

fn valid_stored(stored: &Stored) -> bool {
	stored.version == STORAGE_VERSION
		&& stored.presets.len() <= MAX_PRESETS
		&& stored.presets.iter().all(valid_preset)
		&& stored.presets.iter().enumerate().all(|(index, preset)| {
			!stored.presets[..index]
				.iter()
				.any(|earlier| earlier.name == preset.name)
		})
}

fn encode_stored(stored: &Stored) -> Result<String, &'static str> {
	if !valid_stored(stored) {
		return Err("the preset list is invalid");
	}
	let json = serde_json::to_string(stored).map_err(|_| "the preset list could not be encoded")?;
	if json.len() > MAX_STORAGE_BYTES {
		return Err("the preset list would exceed the storage limit");
	}
	Ok(json)
}

fn parse_status(value: &str) -> Result<Option<String>, &'static str> {
	let value = value.trim();
	if value.is_empty() || value.eq_ignore_ascii_case("unchanged") {
		return Ok(None);
	}
	for status in STATUS_VALUES {
		if value.eq_ignore_ascii_case(status) {
			return Ok(Some(status.to_owned()));
		}
	}
	Err("choose Unchanged, online, idle, dnd or invisible")
}

fn build_preset(input: &Invocation, draft: &SaveDraft) -> Result<Preset, &'static str> {
	let name = draft.name.trim();
	if !valid_name(name) {
		return Err("enter a preset name of at most 32 characters / 128 bytes without controls");
	}
	let status = parse_status(&draft.status)?;
	let clear_text = match input.parse_value::<bool>("clear-text") {
		Ok(value) => value.unwrap_or(false),
		Err(_) => return Err("the clear-text choice is invalid"),
	};
	let text = draft.text.trim();
	if clear_text && !text.is_empty() {
		return Err("clear the text field or uncheck Clear custom status text");
	}
	let custom_status = if clear_text {
		Some(String::new())
	} else if text.is_empty() {
		None
	} else if valid_status_text(text) {
		Some(text.to_owned())
	} else {
		return Err(
			"custom status text must be at most 128 characters / 512 bytes, without controls",
		);
	};
	let expiry = draft.clear_after_seconds.trim();
	let clear_after_seconds = if expiry.is_empty() {
		None
	} else {
		match expiry.parse::<u32>() {
			Ok(seconds) if seconds <= MAX_CLEAR_AFTER_SECONDS => Some(seconds),
			_ => return Err("expiry must be a whole number of seconds from 0 through 86400"),
		}
	};
	if status.is_none() && custom_status.is_none() && clear_after_seconds.is_none() {
		return Err("set a status, custom status text, or expiry");
	}
	Ok(Preset {
		name: name.to_owned(),
		status,
		custom_status,
		clear_after_seconds,
	})
}

fn save(input: &Invocation, state: StorageState) -> AppOutput {
	let draft = SaveDraft::from_invocation(input);
	let preset = match build_preset(input, &draft) {
		Ok(preset) => preset,
		Err(error) => {
			return response(panel(&state, &draft, "", None, Some(error)), None);
		}
	};
	let mut presets = state.presets().to_vec();
	if presets.len() >= MAX_PRESETS {
		return response(
			panel(
				&state,
				&draft,
				"",
				None,
				Some("the 32-preset limit is reached; delete a preset first"),
			),
			None,
		);
	}
	if presets.iter().any(|saved| saved.name == preset.name) {
		return response(
			panel(
				&state,
				&draft,
				"",
				None,
				Some("a preset with that name already exists; delete it first"),
			),
			None,
		);
	}
	presets.push(preset);
	let stored = Stored {
		version: STORAGE_VERSION,
		presets,
	};
	let json = match encode_stored(&stored) {
		Ok(json) => json,
		Err(error) => return response(panel(&state, &draft, "", None, Some(error)), None),
	};
	let panel = panel(
		&StorageState::Valid(stored),
		&SaveDraft::default(),
		"",
		Some("Preset saved locally. Nothing was applied to your status."),
		None,
	);
	response(panel, Some(json))
}

fn delete(input: &Invocation, state: StorageState) -> AppOutput {
	let draft = SaveDraft::from_invocation(input);
	let name = input.value("preset").unwrap_or_default().trim();
	let mut presets = state.presets().to_vec();
	let Some(index) = presets.iter().position(|preset| preset.name == name) else {
		return response(
			panel(
				&state,
				&draft,
				"",
				None,
				Some("choose a saved preset to delete"),
			),
			None,
		);
	};
	presets.remove(index);
	let stored = Stored {
		version: STORAGE_VERSION,
		presets,
	};
	let json = match encode_stored(&stored) {
		Ok(json) => json,
		Err(error) => return response(panel(&state, &draft, "", None, Some(error)), None),
	};
	let panel = panel(
		&StorageState::Valid(stored),
		&draft,
		"",
		Some("Preset deleted. Nothing was applied to your status."),
		None,
	);
	response(panel, Some(json))
}

fn apply(input: &Invocation, state: StorageState) -> AppOutput {
	let draft = SaveDraft::from_invocation(input);
	let name = input.value("preset").unwrap_or_default().trim();
	let Some(preset) = state.presets().iter().find(|preset| preset.name == name) else {
		return response(
			panel(
				&state,
				&draft,
				"",
				None,
				Some("choose a saved preset to apply"),
			),
			None,
		);
	};
	let presence = OwnPresencePatch {
		status: preset.status.clone(),
		custom_status: preset.custom_status.clone(),
		clear_after_seconds: preset.clear_after_seconds,
	};
	if presence == OwnPresencePatch::default() {
		return response(
			panel(
				&state,
				&draft,
				name,
				None,
				Some("that preset has no status fields to apply"),
			),
			None,
		);
	}
	let panel = panel(
		&state,
		&draft,
		name,
		Some(&format!(
			"Ready to propose preset \"{}\". Review the host's Apply dialog; the preset list and stored data are unchanged.",
			preview(name, MAX_NAME_CHARS)
		)),
		None,
	);
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		effects: vec![HostEffect::AppAction {
			action: AppAction::SetOwnPresence { presence },
		}],
	}
}

fn handle(input: Invocation) -> AppOutput {
	let state = load_storage(input.storage.as_deref());
	match input.action.as_str() {
		"open" | "list" => {
			let draft = SaveDraft::from_invocation(&input);
			let selected = input.value("preset").unwrap_or_default();
			response(panel(&state, &draft, selected, None, None), None)
		}
		"save" => save(&input, state),
		"delete" => delete(&input, state),
		"apply" => apply(&input, state),
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;

	fn invocation(action: &str, storage: Option<&str>, values: &[(&str, &str)]) -> Invocation {
		let mut input = Invocation {
			action: action.into(),
			storage: storage.map(str::to_owned),
			..Default::default()
		};
		input.values.extend(
			values
				.iter()
				.map(|(key, value)| ((*key).into(), (*value).into())),
		);
		input
	}

	fn save(name: &str, text: &str) -> AppOutput {
		handle(invocation(
			"save",
			None,
			&[("name", name), ("status", "online"), ("text", text)],
		))
	}

	fn stored(json: Option<&str>) -> Stored {
		serde_json::from_str(json.expect("storage was written")).unwrap()
	}

	fn panel_text(output: &AppOutput) -> String {
		output
			.output
			.panel
			.iter()
			.filter_map(|element| match element {
				Element::Text { text } => Some(text.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n")
	}

	fn panel_elements(elements: &[Element]) -> usize {
		elements
			.iter()
			.map(|element| {
				1 + match element {
					Element::Row { children } => panel_elements(children),
					_ => 0,
				}
			})
			.sum()
	}

	#[test]
	fn preset_capacity_is_32_and_the_panel_stays_bounded() {
		let mut storage = None;
		for index in 0..MAX_PRESETS {
			let name = format!("Preset {index}");
			let output = handle(invocation(
				"save",
				storage.as_deref(),
				&[("name", &name), ("status", "online"), ("text", "Busy")],
			));
			assert!(output.output.storage.is_some(), "preset {index} should fit");
			assert!(output.effects.is_empty());
			storage = output.output.storage;
		}
		let full = handle(invocation(
			"save",
			storage.as_deref(),
			&[("name", "Overflow"), ("status", "idle")],
		));
		assert!(full.output.storage.is_none());
		assert!(full.effects.is_empty());
		assert!(panel_text(&full).contains("32-preset limit"));
		assert_eq!(stored(storage.as_deref()).presets.len(), MAX_PRESETS);
		assert!(panel_elements(&full.output.panel) <= 64);
	}

	#[test]
	fn custom_status_text_uses_character_and_utf8_byte_bounds() {
		let exact = "🦀".repeat(MAX_STATUS_CHARS);
		assert_eq!(exact.len(), MAX_STATUS_BYTES);
		let output = save("Unicode exact", &exact);
		assert!(output.output.storage.is_some());
		assert_eq!(
			stored(output.output.storage.as_deref()).presets[0]
				.custom_status
				.as_deref(),
			Some(exact.as_str())
		);

		for text in [
			"🦀".repeat(MAX_STATUS_CHARS + 1),
			"a".repeat(MAX_STATUS_CHARS + 1),
		] {
			let output = save("Unicode over", &text);
			assert!(output.output.storage.is_none());
			assert!(output.effects.is_empty());
			assert!(panel_text(&output).contains("128 characters"));
		}

		let boundary_name = "🦀".repeat(MAX_NAME_CHARS);
		assert_eq!(boundary_name.len(), MAX_NAME_BYTES);
		assert!(save(&boundary_name, "Fits").output.storage.is_some());
		assert!(
			save(&"🦀".repeat(MAX_NAME_CHARS + 1), "Too long")
				.output
				.storage
				.is_none()
		);
	}

	#[test]
	fn expiry_accepts_zero_and_86400_and_rejects_larger_values() {
		for (name, seconds) in [("No expiry", "0"), ("One day", "86400")] {
			let output = handle(invocation(
				"save",
				None,
				&[
					("name", name),
					("status", "dnd"),
					("clear-after-seconds", seconds),
				],
			));
			assert!(output.output.storage.is_some(), "{seconds}");
			assert_eq!(
				stored(output.output.storage.as_deref()).presets[0].clear_after_seconds,
				Some(seconds.parse().unwrap())
			);
		}
		let base = save("Base", "Busy").output.storage;
		for seconds in ["86401", "-1", "not-a-number"] {
			let output = handle(invocation(
				"save",
				base.as_deref(),
				&[("name", "Too long"), ("clear-after-seconds", seconds)],
			));
			assert!(output.output.storage.is_none(), "{seconds}");
			assert!(panel_text(&output).contains("0 through 86400"), "{seconds}");
			assert_eq!(stored(base.as_deref()).presets.len(), 1);
		}
	}

	#[test]
	fn apply_preserves_omitted_fields_and_never_writes_storage() {
		let presets = vec![
			Preset {
				name: "Status only".into(),
				status: Some("idle".into()),
				custom_status: None,
				clear_after_seconds: None,
			},
			Preset {
				name: "Text only".into(),
				status: None,
				custom_status: Some("Heads down".into()),
				clear_after_seconds: None,
			},
			Preset {
				name: "Remove expiry".into(),
				status: None,
				custom_status: Some("Back soon".into()),
				clear_after_seconds: Some(0),
			},
		];
		let storage = serde_json::to_string(&Stored {
			version: STORAGE_VERSION,
			presets,
		})
		.unwrap();

		let status = handle(invocation(
			"apply",
			Some(&storage),
			&[("preset", "Status only")],
		));
		assert!(status.output.storage.is_none());
		assert_eq!(status.effects.len(), 1);
		let HostEffect::AppAction {
			action: AppAction::SetOwnPresence { presence },
		} = &status.effects[0]
		else {
			panic!("expected a set_own_presence proposal");
		};
		assert_eq!(presence.status.as_deref(), Some("idle"));
		assert!(presence.custom_status.is_none());
		assert!(presence.clear_after_seconds.is_none());
		let wire = serde_json::to_value(&status.effects[0]).unwrap();
		assert!(wire["action"]["presence"].get("custom_status").is_none());
		assert!(
			wire["action"]["presence"]
				.get("clear_after_seconds")
				.is_none()
		);

		let text = handle(invocation(
			"apply",
			Some(&storage),
			&[("preset", "Text only")],
		));
		let HostEffect::AppAction {
			action: AppAction::SetOwnPresence { presence },
		} = &text.effects[0]
		else {
			panic!("expected a set_own_presence proposal");
		};
		assert!(presence.status.is_none());
		assert_eq!(presence.custom_status.as_deref(), Some("Heads down"));
		assert!(presence.clear_after_seconds.is_none());

		let expiry = handle(invocation(
			"apply",
			Some(&storage),
			&[("preset", "Remove expiry")],
		));
		assert!(expiry.output.storage.is_none());
		let HostEffect::AppAction {
			action: AppAction::SetOwnPresence { presence },
		} = &expiry.effects[0]
		else {
			panic!("expected a set_own_presence proposal");
		};
		assert_eq!(presence.clear_after_seconds, Some(0));
	}

	#[test]
	fn stale_unknown_and_malformed_storage_is_preserved_until_explicit_repair() {
		let oversized = format!(
			"{{\"version\":1,\"presets\":[],\"padding\":\"{}\"}}",
			"x".repeat(MAX_STORAGE_BYTES)
		);
		for raw in [
			"not json".to_owned(),
			"{}".to_owned(),
			"{\"version\":0,\"presets\":[]}".to_owned(),
			"{\"version\":1,\"presets\":[{\"name\":\"Old\",\"status\":\"online\",\"future\":true}]}"
				.to_owned(),
			oversized,
		] {
			let listed = handle(invocation("open", Some(&raw), &[]));
			assert!(listed.output.storage.is_none(), "{raw}");
			assert!(panel_text(&listed).contains("left unchanged"), "{raw}");

			let applied = handle(invocation("apply", Some(&raw), &[("preset", "Old")]));
			assert!(applied.output.storage.is_none(), "{raw}");
			assert!(applied.effects.is_empty(), "{raw}");

			let deleted = handle(invocation("delete", Some(&raw), &[("preset", "Old")]));
			assert!(deleted.output.storage.is_none(), "{raw}");

			let invalid = handle(invocation(
				"save",
				Some(&raw),
				&[("name", "Still broken"), ("text", &"a".repeat(129))],
			));
			assert!(invalid.output.storage.is_none(), "{raw}");

			let repaired = handle(invocation(
				"save",
				Some(&raw),
				&[("name", "Repaired"), ("status", "online")],
			));
			let repaired_storage = repaired.output.storage.as_deref().unwrap();
			let saved: Stored = serde_json::from_str(repaired_storage).unwrap();
			assert_eq!(saved.presets.len(), 1);
			assert_eq!(saved.presets[0].name, "Repaired");
		}
	}

	#[test]
	fn delete_removes_only_the_selected_preset() {
		let storage = serde_json::to_string(&Stored {
			version: STORAGE_VERSION,
			presets: vec![
				Preset {
					name: "Keep".into(),
					status: Some("online".into()),
					custom_status: None,
					clear_after_seconds: None,
				},
				Preset {
					name: "Remove".into(),
					status: Some("idle".into()),
					custom_status: None,
					clear_after_seconds: None,
				},
			],
		})
		.unwrap();
		let output = handle(invocation(
			"delete",
			Some(&storage),
			&[("preset", "Remove")],
		));
		let saved = stored(output.output.storage.as_deref());
		assert_eq!(saved.presets.len(), 1);
		assert_eq!(saved.presets[0].name, "Keep");
		assert!(output.effects.is_empty());
	}
}
