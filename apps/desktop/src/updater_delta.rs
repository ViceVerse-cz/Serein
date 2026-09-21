//! Bounded zsync 0.6 block reuse. URLs and filenames in control files are never used.
use super::*;
use std::{
	collections::HashMap,
	fs::File,
	io::{Read, Seek, SeekFrom, Write},
	path::Path,
};

pub(super) const MAX_CONTROL: usize = 16 * 1024 * 1024;
const MAX_BLOCKS: usize = 262_144;
const BUFFER: usize = 1024 * 1024;
const INVALID: &str = "Unsupported or invalid delta metadata.";

struct Control {
	block: usize,
	weak_bytes: usize,
	strong_bytes: usize,
	length: u64,
	sums: Vec<u8>,
}

impl Control {
	fn parse(body: &[u8], length: u64) -> Result<Self, String> {
		if body.len() > MAX_CONTROL || length == 0 || length > MAX_DOWNLOAD {
			return Err(INVALID.into());
		}
		let end = body.windows(2).position(|w| w == b"\n\n").ok_or(INVALID)?;
		if end > 4096 {
			return Err(INVALID.into());
		}
		let header = std::str::from_utf8(&body[..end]).map_err(|_| INVALID)?;
		let mut fields = HashMap::new();
		for line in header.lines() {
			let (key, value) = line.split_once(": ").ok_or(INVALID)?;
			if fields.insert(key, value).is_some() {
				return Err(INVALID.into());
			}
		}
		// zsync 0.6 serializes the trailing bytes of its big-endian rolling sum.
		// Other formats/algorithms safely fall back to the full download.
		if fields.get("zsync") != Some(&"0.6.2")
			|| fields
				.get("Strong-Hash-Algorithm")
				.is_some_and(|v| *v != "MD4")
		{
			return Err(INVALID.into());
		}
		let number = |name| {
			fields
				.get(name)
				.ok_or(INVALID)?
				.parse::<u64>()
				.map_err(|_| INVALID)
		};
		let block = number("Blocksize")?;
		if !(2048..=65_536).contains(&block)
			|| !block.is_power_of_two()
			|| number("Length")? != length
		{
			return Err(INVALID.into());
		}
		let lengths = fields
			.get("Hash-Lengths")
			.ok_or(INVALID)?
			.split(',')
			.map(str::parse::<usize>)
			.collect::<Result<Vec<_>, _>>()
			.map_err(|_| INVALID)?;
		if lengths.len() != 3
			|| !(1..=2).contains(&lengths[0])
			|| !(2..=4).contains(&lengths[1])
			|| !(3..=16).contains(&lengths[2])
		{
			return Err(INVALID.into());
		}
		let count = length.div_ceil(block) as usize;
		let sums = &body[end + 2..];
		if count > MAX_BLOCKS || sums.len() != count * (lengths[1] + lengths[2]) {
			return Err(INVALID.into());
		}
		Ok(Self {
			block: block as usize,
			weak_bytes: lengths[1],
			strong_bytes: lengths[2],
			length,
			sums: sums.to_vec(),
		})
	}

