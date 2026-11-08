/*
 * Vencord/Equicord, a Discord client mod
 * Copyright (c) 2025 nin0
 * Copyright (c) 2026 Serein port contributors
 *
 * Narrow Serein adaptation of Equicord's SongLink.desktop at commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e:
 * src/equicordplugins/songLink.desktop/index.tsx,
 * SongLinker.tsx, Settings.tsx, native.ts and Providers.ts.
 *
 * Changed November 8, 2026: no upstream API request, track cache, inline React
 * accessory, provider-icon assets, native app URI or automatic send. The port
 * recognizes the same Spotify, Apple Music, YouTube Music and TIDAL link shapes
 * and proposes one host-reviewed HTTPS browser page instead.
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License version 3 or (at your
 * option) any later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{AppAction, AppInvocation, AppOutput, Element, HostEffect, Output};
use url::Url;

/// A selected message is normally a Discord-sized message, but the host bound
/// is larger. Scanning stops at this text size so a tampered snapshot cannot
/// cause unbounded token work.
const MAX_TEXT_BYTES: usize = 16 * 1024;
/// One recognized link stays within the ordinary browser URL contract.
const MAX_URL_BYTES: usize = 2048;
/// Only the first few distinct links are considered, matching the bounded
/// accessory idea without rendering arbitrary message content.
const MAX_LINKS: usize = 4;
/// Provider path identifiers are short; this is deliberately far below 2,048.
const MAX_TOKEN_BYTES: usize = 64;

const DESTINATION_SONGLINK: &str = "Songlink page";
const DESTINATION_PROVIDER: &str = "Provider page";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
	Spotify,
	AppleMusic,
	YouTubeMusic,
	Tidal,
}

impl Provider {
	fn label(self) -> &'static str {
		match self {
			Self::Spotify => "Spotify",
			Self::AppleMusic => "Apple Music",
			Self::YouTubeMusic => "YouTube Music",
			Self::Tidal => "TIDAL",
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MusicLink {
	provider: Provider,
	/// Clean provider URL sent to the Songlink page path.
	source_url: String,
	/// The provider's own public HTTPS page.
	provider_url: String,
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
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

fn panel_output(panel: Vec<Element>, effect: Option<HostEffect>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		effects: effect.into_iter().collect(),
	}
}

fn error_panel(title: &str, message: &str) -> AppOutput {
	panel_output(vec![heading(title), text(message)], None)
}

fn valid_token(value: &str) -> bool {
	let bytes = value.as_bytes();
	!bytes.is_empty()
		&& bytes.len() <= MAX_TOKEN_BYTES
		&& bytes
			.iter()
			.all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
}

fn valid_id(value: &str) -> bool {
	let bytes = value.as_bytes();
	!bytes.is_empty()
		&& bytes.len() <= 20
		&& bytes[0] != b'0'
		&& bytes.iter().all(u8::is_ascii_digit)
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn clean_no_fragment(url: &Url) -> String {
	let mut clean = url.clone();
	clean.set_fragment(None);
	clean.to_string()
}

fn parse_apple_album(url: &Url) -> Option<String> {
	let segments: Vec<&str> = url.path_segments()?.collect();
	let country = *segments.first()?;
	if country.len() != 2
		|| !country.bytes().all(|byte| byte.is_ascii_lowercase())
		|| segments.get(1) != Some(&"album")
		|| segments.get(2).is_none_or(|segment| segment.is_empty())
	{
		return None;
	}
	Some(clean_no_fragment(url))
}

fn parse_tidal(segments: &[&str]) -> Option<String> {
	let (raw_kind, id) = match segments {
		[kind, id] => (*kind, *id),
		[kind, id, suffix] if *suffix == "u" => (*kind, *id),
		["browse", kind, id] => (*kind, *id),
		_ => return None,
	};
	if !matches!(raw_kind, "track" | "album") || !valid_id(id) {
		return None;
	}
	let kind = if raw_kind == "album" {
		"album"
	} else {
		"track"
	};
	Some(format!("https://tidal.com/browse/{kind}/{id}"))
}

/// Recognizes only the exact provider shapes used by upstream SongLink and
/// TidalEmbeds. Userinfo, non-default explicit ports, HTTP, lookalike hosts and
/// extra path components produce no link.
fn classify(raw: &str) -> Option<MusicLink> {
	let raw = raw.trim();
	if raw.is_empty()
		|| raw.len() > MAX_URL_BYTES
		|| raw
			.chars()
			.any(|character| character.is_control() || character.is_whitespace())
	{
		return None;
	}
	let url = Url::parse(raw).ok()?;
	if url.scheme() != "https"
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.port().is_some()
	{
		return None;
	}
	let host = url.host_str()?.to_ascii_lowercase();
	let segments: Vec<&str> = url.path_segments()?.collect();

	match host.as_str() {
		"open.spotify.com" | "play.spotify.com" => {
			if segments.len() != 2 || segments[0] != "track" || !valid_token(segments[1]) {
				return None;
			}
			let source = format!("https://open.spotify.com/track/{}", segments[1]);
			Some(MusicLink {
				provider: Provider::Spotify,
				source_url: source.clone(),
				provider_url: source,
			})
		}
		"music.apple.com" | "itunes.apple.com" => {
			let source = parse_apple_album(&url)?;
			Some(MusicLink {
				provider: Provider::AppleMusic,
				source_url: source.clone(),
				provider_url: source,
			})
		}
		"music.youtube.com" => {
			if segments.len() != 1 || segments[0] != "watch" {
				return None;
			}
			let video = url
				.query_pairs()
				.find_map(|(key, value)| (key == "v").then_some(value.into_owned()))?;
			if !valid_token(&video) {
				return None;
			}
			let source = format!("https://music.youtube.com/watch?v={video}");
			Some(MusicLink {
				provider: Provider::YouTubeMusic,
				source_url: source.clone(),
				provider_url: source,
			})
		}
		"tidal.com" => {
			let source = parse_tidal(&segments)?;
			Some(MusicLink {
				provider: Provider::Tidal,
				source_url: source.clone(),
				provider_url: source,
			})
		}
		_ => None,
	}
}

fn trim_token(token: &str) -> &str {
	token.trim_matches(|character: char| {
		matches!(
			character,
			'<' | '>'
				| '(' | ')' | '['
				| ']' | '{' | '}'
				| '"' | '\'' | ','
				| '.' | ';' | '!'
				| ':' | '?'
		)
	})
}

fn recognized_links(text: &str) -> Vec<MusicLink> {
	let mut links = Vec::new();
	for token in text.split_whitespace() {
		let Some(link) = classify(trim_token(token)) else {
			continue;
		};
		if links
			.iter()
			.any(|existing: &MusicLink| existing.source_url == link.source_url)
		{
			continue;
		}
		links.push(link);
		if links.len() == MAX_LINKS {
			break;
		}
	}
	links
}

fn songlink_url(link: &MusicLink) -> Option<String> {
	let url = format!("https://song.link/{}", link.source_url);
	valid_external_target(&url).then_some(url)
}

fn valid_external_target(value: &str) -> bool {
	if value.len() > MAX_URL_BYTES
		|| value
			.chars()
			.any(|character| character.is_control() || character.is_whitespace())
	{
		return false;
	}
	Url::parse(value).is_ok_and(|url| {
		url.scheme() == "https"
			&& url.username().is_empty()
			&& url.password().is_none()
			&& url.port().is_none()
			&& matches!(
				url.host_str(),
				Some(
					"song.link"
						| "open.spotify.com"
						| "music.apple.com"
						| "itunes.apple.com"
						| "music.youtube.com"
						| "tidal.com"
				)
			)
	})
}

fn selected_target(link: &MusicLink, destination: &str) -> Option<String> {
	let target = match destination {
		DESTINATION_SONGLINK => songlink_url(link)?,
		DESTINATION_PROVIDER => link.provider_url.clone(),
		_ => return None,
	};
	valid_external_target(&target).then_some(target)
}

fn open_panel(input: &AppInvocation) -> AppOutput {
	let invocation = &input.invocation;
	let raw_url = invocation.value("url");
	let destination = invocation
		.value("destination")
		.unwrap_or(DESTINATION_SONGLINK);

	if raw_url.is_none() && invocation.value("destination").is_none() {
		return panel_output(
			vec![
				heading("Music link tools"),
				text(
					"Paste a Spotify, Apple Music, YouTube Music or TIDAL track or album link. Serein recognizes the link locally and proposes one browser page for your review.",
				),
				Element::TextInput {
					id: "url".into(),
					label: "Music link".into(),
					value: String::new(),
				},
				Element::Select {
					id: "destination".into(),
					label: "Destination".into(),
					options: vec![DESTINATION_SONGLINK.into(), DESTINATION_PROVIDER.into()],
					value: DESTINATION_SONGLINK.into(),
				},
				button("open", "Review browser open"),
				text(
					"Apply opens the reviewed HTTPS URL in your browser. No Songlink API request is made by Serein, no provider-native scheme is launched, and no message or status changes.",
				),
			],
			None,
		);
	}

	let Some(raw_url) = raw_url.filter(|value| !value.trim().is_empty()) else {
		return error_panel(
			"Music link tools",
			"Enter a complete provider link. Nothing was opened.",
		);
	};
	let Some(link) = classify(raw_url) else {
		return error_panel(
			"Music link tools",
			"This is not a supported HTTPS Spotify, Apple Music, YouTube Music or TIDAL track/album link. Nothing was opened.",
		);
	};
	let Some(target) = selected_target(&link, destination) else {
		return error_panel(
			"Music link tools",
			"Choose a supported destination. Nothing was opened.",
		);
	};
	let mut panel = vec![
		heading("Music link tools"),
		text(format!("Recognized {} link.", link.provider.label())),
		text(format!("Reviewed URL: {target}")),
		text(
			"Apply opens this exact HTTPS URL in your browser. No lookup request is made by Serein.",
		),
	];
	if destination == DESTINATION_SONGLINK {
		panel.insert(
			2,
			text(
				"The Songlink page may resolve the track to additional services in your browser; it is not fetched by this extension.",
			),
		);
	}
	panel_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::OpenExternalLink { url: target },
		}),
	)
}

fn message_link(input: &AppInvocation) -> AppOutput {
	let Some(source) = input.invocation.selected_message.as_deref() else {
		return error_panel(
			"Music link tools",
			"Open this action on a message that contains a music link. Nothing was opened.",
		);
	};
	if source.len() > MAX_TEXT_BYTES {
		return error_panel(
			"Music link tools",
			"The selected message exceeds the 16 KiB scan bound. Nothing was opened.",
		);
	}
	let links = recognized_links(source);
	let Some(link) = links.first() else {
		return error_panel(
			"Music link tools",
			"No supported Spotify, Apple Music, YouTube Music or TIDAL track/album link was found. Nothing was opened.",
		);
	};
	let Some(target) = songlink_url(link) else {
		return error_panel(
			"Music link tools",
			"The reviewed Songlink URL failed the HTTPS allowlist. Nothing was opened.",
		);
	};
	let mut panel = vec![
		heading("Music link tools"),
		text(format!(
			"Using the first of {} recognized {} link(s).",
			links.len(),
			link.provider.label()
		)),
		text(format!("Reviewed URL: {target}")),
		text(
			"Apply opens the Songlink page in your browser. Serein does not call the Songlink API, cache track metadata, or launch a provider-native app.",
		),
	];
	if links.len() > 1 {
		panel.push(text(
			"Only one link per response is proposed. Run the action again on a message containing another link if needed.",
		));
	}
	panel_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::OpenExternalLink { url: target },
		}),
	)
}

fn handle(input: AppInvocation) -> AppOutput {
	match input.invocation.action.as_str() {
		"open" => open_panel(&input),
		"link" => message_link(&input),
		_ => error_panel("Music link tools", "Unknown action. Nothing was opened."),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::serde_json::{self, Value, json};

	fn run(value: Value) -> (AppOutput, Value) {
		let input: AppInvocation = serde_json::from_value(value).expect("valid invocation");
		let output = handle(input);
		let wire = serde_json::to_value(&output).expect("serializable output");
		(output, wire)
	}

	fn external_url(output: &AppOutput) -> &str {
		assert_eq!(output.effects.len(), 1);
		match &output.effects[0] {
			HostEffect::AppAction {
				action: AppAction::OpenExternalLink { url },
			} => url,
			other => panic!("unexpected effect: {other:?}"),
		}
	}

	fn select_value<'a>(output: &'a AppOutput, id: &str) -> Option<&'a str> {
		output
			.output
			.panel
			.iter()
			.find_map(|element| match element {
				Element::Select {
					id: element_id,
					value,
					..
				} if element_id.as_str() == id => Some(value.as_str()),
				_ => None,
			})
	}

	#[test]
	fn exact_provider_shapes_are_normalized_and_lookalikes_are_rejected() {
		let spotify = classify("https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT?si=tracking")
			.expect("spotify track");
		assert_eq!(spotify.provider, Provider::Spotify);
		assert_eq!(
			spotify.source_url,
			"https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT"
		);

		let youtube = classify("https://music.youtube.com/watch?v=dQw4w9WgXcQ&feature=share")
			.expect("youtube track");
		assert_eq!(
			youtube.source_url,
			"https://music.youtube.com/watch?v=dQw4w9WgXcQ"
		);

		let apple = classify("https://music.apple.com/us/album/example/123456789?i=987654321")
			.expect("apple album/track");
		assert_eq!(apple.provider, Provider::AppleMusic);
		assert!(apple.source_url.contains("?i=987654321"));

		let tidal = classify("https://tidal.com/browse/album/123456?utm=1").expect("tidal album");
		assert_eq!(tidal.provider, Provider::Tidal);
		assert_eq!(tidal.source_url, "https://tidal.com/browse/album/123456");

		for rejected in [
			"http://open.spotify.com/track/abc",
			"https://open.spotify.com.evil.example/track/abc",
			"https://user@open.spotify.com/track/abc",
			"https://open.spotify.com:8443/track/abc",
			"https://open.spotify.com/artist/abc",
			"https://tidal.com/browse/album/0",
			"https://music.youtube.com/watch?v=bad id",
			"https://music.apple.com/us/playlist/example/1",
			"https://example.com/track/abc",
		] {
			assert_eq!(classify(rejected), None, "must reject {rejected}");
		}
	}

	#[test]
	fn selected_message_scanning_deduplicates_and_stops_at_the_cap() {
		let text = "https://open.spotify.com/track/a \
			https://open.spotify.com/track/a?si=x \
			https://music.youtube.com/watch?v=one \
			https://music.youtube.com/watch?v=two \
			https://music.youtube.com/watch?v=three \
			https://music.youtube.com/watch?v=four";
		let links = recognized_links(text);
		assert_eq!(links.len(), MAX_LINKS);
		assert_eq!(links[0].source_url, "https://open.spotify.com/track/a");
		assert_eq!(links[1].source_url, "https://music.youtube.com/watch?v=one");
	}

	#[test]
	fn message_action_proposes_exactly_one_reviewed_songlink_page() {
		let (output, wire) = run(json!({
			"action": "link",
			"selected_message": "listen: <https://open.spotify.com/track/abc?si=1>."
		}));
		assert_eq!(
			external_url(&output),
			"https://song.link/https://open.spotify.com/track/abc"
		);
		assert_eq!(wire["effects"][0]["action"]["type"], "open_external_link");
		assert!(
			wire["effects"][0]["action"]["url"]
				.as_str()
				.is_some_and(|url| url.starts_with("https://song.link/https://"))
		);
	}

	#[test]
	fn panel_requires_a_valid_second_invocation_before_proposing() {
		let (initial, _) = run(json!({"action": "open"}));
		assert!(initial.effects.is_empty());
		assert_eq!(
			select_value(&initial, "destination"),
			Some(DESTINATION_SONGLINK)
		);

		let (opened, _) = run(json!({
			"action": "open",
			"values": {
				"url": "https://tidal.com/track/123456/u",
				"destination": "Provider page"
			}
		}));
		assert_eq!(
			external_url(&opened),
			"https://tidal.com/browse/track/123456"
		);

		for values in [
			json!({"url": "https://evil.example/track/1", "destination": DESTINATION_SONGLINK}),
			json!({"url": "https://open.spotify.com/track/abc", "destination": "native app"}),
			json!({"url": "", "destination": DESTINATION_SONGLINK}),
		] {
			let (output, _) = run(json!({"action": "open", "values": values}));
			assert!(output.effects.is_empty());
		}
	}

	#[test]
	fn message_action_is_inactive_without_a_recognized_link() {
		let (missing, _) = run(json!({"action": "link"}));
		assert!(missing.effects.is_empty());
		let (none, _) = run(json!({
			"action": "link",
			"selected_message": "https://example.com/track/abc"
		}));
		assert!(none.effects.is_empty());
		let (oversized, _) = run(json!({
			"action": "link",
			"selected_message": "x".repeat(MAX_TEXT_BYTES + 1)
		}));
		assert!(oversized.effects.is_empty());
	}
}
