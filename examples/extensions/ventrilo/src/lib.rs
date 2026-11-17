//! Ventrilo Mode: a slim, Ventrilo-style server/channel tree.
//!
//! The plugin owns no pixels. It reads the granted app snapshot (servers, channels,
//! the voice roster), restates a declarative [`InterfaceUi`] tree on every result, and
//! receives row clicks as the declared `interface_click` action with the clicked
//! row's id in `values["id"]`: a single click picks a server (`server:<id>`) and
//! does nothing on a channel; a double click on a voice channel (`channel:<id>`)
//! joins it and opens (or closes) the call window, and on a text channel
//! (`text:<id>`) opens (or closes) its chat window. One window shows at a time.

use serein_extension_sdk::serde_json::{self, Value};
use serein_extension_sdk::{
	AppAction, AppInvocation, AppSnapshot, HostEffect, InterfaceIcon, InterfaceNode, InterfaceTint,
	InterfaceUi, InterfaceWindow, Invocation, MAX_INTERFACE_NODES, MAX_INTERFACE_TEXT_BYTES,
	Output, VoiceRosterEntry,
};

/// The declared panel action the host invokes for a row click.
const INTERFACE_CLICK: &str = "interface_click";
/// Row-id prefixes minted by [`build_tree`].
const SERVER_PREFIX: &str = "server:";
const CHANNEL_PREFIX: &str = "channel:";
const TEXT_PREFIX: &str = "text:";
/// Stored window encodings.
const CALL_WINDOW: &str = "call";
const CHAT_WINDOW_PREFIX: &str = "chat:";
/// Channel kinds shown as rows: voice, and the text kinds (server text, announcement).
const VOICE_KIND: u8 = 2;
const TEXT_KINDS: [u8; 2] = [0, 5];

/// Persisted choices. Corrupt or older-format storage falls back to the default:
/// layout on, first server, no window. Fields this version no longer uses are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Saved {
	off: bool,
	server: Option<String>,
	window: Option<InterfaceWindow>,
}

/// A snowflake: 1 to 20 ASCII digits.
fn snowflake(id: &str) -> bool {
	(1..=20).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit())
}

/// `"call"` or `"chat:<snowflake>"`; anything else is corrupt.
fn decode_window(raw: &str) -> Option<InterfaceWindow> {
	if raw == CALL_WINDOW {
		return Some(InterfaceWindow::Call);
	}
	raw.strip_prefix(CHAT_WINDOW_PREFIX)
		.filter(|id| snowflake(id))
		.map(|id| InterfaceWindow::Chat {
			channel_id: id.into(),
		})
}

fn encode_window(window: &InterfaceWindow) -> String {
	match window {
		InterfaceWindow::Call => CALL_WINDOW.into(),
		InterfaceWindow::Chat { channel_id } => format!("{CHAT_WINDOW_PREFIX}{channel_id}"),
	}
}

/// Strict decode: any missing piece keeps its default, any wrong-typed field
/// makes the whole value corrupt (defaults, left unoverwritten).
fn saved(invocation: &Invocation) -> Saved {
	let fields = invocation
		.storage
		.as_deref()
		.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
		.and_then(|value| value.as_object().cloned());
	let Some(fields) = fields else {
		return Saved::default();
	};
	let flag = |name: &str| -> Option<bool> {
		match fields.get(name) {
			None | Some(Value::Null) => Some(false),
			Some(value) => value.as_bool(),
		}
	};
	let server = match fields.get("server") {
		None | Some(Value::Null) => None,
		Some(value) => value.as_str().map(|id| id.to_string()),
	};
	// `Some(None)`: no window; `None`: corrupt.
	let window = match fields.get("window") {
		None | Some(Value::Null) => Some(None),
		Some(value) => value.as_str().and_then(decode_window).map(Some),
	};
	match (flag("off"), window, server) {
		(Some(off), Some(window), server) => Saved {
			off,
			server,
			window,
		},
		_ => Saved::default(),
	}
}

