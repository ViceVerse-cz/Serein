/*
 * Safe Serein adaptations of Equicord/Vencord plugins pinned at
 * Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e:
 *
 * richMagnetLinks (src/equicordplugins/richMagnetLinks/index.tsx)
 * Copyright (c) 2025 Vendicated and contributors
 *
 * showRolesInChat (src/equicordplugins/showRolesInChat/index.tsx)
 * Copyright (c) 2026 Vendicated and contributors
 *
 * Copyright (c) 2026 Serein port contributors
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License as published by the
 * Free Software Foundation, either version 3 of the License, or (at your
 * option) any later version.
 *
 * This program is distributed in the hope that it will be useful, but WITHOUT
 * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
 * FITNESS FOR A PARTICULAR PURPOSE.  See the GNU General Public License for
 * more details.
 *
 * You should have received a copy of the GNU General Public License along with
 * this program.  If not, see <https://www.gnu.org/licenses/gpl-3.0.html>.
 *
 * Changed November 8, 2026: neither upstream renderer can be reproduced through
 * the sandboxed extension ABI. This package offers two bounded, read-only
 * inspections instead: the selected author's highest loaded guild role (with
 * locally stored role exclusions) and the display names of magnet links in the
 * selected message, copied only after Apply. There is no inline decoration, no
 * arbitrary link rendering, no network access, no media handling and no
 * automatic account action.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */
use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	AppOutput, Element, HostEffect, Invocation, MessageActionInvocation, Output, SelectedMessageRef,
};
use std::collections::BTreeSet;

/// The host's canonical snowflake ceiling.
const MAX_ID_BYTES: usize = 20;
/// A local preference list, not a guild-wide role editor.
const MAX_EXCLUDED_ROLES: usize = 64;
/// Copied magnet text must fit the host's clipboard effect limit.
const MAX_CLIPBOARD_BYTES: usize = 4_096;
/// One selected message may contain only this many magnet links.
const MAX_MAGNETS: usize = 8;
/// A single link token is bounded before any parsing.
const MAX_MAGNET_BYTES: usize = 2_048;
/// The displayed filename is user data from the link, not a label we control.
const MAX_NAME_CHARS: usize = 200;

const STORAGE_INVALID: &str =
	"Saved role exclusions are invalid. Open Message Inspector Tools and save them again.";
const STORAGE_LIMIT: &str =
	"The role exclusions exceed the 64 KiB storage limit. Nothing was saved.";
const EXCLUSIONS_LIMIT: &str = "Use at most 64 comma-, space- or newline-separated role IDs, each a nonzero snowflake. Nothing was saved.";
const MISSING_SELECTION: &str =
	"The host did not provide the selected message IDs, so nothing was inspected.";
const STALE_SELECTION: &str =
	"The selected message no longer matches its loaded context. Reopen the message and try again.";
const ROLE_NO_GUILD: &str = "Highest loaded role is unavailable in direct and group messages.";
const ROLE_NO_SNAPSHOT: &str =
	"The host did not provide loaded member details for this conversation. Nothing was inspected.";
const ROLE_STALE_SNAPSHOT: &str =
	"The loaded member details belong to another conversation. Reopen the message and try again.";
const ROLE_NO_MEMBER: &str =
	"The author is not in the loaded member pane, so their roles are unknown. Nothing was guessed.";
const ROLE_NO_CATALOG: &str =
	"The guild role catalog is not loaded, so no role name or position can be shown.";
const ROLE_NONE: &str =
	"The author has no eligible loaded role, or every loaded role is excluded in settings.";
const NO_MAGNETS: &str = "No magnet links were found in the selected message. Nothing was copied.";
const TOO_MANY_MAGNETS: &str =
	"More than 8 magnet links were found. Nothing was copied; split the message and retry.";
const LONG_MAGNET: &str =
	"A magnet link exceeds the 2,048-UTF-8-byte inspection limit. Nothing was copied.";
