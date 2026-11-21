//! Native, service-confirmed Profile Privacy and Content Filters.
use crate::{
	MessagingUi, design,
	i18n::{translate as tr, translate_args},
};
use client_core::{Command, State, auth::Failure};
use model::{
	messaging_permissions::Change,
	privacy_preferences::{
		Audience, Change as PrivacyChange, ContentKind, Keyword, ProfileVisibility, Redaction,
	},
};

impl MessagingUi {
	pub(super) fn account_preferences_header(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let nav = &mut self.settings.messaging_permissions;
		if nav.generation != state.generation {
			*nav = crate::messaging_permissions::Navigation {
				generation: state.generation,
				..Default::default()
			};
		}
		if state.messaging_permissions.snapshot.is_none()
			&& !state.messaging_permissions.pending
			&& state.messaging_permissions.error.is_none()
		{
			nav.requested = false;
		}
		if !nav.requested && !state.messaging_permissions.pending {
			if let Some(command) = state.request_messaging_permissions() {
				commands.push(command);
			}
			nav.requested = true;
		}
		if state.messaging_permissions.pending {
			ui.horizontal(|ui| {
				ui.add(egui::Spinner::new().size(14.0));
				ui.label(tr("privacy-prefs-updating"));
			});
		}
		if let Some(error) = state.messaging_permissions.error {
			design::notice(ui, design::Level::Error, &preference_error(error));
			if !state.messaging_permissions.pending
				&& design::text_action(ui, &tr("privacy-prefs-reload")).clicked()
				&& let Some(command) = state.request_messaging_permissions()
			{
				commands.push(command);
			}
		}
		if let Some(draft) = state
			.messaging_permissions
			.draft
			.as_ref()
			.filter(|_| !state.messaging_permissions.pending)
		{
			let description = match draft {
				Change::Privacy(change) => change_description(*change),
				_ => tr("privacy-prefs-previous"),
			};
			design::notice(
				ui,
				design::Level::Warning,
				&translate_args("privacy-prefs-unsaved", &[("description", &description)]),
			);
			let draft = draft.clone();
			ui.horizontal_wrapped(|ui| {
				if ui
					.add_enabled(
						!state.messaging_permissions.reload_required,
						egui::Button::new(tr("privacy-prefs-apply")),
					)
					.clicked() && let Some(command) = state.update_messaging_permissions(draft)
				{
					commands.push(command);
				}
				if design::text_action(ui, &tr("privacy-prefs-discard")).clicked() {
					state.messaging_permissions.draft = None;
				}
			});
		}
	}
	pub(super) fn privacy_settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		content_filters: bool,
	) {
		self.account_preferences_header(ui, state, commands);
		let Some(snapshot) = state.messaging_permissions.snapshot.as_ref() else {
			return;
		};
		let settings = &snapshot.privacy;
		let enabled =
			!state.messaging_permissions.pending && !state.messaging_permissions.reload_required;
		let mut change = None;
		ui.add_enabled_ui(enabled, |ui| {
			if content_filters {
				for (kind, heading, description) in [
					(
						ContentKind::Sensitive,
						"privacy-sensitive",
						"privacy-sensitive-description",
					),
					(
						ContentKind::Gore,
						"privacy-graphic",
						"privacy-graphic-description",
					),
					(
						ContentKind::SelfHarm,
						"privacy-self-harm",
						"privacy-self-harm-description",
					),
				] {
					design::group(ui, &tr(heading), |ui| {
						design::hint(ui, &tr(description));
						for (audience, label) in [
							(Audience::Servers, "privacy-in-servers"),
							(Audience::Friends, "privacy-friends-dms"),
							(Audience::Others, "privacy-other-dms"),
						] {
							let current = settings.content(kind)[audience.index()];
							design::row(ui, &tr(label), None, |ui| {
								let mut selected = current;
								egui::ComboBox::from_id_salt((
									"content-redaction",
									kind.field(),
									audience.index(),
								))
								.width(150.0)
								.selected_text(redaction_label(current))
								.show_ui(ui, |ui| {
									for level in
										[Redaction::Show, Redaction::Blur, Redaction::Block]
									{
										ui.selectable_value(
											&mut selected,
											level,
											redaction_label(level),
										);
									}
								});
								if selected != current {
									change = Some(PrivacyChange::Content {
										kind,
										audience,
										level: selected,
									});
								}
							});
						}
					});
				}
				design::group(ui, &tr("privacy-filtered-language"), |ui| {
					for (keyword, label) in [
						(Keyword::Profanity, "automod-profanity"),
						(Keyword::SexualContent, "privacy-sexual-language"),
						(Keyword::Slurs, "automod-slurs"),
					] {
						if let Some(enabled) =
							optional_toggle(ui, label, settings.keywords[keyword.index()])
						{
							change = Some(PrivacyChange::Keyword { keyword, enabled });
						}
					}
				});
			} else {
				design::group(ui, &tr("privacy-profile-title"), |ui| {
					design::hint(ui, &tr("privacy-profile-description"));
					if matches!(
						settings.profile_visibility,
						ProfileVisibility::Default | ProfileVisibility::Unknown(_)
					) {
						ui.weak(translate_args(
							"privacy-current-audience",
							&[("audience", &visibility_label(settings.profile_visibility))],
						));
					}
					for (value, hint) in [
						(ProfileVisibility::Friends, "privacy-friends-description"),
						(
							ProfileVisibility::FriendsAndSmallServers,
							"privacy-small-servers-description",
						),
						(
							ProfileVisibility::FriendsAndAllServers,
							"privacy-all-servers-description",
						),
					] {
						if design::radio_row(
							ui,
							settings.profile_visibility == value,
							&visibility_label(value),
							Some(&tr(hint)),
						)
						.clicked() && settings.profile_visibility != value
						{
							change = Some(PrivacyChange::ProfileVisibility(value));
						}
					}
				});
				design::group(ui, &tr("privacy-preferences-heading"), |ui| {
					for (label, current, make) in [
						(
							"privacy-conversation-warnings",
							settings.conversation_warnings,
							PrivacyChange::ConversationWarnings as fn(bool) -> PrivacyChange,
						),
						(
							"privacy-local-time",
							settings.show_local_time,
							PrivacyChange::ShowLocalTime,
						),
						(
							"privacy-detect-platform-accounts",
							settings.detect_platform_accounts,
							PrivacyChange::DetectPlatformAccounts,
						),
						(
							"privacy-non-spam-training",
							settings.non_spam_training,
							PrivacyChange::NonSpamTraining,
						),
					] {
						if let Some(value) = optional_toggle(ui, label, current) {
							change = Some(make(value));
						}
					}
				});
			}
		});
		if let Some(change) = change
			&& let Some(command) = state.update_messaging_permissions(Change::Privacy(change))
		{
			commands.push(command);
		}
	}
	pub(super) fn accessibility_settings(&mut self, ui: &mut egui::Ui) {
		let mut preferences = self.reading_preferences;
		design::group(
			ui,
			&crate::i18n::translate("accessibility-text-and-display"),
			|ui| {
				self.zoom_row(ui, &mut preferences);
				design::switch(
					ui,
					&crate::i18n::translate(
						"reading-chat-reading-settings-compact-message-spacing",
					),
					Some(&crate::i18n::translate("accessibility-compact-description")),
					&mut preferences.compact_messages,
				);
			},
		);
		design::group(
			ui,
			&crate::i18n::translate("accessibility-motion-and-scrolling"),
			|ui| {
				if design::switch(
					ui,
					"accessibility-reduce-interface-motion",
					Some("accessibility-reduce-interface-motion-description"),
					&mut self.reduce_interface_motion,
				)
				.changed()
				{
					design::set_reduced_motion(ui.ctx(), self.reduce_interface_motion);
				}
				design::switch(
					ui,
					&crate::i18n::translate("accessibility-animate-artwork"),
					Some(&crate::i18n::translate(
						"accessibility-animate-artwork-description",
					)),
					&mut preferences.animate_gifs,
				);
				design::switch(
					ui,
					&crate::i18n::translate("reading-chat-reading-settings-smooth-scrolling"),
					Some(&crate::i18n::translate("accessibility-smooth-description")),
					&mut preferences.smooth_scrolling,
				);
				design::slider_row(
					ui,
					&crate::i18n::translate("reading-chat-reading-settings-scrolling-speed"),
					None,
					&mut preferences.scroll_speed_percent,
					25..=300,
					"%",
				);
			},
		);
		if preferences != self.reading_preferences {
			self.apply_reading_preferences(ui.ctx(), preferences);
		}
	}
}

