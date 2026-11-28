//! One second of PCM ahead of playback; decoder reads drive attachment ranges or SoundCloud
//! segments. Output starts after a short pre-roll and, after an underrun, waits for a re-buffer
//! threshold, so a slow network pauses audio once instead of stuttering.
use super::*;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, RingBuffer};

struct Playback {
	frames: Consumer<[f32; 2]>,
	current: Option<[f32; 2]>,
	phase: f64,
	position: f64,
	rate: u32,
	eof: Arc<AtomicBool>,
	finished: Arc<AtomicBool>,
	/// Set by an underrun; cleared once `REBUFFER_FRAMES` are queued again or decoding ended.
	rebuffering: bool,
}

/// Queued audio required to start or resume output, in fractions of a second.
const PREROLL_DIVISOR: u32 = 4;
const REBUFFER_DIVISOR: u32 = 5;

impl Playback {
	fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(
		&mut self,
		data: &mut [T],
		output_channels: usize,
		output_rate: u32,
		gate: &Gate,
		generation: u64,
	) {
		data.fill(T::from_sample(0.0));
		if !gate.current(generation)
			|| gate.paused.load(Ordering::Acquire)
			|| gate.seek_millis.load(Ordering::Acquire) != NO_SEEK
		{
			return;
		}
		if self.rebuffering {
			if !self.eof.load(Ordering::Acquire)
				&& self.frames.slots() < (self.rate / REBUFFER_DIVISOR) as usize
			{
				gate.buffering.store(true, Ordering::Release);
				return;
			}
			self.rebuffering = false;
		}
		let volume = f32::from_bits(gate.volume.load(Ordering::Acquire));
		for output in data.chunks_exact_mut(output_channels) {
			// Observe EOF before reading the queue, so its final frames are visible before ending.
			let eof = self.eof.load(Ordering::Acquire);
			if self.current.is_none() {
				self.current = self.frames.pop().ok();
			}
			while self.phase >= 1.0 && self.current.is_some() {
				match self.frames.pop() {
					Ok(frame) => {
						self.current = Some(frame);
						self.phase -= 1.0;
					}
					Err(_) if eof => {
						self.current = None;
					}
					Err(_) => {
						gate.buffering.store(true, Ordering::Release);
						self.rebuffering = true;
						return;
					}
				}
			}
			let Some(left) = self.current else {
				self.finished.store(eof, Ordering::Release);
				gate.buffering.store(!eof, Ordering::Release);
				return;
			};
			let right = match self.frames.peek() {
				Ok(frame) => *frame,
				Err(_) if eof => left,
				Err(_) => {
					gate.buffering.store(true, Ordering::Release);
					self.rebuffering = true;
					return;
				}
			};
			let stereo = std::array::from_fn::<_, 2, _>(|i| {
				(left[i] + (right[i] - left[i]) * self.phase as f32) * volume
			});
			if output_channels == 1 {
				output[0] = T::from_sample((stereo[0] + stereo[1]) * 0.5);
			} else {
				output[0] = T::from_sample(stereo[0]);
				output[1] = T::from_sample(stereo[1]);
			}
			let step = f64::from(self.rate) / f64::from(output_rate);
			self.phase += step;
			self.position += step;
			gate.position_frames
				.store(self.position as u64, Ordering::Release);
			gate.buffering.store(false, Ordering::Release);
		}
	}
}

fn open_output(
	playback: Playback,
	gate: Arc<Gate>,
	generation: u64,
) -> Result<cpal::Stream, &'static str> {
	let host = cpal::default_host();
	let device = host
		.default_output_device()
		.ok_or("No audio output device")?;
	let supported = device
		.default_output_config()
		.map_err(|_| "Audio output unavailable")?;
	let config = playback_config(host.id(), supported.config());
	if !(1..=8).contains(&config.channels) || !(8000..=192000).contains(&config.sample_rate) {
		return Err("Unsupported audio output format");
	}
	let stream = match supported.sample_format() {
		cpal::SampleFormat::F32 => output::<f32>(&device, &config, playback, gate, generation),
		cpal::SampleFormat::I16 => output::<i16>(&device, &config, playback, gate, generation),
		cpal::SampleFormat::I32 => output::<i32>(&device, &config, playback, gate, generation),
		cpal::SampleFormat::U16 => output::<u16>(&device, &config, playback, gate, generation),
		_ => return Err("Unsupported audio output format"),
	}
	.map_err(|_| "Audio output unavailable")?;
	stream.play().map_err(|_| "Could not start audio output")?;
	Ok(stream)
}

fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
	device: &cpal::Device,
	config: &cpal::StreamConfig,
	mut playback: Playback,
	gate: Arc<Gate>,
	generation: u64,
) -> Result<cpal::Stream, cpal::Error> {
	let channels = usize::from(config.channels);
	let rate = config.sample_rate;
	let errors = gate.clone();
	device.build_output_stream(
		*config,
		move |data: &mut [T], _| playback.render(data, channels, rate, &gate, generation),
		move |_| {
			if errors.current(generation) {
				errors.failed.store(true, Ordering::Release);
			}
		},
		None,
	)
}

/// PCM hand-off shared by the attachment and SoundCloud decoders: it drops the frames before
/// the seek target, queues one second ahead and opens the output once the pre-roll is buffered.
pub(super) struct Feed<'a, P: Fn(Status)> {
	gate: &'a Arc<Gate>,
	wake: &'a Arc<Notify>,
	runtime: &'a tokio::runtime::Handle,
	publish: &'a P,
	generation: u64,
	/// The seek target this pass starts from, in milliseconds.
	pub(super) target: u64,
	/// Frames of the clip before the first one this pass decodes: a SoundCloud pass starts at
	/// the segment holding its target.
	pub(super) origin: u64,
	pub(super) rate: u32,
	pub(super) duration: Duration,
	pub(super) total_frames: u64,
	/// Built with the ring; output opens once enough audio is queued.
	producer: Option<Producer<[f32; 2]>>,
	pending: Option<Playback>,
	stream: Option<cpal::Stream>,
	eof: Arc<AtomicBool>,
	finished: Arc<AtomicBool>,
}
impl<'a, P: Fn(Status)> Feed<'a, P> {
	#[allow(clippy::too_many_arguments)]
	pub(super) fn new(
		gate: &'a Arc<Gate>,
		wake: &'a Arc<Notify>,
		runtime: &'a tokio::runtime::Handle,
		publish: &'a P,
		generation: u64,
		target: u64,
		origin: u64,
		duration: Duration,
	) -> Self {
		Self {
			gate,
			wake,
			runtime,
			publish,
			generation,
			target,
			origin,
			rate: 0,
			duration,
			total_frames: 0,
			producer: None,
			pending: None,
			stream: None,
			eof: Arc::new(AtomicBool::new(false)),
			finished: Arc::new(AtomicBool::new(false)),
		}
	}
	/// The clip is still current and no seek is waiting to restart it.
	pub(super) fn current(&self) -> bool {
		self.gate.current(self.generation)
			&& self.gate.seek_millis.load(Ordering::Acquire) == NO_SEEK
	}

	pub(super) fn generation(&self) -> u64 {
		self.generation
	}
	fn wait(&self) {
		self.runtime.block_on(async {
			tokio::select! { _ = self.wake.notified() => {}, _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
		})
	}
	/// Opens the output for the audio queued so far.
	fn open(&mut self) -> Result<(), &'static str> {
		if let Some(playback) = self.pending.take() {
			self.stream = Some(open_output(playback, self.gate.clone(), self.generation)?);
			(self.publish)(Status {
				state: State::Playing,
				position: Duration::from_millis(self.target),
				duration: self.duration,
			});
		}
		Ok(())
	}
	pub(super) fn push(
		&mut self,
		samples: &[f32],
		channels: usize,
		sample_rate: u32,
		known_duration: Option<Duration>,
	) -> Result<(), &'static str> {
		self.rate = sample_rate;
		self.gate.sample_rate.store(self.rate, Ordering::Release);
		if let Some(known) = known_duration {
			self.duration = known;
		}
		let target_frame = self.target.saturating_mul(u64::from(self.rate)) / 1000;
		let packet_frames = (samples.len() / channels) as u64;
		let before = self.origin.saturating_add(self.total_frames);
		let skip = target_frame.saturating_sub(before).min(packet_frames) as usize;
		self.total_frames += packet_frames;
		if skip == packet_frames as usize {
			return Ok(());
		}
		if self.producer.is_none() {
			let (sender, frames) = RingBuffer::new(self.rate as usize);
			let start = before + skip as u64;
			self.gate.position_frames.store(start, Ordering::Release);
			self.pending = Some(Playback {
				frames,
				current: None,
				phase: 0.0,
				position: start as f64,
				rate: self.rate,
				eof: self.eof.clone(),
				finished: self.finished.clone(),
				rebuffering: false,
			});
			self.producer = Some(sender);
		}
		let producer = self.producer.as_mut().expect("initialized above");
		pump(
			producer,
			&mut self.pending,
			&mut self.stream,
			self.gate,
			self.wake,
			self.runtime,
			self.generation,
			self.target,
			self.duration,
			self.publish,
			samples,
			skip,
			channels,
		)?;
		let queued = self.rate as usize - producer.slots();
		if self.stream.is_none() && queued >= (self.rate / PREROLL_DIVISOR) as usize {
			self.open()?;
		}
		Ok(())
	}
	/// Decoding ended: play what was queued and wait for the output to drain.
	pub(super) fn finish(&mut self) -> Result<(), &'static str> {
		self.eof.store(true, Ordering::Release);
		// A clip shorter than the pre-roll still plays.
		if self.stream.is_none() && self.pending.is_some() {
			self.open()?;
		}
		while self.stream.is_some() && !self.finished.load(Ordering::Acquire) && self.current() {
			if self.gate.failed.load(Ordering::Acquire) {
				return Err("Audio output disconnected");
			}
			self.wait();
		}
		Ok(())
	}
}

