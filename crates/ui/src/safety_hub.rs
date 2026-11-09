//! Clearly labelled read-only account standing in Data & Privacy. The service read happens only
//! after an explicit click and is never inferred from flags, DMs or the profile.
use crate::{MessagingUi, design, i18n};
use client_core::{Command, State};
use model::safety_hub::{AccountStanding, AppealStatus, Snapshot};

/// Discord's official support article for the warning system and account standing. No public
/// Safety Hub web URL is documented, so this is the official reference the app can offer.
const OFFICIAL_STANDING_ARTICLE: &str = "https://support.discord.com/hc/articles/18210965981847";

fn standing_key(standing: Option<AccountStanding>) -> &'static str {
	match standing {
		Some(AccountStanding::AllGood) => "settings-safety-hub-all-good",
		Some(AccountStanding::Limited) => "settings-safety-hub-limited",
		Some(AccountStanding::VeryLimited) => "settings-safety-hub-very-limited",
		Some(AccountStanding::AtRisk) => "settings-safety-hub-at-risk",
		Some(AccountStanding::Suspended) => "settings-safety-hub-suspended",
		None => "settings-safety-hub-unavailable",
	}
}

fn appeal_key(status: AppealStatus) -> &'static str {
	match status {
		AppealStatus::ReviewPending => "settings-safety-hub-appeal-pending",
		AppealStatus::Upheld => "settings-safety-hub-appeal-upheld",
		AppealStatus::Invalidated => "settings-safety-hub-appeal-invalidated",
	}
}

fn standing_status(snapshot: Option<&Snapshot>, loading: bool, error: Option<&str>) -> String {
	if loading {
		return i18n::translate("settings-safety-hub-loading");
	}
	if let Some(error) = error {
		return error.to_owned();
	}
	match snapshot {
		Some(snapshot) => i18n::translate(standing_key(snapshot.standing)),
		None => i18n::translate("settings-safety-hub-not-checked"),
	}
}

