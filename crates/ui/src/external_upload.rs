//! A per-file public hosting consent prompt and a reviewable, never automatically sent link.
use crate::{attachments, design, dialog, icons};
use client_core::{MAX_DRAFT_BYTES, State};
use egui::{Context, RichText};
use model::{
	Id,
	public_upload::{Error, Host, eligible},
};

#[derive(Default)]
pub struct ExternalUpload {
	prompt: Option<Prompt>,
	/// The last host chosen this session; new prompts start from it.
	host: Host,
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
	pub host: Host,
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
	result: Option<Result<String, Error>>,
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
		// Prefer a host that accepts the file when the remembered one cannot.
		if !eligible(self.host, &filename, bytes)
			&& let Some(host) = Host::ALL
				.into_iter()
				.find(|host| eligible(*host, &filename, bytes))
		{
			self.host = host;
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
	pub fn complete(&mut self, result: Result<String, Error>) {
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
		if prompt.running {
			ctx.request_repaint_after(std::time::Duration::from_millis(250));
		}
		let host = &mut self.host;
		let accepted = eligible(*host, &prompt.filename, prompt.bytes);
		let mut close = false;
		let modal = dialog::Dialog::new(
			"public-attachment-upload",
			crate::i18n::translate("public-upload-heading"),
		)
		.subtitle(crate::i18n::translate("public-upload-subtitle"))
		.icon(icons::Icon::Link)
		.width(480.0)
		.show(ctx, |body| {
			body.scroll(200.0, |ui| {
				let colors = design::palette(ui);
				ui.spacing_mut().item_spacing.y = 8.0;
				if state.demo {
					design::notice(ui, design::Level::Info, "public-upload-offline");
				}
				file_card(ui, &prompt.filename, prompt.bytes, state.upload_limit());
				ui.add_space(4.0);
				design::section(ui, "public-upload-host", None);
				ui.add_enabled_ui(!prompt.running && prompt.result.is_none(), |ui| {
					for choice in Host::ALL {
						let detail = if eligible(choice, &prompt.filename, prompt.bytes) {
							crate::i18n::translate(host_detail_key(choice))
						} else {
							crate::i18n::translate("public-upload-limits")
						};
						if design::radio_row(ui, *host == choice, choice.name(), Some(&detail))
							.clicked()
						{
							*host = choice;
						}
					}
				});
				ui.add_space(4.0);
				design::notice(ui, design::Level::Warning, "public-upload-privacy");
				if !same_channel {
					design::notice(ui, design::Level::Warning, "public-upload-return");
				}
				match &prompt.result {
					Some(Ok(link)) => {
						if prompt.draft_full {
							design::notice(ui, design::Level::Error, "public-upload-draft-full");
						}
						design::card(ui, |ui| {
							ui.horizontal(|ui| {
								let (rect, _) = ui.allocate_exact_size(
									egui::Vec2::splat(16.0),
									egui::Sense::hover(),
								);
								icons::paint(
									ui.painter(),
									icons::Icon::Check,
									rect,
									colors.positive,
								);
								ui.add(
									egui::Label::new(
										RichText::new(link).color(colors.link).monospace(),
									)
									.truncate(),
								);
							});
						});
						ui.label(
							RichText::new(crate::i18n::translate("public-upload-review"))
								.size(12.0)
								.color(colors.muted),
						);
					}
					Some(Err(error)) => {
						design::notice(ui, design::Level::Error, error_key(*error));
					}
					None if prompt.running => {
						let (sent, total) = prompt.progress.unwrap_or((0, prompt.bytes));
						ui.label(
							RichText::new(crate::i18n::translate_args(
								"public-upload-uploading",
								&[
									("host", host.name()),
									("sent", &attachments::format_size(sent)),
									("total", &attachments::format_size(total)),
								],
							))
							.size(13.0)
							.color(colors.muted),
						);
						ui.add(
							egui::ProgressBar::new(sent as f32 / total.max(1) as f32)
								.desired_height(6.0)
								.fill(colors.accent),
						);
					}
					None => {}
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
					let label = crate::i18n::translate_args(
						"public-upload-upload",
						&[("host", host.name())],
					);
					if ui
						.add_enabled_ui(accepted && same_channel, |ui| {
							dialog::action(ui, &label, dialog::Action::Primary)
						})
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
							host: *host,
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
/// The file being shared: kind glyph, name, size, and whether Discord itself would refuse it.
fn file_card(ui: &mut egui::Ui, filename: &str, bytes: u64, limit: u64) {
	let colors = design::palette(ui);
	let kind = attachments::file_kind(filename, None);
	design::card(ui, |ui| {
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 12.0;
			let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(32.0), egui::Sense::hover());
			icons::paint(ui.painter(), kind.icon(), rect, kind.tint(&colors));
			ui.vertical(|ui| {
				ui.spacing_mut().item_spacing.y = 2.0;
				ui.add(
					egui::Label::new(design::medium(ui, filename, 15.0).color(colors.text_strong))
						.truncate(),
				);
				let size = attachments::format_size(bytes);
				let (text, color) = if bytes > limit {
					(
						crate::i18n::translate_args(
							"public-upload-over-limit",
							&[("size", &size), ("limit", &attachments::format_size(limit))],
						),
						colors.warning,
					)
				} else {
					(size, colors.muted)
				};
				ui.label(RichText::new(text).size(12.0).color(color));
			});
		});
	});
}
fn host_detail_key(host: Host) -> &'static str {
	match host {
		Host::ZeroX0 => "public-upload-host-zerox0",
		Host::Catbox => "public-upload-host-catbox",
	}
}
fn error_key(error: Error) -> &'static str {
	match error {
		Error::Cancelled => "public-upload-error-cancelled",
		Error::Prepare => "public-upload-error-prepare",
		Error::Changed => "public-upload-error-changed",
		Error::Unsupported => "public-upload-limits",
		Error::Failed => "public-upload-error-failed",
		Error::Rejected => "public-upload-error-rejected",
		Error::Incomplete => "public-upload-error-incomplete",
		Error::ResponseLimit => "public-upload-error-response-limit",
		Error::Interrupted => "public-upload-error-interrupted",
		Error::InvalidLink => "public-upload-error-invalid-link",
		Error::Busy => "public-upload-error-busy",
		Error::ConversationChanged => "public-upload-error-conversation",
		Error::SelectionChanged => "public-upload-error-selection",
		Error::MissingSelection => "public-upload-error-missing",
	}
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
	if draft.chars().count() + added > state.content_limit()
		|| state.draft_bytes() + added > MAX_DRAFT_BYTES
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
	use client_core::MAX_CONTENT;
	#[test]
	fn typed_public_upload_errors_have_english_and_czech_messages() {
		use crate::i18n::Language;
		for error in [
			Error::Cancelled,
			Error::Prepare,
			Error::Changed,
			Error::Unsupported,
			Error::Failed,
			Error::Rejected,
			Error::Incomplete,
			Error::ResponseLimit,
			Error::Interrupted,
			Error::InvalidLink,
			Error::Busy,
			Error::ConversationChanged,
			Error::SelectionChanged,
			Error::MissingSelection,
		] {
			let key = error_key(error);
			let english = Language::English.text(key);
			let czech = Language::Czech.text(key);
			assert!(!english.starts_with("Unknown localization"), "{error:?}");
			assert!(!czech.starts_with("Unknown localization"), "{error:?}");
			assert_ne!(english, czech, "{error:?}");
		}
		let ctx = Context::default();
		let mut state = test_support::demo_state();
		let mut view = ExternalUpload::default();
		view.open(
			state.generation,
			state.selected.unwrap(),
			0,
			None,
			"synthetic.pdf".into(),
			1,
		);
		view.complete(Err(Error::InvalidLink));
		let _ = frame(&ctx, &mut view, &mut state, vec![]);
		let labels = frame(&ctx, &mut view, &mut state, vec![]);
		assert!(
			labels
				.iter()
				.any(|(label, _)| label == &Language::English.text(error_key(Error::InvalidLink)))
		);
	}
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
			.find(|(label, _)| label == "Upload to 0x0.st")
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
