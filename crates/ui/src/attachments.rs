//! Inline service attachments; decoded pixels reuse the existing bounded media cache.
#[cfg(test)]
mod context_tests;
use crate::{
	avatars::Avatars,
	design,
	icons::{self, Icon},
	markdown::external_url,
};
use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind};
use model::{Attachment, Id, Message};

/// Rough content family of a file, chosen from its name and reported type only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
	Image,
	Pdf,
	Archive,
	Text,
	Code,
	Audio,
	Video,
	Other,
}
pub fn file_kind(filename: &str, content_type: Option<&str>) -> FileKind {
	let extension = filename
		.rsplit_once('.')
		.map(|(_, extension)| extension.to_ascii_lowercase())
		.unwrap_or_default();
	let mime = content_type
		.map(|kind| {
			kind.split(';')
				.next()
				.unwrap_or(kind)
				.trim()
				.to_ascii_lowercase()
		})
		.unwrap_or_default();
	match extension.as_str() {
		"png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "tiff" | "heic" | "svg" => {
			return FileKind::Image;
		}
		"pdf" => return FileKind::Pdf,
		"zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" => {
			return FileKind::Archive;
		}
		"txt" | "md" | "rtf" | "log" | "csv" | "doc" | "docx" | "odt" => return FileKind::Text,
		"rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "json" | "toml" | "yaml" | "yml" | "html"
		| "css" | "c" | "h" | "cpp" | "hpp" | "java" | "kt" | "swift" | "go" | "rb" | "sh"
		| "xml" | "sql" => return FileKind::Code,
		"mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "opus" => return FileKind::Audio,
		"mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" => return FileKind::Video,
		_ => {}
	}
	if mime.starts_with("image/") {
		FileKind::Image
	} else if mime == "application/pdf" {
		FileKind::Pdf
	} else if mime.starts_with("audio/") {
		FileKind::Audio
	} else if mime.starts_with("video/") {
		FileKind::Video
	} else if mime.starts_with("text/") {
		FileKind::Text
	} else if mime.contains("zip") || mime.contains("compressed") || mime.contains("tar") {
		FileKind::Archive
	} else {
		FileKind::Other
	}
}
impl FileKind {
	pub fn icon(self) -> Icon {
		match self {
			FileKind::Image => Icon::FileImage,
			FileKind::Pdf => Icon::FilePdf,
			FileKind::Archive => Icon::FileZip,
			FileKind::Text => Icon::FileText,
			FileKind::Code => Icon::FileCode,
			FileKind::Audio => Icon::FileAudio,
			FileKind::Video => Icon::FileVideo,
			FileKind::Other => Icon::File,
		}
	}
	/// Discord-style glyph tint: red documents, amber archives, blurple media, muted text.
	pub fn tint(self, colors: &design::Palette) -> Color32 {
		match self {
			FileKind::Pdf => colors.danger,
			FileKind::Archive => colors.warning,
			FileKind::Image | FileKind::Audio | FileKind::Video => colors.accent,
			FileKind::Code => colors.link,
			FileKind::Text | FileKind::Other => colors.muted,
		}
	}
}
/// Human file size in the units Discord shows next to attachments.
pub fn format_size(bytes: u64) -> String {
	const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
	if bytes < 1024 {
		return format!("{bytes} bytes");
	}
	let mut value = bytes as f64 / 1024.0;
	let mut unit = 0;
	while value >= 1024.0 && unit + 1 < UNITS.len() {
		value /= 1024.0;
		unit += 1;
	}
	if value >= 100.0 {
		format!("{value:.0} {}", UNITS[unit])
	} else {
		format!("{value:.2} {}", UNITS[unit])
	}
}

/// Card for a file that will be uploaded with the next message. Returns `true` when the user
/// asks to remove it.
pub fn pending_card(
	ui: &mut egui::Ui,
	filename: &str,
	bytes: u64,
	preview: Option<&egui::TextureHandle>,
	removable: bool,
) -> bool {
	const WIDTH: f32 = 176.0;
	const HEIGHT: f32 = 168.0;
	// Discord floats the action pill over the card's top edge; reserve that overhang.
	const OVERHANG: f32 = 10.0;
	let colors = design::palette(ui);
	let kind = file_kind(filename, None);
	let (allocated, _) =
		ui.allocate_exact_size(egui::vec2(WIDTH + 12.0, HEIGHT + OVERHANG), Sense::hover());
	let card = Rect::from_min_size(
		allocated.left_bottom() - egui::vec2(0.0, HEIGHT),
		egui::vec2(WIDTH, HEIGHT),
	);
	ui.painter().rect(
		card,
		8,
		colors.sidebar,
		Stroke::new(1.0, colors.border),
		StrokeKind::Inside,
	);
	let preview_rect =
		Rect::from_min_size(card.min + egui::vec2(8.0, 8.0), egui::vec2(160.0, 108.0));
	ui.painter().rect_filled(preview_rect, 4, colors.base);
	match preview {
		Some(texture) => {
			let size = texture.size_vec2();
			let scale = (preview_rect.width() / size.x)
				.min(preview_rect.height() / size.y)
				.clamp(f32::EPSILON, 1.0);
			let fitted = Rect::from_center_size(preview_rect.center(), size * scale);
			ui.put(
				fitted,
				egui::Image::from_texture(texture)
					.fit_to_exact_size(fitted.size())
					.corner_radius(4),
			);
		}
		None => {
			icons::paint(
				ui.painter(),
				kind.icon(),
				Rect::from_center_size(preview_rect.center(), egui::Vec2::splat(56.0)),
				kind.tint(&colors),
			);
		}
	}
	let name = Rect::from_min_size(
		preview_rect.left_bottom() + egui::vec2(0.0, 6.0),
		egui::vec2(preview_rect.width(), 18.0),
	);
	ui.put(
		name,
		egui::Label::new(design::semibold(ui, filename, 13.0).color(colors.text_strong))
			.truncate()
			.selectable(false),
	)
	.on_hover_text(filename);
	let size = Rect::from_min_size(name.left_bottom(), egui::vec2(name.width(), 16.0));
	ui.put(
		size,
		egui::Label::new(
			RichText::new(format_size(bytes))
				.size(12.0)
				.color(colors.muted),
		)
		.truncate()
		.selectable(false),
	);
	let pill = Rect::from_min_size(
		egui::pos2(card.right() - 28.0, card.top() - OVERHANG),
		egui::vec2(36.0, 36.0),
	);
	ui.painter().rect(
		pill,
		6,
		colors.raised,
		Stroke::new(1.0, colors.border),
		StrokeKind::Inside,
	);
	ui.scope_builder(
		egui::UiBuilder::new().max_rect(pill.shrink(2.0)).layout(
			egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
		),
		|ui| {
			ui.add_enabled_ui(removable, |ui| {
				icons::button(ui, Icon::Trash, 32.0, "Remove attachment").clicked()
			})
			.inner
		},
	)
	.inner
}