fn optional_toggle(ui: &mut egui::Ui, key: &'static str, current: Option<bool>) -> Option<bool> {
	let label = tr(key);
	if let Some(mut value) = current {
		return design::switch(ui, &label, None, &mut value)
			.changed()
			.then_some(value);
	}
	let mut value = None;
	design::row(ui, &label, None, |ui| {
		egui::ComboBox::from_id_salt(("optional-privacy-setting", key))
			.width(150.0)
			.selected_text(tr("privacy-discord-default"))
			.show_ui(ui, |ui| {
				ui.selectable_value(
					&mut value,
					Some(true),
					tr("server-integrations-app-enabled"),
				);
				ui.selectable_value(
					&mut value,
					Some(false),
					tr("server-integrations-app-disabled"),
				);
			});
	});
	value
}
fn visibility_label(value: ProfileVisibility) -> String {
	tr(match value {
		ProfileVisibility::Default => "privacy-discord-default",
		ProfileVisibility::Friends => "privacy-friends-only",
		ProfileVisibility::FriendsAndSmallServers => "privacy-friends-small-servers",
		ProfileVisibility::FriendsAndAllServers => "privacy-friends-all-servers",
		ProfileVisibility::Unknown(_) => "privacy-unsupported-audience",
	})
}
fn redaction_label(value: Redaction) -> String {
	tr(match value {
		Redaction::Default => "privacy-discord-default",
		Redaction::Show => "privacy-redaction-show",
		Redaction::Blur => "privacy-redaction-blur",
		Redaction::Block => "privacy-redaction-block",
		Redaction::Unknown(_) => "privacy-unsupported-setting",
	})
}
fn change_description(change: PrivacyChange) -> String {
	let enabled_label = |value| {
		tr(if value {
			"server-integrations-app-enabled"
		} else {
			"server-integrations-app-disabled"
		})
	};
	match change {
		PrivacyChange::ProfileVisibility(value) => format!(
			"{} — {}",
			tr("privacy-profile-title"),
			visibility_label(value)
		),
		PrivacyChange::Content {
			kind,
			audience,
			level,
		} => format!(
			"{} · {} — {}",
			tr(match kind {
				ContentKind::Sensitive => "privacy-sensitive",
				ContentKind::Gore => "privacy-graphic",
				ContentKind::SelfHarm => "privacy-self-harm",
			}),
			tr(match audience {
				Audience::Servers => "privacy-in-servers",
				Audience::Friends => "privacy-friends-dms",
				Audience::Others => "privacy-other-dms",
			}),
			redaction_label(level)
		),
		PrivacyChange::Keyword { keyword, enabled } => format!(
			"{} — {}",
			tr(match keyword {
				Keyword::Profanity => "automod-profanity",
				Keyword::SexualContent => "privacy-sexual-language",
				Keyword::Slurs => "automod-slurs",
			}),
			enabled_label(enabled)
		),
		PrivacyChange::ConversationWarnings(value) => format!(
			"{} — {}",
			tr("privacy-conversation-warnings"),
			enabled_label(value)
		),
		PrivacyChange::ShowLocalTime(value) => {
			format!(
				"{} — {}",
				tr("privacy-local-time"),
				tr(if value {
					"privacy-state-shown"
				} else {
					"privacy-state-hidden"
				})
			)
		}
		PrivacyChange::DetectPlatformAccounts(value) => format!(
			"{} — {}",
			tr("privacy-detect-platform-accounts"),
			enabled_label(value)
		),
		PrivacyChange::NonSpamTraining(value) => {
			format!(
				"{} — {}",
				tr("privacy-non-spam-training"),
				enabled_label(value)
			)
		}
	}
}

