//! PulseAudio output monitor (also supported by PipeWire's PulseAudio server).
//! Separate from the ScreenCast portal, whose restricted remote contains video only.
use super::{AudioChunk, MAX_AUDIO_SAMPLES};
use ::gstreamer as gst;
use gst::prelude::*;
use gstreamer_app as app;
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU64, Ordering},
};

const UNAVAILABLE: &str = "System audio is unavailable; check the default output and GStreamer PulseAudio plugin, or share without audio";

pub(super) struct Worker {
	thread: Option<std::thread::JoinHandle<Result<(), &'static str>>>,
	stop: Arc<AtomicBool>,
}

impl Worker {
	pub(super) fn start(
		send: tokio::sync::mpsc::Sender<AudioChunk>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		epoch: Arc<AtomicU64>,
	) -> Result<Self, &'static str> {
		let worker_stop = stop.clone();
		let thread = std::thread::Builder::new()
			.name("screen-audio".into())
			.spawn(move || {
				// PulseAudio opening/device queries can block. Keep them off the video/portal worker.
				let runtime = tokio::runtime::Builder::new_current_thread()
					.enable_time()
					.build()
					.map_err(|_| UNAVAILABLE)?;
				runtime.block_on(async {
					if worker_stop.load(Ordering::Acquire) || send.is_closed() {
						return Ok(());
					}
					let audio = Audio::start(worker_stop.clone(), ready.clone(), epoch)?;
					while !worker_stop.load(Ordering::Acquire) && !send.is_closed() {
						audio.pump(&send, &ready, &worker_stop)?;
						let _ = tokio::time::timeout(
							std::time::Duration::from_millis(100),
							audio.changed.notified(),
						)
						.await;
					}
					Ok(())
				})
			})
			.map_err(|_| UNAVAILABLE)?;
		Ok(Self {
			thread: Some(thread),
			stop,
		})
	}
	pub(super) fn result(&mut self) -> Option<Result<(), &'static str>> {
		if !self.thread.as_ref()?.is_finished() {
			return None;
		}
		Some(self.thread.take()?.join().unwrap_or(Err(UNAVAILABLE)))
	}
}

impl Drop for Worker {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Release);
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

pub(super) struct Audio {
	pipeline: gst::Pipeline,
	source: gst::Element,
	sink: app::AppSink,
	failed: Arc<AtomicBool>,
	check_device: Arc<AtomicBool>,
	pub changed: Arc<tokio::sync::Notify>,
	monitor: bool,
}

impl Drop for Audio {
	fn drop(&mut self) {
		let _ = self.pipeline.set_state(gst::State::Null);
	}
}

