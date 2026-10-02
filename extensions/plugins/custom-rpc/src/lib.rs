use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	CustomRichPresence, Element, Invocation, Output, RichPresenceButton, RichPresenceImage,
	RichPresenceKind, RichPresenceOutput, RichPresenceParty, RichPresenceTimer, RichPresenceUpdate,
};
use std::collections::BTreeMap;

const FIELDS: &[(&str, &str)] = &[
	("application-id", "Application ID"),
	("name", "Activity name"),
	("details", "Details"),
	("details-url", "Details link (HTTPS, optional)"),
	("state", "State"),
	("state-url", "State link (HTTPS, optional)"),
	("stream-url", "Stream link (HTTPS, required for Streaming)"),
	("large-key", "Large image asset key or HTTPS image URL"),
	("large-text", "Large image tooltip"),
	("large-url", "Large image link (HTTPS, optional)"),
	("small-key", "Small image asset key or HTTPS image URL"),
	("small-text", "Small image tooltip"),
	("small-url", "Small image link (HTTPS, optional)"),
	("button1-label", "First button label"),
	("button1-url", "First button link (HTTPS)"),
	("button2-label", "Second button label"),
	("button2-url", "Second button link (HTTPS)"),
	("party-current", "Party members (optional)"),
	("party-max", "Party capacity (optional)"),
	("start", "Custom start (UTC: YYYY-MM-DD HH:mm, optional)"),
	("end", "Custom end (UTC: YYYY-MM-DD HH:mm, optional)"),
];

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
	active: bool,
	fields: BTreeMap<String, String>,
}

fn get<'a>(fields: &'a BTreeMap<String, String>, key: &str) -> &'a str {
	fields.get(key).map_or("", String::as_str)
}

fn optional(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
	let value = get(fields, key).trim();
	(!value.is_empty()).then(|| value.to_owned())
}

fn number<T: std::str::FromStr>(
	fields: &BTreeMap<String, String>,
	key: &str,
	error: &'static str,
) -> Result<Option<T>, &'static str> {
	optional(fields, key)
		.map(|v| v.parse().map_err(|_| error))
		.transpose()
}

fn image(
	fields: &BTreeMap<String, String>,
	prefix: &str,
) -> Result<Option<RichPresenceImage>, &'static str> {
	let key = optional(fields, &format!("{prefix}-key"));
	let text = optional(fields, &format!("{prefix}-text"));
	let url = optional(fields, &format!("{prefix}-url"));
	if key.is_none() && (text.is_some() || url.is_some()) {
		return Err("Add an image key or URL before its tooltip or link.");
	}
	Ok(key.map(|key| RichPresenceImage { key, text, url }))
}

fn timestamp(value: &str) -> Result<Option<u64>, &'static str> {
	let value = value.trim();
	if value.is_empty() {
		return Ok(None);
	}
	let error = "Use a valid UTC date and time: YYYY-MM-DD HH:mm (1970 or later).";
	let bytes = value.as_bytes();
	if bytes.len() != 16
		|| bytes[4] != b'-'
		|| bytes[7] != b'-'
		|| bytes[10] != b' '
		|| bytes[13] != b':'
		|| bytes
			.iter()
			.enumerate()
			.any(|(i, b)| ![4, 7, 10, 13].contains(&i) && !b.is_ascii_digit())
	{
		return Err(error);
	}
	let part = |start, end| value[start..end].parse::<u64>().map_err(|_| error);
	let (year, month, day, hour, minute) = (
		part(0, 4)?,
		part(5, 7)?,
		part(8, 10)?,
		part(11, 13)?,
		part(14, 16)?,
	);
	let leap = |year: u64| {
		year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
	};
	let months = [
		31,
		if leap(year) { 29 } else { 28 },
		31,
		30,
		31,
		30,
		31,
		31,
		30,
		31,
		30,
		31,
	];
	if year < 1970
		|| !(1..=12).contains(&month)
		|| day == 0
		|| day > months[(month - 1) as usize]
		|| hour > 23
		|| minute > 59
	{
		return Err(error);
	}
	let leap_days = |year: u64| year / 4 - year / 100 + year / 400;
	let days = (year - 1970) * 365 + leap_days(year - 1) - leap_days(1969)
		+ months[..(month - 1) as usize].iter().sum::<u64>()
		+ day - 1;
	Ok(Some(((days * 24 + hour) * 60 + minute) * 60_000))
}

