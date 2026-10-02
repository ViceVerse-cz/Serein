//! One second of decoded stereo PCM; the device callback only touches its ring and atomics.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

#[derive(Clone)]
pub struct Controls {
	pub cancelled: Arc<AtomicBool>,
	pub paused: Arc<AtomicBool>,
	pub seek: Arc<AtomicU64>,
	pub volume: Arc<AtomicU32>,
	pub position: Arc<AtomicU64>,
	pub eof: Arc<AtomicBool>,
	pub failed: Arc<AtomicBool>,
}
pub struct Output {
	pub producer: rtrb::Producer<[f32; 2]>,
	_stream: cpal::Stream,
}
struct Playback {
	frames: rtrb::Consumer<[f32; 2]>,
	current: Option<[f32; 2]>,
	phase: f64,
	read_frames: u64,
	rate: u32,
	controls: Controls,
}
impl Playback {
	fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(
		&mut self,
		data: &mut [T],
		channels: usize,
		rate: u32,
	) {
		data.fill(T::from_sample(0.0));
		if self.controls.cancelled.load(Ordering::Acquire)
			|| self.controls.paused.load(Ordering::Acquire)
			|| self.controls.seek.load(Ordering::Acquire) != u64::MAX
		{
			return;
		}
		let volume = f32::from_bits(self.controls.volume.load(Ordering::Acquire));
		let volume = if volume.is_finite() {
			volume.clamp(0.0, 1.0)
		} else {
			0.0
		};
		for output in data.chunks_exact_mut(channels) {
			// Acquire EOF before examining the queue to observe all final producer writes.
			let eof = self.controls.eof.load(Ordering::Acquire);
			if self.current.is_none() {
				self.current = self.frames.pop().ok();
				self.read_frames += u64::from(self.current.is_some());
			}
			while self.phase >= 1.0 && self.current.is_some() {
				match self.frames.pop() {
					Ok(frame) => {
						self.current = Some(frame);
						self.read_frames += 1;
						self.phase -= 1.0;
					}
					Err(_) if eof => self.current = None,
					Err(_) => return,
				}
			}
			let Some(left) = self.current else { return };
			let right = match self.frames.peek() {
				Ok(frame) => *frame,
				Err(_) if eof => left,
				Err(_) => return,
			};
			let step = f64::from(self.rate) / f64::from(rate);
			let available = 1.0 + self.frames.slots() as f64 - self.phase;
			if !eof && step > available {
				return;
			}
			let clean = |sample: f32| {
				if sample.is_finite() {
					sample.clamp(-1.0, 1.0)
				} else {
					0.0
				}
			};
			let stereo = std::array::from_fn::<_, 2, _>(|i| {
				(clean(left[i]) + (clean(right[i]) - clean(left[i])) * self.phase as f32) * volume
			});
			if channels == 1 {
				output[0] = T::from_sample((stereo[0] + stereo[1]) * 0.5);
			} else {
				output[0] = T::from_sample(stereo[0]);
				output[1] = T::from_sample(stereo[1]);
			}
			// The final device sample may span the end of a short source: do not overrun its clock.
			let final_frame = eof && step >= available;
			let step = if eof { step.min(available) } else { step };
			self.phase += step;
			// Count source frames exactly instead of accumulating a fractional device-rate clock.
			let position = if final_frame {
				self.read_frames + self.frames.slots() as u64
			} else {
				self.read_frames - 1 + self.phase as u64
			};
			self.controls.position.store(position, Ordering::Release);
		}
	}
}
pub fn open(rate: u32, controls: Controls) -> Result<Output, &'static str> {
	if !(8000..=96000).contains(&rate) {
		return Err("Unsupported video audio sample rate");
	}
	let host = cpal::default_host();
	let device = host
		.default_output_device()
		.ok_or("No audio output device")?;
	let supported = device
		.default_output_config()
		.map_err(|_| "Audio output unavailable")?;
	let config = crate::audio::playback_config(host.id(), supported.config());
	if !(1..=8).contains(&config.channels) || !(8000..=192000).contains(&config.sample_rate) {
		return Err("Unsupported audio output format");
	}
	let (producer, frames) = rtrb::RingBuffer::new(rate as usize);
	controls.position.store(0, Ordering::Release);
	let playback = Playback {
		frames,
		current: None,
		phase: 0.0,
		read_frames: 0,
		rate,
		controls,
	};
	let stream = match supported.sample_format() {
		cpal::SampleFormat::F32 => output::<f32>(&device, &config, playback),
		cpal::SampleFormat::I16 => output::<i16>(&device, &config, playback),
		cpal::SampleFormat::I32 => output::<i32>(&device, &config, playback),
		cpal::SampleFormat::U16 => output::<u16>(&device, &config, playback),
		_ => return Err("Unsupported audio output format"),
	}
	.map_err(|_| "Audio output unavailable")?;
	stream.play().map_err(|_| "Could not start audio output")?;
	Ok(Output {
		producer,
		_stream: stream,
	})
}
fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
	device: &cpal::Device,
	config: &cpal::StreamConfig,
	mut playback: Playback,
) -> Result<cpal::Stream, cpal::Error> {
	let channels = usize::from(config.channels);
	let rate = config.sample_rate;
	let controls = playback.controls.clone();
	device.build_output_stream(
		*config,
		move |data: &mut [T], _| playback.render(data, channels, rate),
		move |_| {
			if !controls.cancelled.load(Ordering::Acquire) {
				controls.failed.store(true, Ordering::Release);
			}
		},
		None,
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn video_pcm_clock_freezes_when_paused_cancelled_starved_or_ended() {
		for output_rate in [8000, 44100, 48000, 96000] {
			let controls = Controls {
				cancelled: Arc::new(AtomicBool::new(false)),
				paused: Arc::new(AtomicBool::new(false)),
				seek: Arc::new(AtomicU64::new(u64::MAX)),
				volume: Arc::new(AtomicU32::new(0.5f32.to_bits())),
				position: Arc::new(AtomicU64::new(0)),
				eof: Arc::new(AtomicBool::new(false)),
				failed: Arc::new(AtomicBool::new(false)),
			};
			let (mut producer, frames) = rtrb::RingBuffer::new(8);
			let mut playback = Playback {
				frames,
				current: None,
				phase: 0.0,
				read_frames: 0,
				rate: 48000,
				controls: controls.clone(),
			};
			let mut data = [1.0f32; 64];
			playback.render(&mut data, 2, output_rate);
			assert_eq!(data, [0.0; 64]);
			assert_eq!(controls.position.load(Ordering::Acquire), 0);
			for _ in 0..8 {
				producer.push([0.5; 2]).unwrap();
			}
			for flag in [&controls.paused, &controls.cancelled] {
				flag.store(true, Ordering::Release);
				playback.render(&mut data, 2, output_rate);
				assert_eq!(data, [0.0; 64]);
				assert_eq!(controls.position.load(Ordering::Acquire), 0);
				assert_eq!(producer.slots(), 0);
				flag.store(false, Ordering::Release);
			}
			playback.render(&mut data, 2, output_rate);
			assert_eq!(data[0], 0.25);
			let starved = controls.position.load(Ordering::Acquire);
			playback.render(&mut data, 2, output_rate);
			assert_eq!(data, [0.0; 64]);
			assert_eq!(controls.position.load(Ordering::Acquire), starved);
			controls.eof.store(true, Ordering::Release);
			playback.render(&mut data, 2, output_rate);
			assert_eq!(controls.position.load(Ordering::Acquire), 8);
			playback.render(&mut data, 2, output_rate);
			assert_eq!(data, [0.0; 64]);
			assert_eq!(controls.position.load(Ordering::Acquire), 8);
			// Three seconds plus a partial packet exercises the fractional 48k -> 44.1k clock.
			controls.eof.store(false, Ordering::Release);
			controls.position.store(0, Ordering::Release);
			let (mut producer, frames) = rtrb::RingBuffer::new(1024);
			let mut playback = Playback {
				frames,
				current: None,
				phase: 0.0,
				read_frames: 0,
				rate: 48000,
				controls: controls.clone(),
			};
			let mut queued = 0;
			let total = 144384;
			let mut block = [0.0f32; 2048];
			for _ in 0..512 {
				while queued < total && producer.push([0.5; 2]).is_ok() {
					queued += 1;
				}
				controls.eof.store(queued == total, Ordering::Release);
				playback.render(&mut block, 2, output_rate);
				if controls.position.load(Ordering::Acquire) == total {
					break;
				}
			}
			assert_eq!(controls.position.load(Ordering::Acquire), total);
			playback.render(&mut block, 2, output_rate);
			assert_eq!(block, [0.0; 2048]);
			assert_eq!(controls.position.load(Ordering::Acquire), total);
		}
	}
}
