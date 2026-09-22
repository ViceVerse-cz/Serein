//! Explicitly confirmed extension proposals reuse the ordinary native UI paths.
use crate::{ExtensionContext, MessagingUi, design};
use client_core::{Command, State};
use extensions::{AppView, HostEffect, LocalSettingsSnapshot, NotificationSettingsSnapshot};
use model::Id;

pub(crate) struct ConfirmedEffect {
	pub plugin: String,
	pub context: ExtensionContext,
	pub effect: HostEffect,
}

pub(crate) fn effect_description(effect: &HostEffect) -> String {
	match effect {
		HostEffect::Navigate { channel_id } => format!("Open channel {channel_id}"),
		HostEffect::Home => "Open Friends / Home".into(),
		HostEffect::OpenView { view } => format!("Open {}", view_label(*view)),
		HostEffect::OpenProfile { user_id } => format!("Open profile for user {user_id}"),
		HostEffect::JumpToMessage {
			channel_id,
			message_id,
		} => {
			format!("Open message {message_id} in channel {channel_id}")
		}
		HostEffect::Search { query } => format!("Search this conversation for:\n{query}"),
		HostEffect::Notice { text } => format!("Show this local notice:\n{text}"),
		HostEffect::CopyText { text } => format!("Replace clipboard text with:\n{text}"),
		HostEffect::SetVoice { muted, deafened } => format!(
			"Set the current call to {} and {}",
			if *muted { "muted" } else { "unmuted" },
			if *deafened { "deafened" } else { "undeafened" },
		),
		HostEffect::LeaveVoice => "Leave the current voice call".into(),
		HostEffect::SetLocalSettings { settings } => {
			let mut lines = vec!["Change local reading settings:".into()];
			if let Some(value) = settings.zoom_percent {
				lines.push(format!("Zoom: {value}%"));
			}
			if let Some(value) = settings.sidebar_width {
				lines.push(format!("Sidebar width: {value}"));
			}
			for (label, value) in [
				("Show members", settings.show_members),
				("Animate GIFs", settings.animate_gifs),
				("Smooth scrolling", settings.smooth_scrolling),
				("Hide media links", settings.hide_media_links),
			] {
				if let Some(value) = value {
					lines.push(format!("{label}: {}", if value { "on" } else { "off" }));
				}
			}
			if let Some(value) = settings.scroll_speed_percent {
				lines.push(format!("Scroll speed: {value}%"));
			}
			lines.join("\n")
		}
		HostEffect::SetNotificationSettings { settings } => {
			let mut lines = vec!["Change local notification settings:".into()];
			if let Some(value) = settings.volume {
				lines.push(format!("Sound volume: {value}%"));
			}
			for (label, value) in [
				("New message sound", settings.new_message),
				("Current channel sound", settings.current_channel),
				("Incoming ring", settings.incoming_ring),
				("Outgoing ring", settings.outgoing_ring),
				("Disable sounds", settings.disable_sounds),
				("Unread badge", settings.unread_badge),
				("Mute sound", settings.mute),
				("Unmute sound", settings.unmute),
				("Deafen sound", settings.deafen),
				("Undeafen sound", settings.undeafen),
				("Camera on sound", settings.camera_on),
				("Screen share sound", settings.screen_share_on),
				("User join sound", settings.user_join),
				("User leave sound", settings.user_leave),
			] {
				if let Some(value) = value {
					lines.push(format!("{label}: {}", if value { "on" } else { "off" }));
				}
			}
			lines.join("\n")
		}
	}
}

pub(crate) fn effect_button(effect: &HostEffect) -> &'static str {
	match effect {
		HostEffect::CopyText { .. } => "Apply: Copy text",
		HostEffect::Notice { .. } => "Apply: Show notice",
		HostEffect::SetVoice { .. } => "Apply: Change call audio",
		HostEffect::LeaveVoice => "Apply: Leave call",
		HostEffect::SetLocalSettings { .. } => "Apply: Change settings",
		HostEffect::SetNotificationSettings { .. } => "Apply: Change notifications",
		HostEffect::Search { .. } => "Apply: Search",
		_ => "Apply: Open view",
	}
}