	fn seed(
		&self,
		source: &mut File,
		output: &mut File,
		cancel: &AtomicBool,
		progress: &AtomicU64,
	) -> Result<Vec<(u64, u64)>, String> {
		let metadata = source.metadata().map_err(|_| INVALID)?;
		if !metadata.is_file() || metadata.len() > MAX_DOWNLOAD {
			return Err(INVALID.into());
		}
		let mut targets: HashMap<(u32, [u8; 16]), Vec<usize>> = HashMap::new();
		let mut weak_keys: HashMap<u32, usize> = HashMap::new();
		let record = self.weak_bytes + self.strong_bytes;
		for (index, sum) in self.sums.chunks_exact(record).enumerate() {
			let mut weak = [0; 4];
			weak[4 - self.weak_bytes..].copy_from_slice(&sum[..self.weak_bytes]);
			let weak = u32::from_be_bytes(weak);
			let mut strong = [0; 16];
			strong[..self.strong_bytes].copy_from_slice(&sum[self.weak_bytes..]);
			targets.entry((weak, strong)).or_default().push(index);
			*weak_keys.entry(weak).or_default() += 1;
		}
		let mask = u32::MAX >> (8 * (4 - self.weak_bytes));
		let mut known = vec![false; self.sums.len() / record];
		let mut buffer = vec![0; BUFFER + self.block];
		let mut offset = 0;
		let mut reused = 0;
		let mut hashed = 0_u64;
		let started = Instant::now();
		'scan: while offset < metadata.len() {
			let size = (metadata.len() - offset).min(buffer.len() as u64) as usize;
			buffer.fill(0);
			source
				.seek(SeekFrom::Start(offset))
				.and_then(|_| source.read_exact(&mut buffer[..size]))
				.map_err(|_| INVALID)?;
			let windows = (metadata.len() - offset).min(BUFFER as u64) as usize;
			let (mut a, mut b) = rolling(&buffer[..self.block]);
			for position in 0..windows {
				if position % 4096 == 0
					&& (cancel.load(Ordering::Relaxed)
						|| started.elapsed() > Duration::from_secs(15))
				{
					return Err("Delta scan cancelled or exceeded its time limit.".into());
				}
				let weak = ((u32::from(a) << 16) | u32::from(b)) & mask;
				if weak_keys.contains_key(&weak) {
					hashed += self.block as u64;
					if hashed > 2 * MAX_DOWNLOAD {
						return Err("Delta scan exceeded its work limit.".into());
					}
					let mut strong: [u8; 16] =
						md4::Md4::digest(&buffer[position..position + self.block]).into();
					strong[self.strong_bytes..].fill(0);
					if let Some(indices) = targets.remove(&(weak, strong)) {
						if let Some(remaining) = weak_keys.get_mut(&weak) {
							*remaining -= indices.len();
							if *remaining == 0 {
								weak_keys.remove(&weak);
							}
						}
						for (copied, index) in indices.into_iter().enumerate() {
							if copied % 256 == 0
								&& (cancel.load(Ordering::Relaxed)
									|| started.elapsed() > Duration::from_secs(15))
							{
								return Err(
									"Delta scan cancelled or exceeded its time limit.".into()
								);
							}
							let start = (index * self.block) as u64;
							let bytes = (self.length - start).min(self.block as u64) as usize;
							output
								.seek(SeekFrom::Start(start))
								.and_then(|_| output.write_all(&buffer[position..position + bytes]))
								.map_err(|_| INVALID)?;
							known[index] = true;
							reused += bytes as u64;
						}
						progress.store(reused, Ordering::Relaxed);
						if targets.is_empty() {
							break 'scan;
						}
					}
				}
				let old = u16::from(buffer[position]);
				a = a
					.wrapping_sub(old)
					.wrapping_add(u16::from(buffer[position + self.block]));
				b = b
					.wrapping_sub(old.wrapping_mul(self.block as u16))
					.wrapping_add(a);
			}
			offset += windows as u64;
		}
		if reused == 0 {
			return Err("No reusable AppImage blocks.".into());
		}
		// Coalesce nearby holes to avoid a request per block; cap requests for fragmented images.
		let mut ranges: Vec<(u64, u64)> = Vec::new();
		for (index, present) in known.into_iter().enumerate() {
			if present {
				continue;
			}
			let start = (index * self.block) as u64;
			let end = (start + self.block as u64).min(self.length);
			if let Some(last) = ranges.last_mut().filter(|last| start - last.1 <= 64 * 1024) {
				last.1 = end;
			} else {
				if ranges.len() == 128 {
					return Err("Delta update is too fragmented.".into());
				}
				ranges.push((start, end));
			}
		}
		Ok(ranges)
	}
}

fn rolling(bytes: &[u8]) -> (u16, u16) {
	let mut a = 0_u16;
	let mut b = 0_u16;
	for byte in bytes {
		a = a.wrapping_add(u16::from(*byte));
		b = b.wrapping_add(a);
	}
	(a, b)
}

