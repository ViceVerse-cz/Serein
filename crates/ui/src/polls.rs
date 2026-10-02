//! Native poll cards and a bounded creation dialog.
use crate::{
	avatars::Avatars,
	design, dialog,
	emoji_picker::Picker,
	i18n::{translate, translate_args},
	icons::{self, Icon},
};
use client_core::{
	State,
	polls::{Action, now_ms},
};
use egui::{RichText, Stroke};
use model::{
	Id, Message, ReactionEmoji,
	polls::{Create, MAX_ANSWERS, Media},
};

#[derive(Default)]
pub struct Cards {
	selection: Option<(Id, Id, Vec<u32>, bool)>,
}
impl Cards {
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		message: &Message,
		avatars: &mut Avatars,
	) -> Option<Action> {
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
			.fill(design::message_card_fill(ui, colors.sidebar))
			.corner_radius(8)
			.inner_margin(16)
			.show(ui, |ui| {
				ui.set_width(width);
				ui.spacing_mut().item_spacing.y = 8.0;
				ui.label(
					crate::design::semibold(ui, &poll.question, 18.0).color(colors.text_strong),
				);
				ui.label(
					RichText::new(translate(if ended {
						"polls-card-ended"
					} else if poll.multiselect {
						"polls-card-select-many"
					} else {
						"polls-card-select-one"
					}))
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
					let emoji =
						answer.media.emoji.as_ref().and_then(|emoji| {
							emoji_image(ui.ctx(), avatars, emoji, 24.0, state.demo)
						});
					let response = ui
						.add_enabled_ui(results || enabled, |ui| {
							answer_row(
								ui,
								answer,
								emoji,
								checked,
								results,
								poll.results_known,
								total,
							)
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
					translate(if poll.finalized {
						"polls-card-final-results"
					} else {
						"polls-card-awaiting-results"
					})
				} else {
					remaining.map_or_else(
						|| translate("polls-card-in-progress"),
						|m| {
							if m >= 60 {
								translate_args(
									"polls-card-hours-left",
									&[("hours", &((m + 59) / 60).to_string())],
								)
							} else {
								translate_args(
									"polls-card-minutes-left",
									&[("minutes", &m.to_string())],
								)
							}
						},
					)
				};
				let footer = if poll.results_known {
					format!("{} • {footer}", vote_label(poll.votes()))
				} else {
					format!("{} • {footer}", translate("polls-card-results-not-loaded"))
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
						"polls-card-back-to-voting"
					} else if results {
						"polls-card-refresh-results"
					} else {
						"polls-card-show-results"
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
								design::button(
									ui,
									"polls-card-remove-vote",
									design::ButtonKind::Outline,
								)
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
							design::button(ui, "polls-card-vote", design::ButtonKind::Primary)
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
							ui.label(translate("polls-card-end-confirm"));
							if ui
								.add_enabled(
									enabled,
									egui::Button::new(translate("polls-card-end-now")),
								)
								.clicked()
							{
								action = Some(Action::End);
								ui.close();
							}
						})
						.response
						.on_hover_text(translate("polls-card-end-poll"));
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
	translate_args(
		if votes == 1 {
			"polls-card-vote-count-one"
		} else {
			"polls-card-vote-count-many"
		},
		&[("count", &votes.to_string())],
	)
}

/// Bundled Twemoji artwork for Unicode emoji, the bounded image cache for custom ones.
fn emoji_image(
	ctx: &egui::Context,
	avatars: &mut Avatars,
	emoji: &ReactionEmoji,
	size: f32,
	demo: bool,
) -> Option<egui::Image<'static>> {
	match emoji.id {
		Some(id) => avatars.custom_image(ctx, id, size, demo),
		None => crate::emoji::image(ctx, emoji.name.as_deref()?, size),
	}
}

fn answer_row(
	ui: &mut egui::Ui,
	answer: &model::polls::Answer,
	emoji_image: Option<egui::Image<'static>>,
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
		design::message_card_fill(ui, if hot && !results { p.hover } else { p.raised }),
	);
	if show_tally && fraction > 0.0 {
		let bar = rect.with_max_x(rect.left() + width * fraction);
		painter.rect_filled(
			bar,
			8,
			design::message_card_fill(
				ui,
				design::mix(
					p.raised,
					if checked { p.accent } else { p.text },
					if checked { 0.3 } else { 0.08 },
				),
			),
		);
	} else if checked {
		painter.rect_filled(
			rect,
			8,
			design::message_card_fill(ui, design::mix(p.raised, p.accent, 0.18)),
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

/// Durations Discord's poll creator offers, in hours.
const DURATIONS: [u16; 6] = [1, 4, 8, 24, 72, 168];
/// Height shared by the answer fields and the "add another answer" row.
const FIELD: f32 = 40.0;
/// Width the trailing remove-answer control and its gap reserve beside each field.
const REMOVE: f32 = 40.0;

struct Draft {
	channel: Id,
	poll: Create,
	created: u64,
	posting: bool,
	/// Answer whose emoji the picker is choosing.
	picking: Option<usize>,
}

#[derive(Default)]
pub struct Creator {
	draft: Option<Draft>,
	picker: Picker,
}
impl Creator {
	pub fn open(&mut self, state: &State, channel: Id) {
		if self.draft.is_none() {
			self.draft = Some(Draft {
				channel,
				poll: Create {
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
				created: state.polls.created,
				posting: false,
				picking: None,
			});
		}
	}
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		avatars: &mut Avatars,
	) -> Option<Action> {
		let Self {
			draft: slot,
			picker,
		} = self;
		let draft = slot.as_mut()?;
		if state.selected != Some(draft.channel) || state.polls.created != draft.created {
			*slot = None;
			picker.dismiss(state, &mut Vec::new());
			return None;
		}
		if draft.posting && state.polls.pending.is_none() {
			draft.posting = false;
		}
		let posting = draft.posting;
		let narrow = ctx.content_rect().width() < 380.0;
		let mut action = None;
		let response = dialog::Dialog::new("poll-create", translate("polls-creator-title"))
			.width(480.0)
			.show(ctx, |body| {
				body.scroll(if narrow { 250.0 } else { 210.0 }, |ui| {
					ui.add_enabled_ui(!posting, |ui| editor(ui, state, avatars, picker, draft));
				});
				if let Some(error) = state.polls.error {
					body.content(|ui| {
						ui.add_space(8.0);
						design::notice(ui, design::Level::Error, error);
					});
				}
				body.footer(|ui| {
					let ready =
						!posting && draft.poll.valid() && state.can_create_poll(draft.channel);
					if ui
						.add_enabled_ui(ready, |ui| {
							design::button(
								ui,
								if posting {
									"polls-creator-posting"
								} else {
									"polls-creator-post"
								},
								design::ButtonKind::Primary,
							)
						})
						.inner
						.clicked()
					{
						action = Some(Action::Create(Box::new(draft.poll.clone())));
						draft.posting = true;
					}
					// The checkbox takes the rest of the strip from the left edge and wraps
					// rather than pushing the Post button out of narrow dialogs.
					ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
						multiple_answers(ui, &mut draft.poll.multiselect, !posting);
					});
				});
			});
		if response.close && !posting {
			*slot = None;
			picker.dismiss(state, &mut Vec::new());
		}
		action
	}
}

/// Question, answer rows and duration, laid out like Discord's poll creator.
fn editor(
	ui: &mut egui::Ui,
	state: &mut State,
	avatars: &mut Avatars,
	picker: &mut Picker,
	draft: &mut Draft,
) {
	ui.spacing_mut().item_spacing.y = 8.0;
	design::label(ui, "polls-creator-question");
	design::input(
		ui,
		egui::TextEdit::multiline(&mut draft.poll.question)
			.hint_text(translate("polls-creator-question-hint"))
			.char_limit(300)
			.desired_rows(1),
	);
	ui.add_space(12.0);
	design::label(ui, "polls-creator-answers");
	if draft.picking.is_some() && !picker.choosing() {
		draft.picking = None;
	}
	let removable = draft.poll.answers.len() > 2;
	let mut remove = None;
	let mut trigger = None;
	for (index, answer) in draft.poll.answers.iter_mut().enumerate() {
		let open = draft.picking == Some(index);
		let row = ui
			.push_id(index, |ui| {
				answer_field(ui, answer, index, removable, open, (avatars, state.demo))
			})
			.inner;
		if row.emoji.clicked() {
			if open {
				picker.dismiss(state, &mut Vec::new());
				draft.picking = None;
			} else {
				picker.open_choice(state, draft.channel, &row.emoji, answer.emoji.is_some());
				draft.picking = Some(index);
			}
		}
		if draft.picking == Some(index) {
			trigger = Some(row.emoji);
		}
		if row.remove.clicked() {
			remove = Some(index);
		}
	}
	if let Some(index) = remove {
		draft.poll.answers.remove(index);
		if draft.picking.take().is_some() {
			picker.dismiss(state, &mut Vec::new());
		}
		trigger = None;
	}
	if draft.poll.answers.len() < MAX_ANSWERS
		&& add_answer(ui, (ui.available_width() - REMOVE).max(80.0)).clicked()
	{
		draft.poll.answers.push(Media {
			text: String::new(),
			emoji: None,
		});
	}
	ui.add_space(12.0);
	design::label(ui, "polls-creator-duration");
	duration_select(ui, &mut draft.poll.duration);
	if let (Some(index), Some(trigger)) = (draft.picking, trigger) {
		if let Some(choice) = picker.show_choice(ui, state, draft.channel, avatars, &trigger)
			&& let Some(answer) = draft.poll.answers.get_mut(index)
		{
			answer.emoji =
				choice.filter(|emoji| emoji.valid() && emoji.id.is_none_or(|id| id.0 != 0));
		}
		if !picker.choosing() {
			draft.picking = None;
		}
	}
}

struct AnswerRow {
	emoji: egui::Response,
	remove: egui::Response,
}

/// One answer input: the emoji button sits inside the field, the remove control beside it.
fn answer_field(
	ui: &mut egui::Ui,
	answer: &mut Media,
	index: usize,
	removable: bool,
	open: bool,
	(avatars, demo): (&mut Avatars, bool),
) -> AnswerRow {
	let p = design::palette(ui);
	ui.horizontal(|ui| {
		ui.spacing_mut().item_spacing.x = 8.0;
		let width = (ui.available_width() - REMOVE).max(80.0);
		let field = egui::Frame::new()
			.fill(p.base)
			.corner_radius(8)
			.inner_margin(egui::Margin {
				left: 4,
				right: 12,
				top: 4,
				bottom: 4,
			})
			.show(ui, |ui| {
				ui.set_width(width - 16.0);
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					let emoji = emoji_button(ui, answer.emoji.as_ref(), open, avatars, demo);
					let text = ui.add(
						egui::TextEdit::singleline(&mut answer.text)
							.hint_text(translate_args(
								"polls-creator-answer-hint",
								&[("number", &(index + 1).to_string())],
							))
							.char_limit(55)
							.desired_width(ui.available_width())
							.frame(egui::Frame::NONE),
					);
					(emoji, text)
				})
				.inner
			});
		let (emoji, text) = field.inner;
		let stroke = if text.has_focus() {
			Stroke::new(2.0, p.accent)
		} else {
			Stroke::new(1.0, p.border)
		};
		ui.painter()
			.rect_stroke(field.response.rect, 8, stroke, egui::StrokeKind::Inside);
		let remove = ui
			.add_enabled_ui(removable, |ui| {
				icons::button(ui, Icon::Trash, 32.0, "polls-creator-remove-answer")
			})
			.inner;
		AnswerRow { emoji, remove }
	})
	.inner
}

