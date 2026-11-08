/*
 * Serein loaded role members
 * Copyright (c) 2026 Serein contributors
 *
 * Safe port of Equicord's `inRole` plugin (Copyright (c) 2024 nin0dev),
 * `clickableRoles` plugin (Copyright (c) 2026 Vendicated and contributors),
 * `betterRoleContext` plugin (Copyright (c) 2024 Vendicated and contributors)
 * and `betterRoleDot` plugin (Copyright (c) 2022 Vendicated and contributors),
 * all GPL-3.0-or-later, at upstream commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e.
 * Changed November 8, 2026: only the already-loaded member_details snapshot is
 * inspected. No guild-wide member request, member store, context-menu patch or
 * raw ID input is used; profile opening is an Apply-gated navigation proposal.
 * Role-color copying is limited to an explicit nonzero 24-bit color on a role
 * in the current loaded catalog; zero never becomes black or implies absence.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppInvocation, AppOutput, Element, HostEffect, MemberDetailSnapshot, MemberDetailsSnapshot,
	MemberRoleSnapshot, Output, serde_json,
};
use std::collections::BTreeSet;

/// The host caps a member_details group at 6 KiB. Keep the complete response at
/// or below the same ceiling instead of relying on the 256 KiB ABI limit.
const MAX_RESPONSE_BYTES: usize = 6 * 1024;
const MAX_MEMBER_ROWS: usize = 20;
const MAX_MEMBER_ROLE_IDS: usize = 32;
const MAX_ROLE_OPTIONS: usize = 32;
const MAX_SELECT_OPTION_BYTES: usize = 128;
/// Discord role colors are 24-bit RGB. Zero means "no explicit color", not black.
const MAX_ROLE_COLOR: u32 = 0x00ff_ffff;

/// Only the ID chosen from the current loaded catalog is persisted. The shape is
/// deliberately strict: anything else is corrupt and is preserved untouched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SavedRole {
	None,
	Valid,
	Corrupt,
}

fn valid_id(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 20
		&& !value.starts_with('0')
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn saved_role(input: &serein_extension_sdk::Invocation) -> (SavedRole, Option<String>) {
	match input.storage_json::<serde_json::Value>() {
		Ok(None) => (SavedRole::None, None),
		Ok(Some(value)) => {
			let role_id = value
				.as_object()
				.filter(|object| object.len() == 1)
				.and_then(|object| object.get("role_id"))
				.and_then(|value| value.as_str());
			match role_id {
				Some(role_id) if valid_id(role_id) => (SavedRole::Valid, Some(role_id.to_owned())),
				_ => (SavedRole::Corrupt, None),
			}
		}
		Err(_) => (SavedRole::Corrupt, None),
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

fn scope_text(members: &MemberDetailsSnapshot) -> String {
	let mut text = String::from(
		"Loaded members only: this checks the current member pane. Serein does not fetch missing members and does not show server-wide totals.",
	);
	if members.truncated {
		text.push_str(" The loaded member pane is partial.");
	}
	if members.roles_truncated {
		text.push_str(" The loaded role catalog is partial.");
	}
	text
}

fn role_options(members: &MemberDetailsSnapshot) -> Vec<(String, String)> {
	let Some(roles) = members.roles.as_ref() else {
		return Vec::new();
	};
	let mut seen = BTreeSet::new();
	let mut roles = roles
		.iter()
		.filter(|role| valid_id(&role.id) && seen.insert(role.id.as_str()))
		.collect::<Vec<&MemberRoleSnapshot>>();
	roles.sort_by(|left, right| {
		right
			.position
			.cmp(&left.position)
			.then_with(|| left.name.cmp(&right.name))
			.then_with(|| left.id.cmp(&right.id))
	});
	roles.truncate(MAX_ROLE_OPTIONS);
	roles
		.into_iter()
		.map(|role| (option_label(&role.name, &role.id), role.id.clone()))
		.collect()
}

fn role_known(members: &MemberDetailsSnapshot, role_id: &str) -> bool {
	members
		.roles
		.as_ref()
		.is_some_and(|roles| roles.iter().any(|role| role.id == role_id))
		|| members.items.iter().any(|member| {
			member
				.role_ids
				.iter()
				.take(MAX_MEMBER_ROLE_IDS)
				.any(|id| id == role_id)
		})
}

fn role_label(members: &MemberDetailsSnapshot, role_id: &str) -> String {
	members
		.roles
		.as_ref()
		.and_then(|roles| roles.iter().find(|role| role.id == role_id))
		.map_or_else(
			|| format!("Unlisted role [{role_id}]"),
			|role| option_label(&role.name, &role.id),
		)
}

/// Returns the canonical `#rrggbb` form only for an explicit 24-bit color.
/// Zero is Discord's "no color" sentinel and must never be rendered as black.
fn canonical_role_color(color: u32) -> Option<String> {
	(1..=MAX_ROLE_COLOR)
		.contains(&color)
		.then(|| format!("#{color:06x}"))
}

fn matching_members<'a>(
	members: &'a MemberDetailsSnapshot,
	role_id: &str,
) -> Vec<&'a MemberDetailSnapshot> {
	let mut seen = BTreeSet::new();
	let mut rows = members
		.items
		.iter()
		.filter(|member| {
			member
				.role_ids
				.iter()
				.take(MAX_MEMBER_ROLE_IDS)
				.any(|id| id == role_id)
		})
		.filter(|member| seen.insert(member.user.id.as_str()))
		.collect::<Vec<&MemberDetailSnapshot>>();
	rows.sort_by(|left, right| {
		left.display_name
			.to_lowercase()
			.cmp(&right.display_name.to_lowercase())
			.then_with(|| left.display_name.cmp(&right.display_name))
			.then_with(|| left.user.id.cmp(&right.user.id))
	});
	rows.truncate(MAX_MEMBER_ROWS);
	rows
}

fn member_options(matches: &[&MemberDetailSnapshot]) -> Vec<(String, String)> {
	matches
		.iter()
		.filter(|member| valid_id(&member.user.id))
		.map(|member| {
			(
				option_label(&member.display_name, &member.user.id),
				member.user.id.clone(),
			)
		})
		.collect()
}

fn member_row(member: &MemberDetailSnapshot) -> String {
	let mut row = format!(
		"{} [{}]",
		sanitize_label(&member.display_name),
		member.user.id
	);
	if member.roles_truncated {
		row.push_str(" (role list partial)");
	}
	row
}

fn select(options: &[(String, String)], id: &str, label: &str, value: Option<&str>) -> Element {
	let values = options
		.iter()
		.map(|(option, _)| option.clone())
		.collect::<Vec<_>>();
	let value = value
		.filter(|value| values.iter().any(|option| option == value))
		.map(str::to_owned)
		.or_else(|| values.first().cloned())
		.unwrap_or_default();
	Element::Select {
		id: id.into(),
		label: label.into(),
		options: values,
		value,
	}
}

fn picker_panel(members: &MemberDetailsSnapshot, notice: Option<&str>) -> Vec<Element> {
	let mut panel = vec![
		Element::Heading {
			text: "Loaded role members".into(),
		},
		text(scope_text(members)),
	];
	let options = role_options(members);
	if options.is_empty() {
		panel.push(text(if members.roles.is_none() {
			"The loaded role catalog is unavailable. Open a server channel with a loaded member pane and try again."
		} else {
			"The loaded role catalog is empty. No role can be chosen from this snapshot."
		}));
	} else {
		panel.push(select(&options, "role", "Loaded role", None));
		panel.push(Element::Button {
			id: "select-role".into(),
			label: "Use this loaded role".into(),
		});
		panel.push(Element::Button {
			id: "copy-role-color".into(),
			label: "Copy selected role color".into(),
		});
	}
	if let Some(notice) = notice {
		panel.push(text(notice));
	}
	panel.push(text(
		"Choosing a role only filters already-loaded members. Opening a profile and copying a role color are separate proposals that require Apply.",
	));
	panel
}

fn results_panel(
	members: &MemberDetailsSnapshot,
	role_id: &str,
	notice: Option<&str>,
) -> Vec<Element> {
	let mut panel = vec![
		Element::Heading {
			text: "Loaded role members".into(),
		},
		text(scope_text(members)),
		text(format!("Selected role: {}", role_label(members, role_id))),
	];
	if members.roles.is_none() {
		panel.push(text(
			"The loaded role catalog is unavailable, so the role name is unknown.",
		));
	}
	let matches = matching_members(members, role_id);
	panel.push(text(format!(
		"Matching loaded members: {} (loaded subset only).",
		matches.len()
	)));
	if matches.is_empty() {
		panel.push(text(
			"No loaded member in the current pane lists this role. This is not a server-wide result.",
		));
	} else {
		for member in &matches {
			panel.push(text(member_row(member)));
		}
		if matches.iter().any(|member| member.roles_truncated) {
			panel.push(text(
				"One or more matched members have partial role lists; a member without a visible match may still hold this role.",
			));
		}
		let options = member_options(&matches);
		if !options.is_empty() {
			panel.push(select(&options, "member", "Loaded matching member", None));
			panel.push(Element::Button {
				id: "open-profile".into(),
				label: "Propose opening selected profile".into(),
			});
			panel.push(text(
				"Opening a profile is a proposal. Serein opens it only after you choose Apply.",
			));
		}
	}
	if members.truncated {
		panel.push(text(
			"The loaded member pane is partial; members not loaded here are not checked.",
		));
	}
	if members.roles_truncated {
		panel.push(text(
			"The loaded role catalog is partial; some role names may be missing.",
		));
	}
	if let Some(notice) = notice {
		panel.push(text(notice));
	}
	let options = role_options(members);
	if !options.is_empty() {
		let selected = options
			.iter()
			.find(|(_, id)| id == role_id)
			.map(|(option, _)| option.as_str());
		panel.push(select(&options, "role", "Loaded role", selected));
		panel.push(Element::Button {
			id: "select-role".into(),
			label: "Use this loaded role".into(),
		});
		panel.push(Element::Button {
			id: "copy-role-color".into(),
			label: "Copy selected role color".into(),
		});
	}
	panel.push(text(
		"Only already-loaded members are checked. Serein does not fetch missing members or show server-wide totals.",
	));
	panel
}

fn unavailable_panel() -> Vec<Element> {
	vec![
		Element::Heading {
			text: "Loaded role members".into(),
		},
		text(
			"Member details are unavailable. Open a server channel with a loaded member pane and try again. Serein does not fetch members.",
		),
	]
}

fn panel_output(panel: Vec<Element>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		..Default::default()
	}
}

fn bounded(mut output: AppOutput) -> AppOutput {
	let too_large =
		serde_json::to_vec(&output).map_or(true, |bytes| bytes.len() > MAX_RESPONSE_BYTES);
	if too_large {
		output.output.panel = vec![text(
			"The loaded role view exceeded the 6 KiB response limit, so no member rows were shown. Choose a role with fewer loaded matches.",
		)];
		output.effects.clear();
	}
	output
}

fn select_role(
	members: &MemberDetailsSnapshot,
	input: &serein_extension_sdk::Invocation,
) -> AppOutput {
	let options = role_options(members);
	let chosen = input.value("role");
	let Some((_, role_id)) = options
		.iter()
		.find(|(option, _)| Some(option.as_str()) == chosen)
	else {
		return panel_output(picker_panel(
			members,
			Some(
				"That role selection is not in the current loaded role catalog. Nothing was saved.",
			),
		));
	};
	let mut output = panel_output(results_panel(
		members,
		role_id,
		Some("Saved this loaded role for the current account."),
	));
	if output
		.output
		.set_storage_json(&serde_json::json!({ "role_id": role_id }))
		.is_err()
	{
		return panel_output(picker_panel(
			members,
			Some("The role selection could not be saved. Nothing was changed."),
		));
	}
	output
}

fn copy_role_color(
	members: &MemberDetailsSnapshot,
	input: &serein_extension_sdk::Invocation,
) -> AppOutput {
	let options = role_options(members);
	let chosen = input.value("role");
	let Some((_, role_id)) = options
		.iter()
		.find(|(option, _)| Some(option.as_str()) == chosen)
	else {
		let notice = if members.roles.is_none() {
			"The loaded role catalog is unavailable, so no role color can be copied. Nothing was copied."
		} else if members.roles_truncated {
			"That role selection is not in the partial loaded role catalog. Serein cannot know its color and will not guess. Nothing was copied."
		} else {
			"That role selection is not in the current loaded role catalog. Nothing was copied."
		};
		return panel_output(picker_panel(members, Some(notice)));
	};
	let Some(role) = members.roles.as_ref().and_then(|roles| {
		roles
			.iter()
			.find(|role| role.id.as_str() == role_id.as_str() && valid_id(&role.id))
	}) else {
		return panel_output(picker_panel(
			members,
			Some("The selected role is no longer in the loaded role catalog. Nothing was copied."),
		));
	};
	let Some(color) = canonical_role_color(role.color) else {
		let notice = if role.color == 0 {
			"This loaded role has no explicit color (0). Serein will not copy #000000 or infer black. Nothing was copied."
		} else {
			"This loaded role has an invalid color outside the 24-bit RGB range. Serein will not guess a color. Nothing was copied."
		};
		return panel_output(picker_panel(members, Some(notice)));
	};
	let label = option_label(&role.name, &role.id);
	AppOutput {
		output: Output {
			panel: vec![
				Element::Heading {
					text: "Copy loaded role color".into(),
				},
				text(format!("Role: {label}")),
				text(format!("Proposed clipboard text: {color}")),
				text(
					"Serein replaces the clipboard only after you choose Apply. It never reads the clipboard.",
				),
			],
			..Default::default()
		},
		effects: vec![HostEffect::CopyText { text: color }],
	}
}

fn open_profile(
	members: &MemberDetailsSnapshot,
	input: &serein_extension_sdk::Invocation,
	saved: (SavedRole, Option<String>),
) -> AppOutput {
	let (state, role_id) = saved;
	let Some(role_id) = role_id.filter(|_| state == SavedRole::Valid) else {
		return panel_output(picker_panel(
			members,
			Some("Choose and save a loaded role before proposing a profile."),
		));
	};
	if !role_known(members, &role_id) {
		return panel_output(picker_panel(
			members,
			Some("The saved role is no longer loaded. Choose a currently loaded role."),
		));
	}
	let matches = matching_members(members, &role_id);
	let options = member_options(&matches);
	let chosen = input.value("member");
	let Some((_, user_id)) = options
		.iter()
		.find(|(option, _)| Some(option.as_str()) == chosen)
	else {
		return panel_output(results_panel(
			members,
			&role_id,
			Some("Select a currently loaded matching member before proposing a profile."),
		));
	};
	let display_name = matches
		.iter()
		.find(|member| member.user.id.as_str() == user_id.as_str())
		.map_or_else(
			|| user_id.clone(),
			|member| sanitize_label(&member.display_name),
		);
	AppOutput {
		output: Output {
			panel: vec![
				Element::Heading {
					text: "Open loaded member profile".into(),
				},
				text(format!("Proposed profile: {display_name} [{user_id}]")),
				text(
					"This is a proposal only. Serein opens the profile after you choose Apply; nothing opens automatically.",
				),
			],
			..Default::default()
		},
		effects: vec![HostEffect::OpenProfile {
			user_id: user_id.clone(),
		}],
	}
}

fn handle(input: AppInvocation) -> AppOutput {
	if input.app_event.is_some() || input.message_event.is_some() {
		return AppOutput::default();
	}
	let action = input.invocation.action.as_str();
	if !matches!(
		action,
		"show" | "select-role" | "open-profile" | "copy-role-color"
	) {
		return AppOutput::default();
	}
	let Some(members) = input
		.app
		.as_ref()
		.and_then(|app| app.member_details.as_ref())
	else {
		return bounded(panel_output(unavailable_panel()));
	};
	let saved = saved_role(&input.invocation);
	let output = match action {
		"show" => match (&saved.0, &saved.1) {
			(SavedRole::Corrupt, _) => panel_output(picker_panel(
				members,
				Some(
					"Saved role selection is unreadable and was preserved. Choose a loaded role to replace it.",
				),
			)),
			(SavedRole::Valid, Some(role_id)) if role_known(members, role_id) => {
				panel_output(results_panel(members, role_id, None))
			}
			(SavedRole::Valid, _) => panel_output(picker_panel(
				members,
				Some(
					"The saved role is no longer in the loaded role catalog or any loaded member's role list. Choose a currently loaded role.",
				),
			)),
			(SavedRole::None, _) => panel_output(picker_panel(members, None)),
		},
		"select-role" => select_role(members, &input.invocation),
		"open-profile" => open_profile(members, &input.invocation, saved),
		"copy-role-color" => copy_role_color(members, &input.invocation),
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

	fn members_input() -> serde_json::Value {
		serde_json::json!({
			"member_details": {
				"channel_id": "10",
				"guild_id": "20",
				"truncated": false,
				"roles": [
					{"id": "400", "name": "Moderator", "color": 0, "position": 2},
					{"id": "500", "name": "Member", "color": 0, "position": 1}
				],
				"roles_truncated": false,
				"items": [
					{"user": {"id": "3", "name": "Zed"}, "display_name": "Zed",
						"role_ids": ["400"], "roles_truncated": false},
					{"user": {"id": "2", "name": "Alice"}, "display_name": "Alice",
						"role_ids": ["400"], "roles_truncated": false},
					{"user": {"id": "1", "name": "Bob"}, "display_name": "Bob",
						"role_ids": ["500"], "roles_truncated": false},
					{"user": {"id": "3", "name": "Zed"}, "display_name": "Zed duplicate",
						"role_ids": ["400"], "roles_truncated": false}
				]
			}
		})
	}

	#[test]
	fn matches_role_ids_deduplicates_and_sorts_loaded_members() {
		let output = run(serde_json::json!({
			"action": "show",
			"storage": "{\"role_id\":\"400\"}",
			"app": members_input()
		}));
		let shown = panel_text(&output);
		assert!(shown.contains("Matching loaded members: 2 (loaded subset only)."));
		assert!(shown.contains("Alice [2]"));
		assert!(shown.contains("Zed [3]"));
		assert!(!shown.contains("Bob [1]"));
		assert!(!shown.contains("Zed duplicate"));
		assert!(shown.find("Alice [2]").unwrap() < shown.find("Zed [3]").unwrap());
		assert!(output.effects.is_empty());
		assert!(output.output.storage.is_none());
	}

	#[test]
	fn stale_or_invalid_roles_do_not_match_or_overwrite_storage() {
		let stale = run(serde_json::json!({
			"action": "show",
			"storage": "{\"role_id\":\"999\"}",
			"app": members_input()
		}));
		let stale_text = panel_text(&stale);
		assert!(stale_text.contains("no longer"));
		assert!(!stale_text.contains("Matching loaded members"));
		assert!(stale.output.storage.is_none());

		let invalid = run(serde_json::json!({
			"action": "select-role",
			"values": {"role": "Not a loaded role"},
			"app": members_input()
		}));
		assert!(panel_text(&invalid).contains("not in the current loaded role catalog"));
		assert!(invalid.output.storage.is_none());

		let mut without_catalog = members_input();
		without_catalog["member_details"]
			.as_object_mut()
			.unwrap()
			.remove("roles");
		let unlisted = run(serde_json::json!({
			"action": "show",
			"storage": "{\"role_id\":\"400\"}",
			"app": without_catalog
		}));
		let unlisted_text = panel_text(&unlisted);
		assert!(unlisted_text.contains("Unlisted role [400]"));
		assert!(unlisted_text.contains("Matching loaded members: 2 (loaded subset only)."));
	}

	#[test]
	fn partial_loaded_sets_are_described_as_partial() {
		let mut input = members_input();
		let members = input["member_details"].as_object_mut().unwrap();
		members.insert("truncated".into(), serde_json::Value::Bool(true));
		members.insert("roles_truncated".into(), serde_json::Value::Bool(true));
		members["items"][0]["roles_truncated"] = serde_json::Value::Bool(true);
		let output = run(serde_json::json!({
			"action": "show",
			"storage": "{\"role_id\":\"400\"}",
			"app": input
		}));
		let shown = panel_text(&output);
		assert!(shown.contains("member pane is partial"));
		assert!(shown.contains("role catalog is partial"));
		assert!(shown.contains("partial role lists"));
	}

	#[test]
	fn unknown_roles_are_not_treated_as_absent_from_the_server() {
		let mut input = members_input();
		input["member_details"]["items"] = serde_json::json!([]);
		let output = run(serde_json::json!({
			"action": "show",
			"storage": "{\"role_id\":\"400\"}",
			"app": input
		}));
		let shown = panel_text(&output);
		assert!(shown.contains("Matching loaded members: 0"));
		assert!(shown.contains("not a server-wide result"));
	}

	#[test]
	fn corrupt_storage_is_preserved_until_an_explicit_valid_selection() {
		for storage in [
			"not-json",
			"{\"role_id\":\"abc\"}",
			"{\"role_id\":\"400\",\"extra\":1}",
			"[]",
		] {
			let output = run(serde_json::json!({
				"action": "show",
				"storage": storage,
				"app": members_input()
			}));
			assert!(panel_text(&output).contains("unreadable and was preserved"));
			assert!(output.output.storage.is_none());
		}

		let repaired = run(serde_json::json!({
			"action": "select-role",
			"storage": "not-json",
			"values": {"role": "Moderator [400]"},
			"app": members_input()
		}));
		let saved: serde_json::Value =
			serde_json::from_str(repaired.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(saved["role_id"], "400");
		assert!(panel_text(&repaired).contains("Matching loaded members: 2"));
	}

	#[test]
	fn profile_navigation_is_a_single_apply_gated_proposal() {
		let output = run(serde_json::json!({
			"action": "open-profile",
			"storage": "{\"role_id\":\"400\"}",
			"values": {"member": "Zed [3]"},
			"app": members_input()
		}));
		assert_eq!(
			output.effects,
			vec![HostEffect::OpenProfile {
				user_id: "3".into()
			}]
		);
		assert!(panel_text(&output).contains("choose Apply"));

		for (values, storage) in [
			(
				serde_json::json!({"member": "Bob [1]"}),
				"{\"role_id\":\"400\"}",
			),
			(serde_json::json!({"member": "Zed [3]"}), "not-json"),
		] {
			let rejected = run(serde_json::json!({
				"action": "open-profile",
				"storage": storage,
				"values": values,
				"app": members_input()
			}));
			assert!(rejected.effects.is_empty());
		}
	}

	#[test]
	fn role_color_copy_formats_exact_24_bit_rgb() {
		for (color, expected) in [
			(0x12ab34, "#12ab34"),
			(0x0000ff, "#0000ff"),
			(0x000001, "#000001"),
			(0xffffff, "#ffffff"),
		] {
			let mut input = members_input();
			input["member_details"]["roles"][0]["color"] = serde_json::json!(color);
			let output = run(serde_json::json!({
				"action": "copy-role-color",
				"values": {"role": "Moderator [400]"},
				"app": input
			}));
			assert_eq!(
				output.effects,
				vec![HostEffect::CopyText {
					text: expected.into()
				}]
			);
			assert!(output.output.storage.is_none());
			let shown = panel_text(&output);
			assert!(shown.contains(&format!("Proposed clipboard text: {expected}")));
			assert!(shown.contains("choose Apply"));
			assert!(!shown.contains("Matching loaded members"));
		}
	}

	#[test]
	fn zero_and_out_of_range_colors_are_not_guessed() {
		let zero = run(serde_json::json!({
			"action": "copy-role-color",
			"values": {"role": "Moderator [400]"},
			"app": members_input()
		}));
		assert!(zero.effects.is_empty());
		assert!(zero.output.storage.is_none());
		let zero_text = panel_text(&zero);
		assert!(zero_text.contains("no explicit color (0)"));
		assert!(zero_text.contains("will not copy #000000 or infer black"));
		assert!(!zero_text.contains("Proposed clipboard text"));

		let mut input = members_input();
		input["member_details"]["roles"][0]["color"] = serde_json::json!(0x0100_0000u32);
		let invalid = run(serde_json::json!({
			"action": "copy-role-color",
			"values": {"role": "Moderator [400]"},
			"app": input
		}));
		assert!(invalid.effects.is_empty());
		assert!(invalid.output.storage.is_none());
		assert!(panel_text(&invalid).contains("outside the 24-bit RGB range"));
	}

	#[test]
	fn stale_partial_and_unavailable_color_selections_do_not_copy() {
		let stale = run(serde_json::json!({
			"action": "copy-role-color",
			"values": {"role": "Ghost [999]"},
			"app": members_input()
		}));
		assert!(stale.effects.is_empty());
		assert!(stale.output.storage.is_none());
		assert!(panel_text(&stale).contains("not in the current loaded role catalog"));

		let mut partial = members_input();
		partial["member_details"]["roles_truncated"] = serde_json::Value::Bool(true);
		let partial = run(serde_json::json!({
			"action": "copy-role-color",
			"values": {"role": "Ghost [999]"},
			"app": partial
		}));
		assert!(partial.effects.is_empty());
		assert!(partial.output.storage.is_none());
		let partial_text = panel_text(&partial);
		assert!(partial_text.contains("partial loaded role catalog"));
		assert!(partial_text.contains("will not guess"));

		let mut unavailable = members_input();
		unavailable["member_details"]
			.as_object_mut()
			.unwrap()
			.remove("roles");
		let unavailable = run(serde_json::json!({
			"action": "copy-role-color",
			"values": {"role": "Moderator [400]"},
			"app": unavailable
		}));
		assert!(unavailable.effects.is_empty());
		assert!(unavailable.output.storage.is_none());
		assert!(panel_text(&unavailable).contains("catalog is unavailable"));
	}

	#[test]
	fn role_color_copy_ignores_saved_storage_and_returns_no_unrelated_data() {
		let mut input = members_input();
		input["member_details"]["roles"][0]["color"] = serde_json::json!(0x00ff00);
		input["member_details"]["roles"][1]["color"] = serde_json::json!(0x0000ff);
		for storage in ["not-json", "{\"role_id\":\"500\"}"] {
			let output = run(serde_json::json!({
				"action": "copy-role-color",
				"storage": storage,
				"values": {"role": "Moderator [400]"},
				"app": input
			}));
			assert_eq!(
				output.effects,
				vec![HostEffect::CopyText {
					text: "#00ff00".into()
				}]
			);
			assert!(output.output.storage.is_none());
			let shown = panel_text(&output);
			assert!(!shown.contains("Alice"));
			assert!(!shown.contains("Zed"));
			assert!(!shown.contains("Matching loaded members"));
		}
	}

	#[test]
	fn response_and_select_limits_hold_for_a_full_unicode_batch() {
		fn full_batch(name: &str) -> serde_json::Value {
			let roles = (0..32)
				.map(|index| {
					serde_json::json!({
						"id": format!("{}", 400 + index),
						"name": format!("{name}{index}"),
						"color": 0,
						"position": index
					})
				})
				.collect::<Vec<_>>();
			let items = (0..20)
				.map(|index| {
					serde_json::json!({
						"user": {"id": format!("{}", 1000 + index), "name": name},
						"display_name": format!("{name}{index}"),
						"role_ids": ["400"],
						"roles_truncated": false
					})
				})
				.collect::<Vec<_>>();
			serde_json::json!({"member_details": {
				"channel_id": "10", "guild_id": "20", "items": items,
				"truncated": false, "roles": roles, "roles_truncated": false
			}})
		}

		let output = run(serde_json::json!({
			"action": "show",
			"storage": "{\"role_id\":\"400\"}",
			"app": full_batch(&"🦀".repeat(8))
		}));
		let bytes = serde_json::to_vec(&output).unwrap();
		assert!(bytes.len() <= MAX_RESPONSE_BYTES);
		assert!(output.output.panel.len() <= 64);
		let mut selects = 0;
		for element in &output.output.panel {
			match element {
				Element::Text { text } => assert!(text.len() <= 4096),
				Element::Select { options, value, .. } => {
					selects += 1;
					assert!(options.len() <= 32);
					assert!(options.contains(value));
					assert!(options.iter().all(|option| option.len() <= 128));
					assert!(
						options
							.iter()
							.all(|option| !option.chars().any(char::is_control))
					);
				}
				_ => {}
			}
		}
		assert_eq!(selects, 2);

		let overflow = run(serde_json::json!({
			"action": "show",
			"storage": "{\"role_id\":\"400\"}",
			"app": full_batch(&"🦀".repeat(100))
		}));
		assert!(serde_json::to_vec(&overflow).unwrap().len() <= MAX_RESPONSE_BYTES);
		assert!(panel_text(&overflow).contains("6 KiB response limit"));

		let label = option_label(&"🦀".repeat(100), "400");
		assert!(label.len() <= 128);
		assert!(label.ends_with(" [400]"));
	}
}