/// Queues one decoded packet, waiting for room or for a pause to end. A ring that fills is
/// past the pre-roll, so the output opens before waiting on it.
#[allow(clippy::too_many_arguments)]
fn pump(
	producer: &mut Producer<[f32; 2]>,
	pending: &mut Option<Playback>,
	stream: &mut Option<cpal::Stream>,
	gate: &Arc<Gate>,
	wake: &Arc<Notify>,
	runtime: &tokio::runtime::Handle,
	generation: u64,
	target: u64,
	duration: Duration,
	publish: &impl Fn(Status),
	samples: &[f32],
	skip: usize,
	channels: usize,
) -> Result<(), &'static str> {
	let current =
		|| gate.current(generation) && gate.seek_millis.load(Ordering::Acquire) == NO_SEEK;
	let wait = || {
		runtime.block_on(async {
			tokio::select! { _ = wake.notified() => {}, _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
		})
	};
	for frame in samples[skip * channels..].chunks_exact(channels) {
		// A full ring is past the pre-roll; open output before waiting on it.
		if producer.slots() == 0
			&& stream.is_none()
			&& let Some(playback) = pending.take()
		{
			*stream = Some(open_output(playback, gate.clone(), generation)?);
			publish(Status {
				state: State::Playing,
				position: Duration::from_millis(target),
				duration,
			});
		}
		while gate.paused.load(Ordering::Acquire) || producer.slots() == 0 {
			if !current() {
				return Err("Cancelled");
			}
			if gate.failed.load(Ordering::Acquire) {
				return Err("Audio output disconnected");
			}
			wait();
		}
		if !current() {
			return Err("Cancelled");
		}
		let clean = |sample: f32| {
			if sample.is_finite() {
				sample.clamp(-1.0, 1.0)
			} else {
				0.0
			}
		};
		producer
			.push([clean(frame[0]), clean(frame[channels - 1])])
			.map_err(|_| INVALID)?;
	}
	Ok(())
}

pub(super) fn play(
	request: &Request,
	gate: &Arc<Gate>,
	wake: &Arc<Notify>,
	runtime: &tokio::runtime::Handle,
	publish: &impl Fn(Status),
) -> Result<(), &'static str> {
	if request.soundcloud.is_some() {
		return super::soundcloud::play(request, gate, wake, runtime, publish);
	}
	let mut target = 0;
	// A pass learns the clip's length as it decodes; a pass restarted by a seek starts with
	// what the last one knew.
	let mut duration = request.duration.min(Duration::from_secs(MAX_SECONDS));
	loop {
		if !gate.current(request.generation) {
			return Ok(());
		}
		gate.failed.store(false, Ordering::Release);
		gate.buffering.store(true, Ordering::Release);
		publish(Status {
			state: State::Loading,
			position: Duration::from_millis(target),
			duration,
		});
		let mut feed = Feed::new(
			gate,
			wake,
			runtime,
			publish,
			request.generation,
			target,
			0,
			duration,
		);
		let current = || {
			gate.current(request.generation) && gate.seek_millis.load(Ordering::Acquire) == NO_SEEK
		};
		let result = super::source::source(request, gate.clone(), wake.clone(), runtime.clone())
			.and_then(|source| {
				super::decode_stream(
					source,
					&current,
					&mut |samples, channels, sample_rate, known_duration| {
						feed.push(samples, channels, sample_rate, known_duration)
					},
				)
			});
		if result.is_ok() {
			feed.finish()?;
		}
		let (total_frames, rate, known) = (feed.total_frames, feed.rate, feed.duration);
		drop(feed);
		if !gate.current(request.generation) {
			return Ok(());
		}
		let seek = gate.seek_millis.swap(NO_SEEK, Ordering::AcqRel);
		if seek != NO_SEEK {
			// ponytail: seeking replays decoding from the start; add a bounded index if long-clip seeking matters.
			target = seek;
			duration = known;
			continue;
		}
		result?;
		duration = Duration::from_secs_f64(total_frames as f64 / f64::from(rate.max(1)));
		publish(Status {
			state: State::Ended,
			position: duration,
			duration,
		});
		return Ok(());
	}
}

