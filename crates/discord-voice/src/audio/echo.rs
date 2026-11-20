//! Worker-owned speech processing; never runs in device callbacks.
use deepfilternet::DeepFilterNet;
use model::voice_settings::{NoiseSuppression, Processing, VoiceProcessing};
use nnnoiseless::DenoiseState;
use sonora::{
	AudioProcessing, Config, StreamConfig,
	config::{AdaptiveDigital, EchoCanceller, GainController2, NoiseSuppressionLevel},
};
use std::time::{Duration, Instant};

/// Hops of denoiser input kept for RNNoise to take over from DeepFilterNet.
const RECENT: usize = 8;
/// DeepFilterNet's output trails its input by three hops and RNNoise's by one.
const BEHIND: usize = 2;

pub struct Echo {
	processor: AudioProcessing,
	gain: Option<AudioProcessing>,
	noise: Option<Box<DenoiseState<'static>>>,
	deep: Option<Box<DeepFilterNet>>,
	budget: Budget,
	/// The last denoiser inputs while automatic selection runs DeepFilterNet, oldest first.
	recent: Box<[[f32; 480]; RECENT]>,
	/// Input that RNNoise has yet to process after taking over. It stays for as long as that
	/// RNNoise stream runs, since shortening the delay would skip captured audio.
	queued: Option<[[f32; 480]; BEHIND]>,
	settings: Processing,
}

/// What DeepFilterNet may cost under automatic selection before RNNoise takes over.
struct Budget {
	/// Microseconds each of the last hops took, none counted above `HOP`.
	hops: [u16; Self::WINDOW],
	next: usize,
	total: u32,
	last: Option<Instant>,
	/// Kept until the suppression choice changes, so a busy device does not switch back and
	/// forth during a call.
	exceeded: bool,
}
impl Budget {
	/// Half a second of 10 ms hops.
	const WINDOW: usize = 50;
	/// A hop counts for at most its own 10 ms, so one long stall of the whole device (a
	/// page-in, a suspended process) is not mistaken for sustained load.
	const HOP: u16 = 10_000;
	/// 3 ms per hop on average: 30% of real time on the audio worker.
	const LIMIT: u32 = 3_000 * Self::WINDOW as u32;
	/// A longer gap means capture was paused (mute, push-to-talk). An overrun restarts
	/// capture within milliseconds and must keep its evidence of the overload.
	const PAUSE: Duration = Duration::from_secs(1);

	fn new() -> Self {
		Self {
			hops: [0; Self::WINDOW],
			next: 0,
			total: 0,
			last: None,
			exceeded: false,
		}
	}

