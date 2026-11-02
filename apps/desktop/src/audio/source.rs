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
/// Sequential range reads double from `CHUNK` up to this, so startup stays one small request
/// while steady playback needs few round trips. At most two blocks are held at once.
const MAX_BLOCK: usize = 256 * 1024;
/// Downloaded bytes kept per clip so seeking, which replays decoding from the start, does not
/// download again. Ranges are kept from the start of the file until the budget is spent.
const MAX_DOWNLOADS: usize = 16 * 1024 * 1024;

/// Ranges already downloaded for one clip generation; replaced when the generation changes.
#[derive(Default)]
pub(super) struct Downloads {
	generation: u64,
	ranges: Vec<(usize, Arc<[u8]>)>,
	bytes: usize,
}
impl Downloads {
	fn get(&self, generation: u64, position: usize) -> Option<(usize, Arc<[u8]>)> {
		(self.generation == generation)
			.then(|| {
				self.ranges
					.iter()
					.find(|(start, bytes)| (*start..start + bytes.len()).contains(&position))
					.cloned()
			})
			.flatten()
	}
	fn put(&mut self, generation: u64, start: usize, bytes: &Arc<[u8]>) {
		if self.generation != generation {
			*self = Self {
				generation,
				..Self::default()
			};
		}
		if self.bytes + bytes.len() > MAX_DOWNLOADS
			|| self.ranges.iter().any(|(existing, _)| *existing == start)
		{
			return;
		}
		self.bytes += bytes.len();
		self.ranges.push((start, bytes.clone()));
	}
}
/// Top-level MP4 boxes, nested boxes and the movie index each have a fixed ceiling.
const MAX_MP4_BOXES: usize = 256;
const MAX_MP4_CHILDREN: usize = 4096;
const MAX_MOOV: usize = 8 * 1024 * 1024;
const MAX_FLAC_BLOCKS: usize = 128;
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
				cache: Arc::from([]),
				cache_start: 0,
				block: CHUNK,
				ahead: None,
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
	#[cfg(any(test, feature = "demo"))]
	Memory(Vec<u8>),
	Http {
		client: reqwest::Client,
		url: url::Url,
		gate: Arc<Gate>,
		wake: Arc<Notify>,
		runtime: Handle,
		generation: u64,
		cache: Arc<[u8]>,
		cache_start: usize,
		/// Size of the next sequential range request.
		block: usize,
		/// The following block, fetched in the background once reading passes half the cache.
		ahead: Option<(usize, tokio::task::JoinHandle<io::Result<Vec<u8>>>)>,
	},
}
impl Drop for Raw {
	fn drop(&mut self) {
		if let Input::Http {
			ahead: Some((_, task)),
			..
		} = &self.input
		{
			task.abort();
		}
	}
}