/// The answer's Twemoji (or custom emoji) artwork, or a smiley inviting one.
fn emoji_button(
	ui: &mut egui::Ui,
	emoji: Option<&ReactionEmoji>,
	open: bool,
	avatars: &mut Avatars,
	demo: bool,
) -> egui::Response {
	let p = design::palette(ui);
	let label = translate(if emoji.is_some() {
		"polls-creator-change-emoji"
	} else {
		"polls-creator-add-emoji"
	});
	let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(32.0), egui::Sense::click());
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
	let hot = ui.is_enabled() && (response.hovered() || response.has_focus());
	if hot || open {
		ui.painter().rect_filled(rect, 6, p.hover);
	}
	let art = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(22.0));
	match emoji {
		Some(emoji) => match emoji_image(ui.ctx(), avatars, emoji, 22.0, demo) {
			Some(image) => image.paint_at(ui, art),
			// Outside the bundled set: the raw glyph keeps the choice visible.
			None if emoji.id.is_none() => {
				ui.painter().text(
					rect.center(),
					egui::Align2::CENTER_CENTER,
					emoji.label(),
					egui::FontId::proportional(18.0),
					p.text_strong,
				);
			}
			// Custom artwork still loading.
			None => {}
		},
		None => icons::paint(
			ui.painter(),
			Icon::Smile,
			art,
			if hot || open { p.text_strong } else { p.muted },
		),
	}
	response.on_hover_text(label)
}

