/*
 * Serein loaded permissions viewer
 * Copyright (c) 2023 Vendicated and contributors
 * Copyright (c) 2026 Serein port contributors
 *
 * Safe subset of Equicord's `permissionsViewer` plugin at upstream commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e (`src/plugins/permissionsViewer`),
 * licensed GPL-3.0-or-later. Changed November 8, 2026: this port reads only the
 * host-granted `channel_metadata` and `member_details` snapshots. It does not
 * patch Discord internals, fetch members or roles, read role permission bits,
 * open a popout, or write anything. It shows the account's already-loaded
 * channel permission decisions and the loaded role order of one loaded member.
 * It never claims to show another member's effective permissions.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppInvocation, AppOutput, ChannelMetadataSnapshot, ChannelPermission, Element,
	MemberDetailSnapshot, MemberDetailsSnapshot, MemberRoleSnapshot, Output, serde_json,
};
use std::collections::BTreeSet;

/// The host caps each supplied snapshot group at 6 KiB. Keep the whole response
/// below the same ceiling instead of approaching the 256 KiB ABI limit.
const MAX_RESPONSE_BYTES: usize = 6 * 1024;
const MAX_TEXT_BYTES: usize = 4096;
const MAX_MEMBER_ROWS: usize = 20;
const MAX_SELECT_OPTION_BYTES: usize = 128;
/// Discord role colors are 24-bit RGB. Zero means "no explicit color", not black.
const MAX_ROLE_COLOR: u32 = 0x00ff_ffff;

fn valid_id(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 20
		&& !value.starts_with('0')
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
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

fn bounded_text(value: &str, max_bytes: usize) -> String {
	if value.len() <= max_bytes {
		return value.to_owned();
	}
	let marker = "…";
	let budget = max_bytes.saturating_sub(marker.len());
	let mut text = truncate_utf8(value, budget).to_owned();
	text.push_str(marker);
	text
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
	let value = value.into();
	Element::Text {
		text: bounded_text(&value, MAX_TEXT_BYTES),
	}
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

fn permission_label(permission: ChannelPermission) -> &'static str {
	match permission {
		ChannelPermission::ViewChannel => "View channel",
		ChannelPermission::ReadMessageHistory => "Read message history",
		ChannelPermission::SendMessages => "Send messages",
		ChannelPermission::SendMessagesInThreads => "Send messages in threads",
		ChannelPermission::AttachFiles => "Attach files",
		ChannelPermission::EmbedLinks => "Embed links",
		ChannelPermission::AddReactions => "Add reactions",
		ChannelPermission::MentionEveryone => "Mention everyone",
		ChannelPermission::UseExternalEmojis => "Use external emojis",
		ChannelPermission::UseExternalStickers => "Use external stickers",
		ChannelPermission::UseApplicationCommands => "Use application commands",
		ChannelPermission::ManageChannels => "Manage channels",
		ChannelPermission::ManageMessages => "Manage messages",
		ChannelPermission::ManageRoles => "Manage roles",
		ChannelPermission::ManageThreads => "Manage threads",
		ChannelPermission::CreatePublicThreads => "Create public threads",
		ChannelPermission::CreatePrivateThreads => "Create private threads",
		ChannelPermission::ManageWebhooks => "Manage webhooks",
		ChannelPermission::Connect => "Connect to voice",
		ChannelPermission::Speak => "Speak in voice",
		ChannelPermission::Stream => "Stream in voice",
		ChannelPermission::MuteMembers => "Mute members",
		ChannelPermission::DeafenMembers => "Deafen members",
		ChannelPermission::MoveMembers => "Move members",
		ChannelPermission::UseVad => "Use voice activity",
		ChannelPermission::PinMessages => "Pin messages",
	}
}

/// Stable groups in the snapshot's own key order. Missing decisions stay
/// distinct from denials; the panel never renders unknown as allowed.
fn grouped_permissions(channel: &ChannelMetadataSnapshot) -> (String, String, String) {
	let mut allowed = Vec::new();
	let mut denied = Vec::new();
	let mut unknown = Vec::new();
	for (permission, decision) in &channel.permissions {
		let label = permission_label(*permission);
		match decision {
			Some(true) => allowed.push(label),
			Some(false) => denied.push(label),
			None => unknown.push(label),
		}
	}
	let join = |labels: Vec<&'static str>| {
		if labels.is_empty() {
			"none".to_owned()
		} else {
			labels.join(", ")
		}
	};
	(join(allowed), join(denied), join(unknown))
}

fn member_options(members: &MemberDetailsSnapshot) -> Vec<(String, String)> {
	let mut seen = BTreeSet::new();
	let mut options = Vec::new();
	for member in members.items.iter().take(MAX_MEMBER_ROWS) {
		if !valid_id(&member.user.id) || !seen.insert(member.user.id.as_str()) {
			continue;
		}
		options.push((
			option_label(&member.display_name, &member.user.id),
			member.user.id.clone(),
		));
	}
	options
}

/// Only roles present in the loaded catalog are returned. Unknown role IDs are
/// counted, never guessed or turned into a synthetic role.
fn ordered_roles<'a>(
	members: &'a MemberDetailsSnapshot,
	member: &'a MemberDetailSnapshot,
) -> (Vec<&'a MemberRoleSnapshot>, usize) {
	let wanted = member
		.role_ids
		.iter()
		.map(String::as_str)
		.filter(|id| valid_id(id))
		.collect::<BTreeSet<_>>();
	let Some(catalog) = members.roles.as_deref() else {
		return (Vec::new(), wanted.len());
	};
	let mut known = BTreeSet::new();
	let mut roles = Vec::new();
	for role in catalog {
		if valid_id(&role.id) && wanted.contains(role.id.as_str()) && known.insert(role.id.as_str())
		{
			roles.push(role);
		}
	}
	let unknown = wanted.iter().filter(|id| !known.contains(*id)).count();
	roles.sort_by(|left, right| {
		right
			.position
			.cmp(&left.position)
			.then_with(|| left.name.cmp(&right.name))
			.then_with(|| left.id.cmp(&right.id))
	});
	(roles, unknown)
}

fn role_line(role: &MemberRoleSnapshot) -> String {
	let name = sanitize_label(&role.name);
	let color = if role.color != 0 && role.color <= MAX_ROLE_COLOR {
		format!(", #{:06x}", role.color)
	} else {
		String::new()
	};
	format!("{} [{}] (position {}{color})", name, role.id, role.position)
}

fn member_summary(members: &MemberDetailsSnapshot, member: &MemberDetailSnapshot) -> String {
	let mut lines = vec![format!(
		"Member: {} [{}]",
		sanitize_label(&member.display_name),
		member.user.id
	)];
	let (roles, unknown) = ordered_roles(members, member);
	if members.roles.is_some() {
		if roles.is_empty() {
			lines.push(
				"No loaded role in the catalog matches this member's loaded role IDs.".into(),
			);
		} else {
			lines.push(format!(
				"Loaded roles, highest position first ({}):",
				roles.len()
			));
			for role in roles {
				lines.push(role_line(role));
			}
		}
	} else {
		lines.push(
			"The loaded role catalog is unavailable, so role names and positions cannot be shown."
				.into(),
		);
	}
	if member.roles_truncated {
		lines.push("This member's loaded role ID list is partial.".into());
	}
	if members.roles_truncated {
		lines.push("The loaded role catalog is partial.".into());
	}
	if unknown > 0 {
		lines.push(format!(
			"{unknown} loaded role ID(s) are not in the loaded role catalog and were not guessed."
		));
	}
	lines.join("\n")
}

fn viewer_panel(
	channel: Option<&ChannelMetadataSnapshot>,
	members: Option<&MemberDetailsSnapshot>,
	requested: Option<&str>,
	notice: Option<&str>,
) -> Vec<Element> {
	let mut panel = vec![heading("Permissions viewer (loaded data)")];
	if let Some(notice) = notice {
		panel.push(text(notice));
	}
	match channel {
		Some(channel) => {
			panel.push(heading("Channel decisions"));
			panel.push(text(format!(
				"Loaded channel {} in server {}.",
				channel.channel_id, channel.guild_id
			)));
			if channel.permissions.is_empty() {
				panel.push(text(
					"The host did not include permission decisions for this channel.",
				));
			} else {
				let (allowed, denied, unknown) = grouped_permissions(channel);
				panel.push(text(format!("Allowed: {allowed}")));
				panel.push(text(format!("Denied: {denied}")));
				panel.push(text(format!(
					"Unknown (the loaded decision is not available): {unknown}"
				)));
			}
			panel.push(text(
				"These are the account's loaded decisions for the current channel, not another member's effective permissions, and they do not guarantee an action can run.",
			));
		}
		None => panel.push(text(
			"Channel permission decisions are unavailable. Open a readable server channel with loaded metadata and try again. Serein does not fetch permissions.",
		)),
	}
	match members {
		Some(members) => {
			panel.push(heading("Loaded member roles"));
			let options = member_options(members);
			if options.is_empty() {
				panel.push(text(
					"No loaded members are available in this pane. Serein does not fetch members.",
				));
			} else {
				let chosen = requested
					.and_then(|value| options.iter().find(|(label, _)| label.as_str() == value))
					.or_else(|| options.first());
				let value = chosen.map_or_else(String::new, |(label, _)| label.clone());
				panel.push(Element::Select {
					id: "member".into(),
					label: "Loaded member".into(),
					options: options
						.iter()
						.map(|(label, _)| label.clone())
						.collect(),
					value,
				});
				panel.push(button(
					"select-member",
					"Show selected member's role order",
				));
				if let Some((_, user_id)) = chosen
					&& let Some(member) = members
						.items
						.iter()
						.find(|member| member.user.id == user_id.as_str())
				{
					panel.push(text(member_summary(members, member)));
				}
			}
			panel.push(text(
				"Only roles in the loaded role catalog are shown, ordered by descending position. Serein does not fetch roles or members.",
			));
		}
		None => panel.push(text(
			"Loaded member roles are unavailable. Open a server channel with a loaded member pane and try again. Serein does not fetch members.",
		)),
	}
	panel
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
			"The loaded permissions view exceeded the 6 KiB response limit, so no rows were shown. Open a smaller channel or member pane.",
		)];
	}
	output
}

fn handle(input: AppInvocation) -> AppOutput {
	if input.app_event.is_some() || input.message_event.is_some() {
		return AppOutput::default();
	}
	let action = input.invocation.action.as_str();
	if !matches!(action, "show" | "select-member") {
		return AppOutput::default();
	}
	let channel = input
		.app
		.as_ref()
		.and_then(|app| app.channel_metadata.as_ref());
	let members = input
		.app
		.as_ref()
		.and_then(|app| app.member_details.as_ref());
	let requested = input.invocation.value("member");
	let output = if action == "select-member" {
		match members {
			Some(members) if !members.items.is_empty() => {
				let options = member_options(members);
				if options.is_empty() {
					panel_output(viewer_panel(
						channel,
						Some(members),
						None,
						Some("No loaded members are available in this pane."),
					))
				} else if requested
					.is_some_and(|value| options.iter().any(|(label, _)| label.as_str() == value))
				{
					panel_output(viewer_panel(channel, Some(members), requested, None))
				} else {
					panel_output(viewer_panel(
						channel,
						Some(members),
						None,
						Some(
							"That member selection is not in the current loaded member pane. Nothing was changed.",
						),
					))
				}
			}
			Some(members) => panel_output(viewer_panel(
				channel,
				Some(members),
				None,
				Some("No loaded members are available in this pane."),
			)),
			None => panel_output(viewer_panel(
				channel,
				None,
				None,
				Some("Member details are unavailable, so no member was selected."),
			)),
		}
	} else {
		panel_output(viewer_panel(channel, members, requested, None))
	};
	bounded(output)
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::serde_json;
	use serein_extension_sdk::serde_json::json;

	fn run(input: serde_json::Value) -> AppOutput {
		let bytes = serde_json::to_vec(&input).unwrap();
		let output =
			serein_extension_sdk::dispatch_typed::<AppInvocation, AppOutput>(&bytes, handle)
				.unwrap();
		serde_json::from_slice(&output).unwrap()
	}

	fn panel_text(output: &AppOutput) -> String {
		fn walk(elements: &[Element], text: &mut String) {
			for element in elements {
				match element {
					Element::Text { text: value } | Element::Heading { text: value } => {
						text.push_str(value);
						text.push('\n');
					}
					Element::Row { children } => walk(children, text),
					_ => {}
				}
			}
		}
		let mut text = String::new();
		walk(&output.output.panel, &mut text);
		text
	}

	fn member_details(
		members: impl Into<serde_json::Value>,
		roles: impl Into<serde_json::Value>,
	) -> serde_json::Value {
		let members = members.into();
		let roles = roles.into();
		json!({
			"member_details": {
				"channel_id": "10",
				"guild_id": "20",
				"items": members,
				"truncated": false,
				"roles": roles,
				"roles_truncated": false
			}
		})
	}

	#[test]
	fn channel_decisions_keep_allowed_denied_and_unknown_distinct() {
		let output = run(json!({
			"action": "show",
			"app": {
				"channel_metadata": {
					"channel_id": "10",
					"guild_id": "20",
					"topic_truncated": false,
					"permissions": {
						"view_channel": true,
						"send_messages": false,
						"manage_roles": null
					}
				}
			}
		}));
		let shown = panel_text(&output);
		assert!(shown.contains("Allowed: View channel"));
		assert!(shown.contains("Denied: Send messages"));
		assert!(shown.contains("Unknown (the loaded decision is not available): Manage roles"));
		assert!(shown.contains("account's loaded decisions"));
		assert!(output.effects.is_empty());
		assert!(output.output.storage.is_none());
	}

	#[test]
	fn member_roles_use_only_the_loaded_catalog_and_sort_by_position() {
		let output = run(json!({
			"action": "show",
			"app": member_details(
				json!([{
					"user": {"id": "100", "name": "ada"},
					"display_name": "Ada",
					"role_ids": ["1", "3", "5", "999"],
					"roles_truncated": false
				}]),
				json!([
					{"id": "1", "name": "Low", "color": 0, "position": 1},
					{"id": "3", "name": "High", "color": 0x112233, "position": 30},
					{"id": "5", "name": "Mid", "color": 0xabcdef, "position": 5}
				])
			)
		}));
		let shown = panel_text(&output);
		let high = shown.find("High [3]").unwrap();
		let mid = shown.find("Mid [5]").unwrap();
		let low = shown.find("Low [1]").unwrap();
		assert!(high < mid && mid < low);
		assert!(shown.contains("#112233"));
		assert!(shown.contains(
			"1 loaded role ID(s) are not in the loaded role catalog and were not guessed."
		));
		assert!(!shown.contains("999"));
	}

	#[test]
	fn select_member_rejects_an_option_outside_the_current_snapshot() {
		let output = run(json!({
			"action": "select-member",
			"values": {"member": "Mallory [999]"},
			"app": member_details(
				json!([{
					"user": {"id": "100", "name": "ada"},
					"display_name": "Ada",
					"role_ids": ["1"],
					"roles_truncated": false
				}]),
				json!([{"id": "1", "name": "Low", "color": 0, "position": 1}])
			)
		}));
		let shown = panel_text(&output);
		assert!(shown.contains("not in the current loaded member pane"));
		assert!(shown.contains("Ada [100]"));
		assert!(!shown.contains("Mallory [999]"));
	}

	#[test]
	fn labels_strip_control_characters_and_missing_snapshots_stay_explicit() {
		let output = run(json!({
			"action": "show",
			"app": member_details(
				json!([{
					"user": {"id": "100", "name": "ada"},
					"display_name": "Evil\nName",
					"role_ids": ["1"],
					"roles_truncated": false
				}]),
				json!([{"id": "1", "name": "Role\nName", "color": 0, "position": 1}])
			)
		}));
		for element in &output.output.panel {
			if let Element::Select { options, value, .. } = element {
				assert!(
					options
						.iter()
						.all(|option| !option.chars().any(char::is_control))
				);
				assert!(value.as_str() == "Evil Name [100]");
			}
		}
		let shown = panel_text(&output);
		assert!(shown.contains("Role Name [1]"));
		assert!(shown.contains("Channel permission decisions are unavailable"));

		let empty = run(json!({"action": "show"}));
		assert!(panel_text(&empty).contains("Loaded member roles are unavailable"));
		assert!(empty.output.panel.len() <= 64);
	}

	#[test]
	fn unknown_actions_and_events_produce_no_effect_or_panel() {
		let unknown = run(json!({"action": "delete-everything"}));
		assert!(unknown.output.panel.is_empty());
		assert!(unknown.effects.is_empty());
		let event = run(json!({
			"action": "show",
			"app_event": "presence",
			"app": member_details(json!([]), json!([]))
		}));
		assert!(event.output.panel.is_empty());
	}

	#[test]
	fn full_batch_keeps_response_and_select_limits() {
		let roles = (0..32)
			.map(|index| {
				json!({
					"id": format!("{}", 400 + index),
					"name": format!("role{index}"),
					"color": 0,
					"position": index
				})
			})
			.collect::<Vec<_>>();
		let members = (0..20)
			.map(|index| {
				json!({
					"user": {"id": format!("{}", 1000 + index), "name": format!("user{index}")},
					"display_name": format!("Member {index}"),
					"role_ids": ["400", "401"],
					"roles_truncated": false
				})
			})
			.collect::<Vec<_>>();
		let mut permissions = serde_json::Map::new();
		for key in [
			"view_channel",
			"read_message_history",
			"send_messages",
			"send_messages_in_threads",
			"attach_files",
			"embed_links",
			"add_reactions",
			"mention_everyone",
			"use_external_emojis",
			"use_external_stickers",
			"use_application_commands",
			"manage_channels",
			"manage_messages",
			"manage_roles",
			"manage_threads",
			"create_public_threads",
			"create_private_threads",
			"manage_webhooks",
			"connect",
			"speak",
			"stream",
			"mute_members",
			"deafen_members",
			"move_members",
			"use_vad",
			"pin_messages",
		] {
			permissions.insert(key.into(), json!(true));
		}
		let output = run(json!({
			"action": "show",
			"app": {
				"channel_metadata": {
					"channel_id": "10",
					"guild_id": "20",
					"topic_truncated": false,
					"permissions": permissions
				},
				"member_details": {
					"channel_id": "10",
					"guild_id": "20",
					"items": members,
					"truncated": false,
					"roles": roles,
					"roles_truncated": false
				}
			}
		}));
		assert!(serde_json::to_vec(&output).unwrap().len() <= MAX_RESPONSE_BYTES);
		assert!(output.output.panel.len() <= 64);
		for element in &output.output.panel {
			match element {
				Element::Text { text } => assert!(text.len() <= MAX_TEXT_BYTES),
				Element::Select { options, value, .. } => {
					assert!(options.len() <= 32);
					assert!(options.contains(value));
					assert!(options.iter().all(|option| option.len() <= 128));
				}
				_ => {}
			}
		}

		let overflow = run(json!({
			"action": "show",
			"app": member_details(
				(0..20)
					.map(|index| json!({
						"user": {"id": format!("{}", 1000 + index), "name": "ada"},
						"display_name": "🦀".repeat(100),
						"role_ids": (0..32).map(|role| format!("{}", 400 + role)).collect::<Vec<_>>(),
						"roles_truncated": false
					}))
					.collect::<Vec<_>>(),
				(0..32)
					.map(|index| json!({
						"id": format!("{}", 400 + index),
						"name": "🦀".repeat(100),
						"color": 0,
						"position": index
					}))
					.collect::<Vec<_>>()
			)
		}));
		assert!(serde_json::to_vec(&overflow).unwrap().len() <= MAX_RESPONSE_BYTES);
		assert!(panel_text(&overflow).contains("6 KiB response limit"));
	}
}
