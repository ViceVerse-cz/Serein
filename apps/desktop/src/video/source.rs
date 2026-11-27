//! Decoder-driven video ranges. No file cache or eager whole-video download.
use std::{
	collections::VecDeque,
	io::{self, Read, Seek, SeekFrom},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	time::Duration,
};
use tokio::runtime::Handle;

const MAX_BYTES: usize = 100 * 1024 * 1024;
const CHUNK: usize = 256 * 1024;
const CACHE_CHUNKS: usize = 8;
const INVALID: &str = "Video download failed or changed; reload the conversation";
const UNSUPPORTED: &str = "Video server does not support buffering; download to play externally";
const TOO_LARGE: &str = "Video preview limit: 100 MiB";

/// `expected` is the attachment's byte size; `None` (embed videos) learns it from the
/// first range response, which stays cached as the first chunk.
pub(super) fn source(
	url: Option<url::Url>,
	expected: Option<usize>,
	cancelled: Arc<AtomicBool>,
	runtime: Handle,
) -> Result<Box<dyn platform::video::ReadSeek>, &'static str> {
	if expected.is_some_and(|expected| expected == 0 || expected > MAX_BYTES) {
		return Err(TOO_LARGE);
	}
	let (input, len) = if let Some(url) = url {
		// The caller has validated the service URL; never let URL userinfo add credentials.
		if !url.username().is_empty() || url.password().is_some() {
			return Err(INVALID);
		}
		let client = wreq::Client::builder()
			.no_proxy()
			.no_gzip()
			.no_brotli()
			.no_deflate()
			.no_zstd()
			.redirect(wreq::redirect::Policy::none())
			.timeout(Duration::from_secs(15))
			.build()
			.map_err(|_| INVALID)?;
		(
			Input::Http {
				client,
				url,
				runtime,
			},
			expected,
		)
	} else {
		#[cfg(not(feature = "demo"))]
		return Err(INVALID);
		#[cfg(feature = "demo")]
		{
			let bytes = demo_bytes();
			if bytes.is_empty() || bytes.len() > MAX_BYTES {
				return Err(INVALID);
			}
			(Input::Demo(bytes), Some(bytes.len()))
		}
	};
	let mut source = Source {
		input,
		cancelled,
		len: len.unwrap_or(0),
		position: 0,
		cache: VecDeque::new(),
	};
	if len.is_none() {
		let (bytes, len) = source.fetch(0, None).map_err(|error| match error.kind() {
			io::ErrorKind::Unsupported => UNSUPPORTED,
			io::ErrorKind::FileTooLarge => TOO_LARGE,
			_ => INVALID,
		})?;
		source.len = len;
		source.cache.push_back((0, bytes));
	}
	Ok(Box::new(source))
}

/// The demo attachment: the bundled fixture, or the local file named by
/// `--demo-video-file=<path>` so playback can be measured on a real-sized clip.
#[cfg(feature = "demo")]
fn demo_bytes() -> &'static [u8] {
	static BYTES: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();
	BYTES.get_or_init(|| {
		// One byte past the limit is enough to tell an oversized file without loading it.
		let read = |path: &str| {
			let mut bytes = Vec::new();
			std::fs::File::open(path)
				.ok()?
				.take(MAX_BYTES as u64 + 1)
				.read_to_end(&mut bytes)
				.ok()?;
			(bytes.len() <= MAX_BYTES).then_some(bytes)
		};
		std::env::args()
			.find_map(|arg| arg.strip_prefix("--demo-video-file=").map(read))
			.flatten()
			.map_or(
				include_bytes!("../../tests/fixtures/video.mov").as_slice(),
				|bytes| bytes.leak(),
			)
	})
}

