//! Unofficial session-scoped interaction responses. Uncorrelatable events are ignored.
use client_core::{Event, auth::Failure, interactions};
use serde::Deserialize;

#[derive(Deserialize)]
struct Outcome {
	nonce: Option<Nonce>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Nonce {
	Text(String),
	Number(u64),
}
impl Nonce {
	fn into_string(self) -> String {
		match self {
			Self::Text(s) => s,
			Self::Number(n) => n.to_string(),
		}
	}
}
#[derive(Deserialize)]
struct Application {
	id: model::Id,
}
#[derive(Deserialize)]
struct Modal {
	id: model::Id,
	application: Application,
	custom_id: String,
	title: String,
	components: Vec<model::Component>,
}
pub(super) fn event(name: &str, bytes: &[u8]) -> Result<Option<Event>, Failure> {
	if bytes.len() > 256 * 1024 {
		return Ok(None);
	}
	let Ok(outcome) = serde_json::from_slice::<Outcome>(bytes) else {
		return Ok(None);
	};
	let Some(nonce) = outcome
		.nonce
		.map(Nonce::into_string)
		.filter(|n| !n.is_empty() && n.len() <= 64 && !n.chars().any(char::is_control))
	else {
		return Ok(None);
	};
	let event = match name {
		"INTERACTION_SUCCESS" => interactions::Event::Success { nonce },
		"INTERACTION_FAILURE" => interactions::Event::Failed { nonce },
		"INTERACTION_MODAL_CREATE" => {
			let Ok(modal) = serde_json::from_slice::<Modal>(bytes) else {
				return Ok(None);
			};
			if modal.id.0 == 0
				|| modal.application.id.0 == 0
				|| modal.title.is_empty()
				|| modal.title.chars().count() > 45
				|| modal.custom_id.is_empty()
				|| modal.custom_id.chars().count() > 100
				|| modal.components.len() > 5
				|| !model::valid_components(&modal.components)
			{
				return Ok(None);
			}
			interactions::Event::Modal {
				nonce,
				modal: Box::new(interactions::Modal {
					id: modal.id,
					application_id: modal.application.id,
					custom_id: modal.custom_id,
					title: modal.title,
					components: modal.components,
				}),
			}
		}
		_ => return Ok(None),
	};
	Ok(Some(Event::Interaction(event)))
}
