use crate::MessagingUi;
use client_core::{Command, State};
use extensions::AppAction;
use model::{
	Id,
	voice_settings::{InputProfile, NoiseSuppression},
};

fn id(value: &str) -> Result<Id, String> {
	value
		.parse::<u64>()
		.ok()
		.filter(|value| *value != 0)
		.map(Id)
		.ok_or_else(|| "Invalid identifier".into())
}

fn queue(command: Option<Command>, commands: &mut Vec<Command>) -> Result<(), String> {
	commands.push(command.ok_or("This action is no longer available; refresh the app data")?);
	Ok(())
}

impl MessagingUi {
	pub fn extension_audio_settings(&self) -> extensions::AudioSettingsSnapshot {
		let effective = self.voice_processing.effective();
		extensions::AudioSettingsSnapshot {
			input_percent: self.voice_gain.input_percent,
			output_percent: self.voice_gain.output_percent,
			push_to_talk: self.voice_push_to_talk,
			input_profile: match self.voice_processing.profile {
				InputProfile::VoiceIsolation => "voice_isolation",
				InputProfile::Studio => "studio",
				InputProfile::Custom => "custom",
			}
			.into(),
			suppression: match effective.suppression {
				NoiseSuppression::Off => "off",
				NoiseSuppression::RnNoise => "rnnoise",
				NoiseSuppression::WebRtc => "webrtc",
			}
			.into(),
			suppression_level: effective.suppression_level,
			echo_cancellation: effective.echo_cancellation,
			automatic_gain: effective.automatic_gain,
			sensitivity_db: effective.sensitivity_db,
		}
	}

	pub fn extension_own_presence(&self) -> extensions::OwnPresenceSnapshot {
		extensions::OwnPresenceSnapshot {
			status: self.own_presence.status.wire().into(),
			custom_status: self.own_presence.custom_status.clone(),
			expires_at_ms: self.own_presence_expires,
			share_game_activity: self.share_game_activity,
		}
	}

