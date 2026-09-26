//! Inline player: one bounded texture with Discord-style overlay controls. The desktop owns
//! media decoding and playback; this module only draws frames and emits commands.
use model::{Attachment, Id, Message};

const CORNER: u8 = 8;
const MAX_WIDTH: f32 = crate::avatars::media::MEDIA_MAX_WIDTH;
const MAX_HEIGHT: f32 = crate::avatars::media::MEDIA_MAX_HEIGHT;
const BAR_HEIGHT: f32 = 60.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoState {
	#[default]
	Idle,
	Loading,
	Playing,
	Paused,
	Ended,
	Failed(&'static str),
}
/// A provider player page (YouTube/Vimeo) the desktop hosts in a temporary webview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebVideo {
	/// Validated provider embed URL; the only page the webview opens.
	pub player: String,
	/// The shared link, offered as "Open in browser".
	pub page: Option<String>,
	pub title: String,
	pub provider: String,
	pub width: u32,
	pub height: u32,
}
pub enum VideoCommand {
	Play(Attachment),
	Pause(bool),
	Seek(f64),
	Volume(f32),
	Stop,
}
pub struct VideoUi {
	pub active: Option<(Id, Id, Attachment)>,
	pub state: VideoState,
	pub position: f64,
	pub duration: f64,
	pub command: Option<VideoCommand>,
	pub seen: bool,
	pub volume: f32,
	texture: Option<egui::TextureHandle>,
	/// Staging pixels for the current frame. The renderer drops its reference after the
	/// upload, so the same allocation is refilled each frame instead of reallocating up to
	/// 8 MB per frame (1080p at 60 fps churned ~500 MB/s through the allocator).
	frame: Option<std::sync::Arc<egui::ColorImage>>,
	/// Vertical transparent-to-black ramp behind the overlay controls.
	shade: Option<egui::TextureHandle>,
	/// Keyboard focus rested on an overlay control last frame, so keep the overlay visible.
	controls_focused: bool,
	/// Keep the viewport's previous mode so leaving playback restores the window.
	fullscreen: Option<(egui::Context, bool, egui::Id)>,
	/// Native window transition for the desktop to apply after this UI frame.
	fullscreen_request: Option<bool>,
	/// The open provider player; the desktop places its webview over `web_bounds`.
	pub web: Option<WebVideo>,
	pub web_bounds: Option<egui::Rect>,
	/// Painted in the player stage when no webview can be shown (offline demo).
	pub web_notice: Option<&'static str>,
	/// A link the provider player asked to open; routed through the link confirmation.
	pub web_external: Option<String>,
}
impl Default for VideoUi {
	fn default() -> Self {
		Self {
			active: None,
			state: VideoState::Idle,
			position: 0.0,
			duration: 0.0,
			command: None,
			seen: false,
			volume: 1.0,
			texture: None,
			frame: None,
			shade: None,
			controls_focused: false,
			fullscreen: None,
			fullscreen_request: None,
			web: None,
			web_bounds: None,
			web_notice: None,
			web_external: None,
		}
	}
}
impl VideoUi {
	pub fn stop(&mut self) {
		self.exit_fullscreen();
		self.active = None;
		self.texture = None;
		self.frame = None;
		self.state = VideoState::Idle;
		self.position = 0.0;
		self.duration = 0.0;
		self.seen = false;
		self.command = Some(VideoCommand::Stop);
	}
	fn exit_fullscreen(&mut self) {
		if let Some((ctx, previous, focus)) = self.fullscreen.take() {
			self.fullscreen_request = Some(previous);
			ctx.memory_mut(|memory| memory.request_focus(focus));
			ctx.request_repaint();
		}
	}
	pub fn take_fullscreen_request(&mut self) -> Option<bool> {
		self.fullscreen_request.take()
	}
	pub(super) fn is_fullscreen(&self) -> bool {
		self.fullscreen.is_some()
	}
	pub(super) fn show_fullscreen(
		&mut self,
		ctx: &egui::Context,
		message: &Message,
		attachment: &Attachment,
		original: Option<&str>,
		download: &mut crate::DownloadUi,
		opening: &mut Option<String>,
		demo: bool,
	) {
		// A modal sizing pass is invisible; it must not stop the active decoder.
		self.seen = true;
		let screen = ctx.content_rect();
		let id = egui::Id::unique("video-fullscreen");
		let overlay = egui::Modal::new(id)
			.area(
				egui::Modal::default_area(id)
					.anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
					.fade_in(false),
			)
			.backdrop_color(egui::Color32::BLACK)
			.frame(egui::Frame::NONE)
			.show(ctx, |ui| {
				ui.set_min_size(screen.size());
				ui.set_max_size(screen.size());
				let response = self.show_player(
					ui, message, attachment, true, original, false, download, opening, demo,
				);
				if !is_embedded(attachment) {
					crate::attachments::media_context_menu(
						&response, attachment, download, opening, demo,
					);
				}
			});
		// The shared link confirmation is drawn before the timeline. Leave the video
		// overlay when opening an original so that confirmation remains visible.
		if overlay.should_close() || opening.is_some() {
			self.exit_fullscreen();
		}
	}
	/// The desktop rejects stale session/player frames before handing over decoded pixels.
	pub fn accept_frame(
		&mut self,
		ctx: &egui::Context,
		width: usize,
		height: usize,
		rgba: &[u8],
	) -> bool {
		if self.active.is_none()
			|| width == 0
			|| height == 0
			|| width > 1920
			|| height > 1920
			|| width * height > 1920 * 1080
			|| rgba.len() != width * height * 4
		{
			return false;
		}
		let frame = self.frame.get_or_insert_with(|| {
			std::sync::Arc::new(egui::ColorImage::filled(
				[width, height],
				egui::Color32::BLACK,
			))
		});
		// Reuses the buffer once the previous upload released it; clones only if the
		// renderer still holds the last frame.
		let image = std::sync::Arc::make_mut(frame);
		image.size = [width, height];
		image.source_size = egui::vec2(width as f32, height as f32);
		image.pixels.clear();
		image.pixels.extend(
			rgba.as_chunks::<4>()
				.0
				.iter()
				.map(|&[r, g, b, a]| egui::Color32::from_rgba_unmultiplied(r, g, b, a)),
		);
		let image = std::sync::Arc::clone(frame);
		if let Some(texture) = &mut self.texture {
			texture.set(image, egui::TextureOptions::LINEAR);
		} else {
			self.texture =
				Some(ctx.load_texture("inline-video", image, egui::TextureOptions::LINEAR));
		}
		true
	}
	fn toggle(&mut self, message: &Message, attachment: &Attachment, state: VideoState) {
		self.command = Some(match state {
			VideoState::Loading => {
				self.stop();
				VideoCommand::Stop
			}
			VideoState::Playing => VideoCommand::Pause(true),
			VideoState::Paused => VideoCommand::Pause(false),
			_ => {
				self.active = Some((message.channel, message.id, attachment.clone()));
				self.texture = None;
				self.frame = None;
				self.state = VideoState::Loading;
				self.position = 0.0;
				self.duration = 0.0;
				VideoCommand::Play(attachment.clone())
			}
		});
	}
	fn shade(&mut self, ctx: &egui::Context) -> egui::TextureId {
		self.shade
			.get_or_insert_with(|| {
				let pixels = (0..16)
					.map(|row| egui::Color32::from_black_alpha((row * 200 / 15) as u8))
					.collect();
				ctx.load_texture(
					"inline-video-shade",
					egui::ColorImage {
						size: [1, 16],
						source_size: egui::vec2(1.0, 16.0),
						pixels,
					},
					egui::TextureOptions::LINEAR,
				)
			})
			.id()
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		message: &Message,
		attachment: &Attachment,
		download: &mut crate::DownloadUi,
		opening: &mut Option<String>,
		demo: bool,
	) -> egui::Response {
		let original = attachment.media.url.clone();
		self.show_player(
			ui,
			message,
			attachment,
			false,
			original.as_deref(),
			false,
			download,
			opening,
			demo,
		)
	}
	/// A directly proxied embed video over an already painted poster.
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn show_embedded(
		&mut self,
		ui: &mut egui::Ui,
		message: &Message,
		attachment: &Attachment,
		original: Option<&str>,
		poster: bool,
		download: &mut crate::DownloadUi,
		opening: &mut Option<String>,
		demo: bool,
	) -> egui::Response {
		self.show_player(
			ui, message, attachment, false, original, poster, download, opening, demo,
		)
	}
	#[allow(clippy::too_many_arguments)]
	fn show_player(
		&mut self,
		ui: &mut egui::Ui,
		message: &Message,
		attachment: &Attachment,
		fullscreen: bool,
		original: Option<&str>,
		poster: bool,
		download: &mut crate::DownloadUi,
		opening: &mut Option<String>,
		demo: bool,
	) -> egui::Response {
		let colors = crate::design::palette(ui);
		let active = self.active.as_ref().is_some_and(|(channel, id, file)| {
			*channel == message.channel && *id == message.id && file == attachment
		});
		let state = if active { self.state } else { VideoState::Idle };
		let width = ui.available_width().clamp(1.0, MAX_WIDTH);
		let size = if fullscreen {
			ui.available_size().max(egui::Vec2::splat(1.0))
		} else {
			stage_size(attachment, width)
		};
		let (stage, mut response) = ui.allocate_exact_size(size, egui::Sense::hover());
		if active && self.is_fullscreen() && !fullscreen {
			return response;
		}
		let label = match state {
			VideoState::Loading => "Cancel",
			VideoState::Playing => "Pause",
			VideoState::Paused => "Resume",
			VideoState::Ended => "Replay",
			VideoState::Failed(_) => "Retry",
			VideoState::Idle => "Play",
		};
		let painter = ui.painter().with_clip_rect(stage);
		if !poster || (active && self.texture.is_some()) {
			painter.rect_filled(stage, CORNER, egui::Color32::BLACK);
		}
		if let Some(texture) = self.texture.as_ref().filter(|_| active) {
			let size = texture.size_vec2();
			let scale = (stage.width() / size.x).min(stage.height() / size.y);
			let image_rect = egui::Rect::from_center_size(stage.center(), size * scale);
			egui::Image::new(egui::load::SizedTexture::new(
				texture.id(),
				image_rect.size(),
			))
			.corner_radius(CORNER)
			.paint_at(ui, image_rect);
		}
		let hovered = response.hovered() || ui.rect_contains_pointer(stage);
		let show_controls = active
			&& state != VideoState::Idle
			&& (hovered || state != VideoState::Playing || self.controls_focused);
		// The controls are painted over the stage. Keep playback interaction out of both
		// overlay bands so a control click cannot also become a play/pause click.
		let action_top = (stage.top() + 44.0).min(stage.bottom());
		let action_bottom =
			(stage.bottom() - if show_controls { BAR_HEIGHT } else { 0.0 }).max(action_top);
		let action = ui.interact(
			egui::Rect::from_min_max(
				egui::pos2(stage.left(), action_top),
				egui::pos2(stage.right(), action_bottom),
			),
			response.id.with("playback"),
			egui::Sense::click(),
		);
		action.widget_info(|| {
			egui::WidgetInfo::labeled(
				egui::Role::Button,
				ui.is_enabled(),
				format!("{label} video {}", attachment.filename),
			)
		});
		let center = if show_controls {
			stage.center() - egui::vec2(0.0, BAR_HEIGHT * 0.25)
		} else {
			stage.center()
		};
		let white = egui::Color32::WHITE;
		let show_center = match state {
			VideoState::Loading => {
				ui.put(
					egui::Rect::from_center_size(center, egui::Vec2::splat(36.0)),
					egui::Spinner::new().size(36.0).color(white),
				);
				false
			}
			VideoState::Playing => false,
			_ => true,
		};
		if show_center {
			let radius = 28.0;
			painter.circle_filled(
				center,
				radius,
				if hovered {
					egui::Color32::from_rgba_unmultiplied(0, 0, 0, 200)
				} else {
					egui::Color32::from_rgba_unmultiplied(0, 0, 0, 160)
				},
			);
			painter.circle_stroke(
				center,
				radius,
				egui::Stroke::new(1.5, egui::Color32::from_white_alpha(60)),
			);
			if matches!(state, VideoState::Ended | VideoState::Failed(_)) {
				crate::icons::paint(
					&painter,
					crate::icons::Icon::Reload,
					egui::Rect::from_center_size(center, egui::Vec2::splat(26.0)),
					white,
				);
			} else {
				painter.add(egui::Shape::convex_polygon(
					vec![
						center + egui::vec2(-8.0, -12.0),
						center + egui::vec2(13.0, 0.0),
						center + egui::vec2(-8.0, 12.0),
					],
					white,
					egui::Stroke::NONE,
				));
			}
		}
		if let VideoState::Failed(error) = state {
			let text_rect = egui::Rect::from_min_max(
				egui::pos2(stage.left() + 12.0, center.y + 38.0),
				egui::pos2(stage.right() - 12.0, stage.bottom()),
			);
			ui.scope_builder(
				egui::UiBuilder::new()
					.max_rect(text_rect)
					.layout(egui::Layout::top_down(egui::Align::Center)),
				|ui| {
					ui.add(
						egui::Label::new(
							egui::RichText::new(error)
								.size(12.0)
								.color(egui::Color32::from_white_alpha(230)),
						)
						.wrap(),
					)
					.on_hover_text(error);
				},
			);
		}
		if (show_controls && state != VideoState::Playing) || (!active && hovered) {
			let font = egui::FontId::proportional(12.0);
			let galley = painter.layout(
				attachment.filename.clone(),
				font,
				white,
				(stage.width() - 32.0).max(16.0),
			);
			let pill = egui::Rect::from_min_size(
				stage.left_top() + egui::vec2(8.0, 8.0),
				galley.size() + egui::vec2(12.0, 6.0),
			);
			painter.rect_filled(pill, 4, egui::Color32::from_black_alpha(150));
			painter.galley(pill.min + egui::vec2(6.0, 3.0), galley, white);
		}
		if state == VideoState::Idle
			&& let Some(duration) = attachment.duration_ms.filter(|ms| *ms > 0)
		{
			let galley = painter.layout_no_wrap(
				timestamp(duration as f64 / 1000.0),
				egui::FontId::proportional(12.0),
				white,
			);
			let badge = egui::Rect::from_min_size(
				egui::pos2(
					stage.left() + 8.0,
					stage.bottom() - 8.0 - galley.size().y - 6.0,
				),
				galley.size() + egui::vec2(12.0, 6.0),
			);
			painter.rect_filled(badge, 4, egui::Color32::from_black_alpha(150));
			painter.galley(badge.min + egui::vec2(6.0, 3.0), galley, white);
		}
		if action.clicked() {
			self.toggle(message, attachment, state);
		}
		response = action | response;
		let mut controls_focused = false;
		let context_click = ui.input(|i| {
			i.pointer.button_down(egui::PointerButton::Secondary)
				|| i.pointer.button_released(egui::PointerButton::Secondary)
		});
		if show_controls {
			let bar = egui::Rect::from_min_max(
				egui::pos2(stage.left(), stage.bottom() - BAR_HEIGHT),
				stage.right_bottom(),
			);
			let shade = self.shade(ui.ctx());
			egui::Image::new(egui::load::SizedTexture::new(shade, bar.size()))
				.corner_radius(egui::CornerRadius {
					nw: 0,
					ne: 0,
					sw: CORNER,
					se: CORNER,
				})
				.paint_at(ui, bar);
			let duration = if self.duration.is_finite() && self.duration > 0.0 {
				self.duration
			} else {
				1.0
			};
			let can_seek = self.duration.is_finite()
				&& self.duration > 0.0
				&& matches!(state, VideoState::Playing | VideoState::Paused);
			let inner = bar.shrink2(egui::vec2(10.0, 6.0));
			ui.scope_builder(
				egui::UiBuilder::new()
					.max_rect(inner)
					.layout(egui::Layout::top_down(egui::Align::Min)),
				|ui| {
					let visuals = ui.visuals_mut();
					visuals.selection.bg_fill = colors.accent;
					visuals.widgets.inactive.bg_fill = egui::Color32::from_white_alpha(70);
					visuals.widgets.hovered.bg_fill = egui::Color32::from_white_alpha(110);
					visuals.widgets.active.bg_fill = egui::Color32::from_white_alpha(140);
					visuals.widgets.inactive.fg_stroke.color = white;
					visuals.widgets.hovered.fg_stroke.color = white;
					visuals.widgets.active.fg_stroke.color = white;
					visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
					visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
					visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
					ui.spacing_mut().item_spacing = egui::vec2(8.0, 2.0);
					ui.spacing_mut().slider_rail_height = 4.0;
					ui.spacing_mut().interact_size.y = 18.0;
					ui.spacing_mut().slider_width = ui.available_width().max(16.0);
					let mut position = self.position.max(0.0);
					let seek = ui.add_enabled(
						can_seek,
						egui::Slider::new(&mut position, 0.0..=duration)
							.show_value(false)
							.trailing_fill(true),
					);
					seek.widget_info(|| egui::WidgetInfo::slider(can_seek, position, "Seek video"));
					controls_focused |= seek.has_focus();
					response |= seek.clone();
					if seek.on_hover_text("Seek video").changed() && !context_click {
						self.command = Some(VideoCommand::Seek(position));
					}
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
						let (glyph_rect, glyph) =
							ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::CLICK);
						let glyph_color = if glyph.hovered() {
							white
						} else {
							egui::Color32::from_white_alpha(220)
						};
						let c = glyph_rect.center();
						match state {
							VideoState::Playing => {
								for x in [-3.5, 3.5] {
									ui.painter().rect_filled(
										egui::Rect::from_center_size(
											c + egui::vec2(x, 0.0),
											egui::vec2(3.5, 13.0),
										),
										1,
										glyph_color,
									);
								}
							}
							VideoState::Paused | VideoState::Idle => {
								ui.painter().add(egui::Shape::convex_polygon(
									vec![
										c + egui::vec2(-5.0, -7.0),
										c + egui::vec2(7.0, 0.0),
										c + egui::vec2(-5.0, 7.0),
									],
									glyph_color,
									egui::Stroke::NONE,
								));
							}
							VideoState::Loading => crate::icons::paint(
								ui.painter(),
								crate::icons::Icon::Close,
								glyph_rect.shrink(4.0),
								glyph_color,
							),
							VideoState::Ended | VideoState::Failed(_) => crate::icons::paint(
								ui.painter(),
								crate::icons::Icon::Reload,
								glyph_rect.shrink(3.0),
								glyph_color,
							),
						}
						response |= glyph.clone();
						if glyph.on_hover_text(label).clicked() {
							self.toggle(message, attachment, state);
						}
						ui.label(
							egui::RichText::new(format!(
								"{} / {}",
								timestamp(self.position),
								if self.duration > 0.0 {
									timestamp(self.duration)
								} else {
									"--:--".into()
								}
							))
							.size(12.0)
							.color(white),
						);
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							let (rect, button) = ui
								.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::click());
							let label = if fullscreen {
								"Exit fullscreen (Esc)"
							} else {
								"Fullscreen"
							};
							button.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									ui.is_enabled(),
									label,
								)
							});
							if fullscreen {
								crate::icons::paint(
									ui.painter(),
									crate::icons::Icon::Close,
									rect.shrink(3.0),
									white,
								);
							} else {
								for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
									let corner = rect.center() + egui::vec2(x * 7.0, y * 7.0);
									ui.painter().add(egui::Shape::line(
										vec![
											corner - egui::vec2(x * 5.0, 0.0),
											corner,
											corner - egui::vec2(0.0, y * 5.0),
										],
										egui::Stroke::new(1.5, white),
									));
								}
							}
							controls_focused |= button.has_focus();
							if button.has_focus() {
								ui.painter().rect_stroke(
									rect,
									3,
									egui::Stroke::new(2.0, colors.accent),
									egui::StrokeKind::Inside,
								);
							}
							response |= button.clone();
							let button_id = button.id;
							if button
								.on_hover_text(label)
								.on_hover_cursor(egui::CursorIcon::PointingHand)
								.clicked()
							{
								if fullscreen {
									self.exit_fullscreen();
								} else {
									let previous =
										ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
									self.fullscreen = Some((ui.ctx().clone(), previous, button_id));
									self.fullscreen_request = Some(true);
								}
							}
							ui.spacing_mut().slider_width =
								(ui.available_width() - 24.0).clamp(24.0, 56.0);
							let mut volume_value = self.volume;
							let volume = ui.add(
								egui::Slider::new(&mut volume_value, 0.0..=1.0)
									.show_value(false)
									.trailing_fill(true),
							);
							volume.widget_info(|| {
								egui::WidgetInfo::slider(
									ui.is_enabled(),
									volume_value as f64,
									"Video volume",
								)
							});
							controls_focused |= volume.has_focus();
							response |= volume.clone();
							if volume.on_hover_text("Video volume").changed() && !context_click {
								self.volume = volume_value;
								self.command = Some(VideoCommand::Volume(self.volume));
							}
							crate::icons::inline(
								ui,
								crate::icons::Icon::Speaker,
								16.0,
								egui::Color32::from_white_alpha(220),
							);
						});
					});
				},
			);
		}
		self.controls_focused = controls_focused;
		// Download/open controls stay visible regardless of load state, unlike the bottom bar.
		// Added last so Tab order still reaches the playback controls first.
		let overlay = egui::Rect::from_min_size(
			egui::pos2(stage.left() + 8.0, stage.top() + 8.0),
			egui::vec2((stage.width() - 16.0).max(0.0), 28.0),
		);
		ui.scope_builder(
			egui::UiBuilder::new()
				.max_rect(overlay)
				.layout(egui::Layout::right_to_left(egui::Align::Min)),
			|ui| {
				ui.spacing_mut().item_spacing.x = 6.0;
				let idle = !demo && !download.busy();
				if !is_embedded(attachment)
					&& ui
						.add_enabled_ui(idle, |ui| {
							crate::attachments::glass_button(
								ui,
								crate::icons::Icon::Download,
								28.0,
								"Download",
							)
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
				if let Some(url) = original.and_then(crate::markdown::external_url)
					&& crate::attachments::glass_button(
						ui,
						crate::icons::Icon::External,
						28.0,
						"Open original…",
					)
					.clicked()
				{
					*opening = Some(url);
				}
			},
		);
		if response.has_focus() {
			ui.painter().rect_stroke(
				stage,
				CORNER,
				egui::Stroke::new(2.0, colors.accent),
				egui::StrokeKind::Inside,
			);
		}
		if ui.is_rect_visible(stage)
			&& self.active.as_ref().is_some_and(|(channel, id, file)| {
				*channel == message.channel && *id == message.id && file == attachment
			}) {
			self.seen = true;
		}
		response
	}
}
impl VideoUi {
	/// A provider embed's poster; playing opens the theater player instead of a browser.
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn show_provider(
		&mut self,
		ui: &mut egui::Ui,
		video: &WebVideo,
		poster: Option<&model::EmbedMedia>,
		images: &mut crate::avatars::Avatars,
		opening: &mut Option<String>,
		demo: bool,
	) {
		let size = provider_stage(video, ui.available_width());
		let stage = egui::Rect::from_min_size(ui.cursor().min, size);
		if let Some(poster) = poster {
			ui.scope_builder(egui::UiBuilder::new().max_rect(stage), |ui| {
				images.show_media(ui, poster, size, demo, crate::avatars::Surface::Banner);
			});
		}
		let (stage, _) = ui.allocate_exact_size(size, egui::Sense::hover());
		if poster.is_none() {
			ui.painter()
				.rect_filled(stage, CORNER, egui::Color32::BLACK);
		}
		let play = ui.interact(
			stage.shrink2(egui::vec2(0.0, 44.0).min(stage.size() / 3.0)),
			ui.scope_id().with(("provider-play", &video.player)),
			egui::Sense::click(),
		);
		let label = format!("Play {} video {}", video.provider, video.title);
		play.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
		let hovered = ui.rect_contains_pointer(stage);
		let painter = ui.painter().with_clip_rect(stage);
		let center = stage.center();
		painter.circle_filled(
			center,
			28.0,
			egui::Color32::from_black_alpha(if hovered { 200 } else { 160 }),
		);
		painter.circle_stroke(
			center,
			28.0,
			egui::Stroke::new(1.5, egui::Color32::from_white_alpha(60)),
		);
		painter.add(egui::Shape::convex_polygon(
			vec![
				center + egui::vec2(-8.0, -12.0),
				center + egui::vec2(13.0, 0.0),
				center + egui::vec2(-8.0, 12.0),
			],
			egui::Color32::WHITE,
			egui::Stroke::NONE,
		));
		if play.has_focus() {
			painter.rect_stroke(
				stage,
				CORNER,
				egui::Stroke::new(2.0, crate::design::palette(ui).accent),
				egui::StrokeKind::Inside,
			);
		}
		if let Some(page) = &video.page {
			ui.scope_builder(
				egui::UiBuilder::new()
					.max_rect(egui::Rect::from_min_size(
						egui::pos2(stage.left() + 8.0, stage.top() + 8.0),
						egui::vec2((stage.width() - 16.0).max(0.0), 28.0),
					))
					.layout(egui::Layout::right_to_left(egui::Align::Min)),
				|ui| {
					if crate::attachments::glass_button(
						ui,
						crate::icons::Icon::External,
						28.0,
						"Open in browser…",
					)
					.clicked()
					{
						*opening = Some(page.clone());
					}
				},
			);
		}
		if play.on_hover_text("Play here").clicked() {
			self.stop();
			self.web = Some(video.clone());
			self.web_notice = None;
		}
	}
	/// Theater overlay for the provider player. The desktop places the webview over the
	/// stage reserved here, so nothing else may paint above it while it is open.
	pub(super) fn show_web_player(&mut self, ctx: &egui::Context, opening: &mut Option<String>) {
		if let Some(url) = self.web_external.take() {
			*opening = crate::markdown::external_url(&url);
			self.web = None;
		}
		let Some(video) = self.web.clone() else {
			self.web_bounds = None;
			return;
		};
		let screen = ctx.content_rect();
		let id = egui::Id::unique("provider-video-player");
		let mut close = false;
		let modal = egui::Modal::new(id)
			.area(
				egui::Modal::default_area(id)
					.anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
					.fade_in(false),
			)
			.backdrop_color(egui::Color32::from_black_alpha(220))
			.frame(egui::Frame::NONE)
			.show(ctx, |ui| {
				ui.set_min_size(screen.size());
				ui.set_max_size(screen.size());
				let header = 44.0;
				let available =
					(screen.size() - egui::vec2(96.0, 96.0 + header)).max(egui::vec2(160.0, 90.0));
				let aspect = if video.width > 0 && video.height > 0 {
					(video.width as f32 / video.height as f32).clamp(0.5, 2.4)
				} else {
					16.0 / 9.0
				};
				let width = available.x.min(available.y * aspect).min(1600.0);
				let size = egui::vec2(width, width / aspect);
				let stage = egui::Rect::from_center_size(
					screen.center() + egui::vec2(0.0, header / 2.0),
					size,
				);
				let bar = egui::Rect::from_min_max(
					egui::pos2(stage.left(), stage.top() - header),
					egui::pos2(stage.right(), stage.top() - 8.0),
				);
				ui.scope_builder(
					egui::UiBuilder::new()
						.max_rect(bar)
						.layout(egui::Layout::right_to_left(egui::Align::Center)),
					|ui| {
						ui.spacing_mut().item_spacing.x = 8.0;
						if crate::attachments::glass_button(
							ui,
							crate::icons::Icon::Close,
							32.0,
							"Close player (Esc)",
						)
						.clicked()
						{
							close = true;
						}
						if let Some(page) = &video.page
							&& crate::attachments::glass_button(
								ui,
								crate::icons::Icon::External,
								32.0,
								"Open in browser…",
							)
							.clicked()
						{
							*opening = Some(page.clone());
							close = true;
						}
						ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
							ui.add(
								egui::Label::new(
									egui::RichText::new(if video.title.is_empty() {
										&video.provider
									} else {
										&video.title
									})
									.strong()
									.color(egui::Color32::WHITE),
								)
								.truncate(),
							);
						});
					},
				);
				ui.painter()
					.rect_filled(stage, CORNER, egui::Color32::BLACK);
				if let Some(notice) = self.web_notice {
					ui.painter().text(
						stage.center(),
						egui::Align2::CENTER_CENTER,
						notice,
						egui::FontId::proportional(14.0),
						egui::Color32::from_white_alpha(220),
					);
				}
				stage
			});
		if close || modal.should_close() || opening.is_some() {
			self.web = None;
			self.web_bounds = None;
		} else {
			self.web_bounds = Some(modal.inner);
		}
	}
}
impl Drop for VideoUi {
	fn drop(&mut self) {
		self.exit_fullscreen();
	}
}
pub(crate) fn is_embedded(attachment: &Attachment) -> bool {
	attachment.size == 0
}
fn provider_stage(video: &WebVideo, width: f32) -> egui::Vec2 {
	let (w, h) = if video.width > 0 && video.height > 0 {
		(video.width as f32, video.height as f32)
	} else {
		(16.0, 9.0)
	};
	let width = width.clamp(1.0, MAX_WIDTH);
	let scale = (width / w).min(MAX_HEIGHT / h);
	(egui::vec2(w, h) * scale).max(egui::vec2(1.0, 1.0))
}
pub(crate) fn embed_stage_height(width: u32, height: u32, available: f32) -> f32 {
	let (w, h) = if width > 0 && height > 0 {
		(width as f32, height as f32)
	} else {
		(16.0, 9.0)
	};
	let available = available.clamp(1.0, MAX_WIDTH);
	h * (available / w).min(MAX_HEIGHT / h)
}
pub(crate) fn stage_size(attachment: &Attachment, width: f32) -> egui::Vec2 {
	let width = width.clamp(1.0, MAX_WIDTH);
	let (native_width, native_height) = if attachment.media.width > 0 && attachment.media.height > 0
	{
		(
			attachment.media.width as f32,
			attachment.media.height as f32,
		)
	} else {
		(16.0, 9.0)
	};
	let scale = (width / native_width)
		.min(MAX_HEIGHT / native_height)
		.min(1.0);
	(egui::vec2(native_width, native_height) * scale).max(egui::vec2(1.0, 1.0))
}
/// Stage plus the spacing after each attachment; all controls are overlays or menu actions.
pub(super) fn estimated_height(attachment: &Attachment, width: f32) -> f32 {
	stage_size(attachment, width.clamp(1.0, MAX_WIDTH)).y + 6.0
}
fn timestamp(seconds: f64) -> String {
	let seconds = seconds.max(0.0) as u64;
	format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn video_controls_are_explicit_bounded_and_keyboard_operable() {
		let mut message = test_support::message(1, Id(2));
		let attachment = Attachment {
			id: Id(3),
			filename: "synthetic-clip.MOV".into(),
			description: None,
			content_type: Some("application/octet-stream".into()),
			size: 128,
			media: model::EmbedMedia {
				width: 1920,
				height: 1080,
				..Default::default()
			},
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		};
		message.attachments.push(attachment.clone());
		for (width, theme) in [(220.0, egui::Theme::Dark), (420.0, egui::Theme::Light)] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.set_theme(theme);
			let mut video = VideoUi::default();
			let frame = |video: &mut VideoUi, key: Option<egui::Key>| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width + 16.0, 600.0),
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
					|ui| {
						ui.set_width(width);
						crate::attachments::show(
							ui,
							&message,
							&mut crate::avatars::Avatars::default(),
							&mut None,
							&mut None,
							&mut crate::attachments::DownloadUi::default(),
							&mut crate::AudioUi::default(),
							video,
							false,
							&mut crate::select::Surface::new(ui, "attachment-test"),
						);
						assert!(ui.min_rect().width() <= width + 2.0);
						// Only the stage and attachment spacing drive the layout estimate;
						// overlay controls and menu actions never add height.
						assert!(
							(ui.min_rect().height() - estimated_height(&attachment, width)).abs()
								< 24.0,
							"{} vs {}",
							ui.min_rect().height(),
							estimated_height(&attachment, width)
						);
					},
				)
				.drop_without_applying_deltas();
			};
			frame(&mut video, None);
			assert!(video.command.is_none() && video.active.is_none());
			assert!(!video.accept_frame(&ctx, 1, 1, &[0, 0, 0, 255]));
			for key in [egui::Key::Tab, egui::Key::Enter] {
				frame(&mut video, Some(key));
			}
			assert!(
				matches!(video.command.take(), Some(VideoCommand::Play(file)) if file == attachment)
			);
			assert!(video.seen);
			assert!(!video.accept_frame(&ctx, 1921, 1080, &[]));
			assert!(!video.accept_frame(&ctx, 1, 1921, &[]));
			assert!(!video.accept_frame(&ctx, 1920, 1920, &[]));
			assert!(!video.accept_frame(&ctx, usize::MAX, usize::MAX, &[]));
			assert!(!video.accept_frame(&ctx, 1, 1, &[0]));
			assert!(video.accept_frame(&ctx, 1, 1920, &vec![0; 1920 * 4]));
			assert!(video.accept_frame(&ctx, 1, 1, &[0, 0, 0, 255]));
			video.state = VideoState::Playing;
			video.duration = 12.0;
			frame(&mut video, Some(egui::Key::Enter));
			assert!(matches!(
				video.command.take(),
				Some(VideoCommand::Pause(true))
			));
			video.state = VideoState::Paused;
			frame(&mut video, Some(egui::Key::Enter));
			assert!(matches!(
				video.command.take(),
				Some(VideoCommand::Pause(false))
			));
			for key in [egui::Key::Tab, egui::Key::ArrowRight] {
				frame(&mut video, Some(key));
			}
			assert!(matches!(video.command.take(), Some(VideoCommand::Seek(value)) if value > 0.0));
			// While a control keeps keyboard focus, playback resuming must not hide the overlay.
			video.state = VideoState::Playing;
			frame(&mut video, Some(egui::Key::ArrowRight));
			assert!(matches!(video.command.take(), Some(VideoCommand::Seek(_))));
			for key in [egui::Key::Tab, egui::Key::Tab, egui::Key::ArrowLeft] {
				frame(&mut video, Some(key));
			}
			assert!(
				matches!(video.command.take(), Some(VideoCommand::Volume(value)) if value < 1.0)
			);
			video.state = VideoState::Failed("Unsupported video codec");
			frame(&mut video, None);
			video.stop();
			assert!(video.active.is_none() && video.texture.is_none());
			assert!(matches!(video.command, Some(VideoCommand::Stop)));
		}
	}
}
