/*
 * Vencord, a modification for Discord's desktop app
 * Copyright (c) 2023 Vendicated and contributors
 * Copyright (c) 2024 Vendicated and contributors
 * Copyright (c) 2026 Vendicated and contributors
 *
 * Serein adaptation copyright (c) 2026 Serein contributors.
 * Changed November 8, 2026: only explicitly granted selected-message, loaded
 * timeline/details, navigation, browser-link and reaction data cross the ABI.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppAction, AppOutput, Element, HostEffect, MAX_IO_BYTES, MessageActionInvocation, Output,
	serde_json,
};
use url::Url;

/// The selected text is bounded before it becomes a search query.
const MAX_QUERY_BYTES: usize = 1024;
/// Generated browser URLs stay within this byte ceiling before host validation.
const MAX_URL_BYTES: usize = 2048;
/// The upstream Husk default: a custom emoji named `husk`.
const HUSK_EMOJI: &str = "husk:1026532993923293184";
const SEARCH_BASE: &str = "https://www.google.com/search";
const SEARCH_HOST: &str = "www.google.com";

const MISSING_IDS: &str =
	"Message IDs are unavailable on this host, so no loaded message can be inspected.";
const NO_LOADED_DATA: &str = "Serein has not loaded this chat's timeline or message details, so there is nothing safe to inspect.";
const SCOPE_MISMATCH: &str =
	"The selected message no longer matches this loaded chat. Reopen the message and try again.";

#[derive(Clone, Debug)]
struct LoadedMessage {
	id: String,
	reply_to: Option<String>,
	order: usize,
}

#[derive(Clone, Debug)]
struct Loaded {
	messages: Vec<LoadedMessage>,
	details_present: bool,
	details_truncated: bool,
	timeline_truncated: bool,
}

impl Loaded {
	fn find(&self, id: &str) -> Option<&LoadedMessage> {
		self.messages.iter().find(|message| message.id == id)
	}

	fn earliest_reply(&self, message_id: &str) -> Option<&LoadedMessage> {
		self.messages
			.iter()
			.filter(|message| {
				message.id != message_id && message.reply_to.as_deref() == Some(message_id)
			})
			.min_by_key(|message| message.order)
	}

	fn is_truncated(&self) -> bool {
		self.details_truncated || self.timeline_truncated
	}
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

fn error_output(message: &str) -> AppOutput {
	panel_output(
		vec![
			Element::Heading {
				text: "Message reference tools".into(),
			},
			text(message),
		],
		None,
	)
}

fn is_snowflake(value: &str) -> bool {
	value.len() <= 20
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

/// The selected IDs must exist, be usable snowflakes and agree with any loaded
/// snapshot channel. A mismatch is stale data, never a reason to guess.
fn selected_reference(
	input: &MessageActionInvocation,
) -> Result<&serein_extension_sdk::SelectedMessageRef, &'static str> {
	let reference = input.selected_message_ref.as_ref().ok_or(MISSING_IDS)?;
	if !is_snowflake(&reference.channel_id)
		|| !is_snowflake(&reference.message_id)
		|| !is_snowflake(&reference.author_id)
	{
		return Err(MISSING_IDS);
	}
	if let Some(app) = &input.invocation.app
		&& (app
			.timeline
			.as_ref()
			.is_some_and(|timeline| timeline.channel_id != reference.channel_id)
			|| app
				.message_details
				.as_ref()
				.is_some_and(|details| details.channel_id != reference.channel_id))
	{
		return Err(SCOPE_MISMATCH);
	}
	Ok(reference)
}

/// Merge only the host-provided bounded rows. No extra history is requested and
/// no page is fetched. Timeline order wins; details supply reply metadata.
fn loaded_messages(input: &MessageActionInvocation) -> Result<Loaded, &'static str> {
	let reference = selected_reference(input)?;
	let app = input.invocation.app.as_ref().ok_or(NO_LOADED_DATA)?;
	let timeline = app.timeline.as_ref();
	let details = app.message_details.as_ref();
	if timeline.is_none() && details.is_none() {
		return Err(NO_LOADED_DATA);
	}
	let mut loaded = Loaded {
		messages: Vec::new(),
		details_present: details.is_some(),
		details_truncated: details.is_some_and(|details| details.truncated),
		timeline_truncated: timeline.is_some_and(|timeline| timeline.truncated),
	};
	let mut order = 0;
	let mut merge = |id: &str, reply_to: Option<&str>, order: usize| {
		if let Some(existing) = loaded.messages.iter_mut().find(|item| item.id == id) {
			if existing.reply_to.is_none() {
				existing.reply_to = reply_to.map(str::to_owned);
			}
			return;
		}
		loaded.messages.push(LoadedMessage {
			id: id.to_owned(),
			reply_to: reply_to.map(str::to_owned),
			order,
		});
	};
	if let Some(timeline) = timeline {
		for message in &timeline.messages {
			merge(&message.id, None, order);
			order += 1;
		}
	}
	if let Some(details) = details {
		for item in &details.items {
			merge(&item.id, item.reply_to.as_deref(), order);
			order += 1;
		}
	}
	if loaded.find(&reference.message_id).is_none() {
		return Err(
			"The selected message is not in the bounded window Serein has loaded. It may have moved or the chat may need to be reopened.",
		);
	}
	Ok(loaded)
}

fn find_reply(input: &MessageActionInvocation) -> AppOutput {
	let reference = match selected_reference(input) {
		Ok(reference) => reference,
		Err(message) => return error_output(message),
	};
	let loaded = match loaded_messages(input) {
		Ok(loaded) => loaded,
		Err(message) => return error_output(message),
	};
	if !loaded.details_present {
		return error_output(
			"Serein did not provide loaded reply metadata for this chat, so no reference target can be identified.",
		);
	}
	let selected = match loaded.find(&reference.message_id) {
		Some(selected) => selected,
		None => return error_output("The selected message is no longer loaded."),
	};
	let parent = selected
		.reply_to
		.as_deref()
		.filter(|id| *id != reference.message_id)
		.and_then(|id| loaded.find(id));
	let reply = loaded.earliest_reply(&reference.message_id);
	let (target, description) = match (parent, reply) {
		(Some(parent), _) => (
			parent,
			"Nearest loaded original in the reply chain".to_owned(),
		),
		(None, Some(reply)) => (
			reply,
			"Earliest loaded direct reply to the selected message".to_owned(),
		),
		(None, None) => {
			return panel_output(
				vec![
					Element::Heading {
						text: "Find reply".into(),
					},
					text(format!(
						"Selected loaded message: {}.",
						reference.message_id
					)),
					text(
						"No loaded original or direct reply was found in Serein's bounded message window. Nothing was proposed.",
					),
					text(
						"Only already-loaded messages were inspected. This does not fetch, scan older history or claim a complete reply list.",
					),
				],
				None,
			);
		}
	};
	let mut panel = vec![
		Element::Heading {
			text: "Find reply".into(),
		},
		text(format!(
			"Selected loaded message: {}.",
			reference.message_id
		)),
		text(format!(
			"{description}: {} (channel {}).",
			target.id, reference.channel_id
		)),
	];
	if loaded.is_truncated() {
		panel.push(text(
			"Serein reports that its loaded message window may be incomplete; only the provided bounded rows were inspected.",
		));
	}
	panel.extend([
		text(
			"Only messages already loaded in this chat were inspected. This does not fetch history or claim that every reply is known.",
		),
		text("Apply jumps to the proposed loaded message."),
	]);
	panel_output(
		panel,
		Some(HostEffect::JumpToMessage {
			channel_id: reference.channel_id.clone(),
			message_id: target.id.clone(),
		}),
	)
}

/// Trim, reject empty or non-text control input, then cap to 1024 UTF-8 bytes
/// without splitting a character.
fn bounded_query(value: &str) -> Option<(String, bool)> {
	let trimmed = value.trim();
	if trimmed.is_empty()
		|| trimmed
			.chars()
			.any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
	{
		return None;
	}
	let mut end = trimmed.len().min(MAX_QUERY_BYTES);
	while end > 0 && !trimmed.is_char_boundary(end) {
		end -= 1;
	}
	let bounded = trimmed[..end].trim_end();
	(!bounded.is_empty()).then(|| (bounded.to_owned(), bounded.len() < trimmed.len()))
}

fn trusted_search_url(value: &str) -> bool {
	value.len() <= MAX_URL_BYTES
		&& !value
			.chars()
			.any(|ch| ch.is_control() || ch.is_ascii_whitespace() || ch == '\\')
		&& Url::parse(value).is_ok_and(|url| {
			url.scheme() == "https"
				&& url.host_str() == Some(SEARCH_HOST)
				&& url.username().is_empty()
				&& url.password().is_none()
				&& url.port().is_none()
		})
}

fn search_url_once(query: &str) -> Option<String> {
	let mut url = Url::parse(SEARCH_BASE).ok()?;
	url.query_pairs_mut().append_pair("q", query);
	Some(url.to_string())
}

/// Build one fixed-host HTTPS Google search URL. A bounded binary search avoids
/// reconstructing and encoding a long query once per dropped character.
fn search_url_with(query: &str, build: impl Fn(&str) -> Option<String>) -> Option<(String, bool)> {
	if let Some(url) = build(query)
		&& trusted_search_url(&url)
	{
		return Some((url, false));
	}
	let mut boundaries = Vec::with_capacity(query.len() + 1);
	boundaries.push(0);
	boundaries.extend(
		query
			.char_indices()
			.map(|(offset, character)| offset + character.len_utf8()),
	);
	let mut low = 1;
	let mut high = boundaries.len().saturating_sub(1);
	let mut best = None;
	while low <= high {
		let middle = low + (high - low) / 2;
		let candidate = query[..boundaries[middle]].trim_end();
		if candidate.is_empty() {
			low = middle + 1;
			continue;
		}
		match build(candidate) {
			Some(url) if trusted_search_url(&url) => {
				best = Some(url);
				low = middle + 1;
			}
			_ => high = middle - 1,
		}
	}
	best.map(|url| (url, true))
}

fn search_url(query: &str) -> Option<(String, bool)> {
	search_url_with(query, search_url_once)
}

fn google_that(input: &MessageActionInvocation) -> AppOutput {
	if let Err(message) = selected_reference(input) {
		return error_output(message);
	}
	let Some((query, query_shortened)) = input
		.invocation
		.invocation
		.selected_message
		.as_deref()
		.and_then(bounded_query)
	else {
		return error_output(
			"Select a message with non-empty text. Empty or control-only text produces no search URL and no effect.",
		);
	};
	let Some((url, url_shortened)) = search_url(&query) else {
		return error_output("Could not build a safe Google search URL. Nothing was opened.");
	};
	let mut panel = vec![
		Element::Heading {
			text: "Google That".into(),
		},
		text("Proposed Google search for the selected message text:"),
		text(format!("Reviewed public URL: {url}")),
		text(
			"Apply opens this reviewed HTTPS search in your browser. Serein does not fetch the page, replace the message or send anything.",
		),
	];
	if query_shortened || url_shortened {
		panel.push(text(
			"The query was shortened to keep the reviewed URL within the extension's byte bounds.",
		));
	}
	panel_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::OpenExternalLink { url },
		}),
	)
}

fn husk(input: &MessageActionInvocation) -> AppOutput {
	let reference = match selected_reference(input) {
		Ok(reference) => reference,
		Err(message) => return error_output(message),
	};
	let loaded = match loaded_messages(input) {
		Ok(loaded) => loaded,
		Err(message) => return error_output(message),
	};
	if loaded.find(&reference.message_id).is_none() {
		return error_output(
			"The selected message is no longer in Serein's loaded window. Reopen the message and try again.",
		);
	}
	panel_output(
		vec![
			Element::Heading {
				text: "Husk reaction".into(),
			},
			text(format!(
				"Selected loaded message: {} (channel {}).",
				reference.message_id, reference.channel_id
			)),
			text("Proposed reaction: Husk custom emoji (husk)."),
			text(
				"Apply adds this exact reaction once. Adding an existing reaction is a no-op; this never removes or toggles a reaction.",
			),
			text(
				"The server's same-named emoji is not looked up because Serein exposes no emoji store to this plugin.",
			),
		],
		Some(HostEffect::AppAction {
			action: AppAction::SetReaction {
				channel_id: reference.channel_id.clone(),
				message_id: reference.message_id.clone(),
				emoji: HUSK_EMOJI.into(),
				add: true,
			},
		}),
	)
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	let output = match input.invocation.invocation.action.as_str() {
		"find-reply" => find_reply(&input),
		"search" => google_that(&input),
		"husk" => husk(&input),
		_ => AppOutput::default(),
	};
	match serde_json::to_vec(&output) {
		Ok(bytes) if bytes.len() <= MAX_IO_BYTES => output,
		_ => error_output(
			"The response exceeded the extension's 256 KiB limit. Nothing was proposed.",
		),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::dispatch_typed;
	use serein_extension_sdk::serde_json::{self, json};

	fn run(value: serde_json::Value) -> AppOutput {
		let bytes = dispatch_typed(&serde_json::to_vec(&value).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	fn reference(channel: &str, message: &str) -> serde_json::Value {
		json!({
			"channel_id": channel,
			"message_id": message,
			"author_id": "10"
		})
	}

	fn loaded_app() -> serde_json::Value {
		json!({
			"timeline": {
				"channel_id": "100",
				"messages": [
					{"id":"200","author":{"id":"10","name":"Root"},"content":"root","attachment_count":0,"edited":false},
					{"id":"300","author":{"id":"11","name":"Reply"},"content":"reply","attachment_count":0,"edited":false},
					{"id":"400","author":{"id":"12","name":"Later"},"content":"later","attachment_count":0,"edited":false}
				],
				"truncated": true
			},
			"message_details": {
				"channel_id": "100",
				"items": [
					{"id":"200","kind":0,"mention_ids":[],"mentions_truncated":false,"mention_everyone":false,
					 "attachments":[],"attachments_truncated":false,"reactions":[],"reactions_truncated":false},
					{"id":"300","kind":0,"reply_to":"200","mention_ids":[],"mentions_truncated":false,
					 "mention_everyone":false,"attachments":[],"attachments_truncated":false,
					 "reactions":[{"emoji_name":"husk","count":1,"me":true,"me_burst":false}],
					 "reactions_truncated":false},
					{"id":"400","kind":0,"reply_to":"200","mention_ids":[],"mentions_truncated":false,
					 "mention_everyone":false,"attachments":[],"attachments_truncated":false,
					 "reactions":[],"reactions_truncated":false}
				],
				"truncated": true
			}
		})
	}

	fn action(action: &str, message: &str) -> serde_json::Value {
		json!({
			"action": action,
			"selected_message": "Synthetic selected text",
			"selected_message_ref": reference("100", message),
			"app": loaded_app()
		})
	}

	#[test]
	fn find_reply_proposes_loaded_parent_then_earliest_direct_reply() {
		let parent = run(action("find-reply", "300"));
		assert_eq!(
			parent.effects,
			vec![HostEffect::JumpToMessage {
				channel_id: "100".into(),
				message_id: "200".into()
			}]
		);
		assert!(parent.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("Nearest loaded original")
		)));
		assert!(parent.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("does not fetch history")
		)));

		let reply = run(action("find-reply", "200"));
		assert_eq!(
			reply.effects,
			vec![HostEffect::JumpToMessage {
				channel_id: "100".into(),
				message_id: "300".into()
			}]
		);
		assert!(reply.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("Earliest loaded direct reply")
		)));
	}

	#[test]
	fn find_reply_rejects_stale_unloaded_and_mismatched_selection() {
		for value in [
			{
				let mut value = action("find-reply", "999");
				value["selected_message_ref"] = reference("100", "999");
				value
			},
			{
				let mut value = action("find-reply", "300");
				value["selected_message_ref"] = reference("999", "300");
				value
			},
			{
				let mut value = action("find-reply", "300");
				value.as_object_mut().unwrap().remove("app");
				value
			},
		] {
			let output = run(value);
			assert!(output.effects.is_empty());
			assert!(output.output.storage.is_none());
			assert!(output.output.panel.iter().any(|element| matches!(
				element,
				Element::Text { text } if text.contains("not in the bounded window")
					|| text.contains("no longer matches")
					|| text.contains("has not loaded")
			)));
		}
	}

	#[test]
	fn find_reply_without_a_loaded_target_does_not_guess() {
		let mut value = action("find-reply", "400");
		for item in value["app"]["message_details"]["items"]
			.as_array_mut()
			.unwrap()
		{
			item.as_object_mut().unwrap().remove("reply_to");
		}
		let output = run(value);
		assert!(output.effects.is_empty());
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("Nothing was proposed")
		)));

		let mut value = action("find-reply", "300");
		value["app"]
			.as_object_mut()
			.unwrap()
			.remove("message_details");
		let output = run(value);
		assert!(output.effects.is_empty());
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("reply metadata")
		)));
	}

	#[test]
	fn google_that_builds_one_bounded_allowlisted_https_url() {
		let output = run(action("search", "300"));
		assert_eq!(output.effects.len(), 1);
		let HostEffect::AppAction {
			action: AppAction::OpenExternalLink { url },
		} = &output.effects[0]
		else {
			panic!("expected one reviewed external link");
		};
		assert!(url.starts_with("https://www.google.com/search?q="));
		assert!(url.contains("Synthetic+selected+text"));
		let parsed = Url::parse(url).unwrap();
		assert_eq!(parsed.scheme(), "https");
		assert_eq!(parsed.host_str(), Some("www.google.com"));
		assert!(parsed.port().is_none());
		assert!(output.output.storage.is_none());
		assert!(output.output.replacement.is_none());

		for query in ["x".repeat(5000), "🦀".repeat(300)] {
			let mut value = action("search", "300");
			value["selected_message"] = json!(query);
			let output = run(value);
			let HostEffect::AppAction {
				action: AppAction::OpenExternalLink { url },
			} = &output.effects[0]
			else {
				panic!("expected one reviewed external link");
			};
			assert!(url.len() <= MAX_URL_BYTES, "{} bytes", url.len());
			assert!(trusted_search_url(url));
		}
	}

	#[test]
	fn google_search_fitting_is_bounded() {
		let calls = std::cell::Cell::new(0);
		let query = "%".repeat(MAX_QUERY_BYTES);
		let (url, shortened) = search_url_with(&query, |candidate| {
			calls.set(calls.get() + 1);
			search_url_once(candidate)
		})
		.expect("a bounded query should fit");
		assert!(shortened);
		assert!(trusted_search_url(&url));
		assert!(calls.get() <= 12, "{} URL reconstructions", calls.get());
	}

	#[test]
	fn google_that_rejects_empty_or_stale_selection_without_private_output() {
		for value in [
			{
				let mut value = action("search", "300");
				value["selected_message"] = json!("   \n\t ");
				value
			},
			{
				let mut value = action("search", "300");
				value["selected_message"] = json!("\u{0}hidden");
				value
			},
			{
				let mut value = action("search", "300");
				value["selected_message_ref"] = reference("999", "300");
				value
			},
			{
				let mut value = action("search", "300");
				value
					.as_object_mut()
					.unwrap()
					.remove("selected_message_ref");
				value
			},
		] {
			let output = run(value);
			assert!(output.effects.is_empty());
			assert!(output.output.storage.is_none());
			assert!(output.output.replacement.is_none());
			assert!(!output.output.panel.iter().any(|element| matches!(
				element,
				Element::Text { text } if text.contains("hidden")
			)));
		}
	}

	#[test]
	fn husk_adds_exactly_one_reaction_without_toggling() {
		let output = run(action("husk", "300"));
		assert_eq!(
			output.effects,
			vec![HostEffect::AppAction {
				action: AppAction::SetReaction {
					channel_id: "100".into(),
					message_id: "300".into(),
					emoji: HUSK_EMOJI.into(),
					add: true
				}
			}]
		);
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("never removes or toggles")
		)));
		assert!(output.output.storage.is_none());

		for value in [
			{
				let mut value = action("husk", "999");
				value["selected_message_ref"] = reference("100", "999");
				value
			},
			{
				let mut value = action("husk", "300");
				value["selected_message_ref"] = reference("999", "300");
				value
			},
			{
				let mut value = action("husk", "300");
				value
					.as_object_mut()
					.unwrap()
					.remove("selected_message_ref");
				value
			},
		] {
			let output = run(value);
			assert!(output.effects.is_empty());
			assert!(output.output.storage.is_none());
		}
	}

	#[test]
	fn panels_stay_bounded_and_never_expose_storage_or_tokens() {
		for value in [
			action("find-reply", "300"),
			action("search", "300"),
			action("husk", "300"),
		] {
			let output = run(value);
			assert!(output.output.storage.is_none());
			assert!(output.output.replacement.is_none());
			assert!(output.output.panel.len() <= 16);
			for element in &output.output.panel {
				match element {
					Element::Text { text } => assert!(text.len() <= 4096),
					Element::Heading { text } => {
						assert!(!text.is_empty() && text.len() <= 128)
					}
					_ => {}
				}
				let debug = format!("{element:?}");
				assert!(!debug.to_ascii_lowercase().contains("token"));
				assert!(!debug.contains("Synthetic selected text"));
			}
		}
	}
}
