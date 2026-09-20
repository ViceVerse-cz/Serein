//! Optional local codec conversion; the helper receives private files, never service URLs.
use std::{
	fs::{self, File, OpenOptions},
	io::{Read, Write},
	os::unix::fs::{DirBuilderExt, OpenOptionsExt},
	path::PathBuf,
	process::{Command, Stdio},
	sync::atomic::{AtomicBool, Ordering},
	time::{Duration, Instant},
};

const LIMIT: u64 = 100 * 1024 * 1024;
const FAILED: &str = "This video could not be converted for playback";

struct Temporary(PathBuf);
impl Drop for Temporary {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.0);
	}
}

pub(super) fn open(
	source: Box<dyn platform::video::ReadSeek>,
	cancelled: &AtomicBool,
) -> Result<platform::video::Decoder, &'static str> {
	let executable = [
		"/opt/homebrew/bin/ffmpeg",
		"/usr/local/bin/ffmpeg",
		"/usr/bin/ffmpeg",
	]
	.into_iter()
	.find(|path| std::path::Path::new(path).is_file())
	.ok_or("This video needs FFmpeg for playback. Install FFmpeg and try again.")?;
	let mut random = [0; 16];
	getrandom::fill(&mut random).map_err(|_| FAILED)?;
	let name = random
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect::<String>();
	let directory = std::env::temp_dir().join(format!("serein-video-{name}"));
	fs::DirBuilder::new()
		.mode(0o700)
		.create(&directory)
		.map_err(|_| FAILED)?;
	let temporary = Temporary(directory);
	let result = convert(source, cancelled, executable, &temporary);
	fs::remove_dir_all(&temporary.0).map_err(
		|_| "Temporary video cleanup failed; remove serein-video files from the system temporary folder",
	)?;
	result
}

fn convert(
	mut source: Box<dyn platform::video::ReadSeek>,
	cancelled: &AtomicBool,
	executable: &str,
	temporary: &Temporary,
) -> Result<platform::video::Decoder, &'static str> {
	let input = temporary.0.join("input");
	let output = temporary.0.join("output.mp4");
	let mut file = OpenOptions::new()
		.write(true)
		.create_new(true)
		.mode(0o600)
		.open(&input)
		.map_err(|_| FAILED)?;
	let mut buffer = [0; 64 * 1024];
	let mut total = 0;
	loop {
		if cancelled.load(Ordering::Acquire) {
			return Err(FAILED);
		}
		let count = source.read(&mut buffer).map_err(|_| FAILED)?;
		if count == 0 {
			break;
		}
		total += count as u64;
		if total > LIMIT {
			return Err("Video preview limit: 100 MiB");
		}
		file.write_all(&buffer[..count]).map_err(|_| FAILED)?;
	}
	drop(file);
	drop(source);
	// The fixed shell applies a hard 128 MiB file ceiling before replacing itself.
	// FFmpeg's smaller soft ceiling leaves room for the final MP4 sample tables.
	let mut child = Command::new("/bin/sh")
		.args(["-c", "ulimit -f 131072 || exit 1; exec \"$@\"", "serein-video", executable])
		.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-max_alloc", "16777216",
			"-threads", "2", "-protocol_whitelist", "file", "-format_whitelist", "mov,matroska,webm",
			"-i"])
		.arg(&input)
		.args(["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn", "-map_metadata", "-1",
			"-filter_threads", "1", "-vf", "scale=w='max(2,min(1920,iw))':h='max(2,min(1080,ih))':force_original_aspect_ratio=decrease:force_divisible_by=2",
			"-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p",
			"-threads", "2", "-c:a", "aac", "-ac", "2", "-ar", "48000", "-fs", "104857600",
			"-movflags", "+faststart", "-n"])
		.arg(&output)
		.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
		.env_clear()
		.spawn().map_err(|_| FAILED)?;
	let deadline = Instant::now() + Duration::from_secs(120);
	let status = loop {
		if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
			let _ = child.kill();
			let _ = child.wait();
			return Err("Video conversion cancelled or exceeded two minutes");
		}
		match child.try_wait() {
			Ok(Some(status)) => break status,
			Ok(None) => std::thread::sleep(Duration::from_millis(25)),
			Err(_) => {
				let _ = child.kill();
				let _ = child.wait();
				return Err(FAILED);
			}
		}
	};
	if !status.success() {
		return Err(FAILED);
	}
	let file = File::open(&output).map_err(|_| FAILED)?;
	// Reaching the output ceiling can produce a valid but truncated movie: reject it.
	if file.metadata().map_err(|_| FAILED)?.len() >= LIMIT {
		return Err("Converted video exceeds the 100 MiB preview limit");
	}
	// macOS keeps the open descriptor valid after unlink; no playback file is retained.
	platform::video::Decoder::open(Box::new(file))
}
