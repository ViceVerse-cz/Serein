//! Linux ownership: one portal session and one bounded media pipeline, no recorder.
use super::{
	AudioChunk, EncodedFrame, ScreenEncoder, Settings, SourceId, audio_linux,
	gstreamer::{self as capture, Capture},
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

fn niri_desktop(desktop: &str) -> bool {
	desktop
		.split(':')
		.any(|name| name.eq_ignore_ascii_case("niri"))
}

/// Niri 26.04 leaves SPA header PTS at zero, which GstBaseSrc preserves (plus
/// its startup offset) even with do-timestamp=true. Both videorate branches then
/// discard subsequent pictures. Timestamp at arrival, before either branch.
/// Only buffer metadata is made writable; pixel memory remains shared.
pub(super) fn timestamp_niri_frames(source: &gst::Element) -> Result<(), &'static str> {
	let weak = source.downgrade();
	source
		.static_pad("src")
		.ok_or("Screen capture source has no output")?
		.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
			if let Some(time) = weak
				.upgrade()
				.and_then(|source| source.current_running_time())
				&& let Some(gst::PadProbeData::Buffer(buffer)) = &mut info.data
			{
				let buffer = buffer.make_mut();
				buffer.set_pts(time);
				buffer.set_dts(time);
			}
			gst::PadProbeReturn::Ok
		});
	Ok(())
}

fn note(event: &str, value: &str) {
	if std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|set| set == "1") {
		eprintln!("[Serein voice Screen] {event}={value}");
	}
}

