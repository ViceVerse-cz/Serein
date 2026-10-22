//! Timestamp-preserving audio for a watched stream, separate from normal call mixing.
//! Bounded compressed reordering and one Opus packet of PCM per announced source.
use crate::{Frame, stream_sync::Sync, video_receive::MAX_SOURCES};
use opus2::{Channels, Decoder};
use std::time::Instant;

// 320 ms at the shortest legal 2.5 ms packetization: enough for the
// 240 ms adaptive target plus scheduling/reorder headroom. Bytes remain bounded.
const PACKETS: usize = 128;
struct Packet {
	sequence: u16,
	timestamp: u32,
	opus: Box<[u8]>,
}
// Per-window numeric aggregates: decoded/concealed samples, decode failures,
// rebufferings, faster/slower input samples, maximum buffered/target milliseconds.
struct Source {
	stats: [u64; 8],
	started: bool,
	level_error: f64,
	presentation: Option<crate::Presentation>,
	user: u64,
	ssrc: u32,
	decoder: Decoder,
	packets: Vec<Packet>,
	next: Option<u16>,
	cursor: Option<u32>,
	pcm: Box<[f32; 5760]>,
	pcm_timestamp: u32,
	offset: usize,
	length: usize,
	missing: u8,
}
impl Source {
	fn clear(&mut self) {
		self.stats[3] += u64::from(self.cursor.is_some());
		self.started = false;
		self.level_error = 0.0;
		self.presentation = None;
		self.packets.clear();
		self.next = None;
		self.cursor = None;
		self.pcm.fill(0.0);
		self.offset = 0;
		self.length = 0;
		self.missing = 0;
		let _ = self.decoder.reset_state();
	}
	fn first(&self) -> Option<usize> {
		let next = self.next?;
		self.packets
			.iter()
			.enumerate()
			.min_by_key(|(_, packet)| packet.sequence.wrapping_sub(next))
			.map(|(index, _)| index)
	}
	fn push(
		&mut self,
		sequence: u16,
		timestamp: u32,
		opus: Vec<u8>,
		clock: &mut Sync,
		now: Instant,
	) {
		if opus.is_empty() || opus.len() > 1275 {
			return;
		}
		if let Some(next) = self.next {
			let distance = sequence.wrapping_sub(next);
			if distance >= 32768 {
				return;
			}
			if distance >= PACKETS as u16
				|| timestamp.wrapping_sub(self.cursor.unwrap_or(timestamp)) < 0x8000_0000
					&& timestamp.wrapping_sub(self.cursor.unwrap_or(timestamp)) > 24_000
			{
				self.clear();
			}
		}
		if self.packets.len() == PACKETS
			|| self
				.packets
				.iter()
				.any(|packet| packet.sequence == sequence)
		{
			return;
		}
		self.next.get_or_insert(sequence);
		if self.cursor.is_none() {
			self.cursor = Some(timestamp);
			// Restart the scheduling epoch with the compressed buffer. A retained
			// future deadline can outlive this queue's bounded packet window.
			clock.restart_audio(self.ssrc);
			clock.deadline(self.user, self.ssrc, 48_000, timestamp, now);
		}
		clock.observe_audio(self.ssrc, timestamp, now);
		self.packets.push(Packet {
			sequence,
			timestamp,
			opus: opus.into_boxed_slice(),
		});
	}
	fn pop(&mut self, clock: &mut Sync, now: Instant) -> Option<Frame> {
		let timestamp = self.cursor?;
		let deadline = clock.deadline(self.user, self.ssrc, 48_000, timestamp, now);
		let target = clock.target(self.ssrc);
		let startup = deadline + target.saturating_sub(crate::presentation::INITIAL);
		if !self.started && now < startup {
			return None;
		}
		self.started = true;

		let buffered = self
			.packets
			.iter()
			.map(|packet| packet.timestamp.wrapping_sub(timestamp) as i32)
			.max()
			.unwrap_or(0)
			.max(0) as f64;
		self.stats[6] = self.stats[6].max((buffered / 48.0) as u64);
		self.stats[7] = self.stats[7].max(target.as_millis() as u64);
		let desired = target.as_secs_f64() * 48_000.0;
		self.level_error += (buffered - desired - self.level_error) * 0.02;
		let adjustment = if self.level_error.abs() > 480.0 {
			(self.level_error / 480.0).clamp(-5.0, 5.0) as i32
		} else {
			0
		};
		self.stats[4] += adjustment.max(0) as u64;
		self.stats[5] += (-adjustment).max(0) as u64;
		let wanted = (960 + adjustment) as usize;
		let mut frame = [0.0; 965];
		let mut filled = 0usize;
		let mut decoded = 0;
		while filled < wanted {
			let cursor = timestamp.wrapping_add(filled as u32);
			if self.offset == self.length {
				if decoded == 9 {
					break;
				} // At most nine 2.5 ms packets per output tick, including rate correction.
				decoded += 1;
				let first = self.first();
				let gap =
					first.map(|index| self.packets[index].timestamp.wrapping_sub(cursor) as i32);
				if gap.is_none_or(|gap| gap > 0) {
					if self.missing >= 3 {
						break;
					}
					self.missing += 1;
					let count = gap.map_or(960, |gap| gap as usize).min(960);
					self.pcm_timestamp = cursor;
					self.length = self
						.decoder
						.decode_float(&[], &mut self.pcm[..count], false)
						.inspect_err(|_| self.stats[2] += 1)
						.unwrap_or(0);
					self.stats[1] += self.length as u64;
					self.offset = 0;
					if self.length == 0 {
						filled += count.min(wanted - filled);
						continue;
					}
				} else {
					let packet = self
						.packets
						.swap_remove(first.expect("packet has a timestamp"));
					self.next = Some(packet.sequence.wrapping_add(1));
					self.packets
						.retain(|packet| packet.sequence.wrapping_sub(self.next.unwrap()) < 32768);
					self.pcm_timestamp = packet.timestamp;
					self.length = self
						.decoder
						.decode_float(&packet.opus, self.pcm.as_mut(), false)
						.inspect_err(|_| self.stats[2] += 1)
						.unwrap_or(0);
					self.stats[0] += self.length as u64;
					self.offset = 0;
					self.missing = 0;
					if self.length == 0 {
						continue;
					}
				}
			}
			let delta = self
				.pcm_timestamp
				.wrapping_add(self.offset as u32)
				.wrapping_sub(cursor) as i32;
			if delta > 0 {
				filled += (delta as usize).min(wanted - filled);
				continue;
			}
			self.offset += ((-i64::from(delta)) as usize).min(self.length - self.offset);
			let count = (self.length - self.offset).min(wanted - filled);
			frame[filled..filled + count]
				.copy_from_slice(&self.pcm[self.offset..self.offset + count]);
			self.offset += count;
			filled += count;
		}
		if filled == 0 && self.packets.is_empty() && self.offset == self.length {
			self.stats[3] += 1;
			self.cursor = None;
			self.started = false;
			clock.restart_audio(self.ssrc);
			return None;
		}
		self.cursor = Some(timestamp.wrapping_add(wanted as u32));
		let output = std::array::from_fn(|i| {
			let position = i as f32 * wanted as f32 / 960.0;
			let left = position as usize;
			let right = (left + 1).min(wanted - 1);
			frame[left] + (frame[right] - frame[left]) * position.fract()
		});
		self.presentation = clock.audio_frame(self.user, self.ssrc, timestamp, now);
		Some(output)
	}
}
#[derive(Default)]
pub(crate) struct Audio {
	sources: Vec<Source>,
}
impl Audio {
	pub fn take_stats(&mut self) -> [u64; 8] {
		let mut output = [0; 8];
		for source in &mut self.sources {
			for (index, value) in std::mem::take(&mut source.stats).into_iter().enumerate() {
				if index >= 6 {
					output[index] = output[index].max(value);
				} else {
					output[index] += value;
				}
			}
		}
		output
	}
	pub fn announce(&mut self, user: u64, ssrc: u32) -> Result<(), &'static str> {
		if ssrc == 0 {
			return Ok(());
		}
		if self
			.sources
			.iter()
			.any(|source| source.ssrc == ssrc && source.user != user)
		{
			return Err("Stream audio SSRC belongs to another participant");
		}
		if self
			.sources
			.iter()
			.any(|source| source.ssrc == ssrc && source.user == user)
		{
			return Ok(());
		}
		self.remove(user);
		if self.sources.len() == MAX_SOURCES {
			return Err("Stream audio source budget exceeded");
		}
		self.sources.push(Source {
			stats: [0; 8],
			started: false,
			level_error: 0.0,
			presentation: None,
			user,
			ssrc,
			decoder: Decoder::new(48_000, Channels::Mono)
				.map_err(|_| "Stream audio decoder initialization failed")?,
			packets: Vec::new(),
			next: None,
			cursor: None,
			pcm: Box::new([0.0; 5760]),
			pcm_timestamp: 0,
			offset: 0,
			length: 0,
			missing: 0,
		});
		Ok(())
	}
	pub fn user(&self, ssrc: u32) -> Option<u64> {
		self.sources
			.iter()
			.find(|source| source.ssrc == ssrc)
			.map(|source| source.user)
	}
	pub fn remove(&mut self, user: u64) {
		self.sources.retain(|source| source.user != user);
	}
	pub fn clear(&mut self) {
		for source in &mut self.sources {
			source.clear();
		}
	}
	pub fn push(
		&mut self,
		ssrc: u32,
		sequence: u16,
		timestamp: u32,
		opus: Vec<u8>,
		clock: &mut Sync,
		now: Instant,
	) {
		if let Some(source) = self.sources.iter_mut().find(|source| source.ssrc == ssrc) {
			source.push(sequence, timestamp, opus, clock, now);
		}
	}
	#[cfg(test)]
	pub fn pop(&mut self, clock: &mut Sync, now: Instant) -> Option<Frame> {
		self.pop_timed(clock, now).map(|frame| frame.samples)
	}
	pub fn pop_timed(&mut self, clock: &mut Sync, now: Instant) -> Option<crate::PlaybackFrame> {
		let mut output = [0.0; 960];
		let mut heard = false;
		let mut presentation = None;
		for source in &mut self.sources {
			if let Some(frame) = source.pop(clock, now) {
				heard = true;
				if presentation.is_none() {
					presentation = source.presentation.clone();
				}
				for (mixed, sample) in output.iter_mut().zip(frame) {
					if sample.is_finite() {
						*mixed += sample;
					}
				}
			}
		}
		output
			.iter_mut()
			.for_each(|sample| *sample = sample.clamp(-1.0, 1.0));
		heard.then_some(crate::PlaybackFrame {
			samples: output,
			presentation,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use opus2::{Application, Encoder};
	use std::time::Duration;
	#[test]
	fn audio_rebuffering_keeps_video_and_queued_pcm_current_until_the_session_changes() {
		let start = Instant::now();
		let mut sync = Sync::default();
		let due = sync.deadline(7, 71, 90_000, 0, start);
		let video = sync.presentation(7, due).unwrap();
		let mut audio = Audio::default();
		audio.announce(7, 70).unwrap();
		audio.push(70, 0, 0, vec![0xf8, 0xff, 0xfe], &mut sync, start);
		assert!(video.is_current(), "audio joining invalidated queued video");
		let mut queued = None;
		for n in 5..20 {
			if let Some(frame) = audio.pop_timed(&mut sync, start + Duration::from_millis(n * 20)) {
				queued = frame.presentation;
			}
		}
		assert!(audio.sources[0].cursor.is_none());
		assert!(
			video.is_current(),
			"an audio underrun invalidated queued video"
		);
		assert!(
			queued.as_ref().unwrap().is_current(),
			"rebuffering discarded playable PCM"
		);
		audio.push(
			70,
			1,
			48_000,
			vec![0xf8, 0xff, 0xfe],
			&mut sync,
			start + Duration::from_secs(1),
		);
		assert!(video.is_current());
		sync.clear();
		assert!(!video.is_current());
		assert!(!queued.unwrap().is_current());
	}
	#[test]
	fn reordered_variable_duration_audio_keeps_sample_timestamps_across_wrap() {
		let now = Instant::now();
		let mut audio = Audio::default();
		let mut sync = Sync::default();
		audio.announce(7, 70).unwrap();
		let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Audio).unwrap();
		let mut encoded = [0; 1275];
		let mut packets = Vec::new();
		let start = u32::MAX - 479;
		let mut timestamp = start;
		for (n, samples) in [480, 480, 1920, 960].into_iter().enumerate() {
			let input = vec![0.2; samples];
			let length = encoder.encode_float(&input, &mut encoded).unwrap();
			packets.push((
				u16::MAX.wrapping_add(n as u16),
				timestamp,
				encoded[..length].to_vec(),
			));
			timestamp = timestamp.wrapping_add(samples as u32);
		}
		for index in [0, 2, 1, 3] {
			let (seq, ts, data) = &packets[index];
			audio.push(70, *seq, *ts, data.clone(), &mut sync, now);
		}
		assert!(
			audio
				.pop(&mut sync, now + Duration::from_millis(99))
				.is_none()
		);
		for n in 0..4 {
			let frame = audio
				.pop(
					&mut sync,
					now + super::super::stream_sync::BUFFER + Duration::from_millis(n * 20),
				)
				.unwrap();
			assert!(
				frame
					.iter()
					.all(|sample| sample.is_finite() && sample.abs() <= 1.0)
			);
			assert_eq!(
				audio.sources[0].cursor,
				Some(start.wrapping_add((n as u32 + 1) * 960))
			);
		}
		assert!(audio.sources[0].packets.is_empty());
		assert_eq!(audio.sources[0].offset, audio.sources[0].length);
		audio.clear();
		assert!(audio.pop(&mut sync, now + Duration::from_secs(1)).is_none());
		assert!(audio.sources[0].pcm.iter().all(|sample| *sample == 0.0));
	}
	#[test]
	fn packet_source_and_late_input_bounds_survive_pressure_and_owner_changes() {
		let now = Instant::now();
		let mut audio = Audio::default();
		let mut sync = Sync::default();
		for user in 1..=MAX_SOURCES as u64 {
			audio.announce(user, user as u32).unwrap();
		}
		assert!(audio.announce(99, 99).is_err());
		assert!(audio.announce(99, 1).is_err());
		for n in 0..1000 {
			audio.push(1, n, u32::from(n) * 960, vec![1; 1275], &mut sync, now);
		}
		assert!(audio.sources[0].packets.len() <= PACKETS);
		let next = audio.sources[0].next;
		audio.push(1, 1, 0, vec![1], &mut sync, now);
		assert_eq!(audio.sources[0].next, next);
		audio.remove(1);
		audio.announce(99, 1).unwrap();
		assert_eq!(audio.user(1), Some(99));
		assert!(audio.sources.last().unwrap().packets.is_empty());
	}

	#[test]
	fn audio_restarts_after_a_timestamp_jump_instead_of_chasing_an_unreachable_deadline() {
		let start = Instant::now();
		let mut audio = Audio::default();
		let mut sync = Sync::default();
		audio.announce(7, 70).unwrap();
		let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Audio).unwrap();
		let tone: crate::Frame = std::array::from_fn(|i| (i as f32 * 0.05).sin() * 0.2);
		let mut bytes = [0; 1275];
		let length = encoder.encode_float(&tone, &mut bytes).unwrap();
		let mut audible = 0;
		for n in 0..300u32 {
			let now = start + Duration::from_millis(u64::from(n) * 20);
			// Keep valid Opus and sequence ordering, but shift the sender's RTP epoch
			// forward 200 ms. Packet pressure must restart both buffer and clock.
			let timestamp = n * 960 + if n >= 100 { 9600 } else { 0 };
			audio.push(
				70,
				n as u16,
				timestamp,
				bytes[..length].to_vec(),
				&mut sync,
				now,
			);
			if let Some(frame) = audio.pop(&mut sync, now)
				&& n >= 250 && frame.iter().any(|sample| sample.abs() > 0.001)
			{
				audible += 1;
			}
		}
		assert!(
			audible >= 40,
			"audio did not recover after a bounded packet/clock restart: {audible}"
		);
	}
	#[test]
	fn sustained_short_packets_and_clock_drift_keep_continuous_audio() {
		for samples in [120usize, 240, 480, 960, 1920, 2880] {
			for ppm in [-500i64, 0, 500] {
				let start = Instant::now();
				let mut audio = Audio::default();
				let mut sync = Sync::default();
				audio.announce(7, 70).unwrap();
				let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Audio).unwrap();
				let input: Vec<f32> = (0..samples)
					.map(|n| (n as f32 * 0.05).sin() * 0.2)
					.collect();
				let mut bytes = [0; 1275];
				for _ in 0..10 {
					encoder.encode_float(&input, &mut bytes).unwrap();
				}
				let length = encoder.encode_float(&input, &mut bytes).unwrap();
				let mut packet = 0u64;
				let mut audible = 0;
				let mut peak_target = std::time::Duration::ZERO;
				// Three minutes of synthetic time: rate mismatch used to empty/fill a fixed queue.
				for tick in 0..9000u64 {
					let us = tick * 20_000;
					let now = start + Duration::from_micros(us);
					loop {
						let arrival = packet * samples as u64 * 1_000_000_000_000
							/ (48_000 * (1_000_000 + ppm) as u64);
						let burst =
							if samples == 960 && ppm == 0 && packet < 3000 && packet % 10 == 3 {
								60_000
							} else {
								0
							};
						if arrival + burst > us {
							break;
						}

						audio.push(
							70,
							packet as u16,
							(packet * samples as u64) as u32,
							bytes[..length].to_vec(),
							&mut sync,
							now,
						);
						packet += 1;
					}
					peak_target = peak_target.max(sync.target(70));
					if let Some(frame) = audio.pop(&mut sync, now)
						&& tick >= 8950 && frame.iter().any(|value| value.abs() > 0.001)
					{
						audible += 1;
					}
				}
				assert_eq!(
					audible, 50,
					"duration={samples} samples, sender drift={ppm} ppm"
				);
				assert!(audio.sources[0].packets.len() <= PACKETS);
				if samples == 960 && ppm == 0 {
					assert!(peak_target > crate::presentation::INITIAL);
					assert!(sync.target(70) < peak_target);
				}
			}
		}
	}
}
