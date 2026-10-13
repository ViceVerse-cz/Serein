//! Demand-driven, bounded attachment reads; demuxers never see unbounded metadata.
use super::{Gate, INVALID, NO_SEEK, Request};
use std::{
	io::{self, Cursor, Read, Seek, SeekFrom},
	sync::{Arc, atomic::Ordering},
	time::Duration,
};
use symphonia::core::io::MediaSource;
use tokio::{runtime::Handle, sync::Notify};

const CHUNK: usize = 16 * 1024;
const RANGE_UNSUPPORTED: &str =
	"Audio server does not support buffering; download to play externally";

/// Open a cancellable range reader and bound metadata before handing the stream to a demuxer.
pub(super) fn source(
	request: &Request,
	gate: Arc<Gate>,
	wake: Arc<Notify>,
	runtime: Handle,
) -> Result<Box<dyn MediaSource>, &'static str> {
	let raw = if let Some(url) = &request.url {
		if request.expected == 0 {
			return Err(INVALID);
		}
		Raw {
			input: Input::Http {
				client: reqwest::Client::builder()
					.no_proxy()
					.redirect(reqwest::redirect::Policy::none())
					.timeout(Duration::from_secs(15))
					.build()
					.map_err(|_| "Audio download unavailable")?,
				url: url.clone(),
				gate,
				wake,
				runtime,
				generation: request.generation,
				cache: Vec::new(),
				cache_start: 0,
			},
			position: 0,
			len: request.expected,
			#[cfg(debug_assertions)]
			bytes_read: 0,
		}
	} else {
		#[cfg(not(feature = "demo"))]
		return Err(INVALID);
		#[cfg(feature = "demo")]
		Raw::memory(if request.voice_message {
			include_bytes!("../../tests/fixtures/voice-message.ogg").to_vec()
		} else {
			super::demo_wav()
		})
	};
	Sanitized::new(raw)
		.map(|source| Box::new(source) as Box<dyn MediaSource>)
		.map_err(|error| {
			if error.kind() == io::ErrorKind::Unsupported {
				RANGE_UNSUPPORTED
			} else if error.kind() == io::ErrorKind::ConnectionAborted {
				"Cancelled"
			} else {
				INVALID
			}
		})
}

enum Input {
	#[cfg(feature = "demo")]
	Memory(Vec<u8>),
	Http {
		client: reqwest::Client,
		url: url::Url,
		gate: Arc<Gate>,
		wake: Arc<Notify>,
		runtime: Handle,
		generation: u64,
		cache: Vec<u8>,
		cache_start: usize,
	},
}
struct Raw {
	input: Input,
	position: usize,
	len: usize,
	#[cfg(debug_assertions)]
	bytes_read: usize,
}
fn invalid() -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, INVALID)
}
impl Raw {
	#[cfg(feature = "demo")]
	fn memory(bytes: Vec<u8>) -> Self {
		Self {
			len: bytes.len(),
			input: Input::Memory(bytes),
			position: 0,
			#[cfg(debug_assertions)]
			bytes_read: 0,
		}
	}
	fn jump(&mut self, position: usize) -> io::Result<()> {
		if position > self.len {
			return Err(invalid());
		}
		self.position = position;
		Ok(())
	}
}
impl Read for Raw {
	fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
		let mut count = output.len().min(CHUNK).min(self.len - self.position);
		if count == 0 {
			return Ok(0);
		}
		match &mut self.input {
			#[cfg(feature = "demo")]
			Input::Memory(bytes) => {
				output[..count].copy_from_slice(&bytes[self.position..self.position + count])
			}
			Input::Http {
				client,
				url,
				gate,
				wake,
				runtime,
				generation,
				cache,
				cache_start,
			} => {
				if !gate.current(*generation) || gate.seek_millis.load(Ordering::Acquire) != NO_SEEK
				{
					return Err(io::Error::new(
						io::ErrorKind::ConnectionAborted,
						"Cancelled",
					));
				}
				if self.position < *cache_start || self.position >= *cache_start + cache.len() {
					let start = self.position / CHUNK * CHUNK;
					let count = CHUNK.min(self.len - start);
					let end = start + count - 1;
					let total = self.len;
					let bytes = runtime.block_on(async {
						let cancelled = async {
							loop {
								if !gate.current(*generation)
									|| gate.seek_millis.load(Ordering::Acquire) != NO_SEEK
								{
									break;
								}
								tokio::time::sleep(Duration::from_millis(20)).await;
							}
						};
						let transfer = async {
							while gate.paused.load(Ordering::Acquire) {
								wake.notified().await;
							}
							let mut response = client
								.get(url.clone())
								.header(reqwest::header::ACCEPT_ENCODING, "identity")
								.header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
								.send()
								.await
								.map_err(|_| invalid())?;
							if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
								let range = format!("bytes {start}-{end}/{total}");
								if response
									.headers()
									.get(reqwest::header::CONTENT_RANGE)
									.and_then(|v| v.to_str().ok())
									!= Some(range.as_str())
								{
									return Err(invalid());
								}
							} else if !(response.status() == reqwest::StatusCode::OK
								&& start == 0 && count == total)
							{
								return Err(io::Error::new(
									io::ErrorKind::Unsupported,
									RANGE_UNSUPPORTED,
								));
							}
							if response
								.headers()
								.get(reqwest::header::CONTENT_ENCODING)
								.is_some_and(|v| v != "identity")
								|| response
									.content_length()
									.is_some_and(|size| size != count as u64)
							{
								return Err(invalid());
							}
							let mut bytes = Vec::with_capacity(count);
							while let Some(chunk) = response.chunk().await.map_err(|_| invalid())? {
								if chunk.len() > count - bytes.len() {
									return Err(invalid());
								}
								bytes.extend_from_slice(&chunk);
							}
							if bytes.len() != count {
								return Err(invalid());
							}
							Ok(bytes)
						};
						tokio::select! { biased;
							// read_exact retries Interrupted, so cancellation must be terminal.
							_ = cancelled => Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Cancelled")),
							result = transfer => result,
						}
					})?;
					*cache = bytes;
					*cache_start = start;
				}
				let offset = self.position - *cache_start;
				count = count.min(cache.len() - offset);
				output[..count].copy_from_slice(&cache[offset..offset + count]);
			}
		}
		self.position += count;
		#[cfg(debug_assertions)]
		{
			self.bytes_read += count;
		}
		Ok(count)
	}
}