/// Row under the answers, as wide as an answer field, that appends an empty answer.
fn add_answer(ui: &mut egui::Ui, width: f32) -> egui::Response {
	let p = design::palette(ui);
	let label = translate("polls-creator-add-answer");
	let (rect, response) = ui.allocate_exact_size(egui::vec2(width, FIELD), egui::Sense::click());
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
	let hot = ui.is_enabled() && (response.hovered() || response.has_focus());
	let color = if hot { p.text_strong } else { p.muted };
	let painter = ui.painter();
	painter.rect(
		rect,
		8,
		if hot {
			p.hover
		} else {
			egui::Color32::TRANSPARENT
		},
		if response.has_focus() {
			Stroke::new(2.0, p.accent)
		} else {
			Stroke::new(1.0, p.border)
		},
		egui::StrokeKind::Inside,
	);
	// Centred where the emoji button sits inside an answer field, so the glyphs line up.
	icons::paint(
		painter,
		Icon::Plus,
		egui::Rect::from_center_size(
			egui::pos2(rect.left() + 20.0, rect.center().y),
			egui::Vec2::splat(18.0),
		),
		color,
	);
	let mut job = egui::text::LayoutJob::simple_singleline(
		label,
		egui::FontId::new(14.0, design::medium_family(ui.ctx())),
		color,
	);
	job.wrap = egui::text::TextWrapping::truncate_at_width((rect.width() - 52.0).max(8.0));
	let galley = painter.layout_job(job);
	painter.galley(
		egui::pos2(rect.left() + 42.0, rect.center().y - galley.size().y * 0.5),
		galley,
		color,
	);
	response
}