const MAGNET_CONTROL: &str = "A magnet link contains control characters. Nothing was copied.";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Saved {
	excluded_roles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Magnet {
	link: String,
	name: String,
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

fn panel_output(panel: Vec<Element>, effect: Option<HostEffect>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		effects: effect.into_iter().collect(),
	}
}

fn error_output(title: &str, message: &str) -> AppOutput {
	panel_output(vec![heading(title), text(message)], None)
}

fn valid_snowflake(value: &str) -> bool {
	let bytes = value.as_bytes();
	!bytes.is_empty()
		&& bytes.len() <= MAX_ID_BYTES
		&& bytes[0] != b'0'
		&& bytes.iter().all(u8::is_ascii_digit)
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn selected_reference(
	input: &MessageActionInvocation,
) -> Result<&SelectedMessageRef, &'static str> {
	let reference = input
		.selected_message_ref
		.as_ref()
		.ok_or(MISSING_SELECTION)?;
	for id in [
		&reference.channel_id,
		&reference.message_id,
		&reference.author_id,
	]
	.into_iter()
	.chain(reference.guild_id.iter())
	{
		if !valid_snowflake(id) {
			return Err(STALE_SELECTION);
		}
	}
	Ok(reference)
}

fn load_saved(input: &Invocation) -> Result<Saved, &'static str> {
	let saved = match input.storage_json::<Saved>() {
		Ok(None) => Saved::default(),
		Ok(Some(saved)) => saved,
		Err(_) => return Err(STORAGE_INVALID),
	};
	if saved.excluded_roles.len() > MAX_EXCLUDED_ROLES
		|| !saved
			.excluded_roles
			.iter()
			.all(|role_id| valid_snowflake(role_id))
	{
		return Err(STORAGE_INVALID);
	}
	Ok(saved)
}

fn parse_excluded_roles(raw: &str) -> Result<Vec<String>, &'static str> {
	let mut roles = Vec::new();
	let mut seen = BTreeSet::new();
	for item in raw.split([',', ' ', '\n', '\r', '\t']) {
		let item = item.trim();
		if item.is_empty() {
			continue;
		}
		if !valid_snowflake(item) || roles.len() == MAX_EXCLUDED_ROLES {
			return Err(EXCLUSIONS_LIMIT);
		}
		if seen.insert(item.to_owned()) {
			roles.push(item.to_owned());
		}
	}
	Ok(roles)
}

fn panel_action(input: &Invocation) -> Output {
	let saved = match load_saved(input) {
		Ok(saved) => saved,
		Err(error) => {
			return Output {
				panel: vec![heading("Message inspector tools"), text(error)],
				..Default::default()
			};
		}
	};
	let raw = input
		.value("excluded-roles")
		.map(str::to_owned)
		.unwrap_or_else(|| saved.excluded_roles.join(", "));
	let mut panel = vec![
		heading("Message inspector tools"),
		text(
			"Role exclusions apply only to the local highest-loaded-role inspection. Roles are matched by ID from the grant-scoped member/role snapshot; no guild state is changed.",
		),
		Element::TextInput {
			id: "excluded-roles".into(),
			label: "Excluded role IDs (comma, space or newline separated)".into(),
			value: if input.action == "save" {
				raw.clone()
			} else {
				saved.excluded_roles.join(", ")
			},
		},
		Element::Button {
			id: "save".into(),
			label: "Save role exclusions".into(),
		},
	];
	if input.action != "save" {
		return Output {
			panel,
			..Default::default()
		};
	}
	let excluded_roles = match parse_excluded_roles(&raw) {
		Ok(roles) => roles,
		Err(error) => {
			panel.push(text(error));
			return Output {
				panel,
				..Default::default()
			};
		}
	};
	let saved = Saved { excluded_roles };
	let mut output = Output {
		panel,
		..Default::default()
	};
	if output.set_storage_json(&saved).is_err() {
		output.panel.push(text(STORAGE_LIMIT));
		return output;
	}
	output.panel.push(text(
		"Saved. The exclusions are stored as opaque bounded JSON and used only by this package.",
	));
	output
}