/// Discord-style row for a received file without an inline preview.
fn file_card(
	ui: &mut egui::Ui,
	attachment: &Attachment,
	download: &mut DownloadUi,
	opening: &mut Option<String>,
	demo: bool,
	surface: &mut crate::select::Surface,
) {
	let colors = design::palette(ui);
	let kind = file_kind(&attachment.filename, attachment.content_type.as_deref());
	egui::Frame::new()
		.fill(colors.raised)
		.stroke(Stroke::new(1.0, colors.border))
		.corner_radius(8)
		.inner_margin(egui::Margin::symmetric(12, 10))
		.show(ui, |ui| {
			ui.set_width(ui.available_width().min(432.0));
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 10.0;
				icons::inline(ui, kind.icon(), 32.0, kind.tint(&colors));
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					let download = download_button(ui, attachment, download, demo);
					let open = open_original(ui, attachment, opening);
					ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
						ui.vertical(|ui| {
							ui.spacing_mut().item_spacing.y = 0.0;
							let (pos, galley, response) = egui::Label::new(
								design::medium(ui, &attachment.filename, 14.0).color(colors.link),
							)
							.truncate()
							.selectable(false)
							.layout_in_ui(ui);
							response.clone().on_hover_text(&attachment.filename);
							surface.run(ui, &response, pos, galley, Vec::new());
							ui.add(
								egui::Label::new(
									RichText::new(format_size(attachment.size))
										.size(12.0)
										.color(colors.muted),
								)
								.selectable(false),
							);
						});
					});
					if let Some(open) = &open {
						surface.keep(open);
					}
					surface.keep(&download);
				});
			});
		});
}

