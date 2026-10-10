//! Single-worker FFmpeg H.264, H.265 and AV1 encoding for camera and screen sharing.
//! Hardware backends preserve the selected codec; OpenH264 is the H.264 fallback.
#![allow(unsafe_code)] // Small, checked ABI to the owned libavcodec context in the C shim.

use model::voice_settings::{HardwareBackend, VideoCodec, VideoResolution};
use std::{collections::VecDeque, ffi::c_void, marker::PhantomData, ptr::NonNull, rc::Rc};

pub(crate) const MAX_PENDING_PICTURES: usize = 48;

pub(crate) struct EncodedPacket {
	pub data: Vec<u8>,
	pub keyframe: bool,
	pub timestamp: u32,
	pub epoch: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Config {
	pub width: u32,
	pub height: u32,
	pub fps: u32,
	pub bit_rate: u32,
	pub max_bytes: usize,
	pub profile: Profile,
	pub codec: VideoCodec,
	pub adapter: Option<model::VideoAdapter>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
	Baseline,
	Main,
}

impl Config {
	pub(crate) fn picture_bytes(self) -> Result<usize, &'static str> {
		if self.width == 0
			|| self.height == 0
			|| self.width > VideoResolution::MAX_WIDTH
			|| self.height > VideoResolution::MAX_HEIGHT
			|| !self.width.is_multiple_of(2)
			|| !self.height.is_multiple_of(2)
			|| !(1..=60).contains(&self.fps)
			|| !(1_000..=50_000_000).contains(&self.bit_rate)
			|| self.max_bytes == 0
			|| self.max_bytes > 2 * 1024 * 1024
		{
			return Err("Invalid FFmpeg video encoder settings");
		}
		Ok(self.width as usize * self.height as usize * 3 / 2)
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum Backend {
	Software = 0,
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	Nvenc = 1,
	#[cfg(target_os = "macos")]
	VideoToolbox = 2,
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	Amf = 3,
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	Qsv = 4,
}

const BACKENDS: &[Backend] = &[
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	Backend::Nvenc,
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	Backend::Amf,
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	Backend::Qsv,
	#[cfg(target_os = "macos")]
	Backend::VideoToolbox,
	Backend::Software,
];

/// Capability checks select exactly one hardware encoder, including on hybrid
/// systems. A software or different GPU fallback would misreport support.
fn probe_backend(backend: HardwareBackend, _codec: VideoCodec) -> Option<Backend> {
	match backend {
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		HardwareBackend::Nvenc => Some(Backend::Nvenc),
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		HardwareBackend::Amf => Some(Backend::Amf),
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		HardwareBackend::Qsv => Some(Backend::Qsv),
		#[cfg(target_os = "macos")]
		HardwareBackend::VideoToolbox if _codec != VideoCodec::Av1 => Some(Backend::VideoToolbox),
		_ => None,
	}
}

/// Keep failed GPUs out of this stream's remaining attempts, including after
/// a backend opens successfully but rejects its first actual picture.
fn try_backends<T>(
	codec: VideoCodec,
	after: Option<Backend>,
	mut open: impl FnMut(Backend) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
	let start = after.map_or(0, |backend| {
		BACKENDS
			.iter()
			.position(|candidate| *candidate == backend)
			.map_or(BACKENDS.len(), |index| index + 1)
	});
	let mut error = "FFmpeg H.264 encoder is unavailable";
	for backend in &BACKENDS[start..] {
		if *backend == Backend::Software && codec != VideoCodec::H264 {
			continue;
		}
		#[cfg(target_os = "macos")]
		if *backend == Backend::VideoToolbox && codec == VideoCodec::Av1 {
			continue;
		}
		match open(*backend) {
			Ok(encoder) => return Ok(encoder),
			Err(failure) => error = failure,
		}
	}
	Err(match codec {
		VideoCodec::H264 => error,
		VideoCodec::H265 => "FFmpeg H.265 requires a compatible hardware encoder",
		VideoCodec::Av1 => "FFmpeg AV1 requires a compatible hardware encoder",
	})
}

/// Only the running renderer's vendor is eligible; the native bridge then
/// verifies its exact physical identity. None is reserved for standalone tests.
fn matches_adapter(backend: Backend, adapter: Option<model::VideoAdapter>) -> bool {
	if backend == Backend::Software || adapter.is_none() {
		return true;
	}
	let adapter = adapter.unwrap();
	if adapter.identity == model::VideoAdapterIdentity::Unidentified {
		return false;
	}
	match backend {
		Backend::Software => true,
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		Backend::Nvenc => adapter.vendor_id == 0x10de,
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		Backend::Amf => adapter.vendor_id == 0x1002,
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		Backend::Qsv => adapter.vendor_id == 0x8086,
		#[cfg(target_os = "macos")]
		Backend::VideoToolbox => matches!(
			adapter.identity,
			model::VideoAdapterIdentity::MetalRegistry(_)
		),
	}
}

/// Feature bits: up to two B frames (1), sixteen-picture look-ahead (2).
/// Keep H.264 without reordering for the receiver's SPS/DAVE contract.
fn feature_attempts(backend: Backend, profile: Profile, codec: VideoCodec) -> &'static [i32] {
	if profile == Profile::Baseline || backend == Backend::Software {
		return &[0];
	}
	match backend {
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		Backend::Nvenc | Backend::Qsv => {
			if codec == VideoCodec::H264 {
				&[2, 0]
			} else {
				&[3, 2, 1, 0]
			}
		}
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		Backend::Amf => &[2, 0],
		#[cfg(target_os = "macos")]
		Backend::VideoToolbox if codec == VideoCodec::H265 => &[1, 0],
		_ => &[0],
	}
}

fn take_timestamp(timeline: &mut VecDeque<(i64, u32)>, index: i64) -> Result<u32, &'static str> {
	let position = timeline
		.iter()
		.position(|(submitted, _)| *submitted == index)
		.ok_or("FFmpeg returned an unknown presentation timestamp")?;
	Ok(timeline.remove(position).unwrap().1)
}