fn presence(fields: &BTreeMap<String, String>) -> Result<CustomRichPresence, &'static str> {
	let kind = match get(fields, "kind") {
		"" | "Playing" => RichPresenceKind::Playing,
		"Streaming" => RichPresenceKind::Streaming,
		"Listening" => RichPresenceKind::Listening,
		"Watching" => RichPresenceKind::Watching,
		"Competing" => RichPresenceKind::Competing,
		_ => return Err("Choose an activity type."),
	};
	let timer = match get(fields, "timer") {
		"" | "None" => RichPresenceTimer::None,
		"Since starting" => RichPresenceTimer::Elapsed,
		"Since local midnight" => RichPresenceTimer::LocalDay,
		"Custom timestamps" => RichPresenceTimer::Custom {
			start: timestamp(get(fields, "start"))?,
			end: timestamp(get(fields, "end"))?,
		},
		_ => return Err("Choose a timer mode."),
	};
	let mut buttons = Vec::new();
	for prefix in ["button1", "button2"] {
		match (
			optional(fields, &format!("{prefix}-label")),
			optional(fields, &format!("{prefix}-url")),
		) {
			(Some(label), Some(url)) => buttons.push(RichPresenceButton { label, url }),
			(None, None) => {}
			_ => return Err("Each button needs both a label and an HTTPS link."),
		}
	}
	let party = match (
		number(
			fields,
			"party-current",
			"Party members must be a whole number.",
		)?,
		number(
			fields,
			"party-max",
			"Party capacity must be a whole number.",
		)?,
	) {
		(Some(current), Some(max)) => Some(RichPresenceParty { current, max }),
		(None, None) => None,
		_ => return Err("Enter both party members and capacity, or leave both empty."),
	};
	let value = CustomRichPresence {
		application_id: get(fields, "application-id").trim().into(),
		name: get(fields, "name").trim().into(),
		kind,
		stream_url: optional(fields, "stream-url"),
		details: optional(fields, "details"),
		details_url: optional(fields, "details-url"),
		state: optional(fields, "state"),
		state_url: optional(fields, "state-url"),
		large_image: image(fields, "large")?,
		small_image: image(fields, "small")?,
		buttons,
		party,
		timer,
	};
	value.validate()?;
	Ok(value)
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
fn select(fields: &BTreeMap<String, String>, id: &str, label: &str, options: &[&str]) -> Element {
	let value = get(fields, id);
	Element::Select {
		id: id.into(),
		label: label.into(),
		options: options.iter().map(|v| (*v).into()).collect(),
		value: if value.is_empty() { options[0] } else { value }.into(),
	}
}

fn panel(
	fields: &BTreeMap<String, String>,
	active: bool,
	notice: &str,
	preview: Option<CustomRichPresence>,
) -> Vec<Element> {
	let mut elements = vec![
		heading("Custom Rich Presence"),
		text(if active {
			"Saved presence enabled. Apply updates it; Stop clears it."
		} else {
			"Presence stopped. Preview your changes, then Apply."
		}),
	];
	if !notice.is_empty() {
		elements.push(text(notice));
	}
	if let Some(presence) = preview {
		elements.push(Element::ActivityPreview {
			presence: Box::new(presence),
		});
	}
	let actions = || Element::Row {
		children: vec![
			button("preview", "Preview"),
			button("apply", "Apply presence"),
			button("stop", "Stop presence"),
		],
	};
	elements.push(actions());
	elements.push(heading("Activity"));
	elements.push(select(
		fields,
		"kind",
		"Activity type",
		&["Playing", "Streaming", "Listening", "Watching", "Competing"],
	));
	for (id, label) in FIELDS {
		match *id {
			"large-key" => {
				elements.push(heading("Artwork"));
				elements.push(text(
					"Use assets belonging to your application, or public HTTPS images. Empty fields are omitted.",
				));
			}
			"button1-label" => elements.push(heading("Buttons")),
			"party-current" => elements.push(heading("Party")),
			"start" => {
				elements.push(heading("Timer"));
				elements.push(select(
					fields,
					"timer",
					"Timer mode",
					&[
						"None",
						"Since starting",
						"Since local midnight",
						"Custom timestamps",
					],
				));
				elements.push(text("Custom timestamps are used only in Custom mode. End shows a countdown; start alone shows elapsed time."));
			}
			_ => {}
		}
		elements.push(Element::TextInput {
			id: (*id).into(),
			label: (*label).into(),
			value: get(fields, id).into(),
		});
	}
	elements.push(button("reset", "Reset draft fields"));
	elements.push(heading("Setup and help"));
	elements.push(text("Your saved presence resumes when this plugin is enabled. Changes remain a draft until Apply. Stop disables automatic resume and keeps your saved fields."));
	elements.push(text("Create an application at https://discord.com/developers/applications and copy its Application ID. Upload artwork there or use public HTTPS image URLs."));
	elements
}