#[cfg(test)]
mod accessibility_tests {
	use super::*;

	#[test]
	fn accessibility_controls_use_the_selected_language_for_assistive_labels() {
		let mut view = MessagingUi::default();
		for language in [
			crate::i18n::Language::English,
			crate::i18n::Language::Spanish,
		] {
			crate::i18n::set_current(language);
			let ctx = egui::Context::default();
			ctx.enable_accesskit();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(600.0, 800.0),
					)),
					..Default::default()
				},
				|ui| view.accessibility_settings(ui),
			);
			let update = output.platform_output.accesskit_update.as_ref().unwrap();
			for key in [
				"reading-chat-reading-settings-compact-message-spacing",
				"accessibility-animate-artwork",
				"reading-chat-reading-settings-smooth-scrolling",
			] {
				let label = language.text(key);
				assert!(
					update.nodes.iter().any(|(_, node)| {
						node.role() == egui::Role::CheckBox && node.label() == Some(label.as_str())
					}),
					"missing {language:?} control {key}"
				);
			}
			output.drop_without_applying_deltas();
		}
		crate::i18n::set_current(crate::i18n::Language::English);
	}
}

fn preference_error(error: Failure) -> String {
	let key = match error {
		Failure::Expired => "privacy-error-expired",
		Failure::Challenged => "privacy-error-challenged",
		Failure::Forbidden => "privacy-error-forbidden",
		Failure::RateLimited => "privacy-error-rate-limited",
		Failure::Network => "privacy-error-network",
		Failure::Busy => "privacy-error-busy",
		Failure::ResponseLimit => "privacy-error-response-limit",
		Failure::Ambiguous => "privacy-error-ambiguous",
		Failure::Protocol => "privacy-error-protocol",
		Failure::Capacity => "privacy-error-capacity",
		Failure::InvalidCredential => "privacy-error-invalid",
		Failure::ProtocolAt("Connect to Discord to update messaging permissions") => {
			"privacy-error-connect"
		}
		Failure::ProtocolAt("Discord messaging permissions are unavailable or unsupported") => {
			"privacy-error-unavailable"
		}
		Failure::ProtocolAt("Messaging permissions were not confirmed; reload before retrying") => {
			"privacy-error-unconfirmed"
		}
		Failure::ProtocolAt(stage) | Failure::CapacityAt(stage) => return stage.to_owned(),
	};
	tr(key)
}