/// Duration dropdown styled like the dialog's text inputs.
fn duration_select(ui: &mut egui::Ui, duration: &mut u16) {
	let p = design::palette(ui);
	ui.scope(|ui| {
		ui.spacing_mut().button_padding = egui::vec2(12.0, 9.0);
		ui.spacing_mut().interact_size.y = 38.0;
		let widgets = &mut ui.visuals_mut().widgets;
		for (widget, stroke) in [
			(&mut widgets.inactive, Stroke::new(1.0, p.border)),
			(&mut widgets.hovered, Stroke::new(1.0, p.muted)),
			(&mut widgets.active, Stroke::new(2.0, p.accent)),
			(&mut widgets.open, Stroke::new(2.0, p.accent)),
		] {
			widget.weak_bg_fill = p.base;
			widget.bg_fill = p.base;
			widget.bg_stroke = stroke;
			widget.corner_radius = 8.into();
			widget.expansion = 0.0;
		}
		egui::ComboBox::from_id_salt("poll-duration")
			.width(ui.available_width().min(220.0))
			.selected_text(RichText::new(duration_label(*duration)).color(p.text_strong))
			.show_ui(ui, |ui| {
				for hours in DURATIONS {
					ui.selectable_value(duration, hours, duration_label(hours));
				}
			});
	});
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
			egui::Checkbox::new(
				checked,
				RichText::new(translate("polls-creator-multiple-answers")).size(14.0),
			),
		);
	});
}

fn duration_label(hours: u16) -> String {
	match hours {
		1 => translate("polls-duration-1-hour"),
		4 => translate("polls-duration-4-hours"),
		8 => translate("polls-duration-8-hours"),
		24 => translate("polls-duration-24-hours"),
		72 => translate("polls-duration-3-days"),
		168 => translate("polls-duration-1-week"),
		_ => translate_args("polls-duration-hours", &[("hours", &hours.to_string())]),
	}
}

#[cfg(any(feature = "demo", test))]
struct WindowEffectsRestore((bool, u8, u8));

#[cfg(any(feature = "demo", test))]
impl Drop for WindowEffectsRestore {
	fn drop(&mut self) {
		let (enabled, transparency, blur) = self.0;
		design::set_window_effects(enabled, transparency, blur);
	}
}

#[cfg(feature = "demo")]
pub fn debug_poll_check(state: &State) {
	let _restore = WindowEffectsRestore(design::default_window_effects());
	for width in [260.0, 900.0] {
		for (light, transparency) in [
			(true, 0),
			(false, 0),
			(true, 15),
			(false, 15),
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
				card_fill = design::message_card_fill(ui, design::palette(ui).sidebar);
				let chat = design::window_palette(ui).chat;
				if transparency > 0 {
					// Card, answer and result coats must not turn translucent chat opaque.
					assert!(card_fill.a() <= 32);
					assert!(
						chat.blend(card_fill).blend(card_fill).blend(card_fill).a()
							<= chat.a().saturating_add(24)
					);
					if transparency == 100 {
						assert_eq!(card_fill.a(), 0);
					}
				}
				ui.set_width(width - 16.0);
				let mut cards = Cards::default();
				let mut avatars = Avatars::default();
				for message in state.timeline.iter() {
					cards.show(ui, state, message, &mut avatars);
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
				"poll card must use the conversation tint"
			);
			assert_eq!(card_fill.a() == 255, transparency == 0);
			output.textures_delta.clear();
		}
	}
}

#[cfg(test)]
#[path = "polls_tests.rs"]
mod tests;