struct Sanitized {
	raw: Raw,
	prefix: Cursor<Vec<u8>>,
	end: usize,
	ogg: Option<Ogg>,
}
impl Sanitized {
	/// Validate format headers and expose only bounded metadata and forward-only audio payloads.
	fn new(mut raw: Raw) -> io::Result<Self> {
		let mut header = [0; 12];
		raw.read_exact(&mut header)?;
		raw.jump(0)?;
		let mut result = Self {
			end: raw.len,
			raw,
			prefix: Cursor::new(Vec::new()),
			ogg: None,
		};
		if header.starts_with(b"OggS") {
			let mut ogg = Ogg::default();
			let mut prefix = Vec::new();
			while ogg.packets < ogg.headers {
				let page = ogg.page(&mut result.raw)?;
				if prefix.len() + page.len() > 256 * 1024 {
					return Err(invalid());
				}
				prefix.extend_from_slice(&page);
			}
			result.prefix = Cursor::new(prefix);
			result.ogg = Some(ogg);
		} else if header.starts_with(b"RIFF") {
			if &header[8..12] != b"WAVE" {
				return Err(invalid());
			}
			let end = (u32::from_le_bytes(header[4..8].try_into().map_err(|_| invalid())?)
				as usize)
				.checked_add(8)
				.ok_or_else(invalid)?;
			if end > result.raw.len {
				return Err(invalid());
			}
			let mut position = 12;
			let mut format = None;
			let mut data = None;
			for _ in 0..1024 {
				if position == end {
					break;
				}
				if end.saturating_sub(position) < 8 {
					return Err(invalid());
				}
				result.raw.jump(position)?;
				let mut chunk = [0; 8];
				result.raw.read_exact(&mut chunk)?;
				let len =
					u32::from_le_bytes(chunk[4..8].try_into().map_err(|_| invalid())?) as usize;
				let next = position
					.checked_add(8)
					.and_then(|next| next.checked_add(len))
					.and_then(|next| next.checked_add(len % 2))
					.filter(|next| *next <= end)
					.ok_or_else(invalid)?;
				if &chunk[..4] == b"fmt " {
					if format.is_some() || data.is_some() || !(16..=40).contains(&len) {
						return Err(invalid());
					}
					let mut bytes = chunk.to_vec();
					bytes.resize(8 + len + len % 2, 0);
					result.raw.read_exact(&mut bytes[8..])?;
					format = Some(bytes);
				} else if &chunk[..4] == b"data" {
					if format.is_none() || data.is_some() || len == 0 {
						return Err(invalid());
					}
					data = Some((position + 8, len));
				}
				position = next;
			}
			if position != end {
				return Err(invalid());
			}
			let (start, len) = data.ok_or_else(invalid)?;
			let mut prefix = b"RIFF\0\0\0\0WAVE".to_vec();
			prefix.extend_from_slice(&format.ok_or_else(invalid)?);
			// Reuse the existing PCM fmt validator with a tiny synthetic data chunk.
			prefix.extend_from_slice(b"data\x02\0\0\0\0\0");
			let size = (prefix.len() - 8) as u32;
			prefix[4..8].copy_from_slice(&size.to_le_bytes());
			super::prepare_media(&mut prefix).map_err(|_| invalid())?;
			prefix.truncate(prefix.len() - 2);
			let offset = prefix.len() - 4;
			prefix[offset..].copy_from_slice(&(len as u32).to_le_bytes());
			let size = (prefix.len() + len + len % 2 - 8) as u32;
			prefix[4..8].copy_from_slice(&size.to_le_bytes());
			result.prefix = Cursor::new(prefix);
			result.raw.jump(start)?;
			result.end = start + len + len % 2;
		} else {
			let mut offset = 0;
			for _ in 0..16 {
				result.raw.jump(offset)?;
				let mut tag = [0; 10];
				result.raw.read_exact(&mut tag)?;
				if &tag[..3] != b"ID3" {
					break;
				}
				if tag[6..].iter().any(|byte| byte & 0x80 != 0) {
					return Err(invalid());
				}
				let len = tag[6..]
					.iter()
					.fold(0usize, |len, byte| (len << 7) | usize::from(*byte));
				let footer = usize::from(tag[3] == 4 && tag[5] & 0x10 != 0) * 10;
				offset = offset
					.checked_add(10 + len + footer)
					.filter(|offset| *offset < result.end)
					.ok_or_else(invalid)?;
			}
			result.raw.jump(offset)?;
			let mut sync = [0; 2];
			result.raw.read_exact(&mut sync)?;
			if sync[0] != 0xff || sync[1] & 0xe0 != 0xe0 {
				return Err(invalid());
			}
			result.raw.jump(offset)?;
		}
		Ok(result)
	}
}
impl Read for Sanitized {
	fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
		if output.is_empty() {
			return Ok(0);
		}
		let count = self.prefix.read(output)?;
		if count != 0 {
			return Ok(count);
		}
		if let Some(ogg) = &mut self.ogg {
			if self.raw.position == self.end {
				if !ogg.ended || ogg.packet_len != 0 || ogg.packets <= ogg.headers {
					return Err(invalid());
				}
				return Ok(0);
			}
			self.prefix = Cursor::new(ogg.page(&mut self.raw)?);
			self.prefix.read(output)
		} else {
			let count = output.len().min(self.end - self.raw.position);
			self.raw.read(&mut output[..count])
		}
	}
}
impl Seek for Sanitized {
	fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
		Err(io::Error::new(
			io::ErrorKind::Unsupported,
			"Forward-only audio",
		))
	}
}
impl MediaSource for Sanitized {
	fn is_seekable(&self) -> bool {
		false
	}
	fn byte_len(&self) -> Option<u64> {
		None
	}
}

