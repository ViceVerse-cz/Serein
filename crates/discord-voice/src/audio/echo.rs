//! Worker-owned speech processing; never runs in device callbacks.
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
			settings: VoiceProcessing::from_legacy(false).effective(),
		}
	}

	pub fn configure(&mut self, settings: Processing) -> Result<(), &'static str> {
		if !settings.is_valid() {
			return Err("Invalid microphone processing settings");
		}
		if settings == self.settings {
			return Ok(());
		}
		if settings.suppression == NoiseSuppression::RnNoise && self.noise.is_none() {
			self.noise = Some(noise_state());
		} else if settings.suppression != NoiseSuppression::RnNoise {
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

	pub fn reset(&mut self) {
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
				let input = output.map(|s| (s * 32768.0).clamp(-32768.0, 32767.0));
				noise.process_frame(&mut output, &input);
				for sample in &mut output {
					*sample = (*sample / 32768.0).clamp(-1.0, 1.0);
				}
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
