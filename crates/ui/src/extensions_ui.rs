//! Native community extensions UI. The desktop owns all package IO and execution.
use crate::{design, icons};
use client_core::{MAX_CONTENT, MAX_DRAFT_BYTES, State};
use extensions::{Capability, Element, ExtensionKind, Invocation, Manifest, Output, Surface};
use model::Id;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct ExtensionEntry {
	pub description: String,
	pub preview: Option<extensions::Preview>,
	pub theme_preview: Option<extensions::Theme>,
	pub cover_image: Option<Arc<egui::ColorImage>>,
	pub local_theme: bool,
	pub manifest: Manifest,
	pub reviewed: bool,
	pub sha256: String,
	pub download_bytes: u64,
	pub enabled: bool,
	pub cleanup_pending: bool,
	pub update_available: bool,
	pub update_manifest: Option<Manifest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionContext {
	pub app_wide: bool,
	pub generation: u64,
	pub channel: Option<Id>,
	pub draft: Option<String>,
	/// The call at invocation time; only voice proposals use this guard.
	pub voice_request: Option<(Id, u64)>,
}
impl ExtensionContext {
	pub fn capture(state: &State, composer: bool) -> Self {
		Self {
			app_wide: false,
			generation: state.generation,
			channel: state.selected,
			voice_request: state
				.voice
				.active
				.as_ref()
				.map(|call| (call.channel, call.request)),
			draft: composer.then(|| {
				state
					.selected
					.and_then(|id| state.drafts.get(&id))
					.cloned()
					.unwrap_or_default()
			}),
		}
	}
	pub fn panel(state: &State) -> Self {
		Self {
			app_wide: true,
			channel: None,
			..Self::capture(state, false)
		}
	}
	pub fn is_current(&self, state: &State) -> bool {
		self.generation == state.generation
			&& (self.app_wide || self.channel == state.selected)
			&& self.draft.as_ref().is_none_or(|draft| {
				state
					.selected
					.and_then(|id| state.drafts.get(&id))
					.map_or("", String::as_str)
					== draft
			})
	}
}

pub enum ExtensionRequest {
	EditTheme {
		id: String,
		preview: bool,
	},
	PickThemeImage,
	PickThemeCover,
	SaveTheme {
		package: Box<extensions::Package>,
	},
	ExportTheme {
		package: Box<extensions::Package>,
	},
	PreviewTheme {
		theme: Option<Box<extensions::Theme>>,
		image: Option<Arc<egui::ColorImage>>,
	},
	SelectTheme {
		id: Option<String>,
	},
	Preview {
		id: String,
	},
	RefreshCatalog,
	Import,
	Enable {
		id: String,
		grants: Vec<Capability>,
		sha256: String,
		reviewed: bool,
	},
	Disable {
		id: String,
	},
	Invoke {
		id: String,
		invocation: Invocation,
		context: ExtensionContext,
	},
}

#[derive(Clone)]
pub(crate) struct MenuAction {
	pub plugin: String,
	pub action: String,
	pub label: String,
}

struct Consent {
	entry: ExtensionEntry,
	grants: Vec<Capability>,
}
struct ResultPanel {
	id: String,
	context: ExtensionContext,
	invocation: Invocation,
	output: Output,
	values: BTreeMap<String, String>,
}

/// Card chrome under the 16:9 preview: badges, title, blurb and the action row.
const CARD_BODY: f32 = 196.0;
const THEME_CARD_BODY: f32 = 108.0;
const CARD_RADIUS: u8 = 12;
const FOOTER_HEIGHT: f32 = 34.0;

// Twenty-four 640x360 RGBA thumbnails: at most 22,118,400 bytes of image pixels. A three
// column gallery shows nine cards at once, so the bound covers a couple of scrolled pages.
const MAX_PREVIEWS: usize = 24;
struct PreviewImage {
	image: Option<egui::ColorImage>,
	texture: Option<egui::TextureHandle>,
	used: u64,
	loading: bool,
}
#[derive(Default)]
pub struct ExtensionUi {
	pub active_theme: Option<String>,
	pub entries: Vec<ExtensionEntry>,
	pub status: String,
	pub busy: bool,
	pub catalog_refreshing: bool,
	pub requests: Vec<ExtensionRequest>,
	consent: Option<Consent>,
	result: Option<ResultPanel>,
	error: Option<String>,
	message_actions: Arc<Vec<MenuAction>>,
	themes: bool,
	query: String,
	previews: BTreeMap<String, PreviewImage>,
	preview_clock: u64,
	enlarged: Option<String>,
	theme_editor: Option<crate::theme_editor::ThemeEditor>,
	theme_editor_visible: bool,
	gallery_preview: Option<String>,
}
impl ExtensionUi {
	pub fn editing_theme(&self) -> bool {
		self.theme_editor.is_some()
	}
	pub(crate) fn theme_editor_tab_key(&self) -> u8 {
		self.theme_editor
			.as_ref()
			.map_or(0, |editor| editor.tab_key() + 1)
	}
	pub fn theme_editor_dirty(&self) -> bool {
		self.theme_editor
			.as_ref()
			.is_some_and(|editor| editor.dirty)
	}
	pub fn receive_theme_edit(
		&mut self,
		package: Box<extensions::Package>,
		image: Option<Arc<egui::ColorImage>>,
		cover: Option<Arc<egui::ColorImage>>,
		local_theme: bool,
		preview: bool,
	) {
		if self.theme_editor.is_none()
			&& package.manifest.kind == ExtensionKind::Theme
			&& package.theme.is_some()
		{
			self.gallery_preview = preview.then(|| package.manifest.name.clone());
			// Gallery notices such as a finished removal do not belong in the editor.
			self.status.clear();
			self.theme_editor = Some(if local_theme {
				crate::theme_editor::ThemeEditor::edit(package, image, cover)
			} else {
				crate::theme_editor::ThemeEditor::duplicate(package, image, cover)
			});
		}
	}
	#[cfg(feature = "demo")]
	pub fn preview_theme_editor_tab(&mut self, label: &str) {
		if let Some(editor) = &mut self.theme_editor {
			editor.preview_tab(label);
		}
	}
	/// Fixture-only entry point: opens the theme maker on `label` with a fresh draft.
	#[cfg(feature = "demo")]
	pub fn preview_theme_maker(&mut self, label: &str) {
		self.themes = true;
		self.theme_editor = Some(crate::theme_editor::ThemeEditor::new());
		self.preview_theme_editor_tab(label);
	}
	pub fn receive_theme_image(&mut self, bytes: Vec<u8>, image: Arc<egui::ColorImage>) {
		if bytes.len() > extensions::MAX_BACKGROUND_BYTES
			|| image.size.contains(&0)
			|| image.size.into_iter().any(|side| side > 4096)
			|| image.pixels.len() > 4_000_000
			|| image.pixels.len() != image.size[0] * image.size[1]
		{
			return;
		}
		if let Some(editor) = &mut self.theme_editor {
			editor.receive_image(bytes, image);
			if editor.preview {
				self.requests
					.retain(|request| !matches!(request, ExtensionRequest::PreviewTheme { .. }));
				let request = editor.preview_request();
				if self.requests.len() < 4
					&& request_bytes(&request)
						.saturating_add(self.requests.iter().map(request_bytes).sum::<usize>())
						<= 2 * extensions::MAX_PACKAGE_BYTES
				{
					self.requests.push(request);
				}
			}
		}
	}
	pub fn receive_theme_cover(&mut self, bytes: Vec<u8>, image: Arc<egui::ColorImage>) {
		if bytes.len() > extensions::MAX_BACKGROUND_BYTES
			|| image.size.contains(&0)
			|| image.size[0] > 640
			|| image.size[1] > 360
			|| image.pixels.len() != image.size[0] * image.size[1]
		{
			return;
		}
		if let Some(editor) = &mut self.theme_editor {
			editor.receive_cover(bytes, image);
		}
	}
	pub fn theme_saved(&mut self, id: &str) {
		if let Some(editor) = &mut self.theme_editor
			&& editor.package.manifest.id == id
		{
			editor.dirty = false;
			editor.preview = false;
		}
	}
	pub fn stop_theme_preview(&mut self, ctx: &egui::Context) {
		if let Some(editor) = &mut self.theme_editor
			&& editor.preview
		{
			editor.preview = false;
			self.queue(
				ctx,
				ExtensionRequest::PreviewTheme {
					theme: None,
					image: None,
				},
			);
		}
		if self.gallery_preview.take().is_some() {
			self.theme_editor = None;
		}
	}
	pub(crate) fn begin_gallery_preview(&mut self, ctx: &egui::Context) -> bool {
		if self.gallery_preview.is_none() || self.previewing_theme() {
			return false;
		}
		let Some(editor) = &mut self.theme_editor else {
			return false;
		};
		editor.preview = true;
		let request = editor.preview_request();
		self.queue(ctx, request);
		true
	}
	pub(crate) fn begin_theme_editor_frame(&mut self) {
		self.theme_editor_visible = false;
	}
	pub(crate) fn previewing_theme(&self) -> bool {
		self.theme_editor
			.as_ref()
			.is_some_and(|editor| editor.preview)
	}
	pub(crate) fn theme_preview_bar(&mut self, ui: &mut egui::Ui, inset: f32) -> bool {
		if !self.previewing_theme() {
			return false;
		}
		let colors = design::palette(ui);
		let surface = design::section_surface(
			ui,
			design::window_palette(ui).base,
			design::ImageSection::TopBar,
		);
		let gallery = self.gallery_preview.clone();
		let mut back = false;
		let mut customize = false;
		egui::Panel::top("theme-preview-return")
			.exact_size(52.0)
			.show_separator_line(false)
			.frame(egui::Frame::new().fill(surface).inner_margin(egui::Margin {
				left: (inset + 16.0).min(120.0) as i8,
				right: 16,
				top: 0,
				bottom: 0,
			}))
			.show(ui, |ui| {
				let rect = ui.max_rect();
				if inset > 0.0 {
					// The bar sits beside the traffic lights, so it doubles as a drag region.
					design::window_drag(ui, rect);
				}
				ui.painter().hline(
					rect.x_range().expand(inset + 16.0),
					rect.bottom() - 0.5,
					egui::Stroke::new(1.0, colors.border),
				);
				ui.horizontal_centered(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					let (icon, _) =
						ui.allocate_exact_size(egui::Vec2::splat(30.0), egui::Sense::hover());
					ui.painter().circle_filled(
						icon.center(),
						15.0,
						design::mix(colors.raised, colors.accent, 0.22),
					);
					icons::paint(
						ui.painter(),
						icons::Icon::Sparkle,
						egui::Rect::from_center_size(icon.center(), egui::Vec2::splat(16.0)),
						colors.accent,
					);
					// Two buttons plus their gap; the title truncates before they wrap.
					let title_width = (ui.available_width() - 262.0).max(80.0);
					ui.allocate_ui_with_layout(
						egui::vec2(title_width, rect.height()),
						egui::Layout::top_down(egui::Align::Min),
						|ui| {
							ui.add_space(8.0);
							ui.spacing_mut().item_spacing.y = 1.0;
							ui.label(design::eyebrow(
								ui,
								if gallery.is_some() {
									"Previewing theme"
								} else {
									"Theme preview"
								},
								colors.muted,
							));
							ui.add(
								egui::Label::new(
									design::semibold(
										ui,
										gallery.as_deref().unwrap_or("Changes are not saved yet"),
										14.0,
									)
									.color(colors.text_strong),
								)
								.truncate(),
							);
						},
					);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.spacing_mut().item_spacing.x = 8.0;
						back = design::button(
							ui,
							if gallery.is_some() {
								"Back to themes"
							} else {
								"Back to theme editor"
							},
							design::ButtonKind::Primary,
						)
						.clicked();
						if gallery.is_some() {
							customize =
								design::button(ui, "Customize", design::ButtonKind::Outline)
									.clicked();
						}
					});
				});
			});
		if customize {
			self.gallery_preview = None;
		}
		if back || customize {
			self.stop_theme_preview(ui.ctx());
		}
		back || customize
	}
	fn edit_theme(&mut self, ui: &mut egui::Ui) {
		let Some(mut editor) = self.theme_editor.take() else {
			return;
		};
		if !self.status.is_empty() {
			design::card(ui, |ui| {
				ui.add(egui::Label::new(&self.status).wrap());
			});
			ui.add_space(12.0);
		}
		let mut requests = Vec::new();
		let close = editor.show(ui, self.busy, &mut requests);
		for request in requests {
			self.queue(ui.ctx(), request);
		}
		if close {
			if editor.preview {
				self.queue(
					ui.ctx(),
					ExtensionRequest::PreviewTheme {
						theme: None,
						image: None,
					},
				);
			}
		} else {
			self.theme_editor = Some(editor);
		}
	}
	pub(crate) fn theme_editor_toolbar(&mut self, ui: &mut egui::Ui) {
		let Some(mut editor) = self.theme_editor.take() else {
			return;
		};
		let mut requests = Vec::new();
		let close = ui
			.scope(|ui| {
				ui.set_max_width(ui.available_width().min(720.0));
				editor.toolbar(ui, self.busy, &mut requests)
			})
			.inner;
		for request in requests {
			self.queue(ui.ctx(), request);
		}
		if !close {
			self.theme_editor = Some(editor);
		}
	}
	pub fn set_entries(&mut self, entries: Vec<ExtensionEntry>) {
		if self.consent.as_ref().is_some_and(|consent| {
			consent.entry.reviewed
				&& !entries.iter().any(|entry| {
					entry.reviewed
						&& !entry.cleanup_pending
						&& entry.sha256 == consent.entry.sha256
						&& entry.download_bytes == consent.entry.download_bytes
						&& entry.update_manifest.as_ref().unwrap_or(&entry.manifest)
							== &consent.entry.manifest
				})
		}) {
			self.consent = None;
		}
		self.previews.retain(|id, _| {
			let old = self.entries.iter().find(|entry| &entry.manifest.id == id);
			let new = entries.iter().find(|entry| &entry.manifest.id == id);
			matches!((old, new), (Some(old), Some(new))
				if old.preview == new.preview
					&& old.cover_image.as_ref().map(Arc::as_ptr)
						== new.cover_image.as_ref().map(Arc::as_ptr))
		});
		self.entries = entries;
		self.message_actions = Arc::new(
			self.entries
				.iter()
				.filter(|entry| entry.enabled)
				.flat_map(|entry| {
					entry
						.manifest
						.actions
						.iter()
						.filter(|action| action.surface == Surface::Message)
						.map(|action| MenuAction {
							plugin: entry.manifest.id.clone(),
							action: action.id.clone(),
							label: format!("{} · {}", entry.manifest.name, action.label),
						})
				})
				.collect(),
		);
	}
	/// Select the catalog tab in an offline native fixture.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_themes(&mut self, themes: bool) {
		self.themes = themes;
	}
	pub(crate) fn select_themes(&mut self, themes: bool) {
		if self.themes != themes {
			self.themes = themes;
			self.query.clear();
			self.enlarged = None;
		}
	}
	pub fn receive_preview(&mut self, id: String, image: Option<egui::ColorImage>) {
		if !self.entries.iter().any(|entry| entry.manifest.id == id) {
			return;
		}
		if let Some(preview) = self.previews.get_mut(&id) {
			preview.loading = false;
			preview.image = image.filter(|image| {
				image.size[0] > 0
					&& image.size[1] > 0
					&& image.size[0] <= 640
					&& image.size[1] <= 360
					&& image.pixels.len() == image.size[0] * image.size[1]
			});
		}
	}
	pub fn retry_preview(&mut self, id: &str) {
		self.previews.remove(id);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_fixture_image(&mut self, id: String, image: egui::ColorImage) {
		if self.reserve_preview(&id) {
			self.receive_preview(id, Some(image));
		}
	}
	fn reserve_preview(&mut self, id: &str) -> bool {
		if self.previews.contains_key(id) {
			return true;
		}
		if self.previews.len() >= MAX_PREVIEWS {
			let oldest = self
				.previews
				.iter()
				.filter(|(_, image)| image.used.saturating_add(1) < self.preview_clock)
				.min_by_key(|(_, image)| image.used)
				.map(|(id, _)| id.clone());
			let Some(oldest) = oldest else {
				return false;
			};
			self.previews.remove(&oldest);
		}
		self.previews.insert(
			id.into(),
			PreviewImage {
				image: None,
				texture: None,
				used: self.preview_clock,
				loading: true,
			},
		);
		true
	}
	fn open_preview(&mut self, ctx: &egui::Context, entry: &ExtensionEntry) {
		if entry.manifest.kind == ExtensionKind::Theme && entry.theme_preview.is_some() {
			if !self.busy && !entry.cleanup_pending && self.theme_editor.is_none() {
				self.queue(
					ctx,
					ExtensionRequest::EditTheme {
						id: entry.manifest.id.clone(),
						preview: true,
					},
				);
			}
		} else {
			self.enlarged = Some(entry.manifest.id.clone());
		}
	}
	fn preview_image(
		&mut self,
		ui: &mut egui::Ui,
		entry: &ExtensionEntry,
		radius: egui::CornerRadius,
	) {
		let colors = design::palette(ui);
		let size = egui::vec2(ui.available_width(), ui.available_width() * 9.0 / 16.0);
		let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
		if !ui.is_rect_visible(rect) {
			return;
		}
		if entry.cover_image.is_none()
			&& (entry.theme_preview.is_some()
				|| entry.manifest.capabilities.iter().any(|cap| {
					matches!(cap, Capability::DeletedMessages | Capability::ImageSharing)
				})) {
			draw_native_preview(ui, rect, entry, radius);
			let response = ui.interact(
				rect,
				ui.scope_id().with("enlarge-preview"),
				egui::Sense::click(),
			);
			response.widget_info(|| {
				egui::WidgetInfo::labeled(
					egui::Role::Button,
					true,
					format!("Preview {}", entry.manifest.name),
				)
			});
			if response
				.on_hover_text(format!("Preview {}", entry.manifest.name))
				.clicked()
			{
				self.open_preview(ui.ctx(), entry);
			}
			return;
		}
		let id = &entry.manifest.id;
		if let Some(cover) = &entry.cover_image
			&& !self.previews.contains_key(id)
			&& self.reserve_preview(id)
		{
			self.receive_preview(id.clone(), Some((**cover).clone()));
		}
		if entry.cover_image.is_none()
			&& entry.preview.is_some()
			&& !self.previews.contains_key(id)
			&& !self.busy
			&& self.requests.len() < 4
			&& self
				.previews
				.values()
				.filter(|preview| preview.loading)
				.count() < 4
			&& self.reserve_preview(id)
		{
			self.queue(ui.ctx(), ExtensionRequest::Preview { id: id.clone() });
		}
		if let Some(preview) = self.previews.get_mut(id) {
			preview.used = self.preview_clock;
			if let Some(image) = preview.image.take() {
				preview.texture = Some(ui.ctx().load_texture(
					format!("extension-preview-{id}"),
					image,
					egui::TextureOptions::LINEAR,
				));
			}
			if let Some(texture) = &preview.texture {
				ui.painter().rect_filled(rect, radius, colors.base);
				if entry.cover_image.is_some() {
					let source = texture.size_vec2();
					let source_ratio = source.x / source.y;
					let target_ratio = rect.width() / rect.height();
					let uv = if source_ratio > target_ratio {
						let inset = (1.0 - target_ratio / source_ratio) / 2.0;
						egui::Rect::from_min_max(
							egui::pos2(inset, 0.0),
							egui::pos2(1.0 - inset, 1.0),
						)
					} else {
						let inset = (1.0 - source_ratio / target_ratio) / 2.0;
						egui::Rect::from_min_max(
							egui::pos2(0.0, inset),
							egui::pos2(1.0, 1.0 - inset),
						)
					};
					egui::Image::new(texture)
						.uv(uv)
						.corner_radius(radius)
						.paint_at(ui, rect);
				} else {
					let fit = texture.size_vec2()
						* (rect.width() / texture.size_vec2().x)
							.min(rect.height() / texture.size_vec2().y);
					egui::Image::new(texture)
						.corner_radius(radius)
						.paint_at(ui, egui::Rect::from_center_size(rect.center(), fit));
				}
				let response = ui.interact(
					rect,
					ui.scope_id().with("enlarge-preview"),
					egui::Sense::click(),
				);
				response.widget_info(|| {
					egui::WidgetInfo::labeled(
						egui::Role::Button,
						true,
						format!("Preview {}", entry.manifest.name),
					)
				});
				if response.on_hover_text("View preview").clicked() {
					self.open_preview(ui.ctx(), entry);
				}
				return;
			}
		}
		ui.painter().rect_filled(rect, radius, colors.sidebar);
		let inset = rect.shrink(18.0);
		let icon = egui::Rect::from_center_size(
			inset.center() - egui::vec2(0.0, 14.0),
			egui::vec2(42.0, 32.0),
		);
		ui.painter().rect_stroke(
			icon,
			5,
			egui::Stroke::new(1.5, colors.muted),
			egui::StrokeKind::Inside,
		);
		ui.painter().circle_filled(icon.center(), 4.0, colors.muted);
		ui.painter().text(
			inset.center() + egui::vec2(0.0, 23.0),
			egui::Align2::CENTER_CENTER,
			if self.previews.get(id).is_some_and(|preview| preview.loading) {
				"Loading preview..."
			} else if entry.preview.is_some() && !self.previews.contains_key(id) {
				"Preview not loaded"
			} else {
				"Preview unavailable"
			},
			egui::FontId::proportional(12.0),
			colors.muted,
		);
	}
	fn preview_modal(&mut self, ctx: &egui::Context) {
		let Some(id) = self.enlarged.clone() else {
			return;
		};
		let Some(entry) = self.entries.iter().find(|entry| entry.manifest.id == id) else {
			self.enlarged = None;
			return;
		};
		let texture = self.previews.get(&id).and_then(|p| p.texture.as_ref());
		let native = entry.cover_image.is_none()
			&& (entry.theme_preview.is_some()
				|| entry.manifest.capabilities.iter().any(|cap| {
					matches!(cap, Capability::DeletedMessages | Capability::ImageSharing)
				}));
		if texture.is_none() && !native {
			self.enlarged = None;
			return;
		}
		let mut close = false;
		let modal = egui::Modal::new(egui::Id::unique("extension-preview-modal")).show(ctx, |ui| {
			ui.label(design::semibold(ui, &entry.manifest.name, 20.0));
			let available = ctx.content_rect().size() - egui::vec2(64.0, 140.0);
			if native {
				let width = available
					.x
					.clamp(1.0, 800.0)
					.min((available.y * 16.0 / 9.0).max(1.0));
				let (rect, _) = ui.allocate_exact_size(
					egui::vec2(width, width * 9.0 / 16.0),
					egui::Sense::hover(),
				);
				draw_native_preview(ui, rect, entry, egui::CornerRadius::same(8));
				if entry.manifest.kind != ExtensionKind::Theme {
					ui.weak(
						if entry
							.manifest
							.capabilities
							.contains(&Capability::ImageSharing)
						{
							"Selecting artwork sends it as an image attachment."
						} else {
							"Example deleted-message appearance"
						},
					);
				}
			} else if let Some(texture) = texture {
				ui.add(
					egui::Image::new(texture)
						.max_size(available.max(egui::vec2(1.0, 1.0)))
						.corner_radius(8),
				);
				if entry.manifest.kind != ExtensionKind::Theme {
					ui.weak("Creator preview");
				}
			}
			if entry.manifest.kind == ExtensionKind::Theme {
				ui.weak(format!("by {}", entry.manifest.author));
				if !entry.description.is_empty() {
					ui.add(egui::Label::new(&entry.description).wrap());
				}
			}
			close = ui.button("Close preview").clicked();
		});
		if close || modal.should_close() {
			self.enlarged = None;
		}
	}
	pub fn offer_import(&mut self, entry: ExtensionEntry) {
		self.enlarged = None;
		self.consent = Some(Consent {
			entry,
			grants: vec![],
		});
	}
	pub(crate) fn has_result(&self) -> bool {
		self.result.is_some() || self.error.is_some()
	}
	pub fn report_error(&mut self, message: String) {
		let message: String = message.chars().take(512).collect();
		self.status = message.clone();
		self.error = if self.theme_editor_visible && self.theme_editor.is_some() {
			None
		} else {
			Some(message)
		};
	}
	pub fn reset_runtime(&mut self) {
		self.catalog_refreshing = false;
		self.theme_editor = None;
		self.gallery_preview = None;
		self.error = None;
		self.result = None;
		self.consent = None;
		self.requests.clear();
		self.previews.clear();
		self.enlarged = None;
	}
	pub fn remove_runtime(&mut self, id: &str) {
		if self.result.as_ref().is_some_and(|result| result.id == id) {
			self.result = None;
		}
		self.requests.retain(
			|request| !matches!(request, ExtensionRequest::Invoke { id: plugin, .. } if plugin == id),
		);
	}
	pub fn present_output(
		&mut self,
		id: String,
		mut invocation: Invocation,
		context: ExtensionContext,
		mut output: Output,
		state: &State,
	) {
		if !context.is_current(state) {
			self.status = "Result discarded because the conversation or draft changed.".into();
			return;
		}
		invocation.storage = None;
		invocation.app = None;
		output.storage = None;
		self.result = Some(ResultPanel {
			id,
			invocation,
			context,
			output,
			values: BTreeMap::new(),
		});
	}
	pub(crate) fn queue(&mut self, ctx: &egui::Context, request: ExtensionRequest) {
		if matches!(request, ExtensionRequest::PreviewTheme { .. }) {
			self.requests
				.retain(|pending| !matches!(pending, ExtensionRequest::PreviewTheme { .. }));
		}
		if self.requests.len()
			< if matches!(&request, ExtensionRequest::Disable { .. }) {
				extensions::MAX_PLUGINS * 2
			} else {
				4
			} && request_bytes(&request)
			.saturating_add(self.requests.iter().map(request_bytes).sum::<usize>())
			<= if matches!(
				request,
				ExtensionRequest::SaveTheme { .. }
					| ExtensionRequest::ExportTheme { .. }
					| ExtensionRequest::PreviewTheme { .. }
			) {
				2 * extensions::MAX_PACKAGE_BYTES
			} else {
				4 * extensions::MAX_IO_BYTES
			} {
			self.requests.push(request);
			ctx.request_repaint();
		} else {
			self.status =
				"Extensions are busy. Try again after the current action finishes.".into();
		}
	}
	pub(crate) fn message_actions(&self) -> Arc<Vec<MenuAction>> {
		self.message_actions.clone()
	}
	pub(crate) fn invoke_message(
		&mut self,
		action: MenuAction,
		text: String,
		state: &State,
		ctx: &egui::Context,
	) {
		self.queue(
			ctx,
			ExtensionRequest::Invoke {
				id: action.plugin,
				invocation: Invocation {
					action: action.action,
					selected_message: Some(text),
					..Default::default()
				},
				context: ExtensionContext::capture(state, false),
			},
		);
	}
	pub(crate) fn composer_menu(&mut self, ui: &mut egui::Ui, state: &State) {
		if !self.entries.iter().any(|entry| {
			entry.enabled
				&& entry
					.manifest
					.actions
					.iter()
					.any(|action| matches!(action.surface, Surface::Composer | Surface::Panel))
		}) {
			return;
		}
		let mut selected = None;
		ui.menu_button("Tools", |ui| {
			for entry in self.entries.iter().filter(|entry| entry.enabled) {
				for action in
					entry.manifest.actions.iter().filter(|action| {
						matches!(action.surface, Surface::Composer | Surface::Panel)
					}) {
					if ui
						.add_enabled(
							!self.busy,
							egui::Button::new(format!(
								"{} · {}",
								entry.manifest.name, action.label
							)),
						)
						.clicked()
					{
						selected = Some((
							entry.manifest.id.clone(),
							action.id.clone(),
							action.surface == Surface::Composer,
						));
						ui.close();
					}
				}
			}
		});
		if let Some((id, action, composer)) = selected {
			let context = if composer {
				ExtensionContext::capture(state, true)
			} else {
				ExtensionContext::panel(state)
			};
			let invocation = Invocation {
				action,
				composer: context.draft.clone(),
				..Default::default()
			};
			self.queue(
				ui.ctx(),
				ExtensionRequest::Invoke {
					id,
					invocation,
					context,
				},
			);
		}
	}
	fn toolbar(&mut self, ui: &mut egui::Ui, colors: &design::Palette) {
		let height = 38.0;
		// Fixed action widths keep the buttons still while the search field flexes.
		let actions = if self.themes {
			132.0 + 8.0 + 84.0
		} else {
			92.0 + 8.0 + 150.0
		};
		let working = self.busy || self.catalog_refreshing;
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing.x = 8.0;
			ui.spacing_mut().interact_size.y = height;
			let field = (ui.available_width() - actions - 8.0).max(160.0);
			egui::Frame::new()
				.fill(colors.base)
				.corner_radius(9)
				.stroke(egui::Stroke::new(1.0, colors.border))
				.inner_margin(egui::Margin::symmetric(10, 0))
				.show(ui, |ui| {
					ui.set_width(field - 20.0);
					ui.set_height(height);
					ui.horizontal_centered(|ui| {
						ui.spacing_mut().item_spacing.x = 8.0;
						let (icon, _) =
							ui.allocate_exact_size(egui::Vec2::splat(15.0), egui::Sense::hover());
						icons::paint(ui.painter(), icons::Icon::Search, icon, colors.muted);
						// Progress and clear affordances live inside the field so the
						// buttons beside it never shift.
						let trailing =
							24.0 * f32::from(u8::from(working) + u8::from(!self.query.is_empty()));
						ui.add(
							egui::TextEdit::singleline(&mut self.query)
								.hint_text(if self.themes {
									"Search themes"
								} else {
									"Search extensions"
								})
								.char_limit(128)
								.frame(egui::Frame::NONE)
								.desired_width((ui.available_width() - trailing).max(60.0)),
						);
						if working {
							ui.add(egui::Spinner::new().size(16.0).color(colors.muted))
								.on_hover_text(if self.catalog_refreshing {
									"Checking for packages and updates"
								} else {
									"Working on your last action"
								});
						}
						if !self.query.is_empty() {
							let (rect, response) = ui
								.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::click());
							icons::paint(
								ui.painter(),
								icons::Icon::Close,
								rect,
								if response.hovered() {
									colors.text_strong
								} else {
									colors.muted
								},
							);
							if response.on_hover_text("Clear search").clicked() {
								self.query.clear();
							}
						}
					});
				});
			if self.themes {
				if toolbar_button(
					ui,
					"Create theme",
					true,
					!self.busy,
					egui::vec2(132.0, height),
					colors,
				)
				.clicked()
				{
					self.status.clear();
					self.theme_editor = Some(crate::theme_editor::ThemeEditor::new());
				}
				let more =
					toolbar_button(ui, "More", false, true, egui::vec2(84.0, height), colors);
				egui::Popup::menu(&more).show(|ui| {
					ui.set_min_width(176.0);
					if ui
						.add_enabled(!self.busy, egui::Button::new("Import theme…"))
						.clicked()
					{
						self.status.clear();
						self.queue(ui.ctx(), ExtensionRequest::Import);
						ui.close();
					}
					if ui
						.add_enabled(!self.busy, egui::Button::new("Refresh catalog"))
						.clicked()
					{
						self.previews.clear();
						self.status.clear();
						self.queue(ui.ctx(), ExtensionRequest::RefreshCatalog);
						ui.close();
					}
				});
				return;
			}
			if toolbar_button(
				ui,
				"Refresh",
				false,
				!self.busy,
				egui::vec2(92.0, height),
				colors,
			)
			.on_hover_text("Look for new packages and updates. Nothing installs on its own.")
			.clicked()
			{
				self.previews.clear();
				self.status.clear();
				self.queue(ui.ctx(), ExtensionRequest::RefreshCatalog);
			}
			if toolbar_button(
				ui,
				"Import package…",
				true,
				!self.busy,
				egui::vec2(150.0, height),
				colors,
			)
			.on_hover_text("Open a package file from this computer.")
			.clicked()
			{
				self.status.clear();
				self.queue(ui.ctx(), ExtensionRequest::Import);
			}
		});
		if !self.status.is_empty() {
			ui.add_space(10.0);
			let mut dismiss = false;
			egui::Frame::new()
				.fill(colors.raised)
				.corner_radius(9)
				.stroke(egui::Stroke::new(1.0, colors.border))
				.inner_margin(egui::Margin::symmetric(12, 9))
				.show(ui, |ui| {
					ui.set_width((ui.available_width() - 24.0).max(1.0));
					ui.horizontal_top(|ui| {
						ui.spacing_mut().item_spacing.x = 8.0;
						let text = (ui.available_width() - 24.0).max(1.0);
						ui.allocate_ui(egui::vec2(text, 0.0), |ui| {
							ui.add(
								egui::Label::new(
									egui::RichText::new(&self.status)
										.size(12.5)
										.color(colors.text),
								)
								.wrap(),
							);
						});
						let (rect, response) =
							ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::click());
						icons::paint(
							ui.painter(),
							icons::Icon::Close,
							rect,
							if response.hovered() {
								colors.text_strong
							} else {
								colors.muted
							},
						);
						dismiss = response.on_hover_text("Dismiss").clicked();
					});
				});
			if dismiss {
				self.status.clear();
			}
		}
	}
	pub(crate) fn settings(&mut self, ui: &mut egui::Ui, state: &State) {
		self.theme_editor_visible = self.themes;
		if self.themes && self.theme_editor.is_some() {
			self.edit_theme(ui);
			return;
		}
		self.preview_clock = self.preview_clock.saturating_add(1);

		let colors = design::palette(ui);
		self.toolbar(ui, &colors);
		ui.add_space(14.0);
		let mut enable = None;
		let mut disable = None;
		let mut invoke = None;
		let query = self.query.trim().to_lowercase();
		let entries: Vec<_> = self
			.entries
			.iter()
			.enumerate()
			.filter(|(_, entry)| {
				(entry.manifest.kind == ExtensionKind::Theme) == self.themes
					&& (query.is_empty()
						|| [
							&entry.manifest.name,
							&entry.manifest.author,
							&entry.description,
						]
						.into_iter()
						.any(|text| text.to_lowercase().contains(&query)))
			})
			.map(|(index, _)| index)
			.collect();
		let columns = if self.themes && ui.available_width() >= 700.0 {
			3
		} else if ui.available_width() >= if self.themes { 460.0 } else { 560.0 } {
			2
		} else {
			1
		};
		let gap = 16.0;
		let width = ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).max(1.0);
		let card = (width - 2.0).max(1.0);
		let body = (card - 28.0).max(1.0);
		let body_height = if self.themes {
			THEME_CARD_BODY
		} else {
			CARD_BODY
		};
		let card_height = card * 9.0 / 16.0 + body_height;
		// The card frame paints its 1px stroke outside the content, so placeholders for
		// culled rows must match or the scroll extent jitters at the bottom of the list.
		let row_height = card_height + 2.0;
		let top_radius = egui::CornerRadius {
			nw: CARD_RADIUS,
			ne: CARD_RADIUS,
			sw: 0,
			se: 0,
		};
		let mut visible = Vec::new();
		ui.vertical(|ui| {
			ui.spacing_mut().item_spacing.y = gap;
			for row in entries.chunks(columns) {
				let row_rect = egui::Rect::from_min_size(
					ui.cursor().min,
					egui::vec2(ui.available_width(), row_height),
				);
				if !ui.is_rect_visible(row_rect) {
					ui.allocate_space(row_rect.size());
					continue;
				}
				visible.extend_from_slice(row);
				ui.horizontal_top(|ui| {
					ui.spacing_mut().item_spacing.x = gap;
					for index in row {
						let entry = self.entries[*index].clone();
						ui.push_id(&entry.manifest.id, |ui| {
							ui.allocate_ui_with_layout(
								egui::vec2(width, row_height),
								egui::Layout::top_down(egui::Align::Min),
								|ui| {
									egui::Frame::new()
										.fill(colors.raised)
										.corner_radius(CARD_RADIUS)
										.stroke(egui::Stroke::new(
											1.0,
											if self.themes
												&& self.active_theme.as_deref()
													== Some(entry.manifest.id.as_str())
											{
												colors.accent
											} else {
												colors.border
											},
										))
										.show(ui, |ui| {
											ui.set_width(card);
											ui.set_min_height(card_height);
											ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
											self.preview_image(ui, &entry, top_radius);
											egui::Frame::new()
												.inner_margin(egui::Margin {
													left: 14,
													right: 14,
													top: 12,
													bottom: 12,
												})
												.show(ui, |ui| {
													ui.set_width(body);
													ui.set_min_height(body_height - 24.0);
													ui.spacing_mut().item_spacing =
														egui::vec2(6.0, 6.0);
													self.card_body(
														ui,
														&colors,
														&entry,
														&mut enable,
														&mut disable,
														&mut invoke,
													);
												});
										});
								},
							);
						});
					}
				});
			}
		});
		if self
			.previews
			.values()
			.filter(|preview| preview.loading)
			.count() < 4
			&& self
				.previews
				.values()
				.any(|preview| preview.used < self.preview_clock)
			&& visible.iter().any(|index| {
				self.entries[*index].preview.is_some()
					&& !self
						.previews
						.contains_key(&self.entries[*index].manifest.id)
			}) {
			// One settling frame lets a newly visible card replace a previously visible thumbnail.
			ui.ctx().request_repaint();
		}
		if entries.is_empty() {
			egui::Frame::new()
				.fill(colors.raised)
				.corner_radius(CARD_RADIUS)
				.stroke(egui::Stroke::new(1.0, colors.border))
				.inner_margin(28)
				.show(ui, |ui| {
					ui.set_width((ui.available_width() - 58.0).max(1.0));
					ui.vertical_centered(|ui| {
						let (rect, _) =
							ui.allocate_exact_size(egui::Vec2::splat(46.0), egui::Sense::hover());
						ui.painter()
							.circle_filled(rect.center(), 23.0, colors.sidebar);
						icons::paint(
							ui.painter(),
							if query.is_empty() {
								icons::Icon::Sparkle
							} else {
								icons::Icon::Search
							},
							egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(22.0)),
							colors.muted,
						);
						ui.add_space(12.0);
						ui.label(
							design::semibold(
								ui,
								match (query.is_empty(), self.themes) {
									(false, _) => "No matches",
									(true, true) => "No themes yet",
									(true, false) => "No extensions yet",
								},
								17.0,
							)
							.color(colors.text_strong),
						);
						ui.add_space(3.0);
						ui.label(
							egui::RichText::new(if query.is_empty() {
								"Refresh the catalog or import a creator's package to get started."
							} else {
								"Try a different name or creator."
							})
							.size(13.0)
							.color(colors.muted),
						);
					});
				});
		}
		self.preview_modal(ui.ctx());
		if let Some(entry) = enable {
			self.offer_import(entry);
		}
		if let Some(id) = disable {
			self.remove_runtime(&id);
			self.queue(ui.ctx(), ExtensionRequest::Disable { id });
		}
		if let Some((id, action)) = invoke {
			self.queue(
				ui.ctx(),
				ExtensionRequest::Invoke {
					id,
					invocation: Invocation {
						action,
						..Default::default()
					},
					context: ExtensionContext::panel(state),
				},
			);
		}
		self.consent_modal(ui.ctx(), &colors);
	}
	fn card_body(
		&mut self,
		ui: &mut egui::Ui,
		colors: &design::Palette,
		entry: &ExtensionEntry,
		enable: &mut Option<ExtensionEntry>,
		disable: &mut Option<String>,
		invoke: &mut Option<(String, String)>,
	) {
		let active = self.active_theme.as_deref() == Some(entry.manifest.id.as_str());
		let mut update_requested = false;
		if self.themes {
			// Name with the status chip or update link at the right edge.
			ui.horizontal(|ui| {
				ui.spacing_mut().interact_size.y = 20.0;
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if entry.cleanup_pending {
						badge(
							ui,
							"Cleanup pending",
							colors.warning,
							design::mix(colors.raised, colors.warning, 0.16),
						);
					} else if entry.enabled && active {
						badge(ui, "Active", colors.accent_text, colors.accent);
					} else if entry.enabled && entry.update_available {
						update_requested = ui
							.add_enabled(
								!self.busy,
								egui::Button::new(
									egui::RichText::new("Update")
										.size(11.5)
										.color(colors.accent),
								)
								.frame(false),
							)
							.on_hover_text(
								"Review the new release before it replaces this version.",
							)
							.clicked();
					}
					ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
						ui.add(
							egui::Label::new(
								design::semibold(ui, &entry.manifest.name, 16.0)
									.color(colors.text_strong),
							)
							.truncate(),
						);
					});
				});
			});
			ui.spacing_mut().item_spacing.y = 2.0;
			// Creator line; the destructive verb stays quiet at the right instead of
			// competing with the footer buttons.
			ui.horizontal(|ui| {
				ui.spacing_mut().interact_size.y = 18.0;
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if entry.enabled
						&& !entry.cleanup_pending
						&& ui
							.add_enabled_ui(!self.busy, |ui| {
								quiet_action(ui, "Remove", colors.muted, colors.danger)
							})
							.inner
							.on_hover_text("Remove this theme and delete its local data.")
							.clicked()
					{
						*disable = Some(entry.manifest.id.clone());
					}
					ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
						ui.add(
							egui::Label::new(
								egui::RichText::new(format!("by {}", entry.manifest.author))
									.size(12.0)
									.color(colors.muted),
							)
							.truncate(),
						)
						.on_hover_text(&entry.manifest.author);
					});
				});
			});
		} else {
			ui.horizontal(|ui| {
				ui.label(design::eyebrow(ui, "Plugin", colors.muted));
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					if entry.cleanup_pending {
						badge(
							ui,
							"Cleanup pending",
							colors.warning,
							design::mix(colors.raised, colors.warning, 0.16),
						);
					} else if entry.enabled {
						badge(
							ui,
							"Enabled",
							colors.positive,
							design::mix(colors.raised, colors.positive, 0.16),
						);
						if entry.update_available {
							badge(
								ui,
								"Update",
								colors.accent,
								design::mix(colors.raised, colors.accent, 0.2),
							);
						}
					}
				});
			});
			ui.add(
				egui::Label::new(
					design::semibold(ui, &entry.manifest.name, 17.0).color(colors.text_strong),
				)
				.truncate(),
			);
			ui.spacing_mut().item_spacing.y = 2.0;
			ui.add(
				egui::Label::new(
					egui::RichText::new(format!("by {}", entry.manifest.author))
						.size(12.0)
						.color(colors.muted),
				)
				.truncate(),
			)
			.on_hover_text(&entry.manifest.author);
			ui.spacing_mut().item_spacing.y = 8.0;
			let description = if entry.description.is_empty() {
				"Add a new tool to your conversations."
			} else {
				&entry.description
			};
			ui.allocate_ui(egui::vec2(ui.available_width(), 36.0), |ui| {
				let mut text = egui::text::LayoutJob::simple(
					description.into(),
					egui::FontId::proportional(13.0),
					colors.text,
					ui.available_width(),
				);
				text.wrap.max_rows = 2;
				ui.label(text);
			});
		}
		let body_height = if self.themes {
			THEME_CARD_BODY
		} else {
			CARD_BODY
		};
		let footer = ui.min_rect().top() + body_height - 24.0 - FOOTER_HEIGHT;
		ui.add_space((footer - ui.cursor().top()).max(0.0));
		let neutral = design::mix(colors.raised, colors.text, 0.1);
		let outline = egui::Stroke::new(1.0, colors.border);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 8.0;
			if entry.cleanup_pending {
				if card_button(
					ui,
					"Retry cleanup",
					egui::vec2(ui.available_width(), FOOTER_HEIGHT),
					neutral,
					outline,
					colors.text_strong,
				)
				.on_hover_text("Finish removing this extension and its local data.")
				.clicked()
				{
					*disable = Some(entry.manifest.id.clone());
				}
				return;
			}
			if !entry.enabled {
				// Catalog themes can still be opened as a copy in the editor before installing.
				let customizable = self.themes && entry.theme_preview.is_some();
				let width = if customizable {
					((ui.available_width() - 8.0) / 2.0).max(1.0)
				} else {
					ui.available_width().max(1.0)
				};
				if ui
					.add_enabled_ui(!self.busy, |ui| {
						card_button(
							ui,
							if self.themes {
								"Install theme"
							} else {
								"Review & enable"
							},
							egui::vec2(width, FOOTER_HEIGHT),
							if self.themes { neutral } else { colors.accent },
							if self.themes {
								outline
							} else {
								egui::Stroke::NONE
							},
							if self.themes {
								colors.text_strong
							} else {
								colors.accent_text
							},
						)
					})
					.inner
					.clicked()
				{
					*enable = Some(entry.clone());
				}
				if customizable
					&& ui
						.add_enabled_ui(!self.busy && self.theme_editor.is_none(), |ui| {
							card_button(
								ui,
								"Customize",
								egui::vec2(ui.available_width().max(1.0), FOOTER_HEIGHT),
								colors.raised,
								outline,
								colors.text_strong,
							)
						})
						.inner
						.clicked()
				{
					self.queue(
						ui.ctx(),
						ExtensionRequest::EditTheme {
							id: entry.manifest.id.clone(),
							preview: false,
						},
					);
				}
				return;
			}
			if self.themes {
				// Apply first, customize second; both share the footer as equal halves.
				let customizable = entry.theme_preview.is_some();
				if !active {
					let width = if customizable {
						((ui.available_width() - 8.0) / 2.0).max(1.0)
					} else {
						ui.available_width().max(1.0)
					};
					if ui
						.add_enabled_ui(!self.busy, |ui| {
							card_button(
								ui,
								"Use theme",
								egui::vec2(width, FOOTER_HEIGHT),
								colors.accent,
								egui::Stroke::NONE,
								colors.accent_text,
							)
						})
						.inner
						.on_hover_text("Apply this installed theme to the app.")
						.clicked()
					{
						self.queue(
							ui.ctx(),
							ExtensionRequest::SelectTheme {
								id: Some(entry.manifest.id.clone()),
							},
						);
					}
				}
				if customizable
					&& ui
						.add_enabled_ui(!self.busy && self.theme_editor.is_none(), |ui| {
							card_button(
								ui,
								if entry.local_theme {
									"Edit theme"
								} else {
									"Customize"
								},
								egui::vec2(ui.available_width().max(1.0), FOOTER_HEIGHT),
								if active { neutral } else { colors.raised },
								outline,
								colors.text_strong,
							)
						})
						.inner
						.clicked()
				{
					self.queue(
						ui.ctx(),
						ExtensionRequest::EditTheme {
							id: entry.manifest.id.clone(),
							preview: false,
						},
					);
				}
				if update_requested {
					let mut update = entry.clone();
					if let Some(manifest) = &entry.update_manifest {
						update.manifest = manifest.clone();
						update.reviewed = true;
					}
					*enable = Some(update);
				}
				return;
			}
			let panel_actions: Vec<_> = entry
				.manifest
				.actions
				.iter()
				.filter(|action| action.surface == Surface::Panel)
				.collect();
			let count =
				1 + usize::from(entry.update_available) + usize::from(!panel_actions.is_empty());
			let each = ((ui.available_width() - 8.0 * (count - 1) as f32) / count as f32).max(1.0);
			if entry.update_available
				&& ui
					.add_enabled_ui(!self.busy, |ui| {
						card_button(
							ui,
							"Update",
							egui::vec2(each, FOOTER_HEIGHT),
							colors.accent,
							egui::Stroke::NONE,
							colors.accent_text,
						)
						.on_hover_text("Review the new release before it replaces this version.")
					})
					.inner
					.clicked()
			{
				let mut update = entry.clone();
				if let Some(manifest) = &entry.update_manifest {
					update.manifest = manifest.clone();
					update.reviewed = true;
				}
				*enable = Some(update);
			}
			if !panel_actions.is_empty() {
				ui.scope(|ui| {
					let visuals = ui.visuals_mut();
					visuals.widgets.inactive.weak_bg_fill = neutral;
					visuals.widgets.hovered.weak_bg_fill = neutral.linear_multiply(1.12);
					visuals.widgets.active.weak_bg_fill = neutral.gamma_multiply(0.85);
					visuals.widgets.open.weak_bg_fill = neutral.linear_multiply(1.12);
					ui.spacing_mut().button_padding.x = (each / 2.0 - 30.0).max(6.0);
					ui.menu_button("Open tool", |ui| {
						for action in &panel_actions {
							if ui
								.add_enabled(!self.busy, egui::Button::new(&action.label))
								.clicked()
							{
								*invoke = Some((entry.manifest.id.clone(), action.id.clone()));
								ui.close();
							}
						}
					});
				});
			}
			if card_button(
				ui,
				"Disable",
				egui::vec2(ui.available_width().max(1.0), FOOTER_HEIGHT),
				neutral,
				outline,
				colors.text_strong,
			)
			.on_hover_text("Removes this extension and deletes its local data.")
			.clicked()
			{
				*disable = Some(entry.manifest.id.clone());
			}
		});
	}
	fn consent_modal(&mut self, ctx: &egui::Context, colors: &design::Palette) {
		let Some(mut consent) = self.consent.take() else {
			return;
		};
		let mut close = false;
		let theme = consent.entry.manifest.kind == ExtensionKind::Theme;
		let illustrated =
			consent.entry.theme_preview.is_some()
				|| consent.entry.manifest.capabilities.iter().any(|cap| {
					matches!(cap, Capability::DeletedMessages | Capability::ImageSharing)
				});
		let response = crate::dialog::Dialog::new(
			"extension-consent",
			if theme {
				"Enable this theme"
			} else {
				"Enable this extension"
			},
		)
		.subtitle("Everything it may touch is listed below.")
		.width(460.0)
		.show(ctx, |d| {
			d.scroll(260.0, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 10.0;
				egui::Frame::new()
					.fill(colors.raised)
					.corner_radius(10)
					.stroke(egui::Stroke::new(1.0, colors.border))
					.inner_margin(12)
					.show(ui, |ui| {
						ui.set_width((ui.available_width() - 26.0).max(1.0));
						ui.horizontal_top(|ui| {
							ui.spacing_mut().item_spacing.x = 12.0;
							let (rect, _) = ui
								.allocate_exact_size(egui::vec2(84.0, 47.0), egui::Sense::hover());
							if illustrated {
								draw_native_preview(
									ui,
									rect,
									&consent.entry,
									egui::CornerRadius::same(6),
								);
							} else {
								ui.painter().rect_filled(rect, 6, colors.sidebar);
								icons::paint(
									ui.painter(),
									icons::Icon::Sparkle,
									egui::Rect::from_center_size(
										rect.center(),
										egui::Vec2::splat(20.0),
									),
									colors.muted,
								);
							}
							ui.vertical(|ui| {
								ui.spacing_mut().item_spacing.y = 3.0;
								ui.add(
									egui::Label::new(
										design::semibold(ui, &consent.entry.manifest.name, 16.0)
											.color(colors.text_strong),
									)
									.truncate(),
								);
								ui.add(
									egui::Label::new(
										egui::RichText::new(format!(
											"by {}",
											consent.entry.manifest.author
										))
										.size(12.0)
										.color(colors.muted),
									)
									.truncate(),
								);
								ui.horizontal_wrapped(|ui| {
									ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
									if consent.entry.reviewed {
										badge(
											ui,
											"Reviewed",
											colors.positive,
											design::mix(colors.raised, colors.positive, 0.16),
										);
									} else {
										badge(
											ui,
											"Unreviewed",
											colors.warning,
											design::mix(colors.raised, colors.warning, 0.16),
										);
									}
									badge(
										ui,
										&format!("v{}", consent.entry.manifest.version),
										colors.muted,
										colors.sidebar.to_opaque(),
									);
									badge(
										ui,
										&consent.entry.manifest.license,
										colors.muted,
										colors.sidebar.to_opaque(),
									);
									badge(
										ui,
										&format!(
											"{:.1} KiB",
											consent.entry.download_bytes as f64 / 1024.0
										),
										colors.muted,
										colors.sidebar.to_opaque(),
									);
								});
								if !consent.entry.manifest.source.is_empty() {
									ui.hyperlink_to(
										egui::RichText::new("View source").size(12.0),
										&consent.entry.manifest.source,
									);
								}
							});
						});
					});
				if !consent.entry.reviewed {
					egui::Frame::new()
						.fill(design::mix(colors.chat, colors.warning, 0.14))
						.corner_radius(10)
						.inner_margin(12)
						.show(ui, |ui| {
							ui.set_width((ui.available_width() - 26.0).max(1.0));
							ui.horizontal_top(|ui| {
								ui.spacing_mut().item_spacing.x = 10.0;
								let (rect, _) = ui.allocate_exact_size(
									egui::Vec2::splat(18.0),
									egui::Sense::hover(),
								);
								icons::paint(
									ui.painter(),
									icons::Icon::ShieldWarning,
									rect,
									colors.warning,
								);
								ui.label(
									egui::RichText::new(
										"Unreviewed package — its source has not been reviewed for the catalog.",
									)
									.size(12.5)
									.color(colors.text),
								);
							});
						});
				}
				if consent.entry.manifest.capabilities.is_empty() {
					egui::Frame::new()
						.fill(colors.raised)
						.corner_radius(10)
						.stroke(egui::Stroke::new(1.0, colors.border))
						.inner_margin(12)
						.show(ui, |ui| {
							ui.set_width((ui.available_width() - 26.0).max(1.0));
							ui.horizontal_top(|ui| {
								ui.spacing_mut().item_spacing.x = 10.0;
								let (rect, _) = ui.allocate_exact_size(
									egui::Vec2::splat(18.0),
									egui::Sense::hover(),
								);
								icons::paint(
									ui.painter(),
									icons::Icon::Check,
									rect,
									colors.positive,
								);
								ui.label(
									egui::RichText::new(
										"No access to conversations or composer text.",
									)
									.size(13.0)
									.color(colors.text),
								);
							});
						});
				} else {
					ui.label(design::eyebrow(ui, "Allow this extension to", colors.muted));
					ui.spacing_mut().item_spacing.y = 8.0;
					for capability in &consent.entry.manifest.capabilities {
						let mut granted = consent.grants.contains(capability);
						let changed = egui::Frame::new()
							.fill(colors.raised)
							.corner_radius(10)
							.stroke(egui::Stroke::new(
								1.0,
								if granted {
									colors.accent
								} else {
									colors.border
								},
							))
							.inner_margin(12)
							.show(ui, |ui| {
								ui.set_width((ui.available_width() - 26.0).max(1.0));
								ui.spacing_mut().icon_width = 20.0;
								ui.spacing_mut().icon_spacing = 10.0;
								ui.visuals_mut().widgets.inactive.bg_stroke =
									egui::Stroke::new(1.0, colors.muted);
								ui.checkbox(
									&mut granted,
									egui::RichText::new(capability_label(*capability)).size(13.5),
								)
								.changed()
							})
							.inner;
						if changed {
							if granted {
								consent.grants.push(*capability);
							} else {
								consent.grants.retain(|grant| grant != capability);
							}
						}
					}
				}
				ui.label(
					egui::RichText::new(
						"Disabling removes the extension and its local data. Re-enabling starts fresh.",
					)
					.size(12.0)
					.color(colors.muted),
				);
			});
			d.footer(|ui| {
				let ready = consent
					.entry
					.manifest
					.capabilities
					.iter()
					.all(|capability| consent.grants.contains(capability));
				let enable = ui
					.add_enabled_ui(ready && !self.busy, |ui| {
						crate::dialog::action(ui, "Enable", crate::dialog::Action::Primary)
					})
					.inner;
				if !ready {
					enable.on_hover_text("Allow every listed permission to continue.");
				} else if enable.clicked() {
					self.queue(
						ctx,
						ExtensionRequest::Enable {
							id: consent.entry.manifest.id.clone(),
							grants: consent.grants.clone(),
							sha256: consent.entry.sha256.clone(),
							reviewed: consent.entry.reviewed,
						},
					);
					close = true;
				}
				close |=
					crate::dialog::action(ui, "Cancel", crate::dialog::Action::Neutral).clicked();
			});
		});
		if !close && !response.close {
			self.consent = Some(consent);
		}
	}
	pub(crate) fn reset_theme_shortcut(&mut self, ctx: &egui::Context) {
		if ctx.input_mut(|input| {
			input.consume_key(
				egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
				egui::Key::F12,
			)
		}) {
			self.reset_theme(ctx);
		}
	}
	pub(crate) fn reset_theme(&mut self, ctx: &egui::Context) {
		self.stop_theme_preview(ctx);
		self.theme_editor = None;
		design::set_background_image(ctx, None);
		let disable: Vec<_> = self
			.entries
			.iter()
			.filter(|entry| {
				(entry.enabled || entry.cleanup_pending)
					&& (entry.cleanup_pending
						|| (entry.manifest.kind == ExtensionKind::Plugin
							&& entry
								.manifest
								.capabilities
								.contains(&Capability::Appearance)))
			})
			.map(|entry| entry.manifest.id.clone())
			.collect();
		design::set_extension_theme(None);
		design::apply(ctx);
		if self.active_theme.is_some() {
			self.queue(ctx, ExtensionRequest::SelectTheme { id: None });
		}
		for id in disable {
			self.queue(ctx, ExtensionRequest::Disable { id });
		}
	}
	pub(crate) fn show_result(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		changes: &mut Vec<Id>,
		editing: bool,
	) -> Option<crate::extension_app::ConfirmedEffect> {
		if let Some(message) = self.error.take() {
			let mut dismissed = false;
			let response = crate::dialog::Dialog::new("extension-error", "Extension error")
				.width(400.0)
				.show(ctx, |d| {
					d.content(|ui| {
						crate::dialog::notice(ui, crate::dialog::Level::Error, &message);
					});
					d.footer(|ui| {
						dismissed =
							crate::dialog::action(ui, "Dismiss", crate::dialog::Action::Primary)
								.clicked();
					});
				});
			if !dismissed && !response.close {
				self.error = Some(message);
			}
		}
		let mut result = self.result.take()?;
		if !result.context.is_current(state) {
			self.status = "Result discarded because the conversation or draft changed.".into();
			return None;
		}
		let mut applied = false;
		let mut confirmed = None;
		let mut action = None;
		let title = self
			.entries
			.iter()
			.find(|entry| entry.manifest.id == result.id)
			.map_or("Extension tool", |entry| entry.manifest.name.as_str())
			.to_owned();
		let mut close = false;
		let response = crate::dialog::Dialog::new("extension-result", title)
			.subtitle("Review the result. App actions and draft changes need your approval.")
			.width(520.0)
			.show(ctx, |d| {
				d.scroll(240.0, |ui| {
					if let Some(replacement) = &result.output.replacement {
						crate::dialog::label(ui, "Proposed composer text");
						let colors = crate::design::palette(ui);
						egui::Frame::new()
							.fill(colors.base)
							.stroke(egui::Stroke::new(1.0, colors.border))
							.corner_radius(8)
							.inner_margin(egui::Margin::symmetric(12, 10))
							.show(ui, |ui| {
								ui.set_width(ui.available_width());
								ui.add(egui::Label::new(replacement).wrap());
							});
						ui.add_space(10.0);
					}
					if let Some(effect) = result.output.effects.first() {
						crate::dialog::label(ui, "Proposed app action");
						ui.add(
							egui::Label::new(crate::extension_app::effect_description(effect))
								.wrap(),
						);
						ui.add_space(10.0);
					}
					render_elements(ui, &result.output.panel, &mut result.values, &mut action);
				});
				d.footer(|ui| {
					if let Some(effect) = result.output.effects.first()
						&& crate::dialog::action(
							ui,
							crate::extension_app::effect_button(effect),
							crate::dialog::Action::Primary,
						)
						.clicked()
					{
						confirmed = Some(crate::extension_app::ConfirmedEffect {
							plugin: result.id.clone(),
							context: result.context.clone(),
							effect: effect.clone(),
						});
						applied = true;
					}
					if let Some(replacement) = result.output.replacement.clone() {
						ui.add_enabled_ui(!editing && result.context.draft.is_some(), |ui| {
							if crate::dialog::action(
								ui,
								"Apply to Draft",
								crate::dialog::Action::Primary,
							)
							.clicked()
							{
								if apply_proposal(state, &result.context, &replacement) {
									if let Some(channel) = state.selected {
										changes.push(channel);
									}
									applied = true;
								} else {
									self.status =
										"The draft changed or the proposal exceeds the draft limit."
											.into();
								}
							}
						});
					}
					close |= crate::dialog::action(ui, "Close", crate::dialog::Action::Neutral)
						.clicked();
				});
			});
		if let Some(action) = action {
			let mut invocation = result.invocation.clone();
			invocation.action = action;
			invocation.composer = None;
			invocation.selected_message = None;
			invocation.values = result.values.clone();
			self.queue(
				ctx,
				ExtensionRequest::Invoke {
					id: result.id.clone(),
					invocation,
					context: result.context.clone(),
				},
			);
		}
		if !close && !response.close && !applied {
			self.result = Some(result);
		}
		confirmed
	}
}
fn request_bytes(request: &ExtensionRequest) -> usize {
	std::mem::size_of_val(request)
		+ match request {
			ExtensionRequest::PickThemeImage => 0,
			ExtensionRequest::PickThemeCover => 0,
			ExtensionRequest::EditTheme { id, .. } => id.len(),
			ExtensionRequest::SaveTheme { package } | ExtensionRequest::ExportTheme { package } => {
				package.background_image.len()
					+ package.cover_image.len()
					+ package.wasm.len()
					+ 16384
			}
			ExtensionRequest::PreviewTheme { theme, image } => {
				usize::from(theme.is_some()) * 16384
					+ image.as_ref().map_or(0, |image| image.pixels.len() * 4)
			}
			ExtensionRequest::RefreshCatalog | ExtensionRequest::Import => 0,
			ExtensionRequest::SelectTheme { id } => id.as_ref().map_or(0, String::len),
			ExtensionRequest::Enable {
				id, sha256, grants, ..
			} => id.len() + sha256.len() + grants.len() * std::mem::size_of::<Capability>(),
			ExtensionRequest::Disable { id } | ExtensionRequest::Preview { id } => id.len(),
			ExtensionRequest::Invoke {
				id,
				invocation,
				context,
			} => {
				id.len()
					+ invocation.action.len()
					+ invocation.app.as_ref().map_or(0, |app| {
						std::mem::size_of::<extensions::AppSnapshot>()
							+ app.bytes().unwrap_or(4 * extensions::MAX_IO_BYTES)
					}) + invocation.message_event.as_ref().map_or(0, |event| {
					std::mem::size_of::<extensions::MessageEvent>()
						+ event.channel_id.len()
						+ event.message_id.len()
						+ event.author_id.as_ref().map_or(0, String::len)
						+ event.content.as_ref().map_or(0, String::len)
				}) + [
					&invocation.selected_message,
					&invocation.composer,
					&invocation.storage,
					&context.draft,
				]
				.into_iter()
				.flatten()
				.map(String::len)
				.sum::<usize>() + invocation
					.values
					.iter()
					.map(|(key, value)| key.len() + value.len())
					.sum::<usize>()
			}
		}
}

