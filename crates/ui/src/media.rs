//! Bounded playback-rate policy shared by inline audio and video.
//!
//! The native player changes rate by resampling decoded PCM in its output callback and
//! scaling the presentation clock. That is a truthful speed change, but it does not
//! preserve pitch; no decoder or clock is faked here.

/// Slowest supported inline playback rate.
pub const MIN_PLAYBACK_RATE: f32 = 0.25;
/// Fastest supported inline playback rate.
pub const MAX_PLAYBACK_RATE: f32 = 3.5;
/// Increment between keyboard-selectable presets.
pub const PLAYBACK_RATE_STEP: f32 = 0.25;

/// Rates offered by the media controls, including the normal `1x`.
pub const PLAYBACK_RATES: [f32; 14] = [
	0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 2.75, 3.0, 3.25, 3.5,
];

/// Keeps a requested rate finite and inside the media-player contract.
///
/// Non-finite input (including NaN from a malformed command or extension) falls back to
/// normal speed rather than reaching an audio callback or presentation clock.
pub fn normalize_playback_rate(rate: f32) -> f32 {
	if !rate.is_finite() {
		return 1.0;
	}
	let clamped = rate.clamp(MIN_PLAYBACK_RATE, MAX_PLAYBACK_RATE);
	// Discards float dust from percentage-based extension input while keeping presets exact.
	(clamped * 100.0).round() / 100.0
}

/// Formats a rate without trailing zeros: `1x`, `1.25x`, `3.5x`.
pub fn rate_label(rate: f32) -> String {
	let percent = (normalize_playback_rate(rate) * 100.0).round() as u32;
	if percent.is_multiple_of(100) {
		format!("{}x", percent / 100)
	} else if percent.is_multiple_of(10) {
		format!("{}.{}x", percent / 100, (percent % 100) / 10)
	} else {
		format!("{}.{:02}x", percent / 100, percent % 100)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn playback_rate_presets_and_input_are_bounded() {
		assert_eq!(PLAYBACK_RATES.len(), 14);
		assert_eq!(PLAYBACK_RATES[0], MIN_PLAYBACK_RATE);
		assert_eq!(PLAYBACK_RATES[13], MAX_PLAYBACK_RATE);
		for (index, rate) in PLAYBACK_RATES.iter().copied().enumerate() {
			assert_eq!(normalize_playback_rate(rate), rate);
			if index > 0 {
				assert!((rate - PLAYBACK_RATES[index - 1] - PLAYBACK_RATE_STEP).abs() < 1e-6);
			}
		}
		for (input, expected) in [
			(f32::NAN, 1.0),
			(f32::INFINITY, 1.0),
			(f32::NEG_INFINITY, 1.0),
			(-2.0, MIN_PLAYBACK_RATE),
			(0.0, MIN_PLAYBACK_RATE),
			(3.51, MAX_PLAYBACK_RATE),
			(100.0, MAX_PLAYBACK_RATE),
			(1.234, 1.23),
		] {
			assert_eq!(normalize_playback_rate(input), expected, "{input}");
		}
		assert_eq!(rate_label(1.0), "1x");
		assert_eq!(rate_label(1.5), "1.5x");
		assert_eq!(rate_label(1.25), "1.25x");
	}
}
