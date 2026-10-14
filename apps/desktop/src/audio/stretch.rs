//! Pitch-preserving playback speed for attachments: WSOLA (waveform-similarity overlap-add).
//! Each output grain is the input window near its nominal position that best continues the
//! previous grain, cross-faded with a Hann window, so voices keep their pitch at 0.75–2×.

/// One stereo frame and its position in the source, in source frames.
pub(super) type Frame = ([f32; 2], f64);

pub(super) struct Stretch {
	speed: f64,
	/// Half a grain: the output hop and the cross-fade length.
	hop: usize,
	/// How far either side of the nominal position a grain may move to match the waveform.
	search: usize,
	window: Vec<f32>,
	input: Vec<[f32; 2]>,
	/// Source frame of `input[0]`.
	base: usize,
	/// Source frame where the next grain nominally starts.
	nominal: f64,
	previous: Option<usize>,
	tail: Vec<[f32; 2]>,
}

impl Stretch {
	pub(super) fn new(speed: f64, rate: u32, start: usize) -> Self {
		// 40 ms grains with a 20 ms hop, searched ±10 ms, suit speech at any common rate.
		let hop = (rate as usize / 50).max(16);
		let length = 2 * hop;
		let window = (0..length)
			.map(|i| {
				let x = std::f32::consts::PI * i as f32 / length as f32;
				x.sin() * x.sin()
			})
			.collect();
		Self {
			speed,
			hop,
			search: hop / 2,
			window,
			input: Vec::new(),
			base: start,
			nominal: start as f64,
			previous: None,
			tail: vec![[0.0; 2]; hop],
		}
	}

	/// Feeds one source frame; returns the stretched frames now ready, in order.
	pub(super) fn push(&mut self, frame: [f32; 2], output: &mut Vec<Frame>) {
		self.input.push(frame);
		let length = 2 * self.hop;
		loop {
			let nominal = self.nominal.round() as usize;
			let lowest = nominal.saturating_sub(self.search).max(self.base);
			let highest = nominal + self.search;
			// Enough input for the furthest candidate grain and the natural continuation.
			let continuation = self
				.previous
				.map_or(0, |previous| previous + self.hop + length);
			let needed = (highest + length).max(continuation);
			if needed > self.base + self.input.len() {
				return;
			}
			let chosen = match self.previous {
				None => nominal.max(self.base),
				Some(previous) => self.best(previous + self.hop, lowest, highest),
			};
			let start = chosen - self.base;
			for i in 0..self.hop {
				let sample = self.input[start + i];
				let weight = self.window[i];
				let mixed = std::array::from_fn(|c| self.tail[i][c] + sample[c] * weight);
				output.push((mixed, self.nominal + i as f64 * self.speed));
			}
			for i in 0..self.hop {
				let sample = self.input[start + self.hop + i];
				let weight = self.window[self.hop + i];
				self.tail[i] = std::array::from_fn(|c| sample[c] * weight);
			}
			self.previous = Some(chosen);
			self.nominal += self.hop as f64 * self.speed;
			// Keep only what the next grain and its search window can still read.
			let keep = (self.nominal.round() as usize)
				.saturating_sub(self.search)
				.min(chosen + self.hop);
			if keep > self.base {
				self.input.drain(..keep - self.base);
				self.base = keep;
			}
		}
	}

	/// The fade-out of the last grain, once the source has ended.
	pub(super) fn finish(&mut self, output: &mut Vec<Frame>) {
		if self.previous.is_none() {
			// Shorter than one grain: play it as it is.
			for (i, frame) in self.input.drain(..).enumerate() {
				output.push((frame, (self.base + i) as f64));
			}
			return;
		}
		{
			for (i, frame) in self.tail.iter().enumerate() {
				output.push((*frame, self.nominal + i as f64 * self.speed));
			}
		}
	}

	/// The candidate start in `lowest..=highest` whose opening best matches the waveform that
	/// would naturally follow the previous grain at `continuation`.
	fn best(&self, continuation: usize, lowest: usize, highest: usize) -> usize {
		let mono = |frame: [f32; 2]| frame[0] + frame[1];
		let at = |source: usize| mono(self.input[source - self.base]);
		let mut best = (f32::NEG_INFINITY, lowest);
		// Every other frame is enough to find the matching period of a voice.
		for candidate in (lowest..=highest).step_by(2) {
			let (mut dot, mut energy) = (0.0f32, 0.0f32);
			for i in (0..self.hop).step_by(2) {
				let sample = at(candidate + i);
				dot += sample * at(continuation + i);
				energy += sample * sample;
			}
			let score = dot / energy.max(1e-9).sqrt();
			if score > best.0 {
				best = (score, candidate);
			}
		}
		best.1
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn run(speed: f64, input: &[[f32; 2]]) -> Vec<Frame> {
		let mut stretch = Stretch::new(speed, 48000, 0);
		let mut output = Vec::new();
		for frame in input {
			stretch.push(*frame, &mut output);
		}
		stretch.finish(&mut output);
		output
	}

	#[test]
	fn speed_changes_length_but_keeps_the_pitch() {
		// One second of a 220 Hz tone.
		let tone: Vec<[f32; 2]> = (0..48000)
			.map(|i| {
				let s = (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 48000.0).sin() * 0.5;
				[s, s]
			})
			.collect();
		for speed in [0.75, 1.5, 2.0] {
			let output = run(speed, &tone);
			let expected = 48000.0 / speed;
			let length = output.len() as f64;
			assert!(
				(length - expected).abs() < 48000.0 * 0.06 / speed,
				"{speed}×: {length} frames, expected about {expected}"
			);
			// Zero crossings per second stay at about 440: the pitch did not move.
			let steady = &output[4800..output.len() - 4800];
			let crossings = steady
				.windows(2)
				.filter(|pair| (pair[0].0[0] < 0.0) != (pair[1].0[0] < 0.0))
				.count() as f64;
			let per_second = crossings * 48000.0 / steady.len() as f64;
			assert!(
				(per_second - 440.0).abs() < 25.0,
				"{speed}×: {per_second} crossings per second"
			);
			// Source positions advance at the playback speed and never run backwards.
			assert!(output.windows(2).all(|pair| pair[1].1 >= pair[0].1));
			let last = output.last().unwrap().1;
			assert!(
				(last - 48000.0).abs() < 48000.0 * 0.06,
				"{speed}×: ends at {last}"
			);
		}
	}

	#[test]
	fn short_input_still_drains_and_silence_stays_silent() {
		let short = run(1.5, &[[0.25; 2]; 100]);
		assert_eq!(short.len(), 100);
		assert!(
			run(1.5, &[[0.0; 2]; 4800])
				.iter()
				.all(|(s, _)| *s == [0.0; 2])
		);
		assert!(run(2.0, &[]).is_empty());
	}
}
