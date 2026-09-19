//! One bounded, lazy audio worker. Bundled cues never interrupt attachment playback.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use model::notification_preferences::Sound;
use std::{
	io::Cursor,
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
		mpsc::{self, SyncSender},
	},
	time::{Duration, Instant},
};

// Five seconds of audio plus a gap; shared with the automatic incoming-call timer.
pub const RING_INTERVAL: Duration = Duration::from_secs(6);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
	Notification(Sound),
	Mute,
	Unmute,
	Deafen,
	Undeafen,
	Join,
	Leave,
	StreamStart,
}
impl From<Sound> for Cue {
	fn from(value: Sound) -> Self {
		Self::Notification(value)
	}
}

#[derive(Default)]
pub struct Sounds {
	send: Option<SyncSender<(u64, Cue)>>,
	generation: Arc<AtomicU64>,
	status: Arc<AtomicU8>,
}
impl Sounds {
	pub fn status(&self) -> &'static str {
		match self.status.load(Ordering::Acquire) {
			1 => "Playing notification sound...",
			2 => "Audio output unavailable. Check your system sound settings.",
			_ => "",
		}
	}
	pub fn stop(&mut self) {
		self.generation.fetch_add(1, Ordering::AcqRel);
		self.status.store(0, Ordering::Release);
	}
	pub fn play(&mut self, sound: impl Into<Cue>, ctx: &eframe::egui::Context) {
		let sound = sound.into();
		if self.send.is_none() {
			let (send, receive) = mpsc::sync_channel::<(u64, Cue)>(1);
			let generation = self.generation.clone();
			let status = self.status.clone();
			let context = ctx.clone();
			if std::thread::Builder::new()
				.name("serein-notification-audio".into())
				.spawn(move || {
					while let Ok((request, sound)) = receive.recv() {
						if generation.load(Ordering::Acquire) != request {
							continue;
						}
						let finished = Arc::new(AtomicBool::new(false));
						match open(
							sound,
							generation.clone(),
							request,
							status.clone(),
							finished.clone(),
						) {
							Ok((stream, duration)) => {
								let deadline = Instant::now() + duration + Duration::from_secs(2);
								while !finished.load(Ordering::Acquire) && Instant::now() < deadline
								{
									if generation.load(Ordering::Acquire) != request {
										break;
									}
									std::thread::sleep(Duration::from_millis(20));
								}
								drop(stream);
								if generation.load(Ordering::Acquire) == request {
									let _ = status.compare_exchange(
										1,
										if finished.load(Ordering::Acquire) {
											0
										} else {
											2
										},
										Ordering::AcqRel,
										Ordering::Acquire,
									);
								}
							}
							Err(()) => {
								if generation.load(Ordering::Acquire) == request {
									status.store(2, Ordering::Release);
								}
							}
						}
						context.request_repaint();
					}
				})
				.is_err()
			{
				self.status.store(2, Ordering::Release);
				return;
			}
			self.send = Some(send);
		}
		let request = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
		self.status.store(1, Ordering::Release);
		if self
			.send
			.as_ref()
			.expect("worker started")
			.try_send((request, sound))
			.is_err()
		{
			self.status.store(0, Ordering::Release);
		}
	}
}
impl Drop for Sounds {
	fn drop(&mut self) {
		self.stop();
	}
}