unsafe extern "C" {
	fn serein_avc_open_on_adapter(
		width: i32,
		height: i32,
		fps: i32,
		bitrate: i32,
		baseline: i32,
		backend: i32,
		codec: i32,
		max_bytes: usize,
		adapter: *const crate::video_gpu::Adapter,
		features: i32,
	) -> *mut c_void;
	fn serein_avc_close(context: *mut c_void);
	fn serein_avc_amf_split(context: *mut c_void) -> i32;
	fn serein_avc_encode_timed(
		context: *mut c_void,
		picture: *const u8,
		length: usize,
		force: i32,
		output: *mut u8,
		capacity: usize,
		length_out: *mut usize,
		keyframe_out: *mut i32,
		presentation_index: *mut i64,
	) -> i32;
}

/// Owns one FFmpeg context, frame and packet on the capture worker's thread.
struct Native(NonNull<c_void>, PhantomData<Rc<()>>);
impl Drop for Native {
	fn drop(&mut self) {
		// SAFETY: This is the sole owner; the C shim frees all three allocations once.
		unsafe { serein_avc_close(self.0.as_ptr()) };
	}
}

pub(crate) struct Encoder {
	native: Option<Native>,
	config: Config,
	backend: Backend,
	output: Vec<u8>,
	timeline: VecDeque<(i64, u32)>,
	next_input: i64,
	epoch: u64,
	features: i32,
	amf_split: crate::diagnostics::AmfSplit,
	produced_output: bool,
}