fn view_label(view: AppView) -> &'static str {
	match view {
		AppView::Friends => "Friends / Home",
		AppView::Search => "conversation search",
		AppView::Pins => "pinned messages",
		AppView::Members => "conversation members",
		AppView::Threads => "threads",
		AppView::Settings => "general settings",
		AppView::Account => "account settings",
		AppView::ProfileSettings => "profile settings",
		AppView::Appearance => "appearance settings",
		AppView::MessagingPermissions => "messaging permission settings",
		AppView::Notifications => "notification settings",
		AppView::Activity => "activity settings",
		AppView::Extensions => "extensions settings",
		AppView::Themes => "theme settings",
		AppView::VoiceSettings => "voice settings",
		AppView::Keybinds => "keyboard shortcut settings",
		AppView::Storage => "data and privacy settings",
		AppView::Updates => "update settings",
	}
}

fn active_channel(state: &State) -> Result<Id, &'static str> {
	state
		.selected
		.filter(|channel| {
			state.can_read_history(*channel)
				&& state.freshness != model::Freshness::Unavailable
				&& state
					.channel(*channel)
					.is_some_and(|channel| channel.supports_text())
		})
		.ok_or("This conversation is no longer accessible")
}

impl MessagingUi {
	pub fn extension_local_settings(&self) -> LocalSettingsSnapshot {
		let settings = self.reading_preferences;
		LocalSettingsSnapshot {
			zoom_percent: settings.zoom_percent,
			sidebar_width: settings.sidebar_width,
			show_members: settings.show_members,
			animate_gifs: settings.animate_gifs,
			hide_media_links: settings.hide_media_links,
			smooth_scrolling: Some(settings.smooth_scrolling),
			scroll_speed_percent: Some(settings.scroll_speed_percent),
		}
	}

	pub fn extension_notification_settings(&self) -> NotificationSettingsSnapshot {
		let settings = self.notification_options;
		NotificationSettingsSnapshot {
			new_message: settings.new_message,
			current_channel: settings.current_channel,
			incoming_ring: settings.incoming_ring,
			outgoing_ring: settings.outgoing_ring,
			disable_sounds: settings.disable_sounds,
			unread_badge: settings.unread_badge,
			mute: settings.mute,
			unmute: settings.unmute,
			deafen: settings.deafen,
			undeafen: settings.undeafen,
			camera_on: settings.camera_on,
			screen_share_on: settings.screen_share_on,
			user_join: settings.user_join,
			user_leave: settings.user_leave,
			volume: settings.volume,
		}
	}

