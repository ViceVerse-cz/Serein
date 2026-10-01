//! Desktop ownership for watching one other participant's screen share. The choice is an
//! explicit click; frames are decoded off the render thread and never written to disk.
use client_core::{Command, Event, State, screen, voice};
use discord_voice::{Identity, Status};
use eframe::egui;
use model::Id;
use std::{
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::watch, task::JoinHandle};
use zeroize::Zeroizing;

const SIGNAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Replace an undisplayed frame in place; UI ownership moves out through the existing slot.
#[allow(clippy::chunks_exact_to_as_chunks)] // Match egui's conversion loop.
fn store_frame(
	picture: &mut Option<egui::ColorImage>,
	frame: discord_voice::RemoteFrame<'_>,
) -> bool {
	let size = [frame.width as usize, frame.height as usize];
	if size.contains(&0)
		|| size[0]
			.checked_mul(size[1])
			.and_then(|pixels| pixels.checked_mul(4))
			!= Some(frame.rgba.len())
	{
		return false;
	}
	if let Some(image) = picture.as_mut() {
		image.size = size;
		image.source_size = egui::vec2(frame.width as f32, frame.height as f32);
		let pixels = frame.rgba.len() / 4;
		if image.pixels.capacity() > 1024 * 1024 / size_of::<egui::Color32>()
			&& image.pixels.capacity() > pixels.saturating_mul(4)
		{
			// Release a previous large resolution while keeping ordinary resize reuse.
			image.pixels = Vec::with_capacity(pixels);
		}
		if pixels > image.pixels.capacity() {
			// Do not double a previous resolution's allocation beyond the frame bound.
			image.pixels.reserve_exact(pixels - image.pixels.len());
		}
		// egui's pixel conversion is optimized in debug builds too; keep large
		// scalar application loops out of that path with bounded conversion scratch.
		#[cfg(debug_assertions)]
		{
			image.pixels.resize(pixels, egui::Color32::TRANSPARENT);
			for (output, input) in image
				.as_raw_mut()
				.chunks_mut(64 * 1024)
				.zip(frame.rgba.chunks(64 * 1024))
			{
				let converted =
					egui::ColorImage::from_rgba_unmultiplied([input.len() / 4, 1], input);
				output.copy_from_slice(converted.as_raw());
			}
		}
		#[cfg(not(debug_assertions))]
		{
			image.pixels.clear();
			image.pixels.extend(frame.rgba.chunks_exact(4).map(|pixel| {
				egui::Color32::from_rgba_unmultiplied(pixel[0], pixel[1], pixel[2], pixel[3])
			}));
		}
	} else {
		*picture = Some(egui::ColorImage::from_rgba_unmultiplied(size, frame.rgba));
	}
	true
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Context {
	generation: u64,
	channel: Id,
	request: u64,
	stream_request: u64,
	streamer: Id,
}
struct Pending {
	context: Context,
	user: Id,
	peer: Option<Id>,
	session: Zeroizing<String>,
	identity: Arc<Identity>,
	audio: Option<std::sync::mpsc::SyncSender<discord_voice::Frame>>,
	rtc: Option<(Id, Id)>,
	server: Option<(voice::Secret, String)>,
	started: Instant,
}
#[derive(Clone, Copy)]
enum Notice {
	Status(&'static str),
	Failed(&'static str),
}
struct Live {
	context: Context,
	task: JoinHandle<()>,
	events: watch::Receiver<Option<Notice>>,
	frames: Arc<Mutex<Option<egui::ColorImage>>>,
}

#[derive(Default)]
pub(super) struct Watch {
	pending: Option<Pending>,
	live: Option<Live>,
	/// The streamer stopped or Discord failed the view; the state choice is cleared next poll.
	ended: Option<&'static str>,
	sequence: u64,
	status: &'static str,
	/// Why the last view stopped; shown as a stage notice until the next request or hang-up.
	notice: &'static str,
}
impl Watch {
	pub fn stop(&mut self) {
		self.pending = None;
		self.ended = None;
		if let Some(live) = self.live.take() {
			live.task.abort();
		}
	}
	fn context(&self) -> Option<Context> {
		self.pending
			.as_ref()
			.map(|pending| pending.context)
			.or_else(|| self.live.as_ref().map(|live| live.context))
	}
	/// Take negotiation secrets before the UI reduces the event. Nothing is persisted.
	pub fn observe(&mut self, state: &State, event: &mut Event) {
		let Event::Voice(voice::Event::Watch {
			channel,
			request,
			stream_request,
			streamer,
			event,
		}) = event
		else {
			return;
		};
		let Some(context) = self.context() else {
			return;
		};
		if context.generation != state.generation
			|| (
				context.channel,
				context.request,
				context.stream_request,
				context.streamer,
			) != (*channel, *request, *stream_request, *streamer)
		{
			return;
		}
		match event {
			screen::Event::Created {
				rtc_server,
				rtc_channel,
			} => {
				if let Some(pending) = &mut self.pending {
					pending.rtc = Some((*rtc_server, *rtc_channel));
				}
			}
			screen::Event::Server { token, endpoint } => {
				if let Some(pending) = &mut self.pending {
					match (token.take(), endpoint.take()) {
						(Some(token), Some(endpoint)) => pending.server = Some((token, endpoint)),
						// A null endpoint means Discord is still allocating the stream server.
						(_, None) => pending.server = None,
						(None, Some(_)) => {
							self.ended = Some("Discord omitted the stream connection token");
						}
					}
				}
			}
			screen::Event::Deleted { reason } => {
				self.ended = Some(reason.unwrap_or("The stream ended"));
			}
			screen::Event::Failed(message) => self.ended = Some(message),
		}
	}
	pub fn poll(
		&mut self,
		runtime: &Runtime,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		ctx: &egui::Context,
		call: Option<super::screen::Call<'_>>,
		audio: Option<std::sync::mpsc::SyncSender<discord_voice::Frame>>,
	) -> Option<Command> {
		let wanted = state.voice.active.as_ref().and_then(|active| {
			let streamer = active.watching?;
			matches!(
				active.phase,
				voice::Phase::Connected | voice::Phase::Waiting
			)
			.then_some((state.generation, active.channel, active.request, streamer))
		});
		let current = self.context();
		let mut command = None;
		if let Some(context) = current
			&& (state.demo
				|| wanted
					!= Some((
						context.generation,
						context.channel,
						context.request,
						context.streamer,
					))) {
			self.stop();
			self.status = "Stopped watching";
			if context.generation == state.generation && !state.demo {
				command = Some(Command::Voice(voice::Command::StopWatching {
					channel: context.channel,
					request: context.request,
					stream_request: context.stream_request,
				}));
			}
		}
		if state.voice.active.is_none() {
			self.notice = "";
		}
		if let Some(message) = self.ended.take() {
			let context = self.context();
			self.stop();
			self.status = message;
			self.notice = message;
			state.stop_watching();
			ui.voice_stream_view = None;
			ui.voice_stream_status = self.status;
			return command.or_else(|| {
				let context = context.filter(|context| context.generation == state.generation)?;
				Some(Command::Voice(voice::Command::StopWatching {
					channel: context.channel,
					request: context.request,
					stream_request: context.stream_request,
				}))
			});
		}
		if state.demo {
			ui.voice_stream_status = if wanted.is_some() {
				"Offline preview · no stream is received"
			} else {
				""
			};
			return command;
		}
		if let Some((generation, channel, request, streamer)) = wanted
			&& self.context().is_none()
			&& command.is_none()
		{
			let Some(call) = call.filter(|call| {
				call.generation == generation && call.channel == channel && call.request == request
			}) else {
				ui.voice_stream_status = "Connect the call before watching a stream";
				return None;
			};
			self.sequence = self.sequence.wrapping_add(1);
			let context = Context {
				generation,
				channel,
				request,
				stream_request: self.sequence,
				streamer,
			};
			self.pending = Some(Pending {
				context,
				user: call.user,
				peer: call.peer,
				session: Zeroizing::new(call.session.to_owned()),
				identity: call.identity,
				audio,
				rtc: None,
				server: None,
				started: Instant::now(),
			});
			self.status = "Requesting the stream…";
			self.notice = "";
			command = Some(Command::Voice(voice::Command::WatchStream {
				channel,
				request,
				stream_request: context.stream_request,
				streamer,
			}));
		}
		if let Some(pending) = &self.pending {
			if pending.started.elapsed() >= SIGNAL_TIMEOUT {
				self.ended = Some("Discord did not provide the stream connection; try again");
				ctx.request_repaint();
			} else {
				ctx.request_repaint_after(SIGNAL_TIMEOUT.saturating_sub(pending.started.elapsed()));
			}
		}
		if self
			.pending
			.as_ref()
			.is_some_and(|pending| pending.rtc.is_some() && pending.server.is_some())
		{
			let pending = self.pending.take().expect("complete stream negotiation");
			if let Err(error) = self.start(runtime, pending, ctx) {
				self.ended = Some(error);
			}
		}
		if let Some(live) = &mut self.live {
			match *live.events.borrow_and_update() {
				Some(Notice::Status(status)) => self.status = status,
				Some(Notice::Failed(error)) => self.ended = Some(error),
				None => {}
			}
			if live.task.is_finished() && self.ended.is_none() {
				self.ended = Some("The stream connection ended");
			}
			let image = live.frames.try_lock().ok().and_then(|mut slot| slot.take());
			if let Some(image) = image {
				if let Some(texture) = &mut ui.voice_stream_view {
					texture.set(image, egui::TextureOptions::LINEAR);
				} else {
					ui.voice_stream_view = Some(ctx.load_texture(
						"remote-stream",
						image,
						egui::TextureOptions::LINEAR,
					));
				}
				self.status = "Watching the stream";
			}
		} else {
			ui.voice_stream_view = None;
		}
		ui.voice_stream_status = if wanted.is_some() {
			self.status
		} else {
			self.notice
		};
		command
	}
	fn start(
		&mut self,
		runtime: &Runtime,
		mut pending: Pending,
		ctx: &egui::Context,
	) -> Result<(), &'static str> {
		let (rtc_server, rtc_channel) = pending.rtc.take().ok_or("Missing stream RTC identity")?;
		let (token, endpoint) = pending.server.take().ok_or("Missing stream server")?;
		let session = voice::Secret::new(pending.session.to_string())
			.map_err(|_| "Invalid stream voice session")?;
		let credentials = voice::VoiceConnection {
			channel: rtc_channel,
			guild: Some(rtc_server),
			user: pending.user,
			peer: pending.peer,
			session,
			token,
			endpoint,
			request: pending.context.stream_request,
		};
		let frames = Arc::new(Mutex::new(None));
		let slot = frames.clone();
		let wake = ctx.clone();
		let streamer = pending.context.streamer.0;
		let sink: discord_voice::VideoSink = Arc::new(move |frame: discord_voice::RemoteFrame| {
			if frame.user != streamer {
				return;
			}
			if slot
				.lock()
				.is_ok_and(|mut slot| store_frame(&mut slot, frame))
			{
				wake.request_repaint();
			}
		});
		let (send, events) = watch::channel(None);
		let wake = ctx.clone();
		let identity = pending.identity;
		let audio = pending.audio;
		let task = runtime.spawn(async move {
			let (status_send, status_wake) = (send.clone(), wake.clone());
			let result =
				discord_voice::watch_stream(credentials, identity, sink, audio, move |event| {
					let status = match event {
						Status::Connecting => "Connecting to the stream…",
						Status::Discovering => "Checking the stream network…",
						Status::TransportReady | Status::Securing => "Securing the stream…",
						Status::WaitingForPeer => "Waiting for the streamer…",
						Status::Ready { .. } => "Stream secured · waiting for video",
						Status::RemoteAudio | Status::Speaking(_) | Status::CameraAvailable(_) => {
							return Ok(());
						}
					};
					status_send.send_replace(Some(Notice::Status(status)));
					status_wake.request_repaint();
					Ok(())
				})
				.await;
			if let Err(error) = result {
				send.send_replace(Some(Notice::Failed(error)));
			}
			wake.request_repaint();
		});
		self.status = "Connecting to the stream…";
		self.live = Some(Live {
			context: pending.context,
			task,
			events,
			frames,
		});
		Ok(())
	}
}
impl Drop for Watch {
	fn drop(&mut self) {
		self.stop();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn frame(width: u32, height: u32, rgba: &[u8]) -> discord_voice::RemoteFrame<'_> {
		discord_voice::RemoteFrame {
			user: 7,
			width,
			height,
			rgba,
		}
	}

	#[test]
	fn watch_reuses_dropped_frames_and_delivers_only_updates() {
		let mut picture = None;
		assert!(picture.take().is_none());
		assert!(store_frame(
			&mut picture,
			frame(2, 1, &[1, 2, 3, 255, 4, 5, 6, 128])
		));
		let pixels = picture.as_ref().unwrap().pixels.as_ptr();
		for _ in 0..20 {
			assert!(store_frame(
				&mut picture,
				frame(2, 1, &[9, 8, 7, 255, 6, 5, 4, 128])
			));
			assert_eq!(picture.as_ref().unwrap().pixels.as_ptr(), pixels);
		}
		let latest = picture.take().unwrap();
		assert_eq!(
			latest.pixels,
			egui::ColorImage::from_rgba_unmultiplied([2, 1], &[9, 8, 7, 255, 6, 5, 4, 128]).pixels
		);
		assert!(picture.take().is_none());
		picture = Some(latest);
		assert!(store_frame(&mut picture, frame(1, 1, &[3, 2, 1, 0])));
		assert_eq!(picture.as_ref().unwrap().pixels.as_ptr(), pixels);
		let latest = picture.take().unwrap();
		assert_eq!(latest.size, [1, 1]);
		assert_eq!(latest.source_size, egui::vec2(1.0, 1.0));
		assert_eq!(latest.pixels, vec![egui::Color32::TRANSPARENT]);
	}

	#[test]
	fn watch_preserves_pixels_owned_by_pending_egui_uploads() {
		let mut picture = None;
		let ctx = egui::Context::default();
		let first = [1, 2, 3, 255, 4, 5, 6, 128];
		assert!(store_frame(&mut picture, frame(2, 1, &first)));
		// Discard Context's initial font texture allocation so only this upload is inspected.
		ctx.tex_manager().write().take_delta().clear();
		let mut texture = ctx.load_texture(
			"synthetic-watch",
			picture.take().unwrap(),
			egui::TextureOptions::LINEAR,
		);
		assert!(picture.is_none());
		assert!(store_frame(&mut picture, frame(1, 1, &[10, 20, 30, 255])));
		let mut delta = ctx.tex_manager().write().take_delta();
		assert_eq!(delta.set.len(), 1);
		let uploads = &delta.set[&texture.id()];
		assert_eq!(uploads.len(), 1);
		assert!(uploads[0].is_whole());
		let egui::ImageData::Color(pending) = &uploads[0].image;
		assert_eq!(pending.size, [2, 1]);
		assert_eq!(
			pending.pixels,
			egui::ColorImage::from_rgba_unmultiplied([2, 1], &first).pixels
		);
		assert_eq!(picture.as_ref().unwrap().size, [1, 1]);
		delta.clear();
		texture.set(picture.take().unwrap(), egui::TextureOptions::LINEAR);
		assert!(store_frame(&mut picture, frame(2, 1, &first)));
		let mut updated = ctx.tex_manager().write().take_delta();
		assert_eq!(updated.set.len(), 1);
		let uploads = &updated.set[&texture.id()];
		assert_eq!(uploads.len(), 1);
		assert!(uploads[0].is_whole());
		let egui::ImageData::Color(pending) = &uploads[0].image;
		assert_eq!(pending.size, [1, 1]);
		assert_eq!(pending.pixels, vec![egui::Color32::from_rgb(10, 20, 30)]);
		updated.clear();
		drop(texture);
		ctx.tex_manager().write().take_delta().clear();
	}

	#[test]
	fn watch_invalid_frames_preserve_the_last_valid_picture() {
		let mut picture = None;
		assert!(store_frame(&mut picture, frame(1, 1, &[1, 2, 3, 255])));
		let pixels = picture.as_ref().unwrap().pixels.as_ptr();
		for invalid in [
			frame(0, 1, &[]),
			frame(1, 0, &[]),
			frame(1, 1, &[0; 3]),
			frame(u32::MAX, u32::MAX, &[]),
		] {
			assert!(!store_frame(&mut picture, invalid));
			assert_eq!(picture.as_ref().unwrap().pixels.as_ptr(), pixels);
			assert_eq!(picture.as_ref().unwrap().size, [1, 1]);
		}
		assert!(store_frame(&mut picture, frame(3, 1, &[0; 12])));
		assert_eq!(picture.take().unwrap().pixels.len(), 3);
	}

	#[test]
	fn watch_releases_large_pixel_capacity_after_a_resolution_drop() {
		let mut picture = None;
		let rgba = vec![255; 1920 * 1080 * 4];
		assert!(store_frame(&mut picture, frame(1920, 1080, &rgba)));
		assert_eq!(picture.as_ref().unwrap().pixels.len(), 1920 * 1080);
		assert!(store_frame(
			&mut picture,
			frame(16, 16, &[255; 16 * 16 * 4])
		));
		let latest = picture.take().unwrap();
		assert_eq!(latest.size, [16, 16]);
		assert_eq!(latest.pixels.len(), 16 * 16);
		assert!(latest.pixels.capacity() <= 16 * 16 * 4);
	}

	#[test]
	fn watch_resolution_growth_does_not_double_the_pixel_capacity() {
		let mut picture = None;
		let rgba = vec![255; 1920 * 1080 * 4];
		assert!(store_frame(
			&mut picture,
			frame(1600, 900, &rgba[..1600 * 900 * 4])
		));
		assert!(store_frame(&mut picture, frame(1920, 1080, &rgba)));
		let latest = picture.take().unwrap();
		assert_eq!(latest.size, [1920, 1080]);
		assert_eq!(latest.pixels.len(), 1920 * 1080);
		assert!(latest.pixels.capacity() <= 1920 * 1080);
	}

	#[test]
	fn watch_reused_pixels_match_egui_across_scratch_boundaries() {
		let mut picture = None;
		let mut rgba = vec![0; 129 * 129 * 4];
		assert!(store_frame(&mut picture, frame(129, 129, &rgba)));
		let pixels = picture.as_ref().unwrap().pixels.as_ptr();
		for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
			pixel.copy_from_slice(&[index as u8, 70, 210, [255, 128, 0, 32][index % 4]]);
		}
		assert!(store_frame(&mut picture, frame(129, 129, &rgba)));
		let latest = picture.take().unwrap();
		assert_eq!(latest.pixels.as_ptr(), pixels);
		assert_eq!(
			latest.pixels,
			egui::ColorImage::from_rgba_unmultiplied([129, 129], &rgba).pixels
		);
	}

	/// Run this test executable directly under `/usr/bin/time -l` for process peak RSS.
	/// Set SEREIN_WATCH_FRAME_LEGACY=1 for the original allocation-per-frame path.
	/// Set SEREIN_WATCH_FRAME_UPLOAD_EVERY=3 to include a pending upload every third frame.
	/// Both modes include the same synthetic 1080p input; no device or transport starts.
	#[test]
	#[ignore = "synthetic release CPU/RSS workload; run with --release --ignored --nocapture"]
	fn watch_frame_memory_workload() {
		let legacy = std::env::var_os("SEREIN_WATCH_FRAME_LEGACY").is_some();
		let uploads_every = std::env::var("SEREIN_WATCH_FRAME_UPLOAD_EVERY")
			.ok()
			.map(|text| {
				text.parse::<usize>()
					.expect("upload interval must be an integer")
			})
			.unwrap_or(0);
		let rgba = vec![255; 1920 * 1080 * 4];
		let mut samples = Vec::new();
		for run in 0..6 {
			let mut old = None;
			let mut picture = None;
			let mut uploading = None;
			let started = Instant::now();
			for index in 0..120 {
				if legacy {
					old = Some(egui::ColorImage::from_rgba_unmultiplied(
						[1920, 1080],
						&rgba,
					));
				} else {
					assert!(store_frame(&mut picture, frame(1920, 1080, &rgba)));
				}
				if uploads_every > 0 && index % uploads_every == 0 {
					uploading = if legacy { old.take() } else { picture.take() };
				} else {
					uploading = None;
				}
				std::hint::black_box((&old, &picture, &uploading));
			}
			std::hint::black_box(uploading);
			if run > 0 {
				samples.push(started.elapsed().as_secs_f64() * 1000.0);
			}
		}
		samples.sort_by(f64::total_cmp);
		println!(
			"watch_frame_memory_workload legacy={legacy} frames=120 uploads_every={uploads_every} samples_ms={samples:?} median_ms={:.3} frame_bytes={}",
			samples[2],
			rgba.len()
		);
	}
}
