//! Native poll cards and a bounded creation dialog.
use crate::{
	design,
	icons::{self, Icon},
};
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
		let width = (ui.available_width() - 32.0).clamp(48.0, 440.0);
		egui::Frame::new()
			.fill(design::glass(ui, colors.sidebar).0)
			.corner_radius(8)
			.inner_margin(16)
			.show(ui, |ui| {
				ui.set_width(width);
				ui.spacing_mut().item_spacing.y = 8.0;
				ui.label(
					crate::design::semibold(ui, &poll.question, 18.0).color(colors.text_strong),
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
					let response = ui
						.add_enabled_ui(results || enabled, |ui| {
							answer_row(ui, answer, checked, results, poll.results_known, total)
						})
						.inner;
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
				let footer = if poll.results_known {
					format!("{} • {footer}", vote_label(poll.votes()))
				} else {
					format!("Results not loaded • {footer}")
				};
				ui.horizontal_wrapped(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					let footer_width = ui
						.painter()
						.layout_no_wrap(
							footer.clone(),
							egui::FontId::proportional(12.0),
							colors.muted,
						)
						.size()
						.x;
					ui.add(
						egui::Label::new(RichText::new(footer).size(12.0).color(colors.muted))
							.wrap(),
					);
					if width < 320.0 || footer_width > 130.0 {
						ui.end_row();
					}
					let read_label = if *preview && !voted && !ended {
						"Back to voting"
					} else if results {
						"Refresh results"
					} else {
						"Show results"
					};
					if ui
						.add_enabled_ui(state.polls.pending.is_none(), |ui| {
							design::text_action(ui, read_label)
						})
						.inner
						.clicked()
					{
						changed = true;
						*preview = !*preview;
						if *preview || voted || ended {
							action = Some(Action::Read);
						}
					}
					if width < 320.0 {
						ui.end_row();
					}
					if voted && !ended {
						if ui
							.add_enabled_ui(enabled, |ui| {
								design::button(ui, "Remove Vote", design::ButtonKind::Outline)
							})
							.inner
							.clicked()
						{
							action = Some(Action::Vote(Vec::new()));
							selected.clear();
							changed = true;
							*preview = false;
						}
					} else if !ended
						&& !results && ui
						.add_enabled_ui(enabled && !selected.is_empty(), |ui| {
							design::button(ui, "Vote", design::ButtonKind::Primary)
						})
						.inner
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
						ui.menu_button(icons::atom(Icon::More, 16.0, colors.muted), |ui| {
							ui.label("End this poll for everyone?");
							if ui
								.add_enabled(enabled, egui::Button::new("End now"))
								.clicked()
							{
								action = Some(Action::End);
								ui.close();
							}
						})
						.response
						.on_hover_text("End Poll");
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

fn vote_label(votes: u64) -> String {
	format!("{votes} {}", if votes == 1 { "vote" } else { "votes" })
}

fn answer_row(
	ui: &mut egui::Ui,
	answer: &model::polls::Answer,
	checked: bool,
	results: bool,
	known: bool,
	total: u64,
) -> egui::Response {
	let p = design::palette(ui);
	let fraction = if total == 0 {
		0.0
	} else {
		answer.votes as f32 / total as f32
	};
	let label = answer.media.emoji.as_ref().map_or_else(
		|| answer.media.text.clone(),
		|emoji| format!("{} {}", emoji.label(), answer.media.text),
	);
	let emoji_image = answer
		.media
		.emoji
		.as_ref()
		.filter(|emoji| emoji.id.is_none())
		.and_then(|emoji| emoji.name.as_deref())
		.and_then(|name| crate::emoji::image(ui.ctx(), name, 24.0));
	let emoji_width = if emoji_image.is_some() { 32.0 } else { 0.0 };
	let width = ui.available_width();
	let show_tally = results && known;
	let count_width = if show_tally {
		ui.painter()
			.layout_no_wrap(
				vote_label(u64::from(answer.votes)),
				egui::FontId::proportional(12.0),
				p.muted,
			)
			.size()
			.x
	} else {
		0.0
	};
	let stacked = show_tally && width < (count_width + 220.0).max(320.0);
	let marker_width = if checked || !results { 32.0 } else { 0.0 };
	let tally_width = if show_tally && !stacked {
		count_width + 60.0
	} else {
		0.0
	};
	let text = ui.painter().layout(
		if emoji_image.is_some() {
			answer.media.text.clone()
		} else {
			label.clone()
		},
		egui::FontId::new(15.0, design::medium_family(ui.ctx())),
		p.text_strong,
		(width - 24.0 - marker_width - tally_width - emoji_width).max(16.0),
	);
	let height = (text.size().y + 24.0 + if stacked { 24.0 } else { 0.0 }).max(48.0);
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(width, height),
		if results {
			egui::Sense::hover()
		} else {
			egui::Sense::click()
		},
	);
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::Button,
			ui.is_enabled(),
			checked,
			if show_tally {
				format!(
					"{label}, {}, {:.0}%",
					vote_label(u64::from(answer.votes)),
					fraction * 100.0
				)
			} else {
				label.clone()
			},
		)
	});
	let hot = ui.is_enabled() && (response.hovered() || response.has_focus());
	let painter = ui.painter();
	painter.rect_filled(
		rect,
		8,
		design::glass(ui, if hot && !results { p.hover } else { p.raised }).0,
	);
	if show_tally && fraction > 0.0 {
		let bar = rect.with_max_x(rect.left() + width * fraction);
		painter.rect_filled(
			bar,
			8,
			design::glass(
				ui,
				design::mix(
					p.raised,
					if checked { p.accent } else { p.text },
					if checked { 0.3 } else { 0.08 },
				),
			)
			.0,
		);
	} else if checked {
		painter.rect_filled(
			rect,
			8,
			design::glass(ui, design::mix(p.raised, p.accent, 0.18)).0,
		);
	}
	if checked || response.has_focus() {
		painter.rect_stroke(
			rect,
			8,
			Stroke::new(1.5, p.accent),
			egui::StrokeKind::Inside,
		);
	}
	let color = if ui.is_enabled() {
		p.text_strong
	} else {
		p.muted
	};
	if let Some(image) = emoji_image {
		image.paint_at(
			ui,
			egui::Rect::from_min_size(
				egui::pos2(rect.left() + 12.0, rect.top() + 12.0),
				egui::Vec2::splat(24.0),
			),
		);
	}
	painter.galley_with_override_text_color(
		egui::pos2(
			rect.left() + 12.0 + emoji_width,
			if stacked {
				rect.top() + 12.0
			} else {
				rect.center().y - text.size().y * 0.5
			},
		),
		text,
		color,
	);
	let marker = egui::pos2(rect.right() - 22.0, rect.center().y);
	if checked {
		painter.circle_filled(marker, 10.0, p.accent);
		icons::paint(
			painter,
			Icon::Check,
			egui::Rect::from_center_size(marker, egui::Vec2::splat(14.0)),
			p.accent_text,
		);
	} else if !results {
		painter.circle_stroke(
			marker,
			9.0,
			Stroke::new(2.0, if hot { p.text_strong } else { p.muted }),
		);
	}
	if show_tally {
		let y = if stacked {
			rect.bottom() - 16.0
		} else {
			rect.center().y
		};
		let right = rect.right() - 12.0 - if stacked { 0.0 } else { marker_width };
		painter.text(
			egui::pos2(right, y),
			egui::Align2::RIGHT_CENTER,
			format!("{:.0}%", fraction * 100.0),
			egui::FontId::new(15.0, design::semibold_family(ui.ctx())),
			p.text_strong,
		);
		painter.text(
			egui::pos2(
				if stacked {
					rect.left() + 12.0
				} else {
					right - 44.0
				},
				y,
			),
			if stacked {
				egui::Align2::LEFT_CENTER
			} else {
				egui::Align2::RIGHT_CENTER
			},
			vote_label(u64::from(answer.votes)),
			egui::FontId::proportional(12.0),
			p.muted,
		);
	}
	response
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
		let modal = egui::Modal::new(egui::Id::unique("poll-create"))
			.frame(
				egui::Frame::new()
					.fill(ctx.global_style().visuals.window_fill)
					.corner_radius(8),
			)
			.show(ctx, |ui| {
				let p = design::palette(ui);
				let width = 480.0_f32.min((ctx.content_rect().width() - 48.0).max(160.0));
				ui.set_width(width);
				ui.spacing_mut().item_spacing.y = 0.0;
				egui::Frame::new().inner_margin(20).show(ui, |ui| {
					ui.set_width(width - 40.0);
					ui.horizontal(|ui| {
						let title_width = (ui.available_width() - 36.0).max(40.0);
						ui.allocate_ui(egui::vec2(title_width, 28.0), |ui| {
							ui.set_width(title_width);
							ui.add(
								egui::Label::new(
									design::semibold(ui, "Create a Poll", 20.0)
										.color(p.text_strong),
								)
								.wrap(),
							);
						});
						if ui
							.add_enabled_ui(!*posting, |ui| {
								icons::button(ui, Icon::Close, 28.0, "Close poll editor")
							})
							.inner
							.clicked()
						{
							close = true;
						}
					});
					ui.add_space(20.0);
					egui::ScrollArea::vertical()
						.max_height(
							(ctx.content_rect().height()
								- if width < 340.0 { 260.0 } else { 200.0 })
							.max(60.0),
						)
						.show(ui, |ui| {
							ui.spacing_mut().item_spacing.y = 8.0;
							ui.add_enabled_ui(!*posting, |ui| {
								ui.label(design::eyebrow(ui, "QUESTION", p.text));
								ui.add(
									egui::TextEdit::multiline(&mut draft.question)
										.hint_text("What would you like to ask?")
										.char_limit(300)
										.desired_rows(1)
										.desired_width(f32::INFINITY)
										.background_color(p.base)
										.margin(12),
								);
								ui.add_space(12.0);
								ui.label(design::eyebrow(ui, "ANSWERS", p.text));
								let mut remove = None;
								let count = draft.answers.len();
								for (i, answer) in draft.answers.iter_mut().enumerate() {
									ui.push_id(i, |ui| {
										ui.horizontal(|ui| {
											ui.spacing_mut().item_spacing.x = 8.0;
											let field_width =
												(ui.available_width() - 36.0).max(80.0);
											egui::Frame::new()
												.fill(p.base)
												.corner_radius(4)
												.inner_margin(10)
												.show(ui, |ui| {
													ui.set_width(field_width - 20.0);
													ui.horizontal(|ui| {
														let mut emoji = answer
															.emoji
															.as_ref()
															.and_then(|e| e.name.clone())
															.unwrap_or_default();
														if ui
															.add(
																egui::TextEdit::singleline(
																	&mut emoji,
																)
																.hint_text("☺")
																.char_limit(32)
																.desired_width(28.0)
																.frame(egui::Frame::NONE),
															)
															.on_hover_text("Optional Unicode emoji")
															.changed()
														{
															answer.emoji = (!emoji
																.trim()
																.is_empty())
															.then_some(model::ReactionEmoji {
																id: None,
																name: Some(emoji),
															});
														}
														ui.add(
															egui::TextEdit::singleline(
																&mut answer.text,
															)
															.hint_text(format!("Answer {}", i + 1))
															.char_limit(55)
															.desired_width(ui.available_width())
															.frame(egui::Frame::NONE),
														);
													});
												});
											if ui
												.add_enabled_ui(count > 2, |ui| {
													icons::button(
														ui,
														Icon::Trash,
														28.0,
														"Remove answer",
													)
												})
												.inner
												.clicked()
											{
												remove = Some(i);
											}
										});
									});
								}
								if let Some(i) = remove {
									draft.answers.remove(i);
								}
								if ui
									.add_enabled(
										draft.answers.len() < 10,
										egui::Button::new(
											RichText::new("+ Add another answer").color(p.muted),
										)
										.fill(p.chat)
										.corner_radius(4)
										.min_size(egui::vec2(ui.available_width() - 36.0, 40.0)),
									)
									.clicked()
								{
									draft.answers.push(Media {
										text: String::new(),
										emoji: None,
									});
								}
								ui.add_space(12.0);
								ui.horizontal(|ui| {
									ui.label(RichText::new("Duration").color(p.text));
									egui::ComboBox::from_id_salt("poll-duration")
										.width(140.0_f32.min(ui.available_width()))
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
						});
					if let Some(error) = state.polls.error {
						ui.add_space(8.0);
						ui.colored_label(p.danger, error);
					}
				});
				egui::Frame::new()
					.fill(p.sidebar)
					.inner_margin(16)
					.show(ui, |ui| {
						ui.set_width(width - 32.0);
						ui.spacing_mut().item_spacing.x = 12.0;
						// Stack the footer at narrow widths so both controls stay reachable.
						if width < 340.0 {
							multiple_answers(ui, &mut draft.multiselect, !*posting);
							ui.add_space(8.0);
						}
						ui.horizontal(|ui| {
							if width >= 340.0 {
								multiple_answers(ui, &mut draft.multiselect, !*posting);
							}

							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									if ui
										.add_enabled_ui(
											!*posting
												&& draft.valid() && state.can_create_poll(*channel),
											|ui| {
												design::button(
													ui,
													if *posting { "Posting…" } else { "Post" },
													design::ButtonKind::Primary,
												)
											},
										)
										.inner
										.clicked()
									{
										action = Some(Action::Create(Box::new(draft.clone())));
										*posting = true;
									}
								},
							);
						});
					});
			});
		close |= !*posting && modal.should_close();

		if close {
			self.draft = None;
		}
		action
	}
}
fn multiple_answers(ui: &mut egui::Ui, checked: &mut bool, enabled: bool) {
	let p = design::palette(ui);
	ui.scope(|ui| {
		ui.spacing_mut().icon_width = 20.0;
		ui.spacing_mut().icon_width_inner = 14.0;
		let widgets = &mut ui.visuals_mut().widgets;
		for widget in [
			&mut widgets.inactive,
			&mut widgets.hovered,
			&mut widgets.active,
			&mut widgets.noninteractive,
		] {
			widget.corner_radius = 4.into();
			widget.bg_stroke = Stroke::new(1.5, p.muted);
			widget.bg_fill = if *checked {
				p.accent
			} else {
				egui::Color32::TRANSPARENT
			};
		}
		ui.add_enabled(
			enabled,
			egui::Checkbox::new(checked, RichText::new("Allow Multiple Answers").size(14.0)),
		);
	});
}

fn duration_label(hours: u16) -> String {
	match hours {
		1 => "1 hour".into(),
		24 => "24 hours".into(),
		168 => "1 week".into(),
		72 => "3 days".into(),
		_ => format!("{hours} hours"),
	}
}

#[cfg(feature = "demo")]
pub fn debug_poll_check(state: &State) {
	for width in [260.0, 900.0] {
		for (light, transparency) in [
			(true, 0),
			(false, 0),
			(true, 50),
			(false, 50),
			(true, 100),
			(false, 100),
		] {
			design::set_window_effects(true, transparency, 0);
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
			let mut card_fill = egui::Color32::TRANSPARENT;
			let mut output = ctx.run_ui(input, |ui| {
				card_fill = design::glass(ui, design::palette(ui).sidebar).0;
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
			assert!(
				output.shapes.iter().any(|shape| matches!(
					&shape.shape,
					egui::Shape::Rect(rect) if rect.fill == card_fill
				)),
				"poll card must use the conversation glass fill"
			);
			assert_eq!(card_fill.a() == 255, transparency == 0);
			output.textures_delta.clear();
		}
	}
	design::set_window_effects(false, 0, 0);
}

#[cfg(test)]
#[path = "polls_tests.rs"]
mod tests;
