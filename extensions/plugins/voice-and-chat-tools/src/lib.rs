/*
 * Voice & Chat Tools — bounded Serein adaptations of Equicord's VoiceButtons,
 * VoiceChannelLog and the "Mention all Users" action from VoiceChatUtilities.
 *
 * Upstream: Equicord/Equicord at commit
 * 200dba124066146a8f043de7fc8b05e3dc5e68e, specifically:
 *   src/equicordplugins/voiceButtons/index.tsx
 *   src/equicordplugins/voiceButtons/settings.ts
 *   src/equicordplugins/voiceButtons/utils.tsx
 *   src/equicordplugins/voiceChannelLog/index.tsx
 *   src/equicordplugins/voiceChannelLog/types.ts
 *   src/equicordplugins/voiceChannelLog/logs.ts
 *   src/equicordplugins/voiceChatUtils/index.tsx
 *
 * Serein adaptation copyright (c) 2026 Serein contributors.
 * Changed November 8, 2026: patched Discord panels and REST PATCH calls are
 * replaced by bounded SDK proposals. Participant audio is local-only; voice
 * activity is a current-call session log; mention-all edits the draft only.
 * Server mute/deafen, move, disconnect, soundboard capture and arbitrary
 * channel-wide batches are deliberately omitted.
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License version 3 or (at your
 * option) any later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	AppAction, AppEventKind, AppInvocation, AppOutput, AppSnapshot, Element, HostEffect,
	Invocation, MAX_IO_BYTES, Output, RelationshipKind, UserSnapshot, VoiceSnapshot, serde_json,
};
use std::collections::{BTreeMap, BTreeSet};

/// The host caps a select element at 32 options.
const MAX_SELECT_OPTIONS: usize = 32;
/// The voice snapshot itself is bounded to 64 participants.
const MAX_VOICE_PARTICIPANTS: usize = 64;
/// One stored session log stays small enough for the opaque storage field.
const MAX_LOG_ENTRIES: usize = 256;
/// Serialized plugin storage must stay below this many UTF-8 bytes.
const MAX_STORAGE_BYTES: usize = 48 * 1024;
/// Existing composer tools use the same draft ceiling.
const MAX_DRAFT_BYTES: usize = 8000;
/// Discord's ordinary message limit, in Unicode scalar values.
const MAX_MESSAGE_CHARS: usize = 2000;
/// Panel option and display labels stay short and free of control characters.
const MAX_NAME_BYTES: usize = 80;
const MAX_OPTION_BYTES: usize = 128;
const MAX_PANEL_TEXT_BYTES: usize = 4000;
/// A panel is bounded to 64 elements by the host; this module stays well below.
const MAX_LOG_LINES: usize = 40;
const STORAGE_VERSION: u8 = 1;
const NO_CALL: &str = "No active voice call is loaded. Join a call to use participant tools.";
const CORRUPT_STORAGE: &str = "Saved session data is invalid or too large. It was left unchanged; clearing the log is an explicit reset.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LogKind {
	Join,
	Leave,
	SelfMute,
	SelfDeafen,
	Camera,
	Stream,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogEntry {
	seq: u64,
	kind: LogKind,
	channel_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	user_id: Option<String>,
	#[serde(default)]
	enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PreviousVoice {
	channel_id: Option<String>,
	participants: Vec<String>,
	muted: bool,
	deafened: bool,
	camera: bool,
	streaming: bool,
}

impl PreviousVoice {
	fn from_snapshot(voice: &VoiceSnapshot) -> Self {
		let channel_id = voice
			.channel_id
			.as_deref()
			.filter(|id| valid_snowflake(id))
			.map(str::to_owned);
		let mut seen = BTreeSet::new();
		let participants = voice
			.participants
			.iter()
			.filter(|id| valid_snowflake(id))
			.filter(|id| seen.insert(id.to_string()))
			.take(MAX_VOICE_PARTICIPANTS)
			.cloned()
			.collect();
		Self {
			channel_id,
			participants,
			muted: voice.muted,
			deafened: voice.deafened,
			camera: voice.camera,
			streaming: voice.streaming,
		}
	}

	fn validate(&self) -> Result<(), &'static str> {
		if self
			.channel_id
			.as_deref()
			.is_some_and(|id| !valid_snowflake(id))
			|| self.participants.len() > MAX_VOICE_PARTICIPANTS
		{
			return Err(CORRUPT_STORAGE);
		}
		let mut seen = BTreeSet::new();
		for id in &self.participants {
			if !valid_snowflake(id) || !seen.insert(id) {
				return Err(CORRUPT_STORAGE);
			}
		}
		Ok(())
	}
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Saved {
	version: u8,
	next_seq: u64,
	log: Vec<LogEntry>,
	previous: Option<PreviousVoice>,
}

impl Default for Saved {
	fn default() -> Self {
		Self {
			version: STORAGE_VERSION,
			next_seq: 1,
			log: Vec::new(),
			previous: None,
		}
	}
}

impl Saved {
	fn validate(&self) -> Result<(), &'static str> {
		if self.version != STORAGE_VERSION || self.next_seq == 0 || self.log.len() > MAX_LOG_ENTRIES
		{
			return Err(CORRUPT_STORAGE);
		}
		if let Some(previous) = &self.previous {
			previous.validate()?;
		}
		for entry in &self.log {
			if !valid_snowflake(&entry.channel_id)
				|| entry
					.user_id
					.as_deref()
					.is_some_and(|id| !valid_snowflake(id))
			{
				return Err(CORRUPT_STORAGE);
			}
		}
		Ok(())
	}
}

#[derive(Clone, Debug)]
enum StorageState {
	Empty,
	Valid(Saved),
	Corrupt,
}

impl StorageState {
	fn is_corrupt(&self) -> bool {
		matches!(self, Self::Corrupt)
	}

	fn saved(&self) -> Option<&Saved> {
		match self {
			Self::Valid(saved) => Some(saved),
			Self::Empty | Self::Corrupt => None,
		}
	}
}

fn load_storage(input: &Invocation) -> StorageState {
	let Some(raw) = input.storage.as_deref() else {
		return StorageState::Empty;
	};
	if raw.len() > MAX_STORAGE_BYTES {
		return StorageState::Corrupt;
	}
	match serde_json::from_str::<Saved>(raw) {
		Ok(saved) if saved.validate().is_ok() => StorageState::Valid(saved),
		_ => StorageState::Corrupt,
	}
}

fn save_storage(mut output: AppOutput, saved: &Saved) -> AppOutput {
	if saved.validate().is_err() {
		return output;
	}
	if let Ok(encoded) = serde_json::to_string(saved)
		&& encoded.len() <= MAX_STORAGE_BYTES
	{
		output.output.storage = Some(encoded);
	}
	output
}

fn valid_snowflake(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 20
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
	if value.len() <= max_bytes {
		return value;
	}
	let mut boundary = max_bytes;
	while !value.is_char_boundary(boundary) {
		boundary -= 1;
	}
	&value[..boundary]
}

fn sanitize_label(value: &str) -> String {
	let cleaned: String = value
		.chars()
		.filter(|character| !character.is_control())
		.collect();
	let trimmed = cleaned.trim();
	if trimmed.is_empty() {
		"Unknown".into()
	} else {
		truncate_utf8(trimmed, MAX_NAME_BYTES).to_owned()
	}
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

fn self_id(app: &AppSnapshot) -> Option<&str> {
	app.account_profile
		.as_ref()
		.map(|profile| profile.user.id.as_str())
		.filter(|id| valid_snowflake(id))
}

fn user_names(app: &AppSnapshot) -> BTreeMap<String, String> {
	fn add(names: &mut BTreeMap<String, String>, user: &UserSnapshot) {
		if valid_snowflake(&user.id) {
			names
				.entry(user.id.clone())
				.or_insert_with(|| sanitize_label(&user.name));
		}
	}
	let mut names = BTreeMap::new();
	if let Some(profile) = app.account_profile.as_ref() {
		add(&mut names, &profile.user);
	}
	if let Some(members) = app.members.as_ref() {
		for user in &members.items {
			add(&mut names, user);
		}
	}
	if let Some(relationships) = app.relationships.as_ref() {
		for item in &relationships.items {
			add(&mut names, &item.user);
		}
	}
	names
}

fn friend_ids(app: &AppSnapshot) -> BTreeSet<String> {
	app.relationships
		.as_ref()
		.into_iter()
		.flat_map(|relationships| &relationships.items)
		.filter(|item| item.kind == RelationshipKind::Friend)
		.filter(|item| valid_snowflake(&item.user.id))
		.map(|item| item.user.id.clone())
		.collect()
}

fn user_label(user_id: &str, names: &BTreeMap<String, String>) -> String {
	names
		.get(user_id)
		.cloned()
		.unwrap_or_else(|| format!("user {user_id}"))
}

fn option_label(user_id: &str, names: &BTreeMap<String, String>) -> String {
	let name = names
		.get(user_id)
		.cloned()
		.unwrap_or_else(|| sanitize_label(user_id));
	let label = format!("{name} [{user_id}]");
	truncate_utf8(&label, MAX_OPTION_BYTES).to_owned()
}

/// Remote participants shown in the panel, excluding the signed-in user because
/// the host's participant-audio action only targets other call members.
fn participant_options(app: &AppSnapshot) -> Vec<(String, String)> {
	let Some(voice) = app.voice.as_ref() else {
		return Vec::new();
	};
	let own = self_id(app);
	let names = user_names(app);
	let mut seen = BTreeSet::new();
	voice
		.participants
		.iter()
		.filter(|id| valid_snowflake(id))
		.filter(|id| Some(id.as_str()) != own)
		.filter(|id| seen.insert(id.to_string()))
		.take(MAX_SELECT_OPTIONS)
		.map(|id| (id.clone(), option_label(id, &names)))
		.collect()
}

fn selected_participant(app: &AppSnapshot, input: &Invocation) -> Option<String> {
	let value = input.value("participant")?;
	participant_options(app)
		.into_iter()
		.find(|(_, option)| option.as_str() == value)
		.map(|(id, _)| id)
}

fn select_value(options: &[String], current: Option<&str>) -> String {
	current
		.filter(|value| options.iter().any(|option| option.as_str() == *value))
		.or_else(|| options.first().map(String::as_str))
		.unwrap_or_default()
		.to_owned()
}

fn panel_output(panel: Vec<Element>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		effects: Vec::new(),
	}
}

fn proposal(heading_text: &str, body: String, action: AppAction) -> AppOutput {
	AppOutput {
		output: Output {
			panel: vec![
				heading(heading_text),
				text(body),
				text("Review the proposal and choose Apply. Nothing is changed until then."),
			],
			..Default::default()
		},
		effects: vec![HostEffect::AppAction { action }],
	}
}

fn error_panel(title: &str, message: &str) -> AppOutput {
	panel_output(vec![heading(title), text(message)])
}

fn call_summary(voice: &VoiceSnapshot) -> String {
	let channel = voice.channel_id.as_deref().unwrap_or("unknown");
	let mut states = Vec::new();
	states.push(if voice.muted {
		"microphone muted"
	} else {
		"microphone live"
	});
	states.push(if voice.deafened {
		"deafened"
	} else {
		"not deafened"
	});
	if voice.camera {
		states.push("camera on");
	}
	if voice.streaming {
		states.push("screen share active");
	}
	format!(
		"Channel {channel}, {}, {}. Remote participants loaded: {}.",
		voice.phase,
		states.join(", "),
		voice.participants.len().min(MAX_VOICE_PARTICIPANTS)
	)
}

fn open_panel(input: &AppInvocation) -> AppOutput {
	let app = input.app.as_ref();
	let mut panel = vec![
		heading("Voice participant tools"),
		text(
			"Local-only participant audio and friend DM shortcuts for the current call. Server mute/deafen, move and disconnect are not offered because the host has no reviewed batch action.",
		),
	];
	let voice = app.and_then(|app| app.voice.as_ref());
	if let Some((app, voice)) = app
		.zip(voice)
		.filter(|(_, voice)| voice.channel_id.is_some())
	{
		panel.push(Element::Separator);
		panel.push(text(call_summary(voice)));
		let options = participant_options(app);
		if options.is_empty() {
			panel.push(text(
				"No other remote participants are loaded. Your own microphone, camera and deafen controls stay in the native call bar.",
			));
		} else {
			let option_values = options
				.iter()
				.map(|(_, option)| option.clone())
				.collect::<Vec<_>>();
			if options.len() < voice.participants.len() {
				panel.push(text(
					"Only the first 32 remote participants are shown. Nothing is applied to participants outside this list.",
				));
			}
			panel.push(Element::Select {
				id: "participant".into(),
				label: "Remote participant (local audio only)".into(),
				value: select_value(&option_values, input.invocation.value("participant")),
				options: option_values,
			});
			panel.push(Element::Slider {
				id: "volume".into(),
				label: "Local volume (%)".into(),
				min: 0,
				max: 200,
				value: input
					.invocation
					.parse_value::<i32>("volume")
					.ok()
					.flatten()
					.filter(|value| (0..=200).contains(value))
					.unwrap_or(100),
			});
			panel.push(Element::Checkbox {
				id: "muted".into(),
				label: "Mute this participant locally".into(),
				checked: input
					.invocation
					.parse_value::<bool>("muted")
					.ok()
					.flatten()
					.unwrap_or(false),
			});
			panel.push(button(
				"apply-participant-audio",
				"Propose local volume/mute",
			));
			panel.push(button(
				"open-participant-dm",
				"Propose opening a DM (friends only)",
			));
			panel.push(text(
				"Local audio never changes another person's microphone, and opening a DM is a navigation proposal that Serein applies only after confirmation.",
			));
		}
	} else {
		panel.push(Element::Separator);
		panel.push(text(NO_CALL));
		if let Some(app) = app
			&& app.voice.is_none()
		{
			panel.push(text(
				"The voice snapshot is unavailable because voice state was not granted or no call is loaded.",
			));
		}
	}
	panel.push(Element::Separator);
	panel.push(button("voice-log", "View current-call session log"));
	panel_output(panel)
}

fn apply_participant_audio(input: &AppInvocation) -> AppOutput {
	let Some(app) = input.app.as_ref() else {
		return error_panel(
			"Participant audio",
			"No app snapshot is available for this invocation.",
		);
	};
	let Some(voice) = app
		.voice
		.as_ref()
		.filter(|voice| voice.channel_id.is_some())
	else {
		return error_panel("Participant audio", NO_CALL);
	};
	let Some(user_id) = selected_participant(app, &input.invocation) else {
		return error_panel(
			"Participant audio",
			"Choose a remote participant who is still in the current call. Nothing was proposed.",
		);
	};
	let volume = match input.invocation.parse_value::<u16>("volume") {
		Ok(Some(value)) if value <= 200 => value,
		Ok(None) => 100,
		_ => {
			return error_panel(
				"Participant audio",
				"Local volume must be a whole number between 0 and 200. Nothing was proposed.",
			);
		}
	};
	let muted = match input.invocation.parse_value::<bool>("muted") {
		Ok(Some(value)) => value,
		Ok(None) => false,
		Err(_) => {
			return error_panel(
				"Participant audio",
				"The local mute value is invalid. Nothing was proposed.",
			);
		}
	};
	let names = user_names(app);
	let label = user_label(&user_id, &names);
	proposal(
		"Participant audio",
		format!(
			"Propose local volume {volume}% and local mute {muted} for {label} in channel {}.",
			voice.channel_id.as_deref().unwrap_or("unknown")
		),
		AppAction::SetParticipantAudio {
			user_id,
			volume_percent: Some(volume),
			muted: Some(muted),
		},
	)
}

fn open_participant_dm(input: &AppInvocation) -> AppOutput {
	let Some(app) = input.app.as_ref() else {
		return error_panel(
			"Open DM",
			"No app snapshot is available for this invocation.",
		);
	};
	let Some(user_id) = selected_participant(app, &input.invocation) else {
		return error_panel(
			"Open DM",
			"Choose a remote participant who is still in the current call. Nothing was proposed.",
		);
	};
	if !friend_ids(app).contains(&user_id) {
		return error_panel(
			"Open DM",
			"Only a participant in your loaded friend list can be opened from this panel. Nothing was proposed.",
		);
	}
	let names = user_names(app);
	let label = user_label(&user_id, &names);
	proposal(
		"Open DM",
		format!("Propose opening direct messages with {label}."),
		AppAction::OpenFriendDm { user_id },
	)
}

fn push_entry(
	saved: &mut Saved,
	kind: LogKind,
	channel_id: &str,
	user_id: Option<&str>,
	enabled: bool,
) {
	if !valid_snowflake(channel_id) || user_id.is_some_and(|id| !valid_snowflake(id)) {
		return;
	}
	let entry = LogEntry {
		seq: saved.next_seq,
		kind,
		channel_id: channel_id.to_owned(),
		user_id: user_id.map(str::to_owned),
		enabled,
	};
	saved.next_seq = saved.next_seq.saturating_add(1);
	saved.log.push(entry);
	if saved.log.len() > MAX_LOG_ENTRIES {
		let excess = saved.log.len() - MAX_LOG_ENTRIES;
		saved.log.drain(..excess);
	}
}

/// Compare the current call snapshot with the last one and append only real
/// changes. The first snapshot seeds the baseline so enabling the plugin does
/// not fabricate a burst of joins.
fn apply_voice_change(saved: &mut Saved, voice: &VoiceSnapshot) -> bool {
	let current = PreviousVoice::from_snapshot(voice);
	let previous = saved.previous.take();
	let mut changed = false;
	if let Some(previous) = previous {
		if previous.channel_id != current.channel_id {
			if let Some(old) = previous.channel_id.as_deref() {
				push_entry(saved, LogKind::Leave, old, None, false);
				changed = true;
			}
			if let Some(new) = current.channel_id.as_deref() {
				push_entry(saved, LogKind::Join, new, None, false);
				changed = true;
			}
		} else if let Some(channel) = current.channel_id.as_deref() {
			let old_users = previous
				.participants
				.iter()
				.map(String::as_str)
				.collect::<BTreeSet<_>>();
			let new_users = current
				.participants
				.iter()
				.map(String::as_str)
				.collect::<BTreeSet<_>>();
			for user_id in old_users.difference(&new_users) {
				push_entry(saved, LogKind::Leave, channel, Some(*user_id), false);
				changed = true;
			}
			for user_id in new_users.difference(&old_users) {
				push_entry(saved, LogKind::Join, channel, Some(*user_id), false);
				changed = true;
			}
			if previous.muted != current.muted {
				push_entry(saved, LogKind::SelfMute, channel, None, current.muted);
				changed = true;
			}
			if previous.deafened != current.deafened {
				push_entry(saved, LogKind::SelfDeafen, channel, None, current.deafened);
				changed = true;
			}
			if previous.camera != current.camera {
				push_entry(saved, LogKind::Camera, channel, None, current.camera);
				changed = true;
			}
			if previous.streaming != current.streaming {
				push_entry(saved, LogKind::Stream, channel, None, current.streaming);
				changed = true;
			}
		}
	} else if current.channel_id.is_some() || !current.participants.is_empty() {
		// Seed the first observed call without emitting events for people who
		// were already present when the plugin was enabled.
		changed = true;
	}
	saved.previous = Some(current);
	changed
}

fn observe_voice(input: &AppInvocation) -> AppOutput {
	let Some(event) = input.app_event else {
		return AppOutput::default();
	};
	if !matches!(event, AppEventKind::Ready | AppEventKind::Voice) {
		return AppOutput::default();
	}
	let state = load_storage(&input.invocation);
	let mut saved = match state {
		StorageState::Empty => Saved::default(),
		StorageState::Valid(saved) => saved,
		StorageState::Corrupt => return AppOutput::default(),
	};
	let Some(app) = input.app.as_ref() else {
		return AppOutput::default();
	};
	let Some(voice) = app.voice.as_ref() else {
		return AppOutput::default();
	};
	if event == AppEventKind::Ready {
		// Seed the baseline at enable time, including the idle case, so a real
		// join afterwards is recorded without inventing joins for people who
		// were already in the call when the plugin was enabled.
		saved.previous = Some(PreviousVoice::from_snapshot(voice));
		return save_storage(AppOutput::default(), &saved);
	}
	if !apply_voice_change(&mut saved, voice) {
		return AppOutput::default();
	}
	save_storage(AppOutput::default(), &saved)
}

fn log_kind_label(entry: &LogEntry, names: &BTreeMap<String, String>) -> String {
	let who = entry
		.user_id
		.as_deref()
		.map(|id| user_label(id, names))
		.unwrap_or_else(|| "You".into());
	match entry.kind {
		LogKind::Join => format!("{who} joined channel {}", entry.channel_id),
		LogKind::Leave => format!("{who} left channel {}", entry.channel_id),
		LogKind::SelfMute => format!(
			"You {} the microphone in channel {}",
			if entry.enabled { "muted" } else { "unmuted" },
			entry.channel_id
		),
		LogKind::SelfDeafen => format!(
			"You {} in channel {}",
			if entry.enabled {
				"deafened"
			} else {
				"undeafened"
			},
			entry.channel_id
		),
		LogKind::Camera => format!(
			"You turned the camera {} in channel {}",
			if entry.enabled { "on" } else { "off" },
			entry.channel_id
		),
		LogKind::Stream => format!(
			"You {} screen sharing in channel {}",
			if entry.enabled { "started" } else { "stopped" },
			entry.channel_id
		),
	}
}

fn voice_log_panel(input: &AppInvocation) -> AppOutput {
	let state = load_storage(&input.invocation);
	let mut panel = vec![
		heading("Current-call session log"),
		text(
			"Bounded session activity for the call this plugin observed. Entries are stored on this device for this account and are not sent to Discord. The host exposes no wall-clock timestamp, so order is shown by sequence only.",
		),
	];
	if state.is_corrupt() {
		panel.push(text(CORRUPT_STORAGE));
		panel.push(button("clear-voice-log", "Clear and reset stored log"));
		return panel_output(panel);
	}
	let entries: &[LogEntry] = state.saved().map_or(&[], |saved| saved.log.as_slice());
	if entries.is_empty() {
		panel.push(text(
			"No voice activity has been recorded yet. Joins, leaves and your own mute/deafen/camera/screen-share changes appear after the first change.",
		));
	} else {
		let app = input.app.as_ref();
		let names = app.map(user_names).unwrap_or_default();
		let lines = entries
			.iter()
			.rev()
			.take(MAX_LOG_LINES)
			.map(|entry| format!("#{} {}", entry.seq, log_kind_label(entry, &names)))
			.collect::<Vec<_>>()
			.join("\n");
		panel.push(Element::Separator);
		panel.push(text(truncate_utf8(&lines, MAX_PANEL_TEXT_BYTES)));
		if entries.len() > MAX_LOG_LINES {
			panel.push(text(format!(
				"Showing the newest {MAX_LOG_LINES} of {} stored entries; at most {MAX_LOG_ENTRIES} are kept.",
				entries.len()
			)));
		}
		panel.push(text(format!(
			"{} of at most {MAX_LOG_ENTRIES} entries stored.",
			entries.len()
		)));
	}
	panel.push(Element::Separator);
	panel.push(button("open", "Back to participant tools"));
	panel.push(button("clear-voice-log", "Clear stored session log"));
	panel_output(panel)
}

fn clear_voice_log() -> AppOutput {
	let saved = Saved::default();
	let mut output = panel_output(vec![
		heading("Session log cleared"),
		text(
			"Stored current-call voice activity was cleared on this device. Future changes will be recorded from a new baseline.",
		),
		button("open", "Back to participant tools"),
	]);
	output = save_storage(output, &saved);
	output
}

fn mention_all_voice(input: &AppInvocation) -> AppOutput {
	let Some(draft) = input.invocation.composer.as_deref() else {
		return panel_output(vec![text(
			"No composer draft was supplied; open the message box and try again.",
		)]);
	};
	if draft.len() > MAX_DRAFT_BYTES {
		return panel_output(vec![text(
			"The draft is larger than the 8,000-byte composer limit. It was left unchanged.",
		)]);
	}
	let Some(app) = input.app.as_ref() else {
		return panel_output(vec![text(
			"No voice snapshot is available, so no mentions were proposed.",
		)]);
	};
	let Some(voice) = app
		.voice
		.as_ref()
		.filter(|voice| voice.channel_id.is_some())
	else {
		return panel_output(vec![text(NO_CALL)]);
	};
	let own = self_id(app);
	let mut seen = BTreeSet::new();
	let mentions = voice
		.participants
		.iter()
		.filter(|id| valid_snowflake(id))
		.filter(|id| Some(id.as_str()) != own)
		.filter(|id| seen.insert(id.to_string()))
		.take(MAX_VOICE_PARTICIPANTS)
		.map(|id| format!("<@{id}>"))
		.collect::<Vec<_>>();
	if mentions.is_empty() {
		return panel_output(vec![text(
			"No other participants are loaded in the current call, so the draft was left unchanged.",
		)]);
	}
	let joined = mentions.join(" ");
	let mut replacement = if draft.is_empty() {
		joined
	} else if draft.chars().last().is_some_and(char::is_whitespace) {
		format!("{draft}{joined}")
	} else {
		format!("{draft} {joined}")
	};
	replacement.push(' ');
	if replacement.chars().count() > MAX_MESSAGE_CHARS || replacement.len() > MAX_DRAFT_BYTES {
		return panel_output(vec![text(
			"Adding every current participant would exceed the 2,000-character message limit. The draft was left unchanged; mention fewer people.",
		)]);
	}
	AppOutput {
		output: Output {
			replacement: Some(replacement),
			panel: vec![text(format!(
				"Proposed {} mention(s) from the current call. Review the draft and choose Apply; nothing is sent automatically.",
				mentions.len()
			))],
			..Default::default()
		},
		effects: Vec::new(),
	}
}

fn bounded(mut output: AppOutput) -> AppOutput {
	if serde_json::to_vec(&output).map_or(true, |bytes| bytes.len() > MAX_IO_BYTES) {
		output.output.panel = vec![text(
			"The response exceeded the 256 KiB limit, so the view was reduced. Nothing was applied.",
		)];
		output.output.storage = None;
		output.effects.clear();
	}
	output
}

fn handle(input: AppInvocation) -> AppOutput {
	let action = input.invocation.action.as_str();
	let output = match action {
		"open" => open_panel(&input),
		"apply-participant-audio" => apply_participant_audio(&input),
		"open-participant-dm" => open_participant_dm(&input),
		"voice-log" => voice_log_panel(&input),
		"clear-voice-log" => clear_voice_log(),
		"mention-all-voice" => mention_all_voice(&input),
		"observe-voice" => observe_voice(&input),
		_ => AppOutput::default(),
	};
	bounded(output)
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{
		AccountProfileSnapshot, MembersSnapshot, RelationshipSnapshot, RelationshipsSnapshot,
	};

	fn voice(channel: Option<&str>, participants: &[&str]) -> VoiceSnapshot {
		VoiceSnapshot {
			channel_id: channel.map(str::to_owned),
			phase: "connected".into(),
			muted: false,
			deafened: false,
			camera: false,
			streaming: false,
			participants: participants.iter().map(|id| (*id).to_owned()).collect(),
		}
	}

	fn app_with(
		voice: Option<VoiceSnapshot>,
		own: &str,
		members: &[(&str, &str)],
		friends: &[(&str, &str)],
	) -> AppSnapshot {
		AppSnapshot {
			voice,
			account_profile: Some(AccountProfileSnapshot {
				user: UserSnapshot {
					id: own.into(),
					name: "Owner".into(),
				},
				avatar: None,
				profile: None,
			}),
			relationships: Some(RelationshipsSnapshot {
				items: friends
					.iter()
					.map(|(id, name)| RelationshipSnapshot {
						user: UserSnapshot {
							id: (*id).into(),
							name: (*name).into(),
						},
						kind: RelationshipKind::Friend,
					})
					.collect(),
				truncated: false,
				friends_known: true,
				requests_known: false,
				restricted_known: false,
			}),
			members: Some(MembersSnapshot {
				channel_id: "10".into(),
				items: members
					.iter()
					.map(|(id, name)| UserSnapshot {
						id: (*id).into(),
						name: (*name).into(),
					})
					.collect(),
				truncated: false,
			}),
			..Default::default()
		}
	}

	fn run(
		action: &str,
		app: Option<AppSnapshot>,
		values: &[(&str, &str)],
		storage: Option<&str>,
		composer: Option<&str>,
	) -> AppOutput {
		handle(AppInvocation {
			invocation: Invocation {
				action: action.into(),
				composer: composer.map(str::to_owned),
				storage: storage.map(str::to_owned),
				values: values
					.iter()
					.map(|(key, value)| ((*key).into(), (*value).into()))
					.collect(),
				..Default::default()
			},
			app,
			..Default::default()
		})
	}

	fn panel_text(output: &AppOutput) -> String {
		output
			.output
			.panel
			.iter()
			.filter_map(|element| match element {
				Element::Text { text } => Some(text.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n")
	}

	fn previous() -> PreviousVoice {
		PreviousVoice {
			channel_id: Some("10".into()),
			participants: vec!["1".into(), "2".into()],
			muted: false,
			deafened: false,
			camera: false,
			streaming: false,
		}
	}

	#[test]
	fn voice_changes_are_diffed_once_and_record_self_state() {
		let mut saved = Saved {
			previous: Some(previous()),
			..Default::default()
		};
		let mut current = voice(Some("10"), &["1", "3"]);
		current.muted = true;
		current.camera = true;
		assert!(apply_voice_change(&mut saved, &current));
		let kinds = saved
			.log
			.iter()
			.map(|entry| (entry.kind, entry.user_id.as_deref()))
			.collect::<Vec<_>>();
		assert_eq!(
			kinds,
			vec![
				(LogKind::Leave, Some("2")),
				(LogKind::Join, Some("3")),
				(LogKind::SelfMute, None),
				(LogKind::Camera, None),
			]
		);
		let len = saved.log.len();
		assert!(!apply_voice_change(&mut saved, &current));
		assert_eq!(saved.log.len(), len);
	}

	#[test]
	fn first_snapshot_seeds_without_fabricating_joins() {
		let mut saved = Saved::default();
		assert!(apply_voice_change(
			&mut saved,
			&voice(Some("10"), &["1", "2"])
		));
		assert!(saved.log.is_empty());
		assert_eq!(saved.previous.as_ref().unwrap().participants.len(), 2);
	}

	#[test]
	fn ready_baseline_records_a_subsequent_real_join() {
		let idle = app_with(Some(voice(None, &[])), "9", &[], &[]);
		let ready = handle(AppInvocation {
			invocation: Invocation {
				action: "observe-voice".into(),
				..Default::default()
			},
			app: Some(idle),
			app_event: Some(AppEventKind::Ready),
			..Default::default()
		});
		let baseline = ready.output.storage.expect("ready seeds storage");

		let joined_app = app_with(Some(voice(Some("10"), &["9", "1"])), "9", &[], &[]);
		let joined = handle(AppInvocation {
			invocation: Invocation {
				action: "observe-voice".into(),
				storage: Some(baseline),
				..Default::default()
			},
			app: Some(joined_app),
			app_event: Some(AppEventKind::Voice),
			..Default::default()
		});
		let stored: Saved =
			serde_json::from_str(joined.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(stored.log.len(), 1);
		assert_eq!(stored.log[0].kind, LogKind::Join);
		assert_eq!(stored.log[0].user_id, None);
	}

	#[test]
	fn log_entries_are_capped_and_validated() {
		let mut saved = Saved::default();
		for _ in 0..(MAX_LOG_ENTRIES + 20) {
			push_entry(&mut saved, LogKind::Join, "10", Some("1"), false);
		}
		assert_eq!(saved.log.len(), MAX_LOG_ENTRIES);
		assert!(saved.validate().is_ok());

		let mut too_many = saved.clone();
		too_many.log.push(LogEntry {
			seq: 1,
			kind: LogKind::Join,
			channel_id: "10".into(),
			user_id: None,
			enabled: false,
		});
		too_many.log.push(LogEntry {
			seq: 2,
			kind: LogKind::Join,
			channel_id: "10".into(),
			user_id: None,
			enabled: false,
		});
		assert!(too_many.validate().is_err());

		let wrong_version = Saved {
			version: STORAGE_VERSION + 1,
			..Default::default()
		};
		assert!(wrong_version.validate().is_err());

		let mut invalid_id = Saved::default();
		invalid_id.log.push(LogEntry {
			seq: 1,
			kind: LogKind::Join,
			channel_id: "not-an-id".into(),
			user_id: None,
			enabled: false,
		});
		assert!(invalid_id.validate().is_err());
	}

	#[test]
	fn corrupt_storage_is_preserved_until_an_explicit_clear() {
		let app = app_with(Some(voice(Some("10"), &["1", "2"])), "9", &[], &[]);
		let output = handle(AppInvocation {
			invocation: Invocation {
				action: "observe-voice".into(),
				storage: Some("{not json".into()),
				..Default::default()
			},
			app: Some(app.clone()),
			app_event: Some(AppEventKind::Voice),
			..Default::default()
		});
		assert!(output.output.storage.is_none());
		assert!(output.effects.is_empty());

		let cleared = run("clear-voice-log", Some(app), &[], Some("{not json"), None);
		let stored: Saved = serde_json::from_str(
			cleared
				.output
				.storage
				.as_deref()
				.expect("clear writes storage"),
		)
		.expect("clear writes valid JSON");
		assert!(stored.log.is_empty());
		assert!(stored.previous.is_none());
	}

	#[test]
	fn participant_audio_requires_a_current_remote_participant_and_bounded_volume() {
		let app = app_with(Some(voice(Some("10"), &["1", "2", "9"])), "9", &[], &[]);
		let option = participant_options(&app)
			.into_iter()
			.find(|(id, _)| id == "1")
			.expect("remote option exists")
			.1;
		let applied = run(
			"apply-participant-audio",
			Some(app.clone()),
			&[
				("participant", option.as_str()),
				("volume", "150"),
				("muted", "true"),
			],
			None,
			None,
		);
		assert_eq!(
			applied.effects,
			vec![HostEffect::AppAction {
				action: AppAction::SetParticipantAudio {
					user_id: "1".into(),
					volume_percent: Some(150),
					muted: Some(true),
				}
			}]
		);

		let oversized = run(
			"apply-participant-audio",
			Some(app.clone()),
			&[("participant", option.as_str()), ("volume", "201")],
			None,
			None,
		);
		assert!(oversized.effects.is_empty());
		assert!(panel_text(&oversized).contains("between 0 and 200"));

		let self_option = "Owner [9]".to_owned();
		let own = run(
			"apply-participant-audio",
			Some(app.clone()),
			&[("participant", self_option.as_str())],
			None,
			None,
		);
		assert!(own.effects.is_empty());

		let stale = run(
			"apply-participant-audio",
			Some(app),
			&[("participant", "Unknown [999]")],
			None,
			None,
		);
		assert!(stale.effects.is_empty());
	}

	#[test]
	fn dm_action_only_targets_loaded_friends() {
		let app = app_with(Some(voice(Some("10"), &["1"])), "9", &[], &[]);
		let option = participant_options(&app)[0].1.clone();
		let blocked = run(
			"open-participant-dm",
			Some(app.clone()),
			&[("participant", option.as_str())],
			None,
			None,
		);
		assert!(blocked.effects.is_empty());
		assert!(panel_text(&blocked).contains("friend list"));

		let friend_app = app_with(Some(voice(Some("10"), &["1"])), "9", &[], &[("1", "Alice")]);
		let allowed = run(
			"open-participant-dm",
			Some(friend_app),
			&[("participant", "Alice [1]")],
			None,
			None,
		);
		assert_eq!(
			allowed.effects,
			vec![HostEffect::AppAction {
				action: AppAction::OpenFriendDm {
					user_id: "1".into()
				}
			}]
		);
	}

	#[test]
	fn mention_all_excludes_self_dedupes_and_enforces_the_message_limit() {
		let app = app_with(
			Some(voice(Some("10"), &["1", "2", "2", "9"])),
			"9",
			&[],
			&[],
		);
		let output = run(
			"mention-all-voice",
			Some(app.clone()),
			&[],
			None,
			Some("hello"),
		);
		assert_eq!(
			output.output.replacement.as_deref(),
			Some("hello <@1> <@2> ")
		);

		let almost_full = "x".repeat(MAX_MESSAGE_CHARS - 10);
		let too_long = run(
			"mention-all-voice",
			Some(app.clone()),
			&[],
			None,
			Some(almost_full.as_str()),
		);
		assert!(too_long.output.replacement.is_none());
		assert!(panel_text(&too_long).contains("2,000-character"));

		let no_call = run(
			"mention-all-voice",
			Some(app_with(None, "9", &[], &[])),
			&[],
			None,
			Some("hello"),
		);
		assert!(no_call.output.replacement.is_none());
	}

	#[test]
	fn participant_options_are_unique_sanitized_and_bounded() {
		let members = (0..MAX_VOICE_PARTICIPANTS + 10)
			.map(|index| {
				let id = (index + 1).to_string();
				(id, format!("Name\u{0}{}\n", "x".repeat(100)))
			})
			.collect::<Vec<_>>();
		let member_refs = members
			.iter()
			.map(|(id, name)| (id.as_str(), name.as_str()))
			.collect::<Vec<_>>();
		let participants = members
			.iter()
			.map(|(id, _)| id.as_str())
			.collect::<Vec<_>>();
		let app = app_with(
			Some(voice(Some("10"), &participants)),
			"999",
			&member_refs,
			&[],
		);
		let options = participant_options(&app);
		assert_eq!(options.len(), MAX_SELECT_OPTIONS);
		let mut seen = BTreeSet::new();
		for (_, option) in options {
			assert!(option.len() <= MAX_OPTION_BYTES);
			assert!(!option.chars().any(char::is_control));
			assert!(seen.insert(option));
		}
	}

	#[test]
	fn voice_log_panel_is_bounded_and_shows_newest_first() {
		let mut saved = Saved::default();
		for index in 0..(MAX_LOG_LINES + 5) {
			let user_id = (index + 1).to_string();
			push_entry(
				&mut saved,
				LogKind::Join,
				"10",
				Some(user_id.as_str()),
				false,
			);
		}
		let storage = serde_json::to_string(&saved).unwrap();
		let output = run("voice-log", None, &[], Some(storage.as_str()), None);
		let body = panel_text(&output);
		assert!(body.contains(&format!("#{}", saved.log.last().unwrap().seq)));
		assert!(body.contains("Showing the newest"));
		assert!(output.output.panel.len() <= 64);
	}
}