	pub(crate) fn apply_extension_effect(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		confirmed: ConfirmedEffect,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		if !confirmed.context.is_current(state)
			|| state.user.is_none()
			|| !(state.demo || state.auth == client_core::auth::AuthState::Authenticated)
		{
			return Err("The account or conversation changed; run the extension again.".into());
		}
		if confirmed.context.channel.is_some_and(|channel| {
			!state.can_view(channel) || state.freshness == model::Freshness::Unavailable
		}) {
			return Err("This conversation is no longer accessible".into());
		}
		let entry = self
			.extensions
			.entries
			.iter()
			.find(|entry| {
				entry.manifest.id == confirmed.plugin && entry.enabled && !entry.cleanup_pending
			})
			.ok_or("The extension is no longer enabled")?;
		confirmed
			.effect
			.validate(&entry.manifest)
			.map_err(|error| error.to_string())?;
		let plugin_name = entry.manifest.name.clone();
		match confirmed.effect {
			HostEffect::Navigate { channel_id } => {
				self.extension_navigate(state, &channel_id, None, commands)?
			}
			HostEffect::JumpToMessage {
				channel_id,
				message_id,
			} => {
				self.extension_navigate(state, &channel_id, Some(&message_id), commands)?;
			}
			HostEffect::Home
			| HostEffect::OpenView {
				view: AppView::Friends,
			} => {
				if !self.server_settings.navigate_away(state) {
					return Err("Finish editing server settings first".into());
				}
				self.guild = None;
				state.open_home();
				self.search.open = false;
			}
			HostEffect::OpenView { view } => match view {
				AppView::Search => {
					let channel = active_channel(state)?;
					if !state.can_search() {
						return Err("Search is unavailable while disconnected".into());
					}
					let query = state
						.search
						.as_ref()
						.filter(|search| !search.pins && search.channel == channel)
						.map(|search| search.query.clone());
					self.search.open_extension(channel, false, query);
				}
				AppView::Pins => {
					let channel = active_channel(state)?;
					let command = state
						.request_pins()
						.ok_or("Pinned messages are unavailable")?;
					self.search.open_extension(channel, true, None);
					commands.push(command);
				}
				AppView::Members => {
					active_channel(state)?;
					self.search.open = false;
					self.reading_preferences.show_members = true;
					self.members_narrow_open = true;
					commands.extend(state.request_members());
				}
				AppView::Threads => {
					let channel = state.selected.ok_or("Choose a conversation first")?;
					let command = state
						.request_archives(channel, model::archives::Kind::Public, None)
						.ok_or("Threads are unavailable in this conversation")?;
					commands.push(command);
					self.guild = state.channel(channel).and_then(|channel| channel.guild);
					self.archives.focus = true;
					self.search.open = false;
				}
				_ => {
					self.open_extension_settings(view);
				}
			},
			HostEffect::OpenProfile { user_id } => {
				let id = Id(user_id.parse().map_err(|_| "Invalid user identifier")?);
				let user = state
					.user
					.as_ref()
					.filter(|user| user.id == id)
					.cloned()
					.or_else(|| state.friend(id).cloned())
					.or_else(|| {
						active_channel(state).ok().and_then(|channel| {
							crate::mentions::known_users(state, channel)
								.into_iter()
								.find(|user| user.id == id)
						})
					})
					.ok_or("This user is not available in the current session")?;
				self.profile.navigate(user);
			}
			HostEffect::Search { query } => {
				let channel = active_channel(state)?;
				let command = state
					.request_search(query.clone(), None)
					.ok_or("This search is unavailable or invalid")?;
				self.search.open_extension(channel, false, Some(query));
				commands.push(command);
			}
			HostEffect::Notice { text } => self
				.toasts
				.push(design::Level::Info, format!("{plugin_name}: {text}")),
			HostEffect::CopyText { text } => ctx.copy_text(text),
			HostEffect::SetVoice { muted, deafened } => {
				self.extension_voice_current(state, confirmed.context.voice_request)?;
				if !muted
					&& !deafened && !state.demo
					&& !state.can_speak(state.voice.active.as_ref().unwrap().channel)
				{
					return Err("Speaking is unavailable in this call".into());
				}
				let command = state
					.set_call_mute(muted, deafened)
					.ok_or("Call controls are unavailable")?;
				if let Some(call) = &state.voice.active {
					self.voice_muted = call.muted;
					self.voice_deafened = call.deafened;
				}
				commands.push(command);
			}
			HostEffect::LeaveVoice => {
				self.extension_voice_current(state, confirmed.context.voice_request)?;
				commands.extend(state.leave_call());
			}
			HostEffect::SetLocalSettings { settings } => {
				let mut value = self.reading_preferences;
				value.zoom_percent = settings.zoom_percent.unwrap_or(value.zoom_percent);
				value.sidebar_width = settings.sidebar_width.unwrap_or(value.sidebar_width);
				value.show_members = settings.show_members.unwrap_or(value.show_members);
				value.animate_gifs = settings.animate_gifs.unwrap_or(value.animate_gifs);
				value.hide_media_links =
					settings.hide_media_links.unwrap_or(value.hide_media_links);
				value.smooth_scrolling =
					settings.smooth_scrolling.unwrap_or(value.smooth_scrolling);
				value.scroll_speed_percent = settings
					.scroll_speed_percent
					.unwrap_or(value.scroll_speed_percent);
				self.apply_reading_preferences(ctx, value);
			}
			HostEffect::SetNotificationSettings { settings } => {
				let value = &mut self.notification_options;
				value.new_message = settings.new_message.unwrap_or(value.new_message);
				value.current_channel = settings.current_channel.unwrap_or(value.current_channel);
				value.incoming_ring = settings.incoming_ring.unwrap_or(value.incoming_ring);
				value.outgoing_ring = settings.outgoing_ring.unwrap_or(value.outgoing_ring);
				value.disable_sounds = settings.disable_sounds.unwrap_or(value.disable_sounds);
				value.unread_badge = settings.unread_badge.unwrap_or(value.unread_badge);
				value.mute = settings.mute.unwrap_or(value.mute);
				value.unmute = settings.unmute.unwrap_or(value.unmute);
				value.deafen = settings.deafen.unwrap_or(value.deafen);
				value.undeafen = settings.undeafen.unwrap_or(value.undeafen);
				value.camera_on = settings.camera_on.unwrap_or(value.camera_on);
				value.screen_share_on = settings.screen_share_on.unwrap_or(value.screen_share_on);
				value.user_join = settings.user_join.unwrap_or(value.user_join);
				value.user_leave = settings.user_leave.unwrap_or(value.user_leave);
				value.volume = settings.volume.unwrap_or(value.volume);
			}
		}
		ctx.request_repaint();
		Ok(())
	}

