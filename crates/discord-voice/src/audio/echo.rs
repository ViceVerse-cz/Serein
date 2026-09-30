//! Worker-owned speech processing; never runs in device callbacks.
#[path = "echo/deep_filter.rs"]
mod deep_filter;
use deep_filter::DeepFilter;
pub use deep_filter::Status;
use model::voice_settings::{NoiseSuppression, Processing, VoiceProcessing};
use nnnoiseless::DenoiseState;
use sonora::{
	AudioProcessing, Config, StreamConfig,
	config::{AdaptiveDigital, EchoCanceller, GainController2, NoiseSuppressionLevel},
};
use std::time::{Duration, Instant};

pub struct Echo {
	processor: AudioProcessing,
	gain: Option<AudioProcessing>,
	noise: Option<Box<DenoiseState<'static>>>,
	settings: Processing,
	deep_filter: DeepFilter,
	deep_was_active: bool,
}

fn processor(config: Config) -> AudioProcessing {
	AudioProcessing::builder()
		.config(config)
		.capture_config(StreamConfig::new(48_000, 1))
		.render_config(StreamConfig::new(48_000, 1))
		.build()
}
fn noise_state() -> Box<DenoiseState<'static>> {
	let mut noise = DenoiseState::new();
	noise.process_frame(&mut [0.0; 480], &[0.0; 480]);
	noise
}
impl Echo {
	pub fn new() -> Self {
		Self {
			processor: processor(Config {
				echo_canceller: Some(EchoCanceller::default()),
				..Default::default()
			}),
			gain: None,
			noise: None,
			deep_filter: DeepFilter::default(),
			deep_was_active: false,
			settings: VoiceProcessing::from_legacy(false).effective(),
		}
	}

	pub fn configure(&mut self, settings: Processing) -> Result<(), &'static str> {
		if !settings.is_valid() {
			return Err("Invalid microphone processing settings");
		}
		self.deep_filter
			.configure(settings.suppression, settings.suppression_level);
		if settings == self.settings {
			return Ok(());
		}
		let neural = matches!(
			settings.suppression,
			NoiseSuppression::RnNoise | NoiseSuppression::Auto | NoiseSuppression::DeepFilterNet
		);
		if neural && self.noise.is_none() {
			self.noise = Some(noise_state());
		} else if !neural {
			self.noise = None;
		}
		if settings.echo_cancellation != self.settings.echo_cancellation
			|| (settings.suppression == NoiseSuppression::WebRtc)
				!= (self.settings.suppression == NoiseSuppression::WebRtc)
			|| settings.suppression_level != self.settings.suppression_level
		{
			self.processor.apply_config(Config {
				echo_canceller: settings.echo_cancellation.then(EchoCanceller::default),
				noise_suppression: (settings.suppression == NoiseSuppression::WebRtc).then(|| {
					sonora::config::NoiseSuppression {
						level: match settings.suppression_level {
							0 => NoiseSuppressionLevel::Low,
							1 => NoiseSuppressionLevel::Moderate,
							2 => NoiseSuppressionLevel::High,
							_ => NoiseSuppressionLevel::VeryHigh,
						},
						..Default::default()
					}
				}),
				..Default::default()
			});
		}
		if settings.automatic_gain && self.gain.is_none() {
			// Digital-only AGC runs after the chosen denoiser. It never changes OS mic gain.
			self.gain = Some(processor(Config {
				gain_controller2: Some(GainController2 {
					adaptive_digital: Some(AdaptiveDigital {
						max_gain_db: 20.0,
						initial_gain_db: 0.0,
						..Default::default()
					}),
					..Default::default()
				}),
				..Default::default()
			}));
		} else if !settings.automatic_gain {
			self.gain = None;
		}
		self.settings = settings;
		Ok(())
	}

	pub fn suppression_status(&self) -> Status {
		self.deep_filter.status()
	}

	pub fn reset(&mut self) {
		self.deep_filter.reset();
		self.deep_was_active = false;
		let config = StreamConfig::new(48_000, 1);
		self.processor.initialize(config, config, config, config);
		if let Some(gain) = &mut self.gain {
			gain.initialize(config, config, config, config);
		}
		if self.noise.is_some() {
			self.noise = Some(noise_state());
		}
	}

	pub fn render(&mut self, frame: &[f32; 960]) -> Result<(), &'static str> {
		if !self.settings.echo_cancellation {
			return Ok(());
		}
		let mut output = [0.0; 480];
		for chunk in frame.as_chunks::<480>().0 {
			self.processor
				.process_render_f32(&[chunk], &mut [&mut output])
				.map_err(|_| "Echo cancellation could not process speaker audio")?;
		}
		Ok(())
	}

	pub fn capture(
		&mut self,
		frame: &mut [f32; 960],
		time_noise: bool,
	) -> Result<Duration, &'static str> {
		for sample in frame.iter_mut() {
			*sample = if sample.is_finite() {
				sample.clamp(-1.0, 1.0)
			} else {
				0.0
			};
		}
		let mut noise_time = Duration::ZERO;
		for chunk in frame.as_chunks_mut::<480>().0 {
			let mut output = *chunk;
			if self.settings.echo_cancellation
				|| self.settings.suppression == NoiseSuppression::WebRtc
			{
				self.processor
					.set_stream_delay_ms(0)
					.map_err(|_| "Echo cancellation delay is invalid")?;
				self.processor
					.process_capture_f32(&[chunk], &mut [&mut output])
					.map_err(|_| "Microphone processing failed")?;
			}
			if let Some(noise) = &mut self.noise {
				let start = time_noise.then(Instant::now);
				let deep_active = self.deep_filter.process(&mut output);
				if !deep_active {
					if self.deep_was_active {
						*noise = noise_state();
					}
					let input = output.map(|s| (s * 32768.0).clamp(-32768.0, 32767.0));
					noise.process_frame(&mut output, &input);
					for sample in &mut output {
						*sample = (*sample / 32768.0).clamp(-1.0, 1.0);
					}
				}
				self.deep_was_active = deep_active;
				if let Some(start) = start {
					noise_time += start.elapsed();
				}
			}
			chunk.copy_from_slice(&output);
		}
		if let Some(gain) = &mut self.gain {
			for chunk in frame.as_chunks_mut::<480>().0 {
				let mut output = [0.0; 480];
				gain.process_capture_f32(&[chunk], &mut [&mut output])
					.map_err(|_| "Automatic gain processing failed")?;
				chunk.copy_from_slice(&output);
			}
		}
		for sample in frame {
			*sample = if sample.is_finite() {
				sample.clamp(-1.0, 1.0)
			} else {
				0.0
			};
		}
		Ok(noise_time)
	}
}

