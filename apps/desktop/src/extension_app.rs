//! Explicitly granted, bounded snapshots of already-loaded client data. No IO.
use client_core::{State, auth::AuthState};
use extensions::*;
use model::{Freshness, Id};

pub fn available(state: &State) -> bool {
	(state.demo || state.auth == AuthState::Authenticated)
		&& state.user.as_ref().is_some_and(|user| user.id.0 != 0)
}

pub fn uses_app(capabilities: &[Capability]) -> bool {
	capabilities.iter().any(|capability| {
		matches!(
			capability,
			Capability::AppContext
				| Capability::ChannelDirectory
				| Capability::Timeline
				| Capability::Members
				| Capability::Presence
				| Capability::VoiceState
				| Capability::ReadState
				| Capability::LocalSettings
				| Capability::Navigation
				| Capability::LocalNotices
				| Capability::ClipboardWrite
				| Capability::VoiceControl
				| Capability::AppEvents
		)
	})
}

fn name(value: &str) -> String {
	let mut end = value.len().min(128);
	while !value.is_char_boundary(end) {
		end -= 1;
	}
	let name: String = value[..end].chars().filter(|c| !c.is_control()).collect();
	if name.trim().is_empty() {
		"Unnamed".into()
	} else {
		name
	}
}

fn user(value: &model::User) -> UserSnapshot {
	UserSnapshot {
		id: value.id.0.to_string(),
		name: name(&value.name),
	}
}

fn channel(value: &model::Channel) -> ChannelSnapshot {
	ChannelSnapshot {
		id: value.id.0.to_string(),
		guild_id: value.guild.map(|id| id.0.to_string()),
		name: name(&value.name),
		kind: value.kind,
	}
}

// Each candidate is already scalar-bounded before serialization. Account for JSON and
// fixed list storage; these per-group budgets also leave room below the 64 KiB ABI cap.
fn push<T: serde::Serialize>(items: &mut Vec<T>, item: T, left: &mut usize, max: usize) -> bool {
	let bytes = serde_json::to_vec(&item).map_or(usize::MAX, |bytes| {
		bytes.len().saturating_add(std::mem::size_of::<T>())
	});
	if items.len() >= max || bytes > *left {
		return false;
	}
	*left -= bytes;
	items.push(item);
	true
}