enum Input {
	Http {
		client: wreq::Client,
		url: url::Url,
		runtime: Handle,
	},
	#[cfg(feature = "demo")]
	Demo(&'static [u8]),
}
struct Source {
	input: Input,
	cancelled: Arc<AtomicBool>,
	len: usize,
	position: usize,
	cache: VecDeque<(usize, Vec<u8>)>,
}
fn invalid() -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, INVALID)
}
/// Parses `bytes {first}-{last}/{total}` answering the chunk requested at `start`. A known
/// `len` must match the total; an unknown one is learned within [`MAX_BYTES`]. Returns the
/// chunk's byte count and the total.
fn content_range(value: &str, start: usize, len: Option<usize>) -> io::Result<(usize, usize)> {
	let number = |text: &str| {
		(!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
			.then(|| text.parse::<usize>().ok())
			.flatten()
	};
	let (range, total) = value
		.strip_prefix("bytes ")
		.and_then(|rest| rest.split_once('/'))
		.ok_or_else(invalid)?;
	let (first, last) = range.split_once('-').ok_or_else(invalid)?;
	let (Some(first), Some(last), Some(total)) = (number(first), number(last), number(total))
	else {
		return Err(invalid());
	};
	if len.is_some_and(|len| len != total) {
		return Err(invalid());
	}
	if total > MAX_BYTES {
		return Err(io::Error::new(io::ErrorKind::FileTooLarge, TOO_LARGE));
	}
	if first != start || start >= total || last != start + CHUNK.min(total - start) - 1 {
		return Err(invalid());
	}
	Ok((last - start + 1, total))
}
impl Source {
	fn check_cancelled(&self) -> io::Result<()> {
		if self.cancelled.load(Ordering::Acquire) {
			// Read::read_exact retries Interrupted indefinitely.
			Err(io::Error::new(
				io::ErrorKind::ConnectionAborted,
				"Cancelled",
			))
		} else {
			Ok(())
		}
	}
	fn fill(&mut self) -> io::Result<()> {
		// Retain both tracks across demuxer seeks. Evict before transfer so encoded
		// payloads, including the new range, stay within eight chunks / 2 MiB.
		if self.cache.len() == CACHE_CHUNKS {
			self.cache.pop_front();
		}
		let start = self.position / CHUNK * CHUNK;
		let (bytes, _) = self.fetch(start, Some(self.len))?;
		self.check_cancelled()?;
		self.cache.push_back((start, bytes));
		Ok(())
	}
	/// Transfers the chunk at `start`; `len` is the known total, or `None` to learn it.
	/// Returns the chunk and the total.
	fn fetch(&self, start: usize, len: Option<usize>) -> io::Result<(Vec<u8>, usize)> {
		let (client, url, runtime) = match &self.input {
			Input::Http {
				client,
				url,
				runtime,
			} => (client, url, runtime),
			#[cfg(feature = "demo")]
			Input::Demo(_) => return Err(invalid()),
		};
		let end = start + len.map_or(CHUNK, |len| CHUNK.min(len - start)) - 1;
		runtime.block_on(async {
			let cancelled = async {
				while !self.cancelled.load(Ordering::Acquire) {
					tokio::time::sleep(Duration::from_millis(20)).await;
				}
			};
			let transfer = async {
				let response = client_core::fingerprint::discord_request(
					client.get(url.as_str()),
					url.as_str(),
				)
				.header(wreq::header::ACCEPT_ENCODING, "identity")
				.header(wreq::header::RANGE, format!("bytes={start}-{end}"))
				.send()
				.await
				.map_err(|_| invalid())?;
				let (count, total) = if response.status() == wreq::StatusCode::PARTIAL_CONTENT {
					let range = response
						.headers()
						.get(wreq::header::CONTENT_RANGE)
						.and_then(|value| value.to_str().ok())
						.ok_or_else(invalid)?;
					content_range(range, start, len)?
				} else {
					// Without range support only a file within one chunk plays: it arrives whole.
					let total = len.or_else(|| {
						response
							.content_length()
							.and_then(|len| usize::try_from(len).ok())
					});
					match total {
						Some(total)
							if response.status() == wreq::StatusCode::OK
								&& start == 0 && total > 0
								&& total <= CHUNK =>
						{
							(total, total)
						}
						_ => return Err(io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED)),
					}
				};
				if response
					.headers()
					.get(wreq::header::CONTENT_ENCODING)
					.is_some_and(|value| value != "identity")
					|| response
						.content_length()
						.is_some_and(|len| len != count as u64)
				{
					return Err(invalid());
				}
				let mut bytes = Vec::with_capacity(count);
				let mut response_body = std::pin::pin!(response.bytes_stream());
				while let Some(chunk) = futures_util::StreamExt::next(&mut response_body)
					.await
					.transpose()
					.map_err(|_| invalid())?
				{
					if chunk.len() > count - bytes.len() {
						return Err(invalid());
					}
					bytes.extend_from_slice(&chunk);
				}
				if bytes.len() != count {
					return Err(invalid());
				}
				Ok((bytes, total))
			};
			tokio::select! { biased;
				_ = cancelled => Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Cancelled")),
				result = transfer => result,
			}
		})
	}
}
impl Read for Source {
	fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
		self.check_cancelled()?;
		let mut count = output.len().min(CHUNK).min(self.len - self.position);
		if count == 0 {
			return Ok(0);
		}
		#[cfg(feature = "demo")]
		if let Input::Demo(bytes) = self.input {
			output[..count].copy_from_slice(&bytes[self.position..self.position + count]);
			self.position += count;
			return Ok(count);
		}
		if let Some(index) = self.cache.iter().position(|(start, bytes)| {
			self.position >= *start && self.position < start + bytes.len()
		}) {
			let range = self.cache.remove(index).ok_or_else(invalid)?;
			self.cache.push_back(range);
		} else {
			self.fill()?;
		}
		let (start, bytes) = self.cache.back().ok_or_else(invalid)?;
		let offset = self.position - start;
		count = count.min(bytes.len() - offset);
		output[..count].copy_from_slice(&bytes[offset..offset + count]);
		self.position += count;
		Ok(count)
	}
}
impl Seek for Source {
	fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
		self.check_cancelled()?;
		let position = match position {
			SeekFrom::Start(position) => Some(position),
			SeekFrom::Current(offset) => (self.position as u64).checked_add_signed(offset),
			SeekFrom::End(offset) => (self.len as u64).checked_add_signed(offset),
		}
		.filter(|position| *position <= self.len as u64)
		.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid video seek"))?;
		self.position = position as usize;
		Ok(position)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{io::Write, net::TcpListener, sync::atomic::AtomicUsize};

