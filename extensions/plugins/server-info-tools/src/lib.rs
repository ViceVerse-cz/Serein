/*
 * Serein server info tools
 * Copyright (c) 2026 Serein port contributors
 *
 * Safe, read-only subset of Equicord's `serverInfo` plugin
 * (Copyright (c) 2023 Vendicated, Nuckyz, EquicordDevs.Z1xus and
 * contributors), licensed GPL-3.0-or-later, at upstream commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e.
 * Changed November 8, 2026: only already-loaded guild, channel and
 * relationship snapshots are shown. There is no guild member request, no
 * Webpack/store access, no profile or mutual-guild fetch, and no role, owner,
 * boost or vanity lookup that the local snapshot does not contain. Server
 * creation time is derived from the guild snowflake, never fetched.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppInvocation, AppOutput, AppSnapshot, Element, RelationshipKind, serde_json,
};
use std::collections::BTreeSet;

/// Keep the complete response well below the 256 KiB ABI limit.
const MAX_RESPONSE_BYTES: usize = 8 * 1024;
/// The host can load up to 100 guilds; the picker stays smaller and readable.
const MAX_GUILD_OPTIONS: usize = 20;
const MAX_SELECT_OPTION_BYTES: usize = 128;
const MAX_DETAIL_ROWS: usize = 24;
const GUILD_FIELD: &str = "guild";

/// Discord snowflake epoch and bit layout. The timestamp is part of the public ID.
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;
const SNOWFLAKE_TIMESTAMP_SHIFT: u64 = 22;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SavedGuild {
	Missing,
	Valid,
	Corrupt,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ChannelCounts {
	text: usize,
	voice: usize,
	category: usize,
	forum: usize,
	thread: usize,
	other: usize,
	total: usize,
}

fn valid_id(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 20
		&& !value.starts_with('0')
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn saved_guild(invocation: &serein_extension_sdk::Invocation) -> (SavedGuild, Option<String>) {
	match invocation.storage_json::<serde_json::Value>() {
		Ok(None) => (SavedGuild::Missing, None),
		Ok(Some(value)) => {
			let guild_id = value
				.as_object()
				.filter(|object| object.len() == 1)
				.and_then(|object| object.get("guild_id"))
				.and_then(|value| value.as_str());
			match guild_id {
				Some(guild_id) if valid_id(guild_id) => {
					(SavedGuild::Valid, Some(guild_id.to_owned()))
				}
				_ => (SavedGuild::Corrupt, None),
			}
		}
		Err(_) => (SavedGuild::Corrupt, None),
	}
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

fn option_label(name: &str, id: &str) -> String {
	let suffix = format!(" [{id}]");
	let budget = MAX_SELECT_OPTION_BYTES.saturating_sub(suffix.len());
	let mut label = sanitize_label(name);
	label = truncate_utf8(&label, budget).to_owned();
	label.push_str(&suffix);
	label
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

/// `(option label, guild id)` in deterministic display order.
fn guild_options(app: &AppSnapshot) -> Vec<(String, String)> {
	let Some(directory) = app.guilds.as_ref() else {
		return Vec::new();
	};
	let mut seen = BTreeSet::new();
	let mut guilds = directory
		.items
		.iter()
		.filter(|guild| valid_id(&guild.id) && seen.insert(guild.id.as_str()))
		.collect::<Vec<_>>();
	guilds.sort_by(|left, right| {
		let left_name = sanitize_label(&left.name).to_lowercase();
		let right_name = sanitize_label(&right.name).to_lowercase();
		left_name
			.cmp(&right_name)
			.then_with(|| left.name.cmp(&right.name))
			.then_with(|| left.id.cmp(&right.id))
	});
	guilds.truncate(MAX_GUILD_OPTIONS);
	guilds
		.into_iter()
		.map(|guild| (option_label(&guild.name, &guild.id), guild.id.clone()))
		.collect()
}

fn option_for(options: &[(String, String)], guild_id: &str) -> Option<String> {
	options
		.iter()
		.find(|(_, id)| id == guild_id)
		.map(|(label, _)| label.clone())
}

fn selected_guild<'a>(
	app: &'a AppSnapshot,
	guild_id: &str,
) -> Option<&'a serein_extension_sdk::GuildSnapshot> {
	app.guilds
		.as_ref()?
		.items
		.iter()
		.find(|guild| guild.id == guild_id && valid_id(&guild.id))
}

fn current_channel_guild(app: &AppSnapshot) -> Option<&str> {
	app.context
		.as_ref()?
		.channel
		.as_ref()?
		.guild_id
		.as_deref()
		.filter(|id| valid_id(id))
}

fn channel_counts(app: &AppSnapshot, guild_id: &str) -> (ChannelCounts, bool) {
	let Some(directory) = app.channels.as_ref() else {
		return (ChannelCounts::default(), false);
	};
	let mut counts = ChannelCounts::default();
	let mut seen = BTreeSet::new();
	for channel in directory
		.items
		.iter()
		.filter(|channel| channel.guild_id.as_deref() == Some(guild_id))
		.filter(|channel| seen.insert(channel.id.as_str()))
	{
		counts.total += 1;
		match channel.kind {
			0 | 5 => counts.text += 1,
			2 | 13 => counts.voice += 1,
			4 => counts.category += 1,
			15 | 16 => counts.forum += 1,
			10..=12 => counts.thread += 1,
			_ => counts.other += 1,
		}
	}
	(counts, directory.truncated)
}

/// Proleptic Gregorian date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
	let shifted = days + 719_468;
	let era = (if shifted >= 0 {
		shifted
	} else {
		shifted - 146_096
	}) / 146_097;
	let day_of_era = shifted - era * 146_097;
	let year_of_era =
		(day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
	let mut year = year_of_era + era * 400;
	let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
	let month_prime = (5 * day_of_year + 2) / 153;
	let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
	let month = if month_prime < 10 {
		month_prime + 3
	} else {
		month_prime - 9
	};
	if month <= 2 {
		year += 1;
	}
	(year, month as u32, day as u32)
}

/// A readable UTC calendar date from an already-valid guild snowflake.
fn snowflake_date(guild_id: &str) -> Option<String> {
	if !valid_id(guild_id) {
		return None;
	}
	let snowflake = guild_id.parse::<u64>().ok()?;
	let timestamp = i64::try_from(snowflake >> SNOWFLAKE_TIMESTAMP_SHIFT).ok()?;
	let millis = timestamp.checked_add(DISCORD_EPOCH_MS)?;
	let days = millis.div_euclid(86_400_000);
	let (year, month, day) = civil_from_days(days);
	Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn relationship_line(app: &AppSnapshot) -> Option<String> {
	let relationships = app.relationships.as_ref()?;
	let mut friends = 0usize;
	let mut incoming = 0usize;
	let mut outgoing = 0usize;
	let mut blocked = 0usize;
	let mut ignored = 0usize;
	for item in &relationships.items {
		match item.kind {
			RelationshipKind::Friend => friends += 1,
			RelationshipKind::IncomingRequest => incoming += 1,
			RelationshipKind::OutgoingRequest => outgoing += 1,
			RelationshipKind::Blocked => blocked += 1,
			RelationshipKind::Ignored => ignored += 1,
		}
	}
	let mut line = format!(
		"Loaded account relationships: {friends} friends, {incoming} incoming requests, {outgoing} outgoing requests, {blocked} blocked, {ignored} ignored."
	);
	if relationships.truncated
		|| !relationships.friends_known
		|| !relationships.requests_known
		|| !relationships.restricted_known
	{
		line.push_str(" This relationship snapshot is partial.");
	}
	line.push_str(
		" These are account-wide loaded relationships, not a check that they are members of this server.",
	);
	Some(line)
}

fn summary_rows(app: &AppSnapshot, guild_id: &str) -> Vec<Element> {
	let mut rows = Vec::new();
	let Some(guild) = selected_guild(app, guild_id) else {
		rows.push(text(
			"The selected server is no longer in the loaded server directory.",
		));
		return rows;
	};
	rows.push(Element::Heading {
		text: sanitize_label(&guild.name),
	});
	rows.push(text(format!("Loaded server ID: {}", guild.id)));
	if let Some(date) = snowflake_date(&guild.id) {
		rows.push(text(format!(
			"Created (UTC, derived from the server ID): {date}"
		)));
	}
	match app.channels.as_ref() {
		Some(_) => {
			let (counts, truncated) = channel_counts(app, guild_id);
			rows.push(text(format!(
				"Loaded channels: {} (text {}, voice {}, categories {}, forums {}, threads {}, other {}).",
				counts.total,
				counts.text,
				counts.voice,
				counts.category,
				counts.forum,
				counts.thread,
				counts.other
			)));
			if truncated {
				rows.push(text(
					"The loaded channel directory is partial; counts cover loaded channels only.",
				));
			}
		}
		None => rows.push(text(
			"The loaded channel directory is unavailable for this summary.",
		)),
	}
	if let Some(line) = relationship_line(app) {
		rows.push(text(line));
	}
	rows
}

fn selector(app: &AppSnapshot, selected: Option<&str>) -> Option<(Element, Element)> {
	let options = guild_options(app);
	if options.is_empty() {
		return None;
	}
	let labels = options
		.iter()
		.map(|(label, _)| label.clone())
		.collect::<Vec<_>>();
	let value = selected
		.and_then(|id| option_for(&options, id))
		.or_else(|| labels.first().cloned())
		.unwrap_or_default();
	Some((
		Element::Select {
			id: GUILD_FIELD.into(),
			label: "Loaded server".into(),
			options: labels,
			value,
		},
		Element::Button {
			id: "select-guild".into(),
			label: "Show this loaded server".into(),
		},
	))
}

fn panel(app: &AppSnapshot, guild_id: Option<&str>, notice: Option<&str>) -> Vec<Element> {
	let mut panel = vec![Element::Heading {
		text: "Server Info (loaded data only)".into(),
	}];
	panel.push(text(
		"Serein shows only already-loaded server, channel and relationship data. It does not fetch members, roles, owner, boosts or vanity information.",
	));
	if let Some(guild_id) = guild_id {
		panel.extend(
			summary_rows(app, guild_id)
				.into_iter()
				.take(MAX_DETAIL_ROWS),
		);
	} else {
		panel.push(text(
			"No loaded server is selected. Choose one of the loaded servers below.",
		));
	}
	if let Some(notice) = notice {
		panel.push(text(notice));
	}
	if let Some((select, button)) = selector(app, guild_id) {
		panel.push(select);
		panel.push(button);
	} else {
		panel.push(text(
			"No loaded server directory is available. Open a connected account with loaded servers and try again.",
		));
	}
	panel
}

fn chosen_guild(app: &AppSnapshot, selected: &str) -> Option<String> {
	guild_options(app)
		.into_iter()
		.find(|(label, _)| label == selected)
		.map(|(_, id)| id)
}

fn show(input: &AppInvocation) -> AppOutput {
	let Some(app) = input.app.as_ref() else {
		return AppOutput {
			output: serein_extension_sdk::Output {
				panel: vec![text(
					"Loaded server data is unavailable. Connect an account and try again.",
				)],
				..Default::default()
			},
			..Default::default()
		};
	};
	let (saved, saved_id) = saved_guild(&input.invocation);
	let options = guild_options(app);
	let current = current_channel_guild(app);
	let selected = saved_id
		.as_deref()
		.filter(|id| option_for(&options, id).is_some())
		.or_else(|| current.filter(|id| option_for(&options, id).is_some()))
		.or_else(|| options.first().map(|(_, id)| id.as_str()))
		.map(str::to_owned);
	let notice = match saved {
		SavedGuild::Corrupt => Some(
			"Saved server selection is unreadable and was left unchanged. Choose a loaded server to replace it.",
		),
		SavedGuild::Valid if selected.as_deref() != saved_id.as_deref() => Some(
			"The saved server is no longer in the loaded directory; showing a loaded fallback without changing storage.",
		),
		_ => None,
	};
	AppOutput {
		output: serein_extension_sdk::Output {
			panel: panel(app, selected.as_deref(), notice),
			..Default::default()
		},
		..Default::default()
	}
}

fn select_guild(input: &AppInvocation) -> AppOutput {
	let Some(app) = input.app.as_ref() else {
		return show(input);
	};
	let (_saved, saved_id) = saved_guild(&input.invocation);
	let Some(chosen) = input
		.invocation
		.value(GUILD_FIELD)
		.and_then(|label| chosen_guild(app, label))
	else {
		let selected = saved_id
			.as_deref()
			.or_else(|| current_channel_guild(app))
			.map(str::to_owned)
			.or_else(|| guild_options(app).into_iter().next().map(|(_, id)| id));
		return AppOutput {
			output: serein_extension_sdk::Output {
				panel: panel(
					app,
					selected.as_deref(),
					Some(
						"That server is not in the current loaded directory. Storage was not changed.",
					),
				),
				..Default::default()
			},
			..Default::default()
		};
	};
	let mut output = AppOutput {
		output: serein_extension_sdk::Output {
			panel: panel(
				app,
				Some(chosen.as_str()),
				Some("Saved this loaded server selection."),
			),
			..Default::default()
		},
		..Default::default()
	};
	let _ = output
		.output
		.set_storage_json(&serde_json::json!({ "guild_id": chosen }));
	output
}

fn bounded(mut output: AppOutput) -> AppOutput {
	let too_large =
		serde_json::to_vec(&output).map_or(true, |bytes| bytes.len() > MAX_RESPONSE_BYTES);
	if too_large {
		output.output.panel = vec![text(
			"The server info response exceeded its 8 KiB panel budget, so the view was reduced. Nothing was applied.",
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
		"show" => show(&input),
		"select-guild" => select_guild(&input),
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
				Element::Heading { text } | Element::Text { text } => Some(text.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n")
	}

	fn select_options<'a>(output: &'a AppOutput, id: &str) -> Vec<&'a str> {
		output
			.output
			.panel
			.iter()
			.find_map(|element| match element {
				Element::Select {
					id: field, options, ..
				} if field == id => Some(options.iter().map(String::as_str).collect()),
				_ => None,
			})
			.unwrap_or_default()
	}

	#[test]
	fn snowflake_dates_are_derived_and_invalid_ids_are_rejected() {
		assert_eq!(
			snowflake_date("1191168914227200000").as_deref(),
			Some("2024-01-01")
		);
		assert!(snowflake_date("0").is_none());
		assert!(snowflake_date("0002").is_none());
		assert!(snowflake_date("not-a-snowflake").is_none());
	}

	#[test]
	fn summary_filters_loaded_channels_by_the_selected_server() {
		let app = serde_json::json!({
			"guilds": {
				"items": [
					{"id": "20", "name": "Beta"},
					{"id": "10", "name": "Alpha"}
				],
				"truncated": false
			},
			"channels": {
				"items": [
					{"id": "1001", "guild_id": "10", "name": "general", "kind": 0},
					{"id": "1002", "guild_id": "10", "name": "voice", "kind": 2},
					{"id": "1003", "guild_id": "10", "name": "category", "kind": 4},
					{"id": "1004", "guild_id": "20", "name": "beta-chat", "kind": 0},
					{"id": "1005", "name": "direct-message", "kind": 1}
				],
				"truncated": false
			},
			"relationships": {
				"items": [
					{"user": {"id": "90", "name": "Friend"}, "kind": "friend"}
				],
				"truncated": false,
				"friends_known": true,
				"requests_known": true,
				"restricted_known": true
			}
		});
		let shown = run(serde_json::json!({
			"action": "show",
			"storage": "{\"guild_id\":\"10\"}",
			"app": app
		}));
		let text = panel_text(&shown);
		assert!(text.contains("Alpha"));
		assert!(text.contains("Loaded channels: 3 (text 1, voice 1, categories 1"));
		assert!(!text.contains("beta-chat"));
		assert!(!text.contains("direct-message"));
		assert!(text.contains("not a check that they are members of this server"));
		assert!(shown.effects.is_empty());

		let options = select_options(&shown, GUILD_FIELD);
		assert_eq!(options, vec!["Alpha [10]", "Beta [20]"]);
	}

	#[test]
	fn unknown_or_corrupt_selection_never_overwrites_storage() {
		let app = serde_json::json!({
			"guilds": {
				"items": [{"id": "10", "name": "Alpha"}],
				"truncated": false
			},
			"channels": {"items": [], "truncated": false}
		});
		let rejected = run(serde_json::json!({
			"action": "select-guild",
			"storage": "{\"guild_id\":\"10\"}",
			"values": {"guild": "Forged [99]"},
			"app": app
		}));
		assert!(rejected.output.storage.is_none());
		assert!(rejected.effects.is_empty());
		assert!(panel_text(&rejected).contains("Storage was not changed"));

		let corrupt = run(serde_json::json!({
			"action": "show",
			"storage": "{broken",
			"app": app
		}));
		assert!(corrupt.output.storage.is_none());
		assert!(panel_text(&corrupt).contains("left unchanged"));
		assert!(select_options(&corrupt, GUILD_FIELD).contains(&"Alpha [10]"));
	}

	#[test]
	fn guild_options_are_unique_and_bounded() {
		let items = (1..=30)
			.map(|index| {
				serde_json::json!({
					"id": format!("{}", 1000 + index),
					"name": format!("Server {index:02}")
				})
			})
			.collect::<Vec<_>>();
		let app = serde_json::json!({
			"guilds": {"items": items, "truncated": true},
			"channels": {"items": [], "truncated": false}
		});
		let shown = run(serde_json::json!({"action": "show", "app": app}));
		assert_eq!(select_options(&shown, GUILD_FIELD).len(), MAX_GUILD_OPTIONS);
		assert!(serde_json::to_vec(&shown).unwrap().len() <= MAX_RESPONSE_BYTES);
		let first = select_options(&shown, GUILD_FIELD)[0].to_owned();
		let saved = run(serde_json::json!({
			"action": "select-guild",
			"values": {"guild": first},
			"app": app
		}));
		assert_eq!(
			saved.output.storage.as_deref(),
			Some("{\"guild_id\":\"1001\"}")
		);
	}
}