struct Ogg {
	serial: Option<u32>,
	packet: Vec<u8>,
	packet_len: usize,
	packets: usize,
	headers: usize,
	ended: bool,
}
impl Default for Ogg {
	fn default() -> Self {
		Self {
			serial: None,
			packet: Vec::new(),
			packet_len: 0,
			packets: 0,
			headers: 3,
			ended: false,
		}
	}
}
impl Ogg {
	fn page(&mut self, raw: &mut Raw) -> io::Result<Vec<u8>> {
		let mut page = vec![0; 27];
		raw.read_exact(&mut page)?;
		if &page[..4] != b"OggS" || page[4] != 0 || page[5] & !7 != 0 || self.ended {
			return Err(invalid());
		}
		let serial = u32::from_le_bytes(page[14..18].try_into().map_err(|_| invalid())?);
		if self.serial.is_some_and(|previous| previous != serial)
			|| (page[5] & 2 != 0) != self.serial.is_none()
			|| (page[5] & 1 != 0) != (self.packet_len > 0)
		{
			return Err(invalid());
		}
		self.serial = Some(serial);
		self.ended = page[5] & 4 != 0;
		let segments = usize::from(page[26]);
		page.resize(27 + segments, 0);
		raw.read_exact(&mut page[27..])?;
		let body = page[27..]
			.iter()
			.map(|&lace| usize::from(lace))
			.sum::<usize>();
		page.resize(27 + segments + body, 0);
		raw.read_exact(&mut page[27 + segments..])?;
		let mut offset = 27 + segments;
		for index in 0..segments {
			let len = usize::from(page[27 + index]);
			self.packet_len += len;
			if self.packet_len
				> if self.packets < self.headers {
					64 * 1024
				} else {
					1024 * 1024
				} {
				return Err(invalid());
			}
			if self.packets < self.headers {
				self.packet.extend_from_slice(&page[offset..offset + len]);
			}
			offset += len;
			if len < 255 {
				if self.packets == 0 {
					self.headers = if self.packet.starts_with(b"OpusHead") {
						2
					} else if self.packet.starts_with(b"\x01vorbis") {
						3
					} else {
						return Err(invalid());
					};
				} else if self.packets == 1 {
					let comments = self
						.packet
						.strip_prefix(b"OpusTags")
						.or_else(|| self.packet.strip_prefix(b"\x03vorbis"))
						.ok_or_else(invalid)?;
					let number = |offset: usize| -> io::Result<usize> {
						Ok(u32::from_le_bytes(
							comments
								.get(offset..offset + 4)
								.ok_or_else(invalid)?
								.try_into()
								.map_err(|_| invalid())?,
						) as usize)
					};
					let vendor = number(0)?;
					if vendor > comments.len().saturating_sub(4) {
						return Err(invalid());
					}
					let count = number(4 + vendor)?;
					if count > 128 {
						return Err(invalid());
					}
					let mut cursor = 8 + vendor;
					for _ in 0..count {
						cursor = cursor
							.checked_add(4 + number(cursor)?)
							.filter(|end| *end <= comments.len())
							.ok_or_else(invalid)?;
					}
				}
				self.packets += 1;
				if self.packets > 100_000 {
					return Err(invalid());
				}
				self.packet.clear();
				self.packet_len = 0;
			}
		}
		if self.ended
			&& (self.packet_len != 0 || raw.position != raw.len || self.packets <= self.headers)
		{
			return Err(invalid());
		}
		Ok(page)
	}
}