#[cfg(test)]
mod privacy_tests {
	use super::*;

	#[test]
	fn privacy_catalogs_keep_labels_errors_and_draft_details_in_every_language() {
		for language in crate::i18n::Language::ALL {
			crate::i18n::set_current(language);
			for key in [
				"privacy-prefs-updating",
				"privacy-prefs-reload",
				"privacy-prefs-previous",
				"privacy-prefs-apply",
				"privacy-prefs-discard",
				"privacy-sensitive",
				"privacy-sensitive-description",
				"privacy-graphic",
				"privacy-graphic-description",
				"privacy-self-harm",
				"privacy-self-harm-description",
				"privacy-in-servers",
				"privacy-friends-dms",
				"privacy-other-dms",
				"privacy-filtered-language",
				"privacy-sexual-language",
				"privacy-profile-title",
				"privacy-profile-description",
				"privacy-friends-description",
				"privacy-small-servers-description",
				"privacy-all-servers-description",
				"privacy-preferences-heading",
				"privacy-conversation-warnings",
				"privacy-local-time",
				"privacy-detect-platform-accounts",
				"privacy-non-spam-training",
				"privacy-discord-default",
				"privacy-friends-only",
				"privacy-friends-small-servers",
				"privacy-friends-all-servers",
				"privacy-unsupported-audience",
				"privacy-redaction-show",
				"privacy-redaction-blur",
				"privacy-redaction-block",
				"privacy-unsupported-setting",
				"privacy-state-shown",
				"privacy-state-hidden",
				"privacy-error-expired",
				"privacy-error-challenged",
				"privacy-error-forbidden",
				"privacy-error-rate-limited",
				"privacy-error-network",
				"privacy-error-ambiguous",
				"privacy-error-protocol",
				"privacy-error-capacity",
				"privacy-error-invalid",
				"privacy-error-connect",
				"privacy-error-unavailable",
				"privacy-error-unconfirmed",
			] {
				assert!(
					!tr(key).starts_with("Unknown localization key:"),
					"{language:?}: {key}"
				);
			}
			for (key, argument) in [
				("privacy-prefs-unsaved", "description"),
				("privacy-current-audience", "audience"),
			] {
				let text = translate_args(key, &[(argument, "synthetic-value")]);
				assert!(
					text.contains("synthetic-value"),
					"{language:?}: {key}: {text}"
				);
			}
		}
		crate::i18n::set_current(crate::i18n::Language::English);
	}