fn storage_json(state: &Saved) -> String {
	serde_json::json!({
		"off": state.off,
		"server": state.server.clone(),
		"window": state.window.as_ref().map(encode_window),
	})
	.to_string()
}

/// One flat result object: the original `Output` fields plus the interface
/// restatement and at most one host proposal, exactly as the wire allows.
fn result(output: Output, interface_ui: Option<InterfaceUi>, effects: Vec<HostEffect>) -> Value {
	let mut value = serde_json::to_value(&output).expect("plain output data encodes");
	if let Some(interface) = &interface_ui {
		value["interface_ui"] = serde_json::to_value(interface).expect("plain tree encodes");
	}
	if !effects.is_empty() {
		value["effects"] = serde_json::to_value(&effects).expect("plain proposal encodes");
	}
	value
}

/// Keep row text inside the 256-byte bound and never empty.
fn bounded_text(text: &str) -> String {
	let mut out = String::new();
	for (index, character) in text.char_indices() {
		if index + character.len_utf8() > MAX_INTERFACE_TEXT_BYTES {
			break;
		}
		out.push(character);
	}
	if out.is_empty() {
		"Unnamed".into()
	} else {
		out
	}
}

/// The saved server when it still exists, else the first one in directory order.
fn picked_server<'a>(app: &'a AppSnapshot, state: &Saved) -> Option<&'a str> {
	let guilds = app.guilds.as_ref().map(|group| group.items.as_slice())?;
	guilds
		.iter()
		.find(|guild| state.server.as_deref() == Some(guild.id.as_str()))
		.map(|guild| guild.id.as_str())
		.or_else(|| guilds.first().map(|guild| guild.id.as_str()))
}

fn inert(text: &str, icon: InterfaceIcon) -> InterfaceNode {
	InterfaceNode {
		id: String::new(),
		indent: 0,
		icon: Some(icon),
		text: text.into(),
		badge: None,
		emphasis: false,
		selected: false,
		tint: None,
	}
}

/// One inert person row; the status icon renders left of the name.
fn person(entry: &VoiceRosterEntry, mine: bool) -> InterfaceNode {
	let deafened = entry.deafened || entry.server_deafened;
	let muted = entry.muted || entry.server_muted;
	let icon = if deafened {
		InterfaceIcon::HeadphonesSlash
	} else if muted {
		InterfaceIcon::MicrophoneSlash
	} else {
		InterfaceIcon::Speaker
	};
	let tint = if deafened || muted {
		InterfaceTint::Danger
	} else if entry.speaking {
		InterfaceTint::Positive
	} else {
		InterfaceTint::Muted
	};
	InterfaceNode {
		id: String::new(),
		indent: 2,
		icon: Some(icon),
		text: bounded_text(entry.name.as_deref().unwrap_or(&entry.user_id)),
		badge: mine.then(|| "You".into()),
		emphasis: false,
		selected: false,
		tint: Some(tint),
	}
}

