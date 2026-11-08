//! Outgoing camera RTP. Unofficial Discord video signaling; live interoperability is unverified.
//! Signaling reference: https://github.com/dank074/Discord-video-stream/blob/master/src/client/voice/BaseMediaConnection.ts
//! H264 packetization follows RFC 6184 section 5 (single NAL and FU-A).
use crate::crypto::Encryption;
use serde_json::{Value, json};
use std::collections::VecDeque;

pub const MAX_FRAME_BYTES: usize = 128 * 1024;
const PAYLOAD: usize = 1100;

pub struct Frame {
	pub generation: u64,
	pub timestamp: u32,
	pub data: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Sender {
	ssrc: u32,
	rtx: u32,
	sequence: u16,
	pub negotiated: bool,
	pub generation: u64,
	pub announced: bool,
	packets: VecDeque<Vec<u8>>,
}
impl Sender {
	pub fn configure(&mut self, data: &Value, audio: u32) {
		// Request one stream and accept only that exact assignment, never guessed SSRCs.
		let Some(stream) = data["streams"]
			.as_array()
			.filter(|s| s.len() <= 4)
			.and_then(|s| s.iter().find(|s| s["type"] == "video" && s["rid"] == "100"))
		else {
			return;
		};
		let ssrc = stream["ssrc"]
			.as_u64()
			.and_then(|s| u32::try_from(s).ok())
			.unwrap_or(0);
		let rtx = stream["rtx_ssrc"]
			.as_u64()
			.and_then(|s| u32::try_from(s).ok())
			.unwrap_or(0);
		if ssrc != 0 && rtx != 0 && ssrc != audio && rtx != audio && ssrc != rtx {
			self.ssrc = ssrc;
			self.rtx = rtx;
		}
	}
	pub fn available(&self) -> bool {
		self.negotiated && self.ssrc != 0
	}
	pub fn announcement(&self, audio: u32, enabled: bool) -> Value {
		json!({"op":12,"d":{"audio_ssrc":audio,"video_ssrc":if enabled {self.ssrc} else {0},"rtx_ssrc":if enabled {self.rtx} else {0},"streams":if enabled {vec![json!({"type":"video","rid":"100","ssrc":self.ssrc,"rtx_ssrc":self.rtx,"active":true,"quality":100,"max_bitrate":600_000,"max_framerate":crate::camera::FPS,"max_resolution":{"type":"fixed","width":640,"height":480}})]}else{vec![]}}})
	}
	pub fn clear(&mut self) {
		self.packets.clear();
	}
	pub fn is_empty(&self) -> bool {
		self.packets.is_empty()
	}
	pub fn next(&mut self) -> Option<Vec<u8>> {
		self.packets.pop_front()
	}
	pub fn packetize(
		&mut self,
		frame: &[u8],
		timestamp: u32,
		encryption: &mut Encryption,
	) -> Result<(), &'static str> {
		if frame.len() > MAX_FRAME_BYTES + 1024 || !self.packets.is_empty() {
			return Err("Camera frame exceeds the media budget");
		}
		let nals = nal_units(frame)?;
		for (index, nal) in nals.iter().enumerate() {
			let last_nal = index + 1 == nals.len();
			if nal.len() <= PAYLOAD {
				self.push(nal, timestamp, last_nal, encryption)?;
			} else {
				let chunks = nal[1..].chunks(PAYLOAD - 2);
				let count = chunks.len();
				for (i, chunk) in chunks.enumerate() {
					let mut payload = Vec::with_capacity(chunk.len() + 2);
					payload.push((nal[0] & 0xe0) | 28);
					payload.push(
						(nal[0] & 0x1f)
							| if i == 0 { 0x80 } else { 0 }
							| if i + 1 == count { 0x40 } else { 0 },
					);
					payload.extend_from_slice(chunk);
					self.push(&payload, timestamp, last_nal && i + 1 == count, encryption)?;
				}
			}
		}
		Ok(())
	}
	fn push(
		&mut self,
		data: &[u8],
		timestamp: u32,
		marker: bool,
		encryption: &mut Encryption,
	) -> Result<(), &'static str> {
		if self.packets.len() >= 256 {
			return Err("Camera packet queue exceeds its budget");
		}
		let mut header = [0; 12];
		header[0] = 0x80;
		header[1] = 101 | if marker { 0x80 } else { 0 };
		header[2..4].copy_from_slice(&self.sequence.to_be_bytes());
		header[4..8].copy_from_slice(&timestamp.to_be_bytes());
		header[8..12].copy_from_slice(&self.ssrc.to_be_bytes());
		self.sequence = self.sequence.wrapping_add(1);
		self.packets.push_back(encryption.seal(&header, data)?);
		Ok(())
	}
}

fn nal_units(frame: &[u8]) -> Result<Vec<&[u8]>, &'static str> {
	let mut starts = Vec::new();
	let mut i = 0;
	while i + 3 <= frame.len() {
		let size = if frame[i..].starts_with(&[0, 0, 0, 1]) {
			4
		} else if frame[i..].starts_with(&[0, 0, 1]) {
			3
		} else {
			i += 1;
			continue;
		};
		if starts.len() >= 64 {
			return Err("Camera frame contains too many NAL units");
		}
		starts.push((i, i + size));
		i += size;
	}
	if starts.first().is_none_or(|s| s.0 != 0) {
		return Err("Invalid Annex B camera frame");
	}
	starts
		.iter()
		.enumerate()
		.map(|(index, &(_, start))| {
			let end = starts.get(index + 1).map_or(frame.len(), |s| s.0);
			let nal = &frame[start..end];
			if nal.is_empty() || !(1..=23).contains(&(nal[0] & 0x1f)) {
				Err("Invalid H264 camera NAL unit")
			} else {
				Ok(nal)
			}
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn camera_packetization_is_bounded_and_marks_only_the_last_fragment() {
		let mut sender = Sender::default();
		assert_eq!(
			sender.announcement(1, true)["d"]["streams"][0]["max_framerate"].as_u64(),
			Some(u64::from(crate::camera::FPS))
		);
		let mut crypto = Encryption::new(&[7; 32]);
		let mut frame = vec![0, 0, 0, 1, 0x65];
		frame.extend(vec![9; 2400]);
		sender.packetize(&frame, 6000, &mut crypto).unwrap();
		assert_eq!(sender.packets.len(), 3);
		for (i, packet) in sender.packets.iter().enumerate() {
			assert!(packet.len() <= PAYLOAD + 32);
			assert_eq!(packet[1] & 0x80 != 0, i == 2);
			assert_eq!(&packet[4..8], &6000u32.to_be_bytes());
		}
		sender.clear();
		assert!(
			sender
				.packetize(&vec![0; MAX_FRAME_BYTES + 1025], 0, &mut crypto)
				.is_err()
		);
		assert!(nal_units(&[0, 0, 1]).is_err());
	}
}
