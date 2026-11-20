//! Framing, features and spectral gains around the network, ported from DeepFilterNet's
//! `libDF` for its fixed 48 kHz configuration.
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex, num_complex::Complex32};
use std::sync::Arc;

pub const HOP: usize = 480;
/// Bins of the 960-point transform.
pub const SPECTRUM: usize = HOP + 1;
pub const BANDS: usize = 32;
/// Lowest bins, which the network filters individually instead of by band gain.
pub const FILTERED: usize = 96;
pub type Spectrum = [Complex32; SPECTRUM];

/// Bins per ERB band: `libDF`'s filter bank for 48 kHz, 960 points and at least two bins.
const BAND_WIDTHS: [usize; BANDS] = [
	2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 5, 5, 7, 7, 8, 10, 12, 13, 15, 18, 20, 24, 28, 31, 37,
	42, 50, 56, 67,
];
/// Feature normalization decay per frame: one second, rounded as the model was trained.
const DECAY: f32 = 0.99;

/// Overlapping Vorbis-window transform, 20 ms window and 10 ms hop.
pub struct Stft {
	forward: Arc<dyn RealToComplex<f32>>,
	inverse: Arc<dyn ComplexToReal<f32>>,
	forward_scratch: Vec<Complex32>,
	inverse_scratch: Vec<Complex32>,
	window: Vec<f32>,
	samples: Vec<f32>,
	previous: [f32; HOP],
	overlap: [f32; HOP],
}
impl Stft {
	pub fn new() -> Self {
		let mut planner = RealFftPlanner::new();
		let forward = planner.plan_fft_forward(2 * HOP);
		let inverse = planner.plan_fft_inverse(2 * HOP);
		let window = (0..2 * HOP)
			.map(|i| {
				let sin = (std::f64::consts::FRAC_PI_2 * (i as f64 + 0.5) / HOP as f64).sin();
				(std::f64::consts::FRAC_PI_2 * sin * sin).sin() as f32
			})
			.collect();
		Self {
			forward_scratch: forward.make_scratch_vec(),
			inverse_scratch: inverse.make_scratch_vec(),
			samples: forward.make_input_vec(),
			forward,
			inverse,
			window,
			previous: [0.0; HOP],
			overlap: [0.0; HOP],
		}
	}
	pub fn reset(&mut self) {
		self.previous = [0.0; HOP];
		self.overlap = [0.0; HOP];
	}
	pub fn analysis(&mut self, input: &[f32; HOP], spectrum: &mut Spectrum) {
		let (old, new) = self.samples.split_at_mut(HOP);
		let (rise, fall) = self.window.split_at(HOP);
		for ((sample, previous), window) in old.iter_mut().zip(&self.previous).zip(rise) {
			*sample = previous * window;
		}
		for ((sample, input), window) in new.iter_mut().zip(input).zip(fall) {
			*sample = input * window;
		}
		self.previous = *input;
		// Every buffer has the planned length, the only thing the transform checks.
		let _ = self.forward.process_with_scratch(
			&mut self.samples,
			spectrum,
			&mut self.forward_scratch,
		);
		// All scaling happens here, so synthesis is a plain overlap-add.
		for bin in spectrum {
			*bin *= 1.0 / (2 * HOP) as f32;
		}
	}
	/// Overwrites `spectrum`, which the inverse transform uses as scratch space.
	pub fn synthesis(&mut self, spectrum: &mut Spectrum, output: &mut [f32; HOP]) {
		// A real signal has no imaginary part here; the transform rejects any other value.
		spectrum[0].im = 0.0;
		spectrum[HOP].im = 0.0;
		let _ = self.inverse.process_with_scratch(
			spectrum,
			&mut self.samples,
			&mut self.inverse_scratch,
		);
		for (sample, window) in self.samples.iter_mut().zip(&self.window) {
			*sample *= window;
		}
		let (now, next) = self.samples.split_at(HOP);
		for ((output, sample), overlap) in output.iter_mut().zip(now).zip(&self.overlap) {
			*output = sample + overlap;
		}
		self.overlap.copy_from_slice(next);
	}
}

/// Running means that normalize the network inputs.
pub struct Features {
	band_mean: [f32; BANDS],
	bin_magnitude: [f32; FILTERED],
}
impl Features {
	pub fn new() -> Self {
		Self {
			// The model's training started every stream from these ramps.
			band_mean: ramp(-60.0, -90.0),
			bin_magnitude: ramp(0.001, 0.0001),
		}
	}
	/// Normalized band energies in dB, and the lowest bins as unit-normalized real and
	/// imaginary parts.
	pub fn extract(
		&mut self,
		spectrum: &Spectrum,
		bands: &mut [f32; BANDS],
		bins: &mut [[f32; FILTERED]; 2],
	) {
		let mut start = 0;
		for ((band, mean), width) in bands.iter_mut().zip(&mut self.band_mean).zip(BAND_WIDTHS) {
			let energy = spectrum[start..start + width]
				.iter()
				.map(|bin| bin.norm_sqr() * (1.0 / width as f32))
				.sum::<f32>();
			start += width;
			let level = (energy + 1e-10).log10() * 10.0;
			*mean = level * (1.0 - DECAY) + *mean * DECAY;
			*band = (level - *mean) / 40.0;
		}
		let [real, imaginary] = bins;
		for (((bin, magnitude), real), imaginary) in spectrum
			.iter()
			.zip(&mut self.bin_magnitude)
			.zip(real)
			.zip(imaginary)
		{
			*magnitude = bin.norm() * (1.0 - DECAY) + *magnitude * DECAY;
			// The floor avoids 0/0 once sustained digital silence underflows the mean.
			let scale = magnitude.max(1e-10).sqrt();
			*real = bin.re / scale;
			*imaginary = bin.im / scale;
		}
	}
}

fn ramp<const N: usize>(first: f32, last: f32) -> [f32; N] {
	let step = (last - first) / (N - 1) as f32;
	std::array::from_fn(|i| first + i as f32 * step)
}

/// Scales every bin by the gain of its ERB band.
pub fn apply_band_gains(spectrum: &mut Spectrum, gains: &[f32; BANDS]) {
	let mut start = 0;
	for (gain, width) in gains.iter().zip(BAND_WIDTHS) {
		for bin in &mut spectrum[start..start + width] {
			*bin *= gain;
		}
		start += width;
	}
}

/// Attenuates bins the model already reduced strongly a little further (Valin et al., 2020).
pub fn post_filter(noisy: &Spectrum, enhanced: &mut Spectrum, beta: f32) {
	const FLOOR: f32 = 1e-12;
	for (noisy, enhanced) in noisy.iter().zip(enhanced) {
		let gain = (enhanced.norm() / (noisy.norm() + FLOOR)).clamp(FLOOR, 1.0);
		let sin = (gain * std::f32::consts::FRAC_PI_2).sin();
		*enhanced *= (1.0 + beta) / (1.0 + beta / (sin * sin));
	}
}