/// Build the whole tree: servers, the picked server's text and voice channels, and
/// under each voice channel the people the voice roster places there.
fn build_tree(app: &AppSnapshot, state: &Saved) -> InterfaceUi {
	let guilds = app
		.guilds
		.as_ref()
		.map(|group| group.items.as_slice())
		.unwrap_or(&[]);
	let channels = app
		.channels
		.as_ref()
		.map(|group| group.items.as_slice())
		.unwrap_or(&[]);
	let voice = app.voice.as_ref();
	let call_channel = voice.and_then(|voice| voice.channel_id.as_deref());
	let own = app
		.context
		.as_ref()
		.and_then(|context| context.user.as_ref());
	let picked = picked_server(app, state);
	let mut nodes = Vec::new();

	for guild in guilds {
		if nodes.len() == MAX_INTERFACE_NODES {
			break;
		}
		nodes.push(InterfaceNode {
			id: format!("{SERVER_PREFIX}{}", guild.id),
			indent: 0,
			icon: Some(InterfaceIcon::Server),
			text: bounded_text(&guild.name),
			badge: None,
			emphasis: true,
			selected: picked == Some(guild.id.as_str()),
			tint: None,
		});
		if picked != Some(guild.id.as_str()) {
			continue;
		}
		// Text and voice channels interleave in the directory's own order.
		for channel in channels {
			if nodes.len() == MAX_INTERFACE_NODES {
				break;
			}
			if channel.guild_id.as_deref() != Some(guild.id.as_str()) {
				continue;
			}
			if TEXT_KINDS.contains(&channel.kind) {
				let open = matches!(
					&state.window,
					Some(InterfaceWindow::Chat { channel_id }) if *channel_id == channel.id
				);
				nodes.push(InterfaceNode {
					id: format!("{TEXT_PREFIX}{}", channel.id),
					indent: 1,
					icon: Some(InterfaceIcon::Hash),
					text: bounded_text(&channel.name),
					badge: None,
					emphasis: false,
					selected: open,
					tint: None,
				});
				continue;
			}
			if channel.kind != VOICE_KIND {
				continue;
			}
			let is_call_channel = call_channel == Some(channel.id.as_str());
			nodes.push(InterfaceNode {
				id: format!("{CHANNEL_PREFIX}{}", channel.id),
				indent: 1,
				icon: Some(InterfaceIcon::Speaker),
				text: bounded_text(&channel.name),
				badge: None,
				emphasis: false,
				selected: is_call_channel,
				tint: None,
			});
			let roster = voice.map(|voice| voice.roster.as_slice()).unwrap_or(&[]);
			for entry in roster.iter().filter(|entry| entry.channel_id == channel.id) {
				if nodes.len() == MAX_INTERFACE_NODES {
					break;
				}
				nodes.push(person(
					entry,
					own.is_some_and(|user| user.id == entry.user_id),
				));
			}
		}
	}

	if nodes.is_empty() {
		nodes.push(inert("No channels loaded", InterfaceIcon::Server));
	}
	InterfaceUi {
		nodes,
		window: state.window.clone(),
	}
}

fn panel(text: &str, on: bool) -> Vec<serein_extension_sdk::Element> {
	vec![
		serein_extension_sdk::Element::Text { text: text.into() },
		serein_extension_sdk::Element::Button {
			id: "toggle".into(),
			label: if on { "Turn off" } else { "Turn on" }.into(),
		},
	]
}

/// Interface restatement for the current state; `None` restores the normal layout.
fn restated(app: &AppSnapshot, state: &Saved) -> Option<InterfaceUi> {
	(!state.off).then(|| build_tree(app, state))
}