#[cfg(all(debug_assertions, feature = "demo"))]
pub(super) fn debug_check() {
	let gate = Gate::default();
	let (mut sender, frames) = RingBuffer::new(8);
	let eof = Arc::new(AtomicBool::new(false));
	let finished = Arc::new(AtomicBool::new(false));
	let mut playback = Playback {
		frames,
		current: None,
		phase: 0.0,
		position: 0.0,
		rate: 48000,
		eof: eof.clone(),
		finished: finished.clone(),
		rebuffering: false,
	};
	let mut output = [1.0f32; 4];
	playback.render(&mut output, 2, 48000, &gate, 0);
	assert_eq!(output, [0.0; 4]);
	assert_eq!(gate.position_frames.load(Ordering::Acquire), 0);
	for _ in 0..8 {
		sender.push([0.5; 2]).unwrap();
	}
	assert_eq!(sender.slots(), 0);
	gate.paused.store(true, Ordering::Release);
	playback.render(&mut output, 2, 48000, &gate, 0);
	assert_eq!(sender.slots(), 0);
	gate.paused.store(false, Ordering::Release);
	playback.render(&mut output, 2, 48000, &gate, 0);
	assert_eq!(output, [0.5; 4]);
	assert_eq!(gate.position_frames.load(Ordering::Acquire), 2);
	let mut drain = [0.0f32; 32];
	playback.render(&mut drain, 2, 48000, &gate, 0);
	let stalled = gate.position_frames.load(Ordering::Acquire);
	playback.render(&mut drain, 2, 48000, &gate, 0);
	assert_eq!(gate.position_frames.load(Ordering::Acquire), stalled);
	assert!(gate.buffering.load(Ordering::Acquire));
	eof.store(true, Ordering::Release);
	playback.render(&mut drain, 2, 48000, &gate, 0);
	assert!(finished.load(Ordering::Acquire));
	assert_eq!(gate.position_frames.load(Ordering::Acquire), 8);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_underrun_resumes_only_after_the_rebuffer_threshold() {
		// 20 Hz in and out: one frame per output frame, threshold rate / 5 = 4 frames.
		let gate = Gate::default();
		let (mut sender, frames) = RingBuffer::new(16);
		let eof = Arc::new(AtomicBool::new(false));
		let mut playback = Playback {
			frames,
			current: None,
			phase: 0.0,
			position: 0.0,
			rate: 20,
			eof: eof.clone(),
			finished: Arc::new(AtomicBool::new(false)),
			rebuffering: false,
		};
		let mut output = [1.0f32; 4];
		sender.push([0.5; 2]).unwrap();
		playback.render(&mut output, 2, 20, &gate, 0);
		assert!(playback.rebuffering && gate.buffering.load(Ordering::Acquire));
		for _ in 0..3 {
			sender.push([0.5; 2]).unwrap();
		}
		playback.render(&mut output, 2, 20, &gate, 0);
		assert_eq!(output, [0.0; 4], "three frames are below the threshold");
		sender.push([0.5; 2]).unwrap();
		playback.render(&mut output, 2, 20, &gate, 0);
		assert!(!playback.rebuffering);
		assert_eq!(output, [0.5; 4]);
		// The end of decoding releases a short tail without waiting for the threshold.
		let mut drain = [0.0f32; 32];
		playback.render(&mut drain, 2, 20, &gate, 0);
		assert!(playback.rebuffering);
		sender.push([0.25; 2]).unwrap();
		eof.store(true, Ordering::Release);
		playback.render(&mut output, 2, 20, &gate, 0);
		assert!(!playback.rebuffering);
		// The frame held across the underrun plays first, then the tail.
		assert_eq!(output, [0.5, 0.5, 0.25, 0.25]);
	}
}
