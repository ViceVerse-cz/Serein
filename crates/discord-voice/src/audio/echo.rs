//! Worker-owned speech processing; never runs in device callbacks.
use df::tract::{DfModels, DfParams, DfTract, RuntimeParams};
use model::voice_settings::{NoiseSuppression, Processing, VoiceProcessing};
use ndarray::{ArrayView2, ArrayViewMut2};
use nnnoiseless::DenoiseState;
use sonora::{
	AudioProcessing, Config, StreamConfig,
	config::{AdaptiveDigital, EchoCanceller, GainController2, NoiseSuppressionLevel},
};
use std::{
	thread::JoinHandle,
	time::{Duration, Instant},
};

pub struct Echo {
	processor: AudioProcessing,
	gain: Option<AudioProcessing>,
	noise: Option<Box<DenoiseState<'static>>>,
	deep: Option<Box<DeepFilter>>,
	/// DeepFilterNet networks being decoded and optimized on a helper thread.
	deep_load: Option<JoinHandle<Option<DfModels>>>,
	settings: Processing,
}

const DEEP_FAILED: &str = "DeepFilterNet noise suppression could not start";

/// Bundled DeepFilterNet3. The model keeps recurrent and look-ahead history, so a reset
/// restarts from `fresh` instead of decoding and optimizing the network again.
struct DeepFilter {
	fresh: DfTract,
	model: DfTract,
	level: u8,
}
/// The reference command-line stage thresholds. The narrower library defaults skip the
/// stateful decoders on many speech frames, which audibly clips speech in moderate noise.
fn deep_params() -> RuntimeParams {
	RuntimeParams::default_with_ch(1).with_thresholds(-15.0, 35.0, 35.0)
}
impl DeepFilter {
	/// Decoding and optimizing the networks takes about a fifth of a second; doing it on a
	/// helper thread keeps the worker servicing playback meanwhile.
	fn load() -> Result<JoinHandle<Option<DfModels>>, &'static str> {
		std::thread::Builder::new()
			.name("voice-deepfilternet".into())
			.spawn(|| DfModels::new(DfParams::default(), &deep_params()).ok())
			.map_err(|_| DEEP_FAILED)
	}
	fn start(models: DfModels, level: u8) -> Result<Box<Self>, &'static str> {
		let fresh = DfTract::from_models(models, &deep_params())
			.ok()
			.filter(|model| model.sr == 48_000 && model.hop_size == 480)
			.ok_or(DEEP_FAILED)?;
		let mut deep = Box::new(Self {
			model: fresh.clone(),
			fresh,
			level: u8::MAX,
		});
		deep.set_level(level);
		Ok(deep)
	}
	/// Low and Moderate cap the attenuation so some room sound remains; High runs the full
	/// model; Very high adds its post-filter, which attenuates very noisy sections further.
	fn set_level(&mut self, level: u8) {
		if level == self.level {
			return;
		}
		let (limit_db, post_filter_beta) = match level {
			0 => (12.0, 0.0),
			1 => (24.0, 0.0),
			2 => (100.0, 0.0),
			_ => (100.0, 0.02),
		};
		for model in [&mut self.fresh, &mut self.model] {
			model.set_atten_lim(limit_db);
			model.set_pf_beta(post_filter_beta);
		}
		self.level = level;
	}
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
			deep: None,
			deep_load: None,
			settings: VoiceProcessing::from_legacy(false).effective(),
		}
	}

	/// Applies `settings` and reports whether the selected processing is ready. DeepFilterNet
	/// loads on a helper thread, so the worker keeps calling this until it returns true.
	pub fn configure(&mut self, settings: Processing) -> Result<bool, &'static str> {
		if !settings.is_valid() {
			return Err("Invalid microphone processing settings");
		}
		if settings.suppression != NoiseSuppression::DeepFilterNet {
			self.deep = None;
			self.deep_load = None;
		} else if self.deep.is_none() && self.deep_load.is_none() {
			self.deep_load = Some(DeepFilter::load()?);
		}
		if let Some(load) = self.deep_load.take_if(|load| load.is_finished()) {
			let models = load.join().ok().flatten().ok_or(DEEP_FAILED)?;
			self.deep = Some(DeepFilter::start(models, settings.suppression_level)?);
		}
		if let Some(deep) = &mut self.deep {
			deep.set_level(settings.suppression_level);
		}
		if settings == self.settings {
			return Ok(self.deep_load.is_none());
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
		Ok(self.deep_load.is_none())
	}

	/// Configures and waits for a DeepFilterNet load; offline checks only, never the worker.
	#[cfg(debug_assertions)]
	pub fn configure_blocking(&mut self, settings: Processing) -> Result<(), &'static str> {
		while !self.configure(settings)? {
			std::thread::sleep(Duration::from_millis(1));
		}
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
		if let Some(deep) = &mut self.deep {
			deep.model.clone_from(&deep.fresh);
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
			if let Some(deep) = &mut self.deep {
				let start = time_noise.then(Instant::now);
				let input = output;
				let noisy = ArrayView2::from_shape((1, 480), &input);
				let clean = ArrayViewMut2::from_shape((1, 480), &mut output);
				let (Ok(noisy), Ok(clean)) = (noisy, clean) else {
					return Err("DeepFilterNet noise suppression failed");
				};
				deep.model
					.process(noisy, clean)
					.map_err(|_| "DeepFilterNet noise suppression failed")?;
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