	#[test]
	fn alternating_tracks_reuse_buffered_ranges() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		let url = url::Url::parse(&format!("http://{address}/tracks.mov")).unwrap();
		let expected = 8 * 1024 * 1024;
		let requests = Arc::new(AtomicUsize::new(0));
		let observed = requests.clone();
		let stopped = Arc::new(AtomicBool::new(false));
		let stop = stopped.clone();
		let server = std::thread::spawn(move || {
			loop {
				let (mut socket, _) = listener.accept().unwrap();
				if stop.load(Ordering::Acquire) {
					break;
				}
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
				let range = header
					.lines()
					.find_map(|line| line.strip_prefix("range: bytes="))
					.unwrap();
				let (start, end) = range.split_once('-').unwrap();
				let start = start.parse::<usize>().unwrap();
				let end = end.parse::<usize>().unwrap();
				let count = end - start + 1;
				assert!(count <= 256 * 1024 && end < expected);
				observed.fetch_add(1, Ordering::Release);
				std::thread::sleep(Duration::from_millis(10));
				write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{expected}\r\nContent-Length: {count}\r\nConnection: close\r\n\r\n").unwrap();
				socket
					.write_all(&vec![(start / (4 * 1024 * 1024)) as u8; count])
					.unwrap();
			}
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let mut input = source(
			Some(url),
			Some(expected),
			Arc::new(AtomicBool::new(false)),
			runtime.handle().clone(),
		)
		.unwrap();
		assert_eq!(requests.load(Ordering::Acquire), 0);
		let started = std::time::Instant::now();
		for sample in 0..32 {
			for track in 0..2 {
				input
					.seek(SeekFrom::Start(
						(track * 4 * 1024 * 1024 + sample * 4096) as u64,
					))
					.unwrap();
				let mut bytes = [0; 4096];
				input.read_exact(&mut bytes).unwrap();
				assert!(bytes.iter().all(|byte| *byte == track as u8));
			}
		}
		let elapsed = started.elapsed();
		let count = requests.load(Ordering::Acquire);
		stopped.store(true, Ordering::Release);
		std::net::TcpStream::connect(address).unwrap();
		server.join().unwrap();
		eprintln!(
			"alternating tracks: {count} requests, {elapsed:?}, 64 reads, 10 ms response delay"
		);
		assert_eq!(count, 2);
	}

	#[test]
	fn cancellation_interrupts_a_stalled_range_body() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/stalled.mov",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let cancelled = Arc::new(AtomicBool::new(false));
		let stop = cancelled.clone();
		let server = std::thread::spawn(move || {
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
			write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{CHUNK}\r\nContent-Length: {CHUNK}\r\n\r\n", CHUNK - 1).unwrap();
			std::thread::sleep(Duration::from_millis(40));
			stop.store(true, Ordering::Release);
			// Keep the response open until cancellation drops the connection.
			match socket.read(&mut [0]) {
				Ok(0) => {}
				Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
				other => panic!("cancelled range connection stayed open: {other:?}"),
			}
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let mut input =
			source(Some(url), Some(CHUNK), cancelled, runtime.handle().clone()).unwrap();
		assert_eq!(
			input.read(&mut [0]).unwrap_err().kind(),
			io::ErrorKind::ConnectionAborted
		);
		server.join().unwrap();
	}

	#[test]
	fn ranges_follow_reads_and_seeks_and_reject_changed_responses() {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/synthetic.mov",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let expected = CHUNK * (CACHE_CHUNKS + 1);
		let requests = Arc::new(AtomicUsize::new(0));
		let observed = requests.clone();
		let server = std::thread::spawn(move || {
			let starts = [0, CHUNK * CACHE_CHUNKS]
				.into_iter()
				.chain((1..CACHE_CHUNKS).map(|chunk| chunk * CHUNK))
				.chain([CHUNK * CACHE_CHUNKS]);
			for (index, start) in starts.enumerate() {
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
				assert!(
					header.contains(&format!("range: bytes={start}-{}\r\n", start + CHUNK - 1))
				);
				assert!(!header.contains("authorization:"));
				assert!(!header.contains("cookie:"));
				observed.fetch_add(1, Ordering::Release);
				let changed = index == CACHE_CHUNKS + 1;
				let total = expected + usize::from(changed);
				write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{total}\r\nContent-Length: {CHUNK}\r\nConnection: close\r\n\r\n", start + CHUNK - 1).unwrap();
				let result = socket.write_all(&vec![(start / CHUNK) as u8; CHUNK]);
				if !changed {
					result.unwrap();
				}
			}
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let cancelled = Arc::new(AtomicBool::new(false));
		let mut input = source(
			Some(url.clone()),
			Some(expected),
			cancelled.clone(),
			runtime.handle().clone(),
		)
		.unwrap();
		assert_eq!(requests.load(Ordering::Acquire), 0);
		let mut bytes = [1; 32];
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [0; 32]);
		assert_eq!(requests.load(Ordering::Acquire), 1);
		input.seek(SeekFrom::Start(8)).unwrap();
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(requests.load(Ordering::Acquire), 1);
		input.seek(SeekFrom::End(-32)).unwrap();
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [CACHE_CHUNKS as u8; 32]);
		assert_eq!(requests.load(Ordering::Acquire), 2);
		assert_eq!(input.read(&mut bytes).unwrap(), 0);
		assert!(input.seek(SeekFrom::End(1)).is_err());
		assert!(input.seek(SeekFrom::Start(u64::MAX)).is_err());
		assert!(input.seek(SeekFrom::Current(i64::MIN)).is_err());
		for chunk in 1..CACHE_CHUNKS {
			// Keep the first range hot while loading enough others to evict the tail.
			input.seek(SeekFrom::Start(0)).unwrap();
			input.read_exact(&mut bytes).unwrap();
			assert_eq!(bytes, [0; 32]);
			input.seek(SeekFrom::Start((chunk * CHUNK) as u64)).unwrap();
			input.read_exact(&mut bytes).unwrap();
			assert_eq!(bytes, [chunk as u8; 32]);
		}
		assert_eq!(requests.load(Ordering::Acquire), CACHE_CHUNKS + 1);
		input.seek(SeekFrom::Start(0)).unwrap();
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [0; 32]);
		input.seek(SeekFrom::End(-32)).unwrap();
		assert_eq!(
			input.read(&mut bytes).unwrap_err().kind(),
			io::ErrorKind::InvalidData
		);
		assert_eq!(requests.load(Ordering::Acquire), CACHE_CHUNKS + 2);
		server.join().unwrap();
		cancelled.store(true, Ordering::Release);
		assert_eq!(
			input.read(&mut bytes).unwrap_err().kind(),
			io::ErrorKind::ConnectionAborted
		);
		assert!(
			source(
				Some(url),
				Some(MAX_BYTES + 1),
				cancelled,
				runtime.handle().clone()
			)
			.is_err()
		);
	}

	#[test]
	fn unknown_length_is_learned_from_the_first_range_and_bounded() {
		let total = CHUNK * 2 + 7;
		let serve = |totals: Vec<usize>| {
			let listener = TcpListener::bind("127.0.0.1:0").unwrap();
			let url = url::Url::parse(&format!(
				"http://{}/embed.mp4",
				listener.local_addr().unwrap()
			))
			.unwrap();
			let server = std::thread::spawn(move || {
				let mut starts = Vec::new();
				for reported in totals {
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
					let range = header
						.lines()
						.find_map(|line| line.strip_prefix("range: bytes="))
						.unwrap();
					let (start, end) = range.split_once('-').unwrap();
					let start = start.parse::<usize>().unwrap();
					// The probe asks for one whole chunk; the server clips it to the file.
					let end = end.parse::<usize>().unwrap().min(reported - 1);
					let count = end - start + 1;
					starts.push(start);
					write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{reported}\r\nContent-Length: {count}\r\nConnection: close\r\n\r\n").unwrap();
					let _ = socket.write_all(&vec![(start / CHUNK) as u8 + 1; count]);
				}
				starts
			});
			(url, server)
		};
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let open = |url| {
			source(
				Some(url),
				None,
				Arc::new(AtomicBool::new(false)),
				runtime.handle().clone(),
			)
		};
		let (url, server) = serve(vec![total, total]);
		let mut input = open(url).unwrap();
		// The probed first chunk is kept: reading it costs no second request.
		let mut bytes = [0; 16];
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [1; 16]);
		assert_eq!(input.seek(SeekFrom::End(0)).unwrap(), total as u64);
		input.seek(SeekFrom::End(-7)).unwrap();
		let mut tail = [0; 7];
		input.read_exact(&mut tail).unwrap();
		assert_eq!(tail, [3; 7]);
		assert_eq!(input.read(&mut tail).unwrap(), 0);
		assert_eq!(server.join().unwrap(), [0, CHUNK * 2]);
		// A reported size past the preview limit is refused before any playback read.
		let (url, server) = serve(vec![MAX_BYTES + 1]);
		assert_eq!(open(url).err(), Some(TOO_LARGE));
		server.join().unwrap();
		// A later range reporting another total means the file changed.
		let (url, server) = serve(vec![total, total + 1]);
		let mut input = open(url).unwrap();
		input.seek(SeekFrom::Start(CHUNK as u64)).unwrap();
		assert_eq!(
			input.read(&mut bytes).unwrap_err().kind(),
			io::ErrorKind::InvalidData
		);
		server.join().unwrap();
		for malformed in [
			"bytes 0-9/",
			"bytes 1-10/20",
			"bytes 0-8/20",
			"bytes 0-+9/10",
			"items 0-9/10",
		] {
			assert!(content_range(malformed, 0, None).is_err(), "{malformed}");
		}
	}
}
