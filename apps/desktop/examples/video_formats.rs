//! Offline helper smoke: synthesize WebM from our original MOV, then decode it inline.
#[cfg(target_os = "macos")]
#[path = "../src/video/fallback.rs"]
mod fallback;

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	use std::{
		io::{Read, Write},
		process::{Command, Stdio},
		sync::atomic::AtomicBool,
	};
	let executable = [
		"/opt/homebrew/bin/ffmpeg",
		"/usr/local/bin/ffmpeg",
		"/usr/bin/ffmpeg",
	]
	.into_iter()
	.find(|path| std::path::Path::new(path).is_file())
	.ok_or("Install FFmpeg to exercise WebM playback")?;
	let mut child = Command::new(executable)
		.args([
			"-nostdin",
			"-hide_banner",
			"-loglevel",
			"error",
			"-f",
			"mov",
			"-i",
			"pipe:0",
			"-t",
			"0.25",
			"-c:v",
			"libvpx-vp9",
			"-threads",
			"1",
			"-an",
			"-f",
			"webm",
			"pipe:1",
		])
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()?;
	child
		.stdin
		.take()
		.ok_or("Missing helper input")?
		.write_all(include_bytes!("../tests/fixtures/video.mov"))?;
	let mut bytes = Vec::new();
	child
		.stdout
		.take()
		.ok_or("Missing helper output")?
		.take(1024 * 1024)
		.read_to_end(&mut bytes)?;
	if !child.wait()?.success() {
		return Err("Synthetic WebM creation failed".into());
	}
	let mut decoder = fallback::open(
		Box::new(std::io::Cursor::new(bytes)),
		&AtomicBool::new(false),
	)?;
	assert!(matches!(
		decoder.read_video()?,
		Some(platform::video::Sample::Video { .. })
	));
	let mut mov = fallback::open(
		Box::new(std::io::Cursor::new(include_bytes!(
			"../tests/fixtures/video.mov"
		))),
		&AtomicBool::new(false),
	)?;
	assert!(matches!(
		mov.read_video()?,
		Some(platform::video::Sample::Video { .. })
	));
	assert!(fallback::eligible(platform::video::TOO_LARGE));
	assert!(fallback::eligible(platform::video::INVALID));
	assert!(!fallback::eligible(platform::video::TOO_LONG));
	// Valid AAC may use a media clock different from the PCM sample rate.
	// Halve both clock and sample deltas so presentation times remain unchanged.
	let mut bytes = include_bytes!("../tests/fixtures/video.mov").to_vec();
	let mdhd = bytes.windows(4).rposition(|tag| tag == b"mdhd").unwrap();
	let clock = mdhd + 16;
	assert_eq!(&bytes[clock..clock + 4], &48_000_u32.to_be_bytes());
	bytes[clock..clock + 4].copy_from_slice(&24_000_u32.to_be_bytes());
	let duration = u32::from_be_bytes(bytes[clock + 4..clock + 8].try_into()?);
	bytes[clock + 4..clock + 8].copy_from_slice(&(duration / 2).to_be_bytes());
	let stts = bytes.windows(4).rposition(|tag| tag == b"stts").unwrap();
	let count = u32::from_be_bytes(bytes[stts + 8..stts + 12].try_into()?);
	for index in 0..count as usize {
		let at = stts + 16 + index * 8;
		let delta = u32::from_be_bytes(bytes[at..at + 4].try_into()?);
		assert_eq!(delta % 2, 0);
		bytes[at..at + 4].copy_from_slice(&(delta / 2).to_be_bytes());
	}
	let mut native = platform::video::Decoder::open(Box::new(std::io::Cursor::new(bytes)))?;
	let mut packets = 0;
	while let Some(platform::video::Sample::Audio { pts, frames }) = native.read_audio()? {
		assert!(pts.is_finite() && !frames.is_empty());
		packets += 1;
	}
	assert!(packets > 100);
	println!(
		"Synthetic WebM/MOV conversion, fallback selection, and AAC with a separate media clock passed."
	);
	Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
	eprintln!("The optional FFmpeg fallback is currently macOS-only.");
}
