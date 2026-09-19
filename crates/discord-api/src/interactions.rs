//! Unofficial normal-user interaction submission; HTTP acceptance is not bot completion.
use crate::{DiscordApi, Failure};
use client_core::interactions::{Data, Request};
use serde_json::{Value, json};

fn component(c: &model::Component, depth: usize) -> Result<Value, Failure> {
	if depth > 8 {
		return Err(Failure::Capacity);
	}
	let mut out = json!({"type":c.kind});
	if c.id != 0 {
		out["id"] = json!(c.id);
	}
	match c.kind {
		1 => {
			out["components"] = c
				.components
				.iter()
				.filter(|c| c.kind != 10)
				.map(|c| component(c, depth + 1))
				.collect::<Result<Vec<_>, _>>()?
				.into()
		}
		18 => {
			out["component"] =
				component(c.component.as_deref().ok_or(Failure::Protocol)?, depth + 1)?
		}
		3..=8 | 19 | 21..=23 => {
			out["custom_id"] = json!(c.custom_id.as_ref().ok_or(Failure::Protocol)?);
			match c.kind {
				4 => out["value"] = json!(c.value.as_deref().unwrap_or_default()),
				21 => out["value"] = json!(c.value),
				23 => out["value"] = json!(c.checked.unwrap_or(false)),
				_ => out["values"] = json!(c.values),
			}
		}
		_ => return Err(Failure::Protocol),
	}
	Ok(out)
}

pub(crate) fn valid_uploads(request: &Request, count: usize) -> bool {
	fn walk(
		c: &model::Component,
		count: usize,
		seen: &mut std::collections::BTreeSet<usize>,
	) -> bool {
		if c.kind == 19
			&& !c.values.iter().all(|v| {
				v.parse::<usize>()
					.ok()
					.is_some_and(|i| i < count && i.to_string() == *v && seen.insert(i))
			}) {
			return false;
		}
		c.components
			.iter()
			.chain(c.component.as_deref())
			.all(|c| walk(c, count, seen))
	}
	let mut seen = std::collections::BTreeSet::new();
	match &request.data {
		Data::Modal { components, .. } => {
			components.iter().all(|c| walk(c, count, &mut seen)) && seen.len() == count
		}
		_ => count == 0,
	}
}

impl DiscordApi {
	pub fn interaction_session(
		&self,
		session: Option<zeroize::Zeroizing<String>>,
	) -> Result<(), Failure> {
		*self
			.interaction_session
			.lock()
			.map_err(|_| Failure::Protocol)? = session;
		Ok(())
	}
	pub(crate) async fn interaction(
		&self,
		request: &Request,
		attachments: Option<Vec<Value>>,
	) -> Result<(), Failure> {
		if !request.valid() || !valid_uploads(request, attachments.as_ref().map_or(0, Vec::len)) {
			return Err(Failure::ProtocolAt("Invalid interaction; nothing was sent"));
		}
		let session = self
			.interaction_session
			.lock()
			.map_err(|_| Failure::Protocol)?
			.clone()
			.ok_or(Failure::ProtocolAt(
				"Interaction unavailable while reconnecting",
			))?;
		let (kind, mut data) = match &request.data {
			Data::Component {
				custom_id,
				component_type,
				values,
			} => {
				let mut data = json!({"custom_id":custom_id,"component_type":component_type});
				if *component_type != 2 {
					data["values"] = json!(values);
				}
				(3, data)
			}
			Data::Modal {
				id,
				custom_id,
				components,
			} => {
				let components = components
					.iter()
					.filter(|c| c.kind != 10)
					.map(|c| component(c, 0))
					.collect::<Result<Vec<_>, _>>()?;
				(
					5,
					json!({"id":id,"custom_id":custom_id,"components":components}),
				)
			}
		};
		if let Some(attachments) = attachments {
			if kind != 5 {
				return Err(Failure::Protocol);
			}
			data["attachments"] = attachments.into();
		}
		let mut body = json!({"type":kind,"application_id":request.application_id,"channel_id":request.channel_id,"session_id":session.as_str(),"nonce":request.nonce,"data":data});
		if let Some(guild) = request.guild_id {
			body["guild_id"] = json!(guild);
		}
		if kind == 3 {
			body["message_id"] = json!(request.message_id.ok_or(Failure::Protocol)?);
			body["message_flags"] = json!(request.message_flags);
		}
		if serde_json::to_vec(&body)
			.map_err(|_| Failure::Protocol)?
			.len() > 256 * 1024
		{
			return Err(Failure::ProtocolAt(
				"Interaction is too large; nothing was sent",
			));
		}
		self.request_limited(
			reqwest::Method::POST,
			"/interactions",
			Some(body),
			64 * 1024,
		)
		.await
		.map(|_| ())
	}
}

pub(crate) fn valid_file_types(request: &Request, sources: &[crate::upload::Source]) -> bool {
	fn valid(component: &model::Component, sources: &[crate::upload::Source]) -> bool {
		(component.kind != 19
			|| component.values.iter().all(|value| {
				value
					.parse::<usize>()
					.ok()
					.and_then(|i| sources.get(i))
					.is_some_and(|source| component.accepts_file(source.filename()))
			})) && component
			.components
			.iter()
			.chain(component.component.as_deref())
			.all(|c| valid(c, sources))
	}
	match &request.data {
		Data::Modal { components, .. } => components.iter().all(|c| valid(c, sources)),
		_ => sources.is_empty(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn modal_submission_projects_input_values_without_schema_metadata() {
		let input = model::Component {
			kind: 23,
			custom_id: Some("agree".into()),
			checked: Some(true),
			label: Some("Terms".into()),
			..Default::default()
		};
		let label = model::Component {
			kind: 18,
			component: Some(Box::new(input)),
			..Default::default()
		};
		assert_eq!(
			component(&label, 0).unwrap(),
			json!({"type":18,"component":{"type":23,"custom_id":"agree","value":true}})
		);
		assert!(
			component(
				&model::Component {
					kind: 255,
					..Default::default()
				},
				0
			)
			.is_err()
		);
	}
}