#[allow(clippy::too_many_arguments)]
pub fn show(
	ui: &mut egui::Ui,
	message: &Message,
	images: &mut Avatars,
	viewing: &mut Option<(Id, Id)>,
	opening: &mut Option<String>,
	download: &mut DownloadUi,
	audio: &mut crate::audio::AudioUi,
	video: &mut crate::video::VideoUi,
	demo: bool,
	surface: &mut crate::select::Surface,
) {
	show_subset(
		ui,
		message,
		&message.attachments,
		images,
		viewing,
		opening,
		download,
		audio,
		video,
		demo,
		surface,
	);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn show_subset(
	ui: &mut egui::Ui,
	message: &Message,
	attachments: &[Attachment],
	images: &mut Avatars,
	viewing: &mut Option<(Id, Id)>,
	opening: &mut Option<String>,
	download: &mut DownloadUi,
	audio: &mut crate::audio::AudioUi,
	video: &mut crate::video::VideoUi,
	demo: bool,
	surface: &mut crate::select::Surface,
) {
	for group in attachments.chunk_by(|a, b| a.is_image() == b.is_image()) {
		if group[0].is_image() {
			let (columns, size) = image_layout(group.len(), ui.available_width());
			ui.scope(|ui| {
				ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
				for row in group.chunks(columns) {
					ui.horizontal_top(|ui| {
						for attachment in row {
							ui.push_id(("attachment", attachment.id), |ui| {
								let image = images.show_embed(
									ui,
									&attachment.media,
									artwork_size(attachment, size),
									demo,
								);
								let response =
									ui.interact(image.rect, image.id.with("media"), Sense::click());
								response.widget_info(|| {
									egui::WidgetInfo::labeled(
										egui::WidgetType::Button,
										ui.is_enabled(),
										format!("View image {}", attachment.filename),
									)
								});
								media_context_menu(&response, attachment, download, opening, demo);
								surface.keep(&response);
								if response
									.on_hover_text(
										attachment
											.description
											.as_deref()
											.unwrap_or("Enlarge image"),
									)
									.clicked()
								{
									*viewing = Some((message.id, attachment.id));
								}
							});
						}
					});
				}
			});
		} else {
			for attachment in group {
				ui.push_id(("attachment", attachment.id), |ui| {
					if attachment.is_video() {
						let response = video.show(ui, message, attachment, download, opening, demo);
						surface.keep(&response);
						media_context_menu(&response, attachment, download, opening, demo);
					} else if attachment.is_audio() {
						let response = audio.show(ui, message, attachment);
						surface.keep(&response);
						if attachment.is_voice_message() {
							response.context_menu(|ui| {
								download_button(ui, attachment, download, demo);
								open_original(ui, attachment, opening);
							});
						} else {
							ui.horizontal_wrapped(|ui| {
								let download = download_button(ui, attachment, download, demo);
								let open = open_original(ui, attachment, opening);
								if let Some(open) = &open {
									surface.keep(open);
								}
								surface.keep(&download);
							});
						}
					} else {
						file_card(ui, attachment, download, opening, demo, surface);
					}
					ui.add_space(6.0);
				});
			}
		}
	}
}
// Shared with height estimation so compact artwork does not leave a gallery-sized gap.
pub(crate) fn artwork_edge(filename: &str) -> Option<f32> {
	let (stem, extension) = filename.rsplit_once('.')?;
	if !matches!(extension, "png" | "gif") {
		return None;
	}
	let (id, edge) = if let Some(id) = stem.strip_prefix("emoji-") {
		(id, 160.0_f32)
	} else {
		(stem.strip_prefix("sticker-")?, 160.0_f32)
	};
	(!id.is_empty()
		&& id.bytes().all(|b| b.is_ascii_digit())
		&& id.parse::<Id>().is_ok_and(|id| id.0 != 0))
	.then_some(edge)
}

fn artwork_size(attachment: &Attachment, gallery: egui::Vec2) -> egui::Vec2 {
	let Some(edge) = artwork_edge(&attachment.filename) else {
		return gallery;
	};
	egui::Vec2::splat(edge.min(gallery.x).min(gallery.y))
}

pub(crate) fn image_layout(count: usize, width: f32) -> (usize, egui::Vec2) {
	let columns = if count > 1 && width >= 280.0 { 2 } else { 1 };
	let width = ((width.min(420.0) - (columns - 1) as f32 * 6.0) / columns as f32).max(1.0);
	(
		columns,
		egui::vec2(width, if count > 1 { 180.0 } else { 280.0 }),
	)
}

fn open_original(
	ui: &mut egui::Ui,
	attachment: &Attachment,
	opening: &mut Option<String>,
) -> Option<egui::Response> {
	let target = attachment.media.url.as_deref().and_then(external_url)?;
	let response = ui.small_button("Open original…");
	if response.clicked() {
		*opening = Some(target);
	}
	Some(response)
}
#[derive(Default)]
pub struct DownloadUi {
	pub request: Option<Attachment>,
	pub copy_request: Option<Attachment>,
	pub embed_request: Option<(model::EmbedMedia, bool)>,
	pub cancel_requested: bool,
	pub dismiss_requested: bool,
	pub active: bool,
	pub status: String,
}
pub(super) fn media_context_menu(
	response: &egui::Response,
	attachment: &Attachment,
	download: &mut DownloadUi,
	opening: &mut Option<String>,
	demo: bool,
) {
	if let Some(copy) = media_menu(
		response,
		attachment.is_video(),
		attachment.media.url.as_deref(),
		download,
		opening,
		demo,
	) {
		if copy {
			download.copy_request = Some(attachment.clone());
		} else {
			download.request = Some(attachment.clone());
		}
	}
}
pub(crate) fn embed_context_menu(
	response: &egui::Response,
	media: &model::EmbedMedia,
	download: &mut DownloadUi,
	demo: bool,
) {
	if let Some(copy) = media_menu(
		response,
		false,
		media.url.as_deref().or(media.proxy_url.as_deref()),
		download,
		&mut None,
		demo,
	) {
		download.embed_request = Some((media.clone(), copy));
	}
}
fn media_menu(
	response: &egui::Response,
	video: bool,
	url: Option<&str>,
	download: &DownloadUi,
	opening: &mut Option<String>,
	demo: bool,
) -> Option<bool> {
	if !video && response.has_focus() {
		response.ctx.layer_painter(response.layer_id).rect_stroke(
			response.rect,
			5,
			Stroke::new(2.0, design::palette_for(&response.ctx).accent),
			StrokeKind::Inside,
		);
	}
	let mut popup = crate::user_menu::popup(response, response.id.with("media-menu"));
	// Drag-only video sliders do not set secondary_clicked, but still belong to the media.
	if response.contains_pointer() && response.ctx.input(|i| i.pointer.secondary_clicked()) {
		popup = popup
			.open_memory(Some(egui::SetOpenCommand::Bool(true)))
			.at_pointer_fixed();
	}
	let mut action = None;
	popup.show(|ui| {
		let idle = !demo && !download.busy();
		let kind = if video { "video" } else { "image" };
		for (copy, label) in [
			(true, format!("Copy {kind}")),
			(false, format!("Save {kind} as…")),
		] {
			if ui
				.add_enabled(idle, egui::Button::new(label))
				.on_disabled_hover_text(if demo {
					"Unavailable for synthetic attachments"
				} else {
					"A media transfer is already active"
				})
				.clicked()
			{
				action = Some(copy);
				ui.close();
			}
		}
		if let Some(url) = url.and_then(external_url) {
			if video && ui.button("Open original…").clicked() {
				*opening = Some(url.clone());
				ui.close();
			}
			if ui.button("Copy link").clicked() {
				ui.ctx().copy_text(url);
				ui.close();
			}
		}
	});
	action
}
impl DownloadUi {
	pub(crate) fn busy(&self) -> bool {
		self.active
			|| self.request.is_some()
			|| self.copy_request.is_some()
			|| self.embed_request.is_some()
	}
	pub fn show_status(&mut self, ui: &mut egui::Ui) {
		if !self.status.is_empty() {
			ui.horizontal_wrapped(|ui| {
				ui.small(&self.status);
				if self.active && ui.small_button("Cancel download").clicked() {
					self.cancel_requested = true;
				}
				if !self.active && ui.small_button("Dismiss").clicked() {
					self.dismiss_requested = true;
				}
			});
		}
	}
}
fn download_button(
	ui: &mut egui::Ui,
	attachment: &Attachment,
	download: &mut DownloadUi,
	demo: bool,
) -> egui::Response {
	let response = ui
		.add_enabled(!demo && !download.busy(), egui::Button::new("Download"))
		.on_hover_text("Choose where to save this file · up to 100 MiB")
		.on_disabled_hover_text(if demo {
			"Downloads are disabled for synthetic attachments"
		} else {
			"A download is already active"
		});
	if response.clicked() {
		download.request = Some(attachment.clone());
	}
	response
}
/// Resolve against the current message every frame: deleted or hidden media cannot linger.
fn gallery_step(attachments: &[Attachment], current: Id, previous: bool) -> Option<Id> {
	let images: Vec<_> = attachments.iter().filter(|a| a.is_image()).collect();
	let index = images.iter().position(|a| a.id == current)?;
	let index = if previous {
		(index + images.len() - 1) % images.len()
	} else {
		(index + 1) % images.len()
	};
	Some(images[index].id)
}

/// Translucent round control floating over the viewer backdrop; always light-on-dark.
pub(crate) fn glass_button(
	ui: &mut egui::Ui,
	icon: Icon,
	diameter: f32,
	label: &str,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(diameter), Sense::click());
	let enabled = ui.is_enabled();
	let hot = enabled && (response.hovered() || response.has_focus());
	ui.painter().circle_filled(
		rect.center(),
		diameter / 2.0,
		if hot {
			Color32::from_white_alpha(40)
		} else {
			Color32::from_black_alpha(150)
		},
	);
	icons::paint(
		ui.painter(),
		icon,
		Rect::from_center_size(rect.center(), egui::Vec2::splat(diameter * 0.5)),
		if !enabled {
			Color32::from_gray(120)
		} else if hot {
			Color32::WHITE
		} else {
			Color32::from_gray(220)
		},
	);
	response.on_hover_text(label)
}

