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
	Stop,
}
/// One explicitly selected provider player; discarded with its message/session.
pub struct ProviderPlayback {
	pub generation: u64,
	pub channel: Id,
	pub message: Id,
	pub target: model::YouTubeVideo,
	pub source: String,
	pub bounds: Option<egui::Rect>,
	pub clip: Option<egui::Rect>,
	pub expanded: bool,
	pub state: VideoState,
	pub start_requested: bool,
	pub error: Option<&'static str>,
	pub standalone: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Origin {
	#[default]
	Timeline,
	Search,
}
pub struct VideoUi {
	pub(super) origin: Origin,
	pub(super) rendering: Origin,
	pub active: Option<(Id, Id, Attachment)>,
	pub provider: Option<ProviderPlayback>,
	pub state: VideoState,
	pub position: f64,
	pub duration: f64,
	pub command: Option<VideoCommand>,
	pub seen: bool,
	visibility_paused: bool,
	pub volume: f32,
	/// Latest frame; usually the desktop's GPU-converted texture, else [`Self::managed`].
	texture: Option<egui::load::SizedTexture>,
	/// CPU fallback texture behind [`VideoUi::accept_frame`].
	managed: Option<egui::TextureHandle>,
	pub media: Option<MediaView>,
	/// Staging pixels for the current frame. The renderer drops its reference after the
	/// upload, so the same allocation is refilled each frame instead of reallocating up to
	/// 8 MB per frame (1080p at 60 fps churned ~500 MB/s through the allocator).
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
	/// Keep the viewport's previous mode so leaving playback restores the window.
	fullscreen: Option<(egui::Context, bool, egui::Id)>,
	/// Native window transition for the desktop to apply after this UI frame.
	pub(super) fullscreen_request: Option<bool>,
}
impl Default for VideoUi {
	fn default() -> Self {
		Self {
			origin: Origin::Timeline,
			rendering: Origin::Timeline,
			active: None,
			provider: None,
			state: VideoState::Idle,
			position: 0.0,
			duration: 0.0,
			command: None,
			seen: false,
			visibility_paused: false,
			volume: 1.0,
			texture: None,
			media: None,
			managed: None,
			frame: None,
			shade: None,
			posters: HashMap::new(),
			poster_order: VecDeque::new(),
			poster_bytes: 0,
			poster_seen: HashSet::new(),
			poster_queue: Vec::new(),
			controls_focused: false,
			fullscreen: None,
			fullscreen_request: None,
		}
	}
}
impl VideoUi {
	pub fn stop(&mut self) {
		self.exit_fullscreen();
		self.active = None;
		self.origin = Origin::Timeline;
		self.provider = None;
		self.texture = None;
		self.media = None;
		self.managed = None;
		self.frame = None;
		self.state = VideoState::Idle;
		self.position = 0.0;
		self.duration = 0.0;
		self.seen = false;
		self.visibility_paused = false;
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
	pub(crate) fn open_provider(
		&mut self,
		message: &Message,
		target: model::YouTubeVideo,
		source: String,
		generation: u64,
	) {
		self.stop();
		self.origin = self.rendering;
		self.provider = Some(ProviderPlayback {
			generation,
			channel: message.channel,
			message: message.id,
			target,
			source,
			bounds: None,
			clip: None,
			expanded: false,
			state: VideoState::Loading,
			start_requested: true,
			error: None,
			standalone: false,
		});
		self.seen = true;
	}
	pub(crate) fn provider_matches(&self, message: &Message, target: &model::YouTubeVideo) -> bool {
		self.provider.as_ref().is_some_and(|p| {
			p.message == message.id && p.channel == message.channel && &p.target == target
		})
	}
	pub(crate) fn show_provider_card(
		&mut self,
		ui: &mut egui::Ui,
		dimensions: (u32, u32),
		opening: &mut Option<String>,
	) {
		let Some(player) = &mut self.provider else {
			return;
		};
		if player.expanded {
			return;
		}
		let (width, height) = dimensions;
		let w = ui.available_width().clamp(1.0, MAX_WIDTH);
		let ratio = if width > 0 && height > 0 {
			height as f32 / width as f32
		} else {
			9.0 / 16.0
		};
		let h = (w * ratio).min(MAX_HEIGHT);
		let w = h / ratio;
		let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
		Self::provider_stage(ui, player, rect);
		self.seen |= ui.is_rect_visible(rect);
		let mut close = false;
		ui.horizontal_wrapped(|ui| {
			if player.error.is_some()
				&& crate::design::button(
					ui,
					"video-provider-retry",
					crate::design::ButtonKind::Outline,
				)
				.clicked()
			{
				player.error = None;
				player.state = VideoState::Loading;
				player.start_requested = true;
			}
			if crate::design::button(
				ui,
				"video-provider-expand",
				crate::design::ButtonKind::Neutral,
			)
			.clicked()
			{
				player.expanded = true;
			}
			if crate::design::button(
				ui,
				"video-provider-open-original",
				crate::design::ButtonKind::Neutral,
			)
			.clicked()
			{
				*opening = crate::markdown::external_url(&player.source);
			}
			close = crate::design::button(
				ui,
				"video-provider-close",
				crate::design::ButtonKind::Neutral,
			)
			.clicked();
		});
		if close {
			self.stop();
		}
	}
	fn provider_stage(ui: &mut egui::Ui, player: &mut ProviderPlayback, rect: egui::Rect) {
		ui.painter().rect_filled(rect, 4, egui::Color32::BLACK);
		let clipped = rect.intersect(ui.clip_rect());
		if clipped.is_positive() {
			player.bounds = Some(rect);
			player.clip = Some(clipped);
		}
		let text = if let Some(error) = player.error {
			crate::i18n::translate_if_key(error)
		} else {
			crate::i18n::translate(if player.standalone {
				"video-provider-separate-window"
			} else {
				"video-provider-loading"
			})
		};
		let galley = ui.painter().layout(
			text,
			egui::FontId::proportional(14.0),
			egui::Color32::WHITE,
			(rect.width() - 24.0).max(1.0),
		);
		ui.painter().with_clip_rect(clipped).galley(
			rect.center() - galley.size() / 2.0,
			galley,
			egui::Color32::WHITE,
		);
	}
	/// An enlarged view is opened only by the explicit Expand action.
	pub(crate) fn show_provider(&mut self, ctx: &egui::Context, opening: &mut Option<String>) {
		let fullscreen = self.is_fullscreen();
		let Some(player) = &mut self.provider else {
			return;
		};
		if !player.expanded {
			return;
		}
		self.seen = true;
		let screen = ctx.content_rect();
		let mut inline = false;
		let mut toggle_fullscreen = None;
		let id = egui::Id::unique("provider-video");
		let modal = egui::Modal::new(id);
		let modal = if fullscreen {
			modal
				.area(
					egui::Modal::default_area(id).anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO),
				)
				.backdrop_color(egui::Color32::BLACK)
				.frame(egui::Frame::NONE)
		} else {
			modal
		};
		let response = modal.show(ctx, |ui| {
			let width = if fullscreen {
				ui.set_min_size(screen.size());
				screen.width()
			} else {
				(screen.width() - 64.0).clamp(1.0, 960.0)
			};
			ui.set_width(width);
			ui.horizontal_wrapped(|ui| {
				inline = crate::design::button(
					ui,
					"video-provider-return-inline",
					crate::design::ButtonKind::Outline,
				)
				.clicked();
				if !player.standalone {
					let button = crate::design::button(
						ui,
						if fullscreen {
							"video-provider-exit-fullscreen"
						} else {
							"video-provider-fullscreen"
						},
						crate::design::ButtonKind::Outline,
					);
					if button.clicked() {
						toggle_fullscreen = Some(button.id);
					}
				}
			});
			let height = (width * 9.0 / 16.0).min((screen.height() - 120.0).max(1.0));
			let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
			Self::provider_stage(ui, player, rect);
			if player.error.is_some()
				&& crate::design::button(
					ui,
					"video-provider-retry",
					crate::design::ButtonKind::Outline,
				)
				.clicked()
			{
				player.error = None;
				player.state = VideoState::Loading;
				player.start_requested = true;
			}
			if crate::design::button(
				ui,
				"video-provider-open-original",
				crate::design::ButtonKind::Neutral,
			)
			.clicked()
			{
				*opening = crate::markdown::external_url(&player.source);
			}
		});
		if inline || response.should_close() || opening.is_some() {
			player.expanded = false;
			player.bounds = None;
			player.clip = None;
			self.exit_fullscreen();
		} else if let Some(focus) = toggle_fullscreen {
			if fullscreen {
				self.exit_fullscreen();
			} else {
				let previous = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
				self.fullscreen = Some((ctx.clone(), previous, focus));
				self.fullscreen_request = Some(true);
			}
		}
	}
	/// A viewport pause preserves a manual pause and resumes only an interrupted playing clip.
	pub fn update_visibility(&mut self) {
		if self.active.is_none() || self.command.is_some() {
			return;
		}
		if !self.seen
			&& !self.visibility_paused
			&& matches!(self.state, VideoState::Playing | VideoState::Loading)
		{
			self.visibility_paused = true;
			self.command = Some(VideoCommand::Pause(true));
		} else if self.seen && self.visibility_paused {
			self.visibility_paused = false;
			if !matches!(self.state, VideoState::Ended | VideoState::Failed(_)) {
				self.command = Some(VideoCommand::Pause(false));
			}
		}
	}
	pub(super) fn is_fullscreen(&self) -> bool {
		self.fullscreen.is_some()
	}
	#[allow(clippy::too_many_arguments)]
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
		self.media = None;
		if self.active.is_none()
			|| width == 0
			|| height == 0
			|| width > 7680
			|| height > 7680
			|| width * height > 7680 * 4320
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
		self.media = None;
		self.managed = None;
		self.frame = None;
		self.texture = Some(texture);
		true
	}
	/// Switch to the HDR-capable media renderer without retaining an obsolete SDR frame.
	pub fn show_media(&mut self, media: MediaView) -> bool {
		if self.active.is_none() {
			return false;
		}
		self.texture = None;
		self.managed = None;
		self.frame = None;
		self.media = Some(media);
		true
	}
	fn toggle(&mut self, message: &Message, attachment: &Attachment, state: VideoState) {
		self.visibility_paused = false;
		self.command = Some(match state {
			VideoState::Loading => {
				self.stop();
				VideoCommand::Stop
			}
			VideoState::Playing => VideoCommand::Pause(true),
			VideoState::Paused => VideoCommand::Pause(false),
			_ => {
				self.origin = self.rendering;
				self.provider = None;
				self.active = Some((message.channel, message.id, attachment.clone()));
				self.texture = None;
				self.media = None;
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
			|| width > 1920
			|| height > 1920
			|| width * height > 1920 * 1080
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
		let label = crate::i18n::translate_if_key(match state {
			VideoState::Loading => "video-show-player-cancel",
			VideoState::Playing => "video-show-player-pause",
			VideoState::Paused => "video-show-player-resume",
			VideoState::Ended => "video-show-player-replay",
			VideoState::Failed(_) => "video-show-player-retry",
			VideoState::Idle => "video-show-player-play",
		});
		let painter = ui.painter().with_clip_rect(stage);
		if !poster || (active && (self.texture.is_some() || self.media.is_some())) {
			painter.rect_filled(stage, CORNER, egui::Color32::BLACK);
		}
		if !poster && (!active || (self.texture.is_none() && self.media.is_none())) {
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
		if let Some(media) = self.media.as_ref().filter(|_| active) {
			let scale = (stage.width() / media.size.x).min(stage.height() / media.size.y);
			media.paint(
				ui,
				egui::Rect::from_center_size(stage.center(), media.size * scale),
			);
		} else if let Some(texture) = self.texture.filter(|_| active) {
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
					seek.widget_info(|| {
						egui::WidgetInfo::slider(
							can_seek,
							position,
							crate::i18n::translate("video-show-player-seek-video"),
						)
					});
					controls_focused |= seek.has_focus();
					response |= seek.clone();
					if seek
						.on_hover_text(crate::i18n::translate("video-show-player-seek-video"))
						.changed() && !context_click
					{
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
								format!(
									"{} (Esc)",
									crate::i18n::translate("video-provider-exit-fullscreen")
								)
							} else {
								crate::i18n::translate("video-provider-fullscreen")
							};
							button.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									ui.is_enabled(),
									&label,
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
								crate::icons::paint(
									ui.painter(),
									crate::icons::Icon::Fullscreen,
									rect.shrink(1.0),
									white,
								);
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
								.on_hover_text(&label)
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
									crate::i18n::translate("video-show-player-video-volume"),
								)
							});
							controls_focused |= volume.has_focus();
							response |= volume.clone();
							if volume
								.on_hover_text(crate::i18n::translate(
									"video-show-player-video-volume",
								))
								.changed() && !context_click
							{
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
								&crate::i18n::translate("attachments-download-button-download"),
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
				if let Some(url) = original.and_then(crate::markdown::external_url)
					&& crate::attachments::glass_button(
						ui,
						crate::icons::Icon::External,
						28.0,
						&crate::i18n::translate("attachments-media-menu-open-original"),
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
pub(crate) fn is_embedded(attachment: &Attachment) -> bool {
	attachment.size == 0
}
/// Prefer Discord's media proxy, with known public video origins as a fallback.
/// Extensionless offload/proxy endpoints still require the decoder to validate the container.
/// Shared with the desktop worker so a visible Play action has the same URL policy as playback.
pub fn embedded_video_url(media: &model::EmbedMedia) -> Option<url::Url> {
	embedded_video_candidates(media).into_iter().next()
}
/// At most one validated proxy and original, in playback order. Keep signatures opaque.
pub fn embedded_video_candidates(media: &model::EmbedMedia) -> Vec<url::Url> {
	[media.proxy_url.as_deref(), media.url.as_deref()]
		.into_iter()
		.flatten()
		.filter_map(embedded_video_candidate)
		.fold(Vec::with_capacity(2), |mut urls, url| {
			if !urls.contains(&url) {
				urls.push(url);
			}
			urls
		})
}
pub(crate) fn embedded_video_candidate(raw: &str) -> Option<url::Url> {
	if raw.len() > 2048 || raw.contains('\\') || raw.chars().any(char::is_control) {
		return None;
	}
	let mut url = url::Url::parse(raw).ok()?;
	if url.host_str() == Some("media.discordapp.net") && url.path().starts_with("/attachments/") {
		url.set_host(Some("cdn.discordapp.com")).ok()?;
	}
	(url.scheme() == "https"
		&& url.port_or_known_default() == Some(443)
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.fragment().is_none()
		&& embedded_video_origin(&url))
	.then_some(url)
}
fn embedded_video_origin(url: &url::Url) -> bool {
	let Some(host) = url.host_str() else {
		return false;
	};
	match host {
		"cdn.discordapp.com" | "media.discordapp.net"
			if url.path().starts_with("/attachments/") =>
		{
			let parts: Vec<_> = url.path().split('/').skip(1).collect();
			let valid_path = match parts.as_slice() {
				["attachments", channel, id, file] => {
					channel.parse::<Id>().is_ok()
						&& id.parse::<Id>().is_ok()
						&& video_filename(file)
				}
				["attachments", channel, message, id, file] => {
					[channel, message, id]
						.iter()
						.all(|value| value.parse::<Id>().is_ok())
						&& video_filename(file)
				}
				_ => false,
			};
			valid_path
				&& url.query_pairs().all(|(key, value)| match key.as_ref() {
					"ex" | "is" | "hm" => true,
					"backend" => matches!(value.as_ref(), "b2" | "b3"),
					_ => false,
				})
		}
		"media.discordapp.net" | "images-ext-1.discordapp.net" | "images-ext-2.discordapp.net" => {
			url.path().starts_with("/external/")
		}
		"video.twimg.com" => {
			["/ext_tw_video/", "/amplify_video/", "/tweet_video/"]
				.into_iter()
				.any(|prefix| url.path().starts_with(prefix))
				&& url.path().ends_with(".mp4")
		}
		"oginstagram.com" | "www.oginstagram.com" => url.path().starts_with("/offload/"),
		"v.redd.it" => video_filename(url.path()),
		_ => {
			// The offload endpoint can redirect to Instagram's public video CDN.
			(host == "cdninstagram.com"
				|| host.ends_with(".cdninstagram.com")
				|| host == "fbcdn.net"
				|| host.ends_with(".fbcdn.net"))
				&& url.path().ends_with(".mp4")
		}
	}
}
fn video_filename(name: &str) -> bool {
	let name = name.to_ascii_lowercase();
	!name.contains('\\')
		&& !["%2f", "%5c", "%00"]
			.iter()
			.any(|escape| name.contains(escape))
		&& [".mp4", ".mov", ".m4v", ".webm", ".mkv"]
			.iter()
			.any(|ext| name.ends_with(ext))
}
pub(crate) fn embed_stage_height(width: u32, height: u32, available: f32) -> f32 {
	let known = width > 0 && height > 0;
	let (w, h) = if known {
		(width as f32, height as f32)
	} else {
		(16.0, 9.0)
	};
	let available = available.clamp(1.0, MAX_WIDTH);
	h * (available / w)
		.min(MAX_HEIGHT / h)
		.min(if known { 1.0 } else { f32::INFINITY })
}
pub(crate) fn stage_size(attachment: &Attachment, width: f32) -> egui::Vec2 {
	let width = width.clamp(1.0, MAX_WIDTH);
	if attachment.media.width == 0 || attachment.media.height == 0 {
		// 16:9 is an aspect ratio, not the pixel dimensions of the stage.
		return egui::vec2(width, embed_stage_height(0, 0, width));
	}
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
	fn attachment_links_preserve_signatures_and_media_candidates() {
		for (host, ext) in [
			("cdn.discordapp.com", "mov"),
			("media.discordapp.net", "webm"),
			("cdn.discordapp.com", "MP4"),
		] {
			let original = format!(
				"https://{host}/attachments/885617127443284024/1561794092579422319/clip.{ext}?ex=6ad7a598&is=6ad65418&hm=synthetic"
			);
			let proxy = "https://media.discordapp.net/external/hash/https/example.org/clip.mov";
			let media = model::EmbedMedia {
				url: Some(original.clone()),
				proxy_url: Some(proxy.into()),
				..Default::default()
			};
			let urls = embedded_video_candidates(&media);
			assert_eq!(urls.len(), 2);
			assert_eq!(urls[0].as_str(), proxy);
			assert_eq!(
				urls[1].as_str(),
				original.replace("media.discordapp.net", "cdn.discordapp.com")
			);
		}
		for path in [
			"attachments/x/2/file.mov",
			"attachments/1/2/file.txt",
			"attachments/1/2/clip%2f.mov",
			"attachments/1/2/clip.mov/extra",
		] {
			assert!(
				embedded_video_url(&model::EmbedMedia {
					url: Some(format!("https://cdn.discordapp.com/{path}")),
					..Default::default()
				})
				.is_none()
			);
		}
	}
	#[test]
	fn offscreen_pause_preserves_position_and_manual_pause() {
		let message = test_support::message(1, Id(2));
		let attachment = crate::embeds::native_video(&model::Embed {
			url: Some("https://cdn.discordapp.com/attachments/1/2/clip.mov".into()),
			..Default::default()
		})
		.unwrap();
		let mut video = VideoUi::default();
		video.active = Some((message.channel, message.id, attachment));
		video.state = VideoState::Playing;
		video.position = 1.25;
		video.update_visibility();
		assert!(matches!(
			video.command.take(),
			Some(VideoCommand::Pause(true))
		));
		video.state = VideoState::Paused;
		video.update_visibility();
		assert!(video.command.is_none());
		video.seen = true;
		video.update_visibility();
		assert!(matches!(
			video.command.take(),
			Some(VideoCommand::Pause(false))
		));
		assert_eq!(video.position, 1.25);
		video.state = VideoState::Paused;
		video.seen = false;
		video.update_visibility();
		video.seen = true;
		video.update_visibility();
		assert!(
			video.command.is_none(),
			"manual pause must survive scrolling"
		);
	}

	#[test]
	fn provider_card_is_inline_and_reports_the_scroll_clip_without_resizing_content() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let message = test_support::message(1, Id(2));
		let target = model::YouTubeVideo::new("synthet1c01", 0).unwrap();
		let mut video = VideoUi::default();
		video.open_provider(&message, target, "https://youtu.be/synthet1c01".into(), 1);
		for (width, top) in [(480.0, 80.0), (240.0, 0.0)] {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 700.0),
					)),
					..Default::default()
				},
				|ui| {
					ui.set_clip_rect(egui::Rect::from_min_max(
						egui::pos2(0.0, top),
						egui::pos2(width, 700.0),
					));
					video.show_provider_card(ui, (1280, 720), &mut None);
				},
			)
			.drop_without_applying_deltas();
			let player = video.provider.as_ref().unwrap();
			let full = player.bounds.unwrap();
			let clip = player.clip.unwrap();
			assert!((full.width() / full.height() - 16.0 / 9.0).abs() < 0.001);
			assert!(clip.top() >= top && clip.width() <= full.width());
			assert!(!player.expanded);
			assert!(ctx.memory(|m| m.top_modal_layer().is_none()));
		}
	}