fn samples(sound: Cue, rate: u32, current: &impl Fn() -> bool) -> Result<Vec<[f32; 2]>, ()> {
	if !(8000..=192000).contains(&rate) {
		return Err(());
	}
	let Cue::Notification(sound) = sound else {
		return current().then(|| generated(sound, rate)).ok_or(());
	};
	let bytes: &[u8] = match sound {
		Sound::Message => include_bytes!("../../../assets/sounds/message.mp3"),
		Sound::CurrentChannel => include_bytes!("../../../assets/sounds/current-channel.mp3"),
		Sound::IncomingRing => include_bytes!("../../../assets/sounds/incoming-ring.mp3"),
	};
	if bytes.len() > 128 * 1024 {
		return Err(());
	}
	let mut pcm = Vec::new();
	crate::audio::decode_stream(
		Box::new(Cursor::new(bytes)),
		current,
		&mut |chunk, channels, source_rate, _| {
			if channels != 2 || source_rate != 48000 || pcm.len() + chunk.len() > 48000 * 2 * 5 {
				return Err("Invalid bundled notification sound");
			}
			pcm.extend(chunk.iter().map(|sample| {
				if sample.is_finite() {
					sample.clamp(-1.0, 1.0)
				} else {
					0.0
				}
			}));
			Ok(())
		},
	)
	.map_err(|_| ())?;
	if !current() || pcm.is_empty() {
		return Err(());
	}
	let frames = pcm.len() / 2;
	// ponytail: linear rate conversion; use a band-limited resampler if quality measurements require it.
	Ok((0..(frames * rate as usize).div_ceil(48000))
		.map(|frame| {
			let position = frame as f64 * 48000.0 / f64::from(rate);
			let index = (position as usize).min(frames - 1);
			let next = (index + 1).min(frames - 1);
			std::array::from_fn(|channel| {
				let a = pcm[index * 2 + channel];
				let b = pcm[next * 2 + channel];
				a + (b - a) * (position - index as f64) as f32
			})
		})
		.collect())
}
fn generated(sound: Cue, rate: u32) -> Vec<[f32; 2]> {
	let tones: &[(f32, u32)] = match sound {
		Cue::Mute => &[(520.0, 60), (330.0, 100)],
		Cue::Unmute => &[(330.0, 60), (520.0, 100)],
		Cue::Deafen => &[(440.0, 70), (220.0, 130)],
		Cue::Undeafen => &[(220.0, 70), (440.0, 130)],
		Cue::Join => &[(392.0, 70), (523.25, 110)],
		Cue::Leave => &[(523.25, 70), (392.0, 110)],
		Cue::StreamStart => &[(330.0, 50), (440.0, 50), (659.25, 100)],
		Cue::Notification(_) => return Vec::new(),
	};
	let fade = (rate / 200).max(1) as usize;
	let mut output = Vec::with_capacity(
		tones
			.iter()
			.map(|(_, millis)| rate as usize * *millis as usize / 1000)
			.sum(),
	);
	for (frequency, millis) in tones {
		let frames = (rate as usize * *millis as usize / 1000).max(1);
		for frame in 0..frames {
			let edge = frame.min(frames - frame - 1).min(fade) as f32 / fade as f32;
			let phase = std::f32::consts::TAU * *frequency * frame as f32 / rate as f32;
			let value = phase.sin() * edge * 0.12;
			output.push([value, value]);
		}
	}
	output
}
fn open(
	sound: Cue,
	generation: Arc<AtomicU64>,
	request: u64,
	status: Arc<AtomicU8>,
	finished: Arc<AtomicBool>,
) -> Result<(cpal::Stream, Duration), ()> {
	let device = cpal::default_host().default_output_device().ok_or(())?;
	let supported = device.default_output_config().map_err(|_| ())?;
	let config = supported.config();
	if !(8000..=192000).contains(&config.sample_rate) || !(1..=8).contains(&config.channels) {
		return Err(());
	}
	let samples = samples(sound, config.sample_rate, &|| {
		generation.load(Ordering::Acquire) == request
	})?;
	let duration = Duration::from_secs_f64(samples.len() as f64 / f64::from(config.sample_rate));
	let stream = match supported.sample_format() {
		cpal::SampleFormat::F32 => output::<f32>(
			&device, config, samples, generation, request, status, finished,
		),
		cpal::SampleFormat::I16 => output::<i16>(
			&device, config, samples, generation, request, status, finished,
		),
		cpal::SampleFormat::I32 => output::<i32>(
			&device, config, samples, generation, request, status, finished,
		),
		cpal::SampleFormat::U16 => output::<u16>(
			&device, config, samples, generation, request, status, finished,
		),
		_ => return Err(()),
	}
	.map_err(|_| ())?;
	stream.play().map_err(|_| ())?;
	Ok((stream, duration))
}
fn output<T: cpal::SizedSample + cpal::FromSample<f32>>(
	device: &cpal::Device,
	config: cpal::StreamConfig,
	samples: Vec<[f32; 2]>,
	generation: Arc<AtomicU64>,
	request: u64,
	status: Arc<AtomicU8>,
	finished: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error> {
	let errors = generation.clone();
	let failed = finished.clone();
	device.build_output_stream(
		config,
		callback::<T>(config, samples, generation, request, finished),
		move |_| {
			failed.store(true, Ordering::Release);
			if errors.load(Ordering::Acquire) == request {
				status.store(2, Ordering::Release);
			}
		},
		None,
	)
}
fn callback<T: cpal::SizedSample + cpal::FromSample<f32>>(
	config: cpal::StreamConfig,
	samples: Vec<[f32; 2]>,
	generation: Arc<AtomicU64>,
	request: u64,
	finished: Arc<AtomicBool>,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
	let mut position = 0;
	let mut end = None;
	move |data: &mut [T], info| {
		data.fill(T::from_sample(0.0));
		if generation.load(Ordering::Acquire) != request {
			return;
		}
		let timestamp = info.timestamp();
		if end.is_some_and(|end| timestamp.callback >= end) {
			finished.store(true, Ordering::Release);
			return;
		}
		for frame in data.chunks_exact_mut(usize::from(config.channels)) {
			let Some(sample) = samples.get(position) else {
				break;
			};
			if frame.len() == 1 {
				frame[0] = T::from_sample((sample[0] + sample[1]) * 0.5);
			} else {
				for (target, value) in frame.iter_mut().zip(sample) {
					*target = T::from_sample(*value);
				}
			}
			position += 1;
		}
		if position == samples.len() && end.is_none() {
			// Wait until the final buffer reaches the device, including cold-start latency.
			end = timestamp.playback.checked_add(Duration::from_secs_f64(
				data.len() as f64 / f64::from(config.channels) / f64::from(config.sample_rate),
			));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn callback_drains_delayed_output_and_cancellation_is_request_local() {
		let config = cpal::StreamConfig {
			channels: 4,
			sample_rate: 8000,
			buffer_size: cpal::BufferSize::Default,
		};
		let generation = Arc::new(AtomicU64::new(1));
		let finished = Arc::new(AtomicBool::new(false));
		let mut render = callback(
			config,
			vec![[0.2, 0.4]; 2],
			generation.clone(),
			1,
			finished.clone(),
		);
		let info = |callback_ms, playback_ms| {
			cpal::OutputCallbackInfo::new(cpal::OutputStreamTimestamp {
				callback: cpal::StreamInstant::from_millis(callback_ms),
				playback: cpal::StreamInstant::from_millis(playback_ms),
			})
		};
		let mut data = [1.0_f32; 8];
		render(&mut data, &info(0, 1000));
		assert_eq!(data, [0.2, 0.4, 0.0, 0.0, 0.2, 0.4, 0.0, 0.0]);
		render(&mut data, &info(500, 1500));
		assert_eq!(data, [0.0; 8]);
		assert!(!finished.load(Ordering::Acquire));
		render(&mut data, &info(1001, 2001));
		assert!(finished.load(Ordering::Acquire));

		let next_finished = Arc::new(AtomicBool::new(false));
		let mut next = callback(
			cpal::StreamConfig {
				channels: 1,
				..config
			},
			vec![[0.2, 0.4]; 2],
			generation.clone(),
			2,
			next_finished.clone(),
		);
		generation.store(2, Ordering::Release);
		let mut cancelled = callback(config, vec![[1.0, 1.0]; 2], generation.clone(), 1, finished);
		cancelled(&mut data, &info(2000, 3000));
		assert_eq!(data, [0.0; 8]);
		assert!(!next_finished.load(Ordering::Acquire));
		next(&mut data[..2], &info(2000, 3000));
		assert_eq!(&data[..2], &[0.3, 0.3]);
	}
	#[test]
	fn bundled_cues_decode_in_full_at_supported_rates_and_cancel() {
		for rate in [8000, 44100, 48000, 192000] {
			let cues = [Sound::Message, Sound::CurrentChannel, Sound::IncomingRing]
				.map(|s| samples(s.into(), rate, &|| true).unwrap());
			assert_ne!(cues[0], cues[1]);
			for (cue, (min, max)) in cues.iter().zip([(0.2, 0.5), (0.1, 0.4), (3.9, 4.3)]) {
				let seconds = cue.len() as f64 / f64::from(rate);
				assert!(
					(min..max).contains(&seconds),
					"unexpected cue duration {seconds}"
				);
				assert!(cue.len() <= rate as usize * 5);
				assert!(
					cue.iter()
						.flatten()
						.all(|s| s.is_finite() && s.abs() <= 1.0)
				);
				assert!(cue.iter().flatten().any(|s| s.abs() > 0.01));
				assert!(
					Duration::from_secs_f64(seconds) + Duration::from_millis(100) < RING_INTERVAL
				);
			}
		}
		assert!(samples(Sound::Message.into(), 48000, &|| false).is_err());
		assert!(samples(Sound::Message.into(), 0, &|| true).is_err());
	}
	#[test]
	fn voice_cues_are_short_bounded_distinct_and_cancelable() {
		let cues = [
			Cue::Mute,
			Cue::Unmute,
			Cue::Deafen,
			Cue::Undeafen,
			Cue::Join,
			Cue::Leave,
			Cue::StreamStart,
		];
		let rendered = cues.map(|cue| samples(cue, 48000, &|| true).unwrap());
		for cue in &rendered {
			assert!((4800..=9600).contains(&cue.len()));
			assert!(
				cue.iter()
					.flatten()
					.all(|sample| sample.is_finite() && sample.abs() <= 0.12)
			);
			assert!(cue.iter().flatten().any(|sample| sample.abs() > 0.01));
		}
		for pair in rendered.windows(2) {
			assert_ne!(pair[0], pair[1]);
		}
		assert!(samples(Cue::Join, 48000, &|| false).is_err());
	}
}