pub fn snapshot(
	state: &State,
	messaging: &ui::MessagingUi,
	manifest: &Manifest,
) -> Option<Box<AppSnapshot>> {
	if !available(state) || !uses_app(&manifest.capabilities) {
		return None;
	}
	let granted = |capability| manifest.capabilities.contains(&capability);
	let connected = state.gateway_connected;
	let selected = state
		.selected
		.filter(|id| connected && state.freshness != Freshness::Unavailable && state.can_view(*id));
	let readable = selected.filter(|id| {
		state.can_read_history(*id)
			&& state.freshness == Freshness::Fresh
			&& state.channel(*id).is_some_and(|c| c.supports_text())
	});
	let mut app = AppSnapshot::default();
	if granted(Capability::AppContext) {
		app.context = Some(AppContextSnapshot {
			connected,
			user: state.user.as_ref().map(user),
			channel: selected.and_then(|id| state.channel(id)).map(channel),
		});
	}
	if connected && granted(Capability::ChannelDirectory) {
		let mut directory = ChannelDirectorySnapshot {
			items: Vec::new(),
			truncated: false,
		};
		let mut budget = 12 * 1024;
		for value in state.channels.iter().filter(|c| {
			state.can_view(c.id)
				&& !(state.selected == Some(c.id) && state.freshness == Freshness::Unavailable)
		}) {
			if !push(
				&mut directory.items,
				channel(value),
				&mut budget,
				MAX_APP_CHANNELS,
			) {
				directory.truncated = true;
				break;
			}
		}
		app.channels = Some(directory);
	}
	if let Some(id) = readable
		&& granted(Capability::Timeline)
	{
		let mut timeline = TimelineSnapshot {
			channel_id: id.0.to_string(),
			messages: Vec::new(),
			truncated: state.history_before.is_some()
				|| state.history_after.is_some()
				|| !state.older_exhausted,
		};
		let mut budget = 24 * 1024;
		for message in
			state.timeline.iter().rev().filter(|m| {
				m.channel == id && !m.ephemeral && m.flags & 64 == 0 && m.author.id.0 != 0
			}) {
			if message.content.len() > 4096 {
				timeline.truncated = true;
				continue;
			}
			if !push(
				&mut timeline.messages,
				MessageSnapshot {
					id: message.id.0.to_string(),
					author: user(&message.author),
					content: message.content.clone(),
					attachment_count: message.attachments.len().min(u16::MAX as usize) as u16,
					edited: message.edited,
				},
				&mut budget,
				MAX_APP_MESSAGES,
			) {
				timeline.truncated = true;
				break;
			}
		}
		timeline.messages.reverse();
		app.timeline = Some(timeline);
	}
	if let Some(id) = selected {
		let members = state
			.members
			.as_ref()
			.filter(|members| members.channel == id && members.freshness == Freshness::Fresh);
		let recipients = state.channel(id).filter(|c| c.guild.is_none());
		if granted(Capability::Members) && (members.is_some() || recipients.is_some()) {
			let mut group = MembersSnapshot {
				channel_id: id.0.to_string(),
				items: Vec::new(),
				truncated: false,
			};
			let mut budget = 8 * 1024;
			let users = members
				.into_iter()
				.flat_map(|m| m.rows.iter().flatten().map(|m| &m.user))
				.chain(
					recipients
						.filter(|_| members.is_none())
						.into_iter()
						.flat_map(|c| &c.recipients),
				);
			for value in users.filter(|u| u.id.0 != 0) {
				let value = user(value);
				if group.items.iter().any(|item| item.id == value.id) {
					continue;
				}
				if !push(&mut group.items, value, &mut budget, MAX_APP_MEMBERS) {
					group.truncated = true;
					break;
				}
			}
			group.truncated |= members.is_some_and(|m| m.total > group.items.len() as u64);
			app.members = Some(group);
		}
		if granted(Capability::Presence) && (members.is_some() || recipients.is_some()) {
			let mut presence = PresenceSnapshot {
				items: Vec::new(),
				truncated: false,
			};
			let mut budget = 8 * 1024;
			let statuses = members
				.into_iter()
				.flat_map(|m| {
					m.rows
						.iter()
						.flatten()
						.filter_map(|m| m.status.as_deref().map(|status| (m.user.id, status)))
				})
				.chain(
					recipients
						.filter(|_| members.is_none())
						.into_iter()
						.flat_map(|c| &c.recipients)
						.filter_map(|u| {
							state
								.presence_for(u.id)
								.and_then(|p| p.status.as_deref())
								.map(|s| (u.id, s))
						}),
				);
			for (user, status) in statuses {
				let user_id = user.0.to_string();
				if presence.items.iter().any(|item| item.user_id == user_id) {
					continue;
				}
				if !push(
					&mut presence.items,
					PresenceEntry {
						user_id,
						status: status.into(),
					},
					&mut budget,
					MAX_APP_PRESENCES,
				) {
					presence.truncated = true;
					break;
				}
			}
			presence.truncated |= members.is_some_and(|m| m.total > presence.items.len() as u64);
			app.presence = Some(presence);
		}
	}
	if granted(Capability::VoiceState) {
		let call = state.voice.active.as_ref().filter(|call| {
			state.can_view(call.channel)
				&& !(state.selected == Some(call.channel)
					&& state.freshness == Freshness::Unavailable)
		});
		app.voice = Some(VoiceSnapshot {
			channel_id: call.map(|c| c.channel.0.to_string()),
			phase: call.map_or("idle", |c| phase(c.phase)).into(),
			muted: call.is_some_and(|c| c.muted),
			deafened: call.is_some_and(|c| c.deafened),
			camera: call.is_some_and(|c| c.camera),
			streaming: call.is_some() && messaging.screen.busy,
			participants: call
				.into_iter()
				.flat_map(|c| &c.participants)
				.filter(|p| p.user.0 != 0)
				.take(MAX_VOICE_PARTICIPANTS)
				.map(|p| p.user.0.to_string())
				.collect(),
		});
	}
	if granted(Capability::ReadState) {
		app.read_state = Some(ReadSnapshot {
			channel_id: selected.map(|id| id.0.to_string()),
			unread: selected.and_then(|id| state.unread(id)),
			mentions: selected.map_or(0, |id| state.mention_count(id)),
		});
	}
	if granted(Capability::LocalSettings) {
		app.settings = Some(messaging.extension_local_settings());
	}
	// Keep the boundary authoritative if model data or serialization changes later.
	app.validate(manifest).ok()?;
	Some(Box::new(app))
}

