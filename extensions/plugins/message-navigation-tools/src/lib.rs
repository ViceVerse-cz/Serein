/*
 * Vencord, a Discord client mod
 * Copyright (c) 2024 Vendicated and contributors
 * Copyright (c) 2025 Vendicated and contributors
 * Copyright (c) 2026 Serein port contributors
 *
 * Portions adapted from Equicord's JumpTo (Samwich, thororen) and LastActive
 * (Crxa) at commit 200dba124066146a8f043fde7fc8b05e3dc5e68e.
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License version 3 or (at your
 * option) any later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */

use serein_extension_sdk::{
	AppInvocation, AppOutput, Element, HostEffect, MessageActionInvocation, MessageSnapshot,
	Output, TimelineSnapshot,
};

/// The host already caps the timeline snapshot; this is a defensive second cap.
const MAX_SCANNED_MESSAGES: usize = 50;

const MISSING_APP: &str =
	"Serein did not provide the current account and conversation, so navigation was not proposed.";
const MISSING_TIMELINE: &str = "Serein did not provide a loaded timeline for this conversation, so navigation was not proposed.";
const STALE_CONTEXT: &str = "The loaded timeline no longer belongs to the selected conversation. Reopen the action and try again.";
const MISSING_SELECTION: &str =
	"The selected message context is unavailable. Reopen the message menu and try again.";
const NO_MESSAGE: &str =
	"No matching message is present in the bounded window Serein has loaded. Nothing was proposed.";

struct Scope<'a> {
	channel_id: &'a str,
	own_id: &'a str,
	timeline: &'a TimelineSnapshot,
	last_message_id: Option<&'a str>,
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
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

fn no_effect(message: &str) -> AppOutput {
	panel_output(
		vec![
			Element::Heading {
				text: "Message navigation".into(),
			},
			text(message),
		],
		None,
	)
}

/// IDs must be nonzero decimal snowflakes in canonical form. Leading zeroes,
/// signs, whitespace and overlong values are stale or malformed, never guessed.
fn canonical_id(value: &str) -> bool {
	let Ok(id) = value.parse::<u64>() else {
		return false;
	};
	id != 0
		&& value.len() <= 20
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value == id.to_string()
}

fn scope(input: &AppInvocation) -> Result<Scope<'_>, &'static str> {
	let app = input.app.as_ref().ok_or(MISSING_APP)?;
	let context = app.context.as_ref().ok_or(MISSING_APP)?;
	if !context.connected {
		return Err(MISSING_APP);
	}
	let user = context.user.as_ref().ok_or(MISSING_APP)?;
	let channel = context.channel.as_ref().ok_or(MISSING_APP)?;
	if !canonical_id(&user.id) || !canonical_id(&channel.id) {
		return Err(STALE_CONTEXT);
	}
	let timeline = app.timeline.as_ref().ok_or(MISSING_TIMELINE)?;
	if timeline.channel_id != channel.id
		|| !canonical_id(&timeline.channel_id)
		|| timeline.messages.len() > MAX_SCANNED_MESSAGES
	{
		return Err(STALE_CONTEXT);
	}
	let last_message_id = match &app.channel_details {
		Some(details) => {
			if details.channel.id != channel.id {
				return Err(STALE_CONTEXT);
			}
			details.last_message_id.as_deref()
		}
		None => None,
	};
	Ok(Scope {
		channel_id: &channel.id,
		own_id: &user.id,
		timeline,
		last_message_id,
	})
}

/// Select the newest or oldest loaded message, optionally restricted to one author.
/// Malformed IDs are skipped rather than guessed.
fn find_message<'a>(
	messages: &'a [MessageSnapshot],
	author: Option<&str>,
	newest: bool,
) -> Option<&'a MessageSnapshot> {
	let matches = |message: &&MessageSnapshot| {
		canonical_id(&message.id) && author.is_none_or(|id| message.author.id == id)
	};
	if newest {
		messages.iter().rev().find(matches)
	} else {
		messages.iter().find(matches)
	}
}