	fn extension_navigate(
		&mut self,
		state: &mut State,
		channel: &str,
		message: Option<&str>,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		let channel = Id(channel.parse().map_err(|_| "Invalid channel identifier")?);
		let guild = state
			.channel(channel)
			.filter(|_| state.can_read_history(channel))
			.ok_or("This channel is not readable in the current session")?
			.guild;
		let message = message
			.map(|value| value.parse().map(Id))
			.transpose()
			.map_err(|_| "Invalid message identifier")?;
		if !self.server_settings.navigate_away(state) {
			return Err("Finish editing server settings first".into());
		}
		commands.extend(state.open_chat_link(guild, channel, message)?);
		self.guild = guild;
		self.search.open = false;
		Ok(())
	}

	fn extension_voice_current(
		&self,
		state: &State,
		request: Option<(Id, u64)>,
	) -> Result<(), String> {
		if !(self.voice_available || state.demo)
			|| !state
				.voice
				.active
				.as_ref()
				.is_some_and(|call| Some((call.channel, call.request)) == request)
		{
			return Err(
				"The voice call changed or its controls are unavailable; run the extension again."
					.into(),
			);
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ExtensionEntry;
	use extensions::{Capability, ExtensionKind, Invocation, LocalSettingsPatch, Manifest, Output};

	fn view(capability: Capability) -> MessagingUi {
		let mut view = MessagingUi::default();
		view.extensions.entries.push(ExtensionEntry {
			description: String::new(),
			preview: None,
			theme_preview: None,
			cover_image: None,
			local_theme: false,
			manifest: Manifest {
				api_version: 1,
				id: "synthetic.app".into(),
				name: "Synthetic app tools".into(),
				version: "1.0.0".into(),
				author: "Offline fixture".into(),
				license: "MIT".into(),
				source: "https://example.com/source".into(),
				kind: ExtensionKind::Plugin,
				capabilities: vec![capability],
				actions: vec![],
			},
			reviewed: false,
			sha256: "a".repeat(64),
			download_bytes: 4096,
			enabled: true,
			cleanup_pending: false,
			update_available: false,
			update_manifest: None,
		});
		view
	}

	fn proposal(state: &State, effect: HostEffect) -> ConfirmedEffect {
		ConfirmedEffect {
			plugin: "synthetic.app".into(),
			context: ExtensionContext::capture(state, false),
			effect,
		}
	}

	fn frame(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
		events: Vec<egui::Event>,
	) -> Vec<(String, egui::Rect)> {
		fn labels(shape: &egui::Shape, output: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => {
					output.push((text.galley.job.text.clone(), text.visual_bounding_rect()))
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						labels(shape, output);
					}
				}
				_ => {}
			}
		}
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 700.0),
				)),
				events,
				focused: true,
				..Default::default()
			},
			|ui| {
				if let Some(effect) =
					view.extensions
						.show_result(ui.ctx(), state, &mut Vec::new(), false)
				{
					view.apply_extension_effect(ui.ctx(), state, effect, &mut Vec::new())
						.unwrap();
				}
			},
		);
		let mut text = Vec::new();
		for shape in &output.shapes {
			labels(&shape.shape, &mut text);
		}
		output.drop_without_applying_deltas();
		text
	}

	#[test]
	fn host_action_waits_for_apply_and_close_discards_it() {
		for approve in [false, true] {
			let ctx = egui::Context::default();
			let mut state = test_support::demo_state();
			let mut view = view(Capability::LocalSettings);
			let before = view.reading_preferences;
			view.extensions.present_output(
				"synthetic.app".into(),
				Invocation::default(),
				ExtensionContext::capture(&state, false),
				Output {
					effects: vec![HostEffect::SetLocalSettings {
						settings: LocalSettingsPatch {
							sidebar_width: Some(300),
							..Default::default()
						},
					}],
					..Default::default()
				},
				&state,
			);
			frame(&ctx, &mut view, &mut state, vec![]);
			let text = frame(&ctx, &mut view, &mut state, vec![]);
			assert_eq!(view.reading_preferences, before);
			assert!(
				text.iter()
					.any(|(text, _)| text.contains("Sidebar width: 300"))
			);
			let button = if approve {
				"Apply: Change settings"
			} else {
				"Close"
			};
			let pos = text
				.iter()
				.find(|(text, _)| text == button)
				.unwrap()
				.1
				.center();
			for pressed in [true, false] {
				frame(
					&ctx,
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(
				view.reading_preferences.sidebar_width,
				if approve { 300 } else { before.sidebar_width }
			);
			assert!(!view.extensions.has_result());
		}
	}

	#[test]
	fn apply_rechecks_context_extension_and_grant() {
		for failure in 0..7 {
			let ctx = egui::Context::default();
			let mut state = test_support::demo_state();
			let mut view = view(Capability::LocalSettings);
			let effect = proposal(
				&state,
				HostEffect::SetLocalSettings {
					settings: LocalSettingsPatch {
						sidebar_width: Some(300),
						..Default::default()
					},
				},
			);
			let before = view.reading_preferences;
			match failure {
				0 => {
					state.selected = Some(Id(21));
				}
				1 => {
					state.generation += 1;
				}
				2 => {
					view.extensions.entries[0].enabled = false;
				}
				3 => {
					view.extensions.entries[0].cleanup_pending = true;
				}
				4 => {
					view.extensions.entries[0].manifest.capabilities.clear();
				}
				5 => {
					state
						.channels
						.retain(|channel| Some(channel.id) != state.selected);
				}
				_ => {
					state.freshness = model::Freshness::Unavailable;
				}
			}
			let mut commands = Vec::new();
			assert!(
				view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
					.is_err()
			);
			assert_eq!(view.reading_preferences, before);
			assert!(commands.is_empty());
		}
	}

	#[test]
	fn navigation_uses_only_known_readable_channels_and_home_clears_selection() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = view(Capability::Navigation);
		let mut commands = Vec::new();
		let before = state.selected;
		let effect = proposal(
			&state,
			HostEffect::Navigate {
				channel_id: "999999999".into(),
			},
		);
		assert!(
			view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
				.is_err()
		);
		assert_eq!(state.selected, before);
		assert!(commands.is_empty());
		let effect = proposal(
			&state,
			HostEffect::Navigate {
				channel_id: "21".into(),
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert_eq!(state.selected, Some(Id(21)));
		assert_eq!(view.guild, Some(Id(10)));
		view.search.open = true;
		let effect = proposal(&state, HostEffect::Home);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert_eq!(state.selected, None);
		assert_eq!(view.guild, None);
		assert!(!view.search.open);
	}

	#[test]
	fn voice_proposals_cannot_affect_a_replacement_call() {
		for leave in [false, true] {
			for changed_channel in [false, true] {
				let ctx = egui::Context::default();
				let mut state = test_support::call_demo_state();
				let mut view = view(Capability::VoiceControl);
				let effect = proposal(
					&state,
					if leave {
						HostEffect::LeaveVoice
					} else {
						HostEffect::SetVoice {
							muted: true,
							deafened: true,
						}
					},
				);
				let call = state.voice.active.as_mut().unwrap();
				if changed_channel {
					call.channel = Id(25);
				} else {
					call.request += 1;
				}
				let original = (call.channel, call.request, call.muted, call.deafened);
				let mut commands = Vec::new();
				assert!(
					view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
						.is_err()
				);
				let call = state.voice.active.as_ref().unwrap();
				assert_eq!(
					(call.channel, call.request, call.muted, call.deafened),
					original
				);
				assert!(commands.is_empty());
			}
		}
	}

	#[test]
	fn search_and_threads_follow_the_current_channel_through_ui_sync() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = view(Capability::Navigation);
		let mut commands = Vec::new();
		let effect = proposal(
			&state,
			HostEffect::Search {
				query: "hello".into(),
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		view.search.sync(&ctx, &mut state, &mut commands);
		assert!(view.search.open);
		assert_eq!(state.search.as_ref().unwrap().query, "hello");
		assert!(matches!(
			commands.as_slice(),
			[Command::Search {
				channel: Id(20),
				..
			}]
		));
		commands.clear();
		let effect = proposal(
			&state,
			HostEffect::OpenView {
				view: AppView::Threads,
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		view.search.sync(&ctx, &mut state, &mut commands);
		assert!(!view.search.open);
		assert_eq!(view.guild, Some(Id(10)));
		assert_eq!(state.archives.as_ref().unwrap().parent, Id(20));
		assert!(matches!(
			commands.as_slice(),
			[Command::Archives { parent: Id(20), .. }]
		));
		let user = state.user.as_ref().unwrap().clone();
		let effect = proposal(
			&state,
			HostEffect::OpenProfile {
				user_id: user.id.0.to_string(),
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert_eq!(view.profile.open_user().map(|user| user.id), Some(user.id));
	}

	#[test]
	fn current_call_controls_reuse_bounded_voice_commands() {
		let ctx = egui::Context::default();
		let mut state = test_support::call_demo_state();
		let mut view = view(Capability::VoiceControl);
		let mut commands = Vec::new();
		let effect = proposal(
			&state,
			HostEffect::SetVoice {
				muted: true,
				deafened: true,
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert!(matches!(
			commands.pop(),
			Some(Command::Voice(client_core::voice::Command::SetMute {
				mute: true,
				deaf: true,
				..
			}))
		));
		let effect = proposal(&state, HostEffect::LeaveVoice);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert!(matches!(
			commands.pop(),
			Some(Command::Voice(client_core::voice::Command::Leave { .. }))
		));
		assert!(state.voice.active.is_none());
	}

	#[test]
	fn preference_patch_preserves_other_values_and_rejects_invalid_range() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = view(Capability::LocalSettings);
		view.reading_preferences.confirm_external_links = false;
		let before = view.reading_preferences;
		let effect = proposal(
			&state,
			HostEffect::SetLocalSettings {
				settings: LocalSettingsPatch {
					zoom_percent: Some(120),
					hide_media_links: Some(!before.hide_media_links),
					smooth_scrolling: Some(false),
					scroll_speed_percent: Some(250),
					..Default::default()
				},
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut Vec::new())
			.unwrap();
		assert_eq!(
			view.reading_preferences,
			model::ReadingPreferences {
				zoom_percent: 120,
				hide_media_links: !before.hide_media_links,
				smooth_scrolling: false,
				scroll_speed_percent: 250,
				..before
			}
		);
		assert_eq!(view.extension_local_settings().zoom_percent, 120);
		let applied = view.reading_preferences;
		for settings in [
			LocalSettingsPatch {
				zoom_percent: Some(151),
				..Default::default()
			},
			LocalSettingsPatch {
				scroll_speed_percent: Some(301),
				..Default::default()
			},
			LocalSettingsPatch::default(),
		] {
			let effect = proposal(&state, HostEffect::SetLocalSettings { settings });
			assert!(
				view.apply_extension_effect(&ctx, &mut state, effect, &mut Vec::new())
					.is_err()
			);
			assert_eq!(view.reading_preferences, applied);
		}
	}
	#[test]
	fn notification_patch_revalidates_grant_and_preserves_apply_time_values() {
		use extensions::NotificationSettingsPatch;
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = view(Capability::NotificationSettings);
		let effect = proposal(
			&state,
			HostEffect::SetNotificationSettings {
				settings: NotificationSettingsPatch {
					volume: Some(25),
					current_channel: Some(true),
					..Default::default()
				},
			},
		);
		view.notification_options.user_join = false;
		let before = view.notification_options;
		let mut commands = Vec::new();
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert_eq!(
			view.notification_options,
			model::notification_preferences::Device {
				volume: 25,
				current_channel: true,
				..before
			}
		);
		assert!(commands.is_empty());
		assert_eq!(view.extension_notification_settings().volume, 25);
		let applied = view.notification_options;
		for settings in [
			NotificationSettingsPatch {
				volume: Some(101),
				disable_sounds: Some(true),
				..Default::default()
			},
			NotificationSettingsPatch::default(),
		] {
			let effect = proposal(&state, HostEffect::SetNotificationSettings { settings });
			assert!(
				view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
					.is_err()
			);
			assert_eq!(view.notification_options, applied);
		}
		let effect = proposal(
			&state,
			HostEffect::SetNotificationSettings {
				settings: NotificationSettingsPatch {
					volume: Some(0),
					..Default::default()
				},
			},
		);
		view.extensions.entries[0].manifest.capabilities.clear();
		assert!(
			view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
				.is_err()
		);
		assert_eq!(view.notification_options, applied);
	}
}
