//! Linux ownership: one portal session and one bounded media pipeline, no recorder.
use super::{
	AudioChunk, EncodedFrame, MAX_ENCODED_BYTES, Settings, SourceId, audio_linux, encode_pixels,
	encoder,
	gstreamer::{self as capture, Capture, Mode},
	portal_linux::Portal,
	preview_frame,
};
use ::gstreamer as gst;
/// A capture source that has nothing new to send still emits a keepalive picture once a
/// second, so a frozen share is not a gap between pictures but a run of seconds carrying
/// only that keepalive. Report such a run once it ends, with whatever was withheld during
/// it, which separates a desktop that stopped drawing from a pipeline we held back.
const SLOW_PICTURES: u32 = 2;

pub(super) fn x11_session() -> bool {
	std::env::var_os("XDG_SESSION_TYPE").is_some_and(|value| value == "x11")
		&& std::env::var_os("WAYLAND_DISPLAY").is_none()
		&& std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty())
}

fn note(event: &str, value: &str) {
	if std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|set| set == "1") {
		eprintln!("[Serein voice Screen] {event}={value}");
	}
}

use gst::prelude::*;
use openh264::formats::YUVBuffer;
use std::{
	os::fd::AsRawFd,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};

pub(super) fn x11_source(cursor: bool) -> Result<gst::Element, &'static str> {
	// winit initializes Xlib threading when opening the desktop's X11 connection,
	// before this source is started by the capture worker.
	gst::ElementFactory::make("ximagesrc")
		.property("show-pointer", cursor)
		.property("use-damage", false)
		.build()
		.map_err(|_| "X11 capture requires the GStreamer Good plugins (gst-plugins-good)")
}