fn jump(scope: &Scope<'_>, message_id: &str, detail: &str) -> AppOutput {
	if !canonical_id(message_id) {
		return no_effect(NO_MESSAGE);
	}
	panel_output(
		vec![
			Element::Heading {
				text: "Message navigation".into(),
			},
			text(detail),
			text(format!(
				"Loaded window: {} message(s). This proposes navigation only; nothing is sent or edited.",
				scope.timeline.messages.len()
			)),
		],
		Some(HostEffect::JumpToMessage {
			channel_id: scope.channel_id.to_owned(),
			message_id: message_id.to_owned(),
		}),
	)
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	let scope = match scope(&input.invocation) {
		Ok(scope) => scope,
		Err(error) => return no_effect(error),
	};
	match input.invocation.invocation.action.as_str() {
		"jump-latest" => {
			let message_id = scope
				.last_message_id
				.filter(|id| canonical_id(id))
				.or_else(|| {
					find_message(&scope.timeline.messages, None, true).map(|m| m.id.as_str())
				});
			match message_id {
				Some(id) => jump(
					&scope,
					id,
					"The latest message in this conversation is proposed from loaded channel details.",
				),
				None => no_effect(NO_MESSAGE),
			}
		}
		"jump-oldest-loaded" => match find_message(&scope.timeline.messages, None, false) {
			Some(message) => jump(
				&scope,
				&message.id,
				"The oldest message in Serein's loaded window is proposed. Older history is not fetched.",
			),
			None => no_effect(NO_MESSAGE),
		},
		"jump-me-latest" => {
			match find_message(&scope.timeline.messages, Some(scope.own_id), true) {
				Some(message) => jump(
					&scope,
					&message.id,
					"Your latest message in the loaded window is proposed. Older history is not fetched.",
				),
				None => no_effect(NO_MESSAGE),
			}
		}
		"jump-author-latest" | "jump-author-oldest" => {
			let Some(reference) = input.selected_message_ref.as_ref() else {
				return no_effect(MISSING_SELECTION);
			};
			if reference.channel_id != scope.channel_id || !canonical_id(&reference.author_id) {
				return no_effect(STALE_CONTEXT);
			}
			let newest = input.invocation.invocation.action == "jump-author-latest";
			match find_message(
				&scope.timeline.messages,
				Some(reference.author_id.as_str()),
				newest,
			) {
				Some(message) => jump(
					&scope,
					&message.id,
					if newest {
						"This author's latest message in the loaded window is proposed. Older history is not fetched."
					} else {
						"This author's earliest message in the loaded window is proposed. Older history is not fetched."
					},
				),
				None => no_effect(NO_MESSAGE),
			}
		}
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{
		AppContextSnapshot, AppSnapshot, ChannelDetailsSnapshot, ChannelSnapshot, MessageSnapshot,
		SelectedMessageRef, UserSnapshot,
	};

	fn message(id: u64, author: u64) -> MessageSnapshot {
		MessageSnapshot {
			id: id.to_string(),
			author: UserSnapshot {
				id: author.to_string(),
				name: format!("user {author}"),
			},
			content: String::new(),
			attachment_count: 0,
			edited: false,
		}
	}

	fn invocation(action: &str) -> MessageActionInvocation {
		MessageActionInvocation {
			invocation: AppInvocation {
				invocation: serein_extension_sdk::Invocation {
					action: action.into(),
					..Default::default()
				},
				app: Some(AppSnapshot {
					context: Some(AppContextSnapshot {
						connected: true,
						user: Some(UserSnapshot {
							id: "1".into(),
							name: "me".into(),
						}),
						channel: Some(ChannelSnapshot {
							id: "20".into(),
							guild_id: Some("2".into()),
							name: "general".into(),
							kind: 0,
						}),
					}),
					timeline: Some(TimelineSnapshot {
						channel_id: "20".into(),
						messages: vec![message(100, 3), message(101, 1), message(102, 3)],
						truncated: true,
					}),
					channel_details: Some(ChannelDetailsSnapshot {
						channel: ChannelSnapshot {
							id: "20".into(),
							guild_id: Some("2".into()),
							name: "general".into(),
							kind: 0,
						},
						parent_id: None,
						position: 0,
						last_message_id: Some("103".into()),
						message_count: None,
						recipients: Vec::new(),
						recipients_truncated: false,
						can_send: true,
						can_read_history: true,
					}),
					..Default::default()
				}),
				..Default::default()
			},
			..Default::default()
		}
	}

	fn jumped(output: &AppOutput) -> Option<(&str, &str)> {
		match output.effects.as_slice() {
			[
				HostEffect::JumpToMessage {
					channel_id,
					message_id,
				},
			] => Some((channel_id.as_str(), message_id.as_str())),
			_ => None,
		}
	}

	#[test]
	fn latest_uses_channel_details_and_oldest_uses_the_loaded_window() {
		let latest = handle(invocation("jump-latest"));
		assert_eq!(jumped(&latest), Some(("20", "103")));
		assert!(latest.output.replacement.is_none());

		let oldest = handle(invocation("jump-oldest-loaded"));
		assert_eq!(jumped(&oldest), Some(("20", "100")));
	}

	#[test]
	fn author_actions_use_the_selected_author_and_never_guess() {
		let mut latest = invocation("jump-author-latest");
		latest.selected_message_ref = Some(SelectedMessageRef {
			channel_id: "20".into(),
			message_id: "100".into(),
			author_id: "3".into(),
			guild_id: Some("2".into()),
		});
		assert_eq!(jumped(&handle(latest)), Some(("20", "102")));

		let mut oldest = invocation("jump-author-oldest");
		oldest.selected_message_ref = Some(SelectedMessageRef {
			channel_id: "20".into(),
			message_id: "100".into(),
			author_id: "3".into(),
			guild_id: Some("2".into()),
		});
		assert_eq!(jumped(&handle(oldest)), Some(("20", "100")));

		let mut absent = invocation("jump-author-latest");
		absent.selected_message_ref = Some(SelectedMessageRef {
			channel_id: "20".into(),
			message_id: "100".into(),
			author_id: "99".into(),
			guild_id: Some("2".into()),
		});
		assert!(jumped(&handle(absent)).is_none());
	}

	#[test]
	fn stale_or_mismatched_context_proposes_nothing() {
		let mut stale = invocation("jump-me-latest");
		stale
			.invocation
			.app
			.as_mut()
			.unwrap()
			.timeline
			.as_mut()
			.unwrap()
			.channel_id = "21".into();
		assert!(jumped(&handle(stale)).is_none());

		let mut foreign = invocation("jump-author-latest");
		foreign.selected_message_ref = Some(SelectedMessageRef {
			channel_id: "21".into(),
			message_id: "100".into(),
			author_id: "3".into(),
			guild_id: Some("2".into()),
		});
		assert!(jumped(&handle(foreign)).is_none());

		let mut malformed = invocation("jump-latest");
		malformed
			.invocation
			.app
			.as_mut()
			.unwrap()
			.channel_details
			.as_mut()
			.unwrap()
			.last_message_id = Some("0103".into());
		// Falls back to the newest loaded message instead of accepting a non-canonical ID.
		assert_eq!(jumped(&handle(malformed)), Some(("20", "102")));
	}
}
