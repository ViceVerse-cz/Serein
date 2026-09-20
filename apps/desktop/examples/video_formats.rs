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
	println!("Synthetic WebM and MOV conversion produced native video frames.");
	Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {
	eprintln!("The optional FFmpeg fallback is currently macOS-only.");
}
