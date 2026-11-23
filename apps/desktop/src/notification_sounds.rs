//! One bounded, lazy audio worker. Cues never interrupt attachment playback.
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

/// What the worker should play: a bundled cue, or a soundboard clip already fetched into memory.
#[derive(Clone)]
pub enum Cue {
	Bundled(Sound),
	/// Encoded soundboard audio, bounded before decoding by [`model::MAX_SOUND_UPLOAD_BYTES`].
	#[allow(dead_code)]
	// The soundboard UI and CDN fetcher will connect this staged playback path.
	Clip(Arc<[u8]>),
}

#[derive(Default)]
pub struct Sounds {
	send: Option<SyncSender<(u64, Cue, u8)>>,
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
	/// The failure status alone, so a caller can prefer it over another worker's progress text.
	pub fn error(&self) -> &'static str {
		if self.status.load(Ordering::Acquire) == 2 {
			"Audio output unavailable. Check your system sound settings."
		} else {
			""
		}
	}
	pub fn stop(&mut self) {
		self.generation.fetch_add(1, Ordering::AcqRel);
		self.status.store(0, Ordering::Release);
	}
	pub fn play(&mut self, sound: Sound, volume: u8, ctx: &eframe::egui::Context) {
		self.play_cue(Cue::Bundled(sound), volume, ctx);
	}

	/// Plays one fetched soundboard clip through the same bounded worker as the bundled cues.
	#[allow(dead_code)] // Connected by the forthcoming soundboard fetcher.
	pub fn play_clip(&mut self, audio: Arc<[u8]>, volume: u8, ctx: &eframe::egui::Context) {
		if audio.is_empty() || audio.len() > model::MAX_SOUND_UPLOAD_BYTES {
			self.status.store(2, Ordering::Release);
			ctx.request_repaint();
			return;
		}
		self.play_cue(Cue::Clip(audio), volume, ctx);
	}

	fn play_cue(&mut self, cue: Cue, volume: u8, ctx: &eframe::egui::Context) {
		if self.send.is_none() {
			let (send, receive) = mpsc::sync_channel::<(u64, Cue, u8)>(1);
			let generation = self.generation.clone();
			let status = self.status.clone();
			let context = ctx.clone();
			if std::thread::Builder::new()
				.name("serein-notification-audio".into())
				.spawn(move || {
					while let Ok((request, cue, volume)) = receive.recv() {
						if generation.load(Ordering::Acquire) != request {
							continue;
						}
						let finished = Arc::new(AtomicBool::new(false));
						match open(
							cue,
							volume,
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
			.try_send((request, cue, volume))
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

fn samples(sound: Sound, rate: u32, current: &impl Fn() -> bool) -> Result<Vec<[f32; 2]>, ()> {
	let bytes: &[u8] = match sound {
		Sound::Message => include_bytes!("../../../assets/sounds/discord/message.mp3"),
		Sound::CurrentChannel => {
			include_bytes!("../../../assets/sounds/discord/current-channel.mp3")
		}
		Sound::IncomingRing => include_bytes!("../../../assets/sounds/discord/incoming-ring.mp3"),
		Sound::OutgoingRing => include_bytes!("../../../assets/sounds/discord/outgoing-ring.mp3"),
		Sound::Mute => include_bytes!("../../../assets/sounds/discord/mute.mp3"),
		Sound::Unmute => include_bytes!("../../../assets/sounds/discord/unmute.mp3"),
		Sound::Deafen => include_bytes!("../../../assets/sounds/discord/deafen.mp3"),
		Sound::Undeafen => include_bytes!("../../../assets/sounds/discord/undeafen.mp3"),
		Sound::CameraOn => include_bytes!("../../../assets/sounds/discord/camera-on.mp3"),
		Sound::ScreenShareOn => {
			include_bytes!("../../../assets/sounds/discord/screen-share-on.mp3")
		}
		Sound::ScreenShareOff => {
			include_bytes!("../../../assets/sounds/discord/screen-share-off.mp3")
		}
		Sound::UserJoin => include_bytes!("../../../assets/sounds/discord/user-join.mp3"),
		Sound::UserLeave => include_bytes!("../../../assets/sounds/discord/user-leave.mp3"),
		Sound::PttStart => include_bytes!("../../../assets/sounds/discord/ptt-start.mp3"),
		Sound::PttStop => include_bytes!("../../../assets/sounds/discord/ptt-stop.mp3"),
		Sound::ViewerJoin => include_bytes!("../../../assets/sounds/discord/viewer-join.mp3"),
	};
	decode_samples(bytes, 128 * 1024, rate, current, true)
}

fn clip_samples(
	bytes: Arc<[u8]>,
	rate: u32,
	current: &impl Fn() -> bool,
) -> Result<Vec<[f32; 2]>, ()> {
	decode_samples(bytes, model::MAX_SOUND_UPLOAD_BYTES, rate, current, false)
}

fn decode_samples<B: AsRef<[u8]> + Send + Sync + 'static>(
	bytes: B,
	max_bytes: usize,
	rate: u32,
	current: &impl Fn() -> bool,
	bundled: bool,
) -> Result<Vec<[f32; 2]>, ()> {
	if bytes.as_ref().is_empty()
		|| bytes.as_ref().len() > max_bytes
		|| !(8000..=192000).contains(&rate)
	{
		return Err(());
	}
	let mut pcm = Vec::new();
	let mut source_rate = 0;
	let mut source_channels = 0;
	crate::audio::decode_stream(
		Box::new(Cursor::new(bytes)),
		current,
		&mut |chunk, channels, rate, _| {
			// Soundboard clips are documented as at most 5.2 seconds; use integer
			// tenths to keep the decoded limit stable across sample rates.
			let max_frames = if bundled {
				rate as usize * 6
			} else {
				(rate as usize * 52).div_ceil(10)
			};
			if !(1..=2).contains(&channels)
				|| !(8000..=96000).contains(&rate)
				|| (bundled && (channels != 2 || !matches!(rate, 44100 | 48000)))
				|| (source_rate != 0 && source_rate != rate)
				|| (source_channels != 0 && source_channels != channels)
				|| chunk.len() % channels != 0
				|| chunk.len() > max_frames * channels - pcm.len()
			{
				return Err("Invalid or oversized notification sound");
			}
			source_rate = rate;
			source_channels = channels;
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
	let frames = pcm.len() / source_channels;
	// ponytail: linear rate conversion; use a band-limited resampler if quality measurements require it.
	let output_frames = (frames * rate as usize).div_ceil(source_rate as usize);
	let max_output_frames = if bundled {
		rate as usize * 6
	} else {
		(rate as usize * 52).div_ceil(10)
	};
	if output_frames > max_output_frames {
		return Err(());
	}
	Ok((0..output_frames)
		.map(|frame| {
			let position = frame as f64 * f64::from(source_rate) / f64::from(rate);
			let index = (position as usize).min(frames - 1);
			let next = (index + 1).min(frames - 1);
			std::array::from_fn(|channel| {
				let channel = channel.min(source_channels - 1);
				let a = pcm[index * source_channels + channel];
				let b = pcm[next * source_channels + channel];
				a + (b - a) * (position - index as f64) as f32
			})
		})
		.collect())
}
fn open(
	cue: Cue,
	volume: u8,
	generation: Arc<AtomicU64>,
	request: u64,
	status: Arc<AtomicU8>,
	finished: Arc<AtomicBool>,
) -> Result<(cpal::Stream, Duration), ()> {
	let device = cpal::default_host().default_output_device().ok_or(())?;
	let supported = platform::audio::output_config(
		&device,
		platform::audio::OutputConfigPreference::Notification,
	)
	.map_err(|_| ())?;
	let config = supported.config();
	let current = || generation.load(Ordering::Acquire) == request;
	let samples = match cue {
		Cue::Bundled(sound) => samples(sound, config.sample_rate, &current),
		Cue::Clip(bytes) => clip_samples(bytes, config.sample_rate, &current),
	}?;
	let duration = Duration::from_secs_f64(samples.len() as f64 / f64::from(config.sample_rate));
	let stream = match supported.sample_format() {
		cpal::SampleFormat::F32 => output::<f32>(
			&device,
			config,
			samples,
			volume,
			(generation, request),
			status,
			finished,
		),
		cpal::SampleFormat::I16 => output::<i16>(
			&device,
			config,
			samples,
			volume,
			(generation, request),
			status,
			finished,
		),
		cpal::SampleFormat::I32 => output::<i32>(
			&device,
			config,
			samples,
			volume,
			(generation, request),
			status,
			finished,
		),
		cpal::SampleFormat::U16 => output::<u16>(
			&device,
			config,
			samples,
			volume,
			(generation, request),
			status,
			finished,
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
	volume: u8,
	(generation, request): (Arc<AtomicU64>, u64),
	status: Arc<AtomicU8>,
	finished: Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error> {
	let errors = generation.clone();
	let failed = finished.clone();
	device.build_output_stream(
		config,
		callback::<T>(config, samples, volume, generation, request, finished),
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
	volume: u8,
	generation: Arc<AtomicU64>,
	request: u64,
	finished: Arc<AtomicBool>,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
	let mut position = 0;
	let mut end = None;
	let gain = (f32::from(volume) / 100.0).clamp(0.0, 1.0);
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
				frame[0] = T::from_sample((sample[0] + sample[1]) * 0.5 * gain);
			} else {
				for (target, value) in frame.iter_mut().zip(sample) {
					*target = T::from_sample(*value * gain);
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

	fn wav(rate: u32, channels: u16, pcm: &[i16]) -> Vec<u8> {
		let data_bytes = u32::try_from(pcm.len() * 2).unwrap();
		let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
		bytes.extend_from_slice(b"RIFF");
		bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
		bytes.extend_from_slice(b"WAVEfmt ");
		bytes.extend_from_slice(&16_u32.to_le_bytes());
		bytes.extend_from_slice(&1_u16.to_le_bytes());
		bytes.extend_from_slice(&channels.to_le_bytes());
		bytes.extend_from_slice(&rate.to_le_bytes());
		bytes.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
		bytes.extend_from_slice(&(channels * 2).to_le_bytes());
		bytes.extend_from_slice(&16_u16.to_le_bytes());
		bytes.extend_from_slice(b"data");
		bytes.extend_from_slice(&data_bytes.to_le_bytes());
		for sample in pcm {
			bytes.extend_from_slice(&sample.to_le_bytes());
		}
		bytes
	}

	#[test]
	fn soundboard_clip_decodes_mono_at_unusual_rate_and_rejects_oversize() {
		let mono: Arc<[u8]> = wav(11025, 1, &[0, 16384, -16384, 0]).into();
		let frames = clip_samples(mono.clone(), 48000, &|| true).unwrap();
		assert_eq!(frames.len(), 18);
		assert!(frames.iter().all(|frame| frame[0] == frame[1]));
		assert!(frames.iter().flatten().any(|sample| sample.abs() > 0.4));
		assert!(clip_samples(mono.clone(), 48000, &|| false).is_err());
		assert!(clip_samples(Arc::from([]), 48000, &|| true).is_err());
		let long = wav(11025, 1, &vec![0; 11025 * 7]);
		assert!(clip_samples(long.into(), 48000, &|| true).is_err());
		let mut oversized = mono.to_vec();
		oversized.resize(model::MAX_SOUND_UPLOAD_BYTES + 1, 0);
		assert!(clip_samples(oversized.into(), 48000, &|| true).is_err());
	}

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
			100,
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
			100,
			generation.clone(),
			2,
			next_finished.clone(),
		);
		generation.store(2, Ordering::Release);
		let mut cancelled = callback(
			config,
			vec![[1.0, 1.0]; 2],
			100,
			generation.clone(),
			1,
			finished,
		);
		cancelled(&mut data, &info(2000, 3000));
		assert_eq!(data, [0.0; 8]);
		assert!(!next_finished.load(Ordering::Acquire));
		next(&mut data[..2], &info(2000, 3000));
		assert_eq!(&data[..2], &[0.3, 0.3]);
	}

	#[test]
	fn bundled_cues_decode_in_full_at_supported_rates_and_cancel() {
		for rate in [8000, 44100, 48000, 192000] {
			let cues = [
				Sound::Message,
				Sound::CurrentChannel,
				Sound::IncomingRing,
				Sound::Mute,
				Sound::Unmute,
				Sound::Deafen,
				Sound::Undeafen,
				Sound::CameraOn,
				Sound::ScreenShareOn,
				Sound::ScreenShareOff,
				Sound::OutgoingRing,
				Sound::UserJoin,
				Sound::UserLeave,
				Sound::PttStart,
				Sound::PttStop,
				Sound::ViewerJoin,
			]
			.map(|s| samples(s, rate, &|| true).unwrap());
			assert_ne!(cues[0], cues[1]);
			assert_ne!(cues[3], cues[4]);
			assert_ne!(cues[5], cues[6]);
			assert_ne!(cues[8], cues[9]);
			assert_ne!(cues[13], cues[14]);
			let expectations = [
				(0.2, 0.5),
				(0.5, 0.9),
				(5.0, 5.6),
				(0.3, 0.6),
				(0.3, 0.6),
				(0.6, 0.9),
				(0.6, 1.0),
				(0.9, 1.1),
				(1.6, 1.9),
				(1.5, 1.8),
				(2.3, 2.6),
				(1.0, 1.2),
				(0.9, 1.1),
				(0.02, 0.25),
				(0.02, 0.25),
				(1.5, 1.8),
			];
			assert!(
				Duration::from_secs_f64(cues[10].len() as f64 / f64::from(rate))
					+ Duration::from_millis(100)
					< OUTGOING_RING_INTERVAL
			);
			for (index, (cue, (min, max))) in cues.iter().zip(expectations).enumerate() {
				let seconds = cue.len() as f64 / f64::from(rate);
				assert!(
					(min..max).contains(&seconds),
					"unexpected cue {index} duration {seconds} at {rate} Hz"
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
		assert!(samples(Sound::Message, 48000, &|| false).is_err());
		assert!(samples(Sound::Message, 0, &|| true).is_err());
	}
}
