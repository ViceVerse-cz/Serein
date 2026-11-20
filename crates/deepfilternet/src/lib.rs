//! DeepFilterNet3 noise suppression for 48 kHz mono speech in plain Rust. The network weights
//! are embedded in the binary; nothing is loaded, downloaded or allocated per frame.
mod dsp;
mod net;

use dsp::{BANDS, FILTERED, Features, Spectrum, Stft, apply_band_gains, post_filter};
use net::{COEFFICIENTS, Network, TAPS};
use realfft::num_complex::Complex32;

/// Samples per frame: 10 ms at 48 kHz.
pub const FRAME: usize = dsp::HOP;
/// Frames the model looks ahead. With the window overlap, output lags input by 30 ms.
const LOOKAHEAD: usize = 2;

pub struct DeepFilterNet {
	stft: Stft,
	features: Features,
	network: Network,
	/// Noisy spectra of the last `TAPS` frames, oldest first.
	spectra: Box<[Spectrum; TAPS]>,
	/// Share of the noisy signal mixed back in, which caps the attenuation.
	noisy_share: f32,
	post_filter: f32,
}
impl Default for DeepFilterNet {
	fn default() -> Self {
		Self::new()
	}
}
impl DeepFilterNet {
	pub fn new() -> Self {
		Self {
			stft: Stft::new(),
			features: Features::new(),
			network: Network::new(),
			spectra: Box::new([[Complex32::ZERO; dsp::SPECTRUM]; TAPS]),
			noisy_share: 0.0,
			post_filter: 0.0,
		}
	}

	/// Caps noise attenuation at `decibels`, leaving some room sound; 100 or more removes
	/// the cap.
	pub fn set_attenuation_limit(&mut self, decibels: f32) {
		self.noisy_share = if decibels >= 100.0 {
			0.0
		} else {
			10f32.powf(-decibels.max(0.0) / 20.0)
		};
	}

	/// Attenuates bins the model already reduced strongly a little further. Zero disables
	/// the filter; the reference setting is 0.02.
	pub fn set_post_filter(&mut self, beta: f32) {
		self.post_filter = beta.max(0.0);
	}

	/// Drops all audio history, so the next frame starts a new stream.
	pub fn reset(&mut self) {
		self.stft.reset();
		self.features = Features::new();
		self.network.reset();
		self.spectra.fill([Complex32::ZERO; dsp::SPECTRUM]);
	}

	/// Replaces `frame` with the suppressed audio of the frame 30 ms earlier.
	pub fn process(&mut self, frame: &mut [f32; FRAME]) {
		self.spectra.rotate_left(1);
		self.stft.analysis(frame, &mut self.spectra[TAPS - 1]);
		let (mut bands, mut bins) = ([0.0; BANDS], [[0.0; FILTERED]; 2]);
		self.features
			.extract(&self.spectra[TAPS - 1], &mut bands, &mut bins);
		let (mut gains, mut filters) = ([0.0; BANDS], [[0.0; COEFFICIENTS]; FILTERED]);
		self.network
			.forward(&bands, &bins, &mut gains, &mut filters);

		let noisy = &self.spectra[TAPS - 1 - LOOKAHEAD];
		let mut enhanced = *noisy;
		apply_band_gains(&mut enhanced, &gains);
		// The lowest bins are rebuilt from the surrounding noisy frames instead.
		for (bin, (enhanced, filter)) in enhanced.iter_mut().zip(&filters).enumerate() {
			*enhanced = self
				.spectra
				.iter()
				.zip(filter.as_chunks::<2>().0)
				.map(|(spectrum, [re, im])| spectrum[bin] * Complex32::new(*re, *im))
				.sum();
		}
		if self.post_filter > 0.0 {
			post_filter(noisy, &mut enhanced, self.post_filter);
		}
		if self.noisy_share > 0.0 {
			for (enhanced, noisy) in enhanced.iter_mut().zip(noisy) {
				*enhanced = *enhanced * (1.0 - self.noisy_share) + noisy * self.noisy_share;
			}
		}
		self.stft.synthesis(&mut enhanced, frame);
	}
}
