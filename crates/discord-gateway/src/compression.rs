use crate::{Failure, Frame, MAX_GATEWAY_WIRE};
use flate2::{Decompress, FlushDecompress, Status};

const RETAINED_PENDING_BYTES: usize = 128 * 1024;

pub(crate) struct Decoder {
	inflater: Decompress,
	pending: Vec<u8>,
}

impl Default for Decoder {
	fn default() -> Self {
		Self {
			inflater: Decompress::new(true),
			pending: Vec::new(),
		}
	}
}
impl Decoder {
	pub fn frame(&mut self, frame: Frame) -> Result<Option<Frame>, Failure> {
		let Frame::Binary(bytes) = frame else {
			if matches!(frame, Frame::Text(_)) && !self.pending.is_empty() {
				return Err(Failure::Protocol);
			}
			return Ok(Some(frame));
		};
		if bytes.len() > MAX_GATEWAY_WIRE.saturating_sub(self.pending.len()) {
			return Err(Failure::CapacityAt(
				"Compressed Gateway payload exceeds 64 MiB; connection stopped",
			));
		}
		self.pending.extend_from_slice(&bytes);
		if !self.pending.ends_with(&[0, 0, 255, 255]) {
			return Ok(None);
		}
		let mut output = Vec::new();
		let mut consumed = 0;
		loop {
			let mut chunk = [0; 16 * 1024];
			let before_in = self.inflater.total_in();
			let before_out = self.inflater.total_out();
			let status = self
				.inflater
				.decompress(&self.pending[consumed..], &mut chunk, FlushDecompress::Sync)
				.map_err(|_| Failure::Protocol)?;
			let read = (self.inflater.total_in() - before_in) as usize;
			let written = (self.inflater.total_out() - before_out) as usize;
			if status == Status::StreamEnd {
				return Err(Failure::Protocol);
			}
			if written > MAX_GATEWAY_WIRE.saturating_sub(output.len()) {
				return Err(Failure::CapacityAt(
					"Decompressed Gateway payload exceeds 64 MiB; connection stopped",
				));
			}
			output.extend_from_slice(&chunk[..written]);
			consumed += read;
			if consumed == self.pending.len() && written < chunk.len() {
				break;
			}
			if read == 0 && written == 0 {
				return Err(Failure::Protocol);
			}
		}
		self.pending.clear();
		// Keep ordinary packet reuse without retaining a large READY allocation for
		// the connection's lifetime. The streaming inflater dictionary stays intact.
		if self.pending.capacity() > RETAINED_PENDING_BYTES {
			self.pending = Vec::new();
		}
		let text = String::from_utf8(output).map_err(|_| Failure::Protocol)?;
		Ok(Some(Frame::Text(text.into())))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Write;

	#[test]
	fn large_fragmented_payload_releases_capacity_but_keeps_stream_dictionary() {
		let mut seed = 1_u64;
		let large: String = (0..512 * 1024)
			.map(|_| {
				seed ^= seed << 13;
				seed ^= seed >> 7;
				seed ^= seed << 17;
				char::from(b' ' + (seed % 95) as u8)
			})
			.collect();
		let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
		let mut decoder = Decoder::default();
		let mut offset = 0;
		for text in [&large[..], &large[large.len() - 1024..]] {
			encoder.write_all(text.as_bytes()).unwrap();
			encoder.flush().unwrap();
			let bytes = &encoder.get_ref()[offset..];
			let split = bytes.len() - 2;
			assert!(
				decoder
					.frame(Frame::Binary(bytes[..split].to_vec().into()))
					.unwrap()
					.is_none()
			);
			if offset == 0 {
				assert!(decoder.pending.capacity() > RETAINED_PENDING_BYTES);
			}
			assert_eq!(
				decoder
					.frame(Frame::Binary(bytes[split..].to_vec().into()))
					.unwrap(),
				Some(Frame::Text(text.into()))
			);
			assert!(decoder.pending.is_empty());
			assert!(decoder.pending.capacity() <= RETAINED_PENDING_BYTES);
			offset = encoder.get_ref().len();
		}
		assert!(
			decoder.pending.capacity() > 0,
			"small packets still reuse capacity"
		);
	}

	#[test]
	fn split_payloads_share_the_dictionary_and_enforce_both_byte_limits() {
		let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
		let mut decoder = Decoder::default();
		let mut offset = 0;
		for text in [
			r#"{"op":10,"d":{"heartbeat_interval":1000}}"#,
			r#"{"op":11,"d":null}"#,
		] {
			encoder.write_all(text.as_bytes()).unwrap();
			encoder.flush().unwrap();
			let bytes = &encoder.get_ref()[offset..];
			let split = bytes.len() - 2;
			assert!(
				decoder
					.frame(Frame::Binary(bytes[..split].to_vec().into()))
					.unwrap()
					.is_none()
			);
			assert_eq!(
				decoder
					.frame(Frame::Binary(bytes[split..].to_vec().into()))
					.unwrap(),
				Some(Frame::Text(text.into()))
			);
			offset = encoder.get_ref().len();
		}
		assert_eq!(
			Decoder::default().frame(Frame::Binary(vec![0; MAX_GATEWAY_WIRE + 1].into())),
			Err(Failure::CapacityAt(
				"Compressed Gateway payload exceeds 64 MiB; connection stopped"
			))
		);
		// Give Sync enough output space to emit its marker in the same call.
		let mut bomb = flate2::Compress::new(flate2::Compression::fast(), true);
		let mut compressed = Vec::with_capacity(MAX_GATEWAY_WIRE + 1024);
		bomb.compress_vec(
			&vec![b'x'; MAX_GATEWAY_WIRE + 1],
			&mut compressed,
			flate2::FlushCompress::Sync,
		)
		.unwrap();
		assert_eq!(bomb.total_in(), (MAX_GATEWAY_WIRE + 1) as u64);
		assert!(compressed.ends_with(&[0, 0, 255, 255]));
		assert_eq!(
			Decoder::default().frame(Frame::Binary(compressed.into())),
			Err(Failure::CapacityAt(
				"Decompressed Gateway payload exceeds 64 MiB; connection stopped"
			))
		);
	}
}