fn handle(input: AppInvocation) -> Value {
	let invocation = &input.invocation;
	let mut state = saved(invocation);
	let app = input.app.clone().unwrap_or_default();

	// App events refresh the tree: voice changes (and channel-directory changes
	// through data_events) restate the interface; events may not touch panels,
	// proposals or storage.
	if input.app_event.is_some() {
		return result(Output::default(), restated(&app, &state), Vec::new());
	}

	// Row clicks arrive as the declared `interface_click` action with the clicked
	// row's id in `values["id"]`.
	if invocation.action == INTERFACE_CLICK {
		let row = invocation.value("id");
		if let Some(id) = row.and_then(|row| row.strip_prefix(SERVER_PREFIX)) {
			state.server = Some(id.into());
			return persisted(state, &app, Vec::new());
		}
		if let Some(id) = row.and_then(|row| row.strip_prefix(CHANNEL_PREFIX))
			&& invocation.double_clicked
		{
			let voice = app.voice.as_ref();
			let mut effects = Vec::new();
			if voice.and_then(|voice| voice.channel_id.as_deref()) == Some(id) {
				// Already in this call: show the call window, or close it if it
				// is already showing. The call itself stays connected.
				state.window = match state.window {
					Some(InterfaceWindow::Call) => None,
					_ => Some(InterfaceWindow::Call),
				};
			} else {
				// Propose joining; the user still confirms via Apply.
				state.window = Some(InterfaceWindow::Call);
				effects.push(HostEffect::AppAction {
					action: AppAction::JoinVoice {
						channel_id: id.into(),
						ring: false,
						muted: voice.is_some_and(|voice| voice.muted),
						deafened: voice.is_some_and(|voice| voice.deafened),
					},
				});
			}
			return persisted(state, &app, effects);
		}
		// Text rows: a double click opens that channel's chat window, or closes it
		// when it is already open.
		if let Some(id) = row.and_then(|row| row.strip_prefix(TEXT_PREFIX))
			&& invocation.double_clicked
			&& snowflake(id)
		{
			let chat = InterfaceWindow::Chat {
				channel_id: id.into(),
			};
			state.window = (state.window.as_ref() != Some(&chat)).then_some(chat);
			return persisted(state, &app, Vec::new());
		}
		// Single clicks on channels and unknown row ids are inert: restate without
		// changing or saving anything.
		return result(Output::default(), restated(&app, &state), Vec::new());
	}

	match invocation.action.as_str() {
		// Activation re-applies the saved choice; the host does not persist the
		// storage an activation returns, so nothing is written here.
		"activate" => result(Output::default(), restated(&app, &state), Vec::new()),
		"show" => {
			let on = !state.off;
			let text = if on {
				"Ventrilo Mode is on: Serein's main panes are replaced by the slim tree of \
				 servers, their text and voice channels, and the people in each voice channel. \
				 Click a server to switch to it. Double-click a voice channel to join it and \
				 open its call window, or a text channel to open its chat; double-click it \
				 again to close the window. A single click on a channel does nothing. The \
				 button switches back to the normal layout."
			} else {
				"Ventrilo Mode is off: Serein shows its normal layout. The button switches to \
				 the slim Ventrilo-style channel tree."
			};
			result(
				Output {
					panel: panel(text, on),
					..Default::default()
				},
				restated(&app, &state),
				Vec::new(),
			)
		}
		"toggle" => {
			state.off = !state.off;
			let on = !state.off;
			let text = if on {
				"Ventrilo Mode turned on. The slim channel tree is now active."
			} else {
				"Ventrilo Mode turned off. The normal layout is restored."
			};
			result(
				Output {
					panel: panel(text, on),
					storage: Some(storage_json(&state)),
					..Default::default()
				},
				restated(&app, &state),
				Vec::new(),
			)
		}
		_ => result(Output::default(), None, Vec::new()),
	}
}

