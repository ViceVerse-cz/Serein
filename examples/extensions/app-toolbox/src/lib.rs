use serein_extension_sdk::{
	AppInvocation, AppOutput, AppSnapshot, AppView, ChannelSnapshot, Element, HostEffect,
	Invocation, LocalSettingsPatch, Output,
};
use std::str::FromStr;

const VIEWS: &[(&str, AppView)] = &[
	("friends", AppView::Friends),
	("search", AppView::Search),
	("pins", AppView::Pins),
	("members", AppView::Members),
	("threads", AppView::Threads),
	("settings", AppView::Settings),
	("appearance", AppView::Appearance),
	("extensions", AppView::Extensions),
	("themes", AppView::Themes),
	("voice_settings", AppView::VoiceSettings),
	("account", AppView::Account),
	("profile_settings", AppView::ProfileSettings),
	("messaging_permissions", AppView::MessagingPermissions),
	("notifications", AppView::Notifications),
	("activity", AppView::Activity),
	("keybinds", AppView::Keybinds),
	("storage", AppView::Storage),
	("updates", AppView::Updates),
];

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn button(id: &str, label: &str) -> Element {
	Element::Button {
		id: id.into(),
		label: label.into(),
	}
}

fn field(id: &str, label: &str, value: &str) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: value.into(),
	}
}

fn checkbox(id: &str, label: &str, checked: bool) -> Element {
	Element::Checkbox {
		id: id.into(),
		label: label.into(),
		checked,
	}
}

fn channel_option(channel: &ChannelSnapshot) -> String {
	format!(
		"{} ({})",
		channel.name.chars().take(20).collect::<String>(),
		channel.id
	)
}

fn context(app: &AppSnapshot) -> String {
	let Some(context) = &app.context else {
		return "App context unavailable".into();
	};
	format!(
		"Connected: {}\nAccount: {}\nConversation: {}",
		context.connected,
		context
			.user
			.as_ref()
			.map_or("unavailable", |user| user.name.as_str()),
		context
			.channel
			.as_ref()
			.map_or("none", |channel| channel.name.as_str())
	)
}

fn dashboard(app: &AppSnapshot) -> Vec<Element> {
	let mut panel = vec![
		Element::Heading {
			text: "App toolbox".into(),
		},
		text(
			"Snapshots are bounded and may be partial. Each action below proposes one change for confirmation.",
		),
		text(context(app)),
	];
	if let Some(account) = &app.account_profile {
		panel.push(text(match &account.profile {
			Some(profile) => format!(
				"Account profile: {}\nPronouns: {}\nBio: {}",
				profile
					.display_name
					.as_deref()
					.unwrap_or(&account.user.name),
				profile.pronouns,
				profile.bio
			),
			None => format!(
				"Account profile: {} (details unavailable)",
				account.user.name
			),
		}));
	}
	if let Some(details) = &app.channel_details {
		panel.push(text(format!(
			"Channel: {}\nCan send: {}; can read history: {}\nLoaded recipients: {}{}",
			details.channel.name,
			details.can_send,
			details.can_read_history,
			details.recipients.len(),
			if details.recipients_truncated {
				" (partial)"
			} else {
				""
			}
		)));
	}
	for (label, count) in [
		(
			"Joined servers",
			app.guilds.as_ref().map(|v| (v.items.len(), v.truncated)),
		),
		(
			"Channels",
			app.channels.as_ref().map(|v| (v.items.len(), v.truncated)),
		),
		(
			"Loaded messages",
			app.timeline
				.as_ref()
				.map(|v| (v.messages.len(), v.truncated)),
		),
		(
			"Loaded members",
			app.members.as_ref().map(|v| (v.items.len(), v.truncated)),
		),
		(
			"Cached presences",
			app.presence.as_ref().map(|v| (v.items.len(), v.truncated)),
		),
	] {
		panel.push(text(match count {
			Some((count, partial)) => format!(
				"{label}: {count}{}",
				if partial { " (partial)" } else { "" }
			),
			None => format!("{label}: unavailable"),
		}));
	}
	if let Some(message) = app.timeline.as_ref().and_then(|v| v.messages.last()) {
		panel.push(text(format!(
			"Latest loaded message by {}: {}",
			message.author.name,
			message.content.chars().take(240).collect::<String>()
		)));
	}
	if let Some(read) = &app.read_state {
		panel.push(text(format!(
			"Unread: {}; mentions: {}",
			read.unread
				.map_or("unknown", |v| if v { "yes" } else { "no" }),
			read.mentions
		)));
	}
	panel.push(Element::Separator);
	if let Some(channels) = &app.channels {
		let options: Vec<_> = channels.items.iter().take(32).map(channel_option).collect();
		if let Some(value) = options.first().cloned() {
			panel.push(Element::Select {
				id: "channel".into(),
				label: "Channel (first 32 available)".into(),
				options,
				value,
			});
			panel.push(button("open-channel", "Open channel"));
		}
	}
	panel.extend([
		Element::Select {
			id: "view".into(),
			label: "App view".into(),
			options: std::iter::once("home")
				.chain(VIEWS.iter().map(|(name, _)| *name))
				.map(String::from)
				.collect(),
			value: "friends".into(),
		},
		button("open-view", "Open view"),
		field("query", "Search query", ""),
		button("search", "Search"),
		field(
			"message-id",
			"Message ID in current conversation",
			app.timeline
				.as_ref()
				.and_then(|v| v.messages.last())
				.map_or("", |v| v.id.as_str()),
		),
		button("jump", "Jump to message"),
		field(
			"user-id",
			"User ID",
			app.members
				.as_ref()
				.and_then(|v| v.items.first())
				.map_or("", |v| v.id.as_str()),
		),
		button("profile", "Open profile"),
		Element::Separator,
	]);
	if let Some(voice) = &app.voice {
		panel.extend([
			text(format!(
				"Voice: {}; participants: {}; camera: {}; sharing: {}",
				voice.phase,
				voice.participants.len(),
				voice.camera,
				voice.streaming
			)),
			checkbox("muted", "Mute microphone", voice.muted),
			checkbox("deafened", "Deafen", voice.deafened),
			Element::Row {
				children: vec![
					button("voice", "Apply voice controls"),
					button("leave", "Leave voice"),
				],
			},
		]);
	}
	if let Some(settings) = &app.settings {
		panel.extend([
			Element::Heading {
				text: "Local preferences".into(),
			},
			Element::Slider {
				id: "zoom".into(),
				label: "Zoom percent".into(),
				min: 80,
				max: 150,
				value: settings.zoom_percent.into(),
			},
			Element::Slider {
				id: "sidebar".into(),
				label: "Sidebar width".into(),
				min: 190,
				max: 360,
				value: settings.sidebar_width.into(),
			},
			checkbox("show-members", "Show member list", settings.show_members),
			checkbox("animate-gifs", "Animate GIFs", settings.animate_gifs),
			checkbox(
				"hide-media-links",
				"Hide media links",
				settings.hide_media_links,
			),
			button("settings", "Apply preferences"),
		]);
	}
	panel.extend([
		Element::Separator,
		field("notice-text", "Local notice", "Hello from App Toolbox"),
		Element::Row {
			children: vec![
				button("notice", "Show notice"),
				button("copy", "Copy current context"),
			],
		},
		button("show", "Refresh toolbox"),
	]);
	panel
}