use gst::prelude::*;
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
	video_settings: model::voice_settings::VideoSettings,
	adapter: Option<model::VideoAdapter>,
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
					audio_linux::Worker::start(
						send,
						stop.clone(),
						ready.clone(),
						audio_epoch.clone(),
					)
				})
				.transpose()?;
			if stop.load(Ordering::Acquire) || send.is_closed() {
				return Ok(());
			}
			if portal.as_mut().is_some_and(Portal::is_closed) {
				return Err("The desktop stopped screen sharing");
			}
			// Keep the PipeWire descriptor alive until the raw capture pipeline is destroyed.
			let (source, _remote) = if let Some(portal) = &mut portal {
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
				if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| niri_desktop(&desktop))
				{
					timestamp_niri_frames(&source)?;
				}
				// Damage-driven desktops still need a fresh IDR when a viewer joins an idle screen.
				source.set_property("keepalive-time", 1000i32);
				source.set_property("min-buffers", 2i32);
				source.set_property("max-buffers", 4i32);
				(source, Some(remote))
			} else {
				(x11_source(settings.cursor)?, None)
			};
			let capacity = send.clone();
			keyframe.store(true, Ordering::Release);
			let mut reset_generation = audio_epoch.load(Ordering::Acquire);
			let pipeline = Capture::new(
				settings,
				source,
				stop.clone(),
				ready.clone(),
				keyframe.clone(),
				move || capacity.capacity() > 0,
			)?;
			if let Ok(mut label) = status.lock() {
				*label = "Starting screen capture…";
			}
			wake();
			let mut encoding: Option<ScreenEncoder> = None;
			let mut next_epoch = 0;
			// One bounded raw snapshot lets an idle desktop satisfy a new viewer's IDR
			// without waiting for another compositor damage event.
			let mut latest_frame = None;
			let mut capture_reset = false;
			let mut last_encoded: Option<Instant> = None;
			let interval = Duration::from_secs_f64(1.0 / f64::from(settings.fps));
			let mut next_frame = Instant::now();
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
					return Err(
						"Screen capture stopped; check PipeWire, portal and GStreamer plugins",
					);
				}
				let reset = audio_epoch.load(Ordering::Acquire);
				if reset == u64::MAX {
					return Err("Screen security reset generation exhausted");
				}
				if reset != reset_generation {
					reset_generation = reset;
					if let Some(encoder) = &mut encoding {
						encoder.reset_for_security(reset)?;
					}
					latest_frame = None;
					waiting_keyframe = true;
					first_frame = None;
					last_encoded = None;
					next_frame = Instant::now();
					keyframe.store(true, Ordering::Release);
					// Request a post-reset snapshot once the raw gate is open, even if
					// the desktop produces no further damage or keepalive buffers.
					capture_reset = true;
					let _ = pipeline.frames.try_pull_sample(gst::ClockTime::ZERO);
				}
				let target = bitrate
					.load(Ordering::Acquire)
					.clamp(250_000, settings.bit_rate());
				// A rate change updates only the encoder, preserving the approved
				// source, preview and audio. Do not restart while waiting for its first IDR.
				if !waiting_keyframe
					&& let Some(encoder) = encoding.as_mut()
					&& encoder.set_bitrate(target)?
				{
					waiting_keyframe = true;
					keyframe.store(true, Ordering::Release);
				}
				// Counted per pass: whether a picture was taken, and whether one was left
				// in the pipeline because the transport had not drained the last.
				let mut pulled = false;
				let mut withheld = 0;
				let mut wait_for_frame = false;
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
					return Err("Screen capture did not produce a preview");
				}
				if !ready.load(Ordering::Acquire) {
					if let Ok(mut label) = status.lock()
						&& *label != "Screen preview · waiting for others"
					{
						*label = "Screen preview · waiting for others";
						wake();
					}
					if let Some(encoder) = &encoding {
						next_epoch = encoder
							.epoch()
							.checked_add(1)
							.ok_or("Screen encoder epoch exhausted")?;
					}
					encoding = None;
					// The raw gate stops updating during a security pause. Discard its old
					// snapshot so the resumed share starts from newly captured contents.
					latest_frame = None;
					slow = None;
					waiting_keyframe = true;
					first_frame = None;
					last_encoded = None;
					next_frame = Instant::now();
					keyframe.store(true, Ordering::Release);
					let _ = pipeline.frames.try_pull_sample(gst::ClockTime::ZERO);
				} else {
					if capture_reset && send.capacity() > 0 {
						pipeline.request_frame()?;
						capture_reset = false;
					}
					let started = first_frame.get_or_insert_with(Instant::now);
					// While the transport is behind, leave the picture in the appsink rather
					// than pulling and discarding it. The sink then blocks upstream, so
					// raw queues shed stale captures without advancing the encoder reference chain,
					// and this iteration still reaches the await below. Skipping the await
					// here would spin the worker and starve the portal on this runtime.
					let room = send.capacity() > 0;
					withheld = u64::from(!room);
					let now = Instant::now();
					let paced = now + Duration::from_millis(2) >= next_frame;
					wait_for_frame = room && !paced;
					if room && paced {
						let pull = metrics.start();
						let raw = pipeline
							.frames
							.try_pull_sample(gst::ClockTime::ZERO)
							.map(|sample| capture::raw(&sample))
							.transpose()?;
						if let Some(raw) = &raw {
							pulled = true;
							if raw.width != settings.width || raw.height != settings.height {
								return Err("Screen frame dimensions changed unexpectedly");
							}
							metrics.finish(crate::diagnostics::Stage::Receive, pull);
						}
						let requested_keyframe =
							keyframe.load(Ordering::Acquire) || waiting_keyframe;
						// Keep a static share alive at one picture per second, even if the
						// source no longer emits its own PipeWire keepalive buffers.
						let keepalive = last_encoded
							.is_some_and(|last| last.elapsed() >= Duration::from_secs(1));
						if super::retain_screen_frame(
							&mut latest_frame,
							raw,
							ready.load(Ordering::Acquire),
							requested_keyframe
								|| keepalive || encoding.as_ref().is_some_and(ScreenEncoder::pending),
						)? {
							if encoding.is_none() {
								encoding = Some(ScreenEncoder::new_on_adapter(
									settings,
									target,
									video_settings,
									adapter,
									next_epoch,
									reset_generation,
								)?);
							}
							let encoder = encoding.as_mut().expect("screen encoder initialized");
							let start = metrics.start();
							let force_keyframe = keyframe.swap(false, Ordering::AcqRel);
							if audio_epoch.load(Ordering::Acquire) != reset_generation {
								continue;
							}
							let packet = encoder.encode_at(
								latest_frame.as_ref().expect("latest screen frame"),
								force_keyframe,
								(origin.elapsed().as_micros() * 90 / 1000) as u32,
							)?;
							// Advance from the schedule, with jitter tolerance and no catch-up burst.
							next_frame = (next_frame + interval).max(now + interval / 2);
							last_encoded = Some(now);
							// A static source cannot notify again to advance lookahead. Pump
							// its retained snapshot on the selected cadence while pictures wait.
							wait_for_frame = encoder.pending();
							metrics.finish(crate::diagnostics::Stage::Encode, start);
							if !packet.data.is_empty() && (!waiting_keyframe || packet.keyframe) {
								let frame = EncodedFrame {
									codec: video_settings.codec,
									data: packet.data,
									keyframe: packet.keyframe,
									timestamp: packet.timestamp,
									epoch: packet.epoch,
									reset_generation: encoder.reset_generation(),
								};
								if ready.load(Ordering::Acquire)
									&& audio_epoch.load(Ordering::Acquire)
										== encoder.reset_generation()
									&& !stop.load(Ordering::Acquire)
									&& send.try_send(frame).is_ok()
								{
									waiting_keyframe = false;
									*started = Instant::now();
									let active = if audio_stopped {
										"Screen sharing · system audio stopped"
									} else {
										encoder.label()
									};
									if let Ok(mut label) = status.lock()
										&& *label != active
									{
										*label = active;
										wake();
									}
								} else {
									encoder.restart()?;
									waiting_keyframe = true;
									keyframe.store(true, Ordering::Release);
								}
							}
						}
					}
					// Only startup has a frame deadline: an unchanged desktop can stop producing frames.
					if waiting_keyframe && started.elapsed() > Duration::from_secs(15) {
						return Err("Screen video encoder did not produce a keyframe");
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
							&format!("{} withheld={withheld_total}", since.elapsed().as_millis()),
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
				} else if wait_for_frame {
					// A full raw appsink cannot notify again until it is drained. Wake at
					// the pacing deadline even when the preview branch is hidden.
					tokio::select! {
						_ = tokio::time::sleep_until(tokio::time::Instant::from_std(next_frame)) => {},
						_ = pipeline.changed() => {},
					}
				} else {
					pipeline.changed().await;
				}
			}
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

#[cfg(test)]
mod tests {
	#[test]
	fn niri_detection_preserves_other_desktops() {
		for desktop in ["niri", "Niri", "GNOME:niri"] {
			assert!(super::niri_desktop(desktop));
		}
		for desktop in ["", "GNOME", "KDE", "sway", "Hyprland", "not-niri"] {
			assert!(!super::niri_desktop(desktop));
		}
	}
}
