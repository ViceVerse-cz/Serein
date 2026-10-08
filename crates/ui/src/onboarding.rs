//! Rules screening and onboarding questions a server asks before a new member can talk.
use crate::{
	design, dialog,
	i18n::{translate, translate_args},
};
use client_core::{Command, State};
use model::{
	Id,
	onboarding::{Answer, Field, FieldKind, Form, Outcome, Prompt, Submission, Verification},
};

#[derive(Default)]
pub struct OnboardingUi {
	key: Option<(Id, u64)>,
	seeded: bool,
	chosen: Vec<Id>,
	answers: Vec<Answer>,
}

enum Footer {
	Finish,
	Retry,
	Close,
}

impl OnboardingUi {
	fn seed(&mut self, form: &Form) {
		self.chosen = form
			.onboarding
			.as_ref()
			.map(|o| o.responses.clone())
			.unwrap_or_default();
		self.answers = form
			.verification
			.as_ref()
			.map(|v| {
				v.fields
					.iter()
					.map(|f| match f.kind {
						FieldKind::Terms => Answer::Terms(false),
						FieldKind::MultipleChoice => Answer::Choice(None),
						_ => Answer::Text(String::new()),
					})
					.collect()
			})
			.unwrap_or_default();
		self.seeded = true;
	}
	fn submission(&self, form: &Form) -> Submission {
		Submission {
			onboarding: form.onboarding.as_ref().map(|_| self.chosen.clone()),
			verification: form.verification.as_ref().map(|_| self.answers.clone()),
		}
	}
	fn complete(&self, form: &Form) -> bool {
		form.onboarding
			.as_ref()
			.is_none_or(|o| o.complete(&self.chosen))
			&& form
				.verification
				.as_ref()
				.is_none_or(|v| v.complete(&self.answers))
	}

