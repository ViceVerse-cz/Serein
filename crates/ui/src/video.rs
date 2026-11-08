//! Inline player: one bounded texture with Discord-style overlay controls. The desktop owns
//! media decoding and playback; this module only draws frames and emits commands.
use model::{Attachment, Id, Message};
use std::collections::{HashMap, HashSet, VecDeque};

/// Poster pixel budget: 8 MiB RGBA, about three dozen 320px posters. Oldest goes
/// first; evicted attachments may request again, cached ones never re-download.
const POSTER_BUDGET: usize = 8 * 1024 * 1024;
/// Poster count ceiling, so tiny clips cannot pile up textures under the byte budget.
const POSTER_MAX: usize = 128;
/// Remembered undecodable posters; past this the failures are forgotten and may retry.
const POSTER_SEEN_MAX: usize = 1024;

/// Reinterprets decoded RGBA as texels without a per-pixel pass. Every backend emits
/// opaque frames, where premultiplied and straight alpha are the same bytes; a per-pixel
/// `from_rgba_unmultiplied` cost a full core at 1080p30 in debug builds.
fn video_pixels(rgba: &[u8]) -> &[egui::Color32] {
	bytemuck::cast_slice(rgba)
}

const CORNER: u8 = 8;
const MAX_WIDTH: f32 = crate::avatars::media::MEDIA_MAX_WIDTH;
const MAX_HEIGHT: f32 = crate::avatars::media::MEDIA_MAX_HEIGHT;
const BAR_HEIGHT: f32 = 60.0;
/// Decoded pictures share the platform's inline bound: 4K worth of pixels in a 3840 px square.
const MAX_FRAME_EDGE: usize = 3840;
const MAX_FRAME_PIXELS: usize = 3840 * 2160;

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
pub enum VideoCommand {
	Play(Attachment),
	Pause(bool),
	Seek(f64),
	Volume(f32),
	/// Playback rate; the desktop clamps it through [`crate::media`].
	Speed(f32),
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
	/// Session-local rate for this media type; `1.0` unless the owner changes it.
	pub rate: f32,
	/// Latest frame; usually the desktop's GPU-converted texture, else [`Self::managed`].
	texture: Option<egui::load::SizedTexture>,
	/// CPU fallback texture behind [`VideoUi::accept_frame`].
	managed: Option<egui::TextureHandle>,
	/// Staging pixels for the current frame. The renderer drops its reference after the
	/// upload, so the same allocation is refilled each frame instead of reallocating up to
	/// up to 33 MB per 4K frame (1080p at 60 fps alone churned ~500 MB/s through the allocator).
	frame: Option<std::sync::Arc<egui::ColorImage>>,
	/// Vertical transparent-to-black ramp behind the overlay controls.
	shade: Option<egui::TextureHandle>,
	/// Decoded first frames by attachment id, painted on idle stages instead of
	/// black. Byte-bounded below; Discord ships no video placeholders to reuse.
	pub(crate) posters: HashMap<Id, (egui::TextureHandle, usize)>,
	/// Oldest-first poster insertion order for budget eviction.
	poster_order: VecDeque<Id>,
	/// Live poster bytes against [`POSTER_BUDGET`].
	poster_bytes: usize,
	/// Attachments already asked (or answered) for a poster; no retry until eviction.
	pub(crate) poster_seen: HashSet<Id>,
	/// Idle attachments awaiting one decoded poster. Drained by the desktop each
	/// frame through its own queue so poster work never consumes the single
	/// playback command slot (and a later click cannot strand a request).
	pub poster_queue: Vec<Attachment>,
	/// Keyboard focus rested on an overlay control last frame, so keep the overlay visible.
	controls_focused: bool,
	/// Volume to restore when the mute button is pressed again.
	unmuted_volume: Option<f32>,
	/// Keep the viewport's previous mode so leaving playback restores the window.
	fullscreen: Option<(egui::Context, bool, egui::Id)>,
	/// Native window transition for the desktop to apply after this UI frame.
	pub(super) fullscreen_request: Option<bool>,
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
			rate: 1.0,
			texture: None,
			managed: None,
			frame: None,
			shade: None,
			posters: HashMap::new(),
			poster_order: VecDeque::new(),
			poster_bytes: 0,
			poster_seen: HashSet::new(),
			poster_queue: Vec::new(),
			controls_focused: false,
			unmuted_volume: None,
			fullscreen: None,
			fullscreen_request: None,
		}
	}
}
impl VideoUi {
	pub fn stop(&mut self) {
		self.exit_fullscreen();
		self.active = None;
		self.texture = None;
		self.managed = None;
		self.frame = None;
		self.state = VideoState::Idle;
		self.position = 0.0;
		self.duration = 0.0;
		self.seen = false;
		self.command = Some(VideoCommand::Stop);
	}
	pub(super) fn exit_fullscreen(&mut self) {
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
				let response =
					self.show_player(ui, message, attachment, true, download, opening, demo);
				crate::attachments::media_context_menu(
					&response, attachment, download, opening, demo,
				);
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
			|| width > MAX_FRAME_EDGE
			|| height > MAX_FRAME_EDGE
			|| width * height > MAX_FRAME_PIXELS
			|| width.max(height) > ctx.input(|i| i.max_texture_side)
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
		image.pixels.extend_from_slice(video_pixels(rgba));
		let image = std::sync::Arc::clone(frame);
		let texture = match &mut self.managed {
			Some(texture) => {
				texture.set(image, egui::TextureOptions::LINEAR);
				texture
			}
			None => self.managed.insert(ctx.load_texture(
				"inline-video",
				image,
				egui::TextureOptions::LINEAR,
			)),
		};
		self.texture = Some(egui::load::SizedTexture::from_handle(texture));
		true
	}
	/// Shows a frame the desktop already uploaded (and converted) on the GPU.
	pub fn show_frame(&mut self, texture: egui::load::SizedTexture) -> bool {
		if self.active.is_none() {
			return false;
		}
		self.texture = Some(texture);
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
				self.managed = None;
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
	/// Caches one decoded poster frame; same bounds as [`VideoUi::accept_frame`]
	/// but independent of the active playback, so idle stages keep their preview.
	pub fn accept_poster(
		&mut self,
		ctx: &egui::Context,
		id: Id,
		width: usize,
		height: usize,
		rgba: &[u8],
	) -> bool {
		if width == 0
			|| height == 0
			|| width > MAX_FRAME_EDGE
			|| height > MAX_FRAME_EDGE
			|| width * height > MAX_FRAME_PIXELS
			|| width.max(height) > ctx.input(|i| i.max_texture_side)
			|| rgba.len() != width * height * 4
		{
			return false;
		}
		let bytes = rgba.len();
		// Reinserting refreshes recency; eviction below drops textures (GPU freed).
		self.evict_poster(&id);
		while self.poster_bytes + bytes > POSTER_BUDGET || self.posters.len() >= POSTER_MAX {
			let Some(oldest) = self.poster_order.pop_front() else {
				break;
			};
			self.evict_poster(&oldest);
		}
		let image = egui::ColorImage {
			size: [width, height],
			source_size: egui::vec2(width as f32, height as f32),
			pixels: video_pixels(rgba).to_vec(),
		};
		self.posters.insert(
			id,
			(
				ctx.load_texture("inline-video-poster", image, egui::TextureOptions::LINEAR),
				bytes,
			),
		);
		self.poster_order.push_back(id);
		self.poster_bytes += bytes;
		self.poster_seen.insert(id);
		true
	}
	/// Drops one cached poster and its retry guard, freeing its budget share.
	fn evict_poster(&mut self, id: &Id) {
		if let Some((_, bytes)) = self.posters.remove(id) {
			self.poster_bytes = self.poster_bytes.saturating_sub(bytes);
			self.poster_order.retain(|queued| queued != id);
			self.poster_seen.remove(id);
		}
	}
	/// Marks one cached poster as recently used so repeat views keep it.
	fn touch_poster(&mut self, id: &Id) {
		if self.poster_order.back() != Some(id) && self.posters.contains_key(id) {
			self.poster_order.retain(|queued| queued != id);
			self.poster_order.push_back(*id);
		}
	}
	/// Records an undecodable poster so the stage is not requested again until eviction.
	pub fn poster_failed(&mut self, id: Id) {
		if self.poster_seen.len() >= POSTER_SEEN_MAX {
			let posters = &self.posters;
			self.poster_seen.retain(|seen| posters.contains_key(seen));
		}
		self.poster_seen.insert(id);
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
		self.show_player(ui, message, attachment, false, download, opening, demo)
	}
	#[allow(clippy::too_many_arguments)]
	fn show_player(
		&mut self,
		ui: &mut egui::Ui,
		message: &Message,
		attachment: &Attachment,
		fullscreen: bool,
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
		let label = crate::i18n::translate_if_key(match state {
			VideoState::Loading => "video-show-player-cancel",
			VideoState::Playing => "video-show-player-pause",
			VideoState::Paused => "video-show-player-resume",
			VideoState::Ended => "video-show-player-replay",
			VideoState::Failed(_) => "video-show-player-retry",
			VideoState::Idle => "video-show-player-play",
		});
		let painter = ui.painter().with_clip_rect(stage);
		painter.rect_filled(stage, CORNER, egui::Color32::BLACK);
		if self.texture.as_ref().filter(|_| active).is_none() {
			if let Some((poster, _)) = self.posters.get(&attachment.id) {
				let size = poster.size_vec2();
				if size.x > 0.0 && size.y > 0.0 {
					// Contain, like playback: the decoded aspect can differ
					// from the attachment metadata, and cover would paint
					// outside the allocated stage over nearby content.
					let scale = (stage.width() / size.x).min(stage.height() / size.y);
					let rect = egui::Rect::from_center_size(stage.center(), size * scale);
					egui::Image::new(egui::load::SizedTexture::new(poster.id(), rect.size()))
						.corner_radius(CORNER)
						.paint_at(ui, rect);
					self.touch_poster(&attachment.id);
				}
			} else if state == VideoState::Idle
				&& !self.poster_seen.contains(&attachment.id)
				&& attachment.size > 0
				&& attachment.size <= 100 * 1024 * 1024
				&& ui.is_rect_visible(stage)
				&& !self
					.poster_queue
					.iter()
					.any(|queued| queued.id == attachment.id)
			{
				// Queued, not commanded: playback clicks keep the command slot.
				self.poster_queue.push(attachment.clone());
			}
		}
		if let Some(texture) = self.texture.filter(|_| active) {
			let size = texture.size;
			let scale = (stage.width() / size.x).min(stage.height() / size.y);
			let image_rect = egui::Rect::from_center_size(stage.center(), size * scale);
			egui::Image::new(egui::load::SizedTexture::new(texture.id, image_rect.size()))
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
				format!(
					"{label} {} {}",
					crate::i18n::translate("video-show-player-video"),
					attachment.filename
				),
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
			let id = response.id;
			// Seek: the full-width track along the top of the bar.
			let seek_rect = egui::Rect::from_min_max(
				egui::pos2(bar.left() + 12.0, bar.top() + 6.0),
				egui::pos2(bar.right() - 12.0, bar.top() + 22.0),
			);
			let seek = track(
				ui,
				seek_rect,
				id.with("seek"),
				(self.position.max(0.0), duration),
				5.0,
				can_seek,
				colors.accent,
				&crate::i18n::translate("video-show-player-seek-video"),
			);
			controls_focused |= seek.response.has_focus();
			if let Some(pointer) = seek.response.hover_pos().filter(|_| can_seek) {
				let at = f64::from((pointer.x - seek_rect.left()) / seek_rect.width())
					.clamp(0.0, 1.0)
					* duration;
				let galley = ui.painter().layout_no_wrap(
					timestamp(seek.preview.unwrap_or(at)),
					egui::FontId::proportional(11.0),
					white,
				);
				// Keep the tip inside the stage; one wider than a narrow stage centres on it.
				let half = galley.size().x / 2.0 + 10.0;
				let x = if stage.width() > 2.0 * half {
					pointer.x.clamp(stage.left() + half, stage.right() - half)
				} else {
					stage.center().x
				};
				let tip = egui::Rect::from_center_size(
					egui::pos2(x, seek_rect.top() - galley.size().y / 2.0 - 6.0),
					galley.size() + egui::vec2(12.0, 6.0),
				);
				ui.painter()
					.rect_filled(tip, 4, egui::Color32::from_black_alpha(200));
				ui.painter()
					.galley(tip.min + egui::vec2(6.0, 3.0), galley, white);
			}
			if let Some(position) = seek.commit
				&& !context_click
			{
				self.command = Some(VideoCommand::Seek(position));
				self.position = position;
			}
			response |= seek.response;
			// One row of round buttons below the track.
			let row = bar.top() + 38.0;
			let slot =
				|x: f32| egui::Rect::from_center_size(egui::pos2(x, row), egui::Vec2::splat(32.0));
			let play_rect = slot(bar.left() + 22.0);
			let play = round_button(
				ui,
				play_rect,
				id.with("toggle"),
				&label,
				colors.accent,
				|painter, rect, color| match state {
					VideoState::Playing => glyph_pause(painter, rect, color),
					VideoState::Paused | VideoState::Idle => glyph_play(painter, rect, color),
					VideoState::Loading => {
						crate::icons::paint(painter, crate::icons::Icon::Close, rect, color)
					}
					VideoState::Ended | VideoState::Failed(_) => {
						crate::icons::paint(painter, crate::icons::Icon::Reload, rect, color)
					}
				},
			);
			controls_focused |= play.has_focus();
			if play.clicked() {
				self.toggle(message, attachment, state);
			}
			response |= play;
			let muted = self.volume <= 0.0;
			let mute_label = crate::i18n::translate(if muted {
				"video-show-player-unmute"
			} else {
				"video-show-player-mute"
			});
			let mute_rect = slot(play_rect.right() + 20.0);
			let mute = round_button(
				ui,
				mute_rect,
				id.with("mute"),
				&mute_label,
				colors.accent,
				|painter, rect, color| {
					crate::icons::paint(painter, crate::icons::Icon::Speaker, rect, color);
					if muted {
						painter.line_segment(
							[
								rect.left_top() + egui::vec2(1.0, 1.0),
								rect.right_bottom() - egui::vec2(1.0, 1.0),
							],
							egui::Stroke::new(2.0, color),
						);
					}
				},
			);
			controls_focused |= mute.has_focus();
			if mute.clicked() && !context_click {
				self.volume = if muted {
					self.unmuted_volume
						.filter(|volume| *volume > 0.0)
						.unwrap_or(1.0)
				} else {
					self.unmuted_volume = Some(self.volume);
					0.0
				};
				self.command = Some(VideoCommand::Volume(self.volume));
			}
			response |= mute;
			let volume_width = (stage.width() * 0.16).clamp(40.0, 80.0);
			let volume_rect = egui::Rect::from_min_size(
				egui::pos2(mute_rect.right() + 2.0, row - 8.0),
				egui::vec2(volume_width, 16.0),
			);
			let volume = track(
				ui,
				volume_rect,
				id.with("volume"),
				(f64::from(self.volume), 1.0),
				0.05,
				true,
				white,
				"Video volume",
			);
			controls_focused |= volume.response.has_focus();
			if let Some(value) = volume.preview.or(volume.commit)
				&& !context_click
			{
				self.volume = value as f32;
				self.command = Some(VideoCommand::Volume(self.volume));
			}
			response |= volume
				.response
				.on_hover_text(crate::i18n::translate("video-show-player-video-volume"));
			let speed_rect = egui::Rect::from_min_size(
				egui::pos2(volume_rect.right() + 8.0, row - 9.0),
				egui::vec2(38.0, 18.0),
			);
			let speed = ui
				.put(
					speed_rect,
					egui::Button::new(
						egui::RichText::new(crate::media::rate_label(self.rate))
							.size(11.0)
							.color(white),
					)
					.frame(false)
					.small(),
				)
				.on_hover_text(format!(
					"{}: {}",
					crate::i18n::translate("video-show-player-playback-speed"),
					crate::media::rate_label(self.rate)
				));
			speed.widget_info(|| {
				egui::WidgetInfo::labeled(
					egui::Role::Button,
					ui.is_enabled(),
					format!(
						"{}: {}",
						crate::i18n::translate("video-show-player-playback-speed"),
						crate::media::rate_label(self.rate)
					),
				)
			});
			controls_focused |= speed.has_focus();
			if crate::audio::playback_speed_menu(&speed, &mut self.rate) {
				self.command = Some(VideoCommand::Speed(self.rate));
			}
			response |= speed;
			let screen_label = if fullscreen {
				"Exit fullscreen (Esc)"
			} else {
				"Fullscreen"
			};
			let screen_rect = slot(bar.right() - 22.0);
			let screen = round_button(
				ui,
				screen_rect,
				id.with("fullscreen"),
				screen_label,
				colors.accent,
				|painter, rect, color| {
					if fullscreen {
						glyph_exit_fullscreen(painter, rect, color);
					} else {
						crate::icons::paint(painter, crate::icons::Icon::Fullscreen, rect, color);
					}
				},
			);
			controls_focused |= screen.has_focus();
			let screen_id = screen.id;
			if screen.clicked() {
				if fullscreen {
					self.exit_fullscreen();
				} else {
					let previous = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
					self.fullscreen = Some((ui.ctx().clone(), previous, screen_id));
					self.fullscreen_request = Some(true);
				}
			}
			response |= screen;
			// Elapsed and total time between the volume and fullscreen controls; the total
			// is dropped first when the bar is narrow.
			let position = seek.preview.unwrap_or(self.position);
			let total = if self.duration > 0.0 {
				timestamp(self.duration)
			} else {
				"--:--".into()
			};
			let room = screen_rect.left() - speed_rect.right() - 20.0;
			let font = egui::FontId::proportional(12.0);
			let full = ui.painter().layout_no_wrap(
				format!("{} / {total}", timestamp(position)),
				font.clone(),
				white,
			);
			let galley = if full.size().x <= room {
				full
			} else {
				ui.painter()
					.layout_no_wrap(timestamp(position), font, white)
			};
			if galley.size().x <= room {
				ui.painter().galley(
					egui::pos2(speed_rect.right() + 12.0, row - galley.size().y / 2.0),
					galley,
					white,
				);
			}
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
				if ui
					.add_enabled_ui(idle, |ui| {
						crate::attachments::glass_button(
							ui,
							crate::icons::Icon::Download,
							28.0,
							"Download",
						)
					})
					.inner
					.on_disabled_hover_text(crate::i18n::translate_if_key(if demo {
						"video-show-player-downloads-are-disabled-for-synthetic-attachments"
					} else {
						"video-show-player-a-download-is-already-active"
					}))
					.clicked()
				{
					download.request = Some(attachment.clone());
				}
				if let Some(url) = attachment
					.media
					.url
					.as_deref()
					.and_then(crate::markdown::external_url)
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
impl Drop for VideoUi {
	fn drop(&mut self) {
		self.exit_fullscreen();
	}
}
fn stage_size(attachment: &Attachment, width: f32) -> egui::Vec2 {
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
/// A seek or volume track: thin and rounded, thicker with a knob while hovered, dragged or
/// focused. Dragging previews; release, a click or an arrow key commits.
struct Track {
	response: egui::Response,
	preview: Option<f64>,
	commit: Option<f64>,
}

#[allow(clippy::too_many_arguments)]
fn track(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	id: egui::Id,
	(value, max): (f64, f64),
	step: f64,
	enabled: bool,
	fill: egui::Color32,
	label: &str,
) -> Track {
	let response = ui.interact(
		rect,
		id,
		if enabled {
			egui::Sense::click_and_drag()
		} else {
			egui::Sense::hover()
		},
	);
	response.widget_info(|| egui::WidgetInfo::slider(enabled, value, label));
	ui.ctx().accesskit_node_builder(id, |node| {
		use egui::accesskit::Action;
		node.set_min_numeric_value(0.0);
		node.set_max_numeric_value(max);
		node.set_numeric_value_step(step);
		if enabled {
			node.add_action(Action::SetValue);
			if value < max {
				node.add_action(Action::Increment);
			}
			if value > 0.0 {
				node.add_action(Action::Decrement);
			}
		}
	});
	// A stage narrower than the bar's insets leaves no track; never divide by its width.
	let at = |x: f32| f64::from((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0) * max;
	let mut preview = None;
	let mut commit = None;
	if enabled {
		if response.has_focus() {
			ui.memory_mut(|memory| {
				memory.set_focus_lock_filter(
					id,
					egui::EventFilter {
						horizontal_arrows: true,
						..Default::default()
					},
				)
			});
		}
		// Arrow keys and a screen reader's increment, decrement or set-value requests, as
		// `egui::Slider` handles them.
		let focused = response.has_focus();
		let (delta, set) = ui.input(|i| {
			use egui::accesskit::{Action, ActionData};
			let keys = if focused {
				f64::from(i8::from(i.key_pressed(egui::Key::ArrowRight)))
					- f64::from(i8::from(i.key_pressed(egui::Key::ArrowLeft)))
			} else {
				0.0
			};
			let requested = i.num_accesskit_action_requests(id, Action::Increment) as f64
				- i.num_accesskit_action_requests(id, Action::Decrement) as f64;
			let set = i
				.accesskit_action_requests(id, Action::SetValue)
				.filter_map(|request| match request.data {
					Some(ActionData::NumericValue(value)) => Some(value),
					_ => None,
				})
				.last();
			(keys + requested, set)
		});
		if let Some(value) = set.filter(|value| value.is_finite()) {
			commit = Some(value.clamp(0.0, max));
		} else if delta != 0.0 {
			commit = Some((value + delta * step).clamp(0.0, max));
		}
		let pointer = response
			.interact_pointer_pos()
			.or_else(|| ui.input(|i| i.pointer.interact_pos()));
		if let Some(pointer) = pointer {
			if response.is_pointer_button_down_on() || response.dragged() {
				preview = Some(at(pointer.x));
			}
			if response.drag_stopped() || response.clicked() {
				commit = Some(at(pointer.x));
			}
		}
	}
	let hot = enabled && (response.hovered() || response.dragged() || response.has_focus());
	let shown = preview.or(commit).unwrap_or(value).clamp(0.0, max);
	let thickness = if hot { 6.0 } else { 4.0 };
	let line = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), thickness));
	let radius = (thickness / 2.0) as u8;
	let painter = ui.painter();
	painter.rect_filled(
		line,
		radius,
		egui::Color32::from_white_alpha(if enabled { 64 } else { 32 }),
	);
	let knob = line.left() + line.width() * (shown / max.max(f64::EPSILON)) as f32;
	painter.rect_filled(
		egui::Rect::from_min_max(line.min, egui::pos2(knob, line.bottom())),
		radius,
		if enabled {
			fill
		} else {
			fill.gamma_multiply(0.5)
		},
	);
	if hot {
		painter.circle_filled(egui::pos2(knob, line.center().y), 6.0, egui::Color32::WHITE);
	}
	if response.has_focus() {
		painter.rect_stroke(
			rect.expand(2.0),
			4,
			egui::Stroke::new(2.0, fill),
			egui::StrokeKind::Outside,
		);
	}
	if enabled && response.hovered() {
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
	}

	Track {
		response,
		preview,
		commit,
	}
}

/// A round overlay button: a soft disc on hover and an accent ring when focused.
fn round_button(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	id: egui::Id,
	label: &str,
	focus: egui::Color32,
	glyph: impl FnOnce(&egui::Painter, egui::Rect, egui::Color32),
) -> egui::Response {
	let response = ui.interact(rect, id, egui::Sense::click());
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
	let hot = response.hovered() || response.has_focus();
	let painter = ui.painter();
	if hot {
		painter.circle_filled(
			rect.center(),
			rect.width() / 2.0,
			egui::Color32::from_white_alpha(36),
		);
	}
	if response.has_focus() {
		painter.circle_stroke(
			rect.center(),
			rect.width() / 2.0,
			egui::Stroke::new(2.0, focus),
		);
	}
	glyph(
		painter,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(18.0)),
		if hot {
			egui::Color32::WHITE
		} else {
			egui::Color32::from_white_alpha(225)
		},
	);
	response
		.on_hover_text(label)
		.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn glyph_play(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
	let (left, right) = (
		rect.left() + rect.width() * 0.2,
		rect.right() - rect.width() * 0.05,
	);
	painter.add(egui::Shape::convex_polygon(
		vec![
			egui::pos2(left, rect.top() + 1.0),
			egui::pos2(right, rect.center().y),
			egui::pos2(left, rect.bottom() - 1.0),
		],
		color,
		egui::Stroke::new(1.5, color),
	));
}

fn glyph_pause(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
	for x in [-0.22, 0.22] {
		painter.rect_filled(
			egui::Rect::from_center_size(
				rect.center() + egui::vec2(rect.width() * x, 0.0),
				egui::vec2(rect.width() * 0.22, rect.height() * 0.85),
			),
			1,
			color,
		);
	}
}

/// Four corners turned inward, the reverse of the fullscreen icon.
fn glyph_exit_fullscreen(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
	let stroke = egui::Stroke::new(2.0, color);
	let arm = rect.width() * 0.3;
	let inset = rect.shrink(rect.width() * 0.1);
	for (corner, sx, sy) in [
		(inset.left_top(), 1.0, 1.0),
		(inset.right_top(), -1.0, 1.0),
		(inset.left_bottom(), 1.0, -1.0),
		(inset.right_bottom(), -1.0, -1.0),
	] {
		let elbow = corner + egui::vec2(sx * arm, sy * arm);
		painter.line_segment([egui::pos2(corner.x, elbow.y), elbow], stroke);
		painter.line_segment([egui::pos2(elbow.x, corner.y), elbow], stroke);
	}
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
							crate::design::MessageCardSurface::Conversation,
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
			assert!(!video.accept_frame(&ctx, 3841, 1080, &[]));
			assert!(!video.accept_frame(&ctx, 1, 3841, &[]));
			assert!(!video.accept_frame(&ctx, 3840, 3840, &[]));
			assert!(!video.accept_frame(&ctx, usize::MAX, usize::MAX, &[]));
			assert!(!video.accept_frame(&ctx, 1, 1, &[0]));
			// Within the pixel bound, but taller than this renderer's largest texture.
			assert!(!video.accept_frame(&ctx, 1, 3840, &vec![0; 3840 * 4]));
			assert!(video.accept_frame(&ctx, 1, 2048, &vec![0; 2048 * 4]));
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
			// Seek, then play/pause and mute, then the volume track.
			for key in [
				egui::Key::Tab,
				egui::Key::Tab,
				egui::Key::Tab,
				egui::Key::ArrowLeft,
			] {
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
	/// Idle stages queue one poster, cache the decoded answer, and evict oldest-first.
	#[test]
	fn idle_stage_requests_and_paints_one_decoded_poster() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let message = test_support::message(1, Id(2));
		let attachment = Attachment {
			id: Id(3),
			filename: "clip.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 128,
			media: model::EmbedMedia {
				width: 720,
				height: 1280,
				..Default::default()
			},
			spoiler: false,
			duration_ms: Some(11_817),
			waveform: Vec::new(),
		};
		let mut video = VideoUi::default();
		let show = |video: &mut VideoUi| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(436.0, 600.0),
					)),
					..Default::default()
				},
				|ui| {
					ui.set_width(420.0);
					video.show(
						ui,
						&message,
						&attachment,
						&mut crate::attachments::DownloadUi::default(),
						&mut None,
						false,
					);
				},
			)
			.drop_without_applying_deltas();
		};
		show(&mut video);
		// One queued poster request, still idle: playback never starts on its own,
		// and the playback command slot stays free for clicks.
		assert_eq!(video.poster_queue.len(), 1);
		assert_eq!(video.poster_queue[0], attachment);
		assert!(video.command.is_none() && video.active.is_none());
		// Draining (as the desktop does each frame) retries while unresolved;
		// the desktop pending set absorbs the duplicates.
		video.poster_queue.clear();
		show(&mut video);
		assert_eq!(video.poster_queue.len(), 1);
		// Bounds match the playback path; failures never cache a texture.
		assert!(!video.accept_poster(&ctx, Id(3), 0, 1, &[]));
		assert!(!video.accept_poster(&ctx, Id(3), 1921, 1, &[]));
		assert!(!video.accept_poster(&ctx, Id(3), 1, 1, &[0]));
		assert!(video.posters.is_empty());
		video.poster_failed(Id(9));
		assert!(video.posters.is_empty());
		// The decoded answer is cached and painted without further requests.
		assert!(video.accept_poster(&ctx, Id(3), 2, 1, &[9, 9, 9, 255, 1, 1, 1, 255]));
		assert_eq!(video.posters.len(), 1);
		assert_eq!(video.poster_bytes, 8);
		show(&mut video);
		assert!(video.command.is_none());
		assert_eq!(video.posters.len(), 1);
		// Budget eviction drops the oldest texture and its retry guard together.
		let big = vec![7; 1920 * 1080 * 4];
		assert!(video.accept_poster(&ctx, Id(4), 1920, 1080, &big));
		assert_eq!(video.posters.len(), 2);
		assert!(video.accept_poster(&ctx, Id(5), 1920, 1080, &big));
		assert_eq!(video.posters.len(), 1);
		assert!(!video.posters.contains_key(&Id(3)));
		assert!(!video.posters.contains_key(&Id(4)));
		assert!(!video.poster_seen.contains(&Id(3)));
		assert!(video.poster_bytes <= POSTER_BUDGET);
	}
}
