//! Driver capability queries and optional bounded tests of the outgoing FFmpeg path.
//! Run in the desktop's disposable probe process: native drivers can block even
//! during open or cleanup, so the caller owns the wall-clock timeout.

use crate::video_encode::{Config, Encoder, Profile};
use model::voice_settings::{HardwareBackend, HardwareSupport, ProbeResult, VideoCodec};

const MAX_PROBE_FRAMES: usize = crate::video_encode::MAX_PENDING_PICTURES;

#[allow(unsafe_code)]
unsafe extern "C" {
	fn serein_video_query(backend: i32, codec: i32) -> i32;
}

/// Ask the vendor driver about the codec without submitting synthetic pictures.
/// This does not prove that a stream's resolution, profile or preset will work.
#[allow(unsafe_code)]
pub fn query(backend: HardwareBackend, codec: VideoCodec) -> ProbeResult {
	query_on_adapter(backend, codec, None)
}

#[allow(unsafe_code)]
pub fn query_on_adapter(
	backend: HardwareBackend,
	codec: VideoCodec,
	adapter: Option<model::VideoAdapter>,
) -> ProbeResult {
	let backend = match backend {
		HardwareBackend::Nvenc => 1,
		HardwareBackend::VideoToolbox => 2,
		HardwareBackend::Amf => 3,
		HardwareBackend::Qsv => 4,
	};
	// The bridge owns all driver resources. Only fixed enum values cross the ABI.
	query_result(match adapter {
		Some(adapter) => crate::video_gpu::query_on_adapter(backend, codec.index() as i32, adapter),
		None => unsafe { serein_video_query(backend, codec.index() as i32) },
	})
}

fn query_result(result: i32) -> ProbeResult {
	match result {
		1 => ProbeResult::Available,
		0 => ProbeResult::Unavailable,
		_ => ProbeResult::Failed,
	}
}

pub fn backends() -> &'static [HardwareBackend] {
	&[
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		HardwareBackend::Nvenc,
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		HardwareBackend::Amf,
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		HardwareBackend::Qsv,
		#[cfg(target_os = "macos")]
		HardwareBackend::VideoToolbox,
	]
}

/// Test one exact backend/codec at both camera and screen-share settings.
/// Available means a real keyframe with inline parameter sets was produced;
/// discovering an encoder name or merely opening a device is insufficient.
pub fn probe(backend: HardwareBackend, codec: VideoCodec) -> HardwareSupport {
	probe_on_adapter(backend, codec, None)
}

pub fn probe_on_adapter(
	backend: HardwareBackend,
	codec: VideoCodec,
	adapter: Option<model::VideoAdapter>,
) -> HardwareSupport {
	probe_with(backend, codec, |mut config, backend| {
		config.adapter = adapter;
		Encoder::hardware_only(config, backend)
	})
}

trait ProbeEncoder {
	fn encode(&mut self, picture: &[u8], first: bool) -> Result<(Vec<u8>, bool), &'static str>;
}

impl ProbeEncoder for Encoder {
	fn encode(&mut self, picture: &[u8], first: bool) -> Result<(Vec<u8>, bool), &'static str> {
		self.encode_without_fallback(picture, first)
	}
}

fn probe_with<E: ProbeEncoder>(
	backend: HardwareBackend,
	codec: VideoCodec,
	mut open: impl FnMut(Config, HardwareBackend) -> Result<E, &'static str>,
) -> HardwareSupport {
	let camera = Config {
		width: 640,
		height: 480,
		fps: 15,
		bit_rate: 600_000,
		max_bytes: 128 * 1024,
		profile: Profile::Main,
		codec,
		adapter: None,
	};
	let screen = Config {
		width: 1280,
		height: 720,
		fps: 30,
		bit_rate: 4_000_000,
		max_bytes: 2 * 1024 * 1024,
		profile: Profile::Main,
		codec,
		adapter: None,
	};
	// Each helper drops its encoder before the next native context is opened.
	let camera = probe_profile(camera, backend, &mut open);
	let screen = probe_profile(screen, backend, &mut open);
	HardwareSupport { camera, screen }
}

