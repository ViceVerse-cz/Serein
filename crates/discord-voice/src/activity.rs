//! Display-only activity; never gates or changes transmitted audio.
// ponytail: a -45 dBFS level threshold detects sound, not speech; use VAD if noise lights it up.
pub(crate) fn hold(energy: f32, previous: u8) -> u8 {
	if energy.is_finite() && energy > 960.0 * 0.000_031_623 {
		10 // 200 ms at the transport's 20 ms cadence.
	} else {
		previous.saturating_sub(1)
	}
}

/// Finite RMS level for the local preview meter, clamped to its display range.
pub(crate) fn level_db(frame: &[f32; 960]) -> f32 {
	let energy: f32 = frame
		.iter()
		.filter(|s| s.is_finite())
		.map(|s| s.clamp(-1.0, 1.0).powi(2))
		.sum();
	(10.0 * (energy / 960.0).max(1e-10).log10()).clamp(-100.0, 0.0)
}

/// Manual sensitivity with 3 dB hysteresis, 200 ms release and a 5 ms click-free ramp.
#[derive(Default)]
pub(crate) struct InputGate {
	remaining: u8,
	gain: f32,
}
impl InputGate {
	pub fn apply(&mut self, frame: &mut [f32; 960], threshold: Option<i16>) -> bool {
		let Some(threshold) = threshold else {
			self.gain = 1.0;
			self.remaining = 0;
			return true;
		};
		let closing = self.remaining > 0;
		let threshold = f32::from(threshold.clamp(-80, 0)) - if closing { 3.0 } else { 0.0 };
		if level_db(frame) > threshold {
			self.remaining = 10;
		} else {
			self.remaining = self.remaining.saturating_sub(1);
		}
		let open = self.remaining > 0;
		let audible = open || self.gain > 0.0;
		for sample in frame {
			self.gain = (self.gain + if open { 1.0 / 240.0 } else { -1.0 / 240.0 }).clamp(0.0, 1.0);
			*sample *= self.gain;
		}
		audible
	}
}

pub(crate) fn hold_at(energy: f32, previous: u8, threshold: i16) -> u8 {
	if energy.is_finite()
		&& energy > 960.0 * 10.0_f32.powf(f32::from(threshold.clamp(-80, 0)) / 10.0)
	{
		10
	} else {
		previous.saturating_sub(1)
	}
}