	pub(super) fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let Some(guild) = state.onboarding.flow.as_ref().map(|f| f.guild) else {
			*self = Self::default();
			return;
		};
		if self.key != Some((guild, state.generation)) {
			*self = Self {
				key: Some((guild, state.generation)),
				..Self::default()
			};
		}
		if let Some(command) = state.load_onboarding() {
			commands.push(command);
		}
		let Some(flow) = state.onboarding.flow.as_ref() else {
			return;
		};
		if !self.seeded
			&& let Some(form) = &flow.form
		{
			self.seed(form);
		}
		let name = server_name(state, guild);
		let finished = flow.outcome.is_some() || flow.form.as_ref().is_some_and(Form::is_empty);
		let footer = if flow.error.is_some() {
			Footer::Retry
		} else if finished {
			Footer::Close
		} else {
			Footer::Finish
		};
		let ready = !flow.loading
			&& !flow.submitting
			&& flow.form.as_ref().is_some_and(|f| self.complete(f));
		let busy = flow.loading || flow.submitting;
		let mut action = None;
		let mut close = false;
		let response = dialog::Dialog::new(
			"server-onboarding",
			translate_args("onboarding-show-title", &[("server", &name)]),
		)
		.subtitle(translate("onboarding-show-subtitle"))
		.icon(crate::icons::Icon::Sparkle)
		.width(540.0)
		.show(ctx, |d| {
			d.scroll(190.0, |ui| self.body(ui, flow));
			d.footer(|ui| {
				let (label, enabled) = match footer {
					Footer::Retry => ("onboarding-show-try-again", !busy),
					Footer::Close => ("onboarding-show-close", true),
					Footer::Finish if busy => ("onboarding-show-please-wait", false),
					Footer::Finish => ("onboarding-show-finish", ready),
				};
				ui.add_enabled_ui(enabled, |ui| {
					if dialog::action(ui, label, dialog::Action::Primary).clicked() {
						action = Some(());
					}
				});
				if !finished
					&& dialog::action(ui, "onboarding-show-not-now", dialog::Action::Neutral)
						.clicked()
				{
					close = true;
				}
			});
		});
		if action.is_some() {
			match footer {
				Footer::Retry => state.retry_onboarding(),
				Footer::Close => close = true,
				Footer::Finish => {
					let submission = state
						.onboarding
						.flow
						.as_ref()
						.and_then(|f| f.form.as_ref())
						.map(|form| self.submission(form));
					if let Some(command) = submission.and_then(|s| state.submit_onboarding(s)) {
						commands.push(command);
					}
				}
			}
		}
		if close || response.close {
			state.close_onboarding();
		}
	}

	fn body(&mut self, ui: &mut egui::Ui, flow: &client_core::onboarding::Flow) {
		ui.spacing_mut().item_spacing.y = 8.0;
		if let Some(error) = flow.error {
			design::notice(ui, design::Level::Error, error);
			return;
		}
		match flow.outcome {
			Some(Outcome::Submitted) => {
				design::notice(ui, design::Level::Success, "onboarding-body-submitted");
				return;
			}
			Some(Outcome::Rejected) => {
				design::notice(ui, design::Level::Error, "onboarding-body-rejected");
				return;
			}
			_ => {}
		}
		let Some(form) = flow.form.as_ref().filter(|_| !flow.loading) else {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.label(translate("onboarding-body-loading"));
			});
			return;
		};
		if form.is_empty() {
			design::notice(ui, design::Level::Success, "onboarding-body-all-set");
			return;
		}
		ui.add_enabled_ui(!flow.submitting, |ui| {
			if let Some(onboarding) = &form.onboarding {
				for prompt in &onboarding.prompts {
					self.prompt(ui, prompt);
					ui.add_space(8.0);
				}
			}
			if let Some(verification) = &form.verification {
				self.verification(ui, verification);
			}
		});
	}

	fn prompt(&mut self, ui: &mut egui::Ui, prompt: &Prompt) {
		let colors = design::palette(ui);
		ui.horizontal_wrapped(|ui| {
			ui.label(design::semibold(ui, &prompt.title, 16.0).color(colors.text_strong));
			if prompt.required {
				ui.label(egui::RichText::new("*").color(colors.danger));
			}
		});
		ui.label(
			egui::RichText::new(translate(if prompt.single_select {
				"onboarding-prompt-pick-one"
			} else {
				"onboarding-prompt-pick-any"
			}))
			.size(12.5)
			.color(colors.muted),
		);
		for option in &prompt.options {
			let selected = self.chosen.contains(&option.id);
			if option_card(
				ui,
				selected,
				option.emoji.as_deref(),
				&option.title,
				option.description.as_deref(),
			)
			.clicked()
			{
				if selected {
					self.chosen.retain(|id| *id != option.id);
				} else {
					if prompt.single_select {
						self.chosen
							.retain(|id| !prompt.options.iter().any(|o| o.id == *id));
					}
					self.chosen.push(option.id);
				}
			}
		}
	}

	fn verification(&mut self, ui: &mut egui::Ui, form: &Verification) {
		let colors = design::palette(ui);
		ui.label(
			design::semibold(ui, translate("onboarding-rules-heading"), 16.0)
				.color(colors.text_strong),
		);
		if let Some(description) = &form.description {
			ui.label(egui::RichText::new(description).color(colors.muted));
		}
		if form.needs_review() {
			design::notice(ui, design::Level::Info, "onboarding-rules-review");
		}
		for (index, field) in form.fields.iter().enumerate() {
			let Some(answer) = self.answers.get_mut(index) else {
				break;
			};
			ui.add_space(4.0);
			ui.push_id(("onboarding-field", index), |ui| {
				field_ui(ui, field, answer)
			});
		}
	}
}