pub(super) async fn download(
	client: &reqwest::Client,
	archive: &Asset,
	control: &Asset,
	checksums: &[u8],
	stage: &Staged,
	cancel: &Arc<AtomicBool>,
	progress: &Arc<AtomicU64>,
) -> Result<(), String> {
	let expected = checksum(checksums, &archive.name)?;
	let body = bounded_body(client, &control.browser_download_url, MAX_CONTROL, cancel).await?;
	if body.len() as u64 != control.size
		|| Sha256::digest(&body).as_slice() != checksum(checksums, &control.name)?
	{
		return Err(INVALID.into());
	}
	let parsed = Control::parse(&body, archive.size)?;
	drop(body);
	let source = stage.installation.clone();
	let path = stage.directory.join("package.zip");
	let output = path.clone();
	let scan_cancel = Arc::clone(cancel);
	let scan_progress = Arc::clone(progress);
	let ranges = tokio::task::spawn_blocking(move || {
		let mut source = File::open(source).map_err(|_| INVALID)?;
		let mut output = File::options()
			.write(true)
			.create_new(true)
			.open(output)
			.map_err(|_| INVALID)?;
		output.set_len(parsed.length).map_err(|_| INVALID)?;
		parsed.seed(&mut source, &mut output, &scan_cancel, &scan_progress)
	})
	.await
	.map_err(|_| INVALID)??;
	tokio::time::timeout(Duration::from_secs(120), async {
		use tokio::io::{AsyncSeekExt, AsyncWriteExt};
		let mut output = tokio::fs::OpenOptions::new()
			.write(true)
			.open(&path)
			.await
			.map_err(|_| INVALID)?;
		let mut completed =
			archive.size - ranges.iter().map(|(start, end)| end - start).sum::<u64>();
		progress.store(completed, Ordering::Relaxed);
		for (start, end) in ranges {
			if cancel.load(Ordering::Relaxed) {
				return Err("Update cancelled.");
			}
			let mut response = client
				.get(&archive.browser_download_url)
				.header(reqwest::header::RANGE, format!("bytes={start}-{}", end - 1))
				.header(reqwest::header::ACCEPT_ENCODING, "identity")
				.timeout(Duration::from_secs(30))
				.send()
				.await
				.map_err(|_| INVALID)?;
			let content_range = format!("bytes {start}-{}/{}", end - 1, archive.size);
			if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
				|| response
					.headers()
					.get(reqwest::header::CONTENT_RANGE)
					.and_then(|v| v.to_str().ok())
					!= Some(content_range.as_str())
				|| response
					.content_length()
					.is_some_and(|size| size != end - start)
			{
				return Err("Server did not provide the requested delta range.");
			}
			output
				.seek(SeekFrom::Start(start))
				.await
				.map_err(|_| INVALID)?;
			let mut received = 0;
			while let Some(chunk) = response.chunk().await.map_err(|_| INVALID)? {
				if cancel.load(Ordering::Relaxed) || chunk.len() as u64 > end - start - received {
					return Err(INVALID);
				}
				output.write_all(&chunk).await.map_err(|_| INVALID)?;
				received += chunk.len() as u64;
				completed += chunk.len() as u64;
				progress.store(completed, Ordering::Relaxed);
			}
			if received != end - start {
				return Err(INVALID);
			}
		}
		output.sync_all().await.map_err(|_| INVALID)
	})
	.await
	.map_err(|_| INVALID)?
	.map_err(str::to_owned)?;
	let size = archive.size;
	let cancel = Arc::clone(cancel);
	tokio::task::spawn_blocking(move || verify(&path, size, &expected, &cancel))
		.await
		.map_err(|_| INVALID)?
}

fn verify(path: &Path, size: u64, expected: &[u8; 32], cancel: &AtomicBool) -> Result<(), String> {
	let mut file = File::open(path).map_err(|_| INVALID)?;
	if file.metadata().map_err(|_| INVALID)?.len() != size {
		return Err(INVALID.into());
	}
	let mut hash = Sha256::new();
	let mut buffer = vec![0; BUFFER];
	let mut remaining = size;
	while remaining > 0 {
		if cancel.load(Ordering::Relaxed) {
			return Err("Update cancelled.".into());
		}
		let bytes = remaining.min(BUFFER as u64) as usize;
		file.read_exact(&mut buffer[..bytes]).map_err(|_| INVALID)?;
		hash.update(&buffer[..bytes]);
		remaining -= bytes as u64;
	}
	if hash.finalize().as_slice() != expected {
		return Err("Delta checksum mismatch.".into());
	}
	Ok(())
}

