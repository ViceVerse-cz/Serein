//! Timestamp-preserving audio for a watched stream, separate from normal call mixing.
//! Bounded compressed reordering and one Opus packet of PCM per announced source.
use crate::{Frame, stream_sync::Sync, video_receive::MAX_SOURCES};
use opus2::{Channels, Decoder};
use std::time::Instant;

const PACKETS: usize = 8;
struct Packet {
	sequence: u16,
	timestamp: u32,
	opus: Box<[u8]>,
}
struct Source {
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
			if distance >= PACKETS as u16 {
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
			clock.deadline(self.user, self.ssrc, 48_000, timestamp, now);
		}
		self.packets.push(Packet {
			sequence,
			timestamp,
			opus: opus.into_boxed_slice(),
		});
	}
	fn pop(&mut self, clock: &mut Sync, now: Instant) -> Option<Frame> {
		let timestamp = self.cursor?;
		if now < clock.deadline(self.user, self.ssrc, 48_000, timestamp, now) {
			return None;
		}
		let mut frame = [0.0; 960];
		let mut filled = 0usize;
		let mut decoded = 0;
		while filled < frame.len() {
			let cursor = timestamp.wrapping_add(filled as u32);
			if self.offset == self.length {
				if decoded == 8 {
					break;
				} // At most eight 2.5 ms packets per output tick.
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
						.unwrap_or(0);
					self.offset = 0;
					if self.length == 0 {
						filled += count.min(frame.len() - filled);
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
						.unwrap_or(0);
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
				filled += (delta as usize).min(frame.len() - filled);
				continue;
			}
			self.offset += ((-i64::from(delta)) as usize).min(self.length - self.offset);
			let count = (self.length - self.offset).min(frame.len() - filled);
			frame[filled..filled + count]
				.copy_from_slice(&self.pcm[self.offset..self.offset + count]);
			self.offset += count;
			filled += count;
		}
		if filled == 0 && self.packets.is_empty() && self.offset == self.length {
			self.cursor = None;
			return None;
		}
		self.cursor = Some(timestamp.wrapping_add(960));
		Some(frame)
	}
}
#[derive(Default)]
pub(crate) struct Audio {
	sources: Vec<Source>,
}
impl Audio {
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
	pub fn pop(&mut self, clock: &mut Sync, now: Instant) -> Option<Frame> {
		let mut output = [0.0; 960];
		let mut heard = false;
		for source in &mut self.sources {
			if let Some(frame) = source.pop(clock, now) {
				heard = true;
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
		heard.then_some(output)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use opus2::{Application, Encoder};
	use std::time::Duration;
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
}