/// Draw a small native conversation using the package's real palette, without image IO.
fn draw_native_preview(
	ui: &egui::Ui,
	rect: egui::Rect,
	entry: &ExtensionEntry,
	radius: egui::CornerRadius,
) {
	let colors = entry.theme_preview.as_ref().map_or_else(
		|| design::palette(ui),
		|theme| design::theme_preview_palette(ui, theme),
	);
	let painter = ui.painter().with_clip_rect(rect);
	let at = |x: f32, y: f32| rect.min + egui::vec2(rect.width() * x, rect.height() * y);
	let panel = |x, y, w, h, color| {
		painter.rect_filled(
			egui::Rect::from_min_max(at(x, y), at(x + w, y + h)),
			3,
			color,
		);
	};
	let plate = |x: f32, y: f32, w: f32, h: f32, color, radius: egui::CornerRadius| {
		painter.rect_filled(
			egui::Rect::from_min_max(at(x, y), at(x + w, y + h)),
			radius,
			color,
		);
	};
	if entry
		.manifest
		.capabilities
		.contains(&Capability::ImageSharing)
	{
		plate(0.0, 0.0, 1.0, 1.0, colors.chat, radius);
		panel(0.08, 0.12, 0.84, 0.68, colors.raised);
		let center = at(0.5, 0.40);
		let r = rect.height() * 0.15;
		painter.circle_filled(center, r, colors.accent);
		for x in [-0.35, 0.35] {
			painter.circle_filled(center + egui::vec2(r * x, -r * 0.2), r * 0.09, colors.chat);
		}
		painter.line_segment(
			[
				center + egui::vec2(-r * 0.3, r * 0.35),
				center + egui::vec2(r * 0.3, r * 0.35),
			],
			egui::Stroke::new((r * 0.08).max(1.0), colors.chat),
		);
		if rect.width() >= 150.0 {
			painter.text(
				at(0.5, 0.68),
				egui::Align2::CENTER_CENTER,
				"Image attachment",
				egui::FontId::proportional((rect.width() / 24.0).clamp(10.0, 20.0)),
				colors.text_strong,
			);
		}
		panel(0.08, 0.86, 0.69, 0.07, colors.raised);
		panel(0.81, 0.86, 0.11, 0.07, colors.accent);
		return;
	}
	// Only the outer plates carry the card's rounding so the preview can sit flush.
	plate(0.0, 0.0, 1.0, 1.0, colors.base, radius);
	panel(0.08, 0.0, 0.22, 1.0, colors.sidebar);
	plate(
		0.30,
		0.0,
		0.70,
		1.0,
		colors.chat,
		egui::CornerRadius {
			nw: 0,
			ne: radius.ne,
			sw: 0,
			se: radius.se,
		},
	);
	// Below thumbnail width the glyphs would collide, so the layout reads as bars alone.
	let labelled = rect.width() >= 150.0;
	let font = egui::FontId::proportional((rect.width() / 29.0).clamp(9.0, 20.0));
	if labelled {
		painter.text(
			at(0.35, 0.09),
			egui::Align2::LEFT_CENTER,
			"# general",
			font.clone(),
			colors.text_strong,
		);
	} else {
		panel(0.35, 0.07, 0.22, 0.04, colors.text_strong);
	}
	for i in 0..3 {
		let y = 0.25 + i as f32 * 0.20;
		let deleted = i == 1
			&& entry
				.manifest
				.capabilities
				.contains(&Capability::DeletedMessages);
		if deleted {
			panel(
				0.31,
				y - 0.045,
				0.68,
				0.17,
				colors.danger.gamma_multiply(0.12),
			);
		}
		painter.circle_filled(
			at(0.355, y),
			rect.height() * 0.035,
			if deleted {
				colors.danger
			} else {
				colors.accent
			},
		);
		if labelled && (entry.theme_preview.is_none() || rect.width() >= 500.0) {
			painter.text(
				at(0.41, y),
				egui::Align2::LEFT_CENTER,
				if deleted {
					"Deleted message"
				} else {
					["Robin", "Alex", "You"][i]
				},
				font.clone(),
				if deleted {
					colors.danger
				} else {
					colors.text_strong
				},
			);
		} else {
			panel(
				0.41,
				y - 0.015,
				if deleted { 0.30 } else { 0.18 },
				0.035,
				if deleted {
					colors.danger
				} else {
					colors.text_strong
				},
			);
		}
		panel(
			0.41,
			y + 0.06,
			if i == 1 { 0.42 } else { 0.31 },
			0.025,
			if deleted { colors.danger } else { colors.muted },
		);
		panel(0.025, y, 0.025, 0.05, colors.accent);
		panel(0.115, y, 0.14, 0.025, colors.muted);
	}
	panel(0.33, 0.86, 0.64, 0.09, colors.raised);
	panel(0.89, 0.88, 0.05, 0.05, colors.accent);
}