#[cfg(all(debug_assertions, feature = "demo"))]
/// Verify bounded reads, malformed metadata rejection and complete oversized WAV decoding offline.
pub(super) fn debug_check() {
	use std::{io::Write, sync::atomic::AtomicUsize};
	let bytes = super::demo_wav();
	let mut source = Sanitized::new(Raw::memory(bytes.clone())).expect("synthetic WAV header");
	assert!(
		source.raw.bytes_read < 128,
		"startup must not read audio payload"
	);
	assert!(!source.is_seekable());
	let mut first = [0; 512];
	source.read_exact(&mut first).unwrap();
	assert!(source.raw.bytes_read < 1024, "reads follow demand");
	let mut streamed = first.to_vec();
	source.read_to_end(&mut streamed).unwrap();
	assert_eq!(streamed, bytes);
	let mut invalid_wav = bytes;
	invalid_wav[22..24].copy_from_slice(&u16::MAX.to_le_bytes());
	assert!(Sanitized::new(Raw::memory(invalid_wav)).is_err());
	let ogg = include_bytes!("../../tests/fixtures/voice-message.ogg");
	let mut source = Sanitized::new(Raw::memory(ogg.to_vec())).expect("synthetic Opus headers");
	assert!(
		source.raw.position < ogg.len(),
		"Ogg startup must stop after headers"
	);
	let mut streamed = Vec::new();
	source.read_to_end(&mut streamed).unwrap();
	assert_eq!(streamed, ogg);
	let mut invalid_ogg = ogg.to_vec();
	let tags = ogg
		.windows(8)
		.position(|window| window == b"OpusTags")
		.unwrap();
	let vendor = u32::from_le_bytes(ogg[tags + 8..tags + 12].try_into().unwrap()) as usize;
	invalid_ogg[tags + 12 + vendor..tags + 16 + vendor].copy_from_slice(&129u32.to_le_bytes());
	assert!(Sanitized::new(Raw::memory(invalid_ogg)).is_err());
	assert!(Sanitized::new(Raw::memory(b"ID3\x04\0\0\x7f\x7f\x7f\x7f\0\0".to_vec())).is_err());
	let mut tagged = b"ID3\x04\0\0\0\0\x20\0".to_vec();
	tagged.resize(10 + 4096, 0);
	tagged.extend_from_slice(&[0xff, 0xfb, 0, 0, 0, 0, 0, 0, 0, 0]);
	let source = Sanitized::new(Raw::memory(tagged)).expect("skip ID3 payload");
	assert_eq!(source.raw.position, 4106);
	assert!(source.raw.bytes_read < 64, "ID3 payload is never fetched");

	// The same source against a synthetic local server: tiny demuxer reads share one range.
	// A valid WAV above the old file-size ceiling still uses tiny bounded ranges.
	let mut fixture = super::demo_wav();
	fixture.resize(24 * 1024 * 1024, 0);
	let riff_size = (fixture.len() - 8) as u32;
	let data_size = (fixture.len() - 44) as u32;
	fixture[4..8].copy_from_slice(&riff_size.to_le_bytes());
	fixture[40..44].copy_from_slice(&data_size.to_le_bytes());
	let expected = fixture.len();
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let url = url::Url::parse(&format!(
		"http://{}/synthetic.wav",
		listener.local_addr().unwrap()
	))
	.unwrap();
	let requests = Arc::new(AtomicUsize::new(0));
	let observed = requests.clone();
	let server = std::thread::spawn(move || {
		let mut probes = 0;
		for (index, start) in [0, CHUNK, 0]
			.into_iter()
			.chain((0..expected).step_by(CHUNK))
			.enumerate()
		{
			let (mut socket, header) = loop {
				let (mut socket, _) = listener.accept().unwrap();
				socket
					.set_read_timeout(Some(Duration::from_secs(2)))
					.unwrap();
				let mut header = Vec::new();
				while !header.ends_with(b"\r\n\r\n") {
					assert!(header.len() < 8192);
					let mut byte = [0];
					socket.read_exact(&mut byte).unwrap();
					header.push(byte[0]);
				}
				let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
				// Some development hosts probe listening ports; these are not range reads.
				if header.starts_with("head / http/1.1\r\n") {
					probes += 1;
					assert!(probes <= 8, "too many unrelated local port probes");
					let _ =
						socket.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
					continue;
				}
				break (socket, header);
			};
			assert!(
				header.contains(&format!("range: bytes={start}-{}\r\n", start + CHUNK - 1)),
				"unexpected synthetic range request #{index}"
			);
			assert!(!header.contains("authorization:"));
			observed.fetch_add(1, Ordering::Release);
			let total = expected + usize::from(index == 2);
			write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{total}\r\nContent-Length: {CHUNK}\r\nConnection: close\r\n\r\n", start + CHUNK - 1).unwrap();
			// The third malformed header is rejected before its body is consumed.
			let result = socket.write_all(&fixture[start..start + CHUNK]);
			if index != 2 {
				result.unwrap();
			}
		}
	});
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.unwrap();
	let gate = Arc::new(Gate::default());
	let wake = Arc::new(Notify::new());
	gate.paused.store(true, Ordering::Release);
	let resume_gate = gate.clone();
	let resume_wake = wake.clone();
	let resume = std::thread::spawn(move || {
		std::thread::sleep(Duration::from_millis(40));
		resume_gate.paused.store(false, Ordering::Release);
		resume_wake.notify_one();
	});
	let request = Request {
		output: None,
		duration: Duration::ZERO,
		generation: 0,
		url: Some(url),
		expected,
		voice_message: false,
	};
	let mut remote = self::source(
		&request,
		gate.clone(),
		wake.clone(),
		runtime.handle().clone(),
	)
	.unwrap();
	resume.join().unwrap();
	assert_eq!(requests.load(Ordering::Acquire), 1);
	remote.read_exact(&mut [0; 64]).unwrap();
	assert_eq!(requests.load(Ordering::Acquire), 1);
	remote.read_exact(&mut [0; CHUNK]).unwrap();
	assert_eq!(requests.load(Ordering::Acquire), 2);
	assert!(
		self::source(
			&request,
			gate.clone(),
			wake.clone(),
			runtime.handle().clone()
		)
		.is_err()
	);
	let remote = self::source(&request, gate, wake, runtime.handle().clone()).unwrap();
	let mut samples = 0;
	super::decode_stream(remote, &|| true, &mut |packet, channels, rate, _| {
		assert_eq!((channels, rate), (1, 24000));
		samples += packet.len();
		Ok(())
	})
	.unwrap();
	assert_eq!(
		samples,
		(expected - 44) / 2,
		"decode the entire WAV payload"
	);
	assert_eq!(
		requests.load(Ordering::Acquire),
		3 + expected.div_ceil(CHUNK)
	);
	server.join().unwrap();
}