fn field_ui(ui: &mut egui::Ui, field: &Field, answer: &mut Answer) {
	let colors = design::palette(ui);
	if field.kind == FieldKind::Unknown {
		if field.required {
			design::notice(ui, design::Level::Warning, "onboarding-field-unsupported");
		}
		return;
	}
	if !field.label.is_empty() {
		ui.horizontal_wrapped(|ui| {
			ui.label(design::medium(ui, &field.label, 14.5).color(colors.text_strong));
			if field.required && field.kind != FieldKind::Terms {
				ui.label(egui::RichText::new("*").color(colors.danger));
			}
		});
	}
	if let Some(description) = &field.description {
		ui.label(
			egui::RichText::new(description)
				.size(12.5)
				.color(colors.muted),
		);
	}
	match (&field.kind, answer) {
		(FieldKind::Terms, Answer::Terms(agreed)) => {
			design::card(ui, |ui| {
				for (index, rule) in field.rules.iter().enumerate() {
					ui.horizontal_top(|ui| {
						ui.label(
							egui::RichText::new(format!("{}.", index + 1))
								.strong()
								.color(colors.muted),
						);
						ui.add(
							egui::Label::new(egui::RichText::new(rule).color(colors.text)).wrap(),
						);
					});
				}
			});
			ui.checkbox(agreed, translate("onboarding-field-agree"));
		}
		(FieldKind::TextInput | FieldKind::Paragraph, Answer::Text(text)) => {
			let editor = if field.kind == FieldKind::Paragraph {
				egui::TextEdit::multiline(text).desired_rows(4)
			} else {
				egui::TextEdit::singleline(text).align(egui::Align2::LEFT_CENTER)
			};
			ui.add(
				editor
					.char_limit(field.max_chars())
					.hint_text(field.placeholder.as_deref().unwrap_or_default())
					.desired_width(f32::INFINITY),
			);
		}
		(FieldKind::MultipleChoice, Answer::Choice(choice)) => {
			for (index, label) in field.choices.iter().enumerate() {
				if design::radio_row(ui, *choice == Some(index), label, None).clicked() {
					*choice = Some(index);
				}
			}
		}
		_ => {}
	}
}

/// One selectable onboarding answer: emoji, title and optional detail in an outlined row.
fn option_card(
	ui: &mut egui::Ui,
	selected: bool,
	emoji: Option<&str>,
	title: &str,
	detail: Option<&str>,
) -> egui::Response {
	let colors = design::palette(ui);
	let enabled = ui.is_enabled();
	let frame = egui::Frame::new()
		.fill(if selected {
			colors.accent.gamma_multiply(0.14)
		} else {
			colors.raised
		})
		.stroke(egui::Stroke::new(
			1.0,
			if selected {
				colors.accent
			} else {
				colors.border
			},
		))
		.corner_radius(8)
		.inner_margin(egui::Margin::symmetric(12, 10))
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			ui.horizontal(|ui| {
				if let Some(image) = emoji.and_then(|e| crate::emoji::image(ui.ctx(), e, 22.0)) {
					ui.add(image);
				}
				ui.vertical(|ui| {
					ui.spacing_mut().item_spacing.y = 2.0;
					ui.label(design::medium(ui, title, 14.5).color(colors.text_strong));
					if let Some(detail) = detail {
						ui.label(egui::RichText::new(detail).size(12.5).color(colors.muted));
					}
				});
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if selected {
						crate::icons::inline(ui, crate::icons::Icon::Check, 16.0, colors.accent);
					}
				});
			});
		});
	let response = ui.interact(
		frame.response.rect,
		frame.response.id.with("option"),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	response
		.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, title));
	if response.hovered() && enabled {
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
	}
	response
}

fn server_name(state: &State, guild: Id) -> String {
	state
		.guild(guild)
		.map(|g| g.name.clone())
		.or_else(|| {
			state
				.invites
				.values()
				.filter_map(|(_, preview)| preview.as_ref()?.as_ref().ok())
				.find(|preview| preview.guild == guild)
				.and_then(|preview| preview.embed.title.clone())
		})
		.unwrap_or_else(|| translate("onboarding-show-this-server"))
}
