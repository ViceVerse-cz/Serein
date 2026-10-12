//! Offline checks against the packaged executable's pre-GUI PDF entrypoint.
#[path = "../src/pdf/protocol.rs"]
mod protocol;
use std::{
	io::{Read, Write},
	process::{Command, Stdio},
};
use ui::PdfError;
fn spawn_helper() -> std::process::Child {
	let mut command = Command::new(env!("CARGO_BIN_EXE_serein"));
	command
		.arg("--serein-pdf-helper")
		.env_clear()
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null());
	#[cfg(target_os = "windows")]
	if let Some(root) = std::env::var_os("SYSTEMROOT") {
		command.env("SYSTEMROOT", root);
	}
	command.spawn().unwrap()
}
fn helper(page: u32, target: [u16; 2], data: &[u8]) -> Result<protocol::Raster, PdfError> {
	let mut child = spawn_helper();
	let mut input = child.stdin.take().unwrap();
	protocol::write_request(&mut input, page, target, data).unwrap();
	drop(input);
	let result = protocol::read_response(&mut child.stdout.take().unwrap(), page);
	assert!(child.wait().unwrap().success());
	result
}
#[test]
fn real_helper_renders_two_pages_without_a_display_or_account() {
	let data = include_bytes!("fixtures/pdf-preview.pdf");
	for target in [[1536, 940], [816, 220], [1536, 1536]] {
		for page in 0..2 {
			let image = helper(page, target, data).unwrap();
			assert_eq!(image.pages, 2);
			assert!(image.height > image.width);
			assert!(image.width <= target[0] as usize && image.height <= target[1] as usize);
			assert_eq!(image.rgba.len(), image.width * image.height * 4);
		}
	}
	let target = [1536, 940];
	assert_eq!(helper(2, target, data).err(), Some(PdfError::Invalid));
	assert_eq!(
		helper(0, target, b"%PDF-malformed").err(),
		Some(PdfError::Invalid)
	);
	assert_eq!(
		helper(0, target, include_bytes!("fixtures/pdf-password.pdf")).err(),
		Some(PdfError::Password)
	);
}
#[test]
fn oversized_header_is_rejected_without_reading_a_payload() {
	let mut child = spawn_helper();
	let mut input = child.stdin.take().unwrap();
	input.write_all(b"SPDF0002").unwrap();
	input.write_all(&0_u32.to_le_bytes()).unwrap();
	input.write_all(&u32::MAX.to_le_bytes()).unwrap();
	input.write_all(&1536_u32.to_le_bytes()).unwrap();
	input.write_all(&940_u32.to_le_bytes()).unwrap();
	drop(input);
	assert_eq!(
		protocol::read_response(&mut child.stdout.take().unwrap(), 0).err(),
		Some(PdfError::Limit)
	);
	assert!(child.wait().unwrap().success());
}
#[test]
fn invalid_target_and_previous_protocol_exit_cleanly_without_payload() {
	for (magic, target, expected) in [
		(b"SPDF0002", [0_u32, 940], PdfError::Limit),
		(b"SPDF0002", [1536, u32::MAX], PdfError::Limit),
		(b"SPDF0001", [1536, 940], PdfError::Invalid),
	] {
		let mut child = spawn_helper();
		let mut input = child.stdin.take().unwrap();
		input.write_all(magic).unwrap();
		for field in [0, ui::MAX_PDF_BYTES as u32, target[0], target[1]] {
			input.write_all(&field.to_le_bytes()).unwrap();
		}
		drop(input);
		assert_eq!(
			protocol::read_response(&mut child.stdout.take().unwrap(), 0).err(),
			Some(expected)
		);
		assert!(child.wait().unwrap().success());
	}
}
#[test]
fn real_helper_downscales_rgb_softmask_and_grayscale_images() {
	let image = helper(0, [1536, 940], include_bytes!("fixtures/pdf-images.pdf")).unwrap();
	assert_eq!((image.width, image.height, image.pages), (723, 940, 1));
	assert!(
		image
			.rgba
			.as_chunks::<4>()
			.0
			.iter()
			.any(|p| p[1].saturating_sub(p[0]) > 50 && p[1].saturating_sub(p[2]) > 50)
	);
}
#[test]
fn truncated_header_exits_cleanly() {
	let mut child = spawn_helper();
	child.stdin.take().unwrap().write_all(b"SPDF").unwrap();
	let mut output = Vec::new();
	child
		.stdout
		.take()
		.unwrap()
		.read_to_end(&mut output)
		.unwrap();
	assert!(child.wait().unwrap().success());
	assert_eq!(
		protocol::read_response(&mut output.as_slice(), 0).err(),
		Some(PdfError::Invalid)
	);
}
#[test]
fn independent_watchdog_exits_when_parent_leaves_input_open() {
	// Keep stdin open without a header: a parent crash/pipe stall must not leave
	// a renderer process alive indefinitely, even without the parent poll loop.
	let mut child = spawn_helper();
	let start = std::time::Instant::now();
	let status = loop {
		if let Some(status) = child.try_wait().unwrap() {
			break status;
		}
		if start.elapsed() > std::time::Duration::from_secs(15) {
			child.kill().unwrap();
			child.wait().unwrap();
			panic!("helper watchdog failed to exit");
		}
		std::thread::sleep(std::time::Duration::from_millis(20));
	};
	assert_eq!(status.code(), Some(2));
	assert!(start.elapsed() >= std::time::Duration::from_secs(9));
	assert!(start.elapsed() < std::time::Duration::from_secs(15));
}

#[cfg(target_os = "linux")]
#[test]
fn helper_applies_resource_limits_before_reading_input() {
	let mut child = spawn_helper();
	let pid = child.id();
	let start = std::time::Instant::now();
	loop {
		let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
		if limits.lines().any(|l| {
			l.starts_with("Max address space")
				&& l.contains(&platform::pdf::MEMORY_BYTES.to_string())
		}) {
			assert!(
				limits
					.lines()
					.any(|l| l.starts_with("Max cpu time")
						&& l.split_whitespace().nth(3) == Some("8"))
			);
			assert!(
				limits.lines().any(|l| l.starts_with("Max core file size")
					&& l.split_whitespace().nth(4) == Some("0"))
			);
			break;
		}
		assert!(start.elapsed() < std::time::Duration::from_secs(3));
		std::thread::sleep(std::time::Duration::from_millis(10));
	}
	child.kill().unwrap();
	child.wait().unwrap();
	assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}
