//! One second of PCM ahead of playback; decoder reads drive bounded HTTP ranges.
use super::*;
use cpal::traits::{DeviceTrait, StreamTrait};
use rtrb::{Consumer, RingBuffer};

struct Playback {
	frames: Consumer<[f32; 2]>,
	current: Option<[f32; 2]>,
	phase: f64,
	position: f64,
	rate: u32,
	eof: Arc<AtomicBool>,
	finished: Arc<AtomicBool>,
}

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
	selected: Option<&str>,
) -> Result<cpal::Stream, &'static str> {
	let host = cpal::default_host();
	let device = crate::media_output::choose(&host, selected)?;
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

pub(super) fn play(
	request: &Request,
	gate: &Arc<Gate>,
	wake: &Arc<Notify>,
	runtime: &tokio::runtime::Handle,
	publish: &impl Fn(Status),
) -> Result<(), &'static str> {
	let mut target = 0;
	loop {
		let current = || {
			gate.current(request.generation) && gate.seek_millis.load(Ordering::Acquire) == NO_SEEK
		};
		if !gate.current(request.generation) {
			return Ok(());
		}
		gate.failed.store(false, Ordering::Release);
		gate.buffering.store(true, Ordering::Release);
		let mut duration = request.duration.min(Duration::from_secs(MAX_SECONDS));
		publish(Status {
			state: State::Loading,
			position: Duration::from_millis(target),
			duration,
		});
		let mut producer = None;
		let mut stream = None;
		let eof = Arc::new(AtomicBool::new(false));
		let finished = Arc::new(AtomicBool::new(false));
		let mut total_frames = 0u64;
		let mut rate = 0;
		let wait = || {
			runtime.block_on(async {
			tokio::select! { _ = wake.notified() => {}, _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
		})
		};
		let result = super::source::source(request, gate.clone(), wake.clone(), runtime.clone())
			.and_then(|source| {
				super::decode_stream(
					source,
					&current,
					&mut |samples, channels, sample_rate, known_duration| {
						rate = sample_rate;
						gate.sample_rate.store(rate, Ordering::Release);
						if let Some(known) = known_duration {
							duration = known;
						}
						let target_frame = target.saturating_mul(u64::from(rate)) / 1000;
						let packet_frames = (samples.len() / channels) as u64;
						let skip =
							target_frame.saturating_sub(total_frames).min(packet_frames) as usize;
						total_frames += packet_frames;
						if skip == packet_frames as usize {
							return Ok(());
						}
						if producer.is_none() {
							let (sender, frames) = RingBuffer::new(rate as usize);
							gate.position_frames.store(target_frame, Ordering::Release);
							stream = Some(open_output(
								Playback {
									frames,
									current: None,
									phase: 0.0,
									position: target_frame as f64,
									rate,
									eof: eof.clone(),
									finished: finished.clone(),
								},
								gate.clone(),
								request.generation,
								request.output.as_deref(),
							)?);
							producer = Some(sender);
							publish(Status {
								state: State::Playing,
								position: Duration::from_millis(target),
								duration,
							});
						}
						let producer = producer.as_mut().expect("initialized above");
						for frame in samples[skip * channels..].chunks_exact(channels) {
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
					},
				)
			});
		if result.is_ok() {
			eof.store(true, Ordering::Release);
			while stream.is_some() && !finished.load(Ordering::Acquire) && current() {
				if gate.failed.load(Ordering::Acquire) {
					return Err("Audio output disconnected");
				}
				wait();
			}
		}
		drop(stream);
		if !gate.current(request.generation) {
			return Ok(());
		}
		let seek = gate.seek_millis.swap(NO_SEEK, Ordering::AcqRel);
		if seek != NO_SEEK {
			// ponytail: seeking replays decoding from the start; add a bounded index if long-clip seeking matters.
			target = seek;
			continue;
		}
		result?;
		let duration = Duration::from_secs_f64(total_frames as f64 / f64::from(rate.max(1)));
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