fn show_role(input: &MessageActionInvocation) -> AppOutput {
	let reference = match selected_reference(input) {
		Ok(reference) => reference,
		Err(message) => return error_output("Highest loaded role", message),
	};
	let saved = match load_saved(&input.invocation.invocation) {
		Ok(saved) => saved,
		Err(message) => return error_output("Highest loaded role", message),
	};
	let Some(guild_id) = reference.guild_id.as_deref() else {
		return error_output("Highest loaded role", ROLE_NO_GUILD);
	};
	let Some(app) = input.invocation.app.as_ref() else {
		return error_output("Highest loaded role", ROLE_NO_SNAPSHOT);
	};
	let Some(group) = app.member_details.as_ref() else {
		return error_output("Highest loaded role", ROLE_NO_SNAPSHOT);
	};
	if group.guild_id != guild_id || group.channel_id != reference.channel_id {
		return error_output("Highest loaded role", ROLE_STALE_SNAPSHOT);
	}
	let Some(member) = group
		.items
		.iter()
		.find(|member| member.user.id == reference.author_id)
	else {
		return error_output("Highest loaded role", ROLE_NO_MEMBER);
	};
	let Some(catalog) = group.roles.as_ref() else {
		return error_output("Highest loaded role", ROLE_NO_CATALOG);
	};
	let highest = catalog
		.iter()
		.filter(|role| member.role_ids.contains(&role.id))
		.filter(|role| !saved.excluded_roles.contains(&role.id))
		.max_by(|left, right| {
			left.position
				.cmp(&right.position)
				.then_with(|| left.id.cmp(&right.id))
		});
	let Some(role) = highest else {
		return panel_output(vec![heading("Highest loaded role"), text(ROLE_NONE)], None);
	};
	let color = if role.color == 0 {
		"default".to_owned()
	} else {
		format!("#{:06x}", role.color & 0x00ff_ffff)
	};
	let author = sanitize_label(&member.display_name, "Unnamed author");
	let role_name = sanitize_label(&role.name, "Unnamed role");
	let mut panel = vec![
		heading("Highest loaded role"),
		text(format!("{author}: {role_name}")),
		text(format!(
			"Position {} · color {color} · role ID {}",
			role.position, role.id
		)),
		text(
			"This reads only loaded guild roles and the loaded member pane. It does not add an inline chat badge, cannot identify bot accounts, and never fetches missing metadata.",
		),
	];
	if member.roles_truncated {
		panel.push(text(
			"This member's loaded role list is truncated; a higher role may be missing.",
		));
	}
	if group.roles_truncated {
		panel.push(text(
			"The guild role catalog is truncated; a higher role may be missing.",
		));
	}
	panel_output(panel, None)
}

fn sanitize_label(value: &str, fallback: &str) -> String {
	let cleaned: String = value
		.chars()
		.map(|character| {
			if character.is_control() || is_bidi_or_zero_width(character) {
				' '
			} else {
				character
			}
		})
		.collect();
	let trimmed = cleaned.trim();
	if trimmed.is_empty() {
		fallback.into()
	} else {
		trimmed.chars().take(100).collect()
	}
}

fn is_bidi_or_zero_width(character: char) -> bool {
	matches!(
		character,
		'\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
	)
}

fn parse_magnets(source: &str) -> Result<Vec<Magnet>, &'static str> {
	let mut magnets = Vec::new();
	for token in source.split_whitespace() {
		if !token.starts_with("magnet:?") {
			continue;
		}
		if magnets.len() == MAX_MAGNETS {
			return Err(TOO_MANY_MAGNETS);
		}
		if token.len() > MAX_MAGNET_BYTES {
			return Err(LONG_MAGNET);
		}
		if token.chars().any(char::is_control) {
			return Err(MAGNET_CONTROL);
		}
		magnets.push(Magnet {
			link: token.to_owned(),
			name: magnet_name(token),
		});
	}
	Ok(magnets)
}

fn magnet_name(link: &str) -> String {
	let query = link.split_once('?').map_or("", |(_, query)| query);
	for pair in query.split('&') {
		let Some((key, value)) = pair.split_once('=') else {
			continue;
		};
		if key != "dn" {
			continue;
		}
		return sanitize_magnet_name(&decode_component(value));
	}
	"unknown filename".into()
}

fn decode_component(value: &str) -> String {
	let bytes = value.as_bytes();
	let mut decoded = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		match bytes[index] {
			b'+' => {
				decoded.push(b' ');
				index += 1;
			}
			b'%' if index + 2 < bytes.len() => {
				let high = hex_value(bytes[index + 1]);
				let low = hex_value(bytes[index + 2]);
				if let (Some(high), Some(low)) = (high, low) {
					decoded.push((high << 4) | low);
					index += 3;
				} else {
					decoded.push(b'%');
					index += 1;
				}
			}
			byte => {
				decoded.push(byte);
				index += 1;
			}
		}
	}
	String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
	match byte {
		b'0'..=b'9' => Some(byte - b'0'),
		b'a'..=b'f' => Some(byte - b'a' + 10),
		b'A'..=b'F' => Some(byte - b'A' + 10),
		_ => None,
	}
}

fn sanitize_magnet_name(value: &str) -> String {
	let cleaned: String = value
		.chars()
		.map(|character| {
			if character.is_control() || is_bidi_or_zero_width(character) {
				' '
			} else {
				character
			}
		})
		.collect();
	let trimmed = cleaned.trim();
	if trimmed.is_empty() {
		"unknown filename".into()
	} else {
		trimmed.chars().take(MAX_NAME_CHARS).collect()
	}
}