fn phase(phase: client_core::voice::Phase) -> &'static str {
	use client_core::voice::Phase::*;
	match phase {
		Connecting => "connecting",
		ConnectingTransport => "connecting_transport",
		Discovering => "discovering",
		OpeningAudio => "opening_audio",
		Ringing => "ringing",
		Securing => "securing",
		Connected => "connected",
		Waiting => "waiting",
		Failed => "failed",
	}
}

/// Fixed-size change key: no copied messages, user names, or per-frame plugin execution.
#[derive(PartialEq, Eq)]
pub struct ChangeKey {
	channel: Option<Id>,
	connected: bool,
	context: (Freshness, bool, Option<Freshness>),
	voice: Option<VoiceKey>,
	settings: LocalSettingsSnapshot,
}
#[derive(PartialEq, Eq)]
struct VoiceKey {
	channel: Id,
	request: u64,
	phase: client_core::voice::Phase,
	muted: bool,
	deafened: bool,
	camera: bool,
	streaming: bool,
	participants: u64,
}
impl ChangeKey {
	pub fn capture(state: &State, messaging: &ui::MessagingUi) -> Self {
		use std::hash::{Hash, Hasher};
		Self {
			channel: state.selected,
			connected: state.gateway_connected,
			context: (
				state.freshness,
				state.history_pending,
				state
					.members
					.as_ref()
					.filter(|members| Some(members.channel) == state.selected)
					.map(|members| members.freshness),
			),
			voice: state.voice.active.as_ref().map(|call| {
				let mut participants = std::collections::hash_map::DefaultHasher::new();
				for p in call.participants.iter().take(MAX_VOICE_PARTICIPANTS) {
					(p.user.0, p.muted, p.deafened, p.video, p.streaming).hash(&mut participants);
				}
				VoiceKey {
					channel: call.channel,
					request: call.request,
					phase: call.phase,
					muted: call.muted,
					deafened: call.deafened,
					camera: call.camera,
					streaming: messaging.screen.busy,
					participants: participants.finish(),
				}
			}),
			settings: messaging.extension_local_settings(),
		}
	}
	pub fn changed(&self, old: &Self) -> Option<AppEventKind> {
		if self.connected != old.connected {
			Some(AppEventKind::Connection)
		} else if self.channel != old.channel {
			Some(AppEventKind::Navigation)
		} else if self.context != old.context {
			Some(AppEventKind::Context)
		} else if self.voice != old.voice {
			Some(AppEventKind::Voice)
		} else if self.settings != old.settings {
			Some(AppEventKind::Settings)
		} else {
			None
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn manifest(capabilities: Vec<Capability>) -> Manifest {
		Manifest {
			api_version: API_VERSION,
			id: "snapshot-test".into(),
			name: "Synthetic test".into(),
			version: "1".into(),
			author: "Tests".into(),
			license: "MIT".into(),
			source: "https://example.org/source".into(),
			kind: ExtensionKind::Plugin,
			capabilities,
			actions: vec![Action {
				id: "show".into(),
				label: "Show".into(),
				surface: Surface::Panel,
			}],
		}
	}
	#[test]
	fn extension_app_snapshot_is_granted_current_and_byte_bounded() {
		let mut state = test_support::demo_state();
		let messaging = ui::MessagingUi::default();
		assert!(snapshot(&state, &messaging, &manifest(vec![Capability::Storage])).is_none());
		let caps = manifest(vec![
			Capability::AppContext,
			Capability::Timeline,
			Capability::ChannelDirectory,
			Capability::Members,
			Capability::Presence,
			Capability::VoiceState,
			Capability::ReadState,
			Capability::LocalSettings,
		]);
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert_eq!(
			app.context.as_ref().unwrap().channel.as_ref().unwrap().id,
			"20"
		);
		assert!(app.bytes().unwrap() <= MAX_APP_SNAPSHOT_BYTES);
		assert!(app.timeline.as_ref().unwrap().messages.len() <= MAX_APP_MESSAGES);
		assert!(app.timeline.as_ref().unwrap().truncated);
		let toolbox = parse_package(include_bytes!(
			"../../../examples/extensions/packages/app-toolbox.serein-extension"
		))
		.unwrap();
		let output = invoke(
			&toolbox,
			&Invocation {
				action: "show".into(),
				app: Some(app),
				..Default::default()
			},
		)
		.expect("the real demo snapshot must fit App Toolbox's unchanged sandbox budget");
		assert!(!output.panel.is_empty() && output.effects.is_empty());
		let app = snapshot(
			&state,
			&messaging,
			&manifest(vec![Capability::LocalSettings]),
		)
		.unwrap();
		assert!(app.context.is_none() && app.channels.is_none() && app.timeline.is_none());
		assert!(app.settings.is_some());
		let member = model::Member {
			user: state.user.clone().unwrap(),
			roles: Vec::new(),
			nick: None,
			status: Some("online".into()),
			custom_status: None,
			activities: Vec::new(),
		};
		state.members = Some(model::MemberList {
			guild: Some(Id(10)),
			channel: Id(20),
			request: state.member_request,
			rows: vec![Some(member.clone()), Some(member)],
			total: 2,
			freshness: Freshness::Loading,
		});
		let loading = ChangeKey::capture(&state, &messaging);
		state.members.as_mut().unwrap().freshness = Freshness::Fresh;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&loading),
			Some(AppEventKind::Context)
		);
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert_eq!(app.members.as_ref().unwrap().items.len(), 1);
		assert_eq!(app.presence.as_ref().unwrap().items.len(), 1);
		state.freshness = Freshness::Unavailable;
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert!(app.context.as_ref().unwrap().channel.is_none());
		assert!(app.members.is_none() && app.presence.is_none() && app.timeline.is_none());
		assert!(
			app.channels
				.as_ref()
				.unwrap()
				.items
				.iter()
				.all(|channel| channel.id != "20")
		);
		state.gateway_connected = false;
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert!(!app.context.as_ref().unwrap().connected);
		assert!(
			app.channels.is_none()
				&& app.timeline.is_none()
				&& app.members.is_none()
				&& app.presence.is_none()
		);
		state.demo = false;
		state.auth = AuthState::Unauthenticated;
		assert!(snapshot(&state, &messaging, &caps).is_none());
	}
	#[test]
	fn extension_app_timeline_excludes_private_deleted_and_oversized_rows() {
		let mut state = test_support::demo_state();
		state.set_preserve_deleted_messages(true);
		let caps = manifest(vec![Capability::Timeline]);
		for (id, text, ephemeral) in [
			(2001, "private".into(), true),
			(2002, "x".repeat(4097), false),
			(2003, "removed".into(), false),
			(2004, "visible".into(), false),
		] {
			let mut message = test_support::message(id, Id(20));
			message.content = text;
			message.ephemeral = ephemeral;
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::Message(message),
			});
		}
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Delete {
				channel: Id(20),
				id: Id(2003),
			},
		});
		let app = snapshot(&state, &ui::MessagingUi::default(), &caps).unwrap();
		let timeline = app.timeline.as_ref().unwrap();
		assert!(timeline.messages.iter().any(|m| m.id == "2004"));
		assert!(
			timeline
				.messages
				.iter()
				.all(|m| !["2001", "2002", "2003"].contains(&m.id.as_str()))
		);
		assert!(timeline.truncated);
		state.freshness = Freshness::Unavailable;
		assert!(
			snapshot(&state, &ui::MessagingUi::default(), &caps)
				.unwrap()
				.timeline
				.is_none()
		);
	}
	#[test]
	fn extension_app_change_key_is_idle_until_meaningful_change() {
		let mut state = test_support::demo_state();
		let mut messaging = ui::MessagingUi::default();
		let first = ChangeKey::capture(&state, &messaging);
		state.revision += 1;
		assert_eq!(ChangeKey::capture(&state, &messaging).changed(&first), None);
		state.history_pending = !state.history_pending;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&first),
			Some(AppEventKind::Context)
		);
		state.history_pending = !state.history_pending;
		state.freshness = Freshness::Loading;
		let loading = ChangeKey::capture(&state, &messaging);
		state.freshness = Freshness::Fresh;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&loading),
			Some(AppEventKind::Context)
		);
		messaging.reading_preferences.zoom_percent = 110;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&first),
			Some(AppEventKind::Settings)
		);
		state.selected = None;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&first),
			Some(AppEventKind::Navigation)
		);
		state.gateway_connected = false;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&first),
			Some(AppEventKind::Connection)
		);
	}
}
