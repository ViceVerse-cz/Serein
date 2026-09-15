//! One bounded decoder and reorder window per authenticated remote speaker.
use crate::{Frame, crypto::MAX_PARTICIPANTS, jitter::Jitter};
use opus2::{Channels, Decoder};

struct Speaker {
	user: u64,
	ssrc: u32,
	decoder: Decoder,
	jitter: Jitter,
	pcm: Box<[f32; 5760]>,
	offset: usize,
	length: usize,
	activity: u8,
}
#[derive(Default)]
pub(crate) struct Mixer {
	speakers: Vec<Speaker>,
}
impl Mixer {
	pub fn announce(&mut self, user: u64, ssrc: u32) -> Result<(), &'static str> {
		if self
			.speakers
			.iter()
			.any(|s| s.ssrc == ssrc && s.user != user)
		{
			return Err("Voice SSRC belongs to another participant");
		}
		if self
			.speakers
			.iter()
			.any(|s| s.user == user && s.ssrc == ssrc)
		{
			return Ok(());
		}
		self.remove(user);
		if self.speakers.len() >= MAX_PARTICIPANTS - 1 {
			return Err("Voice decoder budget exceeded");
		}
		self.speakers.push(Speaker {
			user,
			ssrc,
			decoder: Decoder::new(48_000, Channels::Mono)
				.map_err(|_| "Opus decoder initialization failed")?,
			jitter: Jitter::default(),
			pcm: Box::new([0.0; 5760]),
			offset: 0,
			length: 0,
			activity: 0,
		});
		Ok(())
	}
	pub fn remove(&mut self, user: u64) {
		self.speakers.retain(|s| s.user != user);
	}
	pub fn user(&self, ssrc: u32) -> Option<u64> {
		self.speakers
			.iter()
			.find(|s| s.ssrc == ssrc)
			.map(|s| s.user)
	}
	pub fn push(&mut self, ssrc: u32, sequence: u16, opus: Vec<u8>) {
		if let Some(speaker) = self.speakers.iter_mut().find(|s| s.ssrc == ssrc) {
			speaker.jitter.push(sequence, opus);
		}
	}
	pub fn clear(&mut self) {
		for speaker in &mut self.speakers {
			speaker.jitter.clear();
			speaker.pcm.fill(0.0);
			speaker.offset = 0;
			speaker.length = 0;
			speaker.activity = 0;
		}
	}
	pub fn speaking(&self) -> impl Iterator<Item = u64> + '_ {
		self.speakers
			.iter()
			.filter(|s| s.activity > 0)
			.map(|s| s.user)
	}
	/// Mix one 20ms frame, preserving up to 120ms packets without bursting playback queues.
	pub fn pop(&mut self) -> (Option<Frame>, bool) {
		self.pop_with_volumes(&[])
	}
	pub fn pop_with_volumes(&mut self, volumes: &[(u64, u16)]) -> (Option<Frame>, bool) {
		let mut output = [0.0; 960];
		let mut active = false;
		let mut heard = false;
		for speaker in &mut self.speakers {
			let gain = f32::from(
				volumes
					.iter()
					.take(64)
					.find(|(user, _)| *user == speaker.user)
					.map_or(100, |(_, percent)| (*percent).min(200)),
			) / 100.0;
			let mut energy = 0.0;
			let mut filled = 0;
			let mut decoded = 0;
			while filled < output.len() {
				if speaker.offset == speaker.length {
					// The shortest supported packet is 2.5ms: at most eight decodes per tick.
					if decoded == 8 {
						break;
					}
					let Some(opus) = speaker.jitter.pop() else {
						break;
					};
					decoded += 1;
					let limit = if opus.is_empty() {
						// Preserve the lost packet duration; long concealment drains over 20ms ticks.
						speaker
							.decoder
							.get_last_packet_duration()
							.unwrap_or(960)
							.clamp(120, 5760) as usize
					} else {
						speaker.pcm.len()
					};
					let Ok(length) =
						speaker
							.decoder
							.decode_float(&opus, &mut speaker.pcm[..limit], false)
					else {
						continue;
					};
					speaker.offset = 0;
					speaker.length = length;
					heard |= !opus.is_empty() && opus != davey::OPUS_SILENCE_PACKET;
				}
				let count = (output.len() - filled).min(speaker.length - speaker.offset);
				for (mixed, sample) in output[filled..filled + count]
					.iter_mut()
					.zip(&speaker.pcm[speaker.offset..speaker.offset + count])
				{
					if sample.is_finite() {
						*mixed += sample * gain;
						energy += sample * sample;
					}
				}
				speaker.offset += count;
				filled += count;
				active |= count != 0;
			}
			speaker.activity = crate::activity::hold(energy, speaker.activity);
		}
		// ponytail: hard limiting bounds simultaneous speakers; add a soft limiter if clipping is audible.
		output
			.iter_mut()
			.for_each(|sample| *sample = sample.clamp(-1.0, 1.0));
		(active.then_some(output), heard)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use opus2::{Application, Encoder};
	fn opus(frequency: f32) -> Vec<u8> {
		let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Voip).unwrap();
		let pcm: Frame = std::array::from_fn(|i| (i as f32 * frequency).sin() * 0.2);
		let mut encoded = [0; 1275];
		let length = encoder.encode_float(&pcm, &mut encoded).unwrap();
		encoded[..length].to_vec()
	}
	#[test]
	fn per_user_volume_is_independent_live_and_limited() {
		let mut mixer = Mixer::default();
		mixer.announce(1, 11).unwrap();
		mixer.announce(2, 22).unwrap();
		for (volumes, expected) in [
			(vec![], 0.3),
			(vec![(1, 0)], 0.2),
			(vec![(1, 50), (2, 200)], 0.45),
			(vec![(1, 200), (2, 0)], 0.2),
			(vec![(1, 0), (2, 0)], 0.0),
			(vec![(99, 0)], 0.3),
			(vec![(1, u16::MAX)], 0.4),
		] {
			for speaker in &mut mixer.speakers {
				speaker.pcm.fill(speaker.user as f32 * 0.1);
				speaker.offset = 0;
				speaker.length = 960;
			}
			let frame = mixer.pop_with_volumes(&volumes).0.unwrap();
			assert!(
				frame
					.iter()
					.all(|sample| (*sample - expected).abs() < 0.0001)
			);
			assert_eq!(mixer.speaking().count(), 2);
		}
		for speaker in &mut mixer.speakers {
			speaker.pcm.fill(0.8);
			speaker.offset = 0;
		}
		assert!(
			mixer
				.pop_with_volumes(&[(1, 200)])
				.0
				.unwrap()
				.iter()
				.all(|s| *s == 1.0)
		);
	}

	#[test]
	fn independent_streams_mix_on_one_clock_and_release_on_leave() {
		let mut together = Mixer::default();
		let mut alice = Mixer::default();
		let mut bob = Mixer::default();
		for (user, ssrc, single, frequency) in [(1, 11, &mut alice, 0.03), (2, 22, &mut bob, 0.08)]
		{
			together.announce(user, ssrc).unwrap();
			single.announce(user, ssrc).unwrap();
			// Same sequence from separate SSRCs must never collide or share decoder state.
			let packet = opus(frequency);
			together.push(ssrc, 7, packet.clone());
			single.push(ssrc, 7, packet);
		}
		assert!(together.announce(3, 11).is_err());
		for _ in 0..2 {
			assert!(together.pop().0.is_none());
			assert!(alice.pop().0.is_none());
			assert!(bob.pop().0.is_none());
		}
		let mixed = together.pop().0.unwrap();
		let a = alice.pop().0.unwrap();
		let b = bob.pop().0.unwrap();
		assert!(a.iter().any(|s| s.abs() > 0.01));
		assert!(b.iter().any(|s| s.abs() > 0.01));
		for i in 0..960 {
			assert!((mixed[i] - (a[i] + b[i]).clamp(-1.0, 1.0)).abs() < 0.0001);
		}
		together.remove(1);
		assert_eq!(together.user(11), None);
		together.clear();
		assert!(together.pop().0.is_none());
		for user in 3..65 {
			together.announce(user, user as u32 + 100).unwrap();
		}
		assert!(together.announce(65, 165).is_err());
		assert_eq!(together.speakers.len(), 63);
	}
	#[test]
	fn short_packets_fill_realtime_ticks_without_silence_or_reorder_starvation() {
		for samples in [120, 240, 480, 960, 2880] {
			let mut mixer = Mixer::default();
			mixer.announce(1, 11).unwrap();
			let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Audio).unwrap();
			let mut reference = Decoder::new(48_000, Channels::Mono).unwrap();
			let mut expected = std::collections::VecDeque::new();
			let mut sequence = 0;
			let mut played = 0;
			for tick in 0..18 {
				// Packets arrive at their actual duration, before the shared 20ms playout tick.
				let packets = if samples <= 960 {
					960 / samples
				} else {
					usize::from(tick % 3 == 0)
				};
				for _ in 0..packets {
					let pcm: Vec<_> = (0..samples)
						.map(|index| {
							((usize::from(sequence) * samples + index) as f32 * 0.037).sin() * 0.2
						})
						.collect();
					let mut encoded = [0; 1275];
					let len = encoder.encode_float(&pcm, &mut encoded).unwrap();
					let packet = &encoded[..len];
					let mut decoded = [0.0; 5760];
					let length = reference.decode_float(packet, &mut decoded, false).unwrap();
					assert_eq!(length, samples);
					expected.extend(decoded[..length].iter().copied());
					mixer.push(11, sequence, packet.to_vec());
					sequence = sequence.wrapping_add(1);
				}
				if let Some(frame) = mixer.pop().0 {
					played += 1;
					for actual in frame {
						let expected = expected
							.pop_front()
							.expect("playout cannot outrun received PCM");
						assert!(
							(actual - expected).abs() < 0.0001,
							"duration={samples} tick={tick}"
						);
					}
				}
			}
			assert!(
				played >= 16,
				"duration={samples}: short packets must not starve"
			);
		}
	}

	#[test]
	fn packet_loss_preserves_short_and_long_packet_timing() {
		for samples in [240, 2880, 5760] {
			let mut mixer = Mixer::default();
			mixer.announce(1, 11).unwrap();
			let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Audio).unwrap();
			encoder.set_bitrate(opus2::Bitrate::Bits(32_000)).unwrap();
			let mut reference = Decoder::new(48_000, Channels::Mono).unwrap();
			let mut expected = Vec::new();
			for sequence in 0..4 {
				let mut encoded = [0; 1275];
				let input: Vec<f32> = (0..samples.min(2880))
					.map(|i| (i as f32 * 0.037).sin() * 0.2)
					.collect();
				let mut len = encoder.encode_float(&input, &mut encoded).unwrap();
				if samples == 5760 {
					// A valid 120ms Opus packet can aggregate two matching 60ms packets.
					let packet = encoded[..len].to_vec();
					len = opus2::Repacketizer::new()
						.unwrap()
						.combine(&[&packet, &packet], &mut encoded)
						.unwrap();
				}
				let mut decoded = vec![0.0; samples];
				let packet = if sequence == 1 {
					&[][..]
				} else {
					&encoded[..len]
				};
				assert_eq!(
					reference.decode_float(packet, &mut decoded, false).unwrap(),
					samples
				);
				expected.extend(decoded);
				if sequence != 1 {
					mixer.push(11, sequence, packet.to_vec());
				}
			}
			assert!(mixer.pop().0.is_none());
			assert!(mixer.pop().0.is_none());
			for expected in expected.as_chunks::<960>().0 {
				let frame = mixer.pop().0.expect("one 20ms frame per playout tick");
				for (actual, expected) in frame.iter().zip(expected) {
					assert!(
						(actual - expected).abs() < 0.0001,
						"packet duration={samples}"
					);
				}
			}
		}
	}

	#[test]
	#[ignore = "synthetic release workload; run with --release --ignored --nocapture"]
	fn synthetic_mix_workload() {
		let packet = opus(0.05);
		for peers in [1, 8, 63] {
			let mut mixer = Mixer::default();
			for user in 1..=peers {
				mixer.announce(user, user as u32).unwrap();
			}
			let mut sequence = 0u16;
			let mut samples = Vec::new();
			for run in 0..6 {
				let start = std::time::Instant::now();
				for _ in 0..1000 {
					for user in 1..=peers {
						mixer.push(user as u32, sequence, packet.clone());
					}
					std::hint::black_box(mixer.pop());
					sequence = sequence.wrapping_add(1);
				}
				if run > 0 {
					samples.push(start.elapsed().as_micros());
				}
			}
			samples.sort_unstable();
			println!(
				"synthetic_mix peers={peers} ticks=1000 samples=5 median_us={} us_per_tick={:.2}",
				samples[2],
				samples[2] as f64 / 1000.0
			);
		}
	}
}