/// Full-window media viewer. Returns the attachment to keep showing, or `None` once closed by
/// the close control, Escape, or a click anywhere outside the image and its controls.
pub fn viewer(
	ui: &mut egui::Ui,
	attachments: &[Attachment],
	current: Id,
	images: &mut Avatars,
	download: &mut DownloadUi,
	opening: &mut Option<String>,
	demo: bool,
) -> Option<Id> {
	let mut current = current;
	if ui
		.ctx()
		.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft))
	{
		current = gallery_step(attachments, current, true)?;
	}
	if ui
		.ctx()
		.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight))
	{
		current = gallery_step(attachments, current, false)?;
	}
	let gallery: Vec<&Attachment> = attachments.iter().filter(|a| a.is_image()).collect();
	let index = gallery.iter().position(|a| a.id == current)?;
	let attachment = gallery[index];
	let count = gallery.len();
	let zoom_id = egui::Id::unique("attachment-viewer-zoom");
	let (previous, mut zoom, mut pan) = ui.ctx().data_mut(|data| {
		data.get_temp::<(Id, f32, egui::Vec2)>(zoom_id)
			.unwrap_or((current, 1.0, egui::Vec2::ZERO))
	});
	if previous != current {
		zoom = 1.0;
		pan = egui::Vec2::ZERO;
	}
	let size = ui.ctx().content_rect().size().max(egui::vec2(1.0, 1.0));
	let mut close = false;
	// Modal input capture prevents clicks and keys reaching the conversation. No dialog frame.
	let overlay = egui::Modal::new(egui::Id::unique("attachment-viewer"))
		.backdrop_color(Color32::from_black_alpha(236))
		.frame(egui::Frame::NONE)
		.show(ui.ctx(), |ui| {
			ui.set_min_size(size);
			ui.set_max_size(size);
			*ui.visuals_mut() = egui::Visuals::dark();
			ui.visuals_mut().override_text_color = Some(Color32::WHITE);
			// Everything not covered by a later control is "away": clicking it closes the viewer.
			let (full, backdrop) = ui.allocate_exact_size(size, Sense::click());
			const TOP: f32 = 56.0;
			let bottom = if count > 1 { 116.0 } else { 60.0 };
			let side = if count > 1 { 84.0 } else { 24.0 };
			let stage = Rect::from_min_max(
				full.min + egui::vec2(side, TOP),
				full.max - egui::vec2(side, bottom),
			)
			.intersect(full);
			let stage = if stage.is_positive() {
				stage
			} else {
				Rect::from_center_size(full.center(), egui::vec2(1.0, 1.0))
			};
			// Image, centered on the stage, sized from bounded metadata.
			let original = if attachment.media.width > 0 && attachment.media.height > 0 {
				egui::vec2(
					attachment.media.width.min(16384) as f32,
					attachment.media.height.min(16384) as f32,
				)
			} else {
				egui::vec2(320.0, 180.0)
			};
			let scale = (stage.width() / original.x).min(stage.height() / original.y);
			let fitted = (original * scale).max(egui::vec2(1.0, 1.0));
			let image_rect = Rect::from_center_size(stage.center(), fitted);
			if let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
				&& stage.contains(pointer)
			{
				let factor = ui.input_mut(|i| {
					let factor = (i.smooth_scroll_delta.y * 0.005).exp() * i.zoom_delta();
					i.smooth_scroll_delta = egui::Vec2::ZERO;
					factor
				});
				let next = (zoom * factor).clamp(1.0, (4096.0 / fitted.max_elem()).clamp(1.0, 8.0));
				pan = (pointer - stage.center()) - (pointer - stage.center() - pan) * (next / zoom);
				zoom = next;
			}
			let limit = ((fitted * zoom - stage.size()) * 0.5).max(egui::Vec2::ZERO);
			pan = pan.clamp(-limit, limit);
			let zoomed = Rect::from_center_size(stage.center() + pan, fitted * zoom);
			{
				// Zoomed geometry must not enlarge and recenter the modal or its controls.
				let mut image_ui = ui.new_child(egui::UiBuilder::new().max_rect(zoomed).layout(
					egui::Layout::centered_and_justified(egui::Direction::TopDown),
				));
				let ui = &mut image_ui;
				ui.set_clip_rect(stage.intersect(ui.clip_rect()));
				let image = images.show_large(ui, &attachment.media, fitted * zoom, demo);
				let response = ui
					.interact(image.rect, image.id.with("media"), Sense::click_and_drag())
					.on_hover_cursor(if zoom > 1.0 {
						egui::CursorIcon::Grab
					} else {
						egui::CursorIcon::ZoomIn
					})
					.on_hover_text("Scroll to zoom · Drag to pan · Double-click to reset")
					.on_hover_text(
						attachment
							.description
							.as_deref()
							.unwrap_or(&attachment.filename),
					);
				if response.dragged_by(egui::PointerButton::Primary) {
					pan = (pan + response.drag_delta()).clamp(-limit, limit);
				}
				if response.double_clicked() {
					zoom = 1.0;
					pan = egui::Vec2::ZERO;
				}
				media_context_menu(&response, attachment, download, opening, demo);
			}
			// Top bar: position counter on the left, actions on the right.
			let bar = Rect::from_min_size(full.min, egui::vec2(full.width(), TOP));
			if count > 1 {
				let pill = Rect::from_center_size(
					egui::pos2(bar.center().x, bar.min.y + 28.0),
					egui::vec2(60.0, 28.0),
				);
				ui.painter()
					.rect_filled(pill, 14, Color32::from_black_alpha(150));
				ui.painter().text(
					pill.center(),
					egui::Align2::CENTER_CENTER,
					format!("{} / {count}", index + 1),
					egui::FontId::proportional(13.0),
					Color32::from_gray(230),
				);
			}
			ui.scope_builder(
				egui::UiBuilder::new()
					.max_rect(bar.shrink2(egui::vec2(16.0, 8.0)))
					.layout(egui::Layout::right_to_left(egui::Align::Center)),
				|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					close = glass_button(ui, Icon::Close, 40.0, "Close (Esc)").clicked();
					let idle = !demo && !download.busy();
					if ui
						.add_enabled_ui(idle, |ui| {
							glass_button(ui, Icon::Download, 40.0, "Download")
						})
						.inner
						.on_disabled_hover_text(if demo {
							"Downloads are disabled for synthetic attachments"
						} else {
							"A download is already active"
						})
						.clicked()
					{
						download.request = Some(attachment.clone());
					}
				},
			);
			// Previous / next controls hug the stage edges.
			if count > 1 {
				for (previous, center, icon, label) in [
					(
						true,
						egui::pos2(full.left() + side / 2.0, stage.center().y),
						Icon::CaretLeft,
						"Previous (←)",
					),
					(
						false,
						egui::pos2(full.right() - side / 2.0, stage.center().y),
						Icon::ChevronRight,
						"Next (→)",
					),
				] {
					ui.scope_builder(
						egui::UiBuilder::new()
							.max_rect(Rect::from_center_size(center, egui::Vec2::splat(48.0))),
						|ui| {
							if glass_button(ui, icon, 48.0, label).clicked() {
								current =
									gallery_step(attachments, current, previous).unwrap_or(current);
							}
						},
					);
				}
			}
			// Caption: file name, size and dimensions, plus the browser link.
			let caption_width = image_rect.width().max(360.0).min(stage.width());
			let caption = Rect::from_min_size(
				egui::pos2(
					image_rect.left().min(stage.right() - caption_width),
					(image_rect.bottom() + 8.0).min(stage.bottom()),
				),
				egui::vec2(caption_width, 44.0),
			);
			ui.scope_builder(
				egui::UiBuilder::new()
					.max_rect(caption)
					.layout(egui::Layout::left_to_right(egui::Align::Center)),
				|ui| {
					ui.spacing_mut().item_spacing.x = 10.0;
					ui.add(
						egui::Label::new(
							design::semibold(ui, &attachment.filename, 14.0).color(Color32::WHITE),
						)
						.truncate()
						.selectable(false),
					);
					let mut meta = format_size(attachment.size);
					if attachment.media.width > 0 && attachment.media.height > 0 {
						meta = format!(
							"{meta} · {}×{}",
							attachment.media.width, attachment.media.height
						);
					}
					ui.add(
						egui::Label::new(
							RichText::new(meta)
								.size(13.0)
								.color(Color32::from_gray(170)),
						)
						.selectable(false),
					);
					if let Some(target) = attachment.media.url.as_deref().and_then(external_url)
						&& ui
							.add(
								egui::Label::new(
									design::medium(ui, "Open in browser", 13.0)
										.color(Color32::from_rgb(0, 168, 252)),
								)
								.sense(Sense::click())
								.selectable(false),
							)
							.on_hover_cursor(egui::CursorIcon::PointingHand)
							.clicked()
					{
						*opening = Some(target);
					}
					if download.active || !download.status.is_empty() {
						ui.add(
							egui::Label::new(
								RichText::new(&download.status)
									.size(13.0)
									.color(Color32::from_gray(170)),
							)
							.truncate()
							.selectable(false),
						);
						if download.active
							&& ui
								.add(
									egui::Label::new(
										design::medium(ui, "Cancel", 13.0)
											.color(Color32::from_rgb(0, 168, 252)),
									)
									.sense(Sense::click())
									.selectable(false),
								)
								.clicked()
						{
							download.cancel_requested = true;
						}
					}
				},
			);
			// Thumbnail strip: the whole gallery at a glance, current one highlighted.
			if count > 1 {
				const THUMB: f32 = 48.0;
				const GAP: f32 = 8.0;
				let strip_width = count as f32 * THUMB + (count - 1) as f32 * GAP;
				let strip = Rect::from_center_size(
					egui::pos2(full.center().x, full.bottom() - 36.0),
					egui::vec2(strip_width.min(full.width() - 32.0), THUMB),
				);
				ui.scope_builder(
					egui::UiBuilder::new()
						.max_rect(strip)
						.layout(egui::Layout::left_to_right(egui::Align::Center)),
					|ui| {
						ui.spacing_mut().item_spacing.x = GAP;
						for thumb in &gallery {
							ui.push_id(("thumb", thumb.id), |ui| {
								let response = images
									.show_banner(ui, &thumb.media, egui::Vec2::splat(THUMB), demo)
									.interact(Sense::click())
									.on_hover_text(&thumb.filename);
								let rect = response.rect;
								if thumb.id == current {
									ui.painter().rect_stroke(
										rect.expand(2.0),
										10,
										Stroke::new(2.0, Color32::WHITE),
										StrokeKind::Outside,
									);
								} else if !response.hovered() {
									ui.painter().rect_filled(
										rect,
										8,
										Color32::from_black_alpha(110),
									);
								}
								if response.clicked() {
									current = thumb.id;
								}
							});
						}
					},
				);
			}
			if backdrop.clicked() {
				close = true;
			}
		});
	let result = (!close && !overlay.should_close()).then_some(current);
	ui.ctx().data_mut(|data| {
		if result == Some(attachment.id) {
			data.insert_temp(zoom_id, (current, zoom, pan));
		} else {
			data.remove::<(Id, f32, egui::Vec2)>(zoom_id);
		}
	});
	result
}
pub fn estimated_height(attachments: &[Attachment], width: f32) -> f32 {
	attachments
		.chunk_by(|a, b| a.is_image() == b.is_image())
		.map(|group| {
			if group[0].is_image() {
				let (columns, size) = image_layout(group.len(), width);
				group
					.chunks(columns)
					.map(|row| {
						row.iter()
							.map(|a| artwork_size(a, size).y)
							.fold(0.0_f32, f32::max)
							+ 6.0
					})
					.sum::<f32>()
			} else {
				group
					.iter()
					.map(|attachment| {
						if attachment.is_video() {
							crate::video::estimated_height(attachment, width)
						} else if attachment.is_voice_message() {
							88.0
						} else if attachment.is_audio() {
							138.0
						} else {
							62.0
						}
					})
					.sum()
			}
		})
		.sum()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shared_artwork_keeps_compact_tiles_and_matching_row_heights() {
		let mut attachment = Attachment {
			id: Id(1),
			filename: String::new(),
			description: None,
			content_type: Some("image/png".into()),
			size: 1,
			spoiler: false,
			media: Default::default(),
			duration_ms: None,
			waveform: vec![],
		};
		let gallery = image_layout(1, 500.0).1;
		for name in ["emoji-7.gif", "sticker-8.png"] {
			attachment.filename = name.into();
			attachment.content_type = Some("image/png".into());
			assert_eq!(artwork_size(&attachment, gallery), egui::Vec2::splat(160.0));
			assert_eq!(estimated_height(&[attachment.clone()], 500.0), 166.0);
			assert_eq!(
				artwork_size(&attachment, egui::Vec2::splat(20.0)),
				egui::Vec2::splat(20.0)
			);
		}
		for name in [
			"photo.png",
			"emoji-0.png",
			"emoji-nope.png",
			"sticker-8.txt",
		] {
			attachment.filename = name.into();
			assert_eq!(artwork_size(&attachment, gallery), gallery);
		}
	}

	#[test]
	fn image_gallery_wraps_without_filenames_and_opens_each_attachment() {
		let mut message = test_support::message(1, Id(2));
		message.attachments = (0..3)
			.map(|id| Attachment {
				duration_ms: None,
				waveform: Vec::new(),
				id: Id(id + 10),
				filename: format!("synthetic-image-{id}.png"),
				description: None,
				content_type: Some("image/png".into()),
				size: 512,
				spoiler: false,
				media: model::EmbedMedia {
					width: 640,
					height: 360,
					..Default::default()
				},
			})
			.collect();
		let mut file = message.attachments[0].clone();
		file.id = Id(20);
		file.filename = "synthetic-report.pdf".into();
		file.content_type = Some("application/pdf".into());
		message.attachments.push(file);
		assert_eq!(
			gallery_step(&message.attachments, Id(10), true),
			Some(Id(12))
		);
		assert_eq!(
			gallery_step(&message.attachments, Id(12), false),
			Some(Id(10))
		);
		assert_eq!(
			gallery_step(&message.attachments, Id(10), false),
			Some(Id(11))
		);
		assert_eq!(gallery_step(&message.attachments, Id(20), false), None);
		assert_eq!(gallery_step(&[], Id(10), false), None);
		assert_eq!(
			gallery_step(&message.attachments[..1], Id(10), true),
			Some(Id(10))
		);

		for width in [420.0, 240.0] {
			let ctx = egui::Context::default();
			let mut images = Avatars::default();
			let mut viewing = None;
			let mut opening = None;
			let mut download = DownloadUi::default();
			let mut frame = |events| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width + 16.0, 800.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						ui.set_width(width);
						let mut surface = crate::select::Surface::new(ui, "attachment-test");
						show(
							ui,
							&message,
							&mut images,
							&mut viewing,
							&mut opening,
							&mut download,
							&mut crate::audio::AudioUi::default(),
							&mut crate::video::VideoUi::default(),
							true,
							&mut surface,
						);
						surface.finish(ui);
					},
				)
			};
			frame(vec![]).drop_without_applying_deltas();
			let output = frame(vec![]);
			let rects: Vec<_> = output
				.shapes
				.iter()
				.filter_map(|shape| match &shape.shape {
					egui::Shape::Rect(shape)
						if shape.corner_radius == egui::CornerRadius::same(5) =>
					{
						Some(shape.rect)
					}
					_ => None,
				})
				.collect();
			let labels: Vec<_> = output
				.shapes
				.iter()
				.filter_map(|shape| match &shape.shape {
					egui::Shape::Text(shape) => Some(shape.galley.job.text.as_str()),
					_ => None,
				})
				.collect();
			assert_eq!(rects.len(), 3);
			assert!(labels.contains(&"synthetic-report.pdf"));
			assert!(
				labels
					.iter()
					.all(|label| !label.contains("synthetic-image-"))
			);
			for rect in &rects {
				assert!(rect.width() <= width);
				assert!((rect.width() / rect.height() - 16.0 / 9.0).abs() < 0.01);
			}
			if width >= 280.0 {
				assert_eq!(rects[0].top(), rects[1].top());
				assert!(rects[1].left() > rects[0].right());
			} else {
				assert!(rects[1].top() > rects[0].bottom());
			}
			assert!(rects[2].top() > rects[0].bottom());
			let pos = rects[1].center();
			output.drop_without_applying_deltas();
			for pressed in [true, false] {
				frame(vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				])
				.drop_without_applying_deltas();
			}
			assert_eq!(viewing, Some((message.id, message.attachments[1].id)));
			assert!(opening.is_none() && download.request.is_none());
			assert!(images.take_requests().is_empty());
		}
		assert!(
			estimated_height(&message.attachments, 420.0)
				< estimated_height(&message.attachments, 240.0)
		);
	}

	#[test]
	fn viewer_closes_on_click_away_but_not_on_image_or_controls() {
		let attachments: Vec<_> = (0..2)
			.map(|id| Attachment {
				duration_ms: None,
				waveform: Vec::new(),
				id: Id(id + 10),
				filename: format!("synthetic-image-{id}.png"),
				description: None,
				content_type: Some("image/png".into()),
				size: 512,
				spoiler: false,
				media: model::EmbedMedia {
					width: 640,
					height: 360,
					..Default::default()
				},
			})
			.collect();
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let mut download = DownloadUi::default();
		let mut opening = None;
		let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0));
		let mut frame = |events, current| {
			let mut result = None;
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(screen),
					events,
					..Default::default()
				},
				|ui| {
					result = viewer(
						ui,
						&attachments,
						current,
						&mut images,
						&mut download,
						&mut opening,
						true,
					);
				},
			)
			.drop_without_applying_deltas();
			result
		};
		let click = |pos: egui::Pos2| {
			[true, false].map(|pressed| egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed,
				modifiers: egui::Modifiers::NONE,
			})
		};
		// Modal sizing pass, then a settled frame.
		assert_eq!(frame(vec![], Id(10)), Some(Id(10)));
		assert_eq!(frame(vec![], Id(10)), Some(Id(10)));
		// The image is centered on the stage and must swallow its own clicks.
		let center = screen.center();
		frame(vec![egui::Event::PointerMoved(center)], Id(10));
		let [down, up] = click(center);
		frame(vec![down], Id(10));
		assert_eq!(frame(vec![up], Id(10)), Some(Id(10)));
		// The next control sits on the right edge at the stage's vertical center (top bar 56,
		// thumbnail strip 116) and advances the gallery instead of closing.
		let next = egui::pos2(
			screen.right() - 42.0,
			56.0 + (screen.height() - 172.0) / 2.0,
		);
		frame(vec![egui::Event::PointerMoved(next)], Id(10));
		let [down, up] = click(next);
		frame(vec![down], Id(10));
		assert_eq!(frame(vec![up], Id(10)), Some(Id(11)));
		// Anywhere else on the dimmed backdrop closes the viewer; the bottom-left corner is
		// outside the image, the edge controls and the centered thumbnail strip.
		let away = egui::pos2(screen.left() + 40.0, screen.bottom() - 40.0);
		frame(vec![egui::Event::PointerMoved(away)], Id(11));
		let [down, up] = click(away);
		frame(vec![down], Id(11));
		assert_eq!(frame(vec![up], Id(11)), None);
		assert!(opening.is_none() && download.request.is_none());
	}

	#[test]
	fn nonimage_download_requires_keyboard_action_and_respects_single_job_controls() {
		fn frame(ctx: &egui::Context, key: Option<egui::Key>, draw: impl FnMut(&mut egui::Ui)) {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(640.0, 480.0),
					)),
					events: key
						.into_iter()
						.map(|key| egui::Event::Key {
							key,
							physical_key: None,
							pressed: true,
							repeat: false,
							modifiers: egui::Modifiers::NONE,
						})
						.collect(),
					..Default::default()
				},
				draw,
			);
			assert!(output.platform_output.commands.is_empty());
			output.drop_without_applying_deltas();
		}
		let attachment = Attachment {
			duration_ms: None,
			waveform: Vec::new(),
			id: Id(3),
			filename: "synthetic-report.pdf".into(),
			description: None,
			content_type: Some("application/pdf".into()),
			size: 512,
			spoiler: false,
			media: model::EmbedMedia {
				url: Some("https://cdn.discordapp.com/attachments/2/3/synthetic-report.pdf".into()),
				..Default::default()
			},
		};
		let message = Message {
			sticker_items: vec![],
			id: Id(1),
			channel: Id(2),
			author: model::User {
				id: Id(4),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			},
			content: String::new(),
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: vec![],
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			components: vec![],
			application_id: None,
			ephemeral: false,
			flags: 0,
			embeds: vec![],
			embeds_suppressed: false,
			reactions: None,
			attachments: vec![attachment.clone()],
		};
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let mut viewing = None;
		let mut opening = None;
		let mut download = DownloadUi::default();
		frame(&ctx, None, |ui| {
			show(
				ui,
				&message,
				&mut images,
				&mut viewing,
				&mut opening,
				&mut download,
				&mut crate::audio::AudioUi::default(),
				&mut crate::video::VideoUi::default(),
				false,
				&mut crate::select::Surface::new(ui, "attachment-test"),
			)
		});
		assert!(images.take_requests().is_empty());
		assert!(viewing.is_none() && opening.is_none() && download.request.is_none());
		for key in [egui::Key::Tab, egui::Key::Enter] {
			frame(&ctx, Some(key), |ui| {
				show(
					ui,
					&message,
					&mut images,
					&mut viewing,
					&mut opening,
					&mut download,
					&mut crate::audio::AudioUi::default(),
					&mut crate::video::VideoUi::default(),
					false,
					&mut crate::select::Surface::new(ui, "attachment-test"),
				)
			});
		}
		assert_eq!(download.request.as_ref(), Some(&attachment));
		assert!(download.request.as_ref().unwrap().bytes() <= model::MAX_ATTACHMENT_BYTES);
		assert!(images.take_requests().is_empty());
		assert!(viewing.is_none() && opening.is_none());

		for (demo, active, pending) in [
			(true, false, false),
			(false, true, false),
			(false, false, true),
		] {
			let ctx = egui::Context::default();
			let mut previous = attachment.clone();
			previous.id = Id(9);
			let mut download = DownloadUi {
				active,
				request: pending.then(|| previous.clone()),
				..Default::default()
			};
			for key in [None, Some(egui::Key::Tab), Some(egui::Key::Enter)] {
				frame(&ctx, key, |ui| {
					let _ = download_button(ui, &attachment, &mut download, demo);
				});
			}
			assert_eq!(download.request, pending.then_some(previous));
		}
		let ctx = egui::Context::default();
		let mut download = DownloadUi {
			active: true,
			status: "Downloading: 256 / 512 bytes".into(),
			..Default::default()
		};
		frame(&ctx, None, |ui| download.show_status(ui));
		assert!(!download.cancel_requested);
		for key in [egui::Key::Tab, egui::Key::Enter] {
			frame(&ctx, Some(key), |ui| download.show_status(ui));
		}
		assert!(download.cancel_requested);
	}
}