	#[test]
	fn privacy_and_content_filters_are_localized_without_mutating_saved_preferences() {
		let english = crate::i18n::Language::English.text("privacy-sensitive");
		let spanish = crate::i18n::Language::Spanish.text("privacy-sensitive");
		assert_ne!(english, spanish);
		for language in [
			crate::i18n::Language::English,
			crate::i18n::Language::Spanish,
		] {
			crate::i18n::set_current(language);
			for (content_filters, title_key) in [
				(false, "privacy-profile-title"),
				(true, "privacy-sensitive"),
			] {
				let ctx = egui::Context::default();
				ctx.enable_accesskit();
				let mut view = MessagingUi::default();
				let mut state = State {
					demo: true,
					..Default::default()
				};
				let snapshot = model::messaging_permissions::Snapshot::default();
				state.messaging_permissions.snapshot = Some(snapshot.clone());
				view.settings.messaging_permissions.requested = true;
				view.settings.messaging_permissions.generation = state.generation;
				let mut commands = Vec::new();
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(680.0, 1800.0),
						)),
						..Default::default()
					},
					|ui| view.privacy_settings(ui, &mut state, &mut commands, content_filters),
				);
				let title = language.text(title_key).to_uppercase();
				assert!(
					output
						.platform_output
						.accesskit_update
						.as_ref()
						.unwrap()
						.nodes
						.iter()
						.any(|(_, node)| node.value() == Some(title.as_str())
							|| node.label() == Some(title.as_str()))
				);
				assert!(commands.is_empty());
				assert_eq!(state.messaging_permissions.snapshot, Some(snapshot));
				output.drop_without_applying_deltas();
			}
		}
		crate::i18n::set_current(crate::i18n::Language::English);
	}

	#[test]
	fn privacy_failure_displays_localized_reason_and_preserves_the_unsaved_change() {
		crate::i18n::set_current(crate::i18n::Language::Spanish);
		let ctx = egui::Context::default();
		ctx.enable_accesskit();
		let mut view = MessagingUi::default();
		view.settings.messaging_permissions.requested = true;
		let mut state = State {
			demo: true,
			..Default::default()
		};
		state.messaging_permissions.snapshot =
			Some(model::messaging_permissions::Snapshot::default());
		view.settings.messaging_permissions.generation = state.generation;
		state.messaging_permissions.error = Some(Failure::Ambiguous);
		state.messaging_permissions.reload_required = true;
		let draft = Change::Privacy(PrivacyChange::ShowLocalTime(true));
		state.messaging_permissions.draft = Some(draft.clone());
		let mut commands = Vec::new();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(680.0, 1400.0),
				)),
				..Default::default()
			},
			|ui| view.privacy_settings(ui, &mut state, &mut commands, false),
		);
		let reason = tr("privacy-error-ambiguous");
		assert!(!reason.starts_with("Unknown localization key:"));
		let reason_visible = output
			.platform_output
			.accesskit_update
			.as_ref()
			.unwrap()
			.nodes
			.iter()
			.any(|(_, node)| {
				node.value() == Some(reason.as_str()) || node.label() == Some(reason.as_str())
			});
		output.drop_without_applying_deltas();
		assert!(reason_visible);
		assert_eq!(state.messaging_permissions.draft, Some(draft));
		assert!(
			state.messaging_permissions.reload_required && !state.messaging_permissions.pending
		);
		assert!(commands.is_empty());
		crate::i18n::set_current(crate::i18n::Language::English);
	}
}
