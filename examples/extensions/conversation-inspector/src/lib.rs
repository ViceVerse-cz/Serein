use serein_extension_sdk::{AppInvocation, AppOutput, Element, Output, PollAvailability};
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
			text: "Conversation inspector".into(),
		},
		text(
			"Already-loaded data only; partial lists are not service inventories. Showing at most 3 messages and threads.",
		),
	];
	panel.push(text(match input.host {
		Some(host) => format!(
			"Host revision {}: message_content support {}. Support does not imply a grant.",
			host.sdk_revision,
			host.supports("message_content")
		),
		None => "Host discovery unavailable (older host); optional snapshots may be absent.".into(),
	}));
	let app = input.app.unwrap_or_default();
	match app.message_content {
		None => panel.push(text("Rich message data unavailable.")),
		Some(messages) => {
			panel.push(text(format!(
				"Loaded rich messages: {}{}",
				messages.items.len(),
				if messages.truncated { " (partial)" } else { "" }
			)));
			for message in messages.items.iter().take(3) {
				let poll = match message.poll {
					PollAvailability::Absent => "absent",
					PollAvailability::Unsupported => {
						"present, unsupported: questions, options and results unavailable"
					}
				};
				panel.push(text(format!(
					"Message {}: embeds {}{}; suppressed {}\nPoll: {}",
					message.id,
					message.embeds.len(),
					if message.embeds_truncated {
						" (partial)"
					} else {
						""
					},
					message.embeds_suppressed,
					poll
				)));
				if let Some(embed) = message.embeds.first() {
					panel.push(text(format!(
						"First embed: {}\n{}\nImage {}; thumbnail {}; video {}; summary limited {}",
						embed.title.as_deref().unwrap_or("untitled"),
						embed.description.as_deref().unwrap_or("no description"),
						embed.has_image,
						embed.has_thumbnail,
						embed.has_video,
						embed.limited || embed.fields_truncated
					)));
				}
				panel.push(text(format!(
					"Stickers: {}{}",
					message
						.stickers
						.iter()
						.take(3)
						.map(|s| s.name.as_str())
						.collect::<Vec<_>>()
						.join(", "),
					if message.stickers_truncated {
						" (partial)"
					} else {
						""
					}
				)));
				if let Some(reference) = &message.reference {
					panel.push(text(format!(
						"Reference {}; deleted {}; forwarded {}",
						reference.message_id.as_deref().unwrap_or("unknown"),
						reference.deleted,
						reference.forwarded
					)));
				}
			}
		}
	}
	match app.forum_data {
		None => panel.push(text("Forum data unavailable.")),
		Some(forum) => {
			panel.push(text(format!(
				"Loaded threads under {}: {}{}",
				forum.parent_id,
				forum.posts.len(),
				if forum.truncated { " (partial)" } else { "" }
			)));
			for post in forum.posts.iter().take(3) {
				panel.push(text(format!(
					"{} ({})\nArchived {}; locked {}; pinned {}; followed {}",
					post.name,
					post.id,
					known(post.archived),
					known(post.locked),
					known(post.pinned),
					known(post.followed)
				)));
			}
		}
	}
	match app.conversation_activity {
		None => panel.push(text("Conversation activity unavailable.")),
		Some(activity) => {
			panel.push(text(format!(
				"Typing in {}: {}",
				activity.channel_id,
				activity
					.typing_user_ids
					.iter()
					.take(8)
					.map(String::as_str)
					.collect::<Vec<_>>()
					.join(", ")
			)));
			panel.push(text(match activity.pinned_message_ids {
				None => "Pinned message data unavailable.".into(),
				Some(ids) => format!(
					"Loaded pinned message IDs: {}{}",
					ids.iter()
						.take(8)
						.map(String::as_str)
						.collect::<Vec<_>>()
						.join(", "),
					if activity.pins_truncated {
						" (partial)"
					} else {
						""
					}
				),
			}));
		}
	}
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
	fn run(wire: &str) -> AppOutput {
		serde_json::from_slice(&dispatch_typed(wire.as_bytes(), handle).unwrap()).unwrap()
	}
	fn shown(output: &AppOutput) -> String {
		output
			.output
			.panel
			.iter()
			.filter_map(|e| match e {
				Element::Text { text } => Some(text.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n")
	}
	#[test]
	fn offline_fixtures_show_loaded_data_and_older_host_absence() {
		let output = run(include_str!("../fixtures/loaded.json"));
		assert!(
			output.output.panel.len() <= 64
				&& output.effects.is_empty()
				&& output.output.storage.is_none()
		);
		let text = shown(&output);
		for expected in [
			"message_content support true",
			"Synthetic release note",
			"unsupported: questions, options and results unavailable",
			"Wave",
			"Reference 99",
			"Synthetic topic",
			"locked unknown",
			"Typing in 20: 1, 2",
			"pinned message IDs: 100",
		] {
			assert!(text.contains(expected), "missing {expected}");
		}
		let older = shown(&run(include_str!("../fixtures/older-host.json")));
		for expected in [
			"Host discovery unavailable",
			"Rich message data unavailable",
			"Forum data unavailable",
			"Conversation activity unavailable",
		] {
			assert!(older.contains(expected), "missing {expected}");
		}
		assert_eq!(
			run(r#"{"action":"on-app","app_event":"typing"}"#),
			AppOutput::default()
		);
	}
}