fn handle(input: Invocation) -> RichPresenceOutput {
	let saved_result = input.storage_json::<Saved>();
	let corrupt = saved_result.is_err();
	let mut saved = saved_result.ok().flatten().unwrap_or_default();
	if input.action == "activate" {
		return RichPresenceOutput {
			rich_presence: saved
				.active
				.then(|| presence(&saved.fields).ok())
				.flatten()
				.map(|presence| RichPresenceUpdate::Set {
					presence: Box::new(presence),
				}),
			..Default::default()
		};
	}
	let mut fields = saved.fields.clone();
	if input.action != "open" {
		for key in FIELDS.iter().map(|(id, _)| *id).chain(["kind", "timer"]) {
			if let Some(value) = input.value(key) {
				fields.insert(key.into(), value.into());
			}
		}
	}
	let mut notice = if corrupt {
		"Saved settings could not be read. Review the fields before applying."
	} else {
		""
	};
	let mut preview = None;
	let mut update = None;
	let mut store = false;
	match input.action.as_str() {
		"apply" | "preview" => match presence(&fields) {
			Ok(value) => {
				preview = Some(value.clone());
				if input.action == "apply" {
					saved = Saved {
						active: true,
						fields: fields.clone(),
					};
					store = true;
					update = Some(RichPresenceUpdate::Set {
						presence: Box::new(value),
					});
					notice = "Presence submitted. Visibility depends on your connection and activity-sharing settings.";
				} else {
					notice = "Local preview only. Apply to save and share these changes.";
				}
			}
			Err(error) => notice = error,
		},
		"stop" => {
			saved.active = false;
			store = true;
			update = Some(RichPresenceUpdate::Clear);
			notice = "Presence stopped. Your last applied fields are saved; current draft edits are still shown.";
		}
		"reset" => {
			fields.clear();
			notice = "Draft fields reset. Your saved presence is unchanged until Apply or Stop.";
		}
		"open" => {
			preview = presence(&fields).ok();
		}
		_ => return RichPresenceOutput::default(),
	}
	let mut output = Output {
		panel: panel(&fields, saved.active, notice, preview),
		..Default::default()
	};
	if store && output.set_storage_json(&saved).is_err() {
		return RichPresenceOutput {
			output: Output {
				panel: panel(
					&fields,
					false,
					"Could not save these settings. Nothing was applied.",
					None,
				),
				..Default::default()
			},
			..Default::default()
		};
	}
	RichPresenceOutput {
		output,
		rich_presence: update,
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn complete_editor_fields_reach_typed_presence() {
		let fields = [
			("application-id", "123456789"),
			("name", "Synthetic activity"),
			("kind", "Streaming"),
			("stream-url", "https://example.com/live"),
			("details", "Exploring"),
			("details-url", "https://example.com/details"),
			("state", "With friends"),
			("state-url", "https://example.com/state"),
			("large-key", "cover"),
			("large-text", "Cover art"),
			("large-url", "https://example.com/cover"),
			("small-key", "https://example.com/icon.png"),
			("small-text", "Icon"),
			("small-url", "https://example.com/icon"),
			("button1-label", "Read more"),
			("button1-url", "https://example.com/read"),
			("button2-label", "Watch"),
			("button2-url", "https://example.com/watch"),
			("party-current", "2"),
			("party-max", "8"),
			("timer", "Custom timestamps"),
			("start", "2024-03-01 00:00"),
			("end", "2024-03-01 01:00"),
		]
		.into_iter()
		.map(|(key, value)| (key.into(), value.into()))
		.collect();
		let activity = presence(&fields).unwrap();
		assert_eq!(activity.kind, RichPresenceKind::Streaming);
		assert_eq!(activity.buttons.len(), 2);
		assert_eq!(activity.large_image.unwrap().key, "cover");
		assert_eq!(activity.small_image.unwrap().text.as_deref(), Some("Icon"));
		assert_eq!(activity.party.unwrap().max, 8);
		assert!(matches!(
			activity.timer,
			RichPresenceTimer::Custom {
				start: Some(1_709_251_200_000),
				end: Some(1_709_254_800_000)
			}
		));
	}
	#[test]
	fn utc_calendar_input_validates_real_dates_and_converts_to_milliseconds() {
		assert_eq!(timestamp("1970-01-01 00:01"), Ok(Some(60_000)));
		assert_eq!(timestamp("2000-02-29 00:00"), Ok(Some(951_782_400_000)));
		assert_eq!(timestamp("2024-03-01 00:00"), Ok(Some(1_709_251_200_000)));
		assert_eq!(timestamp(""), Ok(None));
		for value in [
			"2023-02-29 00:00",
			"2100-02-29 00:00",
			"2024-04-31 00:00",
			"2024-01-01 24:00",
			"1969-01-01 00:00",
			"2024-01-01 00:60",
			"２０２４-01-01 00:00",
		] {
			assert!(timestamp(value).is_err(), "{value}");
		}
	}
	#[test]
	fn drafts_are_passive_and_stop_disables_resume_without_losing_saved_fields() {
		let mut input = Invocation {
			action: "preview".into(),
			..Default::default()
		};
		input.values.extend([
			("application-id".into(), "123456789".into()),
			("name".into(), "Synthetic activity".into()),
		]);
		let preview = handle(input.clone());
		assert!(preview.rich_presence.is_none());
		assert!(preview.output.storage.is_none());
		assert!(
			preview
				.output
				.panel
				.iter()
				.any(|element| matches!(element, Element::ActivityPreview { .. }))
		);
		input.action = "apply".into();
		let applied = handle(input.clone());
		assert!(matches!(
			applied.rich_presence,
			Some(RichPresenceUpdate::Set { .. })
		));
		input.storage = applied.output.storage;
		input.action = "activate".into();
		assert!(handle(input.clone()).rich_presence.is_some());
		input.action = "stop".into();
		input.values.insert("name".into(), "Unsaved draft".into());
		let stopped = handle(input.clone());
		assert!(matches!(
			stopped.rich_presence,
			Some(RichPresenceUpdate::Clear)
		));
		let saved: Saved =
			serde_json::from_str(stopped.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(get(&saved.fields, "name"), "Synthetic activity");
		input.storage = stopped.output.storage;
		input.action = "activate".into();
		assert!(handle(input.clone()).rich_presence.is_none());
		input.action = "apply".into();
		input
			.values
			.insert("button1-url".into(), "javascript:alert(1)".into());
		let invalid = handle(input);
		assert!(invalid.output.storage.is_none());
		assert!(invalid.rich_presence.is_none());
		assert!(invalid.output.panel.iter().any(
			|e| matches!(e, Element::TextInput { id, value, .. } if id == "name" && value == "Unsaved draft")
		));
		fn count(elements: &[Element]) -> usize {
			elements
				.iter()
				.map(|e| {
					1 + match e {
						Element::Row { children } => count(children),
						_ => 0,
					}
				})
				.sum()
		}
		assert!(count(&invalid.output.panel) < 64);
	}
}
