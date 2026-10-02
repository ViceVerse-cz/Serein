//! A per-file public hosting consent prompt and a reviewable, never automatically sent link.
use crate::{design, dialog};
use client_core::{MAX_CONTENT, MAX_DRAFT_BYTES, State};
use egui::{Context, RichText};
use model::Id;

#[derive(Default)]
pub struct ExternalUpload {
	prompt: Option<Prompt>,
	pub request: Option<Request>,
	pub cancel_requested: bool,
}
pub struct Request {
	pub generation: u64,
	pub channel: Id,
	pub index: usize,
	pub key: Option<u64>,
	pub filename: String,
	pub bytes: u64,
}
struct Prompt {
	generation: u64,
	channel: Id,
	index: usize,
	key: Option<u64>,
	filename: String,
	bytes: u64,
	running: bool,
	progress: Option<(u64, u64)>,
	result: Option<Result<String, &'static str>>,
	draft_full: bool,
}
impl ExternalUpload {
	pub fn has_unsent(&self) -> bool {
		self.prompt
			.as_ref()
			.is_some_and(|prompt| prompt.running || matches!(prompt.result, Some(Ok(_))))
	}
	#[allow(clippy::too_many_arguments)]
	pub fn open(
		&mut self,
		generation: u64,
		channel: Id,
		index: usize,
		key: Option<u64>,
		filename: String,
		bytes: u64,
	) {
		if self.prompt.as_ref().is_some_and(|p| p.running) {
			return;
		}
		self.prompt = Some(Prompt {
			generation,
			channel,
			index,
			key,
			filename,
			bytes,
			running: false,
			progress: None,
			result: None,
			draft_full: false,
		});
	}
	pub fn progress(&mut self, progress: Option<(u64, u64)>) {
		if let Some(prompt) = &mut self.prompt {
			prompt.progress = progress;
		}
	}
	pub fn complete(&mut self, result: Result<String, &'static str>) {
		if let Some(prompt) = &mut self.prompt {
			prompt.running = false;
			prompt.result = Some(result);
		}
	}
	pub fn show(&mut self, ctx: &Context, state: &mut State, draft_changes: &mut Vec<Id>) {
		let Some(prompt) = &mut self.prompt else {
			return;
		};
		if state.generation != prompt.generation || state.user.is_none() {
			self.cancel_requested = true;
			self.prompt = None;
			self.request = None;
			return;
		}
		let same_channel = state.selected == Some(prompt.channel) && state.can_send(prompt.channel);
		if !same_channel && prompt.running {
			self.cancel_requested = true;
		}
		let mut close = false;
		let modal = dialog::Dialog::new(
			"public-attachment-upload",
			crate::i18n::translate("public-upload-heading"),
		)
		.subtitle(crate::i18n::translate("public-upload-subtitle"))
		.width(460.0)
		.show(ctx, |body| {
			body.scroll(200.0, |ui| {
				let colors = design::palette(ui);
				if state.demo {
					ui.colored_label(
						colors.warning,
						crate::i18n::translate("public-upload-offline"),
					);
				}
				ui.label(RichText::new(&prompt.filename).color(colors.text_strong));
				ui.label(crate::i18n::translate_args(
					"public-upload-size",
					&[("size", &format!("{:.1}", prompt.bytes as f64 / 1_000_000.0))],
				));
				ui.add_space(8.0);
				ui.label(crate::i18n::translate("public-upload-privacy"));
				ui.label(crate::i18n::translate("public-upload-retention"));
				ui.label(crate::i18n::translate("public-upload-review"));
				if !same_channel {
					ui.colored_label(
						colors.warning,
						crate::i18n::translate("public-upload-return"),
					);
				}
				if let Some(result) = &prompt.result {
					match result {
						Ok(link) => {
							if prompt.draft_full {
								ui.colored_label(
									colors.danger,
									crate::i18n::translate("public-upload-draft-full"),
								);
							}
							ui.label(RichText::new(link).color(colors.link));
						}
						Err(error) => {
							ui.colored_label(colors.danger, *error);
						}
					}
				} else if prompt.running {
					if let Some((sent, total)) = prompt.progress {
						ui.add(
							egui::ProgressBar::new(sent as f32 / total.max(1) as f32)
								.show_percentage(),
						);
					} else {
						ui.label(crate::i18n::translate("public-upload-preparing"));
					}
				} else if !eligible(&prompt.filename, prompt.bytes) {
					ui.colored_label(
						colors.danger,
						crate::i18n::translate("public-upload-limits"),
					);
				}
			});
			body.footer(|ui| {
				if let Some(Ok(link)) = &prompt.result {
					if ui
						.add_enabled_ui(same_channel, |ui| {
							dialog::action(ui, "public-upload-add", dialog::Action::Primary)
						})
						.inner
						.clicked()
					{
						if append_link(state, prompt.channel, link) {
							draft_changes.push(prompt.channel);
							close = true;
						} else {
							prompt.draft_full = true;
						}
					}
					if dialog::action(ui, "public-upload-copy", dialog::Action::Neutral).clicked() {
						ctx.copy_text(link.clone());
					}
					close |= dialog::action(ui, "public-upload-close", dialog::Action::Neutral)
						.clicked();
				} else if prompt.result.is_some() {
					close |= dialog::action(ui, "public-upload-close", dialog::Action::Neutral)
						.clicked();
				} else if prompt.running {
					if dialog::action(ui, "public-upload-cancel-upload", dialog::Action::Neutral)
						.clicked()
					{
						self.cancel_requested = true;
					}
				} else {
					if ui
						.add_enabled_ui(
							eligible(&prompt.filename, prompt.bytes) && same_channel,
							|ui| {
								dialog::action(ui, "public-upload-upload", dialog::Action::Primary)
							},
						)
						.inner
						.clicked()
					{
						prompt.running = true;
						self.request = Some(Request {
							generation: prompt.generation,
							channel: prompt.channel,
							index: prompt.index,
							key: prompt.key,
							filename: prompt.filename.clone(),
							bytes: prompt.bytes,
						});
					}
					close |= dialog::action(ui, "public-upload-cancel", dialog::Action::Neutral)
						.clicked();
				}
			});
		});
		if modal.close {
			if prompt.running {
				self.cancel_requested = true;
			} else if prompt.result.is_none() {
				close = true;
			}
		}

		if close {
			self.prompt = None;
		}
	}
}
// Kept in the UI to enable consent buttons without bringing the HTTP adapter into rendering.
fn eligible(filename: &str, bytes: u64) -> bool {
	let extension = filename
		.rsplit_once('.')
		.map_or("", |(_, ext)| ext)
		.to_ascii_lowercase();
	bytes > 0
		&& bytes <= 200_000_000
		&& !matches!(extension.as_str(), "exe" | "scr" | "cpl" | "jar")
		&& !extension.starts_with("doc")
		&& !(extension == "gif" && bytes > 20_000_000)
}
fn append_link(state: &mut State, channel: Id, link: &str) -> bool {
	let draft = state.drafts.get(&channel).map_or("", String::as_str);
	let separator = if draft.is_empty() || draft.ends_with(char::is_whitespace) {
		""
	} else {
		"\n"
	};
	let added = separator.len() + link.len();
	if !state.drafts.contains_key(&channel) && state.drafts.len() >= 64 {
		return false;
	}
	if draft.chars().count() + added > MAX_CONTENT || state.draft_bytes() + added > MAX_DRAFT_BYTES
	{
		return false;
	}
	let mut updated = String::with_capacity(draft.len() + added);
	updated.push_str(draft);
	updated.push_str(separator);
	updated.push_str(link);
	state.drafts.insert(channel, updated);
	true
}
#[cfg(test)]
mod tests {
	use super::*;
	fn frame(
		ctx: &Context,
		view: &mut ExternalUpload,
		state: &mut State,
		events: Vec<egui::Event>,
	) -> Vec<(String, egui::Rect)> {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(760.0, 520.0),
				)),
				events,
				..Default::default()
			},
			|_| {
				view.show(ctx, state, &mut Vec::new());
			},
		);
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.text().to_owned(),
					egui::Rect::from_min_size(text.pos, text.galley.size()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		let mut labels = Vec::new();
		for shape in &output.shapes {
			collect(&shape.shape, &mut labels);
		}
		output.drop_without_applying_deltas();
		labels
	}
	#[test]
	fn public_upload_requires_an_explicit_click_and_keeps_result_for_review() {
		let ctx = Context::default();
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let mut view = ExternalUpload::default();
		view.open(
			state.generation,
			channel,
			2,
			None,
			"synthetic.pdf".into(),
			50_000_000,
		);
		let _ = frame(&ctx, &mut view, &mut state, vec![]);
		let labels = frame(&ctx, &mut view, &mut state, vec![]);
		assert!(view.request.is_none());
		assert!(!view.has_unsent());
		let button = labels
			.iter()
			.find(|(label, _)| label == "Upload publicly to Catbox")
			.unwrap()
			.1
			.center();
		for pressed in [true, false] {
			frame(
				&ctx,
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(button),
					egui::Event::PointerButton {
						pos: button,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: Default::default(),
					},
				],
			);
		}
		let request = view.request.take().unwrap();
		assert_eq!(request.index, 2);
		assert_eq!(request.channel, channel);
		assert!(view.has_unsent());
		view.complete(Ok("https://files.catbox.moe/synthetic.pdf".into()));
		assert!(
			!state
				.drafts
				.values()
				.any(|text| text.contains("files.catbox"))
		);
		state.selected = None;
		let _ = frame(&ctx, &mut view, &mut state, vec![]);
		let labels = frame(&ctx, &mut view, &mut state, vec![]);
		assert!(labels.iter().any(|(label, _)| label == "Copy link"));
		assert!(view.has_unsent());
		state.selected = Some(channel);
		state.drafts.insert(channel, "a".repeat(MAX_CONTENT));
		let _ = frame(&ctx, &mut view, &mut state, vec![]);
		let labels = frame(&ctx, &mut view, &mut state, vec![]);
		let add = labels
			.iter()
			.find(|(label, _)| label == "Add to draft")
			.unwrap()
			.1
			.center();
		for pressed in [true, false] {
			frame(
				&ctx,
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(add),
					egui::Event::PointerButton {
						pos: add,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: Default::default(),
					},
				],
			);
		}
		let labels = frame(&ctx, &mut view, &mut state, vec![]);
		assert!(
			labels
				.iter()
				.any(|(label, _)| label.starts_with("Draft is full"))
		);
		assert!(view.has_unsent());
		assert_eq!(state.drafts[&channel].len(), MAX_CONTENT);
		state.logout();
		frame(&ctx, &mut view, &mut state, vec![]);
		assert!(!view.has_unsent());
	}

	#[test]
	fn link_insertion_preserves_draft_and_limits() {
		let mut state = State::default();
		let channel = Id(1);
		state.drafts.insert(channel, "Keep this text".into());
		assert!(append_link(
			&mut state,
			channel,
			"https://files.catbox.moe/a.png"
		));
		assert_eq!(
			state.drafts[&channel],
			"Keep this text\nhttps://files.catbox.moe/a.png"
		);
		state.drafts.insert(channel, "a".repeat(MAX_CONTENT));
		assert!(!append_link(
			&mut state,
			channel,
			"https://files.catbox.moe/a.png"
		));
		assert_eq!(state.drafts[&channel].len(), MAX_CONTENT);
	}
}