impl MessagingUi {
	pub(super) fn safety_hub_settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let snapshot = state.safety_hub.snapshot.clone();
		let loading = state.safety_hub.loading;
		let error = state.safety_hub.error;
		let mut requested = false;
		design::group(ui, &i18n::translate("settings-safety-hub-title"), |ui| {
			design::hint(
				ui,
				&i18n::translate("settings-safety-hub-read-only-unofficial"),
			);
			let status = standing_status(snapshot.as_ref(), loading, error);
			design::row(ui, "settings-safety-hub-standing", Some(&status), |ui| {
				ui.add_enabled_ui(!loading, |ui| {
					if design::button(ui, "settings-safety-hub-check", design::ButtonKind::Outline)
						.clicked()
					{
						requested = true;
					}
				});
			});
			if let Some(snapshot) = &snapshot {
				if !snapshot.classifications.is_empty() {
					design::card_divider(ui);
				}
				for classification in &snapshot.classifications {
					let title = if classification.description.is_empty() {
						i18n::translate_args(
							"settings-safety-hub-classification",
							&[("type", &classification.classification_type.to_string())],
						)
					} else {
						classification.description.clone()
					};
					ui.add(egui::Label::new(design::medium(ui, title, 14.0)).wrap());
					for action in &classification.actions {
						if let Some(description) = action.descriptions.first() {
							design::hint(ui, description);
						} else {
							design::hint(
								ui,
								&i18n::translate_args(
									"settings-safety-hub-action",
									&[("type", &action.action_type.to_string())],
								),
							);
						}
					}
					if let Some(at) = classification.max_expiration_time
						&& let Some(time) = crate::local_time::discord_timestamp(at, b'F')
					{
						design::hint(
							ui,
							&i18n::translate_args(
								"settings-safety-hub-expires",
								&[("time", &time)],
							),
						);
					}
					if let Some(status) = classification.appeal_status {
						design::hint(ui, &i18n::translate(appeal_key(status)));
					}
					if let Some(link) = &classification.explainer_link
						&& design::button(
							ui,
							"settings-safety-hub-open-explanation",
							design::ButtonKind::Neutral,
						)
						.clicked()
					{
						self.settings.safety_hub_opening = Some(link.clone());
					}
					ui.add_space(4.0);
				}
				if snapshot.is_appeal_eligible || !snapshot.appeal_eligibility.is_empty() {
					design::hint(ui, &i18n::translate("settings-safety-hub-appeal-eligible"));
				}
				if snapshot.is_dsa_eligible {
					design::hint(ui, &i18n::translate("settings-safety-hub-dsa-eligible"));
				}
			}
			design::card_divider(ui);
			if design::button(
				ui,
				"settings-safety-hub-official-info",
				design::ButtonKind::Neutral,
			)
			.clicked()
			{
				self.settings.safety_hub_opening = Some(OFFICIAL_STANDING_ARTICLE.into());
			}
		});
		if requested && let Some(command) = state.load_safety_hub() {
			commands.push(command);
		}
		crate::markdown::confirm_external_link(
			ui.ctx(),
			&mut self.settings.safety_hub_opening,
			true,
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::safety_hub::Snapshot;

	fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
		match shape {
			egui::Shape::Text(text) => {
				(text.galley.job.text == label).then(|| text.visual_bounding_rect())
			}
			egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, label)),
			_ => None,
		}
	}

	fn collect_text(shape: &egui::Shape, labels: &mut Vec<String>) {
		match shape {
			egui::Shape::Text(text) => labels.push(text.galley.job.text.clone()),
			egui::Shape::Vec(shapes) => {
				for shape in shapes {
					collect_text(shape, labels);
				}
			}
			_ => {}
		}
	}

	#[test]
	fn only_known_standing_labels_are_reported_and_missing_is_unavailable() {
		assert_eq!(
			i18n::translate(standing_key(Some(AccountStanding::AllGood))),
			"All Good"
		);
		assert_eq!(
			i18n::translate(standing_key(Some(AccountStanding::Suspended))),
			"Suspended"
		);
		assert_eq!(
			standing_status(Some(&Snapshot::default()), false, None),
			"Unavailable"
		);
		assert_eq!(
			standing_status(
				None,
				false,
				Some("Safety Hub response was invalid or too large")
			),
			"Safety Hub response was invalid or too large"
		);
	}

	#[test]
	fn explicit_check_emits_one_command_and_unavailable_is_not_all_good() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		let mut state = State {
			demo: true,
			..Default::default()
		};
		let render = |view: &mut MessagingUi, state: &mut State, events| {
			let mut labels = Vec::new();
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(720.0, 1400.0),
					)),
					events,
					focused: true,
					..Default::default()
				},
				|ui| {
					view.safety_hub_settings(ui, state, &mut commands);
				},
			);
			for shape in &output.shapes {
				collect_text(&shape.shape, &mut labels);
			}
			let check = output
				.shapes
				.iter()
				.find_map(|shape| text_rect(&shape.shape, "Check account standing"))
				.map(|rect| rect.center());
			output.drop_without_applying_deltas();
			(labels, commands, check)
		};

		let _ = render(&mut view, &mut state, vec![]);
		let point = render(&mut view, &mut state, vec![])
			.2
			.expect("explicit check button");
		let click = |point, pressed| {
			vec![
				egui::Event::PointerMoved(point),
				egui::Event::PointerButton {
					pos: point,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			]
		};
		let mut commands = Vec::new();
		for pressed in [true, false] {
			commands.extend(render(&mut view, &mut state, click(point, pressed)).1);
		}
		assert_eq!(commands.len(), 1, "one explicit request only");
		assert!(matches!(commands[0], Command::SafetyHub { .. }));
		assert!(state.safety_hub.loading, "the request is in flight");
		let more = render(&mut view, &mut state, click(point, true)).1;
		assert!(more.is_empty(), "one in flight only");

		state.safety_hub.loading = false;
		state.safety_hub.snapshot = Some(test_support::safety_hub::Fixture::Unavailable.snapshot());
		let labels = render(&mut view, &mut state, vec![]).0;
		assert!(
			labels.iter().any(|label| label == "Unavailable"),
			"missing standing must be unavailable"
		);
		assert!(
			!labels.iter().any(|label| label == "All Good"),
			"missing standing must never become All Good"
		);

		state.safety_hub.snapshot = Some(test_support::safety_hub::Fixture::Violation.snapshot());
		let labels = render(&mut view, &mut state, vec![]).0;
		assert!(
			labels
				.iter()
				.any(|label| { label == "Synthetic spam classification for the offline preview." })
		);
		assert!(
			labels.iter().any(|label| label == "Open explanation"),
			"HTTPS explainer links remain behind the existing confirmation"
		);

		state.safety_hub.snapshot = Some(Snapshot {
			standing: None,
			classifications: vec![model::safety_hub::Classification {
				id: model::Id(77),
				classification_type: 9999,
				description: String::new(),
				explainer_link: None,
				actions: vec![model::safety_hub::Action {
					id: model::Id(78),
					action_type: 9999,
					descriptions: Vec::new(),
				}],
				max_expiration_time: None,
				appeal_status: None,
			}],
			..Default::default()
		});
		let labels = render(&mut view, &mut state, vec![]).0;
		assert!(
			labels
				.iter()
				.any(|label| label.contains("Violation recorded (type") && label.contains("9999"))
		);
		assert!(
			labels
				.iter()
				.any(|label| label.contains("Action recorded (type") && label.contains("9999"))
		);
		assert!(labels.iter().any(|label| label == "Unavailable"));
	}
}
