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

// Each complete ringtone plus a short gap; shared with the automatic call timers.
pub const RING_INTERVAL: Duration = Duration::from_secs(6);
pub const OUTGOING_RING_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct Sounds {
	send: Option<SyncSender<(u64, Sound, bool)>>,
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
	pub fn play(&mut self, sound: Sound, discord: bool, ctx: &eframe::egui::Context) {
		if self.send.is_none() {
			let (send, receive) = mpsc::sync_channel::<(u64, Sound, bool)>(1);
			let generation = self.generation.clone();
			let status = self.status.clone();
			let context = ctx.clone();
			if std::thread::Builder::new()
				.name("serein-notification-audio".into())
				.spawn(move || {
					while let Ok((request, sound, discord)) = receive.recv() {
						if generation.load(Ordering::Acquire) != request {
							continue;
						}
						let finished = Arc::new(AtomicBool::new(false));
						match open(
							sound,
							discord,
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
			.try_send((request, sound, discord))
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

fn samples(
	sound: Sound,
	discord: bool,
	rate: u32,
	current: &impl Fn() -> bool,
) -> Result<Vec<[f32; 2]>, ()> {
	let bytes: &[u8] = if discord {
		match sound {
			Sound::Message | Sound::CurrentChannel => {
				include_bytes!("../../../assets/sounds/discord/message.mp3")
			}
			Sound::IncomingRing => {
				include_bytes!("../../../assets/sounds/discord/incoming-ring.mp3")
			}
			Sound::OutgoingRing => {
				include_bytes!("../../../assets/sounds/discord/outgoing-ring.mp3")
			}
			Sound::Mute => include_bytes!("../../../assets/sounds/discord/mute.mp3"),
			Sound::Unmute => include_bytes!("../../../assets/sounds/discord/unmute.mp3"),
			Sound::Deafen => include_bytes!("../../../assets/sounds/discord/deafen.mp3"),
			Sound::Undeafen => include_bytes!("../../../assets/sounds/discord/undeafen.mp3"),
			Sound::CameraOn => include_bytes!("../../../assets/sounds/discord/camera-on.mp3"),
			Sound::ScreenShareOn => {
				include_bytes!("../../../assets/sounds/discord/screen-share-on.mp3")
			}
			Sound::UserJoin => include_bytes!("../../../assets/sounds/discord/user-join.mp3"),
			Sound::UserLeave => include_bytes!("../../../assets/sounds/discord/user-leave.mp3"),
		}
	} else {
		match sound {
			Sound::Message => include_bytes!("../../../assets/sounds/message.mp3"),
			Sound::CurrentChannel => include_bytes!("../../../assets/sounds/current-channel.mp3"),
			Sound::IncomingRing => include_bytes!("../../../assets/sounds/incoming-ring.mp3"),
			Sound::OutgoingRing
			| Sound::Mute
			| Sound::Unmute
			| Sound::Deafen
			| Sound::Undeafen
			| Sound::CameraOn
			| Sound::ScreenShareOn
			| Sound::UserJoin
			| Sound::UserLeave => return Err(()),
		}
	};
	if bytes.len() > 128 * 1024 || !(8000..=192000).contains(&rate) {
		return Err(());
	}
	let mut pcm = Vec::new();
	let mut source_rate = 0;
	crate::audio::decode_stream(
		Box::new(Cursor::new(bytes)),
		current,
		&mut |chunk, channels, rate, _| {
			if channels != 2
				|| !matches!(rate, 44100 | 48000)
				|| (source_rate != 0 && source_rate != rate)
				|| pcm.len() + chunk.len() > rate as usize * 2 * 6
			{
				return Err("Invalid bundled notification sound");
			}
			source_rate = rate;
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
	Ok((0..(frames * rate as usize).div_ceil(source_rate as usize))
		.map(|frame| {
			let position = frame as f64 * f64::from(source_rate) / f64::from(rate);
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
fn open(
	sound: Sound,
	discord: bool,
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
	let samples = samples(sound, discord, config.sample_rate, &|| {
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
			let default_cues = [Sound::Message, Sound::CurrentChannel, Sound::IncomingRing]
				.map(|s| samples(s, false, rate, &|| true).unwrap());
			assert_ne!(default_cues[0], default_cues[1]);
			for (cue, (min, max)) in default_cues
				.iter()
				.zip([(0.2, 0.5), (0.1, 0.4), (3.9, 4.3)])
			{
				let seconds = cue.len() as f64 / f64::from(rate);
				assert!(
					(min..max).contains(&seconds),
					"unexpected cue duration {seconds}"
				);
				assert!(cue.len() <= rate as usize * 6);
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
			assert!(samples(Sound::Mute, false, rate, &|| true).is_err());
			assert!(samples(Sound::Unmute, false, rate, &|| true).is_err());
			assert!(samples(Sound::Deafen, false, rate, &|| true).is_err());
			assert!(samples(Sound::Undeafen, false, rate, &|| true).is_err());
			assert!(samples(Sound::CameraOn, false, rate, &|| true).is_err());
			assert!(samples(Sound::ScreenShareOn, false, rate, &|| true).is_err());
			assert!(samples(Sound::OutgoingRing, false, rate, &|| true).is_err());
			assert!(samples(Sound::UserJoin, false, rate, &|| true).is_err());
			assert!(samples(Sound::UserLeave, false, rate, &|| true).is_err());

			let discord_cues = [
				Sound::Message,
				Sound::CurrentChannel,
				Sound::IncomingRing,
				Sound::Mute,
				Sound::Unmute,
				Sound::Deafen,
				Sound::Undeafen,
				Sound::CameraOn,
				Sound::ScreenShareOn,
				Sound::OutgoingRing,
				Sound::UserJoin,
				Sound::UserLeave,
			]
			.map(|s| samples(s, true, rate, &|| true).unwrap());
			assert_eq!(discord_cues[0], discord_cues[1]);
			assert_ne!(discord_cues[3], discord_cues[4]);
			assert_ne!(discord_cues[5], discord_cues[6]);
			let discord_expectations = [
				(0.2, 0.5),
				(0.2, 0.5),
				(5.0, 5.6),
				(0.3, 0.6),
				(0.3, 0.6),
				(0.6, 0.9),
				(0.6, 1.0),
				(0.9, 1.1),
				(1.6, 1.9),
				(2.3, 2.6),
				(1.0, 1.2),
				(0.9, 1.1),
			];
			assert!(
				Duration::from_secs_f64(discord_cues[9].len() as f64 / f64::from(rate))
					+ Duration::from_millis(100)
					< OUTGOING_RING_INTERVAL
			);
			for (cue, (min, max)) in discord_cues.iter().zip(discord_expectations) {
				let seconds = cue.len() as f64 / f64::from(rate);
				assert!(
					(min..max).contains(&seconds),
					"unexpected cue duration {seconds}"
				);
				assert!(cue.len() <= rate as usize * 6);
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
		assert!(samples(Sound::Message, false, 48000, &|| false).is_err());
		assert!(samples(Sound::Message, true, 48000, &|| false).is_err());
		assert!(samples(Sound::Message, false, 0, &|| true).is_err());
		assert!(samples(Sound::Message, true, 0, &|| true).is_err());
	}
}