impl Encoder {
	pub(crate) fn new(config: Config) -> Result<Self, &'static str> {
		config.picture_bytes()?;
		try_backends(config.codec, None, |backend| Self::open(config, backend))
	}

	pub(crate) fn hardware_only(
		config: Config,
		backend: HardwareBackend,
	) -> Result<Self, &'static str> {
		let backend = probe_backend(backend, config.codec)
			.ok_or("FFmpeg hardware backend is unavailable on this platform")?;
		Self::open(config, backend)
	}

	/// Submit probe pictures through the real validator without retrying another
	/// encoder or reopening an AMF session to handle a requested keyframe.
	pub(crate) fn encode_without_fallback(
		&mut self,
		picture: &[u8],
		force: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		if picture.len() != self.config.picture_bytes()? {
			return Err("Invalid FFmpeg video encoder picture");
		}
		self.encode_native(picture, force)
	}

	#[cfg(test)]
	pub(crate) fn software(config: Config) -> Result<Self, &'static str> {
		Self::open(config, Backend::Software)
	}

	/// Restart at the active backend when bitrate changes. Earlier failed GPUs
	/// stay excluded; a software fallback stays software until the stream stops.
	pub(crate) fn reconfigure(&mut self, config: Config) -> Result<(), &'static str> {
		config.picture_bytes()?;
		if config.codec != self.config.codec {
			return Err("Restart video to change its codec");
		}
		if config.adapter != self.config.adapter {
			return Err("Restart video to change its rendering GPU");
		}
		let epoch = self
			.epoch
			.checked_add(1)
			.ok_or("FFmpeg encoder epoch exhausted")?;
		let active = self.backend;
		let features = self.features;
		self.native = None;
		self.backend = Backend::Software;
		self.config = config;
		*self = Self::open_at_features(config, active, features).or_else(|_| {
			try_backends(config.codec, Some(active), |backend| {
				Self::open(config, backend)
			})
		})?;
		self.epoch = epoch;
		Ok(())
	}

	pub(crate) fn restart(&mut self) -> Result<(), &'static str> {
		self.reconfigure(self.config)
	}

	pub(crate) fn epoch(&self) -> u64 {
		self.epoch
	}

	pub(crate) fn pending(&self) -> bool {
		!self.timeline.is_empty()
	}

	fn open(config: Config, backend: Backend) -> Result<Self, &'static str> {
		Self::open_after_features(config, backend, None)
	}

	fn open_at_features(
		config: Config,
		backend: Backend,
		features: i32,
	) -> Result<Self, &'static str> {
		let attempts = feature_attempts(backend, config.profile, config.codec);
		let previous = attempts
			.iter()
			.position(|value| *value == features)
			.and_then(|index| index.checked_sub(1))
			.map(|index| attempts[index]);
		Self::open_after_features(config, backend, previous)
	}

	fn open_after_features(
		config: Config,
		backend: Backend,
		after: Option<i32>,
	) -> Result<Self, &'static str> {
		config.picture_bytes()?;
		if !matches_adapter(backend, config.adapter) {
			return Err("This encoder does not belong to the selected rendering GPU");
		}
		if backend == Backend::Software && config.codec != VideoCodec::H264 {
			return Err("FFmpeg H.265/AV1 software encoding is not bundled");
		}
		// SAFETY: All scalar bounds are checked above; the returned allocation is uniquely
		// owned here and is never sent across workers. A failed open returns null.
		let adapter = config.adapter.map(crate::video_gpu::Adapter::from);
		let mut pointer = std::ptr::null_mut();
		let mut selected_features = 0;
		let attempts = feature_attempts(backend, config.profile, config.codec);
		let start = after
			.and_then(|active| attempts.iter().position(|features| *features == active))
			.map_or(0, |index| index + 1);
		for features in &attempts[start..] {
			pointer = unsafe {
				serein_avc_open_on_adapter(
					config.width as i32,
					config.height as i32,
					config.fps as i32,
					config.bit_rate as i32,
					i32::from(config.profile == Profile::Baseline),
					backend as i32,
					match config.codec {
						VideoCodec::H264 => 0,
						VideoCodec::H265 => 1,
						VideoCodec::Av1 => 2,
					},
					config.max_bytes,
					adapter.as_ref().map_or(std::ptr::null(), |value| value),
					*features,
				)
			};
			if !pointer.is_null() {
				selected_features = *features;
				break;
			}
		}
		let pointer = NonNull::new(pointer).ok_or(match config.codec {
			VideoCodec::H264 => "FFmpeg H.264 encoder is unavailable",
			VideoCodec::H265 => "FFmpeg H.265 hardware encoder is unavailable",
			VideoCodec::Av1 => "FFmpeg AV1 hardware encoder is unavailable",
		})?;
		// SAFETY: The successfully opened context is live and uniquely owned. The
		// getter reads fixed metadata, not GPU state, and makes no driver calls.
		let amf_split = unsafe { serein_avc_amf_split(pointer.as_ptr()) };
		Ok(Self {
			native: Some(Native(pointer, PhantomData)),
			config,
			backend,
			output: vec![0; config.max_bytes],
			timeline: VecDeque::with_capacity(MAX_PENDING_PICTURES),
			next_input: 0,
			epoch: 0,
			features: selected_features,
			amf_split: crate::diagnostics::AmfSplit::from_native(amf_split),
			produced_output: false,
		})
	}

	pub(crate) fn hardware(&self) -> bool {
		self.backend != Backend::Software
	}

	pub(crate) fn amf_split(&self) -> crate::diagnostics::AmfSplit {
		if self.native.is_some() {
			self.amf_split
		} else {
			crate::diagnostics::AmfSplit::Off
		}
	}
	#[cfg(target_os = "linux")]
	pub(crate) fn label(&self) -> &'static str {
		if self.config.codec != VideoCodec::H264 {
			return match (self.config.codec, self.backend) {
				(VideoCodec::H265, Backend::Nvenc) => "H.265 · FFmpeg NVENC hardware encoding",
				(VideoCodec::H265, Backend::Amf) => "H.265 · FFmpeg AMD AMF hardware encoding",
				(VideoCodec::H265, Backend::Qsv) => "H.265 · FFmpeg Intel QSV hardware encoding",
				(VideoCodec::Av1, Backend::Nvenc) => "AV1 · FFmpeg NVENC hardware encoding",
				(VideoCodec::Av1, Backend::Amf) => "AV1 · FFmpeg AMD AMF hardware encoding",
				(VideoCodec::Av1, Backend::Qsv) => "AV1 · FFmpeg Intel QSV hardware encoding",
				_ => "FFmpeg encoder unavailable",
			};
		}
		match self.backend {
			Backend::Software => "H.264 · FFmpeg software encoding",
			#[cfg(any(target_os = "linux", target_os = "windows"))]
			Backend::Nvenc => "H.264 · FFmpeg NVENC hardware encoding",
			#[cfg(any(target_os = "linux", target_os = "windows"))]
			Backend::Amf => "H.264 · FFmpeg AMD AMF hardware encoding",
			#[cfg(any(target_os = "linux", target_os = "windows"))]
			Backend::Qsv => "H.264 · FFmpeg Intel QSV hardware encoding",
			#[cfg(target_os = "macos")]
			Backend::VideoToolbox => "H.264 · FFmpeg VideoToolbox hardware encoding",
		}
	}

	/// Preserve submission order for decoding, but recover the original RTP
	/// presentation timestamp by packet PTS when hardware reorders pictures.
	pub(crate) fn encode_at(
		&mut self,
		picture: &[u8],
		force: bool,
		timestamp: u32,
	) -> Result<EncodedPacket, &'static str> {
		if picture.len() != self.config.picture_bytes()? {
			return Err("Invalid FFmpeg video encoder picture");
		}
		// FFmpeg 7.1 AMF does not forward forced picture types. Restart only
		// an established Main stream, rather than on every look-ahead input.
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		if force
			&& self.produced_output
			&& self.backend == Backend::Amf
			&& self.config.profile == Profile::Main
		{
			self.restart()?;
		}
		let mut force = force;
		loop {
			match self.submit(picture, force, timestamp) {
				Ok(frame) => return Ok(frame),
				Err(_) if self.hardware() => {
					let failed = self.backend;
					let features = self.features;
					let epoch = self
						.epoch
						.checked_add(1)
						.ok_or("FFmpeg encoder epoch exhausted")?;
					self.native = None;
					self.timeline.clear();
					let replacement =
						Self::open_after_features(self.config, failed, Some(features)).or_else(
							|_| {
								try_backends(self.config.codec, Some(failed), |backend| {
									Self::open(self.config, backend)
								})
							},
						)?;
					*self = replacement;
					self.epoch = epoch;
					force = true;
				}
				Err(error) => return Err(error),
			}
		}
	}

	#[cfg(test)]
	pub(crate) fn encode(
		&mut self,
		picture: &[u8],
		force: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let timestamp = ((self.next_input as u64 * 90_000) / u64::from(self.config.fps)) as u32;
		let frame = self.encode_at(picture, force, timestamp)?;
		Ok((frame.data, frame.keyframe))
	}

	fn encode_native(
		&mut self,
		picture: &[u8],
		force: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let frame = self.submit(picture, force, 0)?;
		Ok((frame.data, frame.keyframe))
	}

	fn submit(
		&mut self,
		picture: &[u8],
		force: bool,
		timestamp: u32,
	) -> Result<EncodedPacket, &'static str> {
		let native = self.native.as_ref().ok_or("FFmpeg video encoder stopped")?;
		if self.timeline.len() >= MAX_PENDING_PICTURES || self.next_input == i64::MAX {
			return Err("FFmpeg exceeded its pending picture limit");
		}
		let mut length = 0usize;
		let mut keyframe = 0i32;
		let mut presentation_index = -1i64;
		// SAFETY: Input/output slices have their checked lengths; initialized
		// out-parameters live through the synchronous call. No pointer is retained.
		let result = unsafe {
			serein_avc_encode_timed(
				native.0.as_ptr(),
				picture.as_ptr(),
				picture.len(),
				i32::from(force),
				self.output.as_mut_ptr(),
				self.output.len(),
				&mut length,
				&mut keyframe,
				&mut presentation_index,
			)
		};
		if result < 0 {
			return Err("FFmpeg video encoding failed or exceeded its frame limit");
		}
		self.timeline.push_back((self.next_input, timestamp));
		self.next_input += 1;
		if result == 0 {
			return Ok(EncodedPacket {
				data: Vec::new(),
				keyframe: false,
				timestamp,
				epoch: self.epoch,
			});
		}
		if result != 1 || length == 0 || length > self.config.max_bytes {
			return Err("FFmpeg video encoding failed or exceeded its frame limit");
		}
		let timestamp = take_timestamp(&mut self.timeline, presentation_index)?;
		let data = &self.output[..length];
		crate::video::validate_source_for_codec(data, self.config.codec)
			.map_err(|_| "FFmpeg returned an invalid video bitstream")?;
		let keyframe =
			keyframe != 0 && crate::video::is_keyframe_for_codec(data, self.config.codec);
		if (self.config.profile == Profile::Baseline || !self.produced_output)
			&& (!keyframe || !crate::video::has_parameter_sets_for_codec(data, self.config.codec))
		{
			return Err("FFmpeg initial or camera frame was not independently decodable");
		}
		self.produced_output = true;
		Ok(EncodedPacket {
			data: data.to_vec(),
			keyframe,
			timestamp,
			epoch: self.epoch,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	const CAMERA: Config = Config {
		width: 640,
		height: 480,
		fps: 15,
		bit_rate: 600_000,
		max_bytes: 128 * 1024,
		profile: Profile::Baseline,
		codec: VideoCodec::H264,
		adapter: None,
	};
	#[test]
	fn rendering_adapter_excludes_other_vendors_and_unknown_hardware() {
		for backend in BACKENDS {
			assert!(matches_adapter(*backend, None));
			assert_eq!(
				matches_adapter(*backend, Some(model::VideoAdapter::default())),
				*backend == Backend::Software
			);
		}
		#[cfg(any(target_os = "linux", target_os = "windows"))]
		for (vendor, selected) in [
			(0x10de, Backend::Nvenc),
			(0x1002, Backend::Amf),
			(0x8086, Backend::Qsv),
		] {
			let adapter = model::VideoAdapter {
				vendor_id: vendor,
				device_id: 1,
				identity: model::VideoAdapterIdentity::Pci {
					domain: 0,
					bus: 1,
					device: 0,
					function: 0,
				},
			};
			for backend in BACKENDS {
				assert_eq!(
					matches_adapter(*backend, Some(adapter)),
					*backend == selected || *backend == Backend::Software
				);
			}
		}
	}

	#[test]
	fn h264_never_requests_b_frames_and_baseline_retains_independent_frames() {
		for backend in BACKENDS {
			assert_eq!(
				feature_attempts(*backend, Profile::Baseline, VideoCodec::H264),
				&[0]
			);
			assert!(
				feature_attempts(*backend, Profile::Main, VideoCodec::H264)
					.iter()
					.all(|mode| mode & 1 == 0)
			);
		}
	}

	#[test]
	fn reordered_packets_keep_wrapped_rtp_timestamps_without_reordering_decode_output() {
		let mut timeline = VecDeque::from([(0, u32::MAX - 100), (1, 2899), (2, 5899), (3, 8899)]);
		let mut timestamps = Vec::new();
		for index in [0, 3, 1, 2] {
			timestamps.push(take_timestamp(&mut timeline, index).unwrap());
		}
		assert_eq!(timestamps, [u32::MAX - 100, 8899, 2899, 5899]);
		assert!(timeline.is_empty());
		assert!(take_timestamp(&mut timeline, 2).is_err());
	}

	#[test]
	fn software_timestamps_and_recovery_epochs_follow_the_submitted_picture() {
		let mut encoder = Encoder::software(Config {
			profile: Profile::Main,
			..CAMERA
		})
		.unwrap();
		let picture = vec![128; CAMERA.picture_bytes().unwrap()];
		for timestamp in [u32::MAX - 2999, 0, 3000] {
			let frame = encoder.encode_at(&picture, false, timestamp).unwrap();
			assert_eq!(frame.timestamp, timestamp);
			assert_eq!(frame.epoch, 0);
			assert!(!frame.data.is_empty());
			assert!(!encoder.pending());
		}
		encoder.restart().unwrap();
		let frame = encoder.encode_at(&picture, false, 6000).unwrap();
		assert_eq!((frame.timestamp, frame.epoch), (6000, 1));
		assert!(
			frame.keyframe
				&& crate::video::has_parameter_sets_for_codec(&frame.data, VideoCodec::H264)
		);
	}

	#[test]
	fn unidentified_renderer_keeps_h264_software_fallback_available() {
		let mut encoder = Encoder::new(Config {
			adapter: Some(model::VideoAdapter::default()),
			..CAMERA
		})
		.unwrap();
		assert!(!encoder.hardware());
		let frame = encoder
			.encode_at(&vec![128; CAMERA.picture_bytes().unwrap()], false, 1234)
			.unwrap();
		assert_eq!(frame.timestamp, 1234);
		assert!(frame.keyframe && !frame.data.is_empty());
	}
	#[test]
	fn capability_backends_never_select_software_or_another_gpu() {
		for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1] {
			for backend in HardwareBackend::ALL {
				let selected = probe_backend(backend, codec);
				assert!(selected != Some(Backend::Software));
				#[cfg(any(target_os = "linux", target_os = "windows"))]
				assert!(
					selected
						== match backend {
							HardwareBackend::Nvenc => Some(Backend::Nvenc),
							HardwareBackend::Amf => Some(Backend::Amf),
							HardwareBackend::Qsv => Some(Backend::Qsv),
							HardwareBackend::VideoToolbox => None,
						}
				);
				#[cfg(target_os = "macos")]
				assert!(
					selected
						== if backend == HardwareBackend::VideoToolbox && codec != VideoCodec::Av1 {
							Some(Backend::VideoToolbox)
						} else {
							None
						}
				);
			}
		}
	}
	#[test]
	fn failed_backends_advance_once_without_revisiting_a_gpu() {
		let mut attempts = Vec::new();
		let selected = try_backends(VideoCodec::H264, None, |backend| {
			attempts.push(backend);
			if backend == Backend::Software {
				Ok(backend)
			} else {
				Err("unavailable GPU")
			}
		})
		.unwrap();
		assert!(selected == Backend::Software);
		assert!(attempts == BACKENDS);
		for (index, failed) in BACKENDS.iter().enumerate() {
			attempts.clear();
			let result: Result<(), _> = try_backends(VideoCodec::H264, Some(*failed), |backend| {
				attempts.push(backend);
				Err("failed to open")
			});
			assert!(result.is_err());
			assert!(attempts == BACKENDS[index + 1..]);
		}
	}
	#[cfg(any(target_os = "linux", target_os = "windows"))]
	#[test]
	fn hybrid_gpu_selection_continues_to_intel_after_amd_frame_failure() {
		let mut attempts = Vec::new();
		let amd = try_backends(VideoCodec::H264, None, |backend| {
			attempts.push(backend);
			if backend == Backend::Amf {
				Ok(backend)
			} else {
				Err("GPU unavailable")
			}
		})
		.unwrap();
		assert!(amd == Backend::Amf && attempts == [Backend::Nvenc, Backend::Amf]);
		attempts.clear();
		let intel = try_backends(VideoCodec::H264, Some(amd), |backend| {
			attempts.push(backend);
			Ok(backend)
		})
		.unwrap();
		assert!(intel == Backend::Qsv && attempts == [Backend::Qsv]);
		let software = try_backends(VideoCodec::H264, Some(intel), Ok).unwrap();
		assert!(software == Backend::Software);
		assert!(try_backends(VideoCodec::H264, Some(software), Ok).is_err());
	}
	#[test]
	fn newer_codecs_never_select_h264_software_fallback() {
		for codec in [VideoCodec::H265, VideoCodec::Av1] {
			let mut attempts = Vec::new();
			let result: Result<(), _> = try_backends(codec, None, |backend| {
				attempts.push(backend);
				Err("hardware unavailable")
			});
			assert!(result.is_err());
			assert!(!attempts.contains(&Backend::Software));
			assert!(Encoder::software(Config { codec, ..CAMERA }).is_err());
			#[cfg(target_os = "macos")]
			if codec == VideoCodec::Av1 {
				assert!(attempts.is_empty());
			}
		}
	}
	#[test]
	fn malformed_picture_keeps_existing_native_context_and_backend() {
		let mut encoder = Encoder::software(CAMERA).unwrap();
		let pointer = encoder.native.as_ref().unwrap().0;
		// Simulate each selected GPU using a real software allocation. Rejected
		// input must never reach native encoding or trigger device selection.
		for backend in BACKENDS {
			encoder.backend = *backend;
			assert!(encoder.encode(&[0; 3], true).is_err());
			assert!(encoder.backend == *backend);
			assert!(encoder.native.as_ref().unwrap().0 == pointer);
			assert!(!encoder.produced_output && encoder.timeline.is_empty());
		}
		encoder.backend = Backend::Software;
		let (packet, keyframe) = encoder
			.encode(&vec![128; CAMERA.picture_bytes().unwrap()], true)
			.unwrap();
		assert!(keyframe && !packet.is_empty());
	}
	#[test]
	fn bitrate_restart_keeps_software_fallback_and_starts_with_an_idr() {
		let mut encoder = Encoder::software(CAMERA).unwrap();
		let picture = vec![128; CAMERA.picture_bytes().unwrap()];
		assert!(encoder.encode(&picture, true).unwrap().1);
		encoder
			.reconfigure(Config {
				bit_rate: 450_000,
				..CAMERA
			})
			.unwrap();
		assert!(encoder.backend == Backend::Software && !encoder.produced_output);
		let (packet, keyframe) = encoder.encode(&picture, false).unwrap();
		assert!(keyframe && crate::video_receive::has_parameter_sets(&packet));
	}
	#[test]
	fn ffmpeg_software_camera_is_bounded_and_independently_decodable() {
		use openh264::formats::YUVSource;
		let mut encoder = Encoder::open(CAMERA, Backend::Software).unwrap();
		for length in [0, 640 * 480 * 3 / 2 - 1, 640 * 480 * 3 / 2 + 1] {
			assert!(encoder.encode(&vec![0; length], true).is_err());
		}
		for luma in [16, 126, 235] {
			let mut picture = vec![128; 640 * 480 * 3 / 2];
			picture[..640 * 480].fill(luma);
			let (data, keyframe) = encoder.encode(&picture, true).unwrap();
			assert!(keyframe && data.len() <= CAMERA.max_bytes);
			let mut decoder = openh264::decoder::Decoder::new().unwrap();
			assert_eq!(
				decoder.decode(&data).unwrap().unwrap().dimensions(),
				(640, 480)
			);
		}
	}
	#[test]
	fn ffmpeg_rejects_unbounded_configuration_before_native_allocation() {
		for config in [
			Config { width: 0, ..CAMERA },
			Config {
				width: VideoResolution::MAX_WIDTH + 2,
				..CAMERA
			},
			Config {
				height: 481,
				..CAMERA
			},
			Config { fps: 61, ..CAMERA },
			Config {
				max_bytes: 2 * 1024 * 1024 + 1,
				..CAMERA
			},
		] {
			assert!(Encoder::new(config).is_err());
		}
	}

	#[test]
	fn output_geometry_accepts_presets_through_8k_with_a_fixed_picture_budget() {
		for resolution in VideoResolution::ALL {
			let (width, height) = resolution.dimensions();
			let config = Config {
				width,
				height,
				..CAMERA
			};
			let bytes = config.picture_bytes().unwrap();
			assert_eq!(bytes, width as usize * height as usize * 3 / 2);
			assert!(bytes <= 7680 * 4320 * 3 / 2);
		}
		for config in [
			Config {
				width: 7682,
				height: 4320,
				..CAMERA
			},
			Config {
				width: 7680,
				height: 4322,
				..CAMERA
			},
			Config {
				width: u32::MAX,
				height: u32::MAX,
				..CAMERA
			},
			Config {
				bit_rate: 50_000_001,
				..CAMERA
			},
		] {
			assert!(config.picture_bytes().is_err());
		}
	}

	#[test]
	fn ffmpeg_software_output_above_1080p_encodes_and_decodes_selected_geometry() {
		use openh264::formats::YUVSource;
		let config = Config {
			width: 2560,
			height: 1440,
			bit_rate: 12_000_000,
			max_bytes: 2 * 1024 * 1024,
			profile: Profile::Main,
			..CAMERA
		};
		let mut encoder = Encoder::software(config).unwrap();
		let picture = vec![128; config.picture_bytes().unwrap()];
		let (packet, keyframe) = encoder.encode(&picture, true).unwrap();
		assert!(keyframe && !packet.is_empty() && packet.len() <= config.max_bytes);
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		assert_eq!(
			decoder.decode(&packet).unwrap().unwrap().dimensions(),
			(2560, 1440)
		);
	}

	#[test]
	fn ffmpeg_screen_delta_frames_and_forced_idr_decode() {
		use openh264::formats::YUVSource;
		let config = Config {
			profile: Profile::Main,
			..CAMERA
		};
		let mut encoder = Encoder::software(config).unwrap();
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		let mut picture = vec![128; config.picture_bytes().unwrap()];
		for index in 0..8 {
			picture[..640 * 480].fill(16 + index * 20);
			let forced = index == 0 || index == 5;
			let (data, keyframe) = encoder.encode(&picture, forced).unwrap();
			assert_eq!(keyframe, forced);
			assert_eq!(
				decoder.decode(&data).unwrap().unwrap().dimensions(),
				(640, 480)
			);
			if forced {
				let mut fresh = openh264::decoder::Decoder::new().unwrap();
				assert_eq!(
					fresh.decode(&data).unwrap().unwrap().dimensions(),
					(640, 480)
				);
			}
		}
		let mut capped = Encoder::software(Config {
			max_bytes: 1,
			..config
		})
		.unwrap();
		assert!(capped.encode(&picture, true).is_err());
	}
}
