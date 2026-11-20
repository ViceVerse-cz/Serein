use deepfilternet::{DeepFilterNet, FRAME};

/// A sawtooth "voice" through two formant resonators under a 4 Hz syllable envelope, in
/// white noise. Integer and basic IEEE arithmetic only, so every platform builds the same
/// samples.
struct Voice {
	sample: u32,
	seed: u32,
	formants: [[f32; 2]; 2],
}
impl Voice {
	fn new() -> Self {
		Self {
			sample: 0,
			seed: 17,
			formants: [[0.0; 2]; 2],
		}
	}
	fn frame(&mut self, voiced: bool) -> [f32; FRAME] {
		std::array::from_fn(|_| {
			self.sample += 1;
			self.seed = self
				.seed
				.wrapping_mul(1_664_525)
				.wrapping_add(1_013_904_223);
			let saw = (self.sample * 110 % 48_000) as f32 / 24_000.0 - 1.0;
			let mut voice = 0.0;
			// Resonances near 500 Hz and 1.9 kHz.
			for (memory, (a1, a2)) in self
				.formants
				.iter_mut()
				.zip([(1.955_803, -0.9604), (1.880_308, -0.9409)])
			{
				let resonance = saw + a1 * memory[0] + a2 * memory[1];
				*memory = [resonance, memory[0]];
				voice += 0.002 * resonance;
			}
			let syllable = 1.0 - ((self.sample % 12_000) as f32 / 6_000.0 - 1.0).abs();
			let noise = self.seed as i32 as f32 / i32::MAX as f32 * 0.02;
			if voiced {
				syllable * voice + noise
			} else {
				noise
			}
		})
	}
}

/// `fixtures/reference.f32` holds frames 150..170 of this stream as enhanced by
/// DeepFilterNet's own runtime (libDF on tract 0.22.5, DeepFilterNet Plus `2dc5273d`, stage
/// thresholds -15/35/35 dB, no attenuation limit or post-filter). A wrong weight layout,
/// tap order or state update leaves an error near the signal level; a faithful port
/// differs by float rounding only.
#[test]
fn matches_the_reference_runtime() {
	let golden = include_bytes!("fixtures/reference.f32").as_chunks::<4>().0;
	let mut golden = golden.iter().map(|bytes| f32::from_le_bytes(*bytes));
	let mut voice = Voice::new();
	let mut filter = DeepFilterNet::new();
	let (mut error, mut power) = (0.0, 0.0);
	for tick in 0..170 {
		let mut frame = voice.frame(tick >= 50);
		filter.process(&mut frame);
		if tick >= 150 {
			for (actual, expected) in frame.iter().zip(&mut golden) {
				error += f64::from(actual - expected).powi(2);
				power += f64::from(expected).powi(2);
			}
		}
	}
	assert!(golden.next().is_none() && power > 1.0, "{power}");
	assert!(error < power * 1e-9, "{error} of {power}");
}

/// Look-ahead, window overlap, feature means and recurrent state all hold earlier
/// microphone audio; a reset must drop every one of them.
#[test]
fn reset_restarts_the_stream() {
	let mut voice = Voice::new();
	let mut used = DeepFilterNet::new();
	for _ in 0..30 {
		used.process(&mut voice.frame(true));
	}
	used.reset();
	let mut fresh = DeepFilterNet::new();
	for _ in 0..10 {
		let [mut first, mut second] = [voice.frame(true); 2];
		used.process(&mut first);
		fresh.process(&mut second);
		assert_eq!(first, second);
	}
}
