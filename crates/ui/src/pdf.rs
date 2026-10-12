//! One native PDF modal. The desktop owns fetching/parsing/rasterization; this view
//! only uploads a bounded worker-produced image and emits latest-value commands.
use client_core::State;
use model::{Attachment, Id, Message};
use std::{
	hash::{Hash, Hasher},
	sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	},
};

static DOCUMENT: AtomicU64 = AtomicU64::new(1);

pub const MAX_PDF_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PDF_PAGES: u32 = 256;
pub const MAX_PDF_EDGE: usize = 1536;
pub const MAX_PDF_PIXELS: usize = 1536 * 1024;
const DIALOG_WIDTH: f32 = 900.;
const MAX_ZOOM: f32 = 2.;

fn page_height(viewport_height: f32) -> f32 {
	if viewport_height.is_finite() {
		(viewport_height - 290.).clamp(40., 620.)
	} else {
		40.
	}
}
/// Physical-pixel envelope for Fit through 2× zoom, sampled only on explicit
/// page requests. Resizes/zoom/scroll reuse the texture instead of rendering.
pub fn pdf_render_target(ctx: &egui::Context) -> [u16; 2] {
	render_target(ctx.content_rect().size(), ctx.pixels_per_point())
}
fn render_target(viewport: egui::Vec2, pixels_per_point: f32) -> [u16; 2] {
	let density = if pixels_per_point.is_finite() && pixels_per_point > 0. {
		pixels_per_point
	} else {
		1.
	};
	// Match the shared dialog's viewport clamp and content padding.
	let viewport_width = if viewport.x.is_finite() {
		viewport.x
	} else {
		232.
	};
	let width = DIALOG_WIDTH.min(viewport_width - 32.).max(200.) - 2. * crate::dialog::PAD;
	let height = page_height(viewport.y);
	[width, height].map(|points| {
		(points * density * MAX_ZOOM)
			.ceil()
			.clamp(1., MAX_PDF_EDGE as f32) as u16
	})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfError {
	Unavailable,
	Limit,
	Invalid,
	Password,
	TimedOut,
	Renderer,
}
impl PdfError {
	fn key(self) -> &'static str {
		match self {
			Self::Unavailable => "pdf-error-unavailable",
			Self::Limit => "pdf-error-limit",
			Self::Invalid => "pdf-error-invalid",
			Self::Password => "pdf-error-password",
			Self::TimedOut => "pdf-error-timeout",
			Self::Renderer => "pdf-error-renderer",
		}
	}
}
#[derive(Clone)]
pub struct PdfRequest {
	pub document: u64,
	pub request: u64,
	pub page: u32,
	pub attachment: Attachment,
}
pub enum PdfCommand {
	Render(Box<PdfRequest>),
	Close,
}
#[derive(Clone, Debug)]
pub(crate) enum Source {
	Message,
	Component(Vec<u64>),
	Search {
		request: u64,
		fingerprint: u64,
		timeline: Option<u64>,
	},
}
struct Active {
	channel: Id,
	message: Id,
	fingerprint: u64,
	source: Source,
	document: u64,
	request: u64,
	page: u32,
	pages: Option<u32>,
	attachment: Attachment,
}
#[derive(Default)]
pub struct PdfUi {
	active: Option<Active>,
	scope: Option<(u64, Option<Id>, Option<Id>)>,
	serial: u64,
	pub command: Option<PdfCommand>,
	error: Option<PdfError>,
	texture: Option<egui::TextureHandle>,
	zoom: f32,
}
pub(crate) fn fingerprint(message: &Message) -> u64 {
	let mut h = std::collections::hash_map::DefaultHasher::new();
	// Source/spoiler changes invalidate the document, but the broad revision
	// counter also tracks reactions and must not retire a valid preview.
	message.content.hash(&mut h);
	message.attachments.hash(&mut h);
	message.embeds.hash(&mut h);
	message.components.hash(&mut h);
	message.embeds_suppressed.hash(&mut h);
	h.finish()
}
pub(crate) fn hit_fingerprint(hit: &model::SearchHit) -> u64 {
	let mut h = std::collections::hash_map::DefaultHasher::new();
	hit.channel.hash(&mut h);
	hit.excerpt.hash(&mut h);
	hit.attachments.hash(&mut h);
	hit.embeds.hash(&mut h);
	h.finish()
}
pub fn valid_pdf_dimensions(width: usize, height: usize) -> bool {
	width > 0
		&& height > 0
		&& width <= MAX_PDF_EDGE
		&& height <= MAX_PDF_EDGE
		&& width
			.checked_mul(height)
			.is_some_and(|n| n <= MAX_PDF_PIXELS)
}
impl PdfUi {
	pub fn is_open(&self) -> bool {
		self.active.is_some()
	}
	pub(crate) fn serial(&self) -> u64 {
		self.serial
	}
	pub fn identity(&self) -> Option<(u64, u64)> {
		self.active.as_ref().map(|a| (a.document, a.request))
	}
	pub fn close(&mut self) {
		if self.active.take().is_some() {
			self.command = Some(PdfCommand::Close);
		}
		self.texture = None;
		self.error = None;
		self.zoom = 1.;
	}
	pub(crate) fn bind(&mut self, state: &State) {
		let scope = (
			state.generation,
			state.user.as_ref().map(|u| u.id),
			state.selected,
		);
		if self.scope != Some(scope) {
			self.close();
			self.scope = Some(scope);
		}
	}
	pub(crate) fn open(&mut self, message: &Message, attachment: &Attachment) {
		if !attachment.is_pdf() || !model::valid_attachments(std::slice::from_ref(attachment)) {
			return;
		}
		self.serial = DOCUMENT.fetch_add(1, Ordering::Relaxed);
		self.active = Some(Active {
			channel: message.channel,
			message: message.id,
			fingerprint: fingerprint(message),
			source: Source::Message,
			document: self.serial,
			request: 0,
			page: 0,
			pages: None,
			attachment: attachment.clone(),
		});
		self.zoom = 1.;
		self.render();
	}
	pub(crate) fn set_source(&mut self, source: Source) {
		if let Some(a) = &mut self.active {
			a.source = source;
		}
	}
	fn render(&mut self) {
		self.texture = None;
		self.error = None;
		if let Some(a) = &mut self.active {
			a.request = a.request.wrapping_add(1);
			self.command = Some(PdfCommand::Render(Box::new(PdfRequest {
				document: a.document,
				request: a.request,
				page: a.page,
				attachment: a.attachment.clone(),
			})));
		}
	}
	fn step(&mut self, next: bool) {
		if let Some(a) = &mut self.active {
			let page = if next {
				a.page.saturating_add(1).min(a.pages.unwrap_or(1) - 1)
			} else {
				a.page.saturating_sub(1)
			};
			if page != a.page {
				a.page = page;
				self.render();
			}
		}
	}
	pub fn fail(&mut self, document: u64, request: u64, error: PdfError) {
		if self.identity() == Some((document, request)) {
			self.texture = None;
			self.error = Some(error);
		}
	}
	#[allow(clippy::too_many_arguments)]
	pub fn accept(
		&mut self,
		ctx: &egui::Context,
		document: u64,
		request: u64,
		pages: u32,
		page: u32,
		image: Arc<egui::ColorImage>,
	) {
		let Some(a) = &mut self.active else {
			return;
		};
		if (a.document, a.request, a.page) != (document, request, page) {
			return;
		}
		if pages == 0
			|| pages > MAX_PDF_PAGES
			|| page >= pages
			|| a.pages.is_some_and(|known| known != pages)
			|| !valid_pdf_dimensions(image.size[0], image.size[1])
			|| image.pixels.len() != image.size[0] * image.size[1]
		{
			self.fail(document, request, PdfError::Invalid);
			return;
		}
		a.pages = Some(pages);
		self.error = None;
		self.texture =
			Some(ctx.load_texture("pdf-current-page", image, egui::TextureOptions::LINEAR));
	}
	pub(crate) fn revalidate(
		&mut self,
		state: &State,
		search_open: bool,
		timeline: &crate::timeline::TimelineView,
	) {
		self.bind(state);
		let valid = self.active.as_ref().is_none_or(|a| {
			if state.user.is_none()
				|| !state.can_view(a.channel)
				|| state.timeline.is_deleted(a.message)
			{
				return false;
			}
			match &a.source {
				Source::Search {
					request,
					fingerprint: expected,
					timeline: snapshot,
				} => {
					search_open
						&& state.search.as_ref().is_some_and(|view| {
							view.request == *request
								&& !view.loading && view.page.as_ref().is_some_and(|p| {
								p.hits.iter().any(|h| {
									h.id == a.message
										&& h.channel == a.channel && hit_fingerprint(h) == *expected
										&& h.attachments.contains(&a.attachment)
										&& !crate::embeds::media_has_spoilers(
											&h.attachments,
											&h.embeds,
										)
								})
							})
						}) && match (snapshot, state.timeline.get(a.message)) {
						(None, None) => true,
						(Some(expected), Some(message)) => {
							fingerprint(message) == *expected
								&& message.channel == a.channel
								&& message.attachments.contains(&a.attachment)
								&& !crate::embeds::has_media_spoilers(message)
						}
						_ => false,
					}
				}
				Source::Message | Source::Component(_) => state
					.timeline
					.get(a.message)
					.or_else(|| {
						state
							.interactions
							.ephemeral
							.iter()
							.find(|m| m.id == a.message)
					})
					.is_some_and(|m| {
						m.channel == a.channel
							&& fingerprint(m) == a.fingerprint
							&& m.attachments.contains(&a.attachment)
							&& match &a.source {
								Source::Component(keys) => {
									timeline.components.pdf_reveals_valid(state, keys)
								}
								_ => timeline.pdf_media_visible(m),
							}
					}),
			}
		});
		if !valid {
			self.close();
		}
	}
	/// Consume modal keys before the search pane/composer can claim Escape or arrows.
	pub(crate) fn keys(&mut self, ctx: &egui::Context) {
		if !self.is_open() {
			return;
		}
		if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
			self.close();
			return;
		}
		if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
			self.step(false);
		}
		if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
			self.step(true);
		}
	}
	pub(crate) fn show(
		&mut self,
		ctx: &egui::Context,
		download: &mut crate::DownloadUi,
		demo: bool,
	) {
		let Some(a) = &self.active else {
			return;
		};
		let attachment = a.attachment.clone();
		let page = a.page;
		let pages = a.pages;
		let height = page_height(ctx.content_rect().height());
		let mut previous = false;
		let mut next = false;
		let mut retry = false;
		let response =
			crate::dialog::Dialog::new("pdf-preview", crate::i18n::translate("pdf-title"))
				.width(DIALOG_WIDTH)
				.show(ctx, |body| {
					body.content(|ui| {
						ui.add(egui::Label::new(&attachment.filename).truncate())
							.on_hover_text(&attachment.filename);
						ui.horizontal_wrapped(|ui| {
							previous = ui
								.add_enabled(
									page > 0,
									egui::Button::new(crate::i18n::translate("pdf-previous")),
								)
								.clicked();
							ui.label(crate::i18n::translate_args(
								"pdf-page",
								&[
									("current", &(page + 1).to_string()),
									(
										"total",
										&pages.map_or_else(|| "—".into(), |n| n.to_string()),
									),
								],
							));
							next = ui
								.add_enabled(
									pages.is_some_and(|n| page + 1 < n),
									egui::Button::new(crate::i18n::translate("pdf-next")),
								)
								.clicked();
							if ui.button(crate::i18n::translate("pdf-fit")).clicked() {
								self.zoom = 1.;
							}
							if ui
								.add_enabled(
									self.zoom > 1.,
									egui::Button::new(crate::i18n::translate("pdf-zoom-out")),
								)
								.on_hover_text(crate::i18n::translate("pdf-zoom-out"))
								.clicked()
							{
								self.zoom = (self.zoom - 0.25).max(1.);
							}
							ui.label(crate::i18n::translate_args(
								"pdf-zoom",
								&[("zoom", &format!("{:.2}", self.zoom))],
							));
							if ui
								.add_enabled(
									self.zoom < MAX_ZOOM,
									egui::Button::new(crate::i18n::translate("pdf-zoom-in")),
								)
								.on_hover_text(crate::i18n::translate("pdf-zoom-in"))
								.clicked()
							{
								self.zoom = (self.zoom + 0.25).min(MAX_ZOOM);
							}
						});
						if let Some(texture) = &self.texture {
							let native = texture.size_vec2();
							let fit = (ui.available_width() / native.x)
								.min(height / native.y)
								.min(1.);
							let size = native * fit * self.zoom;
							egui::ScrollArea::both()
								.id_salt(("pdf-page", page))
								.max_height(height)
								.auto_shrink([false, false])
								.animated(false)
								.show(ui, |ui| {
									ui.allocate_ui_with_layout(
										egui::vec2(ui.available_width().max(size.x), size.y),
										egui::Layout::top_down(egui::Align::Center),
										|ui| {
											ui.add(
												egui::Image::new((texture.id(), size))
													.fit_to_exact_size(size)
													.alt_text(crate::i18n::translate_args(
														"pdf-image",
														&[("page", &(page + 1).to_string())],
													)),
											);
										},
									);
								});
						} else {
							ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
								ui.centered_and_justified(|ui| {
									if let Some(error) = self.error {
										ui.label(crate::i18n::translate(error.key()));
									} else {
										ui.horizontal(|ui| {
											ui.spinner();
											ui.label(crate::i18n::translate("pdf-loading"));
										});
									}
								});
							});
						}
					});
					body.footer(|ui| {
						crate::attachments::download_button(ui, &attachment, download, demo);
						if self.error.is_some() {
							retry = ui.button(crate::i18n::translate("pdf-retry")).clicked();
						}
					});
				});
		if response.close {
			self.close();
		} else if previous {
			self.step(false);
		} else if next {
			self.step(true);
		} else if retry {
			self.render();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn render_envelope_matches_viewport_zoom_and_display_density() {
		assert_eq!(render_target(egui::vec2(1120., 760.), 1.), [1536, 940]);
		assert_eq!(render_target(egui::vec2(480., 400.), 1.), [816, 220]);
		assert_eq!(render_target(egui::vec2(480., 400.), 2.), [1536, 440]);
		assert_eq!(render_target(egui::vec2(1120., 760.), 2.), [1536, 1536]);
		assert_eq!(render_target(egui::vec2(1e30, 1e30), 1.), [1536, 1240]);
		for viewport in [
			egui::Vec2::ZERO,
			egui::vec2(-1., -1.),
			egui::vec2(f32::NAN, f32::INFINITY),
		] {
			for density in [0., -1., f32::NAN, f32::INFINITY, f32::MAX] {
				let target = render_target(viewport, density);
				assert!(
					target
						.iter()
						.all(|&edge| edge > 0 && edge as usize <= MAX_PDF_EDGE)
				);
			}
		}
		assert_eq!(
			render_target(egui::vec2(1120., 760.), f32::NAN),
			render_target(egui::vec2(1120., 760.), 1.)
		);
	}
	fn fixture() -> (
		State,
		Message,
		Attachment,
		crate::timeline::TimelineView,
		PdfUi,
	) {
		let state = test_support::pdf_demo_state();
		let message = state.timeline.get(Id(501)).unwrap().clone();
		let attachment = message.attachments[0].clone();
		let mut view = PdfUi::default();
		view.bind(&state);
		view.open(&message, &attachment);
		(
			state,
			message,
			attachment,
			crate::timeline::TimelineView::default(),
			view,
		)
	}
	#[test]
	fn bounds_detection_and_stale_results() {
		assert!(!valid_pdf_dimensions(0, 1));
		assert!(!valid_pdf_dimensions(usize::MAX, 2));
		assert!(!valid_pdf_dimensions(1536, 1536));
		assert!(valid_pdf_dimensions(1536, 1024));
		let (_, message, attachment, _, mut view) = fixture();
		let ctx = egui::Context::default();
		let (doc, request) = view.identity().unwrap();
		let image = Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::WHITE));
		view.accept(&ctx, doc + 1, request, 2, 0, image.clone());
		assert!(view.texture.is_none());
		view.accept(&ctx, doc, request, 2, 0, image.clone());
		assert!(view.texture.is_some());
		view.step(true);
		let (_, next) = view.identity().unwrap();
		assert_eq!(view.active.as_ref().unwrap().page, 1);
		assert!(view.texture.is_none());
		view.fail(doc, request, PdfError::Invalid);
		assert!(view.error.is_none());
		view.accept(&ctx, doc, next, 2, 0, image.clone());
		assert!(view.texture.is_none());
		view.accept(&ctx, doc, next, 2, 1, image.clone());
		assert!(view.texture.is_some());
		view.step(true);
		assert_eq!(view.identity(), Some((doc, next)));
		view.close();
		view.accept(&ctx, doc, next, 2, 1, image.clone());
		assert!(!view.is_open());
		assert!(view.texture.is_none());
		view.open(&message, &attachment);
		assert_ne!(view.identity().unwrap().0, doc);
		view.accept(&ctx, doc, next, 2, 1, image);
		assert!(view.texture.is_none());
	}
	#[test]
	fn invalid_page_counts_pixels_and_metadata_fail_closed() {
		for (pages, page, size) in [
			(0, 0, [2, 2]),
			(257, 0, [2, 2]),
			(2, 0, [0, 1]),
			(2, 0, [1537, 1]),
			(2, 0, [1536, 1536]),
		] {
			let (_, _, _, _, mut view) = fixture();
			let (doc, request) = view.identity().unwrap();
			view.accept(
				&egui::Context::default(),
				doc,
				request,
				pages,
				page,
				Arc::new(egui::ColorImage::filled(size, egui::Color32::WHITE)),
			);
			assert_eq!(view.error, Some(PdfError::Invalid));
			assert!(view.texture.is_none());
		}
		let (_, message, mut attachment, _, mut view) = fixture();
		view.close();
		attachment.filename = "not-a-pdf.txt".into();
		attachment.content_type = None;
		view.open(&message, &attachment);
		assert!(!view.is_open());
	}
	#[test]
	fn reaction_updates_preserve_pdf_source_and_current_texture() {
		for source in 0..3 {
			let (mut state, message, _, timeline, mut view) = fixture();
			if source != 0 {
				search(&mut state, &message, source == 2);
				let hit = &state.search.as_ref().unwrap().page.as_ref().unwrap().hits[0];
				view.set_source(Source::Search {
					request: 1,
					fingerprint: hit_fingerprint(hit),
					timeline: Some(fingerprint(&message)),
				});
			}
			let ctx = egui::Context::default();
			let identity = view.identity().unwrap();
			view.accept(
				&ctx,
				identity.0,
				identity.1,
				2,
				0,
				Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::WHITE)),
			);
			let texture = view.texture.as_ref().unwrap().id();
			view.command.take();
			for count in [1, 2, 0] {
				let reactions = (count > 0).then(|| {
					vec![model::Reaction {
						emoji: model::ReactionEmoji {
							id: None,
							name: Some("👍".into()),
						},
						count,
						me: false,
						me_burst: false,
					}]
				});
				state.timeline.set_reactions(message.id, reactions).unwrap();
				assert!(state.timeline.get(message.id).unwrap().revision > message.revision);
				view.revalidate(&state, true, &timeline);
				assert!(view.is_open(), "source {source}, reaction count {count}");
				assert_eq!(view.identity(), Some(identity));
				assert_eq!(view.texture.as_ref().unwrap().id(), texture);
				assert!(
					view.command.is_none(),
					"reaction updates must not re-render"
				);
			}
		}
	}
	#[test]
	fn navigation_account_logout_edit_delete_and_eviction_close() {
		for change in 0..9 {
			let (mut state, message, _, timeline, mut view) = fixture();
			match change {
				0 => state.generation += 1,
				1 => state.selected = Some(Id(21)),
				2 => state.user = None,
				3 => {
					state.user.as_mut().unwrap().id = Id(999);
				}
				4 => {
					let mut edited = message.clone();
					edited.content = "Edited PDF message".into();
					state.timeline.insert(edited, false, false).unwrap();
				}
				5 => {
					state.timeline.delete(message.id).unwrap();
				}
				6 => state.timeline.clear(),
				8 => {
					state.channels.clear();
				}
				_ => {
					let mut edited = message.clone();
					edited.attachments.clear();
					state.timeline.insert(edited, false, false).unwrap();
				}
			}
			view.revalidate(&state, true, &timeline);
			assert!(!view.is_open(), "change {change}");
			assert!(matches!(view.command, Some(PdfCommand::Close)));
		}
	}
	#[test]
	fn spoiler_flags_names_descriptions_and_ephemeral_lifetime() {
		for kind in 0..3 {
			let (mut state, mut message, _, timeline, mut view) = fixture();
			view.close();
			match kind {
				0 => message.attachments[0].spoiler = true,
				1 => message.attachments[0].filename = "SPOILER_doc.pdf".into(),
				_ => message.attachments[0].description = Some("||secret||".into()),
			}
			state
				.timeline
				.insert(message.clone(), false, false)
				.unwrap();
			message = state.timeline.get(message.id).unwrap().clone();
			view.open(&message, &message.attachments[0]);
			view.revalidate(&state, true, &timeline);
			assert!(!view.is_open());
		}
		let (mut state, message, attachment, timeline, mut view) = fixture();
		state.timeline.clear();
		state.interactions.ephemeral.push(message.clone());
		view.revalidate(&state, true, &timeline);
		assert!(view.is_open());
		state.interactions.ephemeral.clear();
		view.revalidate(&state, true, &timeline);
		assert!(!view.is_open());
		view.open(&message, &attachment);
		view.set_source(Source::Component(vec![1]));
		view.revalidate(&state, true, &timeline);
		assert!(!view.is_open());
	}
	fn search(state: &mut State, message: &Message, pins: bool) {
		state.search = Some(client_core::search::SearchView {
			pins,
			channel: message.channel,
			guild: None,
			query: String::new(),
			before: None,
			offset: 0,
			total: Some(1),
			pin_before: None,
			request: 1,
			loading: false,
			error: None,
			page: Some(model::SearchPage {
				hits: vec![model::SearchHit {
					id: message.id,
					channel: message.channel,
					author: message.author.clone(),
					mentions: vec![],
					excerpt: String::new(),
					attachments: message.attachments.clone(),
					embeds: message.embeds.clone(),
				}],
				total: 1,
				partial: false,
				pin_cursor: None,
			}),
		});
	}
	#[test]
	fn search_and_pin_sources_invalidate_on_navigation_edits_and_spoilers() {
		for pins in [false, true] {
			for change in 0..9 {
				let (mut state, message, attachment, timeline, mut view) = fixture();
				search(&mut state, &message, pins);
				let hit = &state.search.as_ref().unwrap().page.as_ref().unwrap().hits[0];
				view.set_source(Source::Search {
					request: 1,
					fingerprint: hit_fingerprint(hit),
					timeline: Some(fingerprint(&message)),
				});
				view.revalidate(&state, true, &timeline);
				assert!(view.is_open());
				let pane_open = change != 0;
				match change {
					0 => {}
					1 => state.search.as_mut().unwrap().request += 1,
					2 => state.search.as_mut().unwrap().loading = true,
					3 => state
						.search
						.as_mut()
						.unwrap()
						.page
						.as_mut()
						.unwrap()
						.hits
						.clear(),
					4 => {
						state.search.as_mut().unwrap().page.as_mut().unwrap().hits[0].attachments
							[0]
						.spoiler = true
					}
					5 => {
						let mut edited = message.clone();
						edited.content = "edited".into();
						state.timeline.insert(edited, false, false).unwrap();
					}
					6 => state.timeline.clear(),
					7 => state.search = None,
					_ => {
						state.search.as_mut().unwrap().page.as_mut().unwrap().hits[0].attachments
							[0]
						.description = Some("||secret||".into())
					}
				}
				view.revalidate(&state, pane_open, &timeline);
				assert!(!view.is_open(), "pins {pins}, change {change}");
				view.open(&message, &attachment); // New identities must not resurrect an old search render.
			}
		}
	}
	#[test]
	fn raster_modal_renders_in_both_themes_and_short_narrow_viewports() {
		for dark in [true, false] {
			for (width, height) in [(1120., 760.), (760., 520.), (480., 400.)] {
				let (_, _, _, _, mut view) = fixture();
				let ctx = egui::Context::default();
				ctx.set_visuals(if dark {
					egui::Visuals::dark()
				} else {
					egui::Visuals::light()
				});
				let (doc, request) = view.identity().unwrap();
				view.accept(
					&ctx,
					doc,
					request,
					2,
					0,
					Arc::new(egui::ColorImage::filled([600, 780], egui::Color32::WHITE)),
				);
				let mut download = crate::DownloadUi::default();
				for _ in 0..3 {
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, height),
							)),
							..Default::default()
						},
						|ui| view.show(ui.ctx(), &mut download, true),
					);
					assert!(!output.shapes.is_empty());
					let size = ctx
						.data(|d| {
							d.get_temp::<egui::Vec2>(
								egui::Id::unique("pdf-preview").with("dialog-size"),
							)
						})
						.unwrap();
					assert!(
						size.x <= width && size.y <= height,
						"modal {size:?} exceeds {width}x{height}"
					);
					output.drop_without_applying_deltas();
				}
			}
		}
	}
	#[test]
	fn resize_and_zoom_reuse_raster_without_requesting_work() {
		let (_, _, _, _, mut view) = fixture();
		view.command.take();
		let ctx = egui::Context::default();
		let identity = view.identity().unwrap();
		view.accept(
			&ctx,
			identity.0,
			identity.1,
			2,
			0,
			Arc::new(egui::ColorImage::filled([723, 940], egui::Color32::WHITE)),
		);
		let texture = view.texture.as_ref().unwrap().id();
		let mut download = crate::DownloadUi::default();
		for (width, height, zoom) in [(1120., 760., 1.), (480., 400., 2.), (760., 520., 1.25)] {
			view.zoom = zoom;
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, height),
					)),
					..Default::default()
				},
				|ui| view.show(ui.ctx(), &mut download, true),
			);
			output.drop_without_applying_deltas();
			assert!(view.command.is_none(), "no render on layout/zoom changes");
			assert_eq!(view.identity(), Some(identity));
			assert_eq!(view.texture.as_ref().unwrap().id(), texture);
		}
		view.step(true);
		assert!(matches!(view.command, Some(PdfCommand::Render(_))));
	}
	#[test]
	fn keyboard_arrows_and_escape_are_consumed_only_while_open() {
		let (_, _, _, _, mut view) = fixture();
		view.active.as_mut().unwrap().pages = Some(2);
		let ctx = egui::Context::default();
		for key in [
			egui::Key::ArrowRight,
			egui::Key::ArrowLeft,
			egui::Key::Escape,
		] {
			let output = ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
					..Default::default()
				},
				|ui| {
					view.keys(ui.ctx());
					assert!(!ui.ctx().input(|i| i.key_pressed(key)));
				},
			);
			output.drop_without_applying_deltas();
		}
		assert!(!view.is_open());
	}
}