	pub(crate) fn apply_extension_account_action(
		&mut self,
		state: &mut State,
		action: AppAction,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		action.validate().map_err(|error| error.to_string())?;
		match action {
			AppAction::OpenFriendDm { user_id } => {
				let user = id(&user_id)?;
				if state.friend(user).is_none() || !self.server_settings.navigate_away(state) {
					return Err(
						"The friend is unavailable or server settings need attention".into(),
					);
				}
				let command = state.open_friend_dm(user);
				let selected = state.selected.and_then(|channel| state.channel(channel));
				if command.is_none()
					&& !selected.is_some_and(|channel| {
						channel.guild.is_none()
							&& channel.kind == 1 && channel.recipients.len() == 1
							&& channel.recipients[0].id == user
					}) {
					return Err("Opening this direct conversation is no longer available".into());
				}
				commands.extend(command);
				self.guild = None;
				self.search.open = false;
			}
			AppAction::SetFriendNickname { user_id, text } => {
				queue(state.set_friend_nickname(id(&user_id)?, text), commands)?
			}
			AppAction::SetUserNote { user_id, text } => {
				queue(state.set_user_note(id(&user_id)?, text), commands)?
			}
			AppAction::AddFriend { username } => queue(state.add_friend(&username), commands)?,
			AppAction::RemoveFriend { user_id } => {
				queue(state.remove_friend(id(&user_id)?), commands)?
			}
			AppAction::ResolveFriendRequest { user_id, accept } => queue(
				state.resolve_friend_request(id(&user_id)?, accept),
				commands,
			)?,
			AppAction::SetUserBlocked { user_id, blocked } => {
				let user = id(&user_id)?;
				let known = state.friend(user).is_some()
					|| state.restricted_user(user).is_some()
					|| state
						.pending_friends()
						.any(|(person, _, _)| person.id == user)
					|| state
						.selected
						.filter(|channel| state.can_read_history(*channel))
						.is_some_and(|channel| {
							crate::mentions::known_users(state, channel)
								.iter()
								.any(|person| person.id == user)
						});
				if !known {
					return Err("This user is not available in the current session".into());
				}
				queue(state.set_user_blocked(user, blocked), commands)?;
			}
			AppAction::SetOwnProfile { profile } => {
				let changes = model::ProfileEdit {
					global_name: if profile.clear_global_name {
						Some(None)
					} else {
						profile.global_name.map(Some)
					},
					bio: profile.bio,
					pronouns: profile.pronouns,
					accent_color: if profile.clear_accent_color {
						Some(None)
					} else {
						profile.accent_color.map(Some)
					},
					avatar: None,
				};
				queue(state.save_own_profile(changes), commands)?;
			}
			AppAction::SetOwnPresence { presence } => {
				let mut value = self.own_presence.clone();
				if let Some(status) = presence.status {
					value.status =
						model::PresenceStatus::parse(&status).ok_or("Invalid presence status")?;
				}
				if let Some(text) = presence.custom_status {
					value.custom_status = text;
				}
				if let Some(seconds) = presence.clear_after_seconds {
					value.expires_at_ms = if seconds == 0 {
						None
					} else {
						let now = std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.map_err(|_| "System clock is unavailable")?
							.as_millis();
						Some(
							u64::try_from(now)
								.map_err(|_| "System clock is out of range")?
								.checked_add(u64::from(seconds) * 1000)
								.ok_or("Expiry is out of range")?,
						)
					};
				}
				if value.custom_status.is_empty() {
					value.expires_at_ms = None;
				}
				if !value.valid() {
					return Err("Invalid presence preferences".into());
				}
				if value != self.own_presence {
					self.adopt_account_presence(value);
					self.own_presence_changed = true;
				}
			}
			AppAction::SetActivitySharing { enabled } => self.share_game_activity = enabled,
			AppAction::SetAudioSettings { settings } => {
				let mut processing = self.voice_processing;
				if let Some(profile) = settings.input_profile.as_deref() {
					processing.profile = match profile {
						"voice_isolation" => InputProfile::VoiceIsolation,
						"studio" => InputProfile::Studio,
						"custom" => InputProfile::Custom,
						_ => return Err("Invalid microphone profile".into()),
					};
				}
				if settings.suppression.is_some()
					|| settings.suppression_level.is_some()
					|| settings.echo_cancellation.is_some()
					|| settings.automatic_gain.is_some()
					|| settings.sensitivity_db.is_some()
					|| settings.open_microphone
				{
					let custom = processing.edit();
					if let Some(suppression) = settings.suppression.as_deref() {
						custom.suppression = match suppression {
							"off" => NoiseSuppression::Off,
							"rnnoise" => NoiseSuppression::RnNoise,
							"webrtc" => NoiseSuppression::WebRtc,
							_ => return Err("Invalid suppression mode".into()),
						};
					}
					custom.suppression_level = settings
						.suppression_level
						.unwrap_or(custom.suppression_level);
					custom.echo_cancellation = settings
						.echo_cancellation
						.unwrap_or(custom.echo_cancellation);
					custom.automatic_gain =
						settings.automatic_gain.unwrap_or(custom.automatic_gain);
					if settings.open_microphone {
						custom.sensitivity_db = None;
					} else if let Some(db) = settings.sensitivity_db {
						custom.sensitivity_db = Some(db);
					}
				}
				if !processing.custom.is_valid() {
					return Err("Invalid microphone processing settings".into());
				}
				self.voice_processing = processing;
				self.voice_gain.input_percent = settings
					.input_percent
					.unwrap_or(self.voice_gain.input_percent);
				self.voice_gain.output_percent = settings
					.output_percent
					.unwrap_or(self.voice_gain.output_percent);
				self.voice_push_to_talk = settings.push_to_talk.unwrap_or(self.voice_push_to_talk);
			}
			AppAction::SetParticipantAudio {
				user_id,
				volume_percent,
				muted,
			} => {
				let user = id(&user_id)?;
				let call = state
					.voice
					.active
					.as_ref()
					.ok_or("There is no active voice call")?;
				if state.user.as_ref().is_some_and(|own| own.id == user)
					|| !call
						.participants
						.iter()
						.any(|participant| participant.user == user)
				{
					return Err("This remote participant is no longer in the call".into());
				}
				if muted == Some(true)
					&& !self.voice_user_locally_muted(user)
					&& self.voice_user_muted.len() >= 64
				{
					return Err("The local participant mute limit has been reached".into());
				}
				if let Some(volume) = volume_percent {
					self.set_voice_user_volume(user, volume);
				}
				if let Some(muted) = muted {
					self.set_voice_user_locally_muted(user, muted);
				}
			}
			AppAction::SetStreamAudio {
				volume_percent,
				muted,
			} => {
				if state
					.voice
					.active
					.as_ref()
					.is_none_or(|call| call.watching.is_none())
				{
					return Err("There is no watched stream".into());
				}
				if let Some(volume) = volume_percent {
					self.voice_stream_volume = Some(volume);
				}
				if let Some(muted) = muted {
					self.voice_stream_muted = muted;
				}
			}
			AppAction::OpenAttachmentPicker { channel_id } => {
				let channel = id(&channel_id)?;
				if state.selected != Some(channel)
					|| !state.can_view(channel)
					|| state
						.channel(channel)
						.is_none_or(|value| !value.supports_text())
				{
					return Err("The selected conversation is no longer available".into());
				}
				self.attach_requested = true;
			}
			AppAction::SelectAudioDevices {
				input_id,
				output_id,
			} => {
				if let Some(value) = input_id {
					if !self.voice_inputs.iter().any(|(id, _)| id == &value) {
						return Err("The microphone is no longer available".into());
					}
					self.voice_input = Some(value);
				}
				if let Some(value) = output_id {
					if !self.voice_outputs.iter().any(|(id, _)| id == &value) {
						return Err("The speaker device is no longer available".into());
					}
					self.voice_output = Some(value);
				}
			}
			AppAction::RefreshMediaDevices => {
				self.voice_refresh_devices = true;
				self.voice_refresh_cameras = true;
			}
			AppAction::SelectCameraDevice { device_id } => {
				if let Some(value) = device_id.as_ref()
					&& !self.voice_cameras.iter().any(|(id, _)| id == value)
				{
					return Err("The camera is no longer available".into());
				}
				self.voice_camera_device = device_id;
			}
			AppAction::OpenScreenSharePicker => {
				if state.voice.active.is_none() || self.screen.busy {
					return Err("Screen sharing is unavailable in the current call".into());
				}
				self.screen.launch(state);
			}
			AppAction::StopScreenShare => {
				if !self.screen.busy {
					return Err("There is no active screen share".into());
				}
				self.screen.request = Some(crate::screen::Request::Stop);
			}
			AppAction::WatchStream { user_id } => {
				state
					.watch_stream(id(&user_id)?)
					.ok_or("This stream is no longer available")?;
			}
			AppAction::StopWatching => {
				if state
					.voice
					.active
					.as_ref()
					.is_none_or(|call| call.watching.is_none())
				{
					return Err("There is no watched stream".into());
				}
				state.stop_watching();
			}
			AppAction::DeclineCall { channel_id } => {
				if state.voice.incoming != Some(id(&channel_id)?) {
					return Err("This incoming call is no longer available".into());
				}
				queue(state.decline_call(), commands)?;
			}
			other => return self.apply_extension_admin_action(state, other, commands),
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use extensions::{AudioSettingsPatch, OwnPresencePatch, OwnProfilePatch};

	#[test]
	fn audio_patch_uses_visible_preset_and_rejects_invalid_patch_atomically() {
		let mut state = test_support::demo_state();
		let mut view = MessagingUi::default();
		view.voice_processing.profile = InputProfile::Studio;
		view.voice_processing.custom.sensitivity_db = Some(-20);
		let mut commands = Vec::new();
		view.apply_extension_account_action(
			&mut state,
			AppAction::SetAudioSettings {
				settings: AudioSettingsPatch {
					input_percent: Some(140),
					sensitivity_db: Some(-60),
					..Default::default()
				},
			},
			&mut commands,
		)
		.unwrap();
		assert_eq!(view.voice_gain.input_percent, 140);
		assert_eq!(view.voice_gain.output_percent, 100);
		assert_eq!(view.voice_processing.profile, InputProfile::Custom);
		assert_eq!(
			view.voice_processing.custom.suppression,
			NoiseSuppression::Off
		);
		assert_eq!(view.voice_processing.custom.sensitivity_db, Some(-60));
		let previous = (view.voice_gain, view.voice_processing);
		assert!(
			view.apply_extension_account_action(
				&mut state,
				AppAction::SetAudioSettings {
					settings: AudioSettingsPatch {
						input_percent: Some(201),
						open_microphone: true,
						..Default::default()
					}
				},
				&mut commands
			)
			.is_err()
		);
		assert_eq!((view.voice_gain, view.voice_processing), previous);
		assert!(commands.is_empty());
		assert!(!view.voice_preview_requested);
	}

	#[test]
	fn presence_patch_preserves_omitted_values_and_clears_expiry_with_text() {
		let mut state = test_support::demo_state();
		let mut view = MessagingUi::default();
		view.own_presence.custom_status = "Loaded status".into();
		view.own_presence.expires_at_ms = Some(1234);
		view.own_presence_expires = Some(1234);
		let mut commands = Vec::new();
		view.apply_extension_account_action(
			&mut state,
			AppAction::SetOwnPresence {
				presence: OwnPresencePatch {
					status: Some("dnd".into()),
					..Default::default()
				},
			},
			&mut commands,
		)
		.unwrap();
		assert_eq!(view.own_presence.custom_status, "Loaded status");
		assert_eq!(view.own_presence_expires, Some(1234));
		assert!(view.own_presence_changed);
		view.apply_extension_account_action(
			&mut state,
			AppAction::SetOwnPresence {
				presence: OwnPresencePatch {
					custom_status: Some(String::new()),
					..Default::default()
				},
			},
			&mut commands,
		)
		.unwrap();
		assert_eq!(
			view.own_presence.status,
			model::PresenceStatus::DoNotDisturb
		);
		assert_eq!(view.own_presence.expires_at_ms, None);
		assert_eq!(view.own_presence_expires, None);
		assert!(commands.is_empty());
	}

	#[test]
	fn participant_audio_preserves_muted_gain_and_rejects_absent_targets() {
		let mut state = test_support::call_demo_state();
		let mut view = MessagingUi::default();
		let own = state.user.as_ref().unwrap().id;
		let user = state
			.voice
			.active
			.as_ref()
			.unwrap()
			.participants
			.iter()
			.find(|p| p.user != own)
			.unwrap()
			.user;
		let mut commands = Vec::new();
		view.apply_extension_account_action(
			&mut state,
			AppAction::SetParticipantAudio {
				user_id: user.to_string(),
				volume_percent: Some(150),
				muted: Some(true),
			},
			&mut commands,
		)
		.unwrap();
		assert!(view.voice_user_volumes().contains(&(user.0, 0)));
		assert!(view.voice_user_volume_overrides().contains(&(user.0, 150)));
		view.apply_extension_account_action(
			&mut state,
			AppAction::SetParticipantAudio {
				user_id: user.to_string(),
				volume_percent: None,
				muted: Some(false),
			},
			&mut commands,
		)
		.unwrap();
		assert!(view.voice_user_volumes().contains(&(user.0, 150)));
		for user_id in [own.to_string(), "999999999".into()] {
			assert!(
				view.apply_extension_account_action(
					&mut state,
					AppAction::SetParticipantAudio {
						user_id,
						volume_percent: Some(0),
						muted: None,
					},
					&mut commands
				)
				.is_err()
			);
		}
		assert!(
			view.apply_extension_account_action(
				&mut state,
				AppAction::SetStreamAudio {
					volume_percent: Some(0),
					muted: None,
				},
				&mut commands
			)
			.is_err()
		);
		assert!(commands.is_empty());
	}

	#[test]
	fn account_writes_reuse_state_admission_and_do_not_fetch_missing_profiles() {
		let mut state = test_support::friends_demo_state();
		let mut view = MessagingUi::default();
		let mut commands = Vec::new();
		assert!(
			view.apply_extension_account_action(
				&mut state,
				AppAction::SetOwnProfile {
					profile: OwnProfilePatch {
						bio: Some("Synthetic".into()),
						..Default::default()
					},
				},
				&mut commands
			)
			.is_err()
		);
		assert!(!state.own_profile.loading);
		assert!(commands.is_empty());
		view.apply_extension_account_action(
			&mut state,
			AppAction::AddFriend {
				username: "new.synthetic".into(),
			},
			&mut commands,
		)
		.unwrap();
		assert!(
			matches!(&commands[..], [Command::UserAction { action: client_core::user_actions::Action::AddFriend { username }, .. }] if username == "new.synthetic")
		);
		assert!(
			view.apply_extension_account_action(
				&mut state,
				AppAction::AddFriend {
					username: "another.synthetic".into()
				},
				&mut commands
			)
			.is_err()
		);
		assert_eq!(commands.len(), 1);
	}

	#[test]
	fn media_actions_only_select_current_host_devices() {
		let mut state = test_support::demo_state();
		let mut view = MessagingUi {
			voice_inputs: vec![("mic-1".into(), "Synthetic microphone".into())],
			voice_outputs: vec![("out-1".into(), "Synthetic speakers".into())],
			voice_cameras: vec![("cam-1".into(), "Synthetic camera".into())],
			..Default::default()
		};
		let mut commands = Vec::new();
		view.apply_extension_account_action(
			&mut state,
			AppAction::SelectAudioDevices {
				input_id: Some("mic-1".into()),
				output_id: Some("out-1".into()),
			},
			&mut commands,
		)
		.unwrap();
		assert_eq!(view.voice_input.as_deref(), Some("mic-1"));
		assert_eq!(view.voice_output.as_deref(), Some("out-1"));
		assert!(
			view.apply_extension_account_action(
				&mut state,
				AppAction::SelectCameraDevice {
					device_id: Some("missing".into()),
				},
				&mut commands,
			)
			.is_err()
		);
		assert!(commands.is_empty());
	}
}
