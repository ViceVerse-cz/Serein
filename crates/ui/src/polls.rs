//! Native poll cards and a bounded creation dialog.
use client_core::{
	State,
	polls::{Action, now_ms},
};
use egui::{RichText, Stroke};
use model::{
	Id, Message,
	polls::{Create, Media},
};

#[derive(Default)]
pub struct Cards {
	selection: Option<(Id, Id, Vec<u32>, bool)>,
}
impl Cards {
	pub fn show(&mut self, ui: &mut egui::Ui, state: &State, message: &Message) -> Option<Action> {
		let poll = message.poll.as_ref()?;
		let colors = crate::design::palette(ui);
		let ended = poll.ended(now_ms());
		let voted = poll.answers.iter().any(|a| a.me);
		let mut local = self
			.selection
			.as_ref()
			.filter(|s| s.0 == message.channel && s.1 == message.id)
			.cloned()
			.unwrap_or((message.channel, message.id, Vec::new(), false));
		let (_, _, selected, preview) = &mut local;
		let mut changed = false;
		selected.retain(|id| poll.answers.iter().any(|a| a.id == *id));
		let results = ended || voted || *preview;
		let enabled = state.can_vote_poll(message) && state.polls.pending.is_none();
		let mut action = None;
		let width = (ui.available_width() - 28.0).clamp(80.0, 350.0);
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(Stroke::new(1.0, colors.border))
			.corner_radius(8)
			.inner_margin(14)
			.show(ui, |ui| {
				ui.set_width(width);
				ui.label(
					crate::design::semibold(ui, &poll.question, 16.0)
						.size(16.0)
						.color(colors.text_strong),
				);
				ui.label(
					RichText::new(if ended {
						"Poll ended"
					} else if poll.multiselect {
						"Select one or more answers"
					} else {
						"Select one answer"
					})
					.small()
					.color(colors.muted),
				);
				ui.add_space(8.0);
				let total = poll.votes();
				for answer in &poll.answers {
					let checked = if results {
						answer.me
					} else {
						selected.contains(&answer.id)
					};
					let emoji = answer
						.media
						.emoji
						.as_ref()
						.map(|e| format!("{} ", e.label()))
						.unwrap_or_default();
					let fraction = if total == 0 {
						0.0
					} else {
						answer.votes as f32 / total as f32
					};
					let tally = if results && poll.results_known {
						format!("   {} votes - {:.0}%", answer.votes, fraction * 100.0)
					} else {
						String::new()
					};
					let label = format!(
						"{}{}{}{}",
						emoji,
						answer.media.text,
						if checked {
							"   \u{2713}"
						} else if !results {
							"   \u{25cb}"
						} else {
							""
						},
						tally
					);
					let background = ui.painter().add(egui::Shape::Noop);
					let response = ui.add_enabled(
						results || enabled,
						egui::Button::new(RichText::new(label).color(colors.text_strong))
							.wrap()
							.sense(if results {
								egui::Sense::hover()
							} else {
								egui::Sense::click()
							})
							.selected(checked)
							.fill(egui::Color32::TRANSPARENT)
							.stroke(Stroke::new(
								1.0,
								if checked {
									colors.accent
								} else {
									colors.border
								},
							))
							.corner_radius(6)
							.min_size(egui::vec2(width, 40.0)),
					);
					let mut shapes = vec![egui::Shape::rect_filled(response.rect, 6, colors.chat)];
					if results && poll.results_known {
						let mut bar = response.rect;
						bar.max.x = bar.min.x + bar.width() * fraction;
						if fraction > 0.0 {
							shapes.push(egui::Shape::rect_filled(
								bar,
								6,
								colors
									.accent
									.gamma_multiply(if checked { 0.3 } else { 0.12 }),
							));
						}
					} else if checked {
						shapes.push(egui::Shape::rect_filled(
							response.rect,
							6,
							colors.accent.gamma_multiply(0.18),
						));
					}
					ui.painter().set(background, egui::Shape::Vec(shapes));
					if response.clicked() {
						changed = true;
						if checked {
							selected.retain(|id| *id != answer.id);
						} else {
							if !poll.multiselect {
								selected.clear();
							}
							selected.push(answer.id);
						}
					}
				}
				ui.add_space(8.0);
				let remaining = poll.expiry.map(|e| (e - now_ms()).max(0) / 60000);
				let footer = if ended {
					if poll.finalized {
						"Final results".to_owned()
					} else {
						"Awaiting final results".to_owned()
					}
				} else {
					remaining
						.map(|m| {
							if m >= 60 {
								format!("{}h left", (m + 59) / 60)
							} else {
								format!("{m}m left")
							}
						})
						.unwrap_or_else(|| "In progress".into())
				};
				ui.label(
					RichText::new(if poll.results_known {
						format!("{} votes - {footer}", poll.votes())
					} else {
						format!("Results not loaded - {footer}")
					})
					.small()
					.color(colors.muted),
				);
				ui.horizontal_wrapped(|ui| {
					if ui
						.add_enabled(
							state.polls.pending.is_none(),
							egui::Button::new(if *preview && !voted && !ended {
								"Back to voting"
							} else {
								"Show results"
							})
							.small(),
						)
						.clicked()
					{
						changed = true;
						*preview = !*preview;
						if *preview || voted || ended {
							action = Some(Action::Read);
						}
					}
					if voted && !ended {
						if ui
							.add_enabled(enabled, egui::Button::new("Remove Vote").small())
							.clicked()
						{
							action = Some(Action::Vote(Vec::new()));
							selected.clear();
							changed = true;
							*preview = false;
						}
					} else if !ended
						&& ui
							.add_enabled(
								enabled && !selected.is_empty() && !results,
								egui::Button::new(
									RichText::new("Vote").color(egui::Color32::WHITE),
								)
								.fill(colors.accent),
							)
							.clicked()
					{
						action = Some(Action::Vote(selected.clone()));
					}
					if !ended
						&& state
							.user
							.as_ref()
							.is_some_and(|u| u.id == message.author.id)
					{
						ui.menu_button("End Poll", |ui| {
							ui.label("End this poll for everyone?");
							if ui
								.add_enabled(enabled, egui::Button::new("End now"))
								.clicked()
							{
								action = Some(Action::End);
								ui.close();
							}
						});
					}
				});
				if let Some(error) = state.polls.error {
					ui.label(RichText::new(error).small().color(colors.danger));
				}
			});
		if changed {
			self.selection = Some(local);
		}
		action
	}
}