fn probe_profile<E: ProbeEncoder>(
	config: Config,
	backend: HardwareBackend,
	open: &mut impl FnMut(Config, HardwareBackend) -> Result<E, &'static str>,
) -> ProbeResult {
	let Ok(mut encoder) = open(config, backend) else {
		return ProbeResult::Unavailable;
	};
	let luma = config.width as usize * config.height as usize;
	let mut picture = vec![128; luma * 3 / 2];
	picture[..luma].fill(16);
	for index in 0..MAX_PROBE_FRAMES {
		let Ok((data, keyframe)) = encoder.encode(&picture, index == 0) else {
			return ProbeResult::Unavailable;
		};
		if data.is_empty() {
			continue;
		}
		return if data.len() <= config.max_bytes
			&& keyframe
			&& crate::video::validate_source_for_codec(&data, config.codec).is_ok()
			&& crate::video::is_keyframe_for_codec(&data, config.codec)
			&& crate::video::has_parameter_sets_for_codec(&data, config.codec)
		{
			ProbeResult::Available
		} else {
			ProbeResult::Unavailable
		};
	}
	ProbeResult::Unavailable
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{cell::Cell, collections::VecDeque, rc::Rc};

	#[test]
	fn driver_errors_are_inconclusive_instead_of_unsupported() {
		assert_eq!(query_result(1), ProbeResult::Available);
		assert_eq!(query_result(0), ProbeResult::Unavailable);
		for result in [-1, -2, 2, i32::MAX] {
			assert_eq!(query_result(result), ProbeResult::Failed);
		}
	}

	struct Fake {
		frames: VecDeque<Result<(Vec<u8>, bool), &'static str>>,
		calls: Rc<Cell<usize>>,
		live: Rc<Cell<usize>>,
	}
	impl ProbeEncoder for Fake {
		fn encode(
			&mut self,
			picture: &[u8],
			_first: bool,
		) -> Result<(Vec<u8>, bool), &'static str> {
			assert!(matches!(picture.len(), 460_800 | 1_382_400));
			self.calls.set(self.calls.get() + 1);
			self.frames.pop_front().unwrap_or(Ok((Vec::new(), false)))
		}
	}
	impl Drop for Fake {
		fn drop(&mut self) {
			self.live.set(self.live.get() - 1);
		}
	}
	fn keyframe(codec: VideoCodec) -> Vec<u8> {
		match codec {
			VideoCodec::H264 => vec![
				0, 0, 1, 0x67, 0x80, 0, 0, 1, 0x68, 0x80, 0, 0, 1, 0x65, 0x88,
			],
			VideoCodec::H265 => [32, 33, 34, 19]
				.into_iter()
				.flat_map(|kind| [0, 0, 1, kind << 1, 1, 0x80])
				.collect(),
			VideoCodec::Av1 => vec![0x0a, 1, 0, 0x32, 1, 0x10],
		}
	}
	fn fake(
		frames: impl IntoIterator<Item = Result<(Vec<u8>, bool), &'static str>>,
		calls: &Rc<Cell<usize>>,
		live: &Rc<Cell<usize>>,
	) -> Fake {
		assert_eq!(live.get(), 0, "native contexts must never overlap");
		live.set(1);
		Fake {
			frames: frames.into_iter().collect(),
			calls: Rc::clone(calls),
			live: Rc::clone(live),
		}
	}

	#[test]
	fn exact_backend_and_codec_produce_independent_profile_results_without_fallback() {
		for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1] {
			let calls = Rc::new(Cell::new(0));
			let live = Rc::new(Cell::new(0));
			let mut requests = Vec::new();
			let support = probe_with(HardwareBackend::Amf, codec, |config, backend| {
				requests.push((config, backend));
				assert!(config.codec == codec && backend == HardwareBackend::Amf);
				let frames = if config.width == 640 {
					vec![Ok((Vec::new(), false)), Ok((keyframe(codec), true))]
				} else {
					vec![Err("GPU rejected the screen profile")]
				};
				Ok(fake(frames, &calls, &live))
			});
			assert_eq!(support.camera, ProbeResult::Available);
			assert_eq!(support.screen, ProbeResult::Unavailable);
			assert_eq!(requests.len(), 2);
			assert_eq!(
				(requests[0].0.width, requests[0].0.height, requests[0].0.fps),
				(640, 480, 15)
			);
			assert_eq!(
				(requests[1].0.width, requests[1].0.height, requests[1].0.fps),
				(1280, 720, 30)
			);
			assert_eq!(
				(requests[0].0.bit_rate, requests[0].0.max_bytes),
				(600_000, 128 * 1024)
			);
			assert_eq!(
				(requests[1].0.bit_rate, requests[1].0.max_bytes),
				(4_000_000, 2 * 1024 * 1024)
			);
			assert_eq!(calls.get(), 3);
			assert_eq!(live.get(), 0);
		}
	}

	#[test]
	fn successful_open_without_output_is_bounded_and_released() {
		let calls = Rc::new(Cell::new(0));
		let live = Rc::new(Cell::new(0));
		let support = probe_with(HardwareBackend::Nvenc, VideoCodec::H264, |_, _| {
			Ok(fake([], &calls, &live))
		});
		assert_eq!(support.camera, ProbeResult::Unavailable);
		assert_eq!(support.screen, ProbeResult::Unavailable);
		assert_eq!(calls.get(), 2 * MAX_PROBE_FRAMES);
		assert_eq!(live.get(), 0);
	}

	#[test]
	fn failed_camera_open_does_not_prevent_screen_probe() {
		let calls = Rc::new(Cell::new(0));
		let live = Rc::new(Cell::new(0));
		let mut attempts = 0;
		let support = probe_with(HardwareBackend::Qsv, VideoCodec::H264, |config, _| {
			attempts += 1;
			if config.width == 640 {
				Err("camera profile unavailable")
			} else {
				Ok(fake(
					[Ok((keyframe(VideoCodec::H264), true))],
					&calls,
					&live,
				))
			}
		});
		assert_eq!(attempts, 2);
		assert_eq!(support.camera, ProbeResult::Unavailable);
		assert_eq!(support.screen, ProbeResult::Available);
		assert_eq!(calls.get(), 1);
		assert_eq!(live.get(), 0);
	}

	#[test]
	fn reported_keyframe_requires_valid_codec_and_parameter_sets() {
		let calls = Rc::new(Cell::new(0));
		let live = Rc::new(Cell::new(0));
		for packet in [
			(vec![0, 0, 1, 0x65, 0x88], true),
			(keyframe(VideoCodec::H264), false),
			(keyframe(VideoCodec::H265), true),
			(vec![7, 8, 9], true),
			(vec![0; 2 * 1024 * 1024 + 1], true),
		] {
			let support = probe_with(HardwareBackend::Nvenc, VideoCodec::H264, |_, _| {
				Ok(fake([Ok(packet.clone())], &calls, &live))
			});
			assert_eq!(support.camera, ProbeResult::Unavailable);
			assert_eq!(support.screen, ProbeResult::Unavailable);
			assert_eq!(live.get(), 0);
		}
	}
}