fn value<T: FromStr>(input: &Invocation, id: &str) -> Result<T, &'static str> {
	input
		.parse_value(id)
		.map_err(|_| "A field has an invalid value.")?
		.ok_or("Open the toolbox and complete its fields first.")
}

fn id(input: &Invocation, name: &str) -> Result<String, &'static str> {
	let text = input.value(name).ok_or("Enter an ID first.")?;
	if text.len() > 20
		|| !text.bytes().all(|c| c.is_ascii_digit())
		|| !text.parse::<u64>().is_ok_and(|v| v != 0)
	{
		return Err("IDs must be nonzero decimal numbers.");
	}
	Ok(text.into())
}

fn proposal(input: &Invocation, app: &AppSnapshot) -> Result<HostEffect, &'static str> {
	Ok(match input.action.as_str() {
		"open-channel" => HostEffect::Navigate {
			channel_id: app
				.channels
				.as_ref()
				.and_then(|channels| {
					channels.items.iter().take(32).find(|channel| {
						input.value("channel") == Some(channel_option(channel).as_str())
					})
				})
				.ok_or("Choose an available channel in the toolbox first.")?
				.id
				.clone(),
		},
		"open-view" => {
			if input.value("view") == Some("home") {
				return Ok(HostEffect::Home);
			}
			let (_, view) = VIEWS
				.iter()
				.find(|(name, _)| input.value("view") == Some(*name))
				.ok_or("Choose an app view first.")?;
			HostEffect::OpenView { view: *view }
		}
		"search" => {
			let query = input.value("query").unwrap_or("");
			if query.is_empty() || query.len() > 256 || query.chars().any(char::is_control) {
				return Err(
					"Enter a search query of at most 256 bytes without control characters.",
				);
			}
			HostEffect::Search {
				query: query.into(),
			}
		}
		"jump" => HostEffect::JumpToMessage {
			channel_id: app
				.context
				.as_ref()
				.and_then(|v| v.channel.as_ref())
				.ok_or("Open a conversation first.")?
				.id
				.clone(),
			message_id: id(input, "message-id")?,
		},
		"profile" => HostEffect::OpenProfile {
			user_id: id(input, "user-id")?,
		},
		"copy" => {
			if app.context.is_none() {
				return Err("App context is unavailable.");
			}
			HostEffect::CopyText { text: context(app) }
		}
		"notice" => {
			let text = input.value("notice-text").unwrap_or("");
			if text.trim().is_empty() || text.len() > 1024 {
				return Err("Enter a notice of 1 to 1,024 bytes.");
			}
			HostEffect::Notice { text: text.into() }
		}
		"voice" => HostEffect::SetVoice {
			muted: value(input, "muted")?,
			deafened: value(input, "deafened")?,
		},
		"leave" => HostEffect::LeaveVoice,
		"settings" => {
			let zoom = value(input, "zoom")?;
			let sidebar = value(input, "sidebar")?;
			if !(80..=150).contains(&zoom) || !(190..=360).contains(&sidebar) {
				return Err("Zoom must be 80–150 and sidebar width 190–360.");
			}
			HostEffect::SetLocalSettings {
				settings: LocalSettingsPatch {
					zoom_percent: Some(zoom),
					sidebar_width: Some(sidebar),
					show_members: Some(value(input, "show-members")?),
					animate_gifs: Some(value(input, "animate-gifs")?),
					hide_media_links: Some(value(input, "hide-media-links")?),
				},
			}
		}
		_ => return Err("Open the toolbox to choose an action."),
	})
}