/// One validated range request, cancelled when playback moves to another clip or seeks.
#[allow(clippy::too_many_arguments)] // The captured fields of one HTTP input.
async fn fetch(
	client: reqwest::Client,
	url: url::Url,
	gate: Arc<Gate>,
	wake: Arc<Notify>,
	generation: u64,
	start: usize,
	count: usize,
	total: usize,
) -> io::Result<Vec<u8>> {
	let end = start + count - 1;
	let cancelled = async {
		loop {
			if !gate.current(generation) || gate.seek_millis.load(Ordering::Acquire) != NO_SEEK {
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
			.get(url)
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
		} else if !(response.status() == reqwest::StatusCode::OK && start == 0 && count == total) {
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
	#[cfg(any(test, feature = "demo"))]
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
			#[cfg(any(test, feature = "demo"))]
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
				block,
				ahead,
			} => {
				if !gate.current(*generation) || gate.seek_millis.load(Ordering::Acquire) != NO_SEEK
				{
					return Err(io::Error::new(
						io::ErrorKind::ConnectionAborted,
						"Cancelled",
					));
				}
				let position = self.position;
				let total = self.len;
				let task = |start: usize, count: usize| {
					fetch(
						client.clone(),
						url.clone(),
						gate.clone(),
						wake.clone(),
						*generation,
						start,
						count,
						total,
					)
				};
				if position < *cache_start || position >= *cache_start + cache.len() {
					let sequential = !cache.is_empty() && position == *cache_start + cache.len();
					*block = if sequential {
						(*block * 2).min(MAX_BLOCK)
					} else {
						CHUNK
					};
					// Random access (probing, the MP4 index) keeps the original aligned blocks.
					let start = if sequential {
						position
					} else {
						position / CHUNK * CHUNK
					};
					let downloaded = gate
						.downloads
						.lock()
						.ok()
						.and_then(|downloads| downloads.get(*generation, position));
					let (start, bytes) = if let Some(range) = downloaded {
						range
					} else {
						let bytes: Arc<[u8]> = match ahead.take() {
							Some((ahead_start, handle)) if ahead_start == start => runtime
								.block_on(handle)
								.unwrap_or_else(|_| Err(invalid()))?,
							other => {
								if let Some((_, handle)) = other {
									handle.abort();
								}
								runtime.block_on(task(start, (*block).min(total - start)))?
							}
						}
						.into();
						if let Ok(mut downloads) = gate.downloads.lock() {
							downloads.put(*generation, start, &bytes);
						}
						(start, bytes)
					};
					*cache = bytes;
					*cache_start = start;
				}
				let offset = position - *cache_start;
				count = count.min(cache.len() - offset);
				output[..count].copy_from_slice(&cache[offset..offset + count]);
				// Keep the network one block ahead of steady reads; probes near a block start
				// never trigger it.
				let next = *cache_start + cache.len();
				let downloaded = || {
					gate.downloads
						.lock()
						.is_ok_and(|downloads| downloads.get(*generation, next).is_some())
				};
				if ahead.is_none()
					&& next < total && offset + count >= cache.len() / 2
					&& !downloaded()
				{
					let count = (*block * 2).min(MAX_BLOCK).min(total - next);
					*ahead = Some((next, runtime.spawn(task(next, count))));
				}
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
	/// MP4 replacement byte ranges over the original layout. MP4 is the one seekable format:
	/// its index can follow the audio, and sample offsets are absolute, so boxes are renamed in
	/// place rather than removed.
	mp4: Option<Vec<(usize, Vec<u8>)>>,
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
			mp4: None,
		};
		if &header[4..8] == b"ftyp" {
			result.mp4 = Some(mp4_overlays(&mut result.raw, result.end)?);
			result.raw.jump(0)?;
		} else if header.starts_with(b"fLaC") {
			let (prefix, frames) = flac_prefix(&mut result.raw, result.end)?;
			result.prefix = Cursor::new(prefix);
			result.raw.jump(frames)?;
		} else if header.starts_with(b"OggS") {
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
		if let Some(overlays) = &self.mp4 {
			let position = self.raw.position;
			if position >= self.end {
				return Ok(0);
			}
			if let Some((start, bytes)) = overlays
				.iter()
				.find(|(start, bytes)| (*start..start + bytes.len()).contains(&position))
			{
				let from = position - start;
				let count = output.len().min(bytes.len() - from);
				output[..count].copy_from_slice(&bytes[from..from + count]);
				self.raw.jump(position + count)?;
				return Ok(count);
			}
			let next = overlays
				.iter()
				.map(|(start, _)| *start)
				.filter(|start| *start > position)
				.min()
				.unwrap_or(self.end)
				.min(self.end);
			let count = output.len().min(next - position);
			return self.raw.read(&mut output[..count]);
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
	fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
		if self.mp4.is_none() {
			return Err(io::Error::new(
				io::ErrorKind::Unsupported,
				"Forward-only audio",
			));
		}
		let position = match target {
			SeekFrom::Start(offset) => i128::from(offset),
			SeekFrom::Current(delta) => self.raw.position as i128 + i128::from(delta),
			SeekFrom::End(delta) => self.end as i128 + i128::from(delta),
		};
		let position = usize::try_from(position)
			.ok()
			.filter(|position| *position <= self.end)
			.ok_or_else(invalid)?;
		self.raw.jump(position)?;
		Ok(position as u64)
	}
}
impl MediaSource for Sanitized {
	fn is_seekable(&self) -> bool {
		self.mp4.is_some()
	}
	fn byte_len(&self) -> Option<u64> {
		self.mp4.as_ref().map(|_| self.end as u64)
	}
}

/// Box size and header length at `at`; size 0 extends to `end`, size 1 is a 64-bit size.
fn mp4_box(header: &[u8], at: usize, end: usize) -> io::Result<(usize, usize)> {
	let size = u32::from_be_bytes(header[..4].try_into().map_err(|_| invalid())?);
	let (header_len, size) = match size {
		0 => (8, end - at),
		1 => {
			let large = header.get(8..16).ok_or_else(invalid)?;
			(
				16,
				usize::try_from(u64::from_be_bytes(large.try_into().map_err(|_| invalid())?))
					.map_err(|_| invalid())?,
			)
		}
		size => (8, size as usize),
	};
	if size < header_len || at.checked_add(size).is_none_or(|next| next > end) {
		return Err(invalid());
	}
	Ok((header_len, size))
}

/// Validate the top-level MP4 layout and return the in-place replacements: the movie index
/// with metadata boxes renamed `free`, and `free` headers over unknown top-level boxes. Audio
/// payload is never read here.
fn mp4_overlays(raw: &mut Raw, end: usize) -> io::Result<Vec<(usize, Vec<u8>)>> {
	let mut overlays = Vec::new();
	let mut position = 0;
	let mut moov = None;
	let mut mdat = false;
	for index in 0..MAX_MP4_BOXES {
		if position == end {
			break;
		}
		if end - position < 8 {
			return Err(invalid());
		}
		raw.jump(position)?;
		let mut header = [0; 16];
		let available = (end - position).min(16);
		raw.read_exact(&mut header[..available])?;
		let (header_len, size) = mp4_box(&header[..available], position, end)?;
		match &header[4..8] {
			b"ftyp" if index == 0 => {}
			_ if index == 0 => return Err(invalid()),
			b"moov" => {
				if moov.is_some() || size > MAX_MOOV {
					return Err(invalid());
				}
				let mut index = vec![0; size];
				raw.jump(position)?;
				raw.read_exact(&mut index)?;
				let mut children = 0;
				sanitize_mp4(&mut index[header_len..], 0, &mut children)?;
				moov = Some((position, index));
			}
			b"mdat" => mdat = true,
			b"free" | b"skip" | b"wide" => {}
			_ => overlays.push((position + 4, b"free".to_vec())),
		}
		position += size;
	}
	if position != end || !mdat {
		return Err(invalid());
	}
	overlays.push(moov.ok_or_else(invalid)?);
	Ok(overlays)
}

/// Rename metadata boxes (`udta`, `meta`) inside the movie index; the demuxer skips `free`.
fn sanitize_mp4(boxes: &mut [u8], depth: usize, children: &mut usize) -> io::Result<()> {
	if depth > 8 {
		return Err(invalid());
	}
	let mut at = 0;
	while at < boxes.len() {
		*children += 1;
		if *children > MAX_MP4_CHILDREN || boxes.len() - at < 8 {
			return Err(invalid());
		}
		let (header_len, size) = mp4_box(&boxes[at..], at, boxes.len())?;
		match &boxes[at + 4..at + 8] {
			b"udta" | b"meta" => boxes[at + 4..at + 8].copy_from_slice(b"free"),
			b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"dinf" => {
				sanitize_mp4(&mut boxes[at + header_len..at + size], depth + 1, children)?
			}
			_ => {}
		}
		at += size;
	}
	Ok(())
}

/// Keep only FLAC's STREAMINFO block; tags, pictures and padding are skipped unread.
/// Returns the replacement header and the offset of the first audio frame.
fn flac_prefix(raw: &mut Raw, end: usize) -> io::Result<(Vec<u8>, usize)> {
	let mut position = 4;
	let mut info = None;
	for _ in 0..MAX_FLAC_BLOCKS {
		raw.jump(position)?;
		let mut header = [0; 4];
		raw.read_exact(&mut header)?;
		let kind = header[0] & 0x7f;
		let len =
			usize::from(header[1]) << 16 | usize::from(header[2]) << 8 | usize::from(header[3]);
		let next = position
			.checked_add(4 + len)
			.filter(|next| *next < end)
			.ok_or_else(invalid)?;
		if kind == 0 {
			if position != 4 || len != 34 {
				return Err(invalid());
			}
			let mut block = vec![0; 34];
			raw.read_exact(&mut block)?;
			info = Some(block);
		} else if kind == 127 || info.is_none() {
			return Err(invalid());
		}
		position = next;
		if header[0] & 0x80 != 0 {
			let mut prefix = b"fLaC\x80\0\0\x22".to_vec();
			prefix.extend_from_slice(&info.ok_or_else(invalid)?);
			return Ok((prefix, position));
		}
	}
	Err(invalid())
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

	check_remote(24 * 1024 * 1024);
}

/// Decode a synthetic WAV of `len` bytes from a local range server: startup is one small
/// request, steady reads are prefetched in growing blocks, and a mismatched range is rejected.
#[cfg(any(test, all(debug_assertions, feature = "demo")))]
fn check_remote(len: usize) {
	use std::{
		io::Write,
		sync::atomic::{AtomicBool, AtomicUsize},
	};
	let mut fixture = super::demo_wav();
	fixture.resize(len, 0);
	let riff_size = (fixture.len() - 8) as u32;
	let data_size = (fixture.len() - 44) as u32;
	fixture[4..8].copy_from_slice(&riff_size.to_le_bytes());
	fixture[40..44].copy_from_slice(&data_size.to_le_bytes());
	let expected = fixture.len();
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let address = listener.local_addr().unwrap();
	let url = url::Url::parse(&format!("http://{address}/synthetic.wav")).unwrap();
	let requests = Arc::new(AtomicUsize::new(0));
	let corrupt = Arc::new(AtomicBool::new(false));
	let done = Arc::new(AtomicBool::new(false));
	let (observed, corrupt_next, stop) = (requests.clone(), corrupt.clone(), done.clone());
	let server = std::thread::spawn(move || {
		let mut probes = 0;
		for socket in listener.incoming() {
			if stop.load(Ordering::Acquire) {
				break;
			}
			let mut socket = socket.unwrap();
			socket
				.set_read_timeout(Some(Duration::from_secs(2)))
				.unwrap();
			let mut header = Vec::new();
			while !header.ends_with(b"\r\n\r\n") {
				assert!(header.len() < 8192);
				let mut byte = [0];
				if socket.read_exact(&mut byte).is_err() {
					break;
				}
				header.push(byte[0]);
			}
			let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
			// Some development hosts probe listening ports; these are not range reads.
			if header.starts_with("head / http/1.1\r\n") || header.is_empty() {
				probes += 1;
				assert!(probes <= 8, "too many unrelated local port probes");
				let _ = socket.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
				continue;
			}
			assert!(!header.contains("authorization:"));
			let range = header
				.split("range: bytes=")
				.nth(1)
				.and_then(|rest| rest.split("\r\n").next())
				.expect("every read is a range request");
			let (start, end) = range.split_once('-').unwrap();
			let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
			assert!(start <= end && end < expected);
			assert!(end + 1 - start <= MAX_BLOCK, "range blocks stay bounded");
			observed.fetch_add(1, Ordering::AcqRel);
			let total = expected + usize::from(corrupt_next.swap(false, Ordering::AcqRel));
			let _ = write!(
				socket,
				"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
				end + 1 - start
			);
			// A mismatched header is rejected before its body is consumed.
			let _ = socket.write_all(&fixture[start..=end]);
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
		duration: Duration::ZERO,
		generation: 0,
		url: Some(url),
		expected,
		#[cfg(feature = "demo")]
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
	// Header validation and tiny demuxer reads share the first small range; no read-ahead yet.
	assert_eq!(requests.load(Ordering::Acquire), 1);
	remote.read_exact(&mut [0; 64]).unwrap();
	assert_eq!(requests.load(Ordering::Acquire), 1);
	drop(remote);
	// A corrupt range must reach the network, not a cached copy of the first one.
	gate.clear_downloads();
	corrupt.store(true, Ordering::Release);
	assert!(
		self::source(
			&request,
			gate.clone(),
			wake.clone(),
			runtime.handle().clone()
		)
		.is_err()
	);
	let decode = || {
		let before = requests.load(Ordering::Acquire);
		let remote = self::source(
			&request,
			gate.clone(),
			wake.clone(),
			runtime.handle().clone(),
		)
		.unwrap();
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
		requests.load(Ordering::Acquire) - before
	};
	let used = decode();
	assert!(
		used <= expected.div_ceil(MAX_BLOCK) + 8,
		"{used} range requests for {expected} bytes"
	);
	// A seek replays the clip from the start; cached ranges are not downloaded again.
	let again = decode();
	assert!(
		again <= expected.saturating_sub(MAX_DOWNLOADS).div_ceil(MAX_BLOCK) + 2,
		"{again} range requests to replay {expected} bytes"
	);
	if expected <= MAX_DOWNLOADS {
		assert_eq!(again, 0);
	}
	gate.clear_downloads();
	assert!(gate.downloads.lock().unwrap().get(0, 0).is_none());
	done.store(true, Ordering::Release);
	let _ = std::net::TcpStream::connect(address);
	server.join().unwrap();
}

#[cfg(test)]
mod tests {
	use super::*;

	const M4A: &[u8] = include_bytes!("../../tests/fixtures/audio-tone.m4a");
	const ALAC: &[u8] = include_bytes!("../../tests/fixtures/audio-tone-alac.m4a");
	const FLAC: &[u8] = include_bytes!("../../tests/fixtures/audio-tone.flac");

	/// Box types directly inside `moov`, as the demuxer walks them.
	fn moov_children(file: &[u8]) -> Vec<[u8; 4]> {
		let mut at = 0;
		while &file[at + 4..at + 8] != b"moov" {
			at += u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
		}
		let end = at + u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
		let mut kinds = Vec::new();
		at += 8;
		while at < end {
			kinds.push(file[at + 4..at + 8].try_into().unwrap());
			at += u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
		}
		kinds
	}

	fn streamed(bytes: &[u8]) -> io::Result<Vec<u8>> {
		let mut source = Sanitized::new(Raw::memory(bytes.to_vec()))?;
		let mut streamed = Vec::new();
		source.read_to_end(&mut streamed)?;
		Ok(streamed)
	}

	#[test]
	fn mp4_and_flac_audio_decode_without_their_metadata() {
		for fixture in [M4A, ALAC, FLAC] {
			let source = Sanitized::new(Raw::memory(fixture.to_vec())).expect("sanitized");
			let mut samples = 0;
			super::super::decode_stream(
				Box::new(source),
				&|| true,
				&mut |pcm, channels, rate, _| {
					assert_eq!((channels, rate), (1, 24_000));
					samples += pcm.len();
					Ok(())
				},
			)
			.expect("decoded");
			// 0.2 s of 24 kHz mono, give or take encoder priming and padding.
			assert!((3_000..=7_000).contains(&samples), "{samples}");
		}
		// The AAC fixture's index follows its audio, so the source must seek; offsets stay put.
		assert!(
			Sanitized::new(Raw::memory(M4A.to_vec()))
				.unwrap()
				.is_seekable()
		);
		for fixture in [M4A, ALAC] {
			let bytes = streamed(fixture).unwrap();
			assert_eq!(bytes.len(), fixture.len());
			assert!(moov_children(fixture).contains(b"udta"));
			let children = moov_children(&bytes);
			assert!(children.contains(b"trak") && children.contains(b"free"));
			assert!(!children.contains(b"udta") && !children.contains(b"meta"));
		}
		let source = Sanitized::new(Raw::memory(FLAC.to_vec())).unwrap();
		assert!(!source.is_seekable());
		assert!(
			source.raw.bytes_read < 128,
			"tags and padding are never fetched"
		);
		let bytes = streamed(FLAC).unwrap();
		assert!(FLAC.windows(9).any(|w| w == b"Synthetic"));
		assert!(!bytes.windows(9).any(|w| w == b"Synthetic"));
	}

	#[test]
	fn remote_audio_streams_through_few_prefetched_ranges() {
		super::check_remote(4 * 1024 * 1024);
	}

	#[test]
	fn malformed_mp4_and_flac_are_rejected_before_decoding() {
		assert!(streamed(&M4A[..M4A.len() - 1]).is_err(), "truncated index");
		let mut no_ftyp = M4A.to_vec();
		no_ftyp[4..8].copy_from_slice(b"moov");
		assert!(streamed(&no_ftyp).is_err());
		let moov = M4A.windows(4).position(|w| w == b"moov").unwrap() - 4;
		let mut oversized = M4A.to_vec();
		oversized[moov..moov + 4].copy_from_slice(&u32::MAX.to_be_bytes());
		assert!(streamed(&oversized).is_err());
		let mut nested = M4A.to_vec();
		let trak = M4A.windows(4).position(|w| w == b"trak").unwrap() - 4;
		nested[trak..trak + 4].copy_from_slice(&4u32.to_be_bytes());
		assert!(streamed(&nested).is_err(), "child smaller than its header");
		let mut tags_first = FLAC.to_vec();
		tags_first[4] = 4;
		assert!(streamed(&tags_first).is_err(), "STREAMINFO must come first");
		// Blocks: STREAMINFO (34 bytes), tags (63), then the last block, padding.
		let mut endless = FLAC.to_vec();
		assert_eq!(endless[4 + 4 + 34 + 4 + 63] & 0x80, 0x80);
		endless[4 + 4 + 34 + 4 + 63] &= 0x7f;
		assert!(
			streamed(&endless).is_err(),
			"metadata must end before the file does"
		);
	}
}
