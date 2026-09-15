use crate::screen::{AudioChunk, MAX_AUDIO_SAMPLES};
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::mpsc::Sender;

#[cfg(target_os = "windows")]
pub(super) struct Audio {
	_stream: cpal::Stream,
	failed: Arc<AtomicBool>,
}

#[cfg(target_os = "windows")]
impl Audio {
	pub(super) fn start(
		send: Sender<AudioChunk>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		audio_epoch: Arc<AtomicU64>,
	) -> Result<Self, &'static str> {
		use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

		// CPAL's WASAPI backend enables loopback when an output device is opened for input.
		// ponytail: capture the default output mix, including Serein; process exclusion needs
		// Windows 10 build 20348's separate process-loopback activation API.
		let device = cpal::default_host()
			.default_output_device()
			.ok_or("System audio requires an available default output device")?;
		let failed = Arc::new(AtomicBool::new(false));
		let input = Samples {
			send,
			stop: stop.clone(),
			ready,
			epoch: audio_epoch,
			failed: failed.clone(),
		};
		let failure = failed.clone();
		let stream = device
			.build_input_stream(
				cpal::StreamConfig {
					channels: 2,
					sample_rate: 48_000,
					buffer_size: cpal::BufferSize::Default,
				},
				move |data: &[f32], _| input.push(data),
				move |_| {
					failure.store(true, Ordering::Release);
					stop.store(true, Ordering::Release);
				},
				Some(std::time::Duration::from_secs(3)),
			)
			.map_err(|_| {
				"System audio could not start; set the default output to stereo, 48 kHz, or share without audio"
			})?;
		stream.play().map_err(|_| "System audio could not start")?;
		Ok(Self {
			_stream: stream,
			failed,
		})
	}

	pub(super) fn failed(&self) -> bool {
		self.failed.load(Ordering::Acquire)
	}
}

struct Samples {
	send: Sender<AudioChunk>,
	stop: Arc<AtomicBool>,
	ready: Arc<AtomicBool>,
	epoch: Arc<AtomicU64>,
	failed: Arc<AtomicBool>,
}

impl Samples {
	fn push(&self, data: &[f32]) {
		let epoch = self.epoch.load(Ordering::Acquire);
		if self.stop.load(Ordering::Acquire) || !self.ready.load(Ordering::Acquire) {
			return;
		}
		if data.len() > MAX_AUDIO_SAMPLES || !data.len().is_multiple_of(2) {
			self.failed.store(true, Ordering::Release);
			self.stop.store(true, Ordering::Release);
			return;
		}
		if data.is_empty() {
			return;
		}
		// Reserve first: a stalled transport must not allocate in the audio callback.
		if let Ok(permit) = self.send.try_reserve() {
			let samples = data
				.iter()
				.map(|&sample| {
					if sample.is_finite() {
						sample.clamp(-1.0, 1.0)
					} else {
						0.0
					}
				})
				.collect();
			if !self.stop.load(Ordering::Acquire) && self.ready.load(Ordering::Acquire) {
				permit.send(AudioChunk { samples, epoch });
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn loopback_samples_are_gated_sanitized_and_bounded() {
		let (send, mut receive) = tokio::sync::mpsc::channel(1);
		let samples = Samples {
			send,
			stop: Arc::new(AtomicBool::new(false)),
			ready: Arc::new(AtomicBool::new(false)),
			epoch: Arc::new(AtomicU64::new(7)),
			failed: Arc::new(AtomicBool::new(false)),
		};
		samples.push(&[0.25, -0.25]);
		assert!(receive.try_recv().is_err());
		samples.ready.store(true, Ordering::Release);
		samples.push(&[f32::NAN, f32::INFINITY, -2.0, 2.0]);
		samples.push(&[0.5, -0.5]); // Full queue drops the new chunk.
		samples.epoch.store(8, Ordering::Release);
		let chunk = receive.try_recv().unwrap();
		assert_eq!(chunk.samples, vec![0.0, 0.0, -1.0, 1.0]);
		assert_eq!(chunk.epoch, 7); // Queued samples retain their capture generation.
		assert!(receive.try_recv().is_err());
		samples.push(&vec![0.0; MAX_AUDIO_SAMPLES]);
		let chunk = receive.try_recv().unwrap();
		assert_eq!(chunk.samples.len(), MAX_AUDIO_SAMPLES);
		assert_eq!(chunk.epoch, 8);
		samples.stop.store(true, Ordering::Release);
		samples.push(&[0.25, -0.25]);
		assert!(receive.try_recv().is_err());
		samples.stop.store(false, Ordering::Release);
		samples.push(&vec![0.0; MAX_AUDIO_SAMPLES + 2]);
		assert!(samples.failed.load(Ordering::Acquire));
		assert!(samples.stop.load(Ordering::Acquire));
		assert!(receive.try_recv().is_err());
	}
}
