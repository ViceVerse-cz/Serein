use serein_extension_sdk::{AppAction, AppInvocation, AppOutput, Element, HostEffect, Output};

const OPERATIONS: &[&str] = &[
	"Send message",
	"Edit message",
	"Delete message",
	"Add reaction",
	"Remove reaction",
	"Pin message",
	"Unpin message",
	"Mark channel read",
	"Mark message unread",
	"Create thread",
];

fn note(text: &str) -> AppOutput {
	AppOutput {
		output: Output {
			panel: vec![Element::Text { text: text.into() }],
			..Default::default()
		},
		..Default::default()
	}
}

fn field(id: &str, label: &str, value: &str) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: value.into(),
	}
}

fn handle(input: AppInvocation) -> AppOutput {
	if input.app_event.is_some() || input.message_event.is_some() {
		return AppOutput::default();
	}
	let Some(channel) = input
		.app
		.as_ref()
		.and_then(|app| app.context.as_ref())
		.and_then(|context| context.channel.as_ref())
	else {
		return note("Select an accessible conversation first.");
	};
	let channel_id = channel.id.clone();
	match input.invocation.action.as_str() {
		"show" => {
			let message = input
				.app
				.as_ref()
				.and_then(|app| app.timeline.as_ref())
				.and_then(|timeline| timeline.messages.last());
			AppOutput {
				output: Output {
					panel: vec![
						Element::Heading {
							text: "Conversation actions".into(),
						},
						Element::Text {
							text: format!(
								"Conversation: {} ({channel_id}). Review proposes one action; Serein's Apply button performs it. Deletion cannot be undone. Sending preserves your existing draft and attachments.",
								channel.name
							),
						},
						Element::Select {
							id: "operation".into(),
							label: "Action".into(),
							options: OPERATIONS.iter().map(|value| (*value).into()).collect(),
							value: "Mark channel read".into(),
						},
						field(
							"channel",
							"Conversation ID (must remain selected)",
							&channel_id,
						),
						field(
							"message",
							"Loaded message ID (for message actions)",
							message.map_or("", |message| message.id.as_str()),
						),
						field("text", "Message text or new thread name", ""),
						field("emoji", "Reaction: Unicode emoji or name:id", "👍"),
						Element::Button {
							id: "propose".into(),
							label: "Review action".into(),
						},
					],
					..Default::default()
				},
				..Default::default()
			}
		}
		"propose" => {
			if input.invocation.value("channel") != Some(channel_id.as_str()) {
				return note("The conversation changed. Open the tool again.");
			}
			let message_id = input
				.invocation
				.value("message")
				.unwrap_or_default()
				.to_owned();
			let content = input
				.invocation
				.value("text")
				.unwrap_or_default()
				.to_owned();
			let action = match input.invocation.value("operation") {
				Some("Send message") => AppAction::SendMessage {
					channel_id,
					content,
				},
				Some("Edit message") => AppAction::EditMessage {
					channel_id,
					message_id,
					content,
				},
				Some("Delete message") => AppAction::DeleteMessage {
					channel_id,
					message_id,
				},
				Some(operation @ ("Add reaction" | "Remove reaction")) => AppAction::SetReaction {
					channel_id,
					message_id,
					emoji: input.invocation.value("emoji").unwrap_or_default().into(),
					add: operation == "Add reaction",
				},
				Some(operation @ ("Pin message" | "Unpin message")) => {
					AppAction::SetMessagePinned {
						channel_id,
						message_id,
						pinned: operation == "Pin message",
					}
				}
				Some("Mark channel read") => AppAction::MarkChannelRead { channel_id },
				Some("Mark message unread") => AppAction::MarkUnread {
					channel_id,
					message_id,
				},
				Some("Create thread") => AppAction::CreateThread {
					channel_id,
					name: content,
					message_id: (!message_id.is_empty()).then_some(message_id),
				},
				_ => return note("Choose an action first."),
			};
			AppOutput {
				effects: vec![HostEffect::AppAction { action }],
				..Default::default()
			}
		}
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	#[test]
	fn proposal_is_explicit_and_rejects_navigation_and_background_events() {
		let source = r#"{"action":"propose","values":{"operation":"Send message","channel":"20","text":"Hello"},"app":{"context":{"connected":true,"channel":{"id":"20","name":"general","kind":0}}}}"#;
		let result = dispatch_typed(source.as_bytes(), handle).unwrap();
		let result: AppOutput = serde_json::from_slice(&result).unwrap();
		assert!(
			matches!(&result.effects[..], [HostEffect::AppAction { action: AppAction::SendMessage { channel_id, content } }] if channel_id == "20" && content == "Hello")
		);
		let mut input: AppInvocation = serde_json::from_str(source).unwrap();
		input
			.invocation
			.values
			.insert("channel".into(), "21".into());
		assert!(handle(input.clone()).effects.is_empty());
		input.app_event = Some(serein_extension_sdk::AppEventKind::Ready);
		assert!(handle(input).effects.is_empty());
	}
}