	#[test]
	fn provider_fullscreen_requires_an_action_and_escape_restores_inline() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let message = test_support::message(1, Id(2));
		let mut video = VideoUi::default();
		video.open_provider(
			&message,
			model::YouTubeVideo::new("synthet1c01", 0).unwrap(),
			"https://youtu.be/synthet1c01".into(),
			1,
		);
		assert!(!video.is_fullscreen());
		assert!(video.take_fullscreen_request().is_none());
		video.provider.as_mut().unwrap().expanded = true;
		let mut frame = |events| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 800.0),
					)),
					events,
					..Default::default()
				},
				|_| video.show_provider(&ctx, &mut None),
			)
		};
		frame(vec![]).drop_without_applying_deltas();
		frame(vec![]).drop_without_applying_deltas();
		let output = frame(vec![]);
		let label = crate::i18n::translate("video-provider-fullscreen");
		let button = output
			.shapes
			.iter()
			.find_map(|shape| match &shape.shape {
				egui::Shape::Text(text) if text.galley.job.text == label => {
					Some(text.pos + text.galley.rect.center().to_vec2())
				}
				_ => None,
			})
			.expect("explicit fullscreen action");
		output.drop_without_applying_deltas();
		for pressed in [true, false] {
			frame(vec![
				egui::Event::PointerMoved(button),
				egui::Event::PointerButton {
					pos: button,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			])
			.drop_without_applying_deltas();
		}
		assert!(video.is_fullscreen());
		assert_eq!(video.take_fullscreen_request(), Some(true));
		ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1000.0, 800.0),
				)),
				events: vec![egui::Event::Key {
					key: egui::Key::Escape,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}],
				..Default::default()
			},
			|_| video.show_provider(&ctx, &mut None),
		)
		.drop_without_applying_deltas();
		assert!(!video.is_fullscreen());
		assert!(!video.provider.as_ref().unwrap().expanded);
		assert_eq!(video.take_fullscreen_request(), Some(false));
	}

	#[test]
	fn embedded_sources_use_only_media_proxies_or_known_public_video_origins() {
		for host in [
			"media.discordapp.net",
			"images-ext-1.discordapp.net",
			"images-ext-2.discordapp.net",
		] {
			for path in ["clip.MP4", "clip.webm", "clip.mkv", "offload/synthetic/1"] {
				let raw = format!(
					"https://{host}/external/synthetic-hash/https/example.org/{path}?v=2&sig=synthetic"
				);
				for proxy_field in [true, false] {
					let media = model::EmbedMedia {
						url: Some(if proxy_field {
							"https://example.org/original.mp4".into()
						} else {
							raw.clone()
						}),
						proxy_url: proxy_field.then(|| raw.clone()),
						..Default::default()
					};
					assert_eq!(embedded_video_url(&media).unwrap().as_str(), raw);
				}
			}
		}
		for raw in [
			"https://video.twimg.com/ext_tw_video/1/pu/vid/1280x720/clip.mp4?tag=12",
			"https://video.twimg.com/amplify_video/1/vid/clip.mp4",
			"https://video.twimg.com/tweet_video/clip.mp4",
			"https://www.oginstagram.com/offload/synthetic/1?sig=synthetic",
			"https://scontent.cdninstagram.com/video/clip.mp4?sig=synthetic",
			"https://video.xx.fbcdn.net/video/clip.mp4?sig=synthetic",
			"https://v.redd.it/synthetic/DASH_720.mp4?source=fallback&sig=one%2Ftwo",
		] {
			let media = model::EmbedMedia {
				url: Some(raw.into()),
				..Default::default()
			};
			assert_eq!(embedded_video_url(&media).unwrap().as_str(), raw);
			let proxy = "https://media.discordapp.net/external/hash/https/example.org/clip";
			assert_eq!(
				embedded_video_url(&model::EmbedMedia {
					proxy_url: Some(proxy.into()),
					..media
				})
				.unwrap()
				.as_str(),
				proxy,
				"prefer Discord's proxy when present"
			);
		}
		for raw in [
			"https://example.org/video.mp4",
			"https://www.youtube.com/embed/synthetic",
			"http://media.discordapp.net/external/h/clip.mp4",
			"https://media.discordapp.net.evil.test/external/h/clip.mp4",
			"https://user@media.discordapp.net/external/h/clip.mp4",
			"https://media.discordapp.net:444/external/h/clip.mp4",
			"https://media.discordapp.net/external/h/clip.mp4#fragment",
			"https://media.discordapp.net/api/video.mp4",
			"https://media.discordapp.net\\external\\h\\clip.mp4",
			"https://video.twimg.com.evil.test/ext_tw_video/clip.mp4",
			"https://video.twimg.com/ext_tw_video/playlist.m3u8",
			"https://www.oginstagram.com/reel/synthetic",
			"https://www.oginstagram.com.evil.test/offload/synthetic/1",
			"https://user@video.twimg.com/tweet_video/clip.mp4",
			"https://video.twimg.com:444/tweet_video/clip.mp4",
			"https://video.twimg.com/tweet_video/clip.mp4#fragment",
			"https://cdninstagram.com.evil.test/video/clip.mp4",
			"https://evilcdninstagram.com/video/clip.mp4",
			"https://video.xx.fbcdn.net/iframe.html",
			"https://v.redd.it/synthetic",
			"https://v.redd.it/synthetic/DASHPlaylist.mpd",
			"https://v.redd.it.evil.test/synthetic/DASH_720.mp4",
			"https://127.0.0.1/video.mp4",
		] {
			assert!(
				embedded_video_url(&model::EmbedMedia {
					url: Some(raw.into()),
					..Default::default()
				})
				.is_none(),
				"{raw}"
			);
		}
	}

	#[test]
	fn provider_controls_and_error_messages_are_available_in_every_language() {
		for language in crate::i18n::Language::ALL {
			for key in [
				"video-provider-play",
				"video-provider-close",
				"video-provider-loading",
				"video-provider-separate-window",
				"video-provider-retry",
				"video-provider-open-original",
				"video-provider-load-failed",
				"video-provider-expand",
				"video-provider-return-inline",
				"video-provider-fullscreen",
				"video-provider-exit-fullscreen",
				"video-provider-unavailable",
				"video-provider-embedding-disabled",
				"video-provider-identity-failed",
				"video-provider-buffering-failed",
				"video-provider-open-window",
			] {
				let text = language.text(key);
				assert!(
					!text.starts_with("Unknown localization key:"),
					"{language:?}: {key}"
				);
				if matches!(
					key,
					"video-provider-play" | "video-provider-identity-failed"
				) {
					assert!(text.contains("YouTube"), "{language:?}: {key}: {text}");
				}
			}
		}
	}

	#[test]
	fn video_overlay_accessible_controls_follow_the_display_language() {
		crate::i18n::set_current(crate::i18n::Language::Spanish);
		let ctx = egui::Context::default();
		ctx.enable_accesskit();
		crate::design::apply(&ctx);
		let message = test_support::message(1, Id(2));
		let attachment = Attachment {
			id: Id(3),
			filename: "synthetic.mp4".into(),
			description: None,
			content_type: Some("video/mp4".into()),
			size: 128,
			media: model::EmbedMedia {
				url: Some("https://cdn.discordapp.com/attachments/2/3/synthetic.mp4".into()),
				width: 640,
				height: 360,
				..Default::default()
			},
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		};
		let mut video = VideoUi::default();
		video.active = Some((message.channel, message.id, attachment.clone()));
		video.state = VideoState::Paused;
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(640.0, 480.0),
				)),
				focused: true,
				..Default::default()
			},
			|ui| {
				video.show(
					ui,
					&message,
					&attachment,
					&mut crate::DownloadUi::default(),
					&mut None,
					false,
				);
			},
		);
		let update = output.platform_output.accesskit_update.as_ref().unwrap();
		for key in [
			"attachments-download-button-download",
			"attachments-media-menu-open-original",
			"video-provider-fullscreen",
			"video-show-player-video-volume",
		] {
			let label = crate::i18n::Language::Spanish.text(key);
			assert!(!label.starts_with("Unknown localization key:"), "{key}");
			assert!(
				update
					.nodes
					.iter()
					.any(|(_, node)| node.label() == Some(label.as_str())),
				"{label}"
			);
		}
		output.drop_without_applying_deltas();
		crate::i18n::set_current(crate::i18n::Language::English);
	}

	#[test]
	fn missing_dimensions_keep_a_full_size_keyboard_operable_player() {
		let message = test_support::message(1, Id(2));
		for metadata in [(0, 0), (1920, 0), (0, 1080)] {
			let attachment = Attachment {
				id: Id(0),
				filename: "Video".into(),
				description: None,
				content_type: Some("video/mp4".into()),
				size: 0,
				media: model::EmbedMedia {
					width: metadata.0,
					height: metadata.1,
					..Default::default()
				},
				spoiler: false,
				duration_ms: None,
				waveform: vec![],
			};
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			let mut player = VideoUi::default();
			let mut frame = |events| {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(480.0, 600.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						let response = player.show_embedded(
							ui,
							&message,
							&attachment,
							None,
							false,
							&mut crate::DownloadUi::default(),
							&mut None,
							false,
						);
						assert!(response.rect.width() > 400.0 && response.rect.height() > 200.0);
					},
				);
				output.drop_without_applying_deltas();
			};
			frame(vec![]);
			for key in [egui::Key::Tab, egui::Key::Enter] {
				frame(vec![egui::Event::Key {
					key,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}]);
			}
			assert!(matches!(player.command, Some(VideoCommand::Play(_))));
			assert!(player.seen);
			assert!(
				(embed_stage_height(0, 0, 456.0) - stage_size(&attachment, 456.0).y).abs() < 0.01
			);
		}
	}

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
		// Cached posters must yield to active SDR textures and high-precision media.
		assert!(video.accept_poster(&ctx, Id(4), 1, 1, &[0, 0, 0, 255]));
		video.poster_queue.clear();
		video.active = Some((message.channel, message.id, attachment.clone()));
		video.state = VideoState::Playing;
		let painted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
		let callback_painted = painted.clone();
		video.media = Some(MediaView {
			size: egui::vec2(2.0, 1.0),
			resource: std::sync::Arc::new(()),
			failure: Default::default(),
			callback: std::sync::Arc::new(move |rect| {
				callback_painted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
				egui::PaintCallback {
					rect,
					callback: std::sync::Arc::new(()),
				}
			}),
		});
		show(&mut video);
		assert!(painted.load(std::sync::atomic::Ordering::Relaxed) > 0);
		assert_eq!(video.poster_order, VecDeque::from([Id(3), Id(4)]));
		assert!(video.poster_queue.is_empty());
		assert!(video.accept_frame(&ctx, 1, 1, &[0, 0, 0, 255]));
		show(&mut video);
		assert_eq!(video.poster_order, VecDeque::from([Id(3), Id(4)]));
		assert!(video.poster_queue.is_empty());
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