#[allow(clippy::too_many_arguments)] // The existing worker's bounded media outputs.
pub(super) fn run(
	settings: Settings,
	stop: Arc<AtomicBool>,
	ready: Arc<AtomicBool>,
	keyframe: Arc<AtomicBool>,
	bitrate: Arc<AtomicU32>,
	send: tokio::sync::mpsc::Sender<EncodedFrame>,
	audio_send: Option<tokio::sync::mpsc::Sender<AudioChunk>>,
	audio_epoch: Arc<AtomicU64>,
	preview: Arc<Mutex<Option<image::RgbaImage>>>,
	status: Arc<Mutex<&'static str>>,
	preview_visible: Arc<AtomicBool>,
	wake: &impl Fn(),
) -> Result<(), &'static str> {
	let direct = settings.source == SourceId::X11Desktop && x11_session();
	if (!direct && settings.source != SourceId::Portal) || !settings.valid() {
		return Err("Choose a source with the Linux screen picker");
	}
	gst::init().map_err(|_| "GStreamer is unavailable")?;
	// This runtime belongs to the existing media worker, never the render thread.
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.map_err(|_| "Could not start the screen picker")?;
	runtime.block_on(async {
		let mut portal = if direct {
			None
		} else {
			Some(Portal::open(settings.cursor, &stop).await?)
		};
		let origin = Instant::now();
		let mut metrics = crate::diagnostics::Metrics::new(crate::diagnostics::Scope::ScreenVideo);
		let mut audio = None;
		// Sticky: the label must keep saying so after the worker is gone.
		let mut audio_stopped = false;
		let result = async {
			if stop.load(Ordering::Acquire) || send.is_closed() {
				return Ok(());
			}
			audio = audio_send
				.map(|send| {
					audio_linux::Worker::start(send, stop.clone(), ready.clone(), audio_epoch)
				})
				.transpose()?;
			let mut mode_index = 0;
			while let Some(&mode) = Mode::ALL.get(mode_index) {
				mode_index += 1;
				if stop.load(Ordering::Acquire) || send.is_closed() {
					return Ok(());
				}
				if portal.as_mut().is_some_and(Portal::is_closed) {
					return Err("The desktop stopped screen sharing");
				}
				let mut remote = None;
				let source = if let Some(portal) = &mut portal {
					let source = gst::ElementFactory::make("pipewiresrc").build().map_err(
						|_| "Install the GStreamer PipeWire plugin to share your screen",
					)?;
					remote = Some(portal.open_remote(&stop).await?);
					source.set_property(
						"fd",
						remote.as_ref().expect("portal remote opened").as_raw_fd(),
					);
					if let Some(serial) = portal
						.pipewire_serial
						.filter(|_| source.find_property("target-object").is_some())
					{
						source.set_property("target-object", serial.to_string());
					} else {
						source.set_property("path", portal.node_id.to_string());
					}
					source.set_property("do-timestamp", true);
					// Damage-driven desktops still need a fresh IDR when a viewer joins an idle screen.
					source.set_property("keepalive-time", 1000i32);
					source.set_property("min-buffers", 2i32);
					source.set_property("max-buffers", 4i32);
					source
				} else {
					x11_source(settings.cursor)?
				};
				let capacity = send.clone();
				keyframe.store(true, Ordering::Release);
				let mut active_bitrate = bitrate
					.load(Ordering::Acquire)
					.clamp(250_000, settings.bit_rate());
				let Ok(pipeline) = Capture::new(
					settings,
					mode,
					active_bitrate,
					source,
					stop.clone(),
					ready.clone(),
					keyframe.clone(),
					move || capacity.capacity() > 0,
				) else {
					continue;
				};
				if let Ok(mut label) = status.lock() {
					*label = "Starting screen capture…";
				}
				wake();
				let mut software = None;
				let mut encoder_diagnostics = None;
				let mut second = Instant::now();
				let mut pictures_second = 0u32;
				let mut withheld_second = 0u64;
				let mut slow: Option<(Instant, u64)> = None;
				let mut waiting_keyframe = true;
				let mut first_frame = None;
				let mut visible = true;
				let mut first_preview = Some(Instant::now());
				loop {
					if stop.load(Ordering::Acquire) || send.is_closed() {
						return Ok(());
					}
					if portal.as_mut().is_some_and(Portal::is_closed) {
						return Err("The desktop stopped screen sharing");
					}
					// Application audio is an extra, not the share itself. If its worker stops,
					// keep sending video and say so, rather than ending the screen share.
					if audio
						.as_mut()
						.is_some_and(|worker| worker.result().is_some())
					{
						audio = None;
						audio_stopped = true;
					}
					if pipeline.failed() {
						break;
					}
					let target = bitrate
						.load(Ordering::Acquire)
						.clamp(250_000, settings.bit_rate());
					// Let startup/recovery reach its existing deadline: changing targets must
					// not repeatedly restart a failing encoder before fallback can run.
					if target != active_bitrate && !waiting_keyframe {
						if mode != Mode::Software && pipeline.set_bitrate(target) {
							active_bitrate = target;
						} else if super::software_rate_change(active_bitrate, target) {
							// Restarts cost an IDR, so only large moves apply. Older plugins
							// cannot change rate while playing: reopen the same mode with a
							// fresh PipeWire remote, keeping the approved portal.
							if mode != Mode::Software {
								mode_index -= 1;
								break;
							}
							software = None;
							waiting_keyframe = true;
							keyframe.store(true, Ordering::Release);
							active_bitrate = target;
						}
					}
					// Counted per pass: whether a picture was taken, and whether one was left
					// in the pipeline because the transport had not drained the last.
					let mut pulled = false;
					let mut withheld = 0;
					let requested_visible = preview_visible.load(Ordering::Acquire);
					if visible != requested_visible {
						visible = requested_visible;
						pipeline.set_preview_visible(visible);
						if !visible {
							first_preview = None;
						}
					}
					if let Some(sample) = pipeline.preview.try_pull_sample(gst::ClockTime::ZERO) {
						let image = preview_frame(&capture::raw(&sample)?)?;
						if let Ok(mut slot) = preview.try_lock() {
							*slot = Some(image);
						}
						first_preview = None;
						wake();
					}
					if first_preview
						.is_some_and(|start: Instant| start.elapsed() > Duration::from_secs(15))
					{
						break;
					}
					if !ready.load(Ordering::Acquire) {
						if let Ok(mut label) = status.lock()
							&& *label != "Screen preview · waiting for others"
						{
							*label = "Screen preview · waiting for others";
							wake();
						}
						software = None;
						encoder_diagnostics = None;
						slow = None;
						waiting_keyframe = true;
						first_frame = None;
						keyframe.store(true, Ordering::Release);
						let _ = pipeline.frames.try_pull_sample(gst::ClockTime::ZERO);
					} else {
						let started = first_frame.get_or_insert_with(Instant::now);
						// While the transport is behind, leave the picture in the appsink rather
						// than pulling and discarding it. The sink then blocks upstream, so
						// pressure reaches the encoder instead of breaking its reference chain,
						// and this iteration still reaches the await below. Skipping the await
						// here would spin the worker and starve the portal on this runtime.
						let room = send.capacity() > 0;
						withheld = u64::from(!room);
						if room
							&& let Some(sample) =
								pipeline.frames.try_pull_sample(gst::ClockTime::ZERO)
						{
							pulled = true;
							let pull = metrics.start();
							metrics.finish(crate::diagnostics::Stage::Receive, pull);
							let (data, is_keyframe) = if mode == Mode::Software {
								let raw = capture::raw(&sample)?;
								if raw.width != settings.width || raw.height != settings.height {
									return Err("Screen frame dimensions changed unexpectedly");
								}
								if software.is_none() {
									software = Some((
										encoder(settings, active_bitrate)?,
										YUVBuffer::new(
											settings.width as usize,
											settings.height as usize,
										),
									));
								}
								let (encoder, yuv) =
									software.as_mut().expect("software encoder initialized");
								let start = metrics.start();
								let encoded = encode_pixels(
									encoder,
									yuv,
									&raw.data,
									(settings.width as usize, settings.height as usize),
									keyframe.swap(false, Ordering::AcqRel) || waiting_keyframe,
								)?;
								metrics.finish(crate::diagnostics::Stage::Encode, start);
								encoded
							} else {
								let buffer =
									sample.buffer().ok_or("Screen encoder returned no buffer")?;
								if buffer.size() > MAX_ENCODED_BYTES {
									return Err("Encoded screen frame exceeds the sharing limit");
								}
								let map = buffer
									.map_readable()
									.map_err(|_| "Screen video could not be read")?;
								crate::video::validate_source(&map)?;
								(
									map.to_vec(),
									!buffer.flags().contains(gst::BufferFlags::DELTA_UNIT),
								)
							};
							encoder_diagnostics.get_or_insert_with(|| {
								crate::diagnostics::EncoderRegistration::new(
									true,
									mode != Mode::Software,
								)
							});
							if !data.is_empty() && (!waiting_keyframe || is_keyframe) {
								let frame = EncodedFrame {
									data,
									keyframe: is_keyframe,
									timestamp: (origin.elapsed().as_micros() * 90 / 1000) as u32,
								};
								if ready.load(Ordering::Acquire)
									&& !stop.load(Ordering::Acquire)
									&& send.try_send(frame).is_ok()
								{
									waiting_keyframe = false;
									*started = Instant::now();
									let active = if audio_stopped {
										"Screen sharing · system audio stopped"
									} else {
										mode.label()
									};
									if let Ok(mut label) = status.lock()
										&& *label != active
									{
										*label = active;
										wake();
									}
								} else {
									waiting_keyframe = true;
									keyframe.store(true, Ordering::Release);
								}
							}
						}
						// Only startup has a frame deadline: an unchanged desktop can stop producing frames.
						if waiting_keyframe && started.elapsed() > Duration::from_secs(15) {
							break;
						}
					}
					metrics.poll(false, withheld, !pulled, 0);
					pictures_second += u32::from(pulled);
					withheld_second += withheld;
					if second.elapsed() >= Duration::from_secs(1) {
						if ready.load(Ordering::Acquire) && pictures_second <= SLOW_PICTURES {
							let entry = slow.get_or_insert((second, 0));
							entry.1 += withheld_second;
						} else if let Some((since, withheld_total)) = slow.take() {
							note(
								"capture_slow_ms",
								&format!(
									"{} withheld={withheld_total}",
									since.elapsed().as_millis()
								),
							);
						}
						second = Instant::now();
						pictures_second = 0;
						withheld_second = 0;
					}
					if withheld > 0 {
						// Draining the transport does not notify the appsink. Wake on capacity,
						// retaining changed()'s 100 ms bound for cancellation and portal checks.
						tokio::select! {
							_ = send.reserve() => {},
							_ = pipeline.changed() => {},
						}
					} else {
						pipeline.changed().await;
					}
				}
				// Failed encoders advance through the bounded alternatives; rate changes retry
				// the current encoder. Always destroy the old pipeline before opening another.
				drop(pipeline);
				drop(remote);
			}
			Err("No screen encoder could start; check PipeWire, portal and GStreamer plugins")
		}
		.await;
		ready.store(false, Ordering::Release);
		stop.store(true, Ordering::Release);
		drop(send);
		if let Some(portal) = portal {
			portal.close().await;
		}
		// Revoke the portal before waiting for a possibly blocked native audio driver.
		drop(audio);
		result
	})
}
