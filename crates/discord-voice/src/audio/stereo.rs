//! Preserve the first two native input channels; resampling uses preallocated frames.
use super::{Capture, Frame, Gate, StereoFrame, amplify};
use std::sync::atomic::Ordering;

pub(super) struct InputCapture {
	mono: Capture,
	stereo: Option<StereoCapture>,
}
impl InputCapture {
	pub fn new(
		rate: u32,
		mono: rtrb::Producer<Frame>,
		stereo: rtrb::Producer<StereoFrame>,
		enabled: bool,
	) -> Self {
		Self {
			mono: Capture::new(rate, mono),
			stereo: enabled.then(|| StereoCapture::new(rate, stereo)),
		}
	}
	pub fn process<T: cpal::SizedSample>(&mut self, data: &[T], channels: usize, gate: &Gate)
	where
		f32: cpal::FromSample<T>,
	{
		if let Some(stereo) = &mut self.stereo {
			stereo.process(data, channels, gate);
		} else {
			self.mono.process(data, channels, gate);
		}
	}
}
struct StereoCapture {
	generation: u64,
	previous: Option<[f32; 2]>,
	phase: f64,
	step: f64,
	frame: StereoFrame,
	index: usize,
	output: rtrb::Producer<StereoFrame>,
}
impl StereoCapture {
	fn new(rate: u32, output: rtrb::Producer<StereoFrame>) -> Self {
		Self {
			generation: 0,
			previous: None,
			phase: 0.0,
			step: f64::from(rate) / 48_000.0,
			frame: [0.0; 1920],
			index: 0,
			output,
		}
	}
	fn reset(&mut self) {
		self.previous = None;
		self.phase = 0.0;
		self.index = 0;
		self.frame.fill(0.0);
	}
	fn process<T: cpal::SizedSample>(&mut self, data: &[T], channels: usize, gate: &Gate)
	where
		f32: cpal::FromSample<T>,
	{
		let generation = gate.capture_generation.load(Ordering::Acquire);
		if generation != self.generation {
			self.generation = generation;
			self.reset();
		}
		if channels < 2 || !gate.capture() {
			self.reset();
			return;
		}
		for input in data.chunks_exact(channels) {
			let samples = [
				amplify(input[0].to_sample::<f32>(), 1.0),
				amplify(input[1].to_sample::<f32>(), 1.0),
			];
			if let Some(previous) = self.previous {
				while self.phase < 1.0 {
					for channel in 0..2 {
						self.frame[self.index * 2 + channel] = previous[channel]
							+ (samples[channel] - previous[channel]) * self.phase as f32;
					}
					self.index += 1;
					if self.index == 960 {
						if self.output.push(self.frame).is_err() {
							gate.echo_reset.store(true, Ordering::Release);
						}
						self.index = 0;
					}
					self.phase += self.step;
				}
				self.phase -= 1.0;
			}
			self.previous = Some(samples);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn stereo_resampling_preserves_duration_and_both_time_varying_channels() {
		for rate in [44_100, 48_000, 96_000] {
			let (producer, mut receive) = rtrb::RingBuffer::new(8);
			let mut capture = StereoCapture::new(rate, producer);
			let gate = Gate::default();
			gate.ready.store(true, Ordering::Release);
			gate.acknowledged_revision.store(1, Ordering::Release);
			let mut output_samples = 0usize;
			let mut samples = Vec::with_capacity(137 * 3);
			for first in (0..rate as usize + 2).step_by(137) {
				samples.clear();
				for index in first..(first + 137).min(rate as usize + 2) {
					let t = index as f32 / rate as f32;
					samples.extend([0.6 * t, -0.3 + 0.2 * t, 0.9]);
				}
				capture.process(&samples, 3, &gate);
				while let Ok(frame) = receive.pop() {
					for pair in frame.as_chunks::<2>().0.iter() {
						let t = output_samples as f32 / 48_000.0;
						assert!(
							(pair[0] - 0.6 * t).abs() < 0.00002,
							"left timing at {rate}/{output_samples}"
						);
						assert!(
							(pair[1] - (-0.3 + 0.2 * t)).abs() < 0.00002,
							"right timing at {rate}/{output_samples}"
						);
						output_samples += 1;
					}
				}
			}
			assert_eq!(output_samples, 48_000, "one second at {rate}");
		}
	}
	#[test]
	fn stereo_capture_preserves_opposite_channels_and_resets_partial_pcm() {
		for rate in [44_100, 48_000, 96_000] {
			let (producer, mut receive) = rtrb::RingBuffer::new(8);
			let mut capture = StereoCapture::new(rate, producer);
			let gate = Gate::default();
			gate.ready.store(true, Ordering::Release);
			gate.acknowledged_revision.store(1, Ordering::Release);
			let mut samples = Vec::new();
			for _ in 0..rate / 50 + 2 {
				samples.extend([0.5f32, -0.5, 0.9]);
			}
			capture.process(&samples, 3, &gate);
			let frame = receive.pop().unwrap();
			assert!(
				frame
					.as_chunks::<2>()
					.0
					.iter()
					.all(|sample| *sample == [0.5, -0.5])
			);
			while receive.pop().is_ok() {}
			capture.process(&[0.75f32, -0.75].repeat(100), 2, &gate);
			gate.capture_generation.fetch_add(1, Ordering::AcqRel);
			capture.process(&[0.25f32, -0.25].repeat(rate as usize / 50 + 2), 2, &gate);
			let frame = receive.pop().unwrap();
			assert!(
				frame
					.as_chunks::<2>()
					.0
					.iter()
					.all(|sample| *sample == [0.25, -0.25])
			);
			gate.muted.store(true, Ordering::Release);
			capture.process(&samples, 3, &gate);
			assert!(receive.pop().is_err());
		}
	}
}