fn copy_magnets(input: &MessageActionInvocation) -> AppOutput {
	if let Err(message) = selected_reference(input) {
		return error_output("Magnet links", message);
	}
	let source = input
		.invocation
		.invocation
		.selected_message
		.as_deref()
		.unwrap_or_default();
	let magnets = match parse_magnets(source) {
		Ok(magnets) => magnets,
		Err(message) => return error_output("Magnet links", message),
	};
	if magnets.is_empty() {
		return error_output("Magnet links", NO_MAGNETS);
	}
	let mut clipboard = String::new();
	let mut copied = 0;
	for magnet in &magnets {
		let line = format!("{} — {}\n", magnet.name, magnet.link);
		if clipboard.len() + line.len() > MAX_CLIPBOARD_BYTES {
			break;
		}
		clipboard.push_str(&line);
		copied += 1;
	}
	if copied == 0 {
		return error_output(
			"Magnet links",
			"The parsed links exceed the 4,096-byte clipboard limit. Nothing was copied.",
		);
	}
	let clipboard = clipboard.trim_end().to_owned();
	let mut panel = vec![
		heading("Magnet links"),
		text(format!(
			"Parsed {copied} of {} magnet link(s) from the selected message. The display name comes only from the link's dn parameter; nothing is fetched or opened.",
			magnets.len()
		)),
	];
	for magnet in magnets.iter().take(copied) {
		panel.push(text(format!("{} — {}", magnet.name, magnet.link)));
	}
	if copied < magnets.len() {
		panel.push(text(
			"The clipboard copy is truncated to the first links that fit 4,096 UTF-8 bytes.",
		));
	}
	panel.push(text(
		"Apply copies the exact text shown under this panel. Magnet links are never opened, downloaded or previewed.",
	));
	panel.push(text("Clipboard text:".to_owned()));
	for line in clipboard.lines() {
		panel.push(text(line.to_owned()));
	}
	panel_output(panel, Some(HostEffect::CopyText { text: clipboard }))
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	match input.invocation.invocation.action.as_str() {
		"open" | "save" => AppOutput {
			output: panel_action(&input.invocation.invocation),
			effects: Vec::new(),
		},
		"show-role" => show_role(&input),
		"copy-magnets" => copy_magnets(&input),
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	fn run(value: serde_json::Value) -> (AppOutput, serde_json::Value) {
		let bytes = dispatch_typed(&serde_json::to_vec(&value).unwrap(), handle).unwrap();
		let output: AppOutput = serde_json::from_slice(&bytes).unwrap();
		let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
		(output, wire)
	}

	fn reference() -> serde_json::Value {
		serde_json::json!({
			"channel_id": "100",
			"message_id": "200",
			"author_id": "300",
			"guild_id": "400"
		})
	}

	fn message(action: &str, selected: &str) -> serde_json::Value {
		serde_json::json!({
			"action": action,
			"selected_message": selected,
			"selected_message_ref": reference()
		})
	}

	fn role_app() -> serde_json::Value {
		serde_json::json!({
			"member_details": {
				"channel_id": "100",
				"guild_id": "400",
				"items": [{
					"user": {"id": "300", "name": "AuthorName"},
					"display_name": "AuthorName",
					"role_ids": ["11", "12", "13"],
					"roles_truncated": false
				}],
				"truncated": false,
				"roles": [
					{"id": "11", "name": "Member", "color": 0, "position": 1},
					{"id": "12", "name": "Moderator", "color": 16711680, "position": 5},
					{"id": "13", "name": "Admin", "color": 255, "position": 9}
				],
				"roles_truncated": false
			}
		})
	}

	fn storage(value: serde_json::Value) -> String {
		serde_json::to_string(&value).unwrap()
	}

	fn panel_text(output: &AppOutput) -> String {
		fn walk(elements: &[Element], out: &mut Vec<String>) {
			for element in elements {
				match element {
					Element::Text { text } | Element::Heading { text } => out.push(text.clone()),
					Element::Row { children } => walk(children, out),
					_ => {}
				}
			}
		}
		let mut out = Vec::new();
		walk(&output.output.panel, &mut out);
		out.join("\n")
	}

	#[test]
	fn highest_role_skips_excluded_roles_and_never_proposes_an_effect() {
		let (output, wire) = run(serde_json::json!({
			"action": "show-role",
			"selected_message": "hello",
			"selected_message_ref": reference(),
			"storage": storage(serde_json::json!({"excluded_roles": ["13"]})),
			"app": role_app()
		}));
		assert!(output.effects.is_empty());
		assert_eq!(wire.get("effects"), None);
		let text = panel_text(&output);
		assert!(text.contains("AuthorName: Moderator"));
		assert!(!text.contains("Admin"));
		let (all_excluded, _) = run(serde_json::json!({
			"action": "show-role",
			"selected_message": "hello",
			"selected_message_ref": reference(),
			"storage": storage(serde_json::json!({"excluded_roles": ["13", "12"]})),
			"app": role_app()
		}));
		assert!(panel_text(&all_excluded).contains("AuthorName: Member"));
	}

	#[test]
	fn highest_role_rejects_stale_missing_and_corrupt_context_without_guessing() {
		for (app, storage, expected) in [
			(
				serde_json::json!({"member_details": {"channel_id": "999", "guild_id": "400", "items": [], "truncated": false, "roles_truncated": false}}),
				storage(serde_json::json!({})),
				"another conversation",
			),
			(
				role_app(),
				storage(serde_json::json!({"excluded_roles": ["not-an-id"]})),
				"Saved role exclusions are invalid",
			),
		] {
			let (output, _) = run(serde_json::json!({
				"action": "show-role",
				"selected_message": "hello",
				"selected_message_ref": reference(),
				"storage": storage,
				"app": app
			}));
			assert!(output.effects.is_empty());
			assert!(panel_text(&output).contains(expected));
		}
		let (missing, _) = run(serde_json::json!({
			"action": "show-role",
			"selected_message": "hello"
		}));
		assert!(missing.effects.is_empty());
		let (no_guild, _) = run(serde_json::json!({
			"action": "show-role",
			"selected_message": "hello",
			"selected_message_ref": {
				"channel_id": "100", "message_id": "200", "author_id": "300"
			},
			"app": role_app()
		}));
		assert!(panel_text(&no_guild).contains("direct and group"));
	}

	#[test]
	fn magnet_names_decode_and_copy_exact_bounded_text() {
		let source = "look magnet:?xt=urn:btih:abc&dn=My+Favorite%20File&xl=123 \
			and magnet:?xt=urn:btih:def";
		let (output, wire) = run(message("copy-magnets", source));
		assert_eq!(output.effects.len(), 1);
		let HostEffect::CopyText { text } = &output.effects[0] else {
			panic!("expected copy_text");
		};
		assert_eq!(
			text,
			"My Favorite File — magnet:?xt=urn:btih:abc&dn=My+Favorite%20File&xl=123\n\
			 unknown filename — magnet:?xt=urn:btih:def"
		);
		assert_eq!(wire["effects"][0]["type"], "copy_text");
		let (sanitized, _) = run(message(
			"copy-magnets",
			"magnet:?dn=%0AFile%09%20Name%E2%80%AE",
		));
		let HostEffect::CopyText { text } = &sanitized.effects[0] else {
			panic!("expected copy_text");
		};
		assert_eq!(text, "File  Name — magnet:?dn=%0AFile%09%20Name%E2%80%AE");
	}

	#[test]
	fn magnet_action_rejects_missing_oversized_excess_and_control_links() {
		let too_many = (0..=MAX_MAGNETS)
			.map(|index| format!("magnet:?xt=urn:btih:{index}"))
			.collect::<Vec<_>>()
			.join(" ");
		let long = format!("magnet:?dn={}", "x".repeat(MAX_MAGNET_BYTES));
		for source in [
			"no links here".to_owned(),
			too_many,
			long,
			"magnet:?dn=bad\u{0}name".to_owned(),
		] {
			let (output, _) = run(message("copy-magnets", &source));
			assert!(output.effects.is_empty(), "{source}");
		}
		let (missing, _) = run(serde_json::json!({
			"action": "copy-magnets",
			"selected_message": "magnet:?dn=x"
		}));
		assert!(missing.effects.is_empty());
	}

	#[test]
	fn role_exclusion_panel_saves_valid_ids_and_rejects_invalid_input() {
		let (saved, wire) = run(serde_json::json!({
			"action": "save",
			"values": {"excluded-roles": "12, 13 12"}
		}));
		assert!(saved.effects.is_empty());
		assert_eq!(wire["storage"], "{\"excluded_roles\":[\"12\",\"13\"]}");
		let (invalid, wire) = run(serde_json::json!({
			"action": "save",
			"values": {"excluded-roles": "12, nope"}
		}));
		assert!(wire.get("storage").is_none_or(|value| value.is_null()));
		assert!(panel_text(&invalid).contains("nonzero snowflake"));
		let (open, _) = run(serde_json::json!({"action": "open"}));
		assert!(open.output.panel.iter().any(|element| matches!(
			element,
			Element::TextInput { id, .. } if id == "excluded-roles"
		)));
	}
}