#[cfg(test)]
mod deepfilter_tests {
	use super::*;
	fn configured() -> Echo {
		let mut dsp = Echo::new();
		let settings = Processing {
			suppression: NoiseSuppression::DeepFilterNet,
			suppression_level: 2,
			..Processing::studio()
		};
		let deadline = Instant::now();
		loop {
			dsp.configure(settings).unwrap();
			if dsp.suppression_status() != Status::Loading {
				break;
			}
			assert!(deadline.elapsed() < Duration::from_secs(30));
			std::thread::sleep(Duration::from_millis(5));
		}
		assert_eq!(
			dsp.suppression_status(),
			Status::DeepFilter,
			"exercise the actual model, not fallback"
		);
		dsp
	}
	#[test]
	fn deepfilter_suppresses_noise_resets_history_and_switches_without_stale_audio() {
		let mut dsp = configured();
		let mut seed = 17_u32;
		let inputs: Vec<[f32; 960]> = (0..100)
			.map(|_| {
				std::array::from_fn(|_| {
					seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
					seed as i32 as f32 / i32::MAX as f32 * 0.05
				})
			})
			.collect();
		let mut before = 0.0;
		let mut after = 0.0;
		for (index, input) in inputs.iter().enumerate() {
			let mut samples = *input;
			dsp.capture(&mut samples, false).unwrap();
			assert!(samples.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
			if index > 50 {
				before += input.iter().map(|v| v * v).sum::<f32>();
				after += samples.iter().map(|v| v * v).sum::<f32>();
			}
		}
		assert!(
			after < before * 0.5,
			"synthetic noise must be attenuated: {before} -> {after}"
		);
		dsp.reset();
		let mut first = inputs[0];
		dsp.capture(&mut first, false).unwrap();
		for input in &inputs[1..8] {
			let mut samples = *input;
			dsp.capture(&mut samples, false).unwrap();
		}
		dsp.reset();
		let mut fresh = inputs[0];
		dsp.capture(&mut fresh, false).unwrap();
		assert_eq!(
			first, fresh,
			"mute/PTT reset must forget recurrent audio history"
		);
		let rn = Processing {
			suppression: NoiseSuppression::RnNoise,
			..Processing::studio()
		};
		let mut reference = Echo::new();
		reference.configure(rn).unwrap();
		dsp.configure(rn).unwrap();
		let mut expected = inputs[0];
		let mut switched = inputs[0];
		reference.capture(&mut expected, false).unwrap();
		dsp.capture(&mut switched, false).unwrap();
		assert_eq!(
			switched, expected,
			"fallback must start with fresh RNNoise history"
		);
		dsp.configure(Processing::studio()).unwrap();
		let mut raw = inputs[0];
		dsp.capture(&mut raw, false).unwrap();
		assert_eq!(raw, inputs[0]);
	}
}
