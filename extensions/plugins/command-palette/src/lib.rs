/*
 * Serein bounded command palette
 * Copyright (c) 2026 Serein port contributors
 *
 * Safe port of Equicord's `commandPalette` plugin
 * (Copyright (c) 2026 Vendicated and contributors), licensed
 * GPL-3.0-or-later, at upstream commit
 * 200dba124066146a8f043de7fc8b05e3dc5e68e.
 * Changed November 8, 2026: the global hotkey, aliases, custom URL/message
 * commands, shell/eval, plugin management, Discord command execution and
 * generic URL fetching are removed. This is a typed panel launcher over the
 * existing app snapshot and Apply-reviewed navigation effects only.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppInvocation, AppOutput, AppSnapshot, AppView, ChannelSnapshot, Element, HostEffect,
	Invocation, MAX_IO_BYTES, Output, serde_json,
};
use std::collections::{BTreeMap, BTreeSet};

/// The panel query is a single bounded line, not an arbitrary command language.
const MAX_QUERY_BYTES: usize = 256;
/// The host directory can hold more; the panel shows only this many ranked rows.
const MAX_CANDIDATES: usize = 32;
/// Strict storage bound for pinned loaded-channel IDs.
const MAX_PINNED: usize = 16;
/// Keep every response within the unchanged ABI and panel budget.
const MAX_RESPONSE_BYTES: usize = MAX_IO_BYTES;
const MAX_PANEL_ELEMENTS: usize = 64;

// Panel value IDs are shared with the host's `valid_id` contract: lowercase
// ASCII letters, digits and hyphens, starting with a letter or digit, at most
// 64 bytes. Keep one spelling for the element and its `Invocation.values` key.
const QUERY_FIELD: &str = "query";
const CHANNEL_FIELD: &str = "channel";
const DESTINATION_FIELD: &str = "destination";
const SEARCH_FIELD: &str = "search-query";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Destination {
	Home,
	View(AppView),
}

/// Fixed native views only. No arbitrary route, URL, command or Discord action.
const DESTINATIONS: &[(&str, Destination)] = &[
	("Home (Friends)", Destination::Home),
	("Friends", Destination::View(AppView::Friends)),
	("Search", Destination::View(AppView::Search)),
	("Pins", Destination::View(AppView::Pins)),
	("Members", Destination::View(AppView::Members)),
	("Threads", Destination::View(AppView::Threads)),
	("Settings", Destination::View(AppView::Settings)),
	("Account", Destination::View(AppView::Account)),
	(
		"Profile settings",
		Destination::View(AppView::ProfileSettings),
	),
	("Appearance", Destination::View(AppView::Appearance)),
	(
		"Messaging permissions",
		Destination::View(AppView::MessagingPermissions),
	),
	("Notifications", Destination::View(AppView::Notifications)),
	("Activity", Destination::View(AppView::Activity)),
	("Keybinds", Destination::View(AppView::Keybinds)),
	("Storage", Destination::View(AppView::Storage)),
	("Updates", Destination::View(AppView::Updates)),
	("Extensions", Destination::View(AppView::Extensions)),
	("Themes", Destination::View(AppView::Themes)),
	("Voice settings", Destination::View(AppView::VoiceSettings)),
];

/// Storage contains only the pinned channel IDs written by this plugin. Any
/// other shape is corrupt and must survive untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SavedPins {
	Missing,
	Valid(Vec<String>),
	Corrupt,
}

#[derive(Clone, Copy, Debug)]
struct Candidate<'a> {
	channel: &'a ChannelSnapshot,
	guild: Option<&'a str>,
	pinned: bool,
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn button(id: &str, label: &str) -> Element {
	Element::Button {
		id: id.into(),
		label: label.into(),
	}
}

fn field(id: &str, label: &str, value: &str) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: value.into(),
	}
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

fn valid_entity_id(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 20
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

/// Mirror of the host's form/element ID contract. Underscores are not allowed.
fn valid_id(value: &str) -> bool {
	let bytes = value.as_bytes();
	!value.is_empty()
		&& bytes.len() <= 64
		&& bytes
			.iter()
			.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
		&& bytes[0].is_ascii_alphanumeric()
		&& !matches!(
			value,
			"con"
				| "prn" | "aux"
				| "nul" | "com1"
				| "com2" | "com3"
				| "com4" | "com5"
				| "com6" | "com7"
				| "com8" | "com9"
				| "lpt1" | "lpt2"
				| "lpt3" | "lpt4"
				| "lpt5" | "lpt6"
				| "lpt7" | "lpt8"
				| "lpt9"
		)
}

fn panel_ids_are_valid(panel: &[Element]) -> bool {
	fn walk(elements: &[Element], seen: &mut BTreeSet<String>) -> bool {
		for element in elements {
			match element {
				Element::Row { children } => {
					if !walk(children, seen) {
						return false;
					}
				}
				Element::Button { id, .. }
				| Element::TextInput { id, .. }
				| Element::Checkbox { id, .. }
				| Element::Select { id, .. }
				| Element::Slider { id, .. } => {
					if !valid_id(id) || !seen.insert(id.clone()) {
						return false;
					}
				}
				Element::Text { .. }
				| Element::Heading { .. }
				| Element::Separator
				| Element::ActivityPreview { .. } => {}
			}
		}
		true
	}
	walk(panel, &mut BTreeSet::new())
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
	if value.len() <= max_bytes {
		return value;
	}
	let mut end = max_bytes;
	while end > 0 && !value.is_char_boundary(end) {
		end -= 1;
	}
	&value[..end]
}

/// Bound the typed query without splitting a UTF-8 character. The returned flag
/// tells the panel that some input was ignored.
fn bounded_query(value: &str) -> (&str, bool) {
	let bounded = truncate_utf8(value, MAX_QUERY_BYTES);
	(bounded, bounded.len() != value.len())
}

fn sanitize_label(value: &str) -> String {
	let cleaned = value
		.chars()
		.map(|character| {
			if character.is_control() {
				' '
			} else {
				character
			}
		})
		.collect::<String>();
	let trimmed = cleaned.trim();
	if trimmed.is_empty() {
		"Unnamed".into()
	} else {
		trimmed.to_owned()
	}
}

fn parse_pins(invocation: &Invocation) -> SavedPins {
	match invocation.storage_json::<serde_json::Value>() {
		Ok(None) => SavedPins::Missing,
		Ok(Some(serde_json::Value::Object(object))) => {
			if object.len() != 1 {
				return SavedPins::Corrupt;
			}
			let Some(serde_json::Value::Array(items)) = object.get("pins") else {
				return SavedPins::Corrupt;
			};
			if items.len() > MAX_PINNED {
				return SavedPins::Corrupt;
			}
			let mut ids = Vec::with_capacity(items.len());
			let mut seen = BTreeSet::new();
			for item in items {
				let Some(id) = item.as_str() else {
					return SavedPins::Corrupt;
				};
				if !valid_entity_id(id) || !seen.insert(id) {
					return SavedPins::Corrupt;
				}
				ids.push(id.to_owned());
			}
			SavedPins::Valid(ids)
		}
		_ => SavedPins::Corrupt,
	}
}

fn read_pin_ids(pins: &SavedPins) -> &[String] {
	match pins {
		SavedPins::Valid(ids) => ids,
		SavedPins::Missing | SavedPins::Corrupt => &[],
	}
}

fn saved_storage(ids: &[String]) -> Result<String, serde_json::Error> {
	serde_json::to_string(&serde_json::json!({ "pins": ids }))
}

fn channel_option(channel: &ChannelSnapshot, guild: Option<&str>, pinned: bool) -> String {
	let suffix = format!(" [{}]", channel.id);
	let prefix = if pinned { "[pinned] " } else { "" };
	let mut body = format!("#{}", sanitize_label(&channel.name));
	if let Some(guild) = guild {
		body.push_str(" - ");
		body.push_str(&sanitize_label(guild));
	}
	let budget = 128usize.saturating_sub(prefix.len() + suffix.len());
	let body = truncate_utf8(&body, budget);
	format!("{prefix}{body}{suffix}")
}

/// Selected values arrive from a typed Select, but the host does not require
/// their text to still match a current option. Recover the ID suffix and always
/// re-check it against the current loaded directory before proposing anything.
fn selected_id(value: Option<&str>) -> Option<&str> {
	let value = value?.trim_end();
	let inner = value.strip_suffix(']')?;
	let open = inner.rfind('[')?;
	let id = &inner[open + 1..];
	valid_entity_id(id).then_some(id)
}

fn destination(value: Option<&str>) -> Option<(&'static str, Destination)> {
	let value = value?;
	DESTINATIONS
		.iter()
		.find(|(label, _)| *label == value)
		.map(|(label, destination)| (*label, *destination))
}

fn guild_names(app: Option<&AppSnapshot>) -> BTreeMap<&str, &str> {
	app.and_then(|app| app.guilds.as_ref())
		.map(|directory| {
			directory
				.items
				.iter()
				.map(|guild| (guild.id.as_str(), guild.name.as_str()))
				.collect()
		})
		.unwrap_or_default()
}

fn loaded_directory(
	app: Option<&AppSnapshot>,
) -> Option<&serein_extension_sdk::ChannelDirectorySnapshot> {
	app.and_then(|app| app.channels.as_ref())
}

fn loaded_ids(app: Option<&AppSnapshot>) -> BTreeSet<&str> {
	loaded_directory(app)
		.map(|directory| {
			directory
				.items
				.iter()
				.map(|channel| channel.id.as_str())
				.collect()
		})
		.unwrap_or_default()
}

fn candidates<'a>(
	app: Option<&'a AppSnapshot>,
	pins: &SavedPins,
	query: &str,
) -> (Vec<Candidate<'a>>, usize, bool) {
	let Some(directory) = loaded_directory(app) else {
		return (Vec::new(), 0, false);
	};
	let guilds = guild_names(app);
	let query = query.to_lowercase();
	let mut seen = BTreeSet::new();
	let mut matches = Vec::new();
	for channel in &directory.items {
		if !seen.insert(channel.id.as_str()) {
			continue;
		}
		let guild = channel
			.guild_id
			.as_deref()
			.and_then(|id| guilds.get(id).copied());
		if !query.is_empty() {
			let name = channel.name.to_lowercase();
			let guild_match = guild.is_some_and(|name| name.to_lowercase().contains(&query));
			if !name.contains(&query) && !channel.id.contains(&query) && !guild_match {
				continue;
			}
		}
		matches.push(Candidate {
			channel,
			guild,
			pinned: read_pin_ids(pins).iter().any(|id| id == &channel.id),
		});
	}
	matches.sort_by(|left, right| {
		right
			.pinned
			.cmp(&left.pinned)
			.then_with(|| {
				left.channel
					.name
					.to_lowercase()
					.cmp(&right.channel.name.to_lowercase())
			})
			.then_with(|| left.channel.name.cmp(&right.channel.name))
			.then_with(|| left.channel.id.cmp(&right.channel.id))
	});
	let total = matches.len();
	matches.truncate(MAX_CANDIDATES);
	(matches, total, directory.truncated)
}

fn context_line(app: Option<&AppSnapshot>) -> String {
	let Some(context) = app.and_then(|app| app.context.as_ref()) else {
		return "App context is unavailable. Channel filtering is offline; fixed destinations remain available."
			.into();
	};
	let account = context
		.user
		.as_ref()
		.map_or("unknown account", |user| user.name.as_str());
	let selected = context.channel.as_ref().map_or_else(
		|| "no conversation".into(),
		|channel| format!("#{} [{}]", sanitize_label(&channel.name), channel.id),
	);
	format!(
		"Account: {account}; selected: {selected}; connected: {}",
		context.connected
	)
}

fn read_line(app: Option<&AppSnapshot>) -> Option<String> {
	let read = app.and_then(|app| app.read_state.as_ref())?;
	let channel = read.channel_id.as_deref()?;
	let unread = read
		.unread
		.map_or("unknown", |unread| if unread { "unread" } else { "read" });
	Some(format!(
		"Read state for current channel [{channel}]: {unread}; mentions: {}.",
		read.mentions
	))
}

fn scope_line(app: Option<&AppSnapshot>, directory_truncated: bool) -> String {
	let (channels, guilds) = match app {
		Some(app) => (
			app.channels
				.as_ref()
				.map_or("unavailable".to_owned(), |directory| {
					format!(
						"{}{}",
						directory.items.len(),
						if directory.truncated {
							" (partial)"
						} else {
							""
						}
					)
				}),
			app.guilds
				.as_ref()
				.map_or("unavailable".to_owned(), |directory| {
					format!(
						"{}{}",
						directory.items.len(),
						if directory.truncated {
							" (partial)"
						} else {
							""
						}
					)
				}),
		),
		None => ("unavailable".into(), "unavailable".into()),
	};
	let extra = if directory_truncated {
		" The loaded channel directory is partial."
	} else {
		""
	};
	format!(
		"Loaded channels: {channels}; loaded servers: {guilds}. Only channels already in the readable navigation snapshot are searched; nothing is fetched.{extra}"
	)
}

fn pin_line(app: Option<&AppSnapshot>, pins: &SavedPins) -> String {
	match pins {
		SavedPins::Corrupt => {
			"Saved pins are unreadable and were preserved. A successful pin action replaces them."
				.into()
		}
		SavedPins::Missing => "Pinned channels: 0/16.".into(),
		SavedPins::Valid(ids) => {
			let loaded = loaded_ids(app);
			let usable = if loaded.is_empty() {
				0
			} else {
				ids.iter().filter(|id| loaded.contains(id.as_str())).count()
			};
			let stale = if loaded.is_empty() {
				0
			} else {
				ids.len() - usable
			};
			let stale_note = if stale > 0 {
				format!(
					" {stale} stored pin(s) are stale for the current loaded directory and are not offered."
				)
			} else {
				String::new()
			};
			if loaded.is_empty() {
				format!(
					"Pinned channel IDs stored: {}/{MAX_PINNED}. The loaded directory is unavailable, so no pin is usable.{stale_note}",
					ids.len()
				)
			} else {
				format!("Pinned channels loaded: {usable}/{MAX_PINNED}.{stale_note}")
			}
		}
	}
}

fn select_value(options: &[String], current: Option<&str>, fallback: &str) -> String {
	current
		.filter(|value| options.iter().any(|option| option == value))
		.map(str::to_owned)
		.or_else(|| options.first().cloned())
		.unwrap_or_else(|| fallback.to_owned())
}

fn render_panel(input: &AppInvocation, notice: Option<&str>) -> AppOutput {
	let app = input.app.as_ref();
	let pins = parse_pins(&input.invocation);
	let raw_query = input.invocation.value(QUERY_FIELD).unwrap_or("");
	let (query, query_truncated) = bounded_query(raw_query);
	let (matches, total, directory_truncated) = candidates(app, &pins, query);

	let mut panel = vec![
		heading("Command palette"),
		text(context_line(app)),
		text(scope_line(app, directory_truncated)),
		text(pin_line(app, &pins)),
		field(
			QUERY_FIELD,
			"Filter loaded channels (256-byte limit)",
			query,
		),
		button("filter", "Apply filter"),
	];
	if let Some(read) = read_line(app) {
		panel.push(text(read));
	}
	if query_truncated {
		panel.push(text(
			"Filter text is limited to 256 bytes; extra text was ignored.",
		));
	}
	if let Some(notice) = notice {
		panel.push(text(notice));
	}
	panel.push(Element::Separator);
	panel.push(heading("Loaded channels"));

	if loaded_directory(app).is_none() {
		panel.push(text(
			"No loaded channel directory is available. Choose a fixed destination below; this plugin does not fetch channels.",
		));
	} else if total == 0 {
		panel.push(text(if query.is_empty() {
			"No loaded channels are available in the current directory."
		} else {
			"No loaded channel matches this filter. Clear or change the query; this is not a server-wide search."
		}));
	} else {
		let summary = if total > matches.len() {
			format!(
				"Loaded matches: {total}; showing the first {} with pinned channels first.",
				matches.len()
			)
		} else {
			format!("Loaded matches: {total}.")
		};
		panel.push(text(summary));
		let options = matches
			.iter()
			.map(|candidate| channel_option(candidate.channel, candidate.guild, candidate.pinned))
			.collect::<Vec<_>>();
		panel.push(Element::Select {
			id: CHANNEL_FIELD.into(),
			label: "Loaded channel (max 32 matches)".into(),
			value: select_value(&options, input.invocation.value(CHANNEL_FIELD), ""),
			options,
		});
		panel.push(button(
			"navigate",
			"Propose opening selected loaded channel",
		));
		panel.push(button("pin-channel", "Pin selected loaded channel"));
		panel.push(button("unpin-channel", "Unpin selected channel"));
		panel.push(text(
			"Pins store only loaded channel IDs, at most 16. Opening a channel is a proposal; Serein navigates only after Apply.",
		));
	}

	panel.push(Element::Separator);
	panel.push(heading("Fixed native destinations"));
	let destination_options = DESTINATIONS
		.iter()
		.map(|(label, _)| (*label).to_owned())
		.collect::<Vec<_>>();
	panel.push(Element::Select {
		id: DESTINATION_FIELD.into(),
		label: "Native destination".into(),
		value: select_value(
			&destination_options,
			input.invocation.value(DESTINATION_FIELD),
			"Home (Friends)",
		),
		options: destination_options,
	});
	panel.push(button(
		"go-destination",
		"Propose selected native destination",
	));

	let (search, search_truncated) =
		bounded_query(input.invocation.value(SEARCH_FIELD).unwrap_or(""));
	panel.push(field(
		SEARCH_FIELD,
		"Search the current conversation (256-byte limit)",
		search,
	));
	panel.push(button("search", "Propose conversation search"));
	if search_truncated {
		panel.push(text(
			"Search text is limited to 256 bytes; shorten it before proposing a search.",
		));
	}
	panel.push(button("open-profile", "Propose opening your profile"));
	panel.push(text(
		"Every action here returns at most one typed proposal. Nothing navigates, searches or opens a profile until you choose Apply.",
	));

	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		..Default::default()
	}
}

fn proposal(title: &str, detail: String, effect: HostEffect) -> AppOutput {
	AppOutput {
		output: Output {
			panel: vec![
				heading(title),
				text(detail),
				text(
					"This is a proposal only. Serein applies it after you choose Apply and rechecks current access.",
				),
			],
			..Default::default()
		},
		effects: vec![effect],
	}
}

fn navigate(input: &AppInvocation) -> AppOutput {
	let Some(directory) = loaded_directory(input.app.as_ref()) else {
		return render_panel(
			input,
			Some("The loaded channel directory is unavailable. Open a conversation and try again."),
		);
	};
	let Some(id) = selected_id(input.invocation.value(CHANNEL_FIELD)) else {
		return render_panel(
			input,
			Some("Choose a channel from the current loaded channel list first."),
		);
	};
	let Some(channel) = directory.items.iter().find(|channel| channel.id == id) else {
		return render_panel(
			input,
			Some(
				"That channel is not in the currently loaded directory. Stale IDs after an account or channel switch are not navigated.",
			),
		);
	};
	proposal(
		"Open loaded channel",
		format!(
			"Proposed channel: #{} [{}]. This uses the ordinary native navigation path.",
			sanitize_label(&channel.name),
			channel.id
		),
		HostEffect::Navigate {
			channel_id: channel.id.clone(),
		},
	)
}

fn pin_channel(input: &AppInvocation) -> AppOutput {
	let Some(directory) = loaded_directory(input.app.as_ref()) else {
		return render_panel(
			input,
			Some("The loaded channel directory is unavailable; no pin was changed."),
		);
	};
	let Some(id) = selected_id(input.invocation.value(CHANNEL_FIELD)) else {
		return render_panel(
			input,
			Some("Choose a channel from the current loaded channel list first."),
		);
	};
	if !directory.items.iter().any(|channel| channel.id == id) {
		return render_panel(
			input,
			Some("That channel is not in the currently loaded directory; no pin was changed."),
		);
	}
	let pins = parse_pins(&input.invocation);
	let mut ids = match pins {
		SavedPins::Corrupt | SavedPins::Missing => Vec::new(),
		SavedPins::Valid(ids) => ids,
	};
	// Normalize old pins to the current loaded directory before enforcing the
	// bound, so stale IDs from another account or session cannot consume slots.
	let loaded = loaded_ids(input.app.as_ref());
	ids.retain(|id| loaded.contains(id.as_str()));
	if !ids.iter().any(|pinned| pinned == id) {
		if ids.len() >= MAX_PINNED {
			return render_panel(
				input,
				Some(
					"At most 16 loaded channels can be pinned. Unpin one before adding another; nothing was changed.",
				),
			);
		}
		ids.push(id.to_owned());
	}
	let Ok(storage) = saved_storage(&ids) else {
		return render_panel(
			input,
			Some("The pin could not be serialized; nothing was changed."),
		);
	};
	let mut next = input.clone();
	next.invocation.storage = Some(storage.clone());
	let mut output = render_panel(&next, Some("Pinned the selected loaded channel."));
	output.output.storage = Some(storage);
	output
}

fn unpin_channel(input: &AppInvocation) -> AppOutput {
	let Some(id) = selected_id(input.invocation.value(CHANNEL_FIELD)) else {
		return render_panel(
			input,
			Some("Choose a channel from the current loaded channel list first."),
		);
	};
	let pins = parse_pins(&input.invocation);
	let SavedPins::Valid(ids) = pins else {
		return render_panel(
			input,
			Some(match pins {
				SavedPins::Corrupt => {
					"Saved pins are unreadable and were preserved; no pin was removed."
				}
				_ => "There are no saved pins to remove.",
			}),
		);
	};
	let loaded = loaded_ids(input.app.as_ref());
	let next_ids = ids
		.into_iter()
		.filter(|pinned| pinned != id && loaded.contains(pinned.as_str()))
		.collect::<Vec<_>>();
	let Ok(storage) = saved_storage(&next_ids) else {
		return render_panel(
			input,
			Some("The pin change could not be serialized; nothing was changed."),
		);
	};
	let mut next = input.clone();
	next.invocation.storage = Some(storage.clone());
	let mut output = render_panel(&next, Some("Unpinned the selected channel."));
	output.output.storage = Some(storage);
	output
}

fn go_destination(input: &AppInvocation) -> AppOutput {
	let Some((label, destination)) = destination(input.invocation.value(DESTINATION_FIELD)) else {
		return render_panel(
			input,
			Some("Choose one of the fixed native destinations from the list."),
		);
	};
	let effect = match destination {
		Destination::Home => HostEffect::Home,
		Destination::View(view) => HostEffect::OpenView { view },
	};
	proposal(
		"Open native destination",
		format!("Proposed destination: {label}."),
		effect,
	)
}

fn search(input: &AppInvocation) -> AppOutput {
	let raw = input.invocation.value(SEARCH_FIELD).unwrap_or("");
	if raw.len() > MAX_QUERY_BYTES {
		return render_panel(
			input,
			Some("Search text is limited to 256 bytes; no search was proposed."),
		);
	}
	let query = raw.trim();
	if query.is_empty() || query.chars().any(char::is_control) {
		return render_panel(
			input,
			Some("Enter a search query of 1 to 256 bytes without control characters."),
		);
	}
	let has_conversation = input
		.app
		.as_ref()
		.and_then(|app| app.context.as_ref())
		.and_then(|context| context.channel.as_ref())
		.is_some();
	if !has_conversation {
		return render_panel(
			input,
			Some("Open a conversation before proposing a conversation search."),
		);
	}
	proposal(
		"Search current conversation",
		format!("Proposed search query: {query}"),
		HostEffect::Search {
			query: query.to_owned(),
		},
	)
}

fn open_profile(input: &AppInvocation) -> AppOutput {
	let Some(user) = input
		.app
		.as_ref()
		.and_then(|app| app.context.as_ref())
		.and_then(|context| context.user.as_ref())
	else {
		return render_panel(
			input,
			Some("Your account is not available in the current app context."),
		);
	};
	proposal(
		"Open your profile",
		format!(
			"Proposed profile: {} [{}].",
			sanitize_label(&user.name),
			user.id
		),
		HostEffect::OpenProfile {
			user_id: user.id.clone(),
		},
	)
}

fn bounded(mut output: AppOutput) -> AppOutput {
	debug_assert!(output.output.panel.len() <= MAX_PANEL_ELEMENTS);
	debug_assert!(
		panel_ids_are_valid(&output.output.panel),
		"panel form IDs must satisfy Serein valid_id and be unique"
	);
	let too_large =
		serde_json::to_vec(&output).map_or(true, |bytes| bytes.len() > MAX_RESPONSE_BYTES);
	if too_large {
		output.output.panel = vec![text(
			"The command palette response exceeded the 256 KiB limit, so the view was reduced. Nothing was applied.",
		)];
		output.effects.clear();
	}
	output
}

fn handle(input: AppInvocation) -> AppOutput {
	if input.app_event.is_some() || input.message_event.is_some() {
		return AppOutput::default();
	}
	let output = match input.invocation.action.as_str() {
		"show" | "filter" => render_panel(&input, None),
		"navigate" => navigate(&input),
		"pin-channel" => pin_channel(&input),
		"unpin-channel" => unpin_channel(&input),
		"go-destination" => go_destination(&input),
		"search" => search(&input),
		"open-profile" => open_profile(&input),
		_ => AppOutput::default(),
	};
	bounded(output)
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	fn run(input: serde_json::Value) -> AppOutput {
		let bytes = dispatch_typed(&serde_json::to_vec(&input).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
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

	fn select_options<'a>(output: &'a AppOutput, id: &str) -> &'a [String] {
		output
			.output
			.panel
			.iter()
			.find_map(|element| match element {
				Element::Select {
					id: current,
					options,
					..
				} if current == id => Some(options.as_slice()),
				_ => None,
			})
			.expect("select is present")
	}

	fn text_input<'a>(output: &'a AppOutput, id: &str) -> &'a str {
		output
			.output
			.panel
			.iter()
			.find_map(|element| match element {
				Element::TextInput {
					id: current, value, ..
				} if current == id => Some(value.as_str()),
				_ => None,
			})
			.expect("text input is present")
	}

	fn option_for(output: &AppOutput, id: &str) -> String {
		select_options(output, "channel")
			.iter()
			.find(|option| option.ends_with(&format!("[{id}]")))
			.unwrap_or_else(|| panic!("option for {id} is present"))
			.clone()
	}

	fn element_count(panel: &[Element]) -> usize {
		panel
			.iter()
			.map(|element| {
				1 + match element {
					Element::Row { children } => element_count(children),
					_ => 0,
				}
			})
			.sum()
	}

	fn base_app() -> serde_json::Value {
		serde_json::json!({
			"context": {
				"connected": true,
				"user": {"id": "1", "name": "Owner"},
				"channel": {"id": "10", "guild_id": "100", "name": "General", "kind": 0}
			},
			"guilds": {
				"items": [{"id": "100", "name": "Serein"}],
				"truncated": false
			},
			"channels": {
				"items": [
					{"id": "10", "guild_id": "100", "name": "General", "kind": 0},
					{"id": "11", "guild_id": "100", "name": "Random", "kind": 0},
					{"id": "20", "name": "Alice", "kind": 1}
				],
				"truncated": false
			},
			"read_state": {"channel_id": "10", "unread": true, "mentions": 2}
		})
	}

	#[test]
	fn filters_loaded_channels_and_prioritizes_pins() {
		let by_name = run(serde_json::json!({
			"action": "filter",
			"values": {"query": "GEN"},
			"app": base_app()
		}));
		let options = select_options(&by_name, "channel");
		assert!(options.iter().any(|option| option.ends_with("[10]")));
		assert!(!options.iter().any(|option| option.ends_with("[11]")));
		assert!(!options.iter().any(|option| option.ends_with("[20]")));
		assert!(by_name.effects.is_empty());
		assert!(by_name.output.storage.is_none());

		let by_guild = run(serde_json::json!({
			"action": "filter",
			"values": {"query": "serein"},
			"app": base_app()
		}));
		let options = select_options(&by_guild, "channel");
		assert_eq!(options.len(), 2);
		assert!(!options.iter().any(|option| option.ends_with("[20]")));

		let by_id = run(serde_json::json!({
			"action": "filter",
			"values": {"query": "11"},
			"app": base_app()
		}));
		let options = select_options(&by_id, "channel");
		assert_eq!(options.len(), 1);
		assert!(options[0].ends_with("[11]"));

		let pinned = run(serde_json::json!({
			"action": "show",
			"storage": "{\"pins\":[\"11\"]}",
			"app": base_app()
		}));
		let options = select_options(&pinned, "channel");
		assert!(options[0].starts_with("[pinned]"));
		assert!(options[0].ends_with("[11]"));
	}

	#[test]
	fn stale_channel_ids_are_not_offered_or_applied_after_a_switch() {
		let original = run(serde_json::json!({
			"action": "show",
			"app": base_app()
		}));
		let stale_option = option_for(&original, "10");

		let mut switched = base_app();
		switched["context"]["user"] = serde_json::json!({"id": "2", "name": "Other"});
		switched["context"]["channel"] =
			serde_json::json!({"id": "20", "name": "Other DM", "kind": 1});
		switched["guilds"]["items"] = serde_json::json!([]);
		switched["channels"]["items"] =
			serde_json::json!([{"id": "20", "name": "Other DM", "kind": 1}]);
		switched["read_state"] = serde_json::json!({
			"channel_id": "20", "unread": serde_json::Value::Null, "mentions": 0
		});

		let stale = run(serde_json::json!({
			"action": "navigate",
			"storage": "{\"pins\":[\"10\",\"11\"]}",
			"values": {"channel": stale_option},
			"app": switched.clone()
		}));
		assert!(stale.effects.is_empty());
		assert!(panel_text(&stale).contains("currently loaded directory"));
		assert!(stale.output.storage.is_none());

		let shown = run(serde_json::json!({
			"action": "show",
			"storage": "{\"pins\":[\"10\",\"11\"]}",
			"app": switched.clone()
		}));
		let options = select_options(&shown, "channel");
		assert_eq!(options.len(), 1);
		assert!(options[0].ends_with("[20]"));
		assert!(panel_text(&shown).contains("stale"));

		let current_option = option_for(&shown, "20");
		let fresh = run(serde_json::json!({
			"action": "navigate",
			"storage": "{\"pins\":[\"10\",\"11\"]}",
			"values": {"channel": current_option},
			"app": switched.clone()
		}));
		assert_eq!(
			fresh.effects,
			vec![HostEffect::Navigate {
				channel_id: "20".into()
			}]
		);
	}

	#[test]
	fn query_candidates_panel_and_search_stay_bounded() {
		let items = (0..100)
			.map(|index| {
				serde_json::json!({
					"id": format!("{}", 1000 + index),
					"name": format!("Channel {index}"),
					"kind": 0
				})
			})
			.collect::<Vec<_>>();
		let app = serde_json::json!({
			"context": {
				"connected": true,
				"user": {"id": "1", "name": "Owner"},
				"channel": {"id": "1000", "name": "Channel 0", "kind": 0}
			},
			"guilds": {"items": [], "truncated": false},
			"channels": {"items": items, "truncated": true},
			"read_state": {"channel_id": "1000", "unread": false, "mentions": 0}
		});

		let shown = run(serde_json::json!({"action": "show", "app": app}));
		let options = select_options(&shown, "channel");
		assert_eq!(options.len(), MAX_CANDIDATES);
		assert!(options.iter().all(|option| option.len() <= 128));
		assert!(
			panel_text(&shown).contains("Showing the first 32")
				|| panel_text(&shown).contains("showing the first 32")
		);
		assert!(element_count(&shown.output.panel) <= MAX_PANEL_ELEMENTS);
		assert!(serde_json::to_vec(&shown).unwrap().len() <= MAX_RESPONSE_BYTES);
		for element in &shown.output.panel {
			if let Element::Text { text } = element {
				assert!(text.len() <= 4096);
			}
		}

		let long = "界".repeat(200);
		let truncated = run(serde_json::json!({
			"action": "filter",
			"values": {"query": long},
			"app": app.clone()
		}));
		assert!(text_input(&truncated, "query").len() <= MAX_QUERY_BYTES);
		assert!(text_input(&truncated, "query").is_char_boundary(0));
		assert!(panel_text(&truncated).contains("limited to 256 bytes"));
		assert!(serde_json::to_vec(&truncated).unwrap().len() <= MAX_RESPONSE_BYTES);

		let search = run(serde_json::json!({
			"action": "search",
			"values": {"search-query": "x".repeat(300)},
			"app": app.clone()
		}));
		assert!(search.effects.is_empty());
		assert!(panel_text(&search).contains("limited to 256 bytes"));
	}

	fn many_channels_app() -> serde_json::Value {
		let items = (0..20)
			.map(|index| {
				serde_json::json!({
					"id": format!("{}", 1000 + index),
					"name": format!("Channel {index}"),
					"guild_id": "100",
					"kind": 0
				})
			})
			.collect::<Vec<_>>();
		serde_json::json!({
			"context": {
				"connected": true,
				"user": {"id": "1", "name": "Owner"},
				"channel": {"id": "1000", "guild_id": "100", "name": "Channel 0", "kind": 0}
			},
			"guilds": {"items": [{"id": "100", "name": "Serein"}], "truncated": false},
			"channels": {"items": items, "truncated": false},
			"read_state": {"channel_id": "1000", "unread": true, "mentions": 1}
		})
	}

	#[test]
	fn pin_storage_is_bounded_and_corrupt_state_is_preserved() {
		let app = many_channels_app();
		let full = (0..16)
			.map(|index| format!("\"{}\"", 1000 + index))
			.collect::<Vec<_>>()
			.join(",");
		let full_storage = format!("{{\"pins\":[{full}]}}");

		let ready = run(serde_json::json!({
			"action": "show",
			"storage": full_storage.clone(),
			"app": app.clone()
		}));
		let over_limit_option = option_for(&ready, "1016");
		let rejected = run(serde_json::json!({
			"action": "pin-channel",
			"storage": full_storage.clone(),
			"values": {"channel": over_limit_option},
			"app": app.clone()
		}));
		assert!(rejected.output.storage.is_none());
		assert!(panel_text(&rejected).contains("At most 16"));
		assert!(rejected.effects.is_empty());

		let existing_option = option_for(&ready, "1001");
		let existing = run(serde_json::json!({
			"action": "pin-channel",
			"storage": full_storage.clone(),
			"values": {"channel": existing_option},
			"app": app.clone()
		}));
		let saved: serde_json::Value =
			serde_json::from_str(existing.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(saved["pins"].as_array().unwrap().len(), MAX_PINNED);

		let corrupt = run(serde_json::json!({
			"action": "show",
			"storage": "{\"pins\":[\"bad\",\"1000\"]}",
			"app": app.clone()
		}));
		assert!(panel_text(&corrupt).contains("unreadable and were preserved"));
		assert!(corrupt.output.storage.is_none());

		let overfull = (0..17)
			.map(|index| format!("\"{}\"", 1000 + index))
			.collect::<Vec<_>>()
			.join(",");
		let overfull = run(serde_json::json!({
			"action": "show",
			"storage": format!("{{\"pins\":[{overfull}]}}"),
			"app": app.clone()
		}));
		assert!(panel_text(&overfull).contains("unreadable and were preserved"));
		assert!(overfull.output.storage.is_none());

		let repair_option = option_for(&corrupt, "1016");
		let repaired = run(serde_json::json!({
			"action": "pin-channel",
			"storage": "not-json",
			"values": {"channel": repair_option},
			"app": app.clone()
		}));
		let saved: serde_json::Value =
			serde_json::from_str(repaired.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(saved["pins"], serde_json::json!(["1016"]));

		let unpin_option = option_for(&ready, "1000");
		let unpinned = run(serde_json::json!({
			"action": "unpin-channel",
			"storage": full_storage.clone(),
			"values": {"channel": unpin_option},
			"app": app.clone()
		}));
		let saved: serde_json::Value =
			serde_json::from_str(unpinned.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(saved["pins"].as_array().unwrap().len(), MAX_PINNED - 1);
		assert!(
			!saved["pins"]
				.as_array()
				.unwrap()
				.iter()
				.any(|id| id == "1000")
		);
	}

	#[test]
	fn every_action_proposes_at_most_one_validated_effect() {
		let app = base_app();
		let shown = run(serde_json::json!({"action": "show", "app": app}));
		let channel_option = option_for(&shown, "10");

		let navigation = run(serde_json::json!({
			"action": "navigate",
			"values": {"channel": channel_option},
			"app": app.clone()
		}));
		assert_eq!(
			navigation.effects,
			vec![HostEffect::Navigate {
				channel_id: "10".into()
			}]
		);
		assert!(panel_text(&navigation).contains("choose Apply"));

		for (label, expected) in [
			("Home (Friends)", HostEffect::Home),
			(
				"Appearance",
				HostEffect::OpenView {
					view: AppView::Appearance,
				},
			),
			(
				"Search",
				HostEffect::OpenView {
					view: AppView::Search,
				},
			),
		] {
			let output = run(serde_json::json!({
				"action": "go-destination",
				"values": {"destination": label},
				"app": app.clone()
			}));
			assert_eq!(output.effects, vec![expected]);
		}

		let profile = run(serde_json::json!({
			"action": "open-profile",
			"app": app.clone()
		}));
		assert_eq!(
			profile.effects,
			vec![HostEffect::OpenProfile {
				user_id: "1".into()
			}]
		);

		let search = run(serde_json::json!({
			"action": "search",
			"values": {"search-query": "synthetic query"},
			"app": app.clone()
		}));
		assert_eq!(
			search.effects,
			vec![HostEffect::Search {
				query: "synthetic query".into()
			}]
		);

		for output in [
			run(serde_json::json!({
				"action": "go-destination",
				"values": {"destination": "https://example.invalid"},
				"app": app.clone()
			})),
			run(serde_json::json!({
				"action": "search",
				"values": {"search-query": "   "},
				"app": app.clone()
			})),
			run(serde_json::json!({
				"action": "navigate",
				"values": {"channel": "#General - Serein [99]"},
				"app": app.clone()
			})),
			run(serde_json::json!({
				"action": "open-profile",
				"app": {"context": {"connected": true}}
			})),
		] {
			assert!(output.effects.is_empty());
		}

		for output in [&navigation, &profile, &search] {
			assert!(output.effects.len() <= 1);
			assert!(matches!(
				output.effects.first(),
				Some(
					HostEffect::Navigate { .. }
						| HostEffect::Home | HostEffect::OpenView { .. }
						| HostEffect::OpenProfile { .. }
						| HostEffect::Search { .. }
				)
			));
		}
	}

	#[test]
	fn panel_form_ids_match_host_valid_id_and_manifest_actions() {
		for id in [QUERY_FIELD, CHANNEL_FIELD, DESTINATION_FIELD, SEARCH_FIELD] {
			assert!(valid_id(id), "{id} must satisfy host valid_id");
		}
		assert!(!valid_id("search_query"));
		assert!(!valid_id("Query"));

		let manifest: serde_json::Value =
			serde_json::from_str(include_str!("../manifest.json")).unwrap();
		assert_eq!(manifest["capabilities"].as_array().unwrap().len(), 6);
		let actions = manifest["actions"]
			.as_array()
			.unwrap()
			.iter()
			.map(|action| {
				assert_eq!(action["surface"], "panel");
				let id = action["id"].as_str().unwrap();
				assert!(valid_id(id), "{id} must satisfy host valid_id");
				id.to_owned()
			})
			.collect::<BTreeSet<_>>();
		for id in [
			"show",
			"filter",
			"navigate",
			"pin-channel",
			"unpin-channel",
			"go-destination",
			"search",
			"open-profile",
		] {
			assert!(actions.contains(id));
		}

		fn check(elements: &[Element], actions: &BTreeSet<String>) {
			for element in elements {
				match element {
					Element::Row { children } => check(children, actions),
					Element::Button { id, .. } => {
						assert!(valid_id(id));
						assert!(actions.contains(id), "button {id} needs a panel action");
					}
					Element::TextInput { id, .. }
					| Element::Checkbox { id, .. }
					| Element::Slider { id, .. } => assert!(valid_id(id)),
					Element::Select {
						id, options, value, ..
					} => {
						assert!(valid_id(id));
						assert!(!options.is_empty() && options.len() <= 32);
						assert!(options.contains(value));
					}
					Element::Text { .. }
					| Element::Heading { .. }
					| Element::Separator
					| Element::ActivityPreview { .. } => {}
				}
			}
		}

		let shown = run(serde_json::json!({"action": "show", "app": base_app()}));
		assert!(panel_ids_are_valid(&shown.output.panel));
		check(&shown.output.panel, &actions);

		let pinned = run(serde_json::json!({
			"action": "pin-channel",
			"values": {"channel": option_for(&shown, "10")},
			"app": base_app()
		}));
		assert!(panel_ids_are_valid(&pinned.output.panel));
		check(&pinned.output.panel, &actions);
	}
}