/// Persist the changed choices and restate the interface from them.
fn persisted(state: Saved, app: &AppSnapshot, effects: Vec<HostEffect>) -> Value {
	result(
		Output {
			storage: Some(storage_json(&state)),
			..Default::default()
		},
		restated(app, &state),
		effects,
	)
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::serde_json::json;
	use serein_extension_sdk::{
		AppContextSnapshot, AppEventKind, ChannelDirectorySnapshot, ChannelSnapshot,
		GuildDirectorySnapshot, GuildSnapshot, UserSnapshot, VoiceSnapshot, dispatch_typed,
	};
	use std::collections::BTreeMap;

	/// The default tree: Alpha picked, its text and voice channels interleaved in
	/// directory order (the category is skipped), then Beta.
	const ALPHA_TREE: [&str; 6] = [
		"Alpha",
		"general",
		"General Voice",
		"news",
		"Town Square",
		"Beta",
	];

	fn guild(id: &str, name: &str) -> GuildSnapshot {
		GuildSnapshot {
			id: id.into(),
			name: name.into(),
			icon: None,
		}
	}

	fn channel(id: &str, guild_id: &str, name: &str, kind: u8) -> ChannelSnapshot {
		ChannelSnapshot {
			id: id.into(),
			guild_id: Some(guild_id.into()),
			name: name.into(),
			kind,
		}
	}

	fn entry(channel: &str, user: &str, name: Option<&str>) -> VoiceRosterEntry {
		VoiceRosterEntry {
			guild_id: if channel.starts_with('2') { "20" } else { "10" }.into(),
			channel_id: channel.into(),
			user_id: user.into(),
			name: name.map(Into::into),
			muted: false,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			speaking: false,
		}
	}

	/// `call` is (channel, participants, own muted, own deafened); the roster places
	/// every participant in that channel, named `Me` (own) or `User <id>`.
	fn snapshot(call: Option<(&str, &[&str], bool, bool)>) -> AppSnapshot {
		AppSnapshot {
			context: Some(AppContextSnapshot {
				connected: true,
				user: Some(UserSnapshot {
					id: "1".into(),
					name: "Me".into(),
				}),
				channel: None,
			}),
			guilds: Some(GuildDirectorySnapshot {
				items: vec![guild("10", "Alpha"), guild("20", "Beta")],
				truncated: false,
			}),
			channels: Some(ChannelDirectorySnapshot {
				items: vec![
					channel("103", "10", "general", 0),
					channel("101", "10", "General Voice", 2),
					channel("104", "10", "news", 5),
					channel("105", "10", "Lounge", 4),
					channel("102", "10", "Town Square", 2),
					channel("202", "20", "beta-chat", 0),
					channel("201", "20", "Beta Voice", 2),
				],
				truncated: false,
			}),
			voice: Some(VoiceSnapshot {
				channel_id: call.map(|(channel, _, _, _)| channel.to_string()),
				phase: if call.is_some() { "connected" } else { "idle" }.into(),
				muted: call.is_some_and(|(_, _, muted, _)| muted),
				deafened: call.is_some_and(|(_, _, _, deafened)| deafened),
				camera: false,
				streaming: false,
				participants: call
					.map(|(_, participants, _, _)| {
						participants.iter().map(|id| (*id).to_string()).collect()
					})
					.unwrap_or_default(),
				roster: call
					.map(|(channel, participants, muted, deafened)| {
						participants
							.iter()
							.map(|id| {
								if *id == "1" {
									VoiceRosterEntry {
										muted,
										deafened,
										..entry(channel, id, Some("Me"))
									}
								} else {
									entry(channel, id, Some(&format!("User {id}")))
								}
							})
							.collect()
					})
					.unwrap_or_default(),
			}),
			..Default::default()
		}
	}

	fn run(
		action: &str,
		row: Option<&str>,
		storage: Option<&str>,
		double: bool,
		event: Option<AppEventKind>,
		app: AppSnapshot,
	) -> Value {
		let mut values = BTreeMap::new();
		if let Some(row) = row {
			values.insert("id".to_string(), row.to_string());
		}
		let input = AppInvocation {
			invocation: Invocation {
				action: action.into(),
				storage: storage.map(Into::into),
				double_clicked: double,
				values,
				..Default::default()
			},
			app_event: event,
			app: Some(app),
			..Default::default()
		};
		let bytes = dispatch_typed(&serde_json::to_vec(&input).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	fn click(row: &str, storage: Option<&str>, double: bool, app: AppSnapshot) -> Value {
		run(INTERFACE_CLICK, Some(row), storage, double, None, app)
	}

	fn interface(output: &Value) -> InterfaceUi {
		serde_json::from_value(output["interface_ui"].clone()).unwrap()
	}

	fn texts(output: &Value) -> Vec<String> {
		interface(output)
			.nodes
			.iter()
			.map(|node| node.text.clone())
			.collect()
	}

	fn output_storage(output: &Value) -> String {
		output["storage"].as_str().unwrap().to_string()
	}

	/// Decode returned storage with the plugin's own strict reader.
	fn saved_from(output: &Value) -> Saved {
		saved(&Invocation {
			storage: Some(output_storage(output)),
			..Default::default()
		})
	}

	fn chat(id: &str) -> Option<InterfaceWindow> {
		Some(InterfaceWindow::Chat {
			channel_id: id.into(),
		})
	}

	fn no_proposal(output: &Value) {
		assert!(
			output["effects"].as_array().is_none_or(Vec::is_empty),
			"unexpected proposal: {output}"
		);
	}

	#[test]
	fn activation_builds_the_channel_tree() {
		let output = run("activate", None, None, false, None, snapshot(None));
		let tree = interface(&output);
		assert_eq!(texts(&output), ALPHA_TREE);
		assert_eq!(tree.window, None);
		assert_eq!(
			tree.nodes
				.iter()
				.map(|node| node.id.clone())
				.collect::<Vec<_>>(),
			[
				"server:10",
				"text:103",
				"channel:101",
				"text:104",
				"channel:102",
				"server:20"
			]
		);
		let alpha = &tree.nodes[0];
		assert!(alpha.emphasis && alpha.selected && alpha.indent == 0);
		assert_eq!(alpha.icon, Some(InterfaceIcon::Server));
		for (index, icon) in [
			(1, InterfaceIcon::Hash),
			(2, InterfaceIcon::Speaker),
			(3, InterfaceIcon::Hash),
		] {
			assert_eq!(
				(tree.nodes[index].indent, tree.nodes[index].icon),
				(1, Some(icon))
			);
			assert!(!tree.nodes[index].selected);
		}
		assert!(
			output["storage"].is_null()
				&& output["panel"]
					.as_array()
					.is_none_or(|panel| panel.is_empty())
		);
		no_proposal(&output);
		tree.validate().unwrap();
	}

	#[test]
	fn clicking_a_server_switches_the_tree_to_it() {
		let output = click("server:20", None, false, snapshot(None));
		let tree = interface(&output);
		assert_eq!(texts(&output), ["Alpha", "Beta", "beta-chat", "Beta Voice"]);
		assert!(!tree.nodes[0].selected && tree.nodes[1].selected);
		assert_eq!(saved_from(&output).server.as_deref(), Some("20"));
	}

	#[test]
	fn single_click_on_a_channel_changes_nothing() {
		let app = snapshot(Some(("101", &["1", "2"], false, false)));
		let people = [
			"Alpha",
			"general",
			"General Voice",
			"Me",
			"User 2",
			"news",
			"Town Square",
			"Beta",
		];
		for row in ["channel:101", "text:103"] {
			let output = click(row, None, false, app.clone());
			assert!(output["storage"].is_null(), "{row} saves nothing");
			assert_eq!(texts(&output), people, "{row} keeps the people shown");
			assert_eq!(interface(&output).window, None);
			no_proposal(&output);
		}
	}

	#[test]
	fn call_roster_marks_the_own_row() {
		let output = run(
			"activate",
			None,
			None,
			false,
			None,
			snapshot(Some(("101", &["1", "2"], true, false))),
		);
		let tree = interface(&output);
		assert!(tree.nodes[2].selected);
		let own = &tree.nodes[3];
		assert_eq!(
			(own.text.as_str(), own.badge.as_deref()),
			("Me", Some("You"))
		);
		assert_eq!(own.icon, Some(InterfaceIcon::MicrophoneSlash));
		assert_eq!(own.tint, Some(InterfaceTint::Danger));
		assert!(own.id.is_empty() && own.indent == 2);
		let other = &tree.nodes[4];
		assert_eq!(
			(other.text.as_str(), other.badge.as_deref()),
			("User 2", None)
		);
		assert_eq!(other.icon, Some(InterfaceIcon::Speaker));
		assert_eq!(other.tint, Some(InterfaceTint::Muted));
	}

	#[test]
	fn roster_fills_every_channel_with_names_flags_and_speaking() {
		let mut app = snapshot(None);
		app.voice.as_mut().unwrap().roster = vec![
			VoiceRosterEntry {
				speaking: true,
				..entry("101", "2", Some("Talker"))
			},
			VoiceRosterEntry {
				server_muted: true,
				speaking: true,
				..entry("101", "3", Some("Hushed"))
			},
			VoiceRosterEntry {
				server_deafened: true,
				muted: true,
				..entry("102", "4", None)
			},
			entry("201", "5", Some("Elsewhere")),
		];
		let output = run("activate", None, None, false, None, app);
		let tree = interface(&output);
		assert_eq!(
			texts(&output),
			[
				"Alpha",
				"general",
				"General Voice",
				"Talker",
				"Hushed",
				"news",
				"Town Square",
				"4",
				"Beta"
			]
		);
		let row = |index: usize| (tree.nodes[index].icon, tree.nodes[index].tint);
		assert_eq!(
			row(3),
			(Some(InterfaceIcon::Speaker), Some(InterfaceTint::Positive))
		);
		assert_eq!(
			row(4),
			(
				Some(InterfaceIcon::MicrophoneSlash),
				Some(InterfaceTint::Danger)
			)
		);
		assert_eq!(
			row(7),
			(
				Some(InterfaceIcon::HeadphonesSlash),
				Some(InterfaceTint::Danger)
			)
		);
		assert!(tree.nodes.iter().all(|node| node.badge.is_none()));
		tree.validate().unwrap();
	}

	#[test]
	fn text_double_click_opens_and_closes_its_chat() {
		let opened = click("text:104", None, true, snapshot(None));
		let tree = interface(&opened);
		assert_eq!(tree.window, chat("104"));
		assert!(tree.nodes[3].selected && !tree.nodes[1].selected);
		no_proposal(&opened);
		assert_eq!(saved_from(&opened).window, chat("104"));
		assert!(output_storage(&opened).contains("\"chat:104\""));
		tree.validate().unwrap();

		// Another text channel switches the chat; the same one closes it.
		let switched = click(
			"text:103",
			Some(&output_storage(&opened)),
			true,
			snapshot(None),
		);
		assert_eq!(interface(&switched).window, chat("103"));
		let closed = click(
			"text:103",
			Some(&output_storage(&switched)),
			true,
			snapshot(None),
		);
		assert_eq!(interface(&closed).window, None);
		assert!(!interface(&closed).nodes[1].selected);
		assert_eq!(saved_from(&closed).window, None);
	}

	#[test]
	fn voice_double_click_joins_and_replaces_an_open_chat() {
		let storage = storage_json(&Saved {
			window: chat("103"),
			..Default::default()
		});
		let output = click("channel:102", Some(&storage), true, snapshot(None));
		let tree = interface(&output);
		assert_eq!(tree.window, Some(InterfaceWindow::Call));
		assert!(!tree.nodes[1].selected);
		assert_eq!(
			output["effects"],
			json!([{"type": "app_action", "action": {
				"type": "join_voice", "channel_id": "102",
				"ring": false, "muted": false, "deafened": false}}])
		);
		assert_eq!(saved_from(&output).window, Some(InterfaceWindow::Call));
		assert!(output_storage(&output).contains("\"call\""));

		// Opening a chat in turn replaces the call window; the call is untouched.
		let back = click(
			"text:103",
			Some(&output_storage(&output)),
			true,
			snapshot(None),
		);
		assert_eq!(interface(&back).window, chat("103"));
		no_proposal(&back);
	}

	#[test]
	fn double_clicking_the_active_channel_toggles_the_call_window_without_leaving() {
		let app = snapshot(Some(("201", &["1"], false, false)));
		let storage = storage_json(&Saved {
			server: Some("20".into()),
			window: Some(InterfaceWindow::Call),
			..Default::default()
		});
		// The host delivers a double click as a single click, then a double-clicked
		// one; the pair must leave the channel's people shown.
		let first = click("channel:201", Some(&storage), false, app.clone());
		assert!(first["storage"].is_null(), "the single click is inert");
		let closed = click("channel:201", Some(&storage), true, app.clone());
		assert_eq!(interface(&closed).window, None);
		no_proposal(&closed);
		assert_eq!(saved_from(&closed).window, None);
		assert_eq!(
			texts(&closed),
			["Alpha", "Beta", "beta-chat", "Beta Voice", "Me"]
		);

		// From an open chat, the current call's channel brings the call back.
		let storage = storage_json(&Saved {
			server: Some("20".into()),
			window: chat("202"),
			..Default::default()
		});
		let call = click("channel:201", Some(&storage), true, app);
		assert_eq!(interface(&call).window, Some(InterfaceWindow::Call));
		no_proposal(&call);
	}

	#[test]
	fn toggle_round_trips_through_storage() {
		let off = run("toggle", None, None, false, None, snapshot(None));
		assert!(off["interface_ui"].is_null());
		assert_eq!(
			saved_from(&off),
			Saved {
				off: true,
				..Default::default()
			}
		);
		assert_eq!(off["panel"][1]["label"], json!("Turn on"));
		assert_eq!(off["panel"][1]["id"], json!("toggle"));

		let activated = run(
			"activate",
			None,
			Some(&output_storage(&off)),
			false,
			None,
			snapshot(None),
		);
		assert!(activated["interface_ui"].is_null());

		let on = run(
			"toggle",
			None,
			Some(&output_storage(&off)),
			false,
			None,
			snapshot(None),
		);
		assert!(!on["interface_ui"].is_null());
		assert_eq!(on["panel"][1]["label"], json!("Turn off"));
		assert_eq!(saved_from(&on), Saved::default());

		// A saved chat window survives the round trip and is restored on activation.
		let full = Saved {
			server: Some("20".into()),
			window: chat("202"),
			..Default::default()
		};
		let restored = run(
			"activate",
			None,
			Some(&storage_json(&full)),
			false,
			None,
			snapshot(None),
		);
		assert_eq!(interface(&restored).window, chat("202"));
		assert_eq!(
			texts(&restored),
			["Alpha", "Beta", "beta-chat", "Beta Voice"]
		);
	}

	#[test]
	fn corrupt_storage_falls_back_without_overwriting() {
		for corrupt in [
			"{",
			r#"{"window":true}"#,
			r#"{"window":"chat:"}"#,
			r#"{"window":"chat:12x"}"#,
			r#"{"window":"stage"}"#,
		] {
			let output = run("activate", None, Some(corrupt), false, None, snapshot(None));
			assert_eq!(texts(&output), ALPHA_TREE, "{corrupt}");
			assert_eq!(interface(&output).window, None, "{corrupt}");
			assert!(output["storage"].is_null());
		}
	}

	#[test]
	fn app_events_refresh_the_tree_without_storage_or_effects() {
		let output = run(
			"refresh",
			None,
			None,
			false,
			Some(AppEventKind::Voice),
			snapshot(None),
		);
		assert_eq!(texts(&output), ALPHA_TREE);
		assert!(output["storage"].is_null());
		assert!(
			output["panel"]
				.as_array()
				.is_none_or(|panel| panel.is_empty())
		);
		no_proposal(&output);

		let off = storage_json(&Saved {
			off: true,
			..Default::default()
		});
		let idle = run(
			"refresh",
			None,
			Some(&off),
			false,
			Some(AppEventKind::Channels),
			snapshot(None),
		);
		assert!(idle["interface_ui"].is_null());
	}

	#[test]
	fn empty_snapshots_stay_inside_the_node_bounds() {
		let mut app = snapshot(None);
		app.guilds = Some(GuildDirectorySnapshot {
			items: (0..100)
				.map(|index| guild(&index.to_string(), "Guild"))
				.collect(),
			truncated: true,
		});
		app.channels = Some(ChannelDirectorySnapshot {
			items: (0..100)
				.map(|index| {
					channel(
						&index.to_string(),
						"99",
						"Room",
						if index % 2 == 0 { 0 } else { 2 },
					)
				})
				.collect(),
			truncated: true,
		});
		let output = run("activate", None, None, false, None, app);
		let tree = interface(&output);
		assert!(tree.nodes.len() <= MAX_INTERFACE_NODES);
		tree.validate().unwrap();

		let cleared = run("activate", None, None, false, None, AppSnapshot::default());
		assert_eq!(texts(&cleared), ["No channels loaded"]);
		interface(&cleared).validate().unwrap();
	}
}