/// A high precision video view supplied by the desktop renderer. UI code never
/// reads media planes or performs color conversion.
#[derive(Clone)]
pub struct MediaView {
	pub size: egui::Vec2,
	/// Renderer-owned per-player cache, reused across frames and dropped with the view.
	pub resource: std::sync::Arc<dyn std::any::Any + Send + Sync>,
	pub failure: std::sync::Arc<std::sync::Mutex<Option<&'static str>>>,
	pub callback: std::sync::Arc<dyn Fn(egui::Rect) -> egui::PaintCallback + Send + Sync>,
}
impl MediaView {
	pub fn paint(&self, ui: &egui::Ui, rect: egui::Rect) {
		ui.painter()
			.with_clip_rect(rect)
			.add(egui::Shape::Callback((self.callback)(rect)));
		if let Ok(failure) = self.failure.lock()
			&& let Some(error) = *failure
		{
			let painter = ui.painter().with_clip_rect(rect);
			let text = painter.layout(
				error.into(),
				egui::FontId::proportional(13.0),
				egui::Color32::WHITE,
				(rect.width() - 24.0).max(1.0),
			);
			painter.galley(
				rect.center() - text.size() / 2.0,
				text,
				egui::Color32::WHITE,
			);
		}
		// Also revisit presentation capabilities while paused, without polling
		// capture devices or changing the user's playback state.
		ui.ctx()
			.request_repaint_after(std::time::Duration::from_secs(1));
	}
}