impl Audio {
	pub(super) fn start(
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		epoch: Arc<AtomicU64>,
	) -> Result<Self, &'static str> {
		// ponytail: whole default output; per-application routing needs a separate picker.
		// Never use pulsesrc's default device: that is usually a microphone.
		let source = gst::ElementFactory::make("pulsesrc")
			.property("device", "@DEFAULT_MONITOR@")
			.property("client-name", "Serein screen audio")
			.property("buffer-time", 40_000i64)
			.property("latency-time", 10_000i64)
			.build()
			.map_err(|_| UNAVAILABLE)?;
		Self::new(source, stop, ready, epoch, true)
	}

	/// The offline example supplies audiotestsrc; production always requires a monitor.
	pub(super) fn new(
		source: gst::Element,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		epoch: Arc<AtomicU64>,
		monitor: bool,
	) -> Result<Self, &'static str> {
		let pipeline = gst::Pipeline::new();
		let sink = app::AppSink::builder()
			.caps(
				&gst::Caps::builder("audio/x-raw")
					.field("format", "F32LE")
					.field("layout", "interleaved")
					.field("rate", 48_000i32)
					.field("channels", 2i32)
					.field("channel-mask", gst::Bitmask::new(3))
					.build(),
			)
			.max_buffers(4)
			.drop(true)
			.sync(false)
			.async_(false)
			.enable_last_sample(false)
			.wait_on_eos(false)
			.build();
		let failed = Arc::new(AtomicBool::new(false));
		let changed = Arc::new(tokio::sync::Notify::new());
		let check_device = Arc::new(AtomicBool::new(monitor));
		pipeline.bus().ok_or(UNAVAILABLE)?.set_sync_handler({
			let failed = failed.clone();
			let changed = changed.clone();
			move |_, message| {
				if matches!(
					message.view(),
					gst::MessageView::Error(_) | gst::MessageView::Eos(_)
				) {
					failed.store(true, Ordering::Release);
					changed.notify_one();
				}
				gst::BusSyncReply::Drop
			}
		});
		if monitor {
			source.connect_notify(Some("current-device"), {
				let check_device = check_device.clone();
				move |_, _| {
					check_device.store(true, Ordering::Release);
				}
			});
		}
		sink.static_pad("sink")
			.ok_or(UNAVAILABLE)?
			.add_probe(gst::PadProbeType::BUFFER, {
				let failed = failed.clone();
				move |_, info| {
					let epoch = epoch.load(Ordering::Acquire);
					if info.buffer().is_none_or(|b| {
						b.size() > MAX_AUDIO_SAMPLES * 4 || !b.size().is_multiple_of(8)
					}) {
						failed.store(true, Ordering::Release);
						return gst::PadProbeReturn::Drop;
					}
					if stop.load(Ordering::Acquire) || !ready.load(Ordering::Acquire) {
						gst::PadProbeReturn::Drop
					} else {
						// This terminal sink's unused offset tags generation before queueing,
						// even if the worker misses a brief encryption transition.
						if let Some(buffer) = info.buffer_mut() {
							buffer.make_mut().set_offset(epoch);
						}
						gst::PadProbeReturn::Ok
					}
				}
			});
		sink.set_callbacks(
			app::AppSinkCallbacks::builder()
				.new_sample({
					let changed = changed.clone();
					move |_| {
						changed.notify_one();
						Ok(gst::FlowSuccess::Ok)
					}
				})
				.build(),
		);
		pipeline
			.add_many([&source, sink.upcast_ref()])
			.map_err(|_| UNAVAILABLE)?;
		source.link(&sink).map_err(|_| UNAVAILABLE)?;
		let audio = Self {
			pipeline,
			source,
			sink,
			failed,
			check_device,
			changed,
			monitor,
		};
		audio
			.pipeline
			.set_state(gst::State::Playing)
			.map_err(|_| UNAVAILABLE)?;
		Ok(audio)
	}

	pub(super) fn pump(
		&self,
		send: &tokio::sync::mpsc::Sender<AudioChunk>,
		ready: &AtomicBool,
		stop: &AtomicBool,
	) -> Result<(), &'static str> {
		if self.failed.load(Ordering::Acquire) {
			return Err(UNAVAILABLE);
		}
		for _ in 0..4 {
			let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::ZERO) else {
				break;
			};
			if !ready.load(Ordering::Acquire)
				|| stop.load(Ordering::Acquire)
				|| send.capacity() == 0
			{
				continue;
			}
			// Wait for the first buffer before checking the asynchronously opened device.
			// A desktop mixer may move a recording stream; refuse non-monitor destinations.
			if self.monitor && self.check_device.swap(false, Ordering::AcqRel) {
				let name = self
					.source
					.property::<Option<String>>("current-device")
					.ok_or(UNAVAILABLE)?;
				if name.len() > 1024 || !name.ends_with(".monitor") {
					return Err(UNAVAILABLE);
				}
			}
			let buffer = sample.buffer().ok_or(UNAVAILABLE)?;
			if buffer.size() > MAX_AUDIO_SAMPLES * 4 || !buffer.size().is_multiple_of(8) {
				return Err(UNAVAILABLE);
			}
			let bytes = buffer.map_readable().map_err(|_| UNAVAILABLE)?;
			let samples = bytes
				.as_chunks::<4>()
				.0
				.iter()
				.map(|b| {
					let value = f32::from_le_bytes(*b);
					if value.is_finite() {
						value.clamp(-1.0, 1.0)
					} else {
						0.0
					}
				})
				.collect::<Vec<_>>();
			if !samples.is_empty() && ready.load(Ordering::Acquire) && !stop.load(Ordering::Acquire)
			{
				let _ = send.try_send(AudioChunk {
					samples,
					epoch: buffer.offset(),
				});
			}
		}
		Ok(())
	}
}