#[derive(Default)]
pub struct Creator {
	draft: Option<(Id, Create, u64, bool)>,
}
impl Creator {
	pub fn open(&mut self, state: &State, channel: Id) {
		if self.draft.is_none() {
			self.draft = Some((
				channel,
				Create {
					question: String::new(),
					answers: vec![
						Media {
							text: String::new(),
							emoji: None
						};
						2
					],
					duration: 24,
					multiselect: false,
				},
				state.polls.created,
				false,
			));
		}
	}
	pub fn show(&mut self, ctx: &egui::Context, state: &State) -> Option<Action> {
		let (channel, draft, created, posting) = self.draft.as_mut()?;
		if state.selected != Some(*channel) || state.polls.created != *created {
			self.draft = None;
			return None;
		}
		if *posting && state.polls.pending.is_none() {
			*posting = false;
		}
		let mut action = None;
		let mut close = false;
		egui::Modal::new(egui::Id::unique("poll-create")).show(ctx, |ui| {
			ui.set_width(360.0_f32.min(ctx.content_rect().width() - 48.0));
			ui.heading("Create a poll");
			ui.add_space(12.0);
			egui::ScrollArea::vertical()
				.max_height((ctx.content_rect().height() - 180.0).max(100.0))
				.show(ui, |ui| {
					ui.add_enabled_ui(!*posting, |ui| {
						ui.label("Question");
						ui.add(
							egui::TextEdit::multiline(&mut draft.question)
								.hint_text("What would you like to ask?")
								.char_limit(300)
								.desired_rows(2)
								.desired_width(f32::INFINITY),
						);
						ui.add_space(8.0);
						ui.label("Answers");
						let mut remove = None;
						let count = draft.answers.len();
						for (i, answer) in draft.answers.iter_mut().enumerate() {
							ui.horizontal(|ui| {
								let mut emoji = answer
									.emoji
									.as_ref()
									.and_then(|e| e.name.clone())
									.unwrap_or_default();
								if ui
									.add(
										egui::TextEdit::singleline(&mut emoji)
											.hint_text("Emoji")
											.char_limit(32)
											.desired_width(40.0),
									)
									.on_hover_text("Optional Unicode emoji")
									.changed()
								{
									answer.emoji = (!emoji.trim().is_empty()).then_some(
										model::ReactionEmoji {
											id: None,
											name: Some(emoji),
										},
									);
								}
								ui.add(
									egui::TextEdit::singleline(&mut answer.text)
										.hint_text(format!("Answer {}", i + 1))
										.char_limit(55)
										.desired_width((ui.available_width() - 70.0).max(60.0)),
								);
								if ui
									.add_enabled(count > 2, egui::Button::new("Remove"))
									.on_hover_text("Remove answer")
									.clicked()
								{
									remove = Some(i);
								}
							});
						}
						if let Some(i) = remove {
							draft.answers.remove(i);
						}
						if ui
							.add_enabled(
								draft.answers.len() < 10,
								egui::Button::new("+ Add answer"),
							)
							.clicked()
						{
							draft.answers.push(Media {
								text: String::new(),
								emoji: None,
							});
						}
						ui.add_space(8.0);
						ui.checkbox(&mut draft.multiselect, "Allow multiple answers");
						egui::ComboBox::from_id_salt("poll-duration")
							.selected_text(duration_label(draft.duration))
							.show_ui(ui, |ui| {
								for hours in [1, 4, 8, 24, 72, 168] {
									ui.selectable_value(
										&mut draft.duration,
										hours,
										duration_label(hours),
									);
								}
							});
					});
				});
			if let Some(error) = state.polls.error {
				ui.colored_label(crate::design::palette(ui).danger, error);
			}
			ui.add_space(12.0);
			ui.horizontal(|ui| {
				if ui
					.add_enabled(!*posting, egui::Button::new("Cancel"))
					.clicked()
				{
					close = true;
				}
				if ui
					.add_enabled(
						!*posting && draft.valid() && state.can_create_poll(*channel),
						egui::Button::new(if *posting { "Posting..." } else { "Post" }),
					)
					.clicked()
				{
					action = Some(Action::Create(Box::new(draft.clone())));
					*posting = true;
				}
			});
		});
		if close {
			self.draft = None;
		}
		action
	}
}
fn duration_label(hours: u16) -> String {
	match hours {
		1 => "1 hour".into(),
		24 => "1 day".into(),
		168 => "1 week".into(),
		72 => "3 days".into(),
		_ => format!("{hours} hours"),
	}
}

#[cfg(feature = "demo")]
pub fn debug_poll_check(state: &State) {
	for width in [260.0, 900.0] {
		for light in [true, false] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if light {
				egui::Visuals::light()
			} else {
				egui::Visuals::dark()
			});
			let input = egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(width, 900.0),
				)),
				..Default::default()
			};
			let mut output = ctx.run_ui(input, |ui| {
				ui.set_width(width - 16.0);
				let mut cards = Cards::default();
				for message in state.timeline.iter() {
					cards.show(ui, state, message);
				}
				assert!(
					ui.min_rect().width() <= width,
					"poll layout width {} exceeds {}",
					ui.min_rect().width(),
					width
				);
			});
			output.textures_delta.clear();
		}
	}
}
