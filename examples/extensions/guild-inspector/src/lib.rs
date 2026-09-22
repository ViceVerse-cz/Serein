use serein_extension_sdk::{AppInvocation, AppOutput, Element, Output};

fn text(text: impl Into<String>) -> Element {
	Element::Text { text: text.into() }
}

fn known(value: Option<bool>) -> &'static str {
	match value {
		Some(true) => "yes",
		Some(false) => "no",
		None => "unknown",
	}
}

fn handle(input: AppInvocation) -> AppOutput {
	if input.app_event.is_some()
		|| input.message_event.is_some()
		|| input.invocation.action != "show"
	{
		return AppOutput::default();
	}
	let mut panel = vec![
		Element::Heading {
			text: "Guild inspector".into(),
		},
		text(
			"Already-loaded data only. Permissions are current decisions, not guarantees that an action can run.",
		),
	];
	let app = input.app.unwrap_or_default();
	match app.channel_metadata {
		None => panel.push(text("Channel metadata unavailable.")),
		Some(channel) => {
			panel.push(text(format!(
				"Channel {} in server {}\nCategory: {}\nParent: {}",
				channel.channel_id,
				channel.guild_id,
				channel
					.category
					.as_ref()
					.map_or("unavailable", |value| value.name.as_str()),
				channel
					.parent
					.as_ref()
					.map_or("unavailable", |value| value.name.as_str())
			)));
			panel.push(text(format!(
				"Topic: {}{}\nSlowmode: {}\nNSFW: {}",
				channel.topic.as_deref().unwrap_or("unknown"),
				if channel.topic_truncated {
					" (partial)"
				} else {
					""
				},
				channel
					.slowmode_seconds
					.map_or_else(|| "unknown".into(), |seconds| format!("{seconds}s")),
				known(channel.nsfw)
			)));
			if let Some(thread) = channel.thread {
				panel.push(text(format!(
					"Thread: archived {}; locked {}; pinned {}\nOwner: {}; messages: {}",
					known(thread.archived),
					known(thread.locked),
					known(thread.pinned),
					thread.owner_id.as_deref().unwrap_or("unknown"),
					thread
						.message_count
						.map_or_else(|| "unknown".into(), |count| count.to_string())
				)));
			}
			let permissions = channel
				.permissions
				.iter()
				.map(|(permission, value)| format!("{permission:?}: {}", known(*value)))
				.collect::<Vec<_>>()
				.join("\n");
			panel.push(text(if permissions.is_empty() {
				"Permissions unavailable.".into()
			} else {
				permissions
			}));
		}
	}
	match app.member_details {
		None => panel.push(text("Member details unavailable.")),
		Some(members) => {
			panel.push(text(format!(
				"Loaded members: {}{}; showing first 5",
				members.items.len(),
				if members.truncated { " (partial)" } else { "" }
			)));
			panel.push(text(match members.roles {
				None => "Role catalog unavailable.".into(),
				Some(roles) => format!(
					"Loaded roles: {}{}; first 8: {}",
					roles.len(),
					if members.roles_truncated {
						" (partial)"
					} else {
						""
					},
					roles
						.iter()
						.take(8)
						.map(|role| format!("{} ({})", role.name, role.id))
						.collect::<Vec<_>>()
						.join(", ")
				),
			}));
			for member in members.items.iter().take(5) {
				let profile = member.profile.as_ref().map_or_else(
					|| "Server profile unavailable".into(),
					|profile| {
						format!(
							"Pronouns: {}\nBio: {}\nJoined: {}",
							profile.pronouns,
							profile.bio.chars().take(240).collect::<String>(),
							profile.joined_at.as_deref().unwrap_or("unknown")
						)
					},
				);
				panel.push(text(format!(
					"{} ({})\nNickname: {}\nRole IDs: {}{}\n{}",
					member.display_name,
					member.user.id,
					member.nick.as_deref().unwrap_or("not supplied"),
					member.role_ids.join(", "),
					if member.roles_truncated {
						" (partial)"
					} else {
						""
					},
					profile
				)));
			}
		}
	}
	panel.push(Element::Button {
		id: "show".into(),
		label: "Refresh loaded data".into(),
	});
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		..Default::default()
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	#[test]
	fn loaded_data_and_unknown_permissions_remain_distinct() {
		let input = serde_json::json!({
			"action": "show", "app": {
				"channel_metadata": {"channel_id": "1", "guild_id": "2", "topic": "Synthetic topic",
					"topic_truncated": false, "nsfw": false, "slowmode_seconds": 0,
					"thread": {"archived": false, "locked": null, "pinned": true},
					"permissions": {"send_messages": true, "manage_roles": false, "manage_threads": null}},
				"member_details": {"channel_id": "1", "guild_id": "2", "items": [{
					"user": {"id": "3", "name": "Example"}, "nick": "Nickname", "display_name": "Display",
					"role_ids": ["4"], "roles_truncated": false,
					"profile": {"bio": "Loaded server bio", "pronouns": "they/them"}}],
					"truncated": true, "roles": [{"id": "4", "name": "Member", "color": 0, "position": 1}],
					"roles_truncated": false}
			}
		});
		let run = |input: &serde_json::Value| -> AppOutput {
			serde_json::from_slice(
				&dispatch_typed(&serde_json::to_vec(input).unwrap(), handle).unwrap(),
			)
			.unwrap()
		};
		let output = run(&input);
		assert!(output.output.panel.len() <= 64);
		assert!(output.effects.is_empty() && output.output.storage.is_none());
		let shown = output
			.output
			.panel
			.iter()
			.filter_map(|element| match element {
				Element::Text { text } => Some(text.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n");
		for expected in [
			"Synthetic topic",
			"archived no; locked unknown; pinned yes",
			"SendMessages: yes",
			"ManageRoles: no",
			"ManageThreads: unknown",
			"Nickname",
			"Loaded server bio",
			"Member (4)",
		] {
			assert!(shown.contains(expected), "missing {expected}");
		}
		let unavailable = run(&serde_json::json!({"action": "show"}));
		assert!(unavailable.output.panel.iter().any(
			|element| matches!(element, Element::Text { text } if text == "Member details unavailable.")
		));
		assert_eq!(
			run(&serde_json::json!({"action": "on-app", "app_event": "roles"})),
			AppOutput::default()
		);
	}
}
