//! Linux ownership: one portal session and one bounded media pipeline, no recorder.
use super::{
	AudioChunk, EncodedFrame, MAX_ENCODED_BYTES, Settings, SourceId, audio_linux, encode_pixels,
	encoder,
	gstreamer::{self as capture, Capture, Mode},
	portal_linux::Portal,
	preview_frame,
};
use ::gstreamer as gst;
use gst::prelude::*;
use openh264::formats::YUVBuffer;
use std::{
	os::fd::AsRawFd,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};

#[allow(clippy::too_many_arguments)] // The existing worker's bounded media outputs.
pub(super) fn run(
	settings: Settings,
	stop: Arc<AtomicBool>,
	ready: Arc<AtomicBool>,
	keyframe: Arc<AtomicBool>,
	send: tokio::sync::mpsc::Sender<EncodedFrame>,
	audio_send: Option<tokio::sync::mpsc::Sender<AudioChunk>>,
	audio_epoch: Arc<AtomicU64>,
	preview: Arc<Mutex<Option<image::RgbaImage>>>,
	status: Arc<Mutex<&'static str>>,
	preview_visible: Arc<AtomicBool>,
	wake: &impl Fn(),
) -> Result<(), &'static str> {
	if settings.source != SourceId::Portal || !settings.valid() {
		return Err("Choose a source with the Linux screen picker");
	}
	gst::init().map_err(|_| "GStreamer is unavailable")?;
	// This runtime belongs to the existing media worker, never the render thread.
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.map_err(|_| "Could not start the screen picker")?;
	runtime.block_on(async {
		let mut portal = Portal::open(settings.cursor, &stop).await?;
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
			for mode in Mode::ALL {
				if stop.load(Ordering::Acquire) || send.is_closed() {
					return Ok(());
				}
				if portal.is_closed() {
					return Err("The desktop stopped screen sharing");
				}
				let source = gst::ElementFactory::make("pipewiresrc")
					.build()
					.map_err(|_| "Install the GStreamer PipeWire plugin to share your screen")?;
				let remote = portal.open_remote(&stop).await?;
				source.set_property("fd", remote.as_raw_fd());
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
				let capacity = send.clone();
				keyframe.store(true, Ordering::Release);
				let Ok(pipeline) = Capture::new(
					settings,
					mode,
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
				let mut waiting_keyframe = true;
				let mut first_frame = None;
				let mut visible = true;
				let mut first_preview = Some(Instant::now());
				loop {
					if stop.load(Ordering::Acquire) || send.is_closed() {
						return Ok(());
					}
					if portal.is_closed() {
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
						let room = mode != Mode::Software || send.capacity() > 0;
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
										encoder(settings)?,
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
					pipeline.changed().await;
				}
				// One bounded pass through alternatives, always destroying the old pipeline first.
				drop(pipeline);
			}
			Err("No screen encoder could start; check PipeWire, portal and GStreamer plugins")
		}
		.await;
		ready.store(false, Ordering::Release);
		stop.store(true, Ordering::Release);
		drop(send);
		portal.close().await;
		// Revoke the portal before waiting for a possibly blocked native audio driver.
		drop(audio);
		result
	})
}