fn handle(input: AppInvocation) -> AppOutput {
	// Observing app changes must not open panels or trigger commands in the background.
	if input.app_event.is_some()
		|| input.message_event.is_some()
		|| input.invocation.action == "on-app"
	{
		return AppOutput::default();
	}
	let app = input.app.unwrap_or_default();
	if input.invocation.action == "show" {
		return AppOutput {
			output: Output {
				panel: dashboard(&app),
				..Default::default()
			},
			..Default::default()
		};
	}
	match proposal(&input.invocation, &app) {
		Ok(effect) => AppOutput {
			effects: vec![effect],
			..Default::default()
		},
		Err(error) => AppOutput {
			output: Output {
				panel: vec![text(error), button("show", "Open toolbox")],
				..Default::default()
			},
			..Default::default()
		},
	}
}
serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{AppEventKind, dispatch_typed, serde_json};

	#[test]
	fn typed_actions_require_valid_values_and_events_remain_passive() {
		let mut input = AppInvocation {
			invocation: Invocation {
				action: "show".into(),
				..Default::default()
			},
			..Default::default()
		};
		let run = |input: &AppInvocation| -> AppOutput {
			serde_json::from_slice(
				&dispatch_typed(&serde_json::to_vec(input).unwrap(), handle).unwrap(),
			)
			.unwrap()
		};
		input.app = Some(
			serde_json::from_value(serde_json::json!({
				"account_profile": {
					"user": {"id": "1", "name": "Example"},
					"profile": {"display_name": "Display", "bio": "Synthetic profile", "pronouns": "they/them"}
				},
				"guilds": {"items": [{"id": "2", "name": "Example server"}], "truncated": true},
				"channel_details": {
					"channel": {"id": "3", "guild_id": "2", "name": "general", "kind": 0},
					"position": 0, "recipients": [], "recipients_truncated": false,
					"can_send": true, "can_read_history": false
				}
			}))
			.unwrap(),
		);
		let shown = run(&input);
		let text = shown
			.output
			.panel
			.iter()
			.filter_map(|element| match element {
				Element::Text { text } => Some(text.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n");
		assert!(text.contains("Synthetic profile"));
		assert!(text.contains("Joined servers: 1 (partial)"));
		assert!(text.contains("Can send: true; can read history: false"));
		assert!(!shown.output.panel.is_empty());
		assert!(shown.effects.is_empty());
		input.invocation.action = "open-view".into();
		for (name, _) in VIEWS {
			input
				.invocation
				.values
				.insert("view".into(), (*name).into());
			assert_eq!(
				serde_json::to_value(run(&input).effects).unwrap(),
				serde_json::json!([{"type":"open_view", "view":name}])
			);
		}
		input.invocation.action = "voice".into();
		input
			.invocation
			.values
			.insert("muted".into(), "true".into());
		input
			.invocation
			.values
			.insert("deafened".into(), "false".into());
		assert_eq!(
			run(&input).effects,
			vec![HostEffect::SetVoice {
				muted: true,
				deafened: false
			}]
		);
		input
			.invocation
			.values
			.insert("deafened".into(), "invalid".into());
		assert!(run(&input).effects.is_empty());
		input.invocation.action = "settings".into();
		input.invocation.values.extend(
			[
				("zoom", "120"),
				("sidebar", "240"),
				("show-members", "true"),
				("animate-gifs", "false"),
				("hide-media-links", "true"),
			]
			.map(|(k, v)| (k.into(), v.into())),
		);
		assert!(
			matches!(&run(&input).effects[..], [HostEffect::SetLocalSettings { settings }] if settings.zoom_percent == Some(120) && settings.animate_gifs == Some(false))
		);
		input.invocation.values.insert("zoom".into(), "151".into());
		assert!(run(&input).effects.is_empty());
		for kind in [
			AppEventKind::Ready,
			AppEventKind::Navigation,
			AppEventKind::Context,
			AppEventKind::Connection,
			AppEventKind::Voice,
			AppEventKind::Settings,
			AppEventKind::Account,
			AppEventKind::Channels,
			AppEventKind::Members,
			AppEventKind::Presence,
			AppEventKind::ReadState,
		] {
			input.app_event = Some(kind);
			assert_eq!(run(&input), AppOutput::default());
		}
	}
}
