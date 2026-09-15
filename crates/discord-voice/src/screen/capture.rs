#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use super::{RawFrame, Settings, Source};
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use std::sync::{Arc, atomic::AtomicBool, mpsc::SyncSender};

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) const MAX_SOURCES: usize = 64;
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
pub(super) const MAX_NAME_BYTES: usize = 256;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) const MAX_SOURCE_WIDTH: u32 = 7680;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) const MAX_SOURCE_HEIGHT: u32 = 4320;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) const MAX_FRAME_WIDTH: u32 = 3840;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) const MAX_FRAME_HEIGHT: u32 = 2160;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) const MAX_RAW_BYTES: usize = MAX_FRAME_WIDTH as usize * MAX_FRAME_HEIGHT as usize * 4;

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
pub(super) fn bounded_name(mut name: String) -> String {
	if name.len() > MAX_NAME_BYTES {
		let mut end = MAX_NAME_BYTES;
		while !name.is_char_boundary(end) {
			end -= 1;
		}
		name.truncate(end);
		name.shrink_to_fit();
	}
	name
}

#[cfg(target_os = "macos")]
#[path = "capture_macos.rs"]
mod imp;
#[cfg(target_os = "windows")]
#[path = "capture_windows.rs"]
mod imp;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) use imp::{Capture, sources};

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn sources() -> Result<Vec<Source>, &'static str> {
	Err("Screen sharing is supported only on macOS and Windows")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) struct Capture;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl Capture {
	pub(crate) fn start(
		_settings: Settings,
		_frames: SyncSender<RawFrame>,
		_audio: Option<tokio::sync::mpsc::Sender<crate::screen::AudioChunk>>,
		_stop: Arc<AtomicBool>,
		_ready: Arc<AtomicBool>,
		_audio_epoch: Arc<std::sync::atomic::AtomicU64>,
	) -> Result<Self, &'static str> {
		Err("Screen sharing is supported only on macOS and Windows")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn source_names_end_on_utf8_boundaries() {
		let name = bounded_name(format!("{}é", "a".repeat(MAX_NAME_BYTES - 1)));
		assert_eq!(name.len(), MAX_NAME_BYTES - 1);
	}
}