/// Card action button: fixed size, hover feedback and the shared button typography.
fn card_button(
	ui: &mut egui::Ui,
	label: &str,
	size: egui::Vec2,
	fill: egui::Color32,
	stroke: egui::Stroke,
	text: egui::Color32,
) -> egui::Response {
	let enabled = ui.is_enabled();
	paint_button(ui, label, size, fill, stroke, text, enabled)
}

fn paint_button(
	ui: &mut egui::Ui,
	label: &str,
	size: egui::Vec2,
	fill: egui::Color32,
	stroke: egui::Stroke,
	text: egui::Color32,
	enabled: bool,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(
		size,
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
	let fill = if !enabled {
		fill.gamma_multiply(0.5)
	} else if response.is_pointer_button_down_on() {
		fill.gamma_multiply(0.85)
	} else if response.hovered() {
		fill.linear_multiply(1.12)
	} else {
		fill
	};
	let text = if enabled {
		text
	} else {
		text.gamma_multiply(0.6)
	};
	let painter = ui.painter();
	painter.rect(rect, 8, fill, stroke, egui::StrokeKind::Inside);
	let galley = painter.layout_no_wrap(
		label.to_owned(),
		egui::FontId::new(13.5, design::medium_family(ui.ctx())),
		text,
	);
	painter.galley(rect.center() - galley.size() / 2.0, galley, text);
	if enabled && response.hovered() {
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
	}
	response
}

/// Toolbar action: accent-filled for the page's main verb, bordered for the rest.
fn toolbar_button(
	ui: &mut egui::Ui,
	label: &str,
	primary: bool,
	enabled: bool,
	size: egui::Vec2,
	colors: &design::Palette,
) -> egui::Response {
	let (fill, stroke, text) = if primary {
		(colors.accent, egui::Stroke::NONE, colors.accent_text)
	} else {
		(
			colors.raised,
			egui::Stroke::new(1.0, colors.border),
			colors.text_strong,
		)
	};
	// No enabled-scope here: a child scope inside a wrapping toolbar row cannot wrap,
	// so it would widen the page instead of moving to the next row.
	paint_button(ui, label, size, fill, stroke, text, enabled)
}

/// Quiet inline verb: small muted text that takes `hover` colour and an underline when hot.
fn quiet_action(
	ui: &mut egui::Ui,
	label: &str,
	color: egui::Color32,
	hover: egui::Color32,
) -> egui::Response {
	let galley = ui.painter().layout_no_wrap(
		label.to_owned(),
		egui::FontId::new(12.0, design::medium_family(ui.ctx())),
		color,
	);
	let (rect, response) =
		ui.allocate_exact_size(galley.size() + egui::vec2(8.0, 4.0), egui::Sense::click());
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
	let enabled = ui.is_enabled();
	let hot = enabled && (response.hovered() || response.has_focus());
	let color = if !enabled {
		color.gamma_multiply(0.5)
	} else if hot {
		hover
	} else {
		color
	};
	let painter = ui.painter();
	let origin = rect.center() - galley.size() / 2.0;
	painter.galley_with_override_text_color(origin, galley.clone(), color);
	if hot {
		painter.hline(
			origin.x..=origin.x + galley.size().x,
			origin.y + galley.size().y + 1.0,
			egui::Stroke::new(1.0, color),
		);
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
	}
	response
}

/// Small rounded status chip used on cards and in the consent modal.
fn badge(ui: &mut egui::Ui, text: &str, foreground: egui::Color32, background: egui::Color32) {
	let galley = ui.painter().layout_no_wrap(
		text.to_owned(),
		egui::FontId::proportional(11.0),
		foreground,
	);
	let (rect, _) =
		ui.allocate_exact_size(galley.size() + egui::vec2(16.0, 6.0), egui::Sense::hover());
	ui.painter().rect_filled(rect, 7, background);
	ui.painter()
		.galley(rect.center() - galley.size() / 2.0, galley, foreground);
}

fn capability_label(capability: Capability) -> &'static str {
	match capability {
		Capability::ImageSharing => "Enable explicit emoji and sticker image attachment selection",
		Capability::Appearance => "Customize app colors, typography and control styling",
		Capability::MessageEvents => "Read live message events and text in the active conversation",
		Capability::SelectedMessage => "Read the message I choose for an action",
		Capability::Composer => "Read my draft and propose text changes",
		Capability::Storage => "Store up to 1 MiB of local data for this account",
		Capability::AppContext => "Read my account and current conversation details",
		Capability::AccountProfile => "Read my loaded profile, including biography and pronouns",
		Capability::GuildDirectory => "Read my loaded server names and identifiers",
		Capability::ChannelDetails => "Read current channel metadata, recipients and permissions",
		Capability::DataEvents => {
			"Receive changes to separately granted account and conversation data"
		}
		Capability::MessageContent => {
			"Read loaded embed text, stickers and message reference metadata"
		}
		Capability::ForumData => "Read loaded forum and thread summaries",
		Capability::ConversationActivity => {
			"Read current typing users and loaded pins; observe reactions"
		}
		Capability::ChannelMetadata => {
			"Read loaded channel topics, categories, thread details and permissions"
		}
		Capability::MemberDetails => "Read loaded server members, roles and server profiles",
		Capability::ChannelDirectory => "Read the list of loaded, readable conversations",
		Capability::MessageDetails => {
			"Read loaded message replies, mentions, attachment metadata and reactions"
		}
		Capability::Relationships => "Read my loaded friends, requests, blocked and ignored users",
		Capability::Timeline => "Read loaded messages in the active conversation",
		Capability::Members => "Read loaded members of the active conversation",
		Capability::Presence => "Read loaded user presence status",
		Capability::VoiceState => "Read current call state and participant identifiers",
		Capability::ReadState => "Read unread and mention counts in the active conversation",
		Capability::LocalSettings => "Read local reading settings and propose changes for approval",
		Capability::NotificationSettings => {
			"Read local sound and notification settings and propose changes for approval"
		}
		Capability::Navigation => "Propose opening conversations, profiles, search and app views",
		Capability::LocalNotices => "Propose local notices for approval",
		Capability::ClipboardWrite => "Propose clipboard text for approval",
		Capability::VoiceControl => "Propose muting, deafening or leaving my call for approval",
		Capability::AppEvents => "Receive app lifecycle and navigation events while enabled",
		Capability::DeletedMessages => {
			"Keep loaded deleted messages in session memory while enabled"
		}
	}
}
fn render_elements(
	ui: &mut egui::Ui,
	elements: &[Element],
	values: &mut BTreeMap<String, String>,
	action: &mut Option<String>,
) {
	for element in elements {
		match element {
			Element::Text { text } => {
				ui.add(egui::Label::new(text).wrap());
			}
			Element::Heading { text } => {
				ui.add(egui::Label::new(egui::RichText::new(text).heading()).wrap());
			}
			Element::Separator => {
				ui.separator();
			}
			Element::Row { children } => {
				ui.horizontal_wrapped(|ui| render_elements(ui, children, values, action));
			}
			Element::Button { id, label } => {
				if ui.push_id(id, |ui| ui.button(label)).inner.clicked() {
					*action = Some(id.clone());
				}
			}
			Element::TextInput { id, label, value } => {
				let value = values.entry(id.clone()).or_insert_with(|| value.clone());
				let label = ui.label(label);
				ui.add(
					egui::TextEdit::singleline(value)
						.id_salt(id)
						.char_limit(1024)
						.desired_width(ui.available_width().min(320.0)),
				)
				.labelled_by(label.id);
			}
			Element::Checkbox { id, label, checked } => {
				let value = values
					.entry(id.clone())
					.or_insert_with(|| checked.to_string());
				let mut checked = value == "true";
				if ui
					.push_id(id, |ui| ui.checkbox(&mut checked, label))
					.inner
					.changed()
				{
					*value = checked.to_string();
				}
			}
			Element::Select {
				id,
				label,
				options,
				value,
			} => {
				let selected = values.entry(id.clone()).or_insert_with(|| value.clone());
				let label = ui.label(label);
				egui::ComboBox::from_id_salt(id)
					.selected_text(selected.as_str())
					.show_ui(ui, |ui| {
						for option in options {
							ui.selectable_value(selected, option.clone(), option);
						}
					})
					.response
					.labelled_by(label.id);
			}
			Element::Slider {
				id,
				label,
				min,
				max,
				value,
			} => {
				let stored = values
					.entry(id.clone())
					.or_insert_with(|| value.to_string());
				let mut value = stored.parse::<i32>().unwrap_or(*value).clamp(*min, *max);
				if ui
					.push_id(id, |ui| {
						ui.add(egui::Slider::new(&mut value, *min..=*max).text(label))
					})
					.inner
					.changed()
				{
					*stored = value.to_string();
				}
			}
		}
	}
}
fn apply_proposal(state: &mut State, context: &ExtensionContext, replacement: &str) -> bool {
	let Some(channel) = state.selected else {
		return false;
	};
	let old_bytes = state.drafts.get(&channel).map_or(0, String::len);
	if context.draft.is_none()
		|| !context.is_current(state)
		|| replacement.chars().count() > MAX_CONTENT
		|| state
			.draft_bytes()
			.saturating_sub(old_bytes)
			.saturating_add(replacement.len())
			> MAX_DRAFT_BYTES
	{
		return false;
	}
	if replacement.is_empty() {
		state.drafts.remove(&channel);
	} else {
		state.drafts.insert(channel, replacement.to_owned());
	}
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn app_snapshot_payloads_are_bounded_before_ui_queueing() {
		let ctx = egui::Context::default();
		let state = test_support::demo_state();
		let mut shop = ExtensionUi::default();
		let oversized = extensions::AppSnapshot {
			context: Some(extensions::AppContextSnapshot {
				connected: true,
				user: Some(extensions::UserSnapshot {
					id: "1".into(),
					name: "x".repeat(extensions::MAX_APP_SNAPSHOT_BYTES),
				}),
				channel: None,
			}),
			..Default::default()
		};
		shop.queue(
			&ctx,
			ExtensionRequest::Invoke {
				id: "synthetic.app".into(),
				invocation: Invocation {
					app: Some(Box::new(oversized)),
					..Default::default()
				},
				context: ExtensionContext::capture(&state, false),
			},
		);
		assert!(shop.requests.is_empty());
	}

	#[test]
	fn result_drops_private_snapshot_but_keeps_invocation_call_identity() {
		let mut state = test_support::call_demo_state();
		let context = ExtensionContext::capture(&state, false);
		let invocation = Invocation {
			app: Some(Box::new(extensions::AppSnapshot::default())),
			storage: Some("private local state".into()),
			..Default::default()
		};
		state.voice.active.as_mut().unwrap().request += 1;
		let mut shop = ExtensionUi::default();
		shop.present_output(
			"synthetic.app".into(),
			invocation,
			context.clone(),
			Output::default(),
			&state,
		);
		let result = shop.result.as_ref().unwrap();
		assert!(result.invocation.app.is_none());
		assert!(result.invocation.storage.is_none());
		assert_eq!(result.context.voice_request, context.voice_request);
		assert_ne!(
			result.context.voice_request,
			ExtensionContext::capture(&state, false).voice_request
		);
	}

	fn entry() -> ExtensionEntry {
		ExtensionEntry {
			cover_image: None,
			local_theme: false,
			description: String::new(),
			theme_preview: None,
			preview: None,
			manifest: Manifest {
				api_version: 1,
				id: "synthetic.words".into(),
				name: "Synthetic word tools".into(),
				version: "1.0.0".into(),
				author: "Offline fixture".into(),
				license: "MIT".into(),
				source: "https://example.com/source".into(),
				kind: ExtensionKind::Plugin,
				capabilities: vec![Capability::Composer],
				actions: vec![],
			},
			reviewed: false,
			sha256: "a".repeat(64),
			download_bytes: 4096,
			enabled: false,
			cleanup_pending: false,
			update_available: false,
			update_manifest: None,
		}
	}
	#[test]
	fn catalog_refresh_preserves_only_unchanged_reviewed_consent() {
		let mut shop = ExtensionUi::default();
		let mut available = entry();
		available.reviewed = true;
		let mut updated = available.manifest.clone();
		updated.version = "2.0.0".into();
		available.update_manifest = Some(updated.clone());
		let mut proposed = available.clone();
		proposed.manifest = updated;
		shop.offer_import(proposed.clone());
		shop.consent.as_mut().unwrap().grants = vec![Capability::Composer];
		shop.set_entries(vec![available.clone()]);
		assert_eq!(
			shop.consent.as_ref().unwrap().grants,
			vec![Capability::Composer]
		);
		for change in 0..4 {
			shop.offer_import(proposed.clone());
			let mut changed = available.clone();
			match change {
				0 => changed.sha256 = "b".repeat(64),
				1 => changed
					.update_manifest
					.as_mut()
					.unwrap()
					.capabilities
					.clear(),
				2 => changed.cleanup_pending = true,
				_ => {}
			}
			shop.set_entries(if change == 3 { vec![] } else { vec![changed] });
			assert!(shop.consent.is_none());
		}
		shop.offer_import(entry());
		shop.set_entries(vec![]);
		assert!(
			shop.consent.is_some(),
			"unrelated refresh preserves a local import"
		);
	}

	#[test]
	fn original_theme_customization_and_gallery_preview_return() {
		for action in ["Back to themes", "Customize"] {
			let ctx = egui::Context::default();
			let mut shop = ExtensionUi {
				themes: true,
				..Default::default()
			};
			let package = crate::theme_editor::ThemeEditor::new().package;
			let original_id = package.manifest.id.clone();
			let mut original = entry();
			original.manifest = package.manifest.clone();
			original.theme_preview = package.theme.clone();
			shop.set_entries(vec![original.clone()]);
			frame(&ctx, &mut shop, 900.0, vec![]);
			let labels = frame(&ctx, &mut shop, 900.0, vec![]);
			click(&ctx, &mut shop, 900.0, &labels, "Customize");
			assert!(matches!(
				shop.requests.pop(),
				Some(ExtensionRequest::EditTheme { preview: false, .. })
			));
			shop.open_preview(&ctx, &original);
			assert!(shop.enlarged.is_none());
			assert!(matches!(
				shop.requests.pop(),
				Some(ExtensionRequest::EditTheme { preview: true, .. })
			));
			shop.receive_theme_edit(package, None, None, false, true);
			assert!(shop.begin_gallery_preview(&ctx));
			assert!(!shop.begin_gallery_preview(&ctx));
			assert!(matches!(
				shop.requests.pop(),
				Some(ExtensionRequest::PreviewTheme { theme: Some(_), .. })
			));
			frame(&ctx, &mut shop, 900.0, vec![]);
			let labels = frame(&ctx, &mut shop, 900.0, vec![]);
			click(&ctx, &mut shop, 900.0, &labels, action);
			assert!(!shop.previewing_theme());
			assert!(shop.gallery_preview.is_none());
			assert!(matches!(
				shop.requests.pop(),
				Some(ExtensionRequest::PreviewTheme { theme: None, .. })
			));
			if action == "Customize" {
				let editor = shop.theme_editor.as_ref().unwrap();
				assert_ne!(editor.package.manifest.id, original_id);
				assert!(editor.dirty);
			} else {
				assert!(shop.theme_editor.is_none());
			}
		}
	}
	#[test]
	fn local_theme_opens_for_edit_and_cover_replaces_palette_preview() {
		let ctx = egui::Context::default();
		let mut shop = ExtensionUi {
			themes: true,
			..Default::default()
		};
		let mut local = entry();
		local.manifest.kind = ExtensionKind::Theme;
		local.manifest.id = "local-cover".into();
		local.manifest.name = "Local cover".into();
		local.enabled = true;
		local.local_theme = true;
		local.theme_preview = Some(extensions::Theme::default());
		local.cover_image = Some(Arc::new(egui::ColorImage::filled(
			[16, 9],
			egui::Color32::GREEN,
		)));
		shop.set_entries(vec![local.clone()]);
		frame(&ctx, &mut shop, 900.0, vec![]);
		let labels = frame(&ctx, &mut shop, 900.0, vec![]);
		assert!(labels.iter().any(|(text, _)| text == "Edit theme"));
		assert!(
			shop.previews
				.get("local-cover")
				.is_some_and(|preview| preview.texture.is_some()),
			"custom cover should create a card texture"
		);
		let mut replaced = local.clone();
		replaced.cover_image = Some(Arc::new(egui::ColorImage::filled(
			[16, 9],
			egui::Color32::BLUE,
		)));
		shop.set_entries(vec![replaced]);
		assert!(!shop.previews.contains_key("local-cover"));
		click(&ctx, &mut shop, 900.0, &labels, "Edit theme");
		assert!(shop.requests.iter().any(
			|request| matches!(request, ExtensionRequest::EditTheme { id, .. } if id == "local-cover")
		));

		let mut package = crate::theme_editor::ThemeEditor::new().package;
		package.manifest.id = "local-cover".into();
		shop.receive_theme_edit(
			package.clone(),
			None,
			local.cover_image.clone(),
			true,
			false,
		);
		assert_eq!(
			shop.theme_editor.as_ref().unwrap().package.manifest.id,
			"local-cover"
		);
		assert!(!shop.theme_editor.as_ref().unwrap().dirty);
		shop.theme_editor = None;
		shop.receive_theme_edit(package, None, local.cover_image.clone(), false, false);
		assert_ne!(
			shop.theme_editor.as_ref().unwrap().package.manifest.id,
			"local-cover"
		);
	}
	#[test]
	fn theme_card_keeps_use_and_edit_side_by_side() {
		for width in [320.0, 900.0] {
			let ctx = egui::Context::default();
			let mut shop = ExtensionUi {
				themes: true,
				..Default::default()
			};
			let mut theme = entry();
			theme.manifest.kind = ExtensionKind::Theme;
			theme.manifest.id = "local-card".into();
			theme.enabled = true;
			theme.local_theme = true;
			theme.update_available = true;
			theme.theme_preview = Some(extensions::Theme::default());
			shop.set_entries(vec![theme]);
			frame(&ctx, &mut shop, width, vec![]);
			let labels = frame(&ctx, &mut shop, width, vec![]);
			let edit = labels
				.iter()
				.find(|(text, _)| text == "Edit theme")
				.unwrap()
				.1;
			let use_theme = labels
				.iter()
				.find(|(text, _)| text == "Use theme")
				.unwrap()
				.1;
			assert!(
				use_theme.right() < edit.left()
					&& (use_theme.center().y - edit.center().y).abs() < 4.0,
				"apply and edit share the footer row"
			);
			let remove = labels.iter().find(|(text, _)| text == "Remove").unwrap().1;
			assert!(
				remove.bottom() < use_theme.top() && remove.right() <= width,
				"remove stays a quiet link above the footer"
			);
			shop.busy = true;
			click(&ctx, &mut shop, width, &labels, "Use theme");
			assert!(shop.requests.is_empty());
			shop.busy = false;
			let labels = frame(&ctx, &mut shop, width, vec![]);
			click(&ctx, &mut shop, width, &labels, "Use theme");
			assert!(
				matches!(shop.requests.pop(), Some(ExtensionRequest::SelectTheme { id: Some(id) }) if id == "local-card")
			);
			// The desktop confirms selection only after the existing persistence job succeeds.
			shop.active_theme = Some("local-card".into());
			let labels = frame(&ctx, &mut shop, width, vec![]);
			assert!(labels.iter().any(|(text, _)| text == "Active"));
			assert!(!labels.iter().any(|(text, _)| text == "Use theme"));
			assert!(
				!labels
					.iter()
					.any(|(text, _)| text == "Disable & delete data"
						|| text == "THEME" || text
						== "Give your conversations a different look.")
			);
			click(&ctx, &mut shop, width, &labels, "More");
			let menu = frame(&ctx, &mut shop, width, vec![]);
			assert!(menu.iter().any(|(text, _)| text == "Import theme…"));
			assert!(
				!menu
					.iter()
					.any(|(text, _)| text == "Use built-in appearance"),
				"built-in presets live in Appearance, not the gallery menu"
			);
			click(&ctx, &mut shop, width, &menu, "Refresh catalog");
			assert!(matches!(
				shop.requests.as_slice(),
				[ExtensionRequest::RefreshCatalog]
			));
			shop.requests.clear();
			let labels = frame(&ctx, &mut shop, width, vec![]);
			click(&ctx, &mut shop, width, &labels, "Remove");
			assert!(shop.requests.iter().any(
				|request| matches!(request, ExtensionRequest::Disable { id } if id == "local-card")
			));
		}
	}
	fn frame(
		ctx: &egui::Context,
		extensions: &mut ExtensionUi,
		width: f32,
		events: Vec<egui::Event>,
	) -> Vec<(String, egui::Rect)> {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => {
					labels.push((text.galley.job.text.clone(), text.visual_bounding_rect()))
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(width, 1000.0),
				)),
				events,
				focused: true,
				..Default::default()
			},
			|ui| {
				if extensions.previewing_theme() {
					extensions.theme_preview_bar(ui, 0.0);
				} else {
					extensions.settings(ui, &State::default());
				}
			},
		);
		let mut labels = vec![];
		for shape in &output.shapes {
			collect(&shape.shape, &mut labels);
		}
		output.drop_without_applying_deltas();
		labels
	}
	fn click(
		ctx: &egui::Context,
		extensions: &mut ExtensionUi,
		width: f32,
		labels: &[(String, egui::Rect)],
		label: &str,
	) {
		let pos = labels
			.iter()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("Missing {label}"))
			.1
			.center();
		for pressed in [true, false] {
			frame(
				ctx,
				extensions,
				width,
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
	}
	#[test]
	fn consent_requires_explicit_grant_and_cleanup_stays_available() {
		for width in [320.0, 900.0] {
			for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				let mut extensions = ExtensionUi::default();
				extensions.set_entries(vec![entry()]);
				extensions.offer_import(entry());
				frame(&ctx, &mut extensions, width, vec![]);
				let labels = frame(&ctx, &mut extensions, width, vec![]);
				extensions.requests.clear();
				click(&ctx, &mut extensions, width, &labels, "Enable");
				assert!(
					extensions.requests.is_empty(),
					"Permission must be granted before enable"
				);
				click(
					&ctx,
					&mut extensions,
					width,
					&labels,
					capability_label(Capability::Composer),
				);
				let labels = frame(&ctx, &mut extensions, width, vec![]);
				click(&ctx, &mut extensions, width, &labels, "Enable");
				assert!(
					matches!(extensions.requests.as_slice(), [ExtensionRequest::Enable { grants, sha256, reviewed, .. }] if grants == &[Capability::Composer] && sha256 == &"a".repeat(64) && !reviewed)
				);

				extensions.requests.clear();
				extensions.offer_import(entry());
				let labels = frame(&ctx, &mut extensions, width, vec![]);
				click(&ctx, &mut extensions, width, &labels, "Cancel");
				let mut reviewed_entry = entry();
				reviewed_entry.reviewed = true;
				reviewed_entry.sha256 = "b".repeat(64);
				extensions.set_entries(vec![reviewed_entry]);
				let labels = frame(&ctx, &mut extensions, width, vec![]);
				click(&ctx, &mut extensions, width, &labels, "Review & enable");
				let labels = frame(&ctx, &mut extensions, width, vec![]);
				click(
					&ctx,
					&mut extensions,
					width,
					&labels,
					capability_label(Capability::Composer),
				);
				let labels = frame(&ctx, &mut extensions, width, vec![]);
				click(&ctx, &mut extensions, width, &labels, "Enable");
				assert!(
					matches!(extensions.requests.as_slice(), [ExtensionRequest::Enable { sha256, reviewed: true, .. }] if sha256 == &"b".repeat(64))
				);
				extensions.requests.clear();
				let mut pending = entry();
				pending.cleanup_pending = true;
				extensions.set_entries(vec![pending]);
				extensions.busy = true;
				frame(&ctx, &mut extensions, width, vec![]);
				let labels = frame(&ctx, &mut extensions, width, vec![]);
				click(&ctx, &mut extensions, width, &labels, "Retry cleanup");
				assert!(matches!(
					extensions.requests.as_slice(),
					[ExtensionRequest::Disable { .. }]
				));
			}
		}
	}

	#[test]
	fn status_banner_wraps_and_clears_on_refresh() {
		for width in [320.0, 900.0] {
			let ctx = egui::Context::default();
			let mut extensions = ExtensionUi::default();
			extensions.set_entries(vec![entry()]);
			extensions.status = "Disabled. Downloaded code and extension data were removed.".into();
			let labels = frame(&ctx, &mut extensions, width, vec![]);
			assert!(
				labels
					.iter()
					.any(|(text, rect)| text.starts_with("Disabled.") && rect.right() <= width),
				"the status banner must stay inside the page at {width}"
			);
			click(&ctx, &mut extensions, width, &labels, "Refresh");
			assert!(
				extensions.status.is_empty()
					&& matches!(
						extensions.requests.as_slice(),
						[ExtensionRequest::RefreshCatalog]
					),
				"refreshing clears the previous status instead of reporting itself"
			);
		}
	}

	#[test]
	fn image_sharing_plugin_has_local_card_and_enlarged_preview() {
		for width in [320.0, 900.0] {
			let ctx = egui::Context::default();
			let mut shop = ExtensionUi::default();
			let mut plugin = entry();
			plugin.manifest.capabilities = vec![Capability::ImageSharing];
			shop.set_entries(vec![plugin.clone()]);
			for _ in 0..2 {
				frame(&ctx, &mut shop, width, vec![]);
			}
			let labels = frame(&ctx, &mut shop, width, vec![]);
			assert!(labels.iter().any(|(text, _)| text == "Image attachment"));
			assert!(!labels.iter().any(|(text, _)| text == "Preview unavailable"));
			shop.open_preview(&ctx, &plugin);
			frame(&ctx, &mut shop, width, vec![]);
			assert_eq!(shop.enlarged.as_deref(), Some(plugin.manifest.id.as_str()));
			assert!(shop.previews.is_empty());
			assert!(
				!shop
					.requests
					.iter()
					.any(|r| matches!(r, ExtensionRequest::Preview { .. }))
			);
		}
	}

	#[test]
	fn shop_previews_load_visible_cards_once_and_evict_with_metadata() {
		for width in [320.0, 900.0] {
			let ctx = egui::Context::default();
			let mut shop = ExtensionUi::default();
			let entries: Vec<_> = (0..30)
				.map(|i| {
					let mut entry = entry();
					entry.manifest.id = format!("plugin-{i}");
					entry.preview = Some(extensions::Preview {
						url: "https://example.com/preview.png".into(),
						sha256: "a".repeat(64),
						download_bytes: 123,
					});
					entry
				})
				.collect();
			shop.set_entries(entries.clone());
			frame(&ctx, &mut shop, width, vec![]);
			shop.requests.clear();
			frame(&ctx, &mut shop, width, vec![]);
			assert!(
				shop.previews.len() < entries.len(),
				"offscreen cards must not fetch images"
			);
			shop.requests.clear();
			frame(&ctx, &mut shop, width, vec![]);
			assert!(
				shop.requests.is_empty(),
				"pending previews must be deduplicated"
			);
			for entry in &entries {
				shop.preview_clock += 2;
				shop.preview_fixture_image(
					entry.manifest.id.clone(),
					egui::ColorImage::filled([640, 360], egui::Color32::BLUE),
				);
			}
			assert_eq!(shop.previews.len(), MAX_PREVIEWS);
			assert_eq!(
				shop.previews
					.values()
					.filter_map(|preview| preview.image.as_ref())
					.map(|image| image.pixels.len() * 4)
					.sum::<usize>(),
				22_118_400
			);
			shop.receive_preview(
				"plugin-19".into(),
				Some(egui::ColorImage::filled([641, 1], egui::Color32::BLUE)),
			);
			assert!(shop.previews["plugin-19"].image.is_none());
			let mut changed = entries;
			for entry in &mut changed {
				entry.preview.as_mut().unwrap().sha256 = "b".repeat(64);
			}
			shop.set_entries(changed);
			assert!(shop.previews.is_empty());
			shop.query = "missing creator".into();
			let labels = frame(&ctx, &mut shop, width, vec![]);
			assert!(labels.iter().any(|(text, _)| text == "No matches"));
			assert!(shop.previews.is_empty());
		}
	}

	#[test]
	fn tall_shop_keeps_visible_previews_without_repeated_downloads() {
		let ctx = egui::Context::default();
		let mut shop = ExtensionUi::default();
		shop.set_entries(
			(0..20)
				.map(|i| {
					let mut entry = entry();
					entry.manifest.id = format!("plugin-{i}");
					entry.preview = Some(extensions::Preview {
						url: "https://example.com/image.png".into(),
						sha256: "a".repeat(64),
						download_bytes: 123,
					});
					entry
				})
				.collect(),
		);
		for _ in 0..5 {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 8000.0),
					)),
					..Default::default()
				},
				|ui| shop.settings(ui, &State::default()),
			)
			.drop_without_applying_deltas();
			for request in std::mem::take(&mut shop.requests) {
				if let ExtensionRequest::Preview { id } = request {
					shop.receive_preview(
						id,
						Some(egui::ColorImage::filled([1, 1], egui::Color32::BLUE)),
					);
				}
			}
		}
		let retained: Vec<_> = shop.previews.keys().cloned().collect();
		assert_eq!(retained.len(), 20, "every visible card keeps its thumbnail");
		ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 8000.0),
				)),
				..Default::default()
			},
			|ui| shop.settings(ui, &State::default()),
		)
		.drop_without_applying_deltas();
		assert!(shop.requests.is_empty());
		shop.receive_preview(
			"plugin-19".into(),
			Some(egui::ColorImage::filled([1, 1], egui::Color32::BLUE)),
		);
		assert_eq!(
			shop.previews.keys().cloned().collect::<Vec<_>>(),
			retained,
			"late offscreen images cannot evict visible previews"
		);
	}

	#[test]
	fn queued_actions_and_closed_results_are_bounded() {
		let mut extensions = ExtensionUi::default();
		for _ in 0..10 {
			extensions.queue(&egui::Context::default(), ExtensionRequest::RefreshCatalog);
		}
		assert_eq!(extensions.requests.len(), 4);
		assert!(!extensions.status.is_empty());
		extensions.reset_runtime();
		assert!(extensions.requests.is_empty());
		extensions.queue(
			&egui::Context::default(),
			ExtensionRequest::Invoke {
				id: "test".into(),
				invocation: Invocation {
					composer: Some("x".repeat(4 * extensions::MAX_IO_BYTES)),
					..Default::default()
				},
				context: ExtensionContext::capture(&State::default(), false),
			},
		);
		assert!(extensions.requests.is_empty());
	}
	#[test]
	fn proposal_requires_unchanged_account_conversation_and_draft() {
		let mut state = State {
			selected: Some(Id(1)),
			..Default::default()
		};
		state.drafts.insert(Id(1), "original".into());
		let context = ExtensionContext::capture(&state, true);
		state.generation += 1;
		assert!(!apply_proposal(&mut state, &context, "changed"));
		state.generation -= 1;
		state.selected = Some(Id(2));
		assert!(!apply_proposal(&mut state, &context, "changed"));
		state.selected = Some(Id(1));
		state.drafts.insert(Id(1), "new draft".into());
		assert!(!apply_proposal(&mut state, &context, "changed"));
		state.drafts.insert(Id(1), "original".into());
		assert!(!apply_proposal(
			&mut state,
			&context,
			&"x".repeat(MAX_CONTENT + 1)
		));
		assert!(apply_proposal(&mut state, &context, "changed"));
		assert_eq!(state.drafts[&Id(1)], "changed");
	}
}