#[cfg(feature = "demo")]
pub(super) fn debug_check() -> Result<(), String> {
	if rolling(b"abc") != (294, 586) {
		return Err("Legacy rolling checksum compatibility failed.".into());
	}
	if md4::Md4::digest(b"abc").as_slice()
		!= [
			0xa4, 0x48, 0x01, 0x7a, 0xaf, 0x21, 0xd8, 0x52, 0x5f, 0xc1, 0x0a, 0xe8, 0x7a, 0xa6,
			0x72, 0x9d,
		] {
		return Err("MD4 compatibility failed.".into());
	}
	let directory = std::env::temp_dir().join(format!("serein-delta-debug-{}", std::process::id()));
	std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
	let result = (|| {
		let mut random = 42_u32;
		let target: Vec<u8> = (0..(2048 * 4 + 123))
			.map(|_| {
				random ^= random << 13;
				random ^= random >> 17;
				random ^= random << 5;
				random as u8
			})
			.collect();
		let mut body = format!(
			"zsync: 0.6.2\nBlocksize: 2048\nLength: {}\nHash-Lengths: 2,3,6\n\n",
			target.len()
		)
		.into_bytes();
		for block in target.chunks(2048) {
			let mut padded = [0; 2048];
			padded[..block.len()].copy_from_slice(block);
			let (a, b) = rolling(&padded);
			body.extend_from_slice(&((u32::from(a) << 16) | u32::from(b)).to_be_bytes()[1..]);
			body.extend_from_slice(&md4::Md4::digest(padded)[..6]);
		}
		let control = Control::parse(&body, target.len() as u64)?;
		if Control::parse(&body[..body.len() - 1], target.len() as u64).is_ok()
			|| Control::parse(&body, target.len() as u64 + 1).is_ok()
			|| Control::parse(b"zsync: 0.6.2\nBlocksize: 0\nLength: 1\n\n", 1).is_ok()
		{
			return Err("Invalid delta metadata accepted.".into());
		}
		let seed = directory.join("seed");
		let output = directory.join("output");
		let mut old = b"shifted prefix".to_vec();
		old.extend_from_slice(&target[..2048]);
		old.extend_from_slice(&target[4096..]);
		std::fs::write(&seed, old).map_err(|e| e.to_string())?;
		let mut input = File::open(&seed).map_err(|e| e.to_string())?;
		let mut file = File::create(&output).map_err(|e| e.to_string())?;
		file.set_len(target.len() as u64)
			.map_err(|e| e.to_string())?;
		let cancel = AtomicBool::new(false);
		let progress = AtomicU64::new(0);
		let ranges = control.seed(&mut input, &mut file, &cancel, &progress)?;
		if ranges != [(2048, 4096)] {
			return Err(format!("Shifted block reuse failed: {ranges:?}"));
		}
		for (start, end) in ranges {
			file.seek(SeekFrom::Start(start))
				.and_then(|_| file.write_all(&target[start as usize..end as usize]))
				.map_err(|e| e.to_string())?;
		}
		drop(file);
		verify(
			&output,
			target.len() as u64,
			&Sha256::digest(&target).into(),
			&cancel,
		)?;
		if verify(&output, target.len() as u64, &[0; 32], &cancel).is_ok() {
			return Err("Corrupt delta result accepted.".into());
		}
		let mut repeated = vec![0; 3];
		repeated.extend_from_slice(&md4::Md4::digest([0; 2048])[..6]);
		repeated.extend_from_slice(&control.sums[..9]);
		let repeated = Control {
			block: 2048,
			weak_bytes: 3,
			strong_bytes: 6,
			length: 4096,
			sums: repeated,
		};
		std::fs::write(&seed, vec![0; BUFFER]).map_err(|e| e.to_string())?;
		let mut zeros = File::open(&seed).map_err(|e| e.to_string())?;
		let mut file = File::create(&output).map_err(|e| e.to_string())?;
		if repeated.seed(&mut zeros, &mut file, &cancel, &progress)? != [(2048, 4096)] {
			return Err("Repeated seed blocks exceeded the matching budget.".into());
		}
		cancel.store(true, Ordering::Relaxed);
		if control
			.seed(
				&mut input,
				&mut File::create(&output).map_err(|e| e.to_string())?,
				&cancel,
				&progress,
			)
			.is_ok()
		{
			return Err("Cancelled delta scan continued.".into());
		}
		Ok(())
	})();
	let cleanup = std::fs::remove_dir_all(&directory).map_err(|e| e.to_string());
	result.and(cleanup)
}
