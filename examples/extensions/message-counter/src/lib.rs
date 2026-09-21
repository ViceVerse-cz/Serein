use serde::{Deserialize, Serialize};
use serein_extension_sdk::{Element, EventInvocation, MessageEventKind, Output};

// Store only bounded counters, never message content or identifiers.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Counts {
	create: u64,
	update: u64,
	delete: u64,
}

fn handle(input: EventInvocation) -> Output {
	let action = input.invocation.action.as_str();
	if !matches!(action, "message-event" | "show" | "reset") {
		return Output::default();
	}
	let counts = if action == "reset" {
		Ok(Counts::default())
	} else {
		input
			.invocation
			.storage_json::<Counts>()
			.map(Option::unwrap_or_default)
	};
	let mut output = Output::default();
	let text = match counts {
		Ok(mut counts) => {
			if action == "message-event" {
				let Some(event) = input.message_event else {
					return output;
				};
				let count = match event.kind {
					MessageEventKind::Create => &mut counts.create,
					MessageEventKind::Update => &mut counts.update,
					MessageEventKind::Delete => &mut counts.delete,
				};
				*count = count.saturating_add(1);
			}
			if action != "show" {
				output
					.set_storage_json(&counts)
					.expect("three u64 counters fit the storage limit");
			}
			if action == "message-event" {
				return output;
			}
			format!(
				"Created: {}\nUpdated: {}\nDeleted: {}",
				counts.create, counts.update, counts.delete
			)
		}
		Err(_) if action == "message-event" => return output,
		Err(_) => "Saved counts are invalid. Reset counts to start again.".into(),
	};
	output.panel = vec![
		Element::Heading {
			text: "Message counter".into(),
		},
		Element::Text { text },
		Element::Button {
			id: "reset".into(),
			label: "Reset counts".into(),
		},
	];
	output
}
serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{Invocation, MessageEvent, dispatch_typed, serde_json};

	fn run(action: &str, storage: Option<String>, kind: Option<MessageEventKind>) -> Output {
		let input = EventInvocation {
			invocation: Invocation {
				action: action.into(),
				storage,
				..Default::default()
			},
			message_event: kind.map(|kind| MessageEvent {
				kind,
				channel_id: "100".into(),
				message_id: "200".into(),
				author_id: (kind == MessageEventKind::Create).then(|| "300".into()),
				content: (kind == MessageEventKind::Create).then(|| "Synthetic message".into()),
			}),
		};
		let bytes = dispatch_typed(&serde_json::to_vec(&input).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	#[test]
	fn lifecycle_preserves_bad_storage_and_saturates_counts() {
		let mut storage = None;
		for kind in [
			MessageEventKind::Create,
			MessageEventKind::Update,
			MessageEventKind::Delete,
		] {
			let output = run("message-event", storage, Some(kind));
			assert!(output.panel.is_empty());
			assert!(output.replacement.is_none());
			storage = output.storage;
		}
		assert_eq!(
			storage.as_deref(),
			Some(r#"{"create":1,"update":1,"delete":1}"#)
		);
		let shown = run("show", storage, None);
		assert_eq!(
			shown.panel[1],
			Element::Text {
				text: "Created: 1\nUpdated: 1\nDeleted: 1".into()
			}
		);
		assert!(shown.storage.is_none());

		for bad in [
			"{",
			r#"{"create":-1,"update":0,"delete":0}"#,
			r#"{"create":18446744073709551616,"update":0,"delete":0}"#,
		] {
			assert_eq!(
				run(
					"message-event",
					Some(bad.into()),
					Some(MessageEventKind::Create)
				),
				Output::default()
			);
			let shown = run("show", Some(bad.into()), None);
			assert!(shown.storage.is_none());
			assert_eq!(
				shown.panel[1],
				Element::Text {
					text: "Saved counts are invalid. Reset counts to start again.".into()
				}
			);
			let reset = run("reset", Some(bad.into()), None);
			assert_eq!(
				reset.storage.as_deref(),
				Some(r#"{"create":0,"update":0,"delete":0}"#)
			);
			assert_eq!(
				reset.panel[1],
				Element::Text {
					text: "Created: 0\nUpdated: 0\nDeleted: 0".into()
				}
			);
		}
		let full = Counts {
			create: u64::MAX,
			..Default::default()
		};
		let full = serde_json::to_string(&full).unwrap();
		assert_eq!(
			run(
				"message-event",
				Some(full.clone()),
				Some(MessageEventKind::Create)
			)
			.storage,
			Some(full)
		);
		assert_eq!(run("message-event", None, None), Output::default());
		assert_eq!(run("unknown", None, None), Output::default());
	}
}