	/// Records the hop that began at `start` and reports whether DeepFilterNet may continue.
	fn observe(&mut self, start: Instant, elapsed: Duration) -> bool {
		let paused = |last| start.saturating_duration_since(last) > Self::PAUSE;
		if self.last.is_some_and(paused) {
			self.hops = [0; Self::WINDOW];
			self.total = 0;
		}
		self.last = Some(start);
		let cost = elapsed.as_micros().min(u128::from(Self::HOP)) as u16;
		self.total -= u32::from(std::mem::replace(&mut self.hops[self.next], cost));
		self.total += u32::from(cost);
		self.next = (self.next + 1) % Self::WINDOW;
		self.exceeded |= self.total > Self::LIMIT;
		!self.exceeded
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
fn rnnoise(noise: &mut DenoiseState<'static>, hop: &mut [f32; 480]) {
	let input = hop.map(|s| (s * 32768.0).clamp(-32768.0, 32767.0));
	noise.process_frame(hop, &input);
	for sample in hop {
		*sample = (*sample / 32768.0).clamp(-1.0, 1.0);
	}
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
			budget: Budget::new(),
			recent: Box::new([[0.0; 480]; RECENT]),
			queued: None,
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
		if settings.suppression != self.settings.suppression {
			self.budget = Budget::new();
			self.recent.fill([0.0; 480]);
		}
		let (rnnoise, deep) = match settings.suppression {
			NoiseSuppression::RnNoise => (true, false),
			NoiseSuppression::DeepFilterNet => (false, true),
			NoiseSuppression::Auto => (self.budget.exceeded, !self.budget.exceeded),
			NoiseSuppression::Off | NoiseSuppression::WebRtc => (false, false),
		};
		if rnnoise && self.noise.is_none() {
			self.noise = Some(noise_state());
		} else if !rnnoise {
			self.noise = None;
			self.queued = None;
		}
		if deep {
			let deep = self
				.deep
				.get_or_insert_with(|| Box::new(DeepFilterNet::new()));
			// Low and Moderate cap the attenuation so some room sound remains; High runs the
			// full model; Very high adds its post-filter, which attenuates very noisy
			// sections further.
			let (limit_db, post_filter) = match settings.suppression_level {
				0 => (12.0, 0.0),
				1 => (24.0, 0.0),
				2 => (100.0, 0.0),
				_ => (100.0, 0.02),
			};
			deep.set_attenuation_limit(limit_db);
			deep.set_post_filter(post_filter);
		} else {
			self.deep = None;
		}
		// The strength also belongs to DeepFilterNet, which must not disturb this processor.
		let webrtc = |settings: &Processing| {
			(settings.suppression == NoiseSuppression::WebRtc).then_some(settings.suppression_level)
		};
		if settings.echo_cancellation != self.settings.echo_cancellation
			|| webrtc(&settings) != webrtc(&self.settings)
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

	/// Automatic selection found DeepFilterNet too costly here and runs RNNoise instead.
	pub fn fell_back(&self) -> bool {
		self.budget.exceeded
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
			deep.reset();
		}
		self.recent.fill([0.0; 480]);
		self.queued = None;
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
				if let Some([next, last]) = &mut self.queued {
					output = std::mem::replace(next, std::mem::replace(last, output));
				}
				rnnoise(noise, &mut output);
				if let Some(start) = start {
					noise_time += start.elapsed();
				}
			}
			if let Some(deep) = &mut self.deep {
				let automatic = self.settings.suppression == NoiseSuppression::Auto;
				if automatic {
					self.recent.rotate_left(1);
					self.recent[RECENT - 1] = output;
				}
				let start = Instant::now();
				deep.process(&mut output);
				let elapsed = start.elapsed();
				if time_noise {
					noise_time += elapsed;
				}
				if automatic && !self.budget.observe(start, elapsed) {
					// RNNoise takes over without skipping captured audio: it warms up on
					// the older hops and continues two hops behind, where DeepFilterNet's
					// delayed output left off.
					let mut noise = noise_state();
					let (warm, queued) = self.recent.split_at(RECENT - BEHIND);
					for hop in warm {
						rnnoise(&mut noise, &mut hop.clone());
					}
					self.queued = queued.try_into().ok();
					self.noise = Some(noise);
					self.deep = None;
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
mod tests {
	use super::*;

	fn automatic() -> Processing {
		Processing {
			suppression: NoiseSuppression::Auto,
			..Processing::studio()
		}
	}
	fn energy(samples: &[f32]) -> f32 {
		samples.iter().map(|sample| sample * sample).sum()
	}

	#[test]
	fn automatic_budget_ignores_stalls_and_pauses_but_bounds_sustained_load() {
		let start = Instant::now();
		let at = |hop: u32| start + Duration::from_millis(10) * hop;
		let ms = Duration::from_millis;
		let mut budget = Budget::new();
		assert!(budget.observe(at(0), Duration::from_secs(2)));
		for hop in 1..200 {
			assert!(budget.observe(at(hop), ms(2)));
		}
		// 4 ms hops replace 2 ms ones: the last fifty pass 150 ms on the 26th.
		for hop in 200..225 {
			assert!(budget.observe(at(hop), ms(4)));
		}
		assert!(!budget.observe(at(225), ms(4)));
		assert!(!budget.observe(at(226), Duration::ZERO));

		// Fourteen full hops before a pause do not count with fourteen after it; across the
		// short gap of an overrun they do.
		for (gap, kept) in [(ms(2_000), true), (ms(200), false)] {
			let mut budget = Budget::new();
			let mut hops = (0..14).map(|hop| budget.observe(at(hop), ms(10)));
			assert!(hops.all(|kept| kept));
			let mut hops = (0..14).map(|hop| budget.observe(at(14 + hop) + gap, ms(10)));
			assert_eq!(hops.all(|kept| kept), kept);
		}
	}

	#[test]
	fn automatic_selection_falls_back_to_rnnoise_until_the_choice_changes() {
		let mut dsp = Echo::new();
		dsp.configure(automatic()).unwrap();
		// A burst in the hop before the one that trips the budget: DeepFilterNet has taken
		// it in but not yet put it out.
		let mut seed = 17_u32;
		let mut burst = [0.0; 960];
		for sample in &mut burst[480..] {
			seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
			*sample = seed as i32 as f32 / i32::MAX as f32 * 0.3;
		}
		let mut frames = [[0.0; 960], burst, [0.0; 960], [0.0; 960], [0.0; 960]];
		for (index, frame) in frames.iter_mut().enumerate() {
			if index == 2 {
				assert!(dsp.deep.is_some() && dsp.noise.is_none() && !dsp.fell_back());
				dsp.budget.total = Budget::LIMIT;
			}
			dsp.capture(frame, false).unwrap();
		}
		assert!(dsp.deep.is_none() && dsp.noise.is_some() && dsp.fell_back());
		// RNNoise puts the burst out where DeepFilterNet would have: three hops later.
		let heard = energy(&frames[3]);
		assert!(heard > energy(&burst) * 0.02, "{heard}");
		assert!(energy(&frames[2]) + energy(&frames[4]) < heard * 0.1);
		// Other setting changes keep the decision, and a reset ends only the extra delay.
		dsp.configure(Processing {
			automatic_gain: true,
			..automatic()
		})
		.unwrap();
		assert!(dsp.queued.is_some());
		dsp.reset();
		dsp.capture(&mut [0.01; 960], false).unwrap();
		assert!(dsp.deep.is_none() && dsp.noise.is_some() && dsp.fell_back());
		assert!(dsp.queued.is_none());
		// Choosing RNNoise outright after a fallback continues the same delayed stream.
		dsp.configure(Processing::studio()).unwrap();
		dsp.configure(automatic()).unwrap();
		dsp.budget.total = Budget::LIMIT;
		dsp.capture(&mut [0.01; 960], false).unwrap();
		dsp.configure(Processing {
			suppression: NoiseSuppression::RnNoise,
			..automatic()
		})
		.unwrap();
		assert!(dsp.noise.is_some() && dsp.queued.is_some() && !dsp.fell_back());
		// Choosing again assesses again.
		dsp.configure(Processing::studio()).unwrap();
		dsp.configure(automatic()).unwrap();
		assert!(dsp.deep.is_some() && dsp.noise.is_none() && !dsp.fell_back());
	}
}
