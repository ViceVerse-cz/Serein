//! Worker-owned video encoding selection. Stable retains the native H.264 encoders;
//! Experimental uses FFmpeg and the explicitly selected codec.
use crate::video_encode::{Config, EncodedPacket, Profile};
use model::voice_settings::{VideoBackend, VideoCodec};
use openh264::{
	OpenH264API,
	encoder::{
		BitRate, Complexity, EncoderConfig, FrameRate, FrameType, IntraFramePeriod,
		Profile as H264Profile, RateControlMode, UsageType,
	},
	formats::YUVSlices,
};
use std::collections::VecDeque;

const MAX_STABLE_PENDING_PICTURES: usize = 4;

#[cfg(target_os = "linux")]
#[path = "video_encode_linux.rs"]
mod native;
#[cfg(target_os = "windows")]
#[path = "video_encode_windows.rs"]
mod native;
#[cfg(target_os = "macos")]
#[path = "video_encode_macos.rs"]
mod native;

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceFormat {
	Bgra,
	#[cfg_attr(not(test), allow(dead_code))]
	Rgb,
}
#[cfg(target_os = "macos")]
impl SourceFormat {
	fn bytes_per_pixel(self) -> usize {
		match self {
			Self::Bgra => 4,
			Self::Rgb => 3,
		}
	}
}

enum Implementation {
	Stable(Box<Stable>),
	Experimental(crate::video_encode::Encoder),
}

pub(crate) struct Encoder {
	implementation: Implementation,
}

impl Encoder {
	#[cfg(test)]
	pub(crate) fn software(config: Config) -> Result<Self, &'static str> {
		picture_bytes(config)?;
		if config.codec != VideoCodec::H264 {
			return Err("Stable video encoding supports H.264 only");
		}
		Ok(Self {
			implementation: Implementation::Stable(Box::new(Stable {
				hardware: None,
				software: Some(software(config)?),
				config,
				#[cfg(target_os = "macos")]
				bgra: Vec::new(),
				timeline: VecDeque::with_capacity(MAX_STABLE_PENDING_PICTURES),
				epoch: 0,
				produced_output: false,
			})),
		})
	}

	pub(crate) fn new(config: Config, backend: VideoBackend) -> Result<Self, &'static str> {
		picture_bytes(config)?;
		let implementation = match backend {
			VideoBackend::Stable => {
				if config.codec != VideoCodec::H264 {
					return Err("Stable video encoding supports H.264 only");
				}
				Implementation::Stable(Box::new(Stable::new(config)?))
			}
			VideoBackend::Experimental => {
				Implementation::Experimental(crate::video_encode::Encoder::new(config)?)
			}
		};
		Ok(Self { implementation })
	}

	#[cfg(test)]
	pub(crate) fn encode(
		&mut self,
		picture: &[u8],
		force: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let packet = self.encode_at(picture, force, 0)?;
		Ok((packet.data, packet.keyframe))
	}

	pub(crate) fn encode_at(
		&mut self,
		picture: &[u8],
		force: bool,
		timestamp: u32,
	) -> Result<EncodedPacket, &'static str> {
		match &mut self.implementation {
			Implementation::Stable(encoder) => encoder.encode_at(picture, force, timestamp),
			Implementation::Experimental(encoder) => encoder.encode_at(picture, force, timestamp),
		}
	}

	pub(crate) fn pending(&self) -> bool {
		match &self.implementation {
			Implementation::Stable(encoder) => !encoder.timeline.is_empty(),
			Implementation::Experimental(encoder) => encoder.pending(),
		}
	}

	pub(crate) fn epoch(&self) -> u64 {
		match &self.implementation {
			Implementation::Stable(encoder) => encoder.epoch,
			Implementation::Experimental(encoder) => encoder.epoch(),
		}
	}

	pub(crate) fn restart(&mut self) -> Result<(), &'static str> {
		match &mut self.implementation {
			Implementation::Stable(encoder) => encoder.replace(encoder.config),
			Implementation::Experimental(encoder) => encoder.restart(),
		}
	}

	pub(crate) fn reconfigure(&mut self, config: Config) -> Result<(), &'static str> {
		picture_bytes(config)?;
		match &mut self.implementation {
			Implementation::Stable(encoder) => {
				if config.codec != VideoCodec::H264 {
					return Err("Stable video encoding supports H.264 only");
				}
				encoder.reconfigure(config)
			}
			Implementation::Experimental(encoder) => encoder.reconfigure(config),
		}
	}

	pub(crate) fn hardware(&self) -> bool {
		match &self.implementation {
			Implementation::Stable(encoder) => encoder.hardware.is_some(),
			Implementation::Experimental(encoder) => encoder.hardware(),
		}
	}

	pub(crate) fn amf_split(&self) -> crate::diagnostics::AmfSplit {
		match &self.implementation {
			Implementation::Stable(_) => crate::diagnostics::AmfSplit::Off,
			Implementation::Experimental(encoder) => encoder.amf_split(),
		}
	}

	#[cfg(target_os = "linux")]
	pub(crate) fn label(&self) -> &'static str {
		match &self.implementation {
			Implementation::Stable(encoder) => encoder
				.hardware
				.as_ref()
				.map_or("H.264 · Stable software encoding", |hardware| {
					hardware.label()
				}),
			Implementation::Experimental(encoder) => encoder.label(),
		}
	}
}

fn picture_bytes(config: Config) -> Result<usize, &'static str> {
	config.picture_bytes()
}

struct Stable {
	hardware: Option<native::Encoder>,
	software: Option<openh264::encoder::Encoder>,
	config: Config,
	#[cfg(target_os = "macos")]
	bgra: Vec<u8>,
	timeline: VecDeque<u32>,
	epoch: u64,
	produced_output: bool,
}

impl Stable {
	fn new(config: Config) -> Result<Self, &'static str> {
		#[cfg(target_os = "macos")]
		let hardware = native::Encoder::new_on_adapter(config, SourceFormat::Bgra, config.adapter).ok();
		#[cfg(not(target_os = "macos"))]
		let hardware = native::Encoder::new_on_adapter(config, config.adapter).ok();
		let software = hardware.is_none().then(|| software(config)).transpose()?;
		Ok(Self {
			hardware,
			software,
			config,
			#[cfg(target_os = "macos")]
			bgra: Vec::new(),
			timeline: VecDeque::with_capacity(MAX_STABLE_PENDING_PICTURES),
			epoch: 0,
			produced_output: false,
		})
	}

	fn reconfigure(&mut self, config: Config) -> Result<(), &'static str> {
		if config.adapter != self.config.adapter {
			return Err("Restart video to change its rendering GPU");
		}
		// The screen worker treats a rate change as a fresh IDR epoch. Native live
		// setters would retain delayed old frames and violate that shared contract.
		self.replace(config)
	}

	fn replace(&mut self, config: Config) -> Result<(), &'static str> {
		let epoch = self
			.epoch
			.checked_add(1)
			.ok_or("Stable encoder epoch exhausted")?;
		let was_hardware = self.hardware.is_some();
		// Native Drop completes its platform teardown before another session is requested.
		self.hardware = None;
		self.software = None;
		self.config = config;
		self.timeline.clear();
		self.epoch = epoch;
		self.produced_output = false;
		if was_hardware {
			#[cfg(target_os = "macos")]
			let hardware = native::Encoder::new_on_adapter(config, SourceFormat::Bgra, config.adapter).ok();
			#[cfg(not(target_os = "macos"))]
			let hardware = native::Encoder::new_on_adapter(config, config.adapter).ok();
			self.hardware = hardware;
		}
		if self.hardware.is_none() {
			self.software = Some(software(config)?);
		}
		Ok(())
	}

	fn encode_at(
		&mut self,
		picture: &[u8],
		force: bool,
		timestamp: u32,
	) -> Result<EncodedPacket, &'static str> {
		if picture.len() != picture_bytes(self.config)? {
			return Err("Invalid video encoder picture");
		}
		let force = force || self.config.profile == Profile::Baseline || !self.produced_output;
		if self.hardware.is_some() {
			if let Ok(packet) = self.encode_hardware(picture, force, timestamp) {
				return Ok(packet);
			}
			// A failed platform encoder stays excluded through every bitrate change in
			// this stream. The first software picture must reset the receiver with an IDR.
			let epoch = self
				.epoch
				.checked_add(1)
				.ok_or("Stable encoder epoch exhausted")?;
			self.hardware = None;
			self.software = None;
			self.timeline.clear();
			self.epoch = epoch;
			self.produced_output = false;
			self.software = Some(software(self.config)?);
		}
		let (width, height) = (self.config.width as usize, self.config.height as usize);
		let (y, chroma) = picture.split_at(width * height);
		let (u, v) = chroma.split_at(width * height / 4);
		let software = self
			.software
			.as_mut()
			.ok_or("Stable video encoder stopped")?;
		if force || !self.produced_output {
			software.force_intra_frame();
		}
		let encoded = software
			.encode(&YUVSlices::new(
				(y, u, v),
				(width, height),
				(width, width / 2, width / 2),
			))
			.map_err(|_| "Stable video encoding failed")?;
		let mut length = 0usize;
		for index in 0..encoded.num_layers() {
			let layer = encoded
				.layer(index)
				.ok_or("Stable encoder returned an invalid layer")?;
			for index in 0..layer.nal_count() {
				length = length
					.checked_add(
						layer
							.nal_unit(index)
							.ok_or("Stable encoder returned an invalid NAL")?
							.len(),
					)
					.filter(|length| *length <= self.config.max_bytes)
					.ok_or("Stable encoded video frame exceeds its limit")?;
			}
		}
		if length == 0 {
			return Ok(EncodedPacket {
				data: Vec::new(),
				keyframe: false,
				timestamp,
				epoch: self.epoch,
			});
		}
		let mut frame = Vec::with_capacity(length);
		encoded.write_vec(&mut frame);
		let keyframe = matches!(encoded.frame_type(), FrameType::IDR);
		validate_h264(&frame, keyframe, self.config, !self.produced_output)?;
		self.produced_output = true;
		Ok(EncodedPacket {
			data: frame,
			keyframe,
			timestamp,
			epoch: self.epoch,
		})
	}

	fn submit_timestamp(&mut self, timestamp: u32) -> Result<(), &'static str> {
		if self.timeline.len() >= MAX_STABLE_PENDING_PICTURES {
			return Err("Stable video encoder has too many pending pictures");
		}
		self.timeline.push_back(timestamp);
		Ok(())
	}

	fn encode_hardware(
		&mut self,
		picture: &[u8],
		force: bool,
		timestamp: u32,
	) -> Result<EncodedPacket, &'static str> {
		self.submit_timestamp(timestamp)?;
		let hardware = self
			.hardware
			.as_mut()
			.ok_or("Stable native encoder stopped")?;
		#[cfg(target_os = "windows")]
		let before = hardware.submitted_frames();
		#[cfg(target_os = "linux")]
		let frame = hardware.encode(picture, force)?;
		#[cfg(target_os = "windows")]
		let frame = {
			let (y, chroma) =
				picture.split_at(self.config.width as usize * self.config.height as usize);
			let (u, v) = chroma.split_at(y.len() / 4);
			hardware.encode(y, u, v, force)?
		};
		#[cfg(target_os = "macos")]
		let frame = {
			i420_to_bgra(picture, self.config, &mut self.bgra)?;
			hardware.encode(
				&self.bgra,
				(self.config.width as usize, self.config.height as usize),
				force,
			)?
		};
		#[cfg(target_os = "windows")]
		let submitted = hardware.submitted_frames() != before;
		#[cfg(not(target_os = "windows"))]
		let submitted = true;
		self.finish_hardware(frame, timestamp, submitted)
	}

	fn finish_hardware(
		&mut self,
		frame: (Vec<u8>, bool),
		timestamp: u32,
		submitted: bool,
	) -> Result<EncodedPacket, &'static str> {
		if !submitted {
			// An output-only call drained an older picture without accepting this capture.
			self.timeline.pop_back();
		}
		if frame.0.is_empty() {
			#[cfg(target_os = "macos")]
			{
				// CompleteFrames makes native macOS encoding synchronous. An
				// empty callback is a dropped input, rather than delayed output.
				self.timeline.pop_back();
			}
			if self.timeline.len() >= MAX_STABLE_PENDING_PICTURES {
				return Err("Stable native encoder produced no bounded output");
			}
			return Ok(EncodedPacket {
				data: frame.0,
				keyframe: false,
				timestamp,
				epoch: self.epoch,
			});
		}
		validate_h264(&frame.0, frame.1, self.config, !self.produced_output)?;
		let timestamp = self
			.timeline
			.pop_front()
			.ok_or("Stable encoder lost its input timestamp")?;
		self.produced_output = true;
		Ok(EncodedPacket {
			data: frame.0,
			keyframe: frame.1,
			timestamp,
			epoch: self.epoch,
		})
	}
}

fn validate_h264(
	frame: &[u8],
	keyframe: bool,
	config: Config,
	initial: bool,
) -> Result<(), &'static str> {
	if frame.is_empty() || frame.len() > config.max_bytes {
		return Err("Stable encoded video frame exceeds its limit");
	}
	crate::video::validate_source(frame).map_err(|_| "Stable encoder returned invalid H.264")?;
	if (initial || config.profile == Profile::Baseline)
		&& (!keyframe
			|| !crate::video_receive::is_keyframe(frame)
			|| !crate::video_receive::has_parameter_sets(frame))
	{
		return Err("Stable initial or camera frame is not independently decodable");
	}
	Ok(())
}

fn software(config: Config) -> Result<openh264::encoder::Encoder, &'static str> {
	let camera = config.profile == Profile::Baseline;
	let threads = if camera {
		1
	} else {
		std::thread::available_parallelism().map_or(2, |count| count.get().clamp(2, 8) as u16)
	};
	openh264::encoder::Encoder::with_api_config(
		OpenH264API::from_source(),
		EncoderConfig::new()
			.bitrate(BitRate::from_bps(config.bit_rate))
			.max_frame_rate(FrameRate::from_hz(config.fps as f32))
			.profile(if camera {
				H264Profile::Baseline
			} else {
				H264Profile::Main
			})
			.usage_type(if camera {
				UsageType::CameraVideoRealTime
			} else {
				UsageType::ScreenContentRealTime
			})
			.rate_control_mode(RateControlMode::Bitrate)
			.complexity(if cfg!(target_os = "windows") || camera {
				Complexity::Low
			} else {
				Complexity::Medium
			})
			.num_threads(threads)
			.intra_frame_period(IntraFramePeriod::from_num_frames(if camera {
				1
			} else {
				config.fps * 2
			}))
			.debug(false),
	)
	.map_err(|_| "Stable software video encoder is unavailable")
}

#[cfg(any(target_os = "windows", test))]
fn i420_to_nv12(y: &[u8], u: &[u8], v: &[u8], output: &mut [u8]) -> Result<(), &'static str> {
	if u.len() != v.len() || y.len() != u.len() * 4 || output.len() != y.len() + u.len() + v.len() {
		return Err("Invalid video encoder color planes");
	}
	output[..y.len()].copy_from_slice(y);
	for (pair, (&u, &v)) in output[y.len()..]
		.as_chunks_mut::<2>()
		.0
		.iter_mut()
		.zip(u.iter().zip(v))
	{
		pair.copy_from_slice(&[u, v]);
	}
	Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn i420_to_bgra(picture: &[u8], config: Config, output: &mut Vec<u8>) -> Result<(), &'static str> {
	if picture.len() != picture_bytes(config)? {
		return Err("Invalid video encoder color planes");
	}
	let (width, height) = (config.width as usize, config.height as usize);
	let (y, chroma) = picture.split_at(width * height);
	let (u, v) = chroma.split_at(width * height / 4);
	output.resize(width * height * 4, 0);
	for (index, pixel) in output.as_chunks_mut::<4>().0.iter_mut().enumerate() {
		let chroma = index / width / 2 * (width / 2) + index % width / 2;
		let (c, d, e) = (
			i32::from(y[index]) - 16,
			i32::from(u[chroma]) - 128,
			i32::from(v[chroma]) - 128,
		);
		let byte = |value: i32| ((value + 128) >> 8).clamp(0, 255) as u8;
		*pixel = [
			byte(298 * c + 516 * d),
			byte(298 * c - 100 * d - 208 * e),
			byte(298 * c + 409 * e),
			255,
		];
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use openh264::formats::YUVSource;

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

	fn software_only(config: Config) -> Encoder {
		Encoder::software(config).unwrap()
	}

	#[test]
	fn stable_native_timestamp_fifo_is_bounded_and_preserves_wrapping_rtp_values() {
		let picture = vec![128; picture_bytes(CAMERA).unwrap()];
		let mut source = software_only(CAMERA);
		let packet = source.encode_at(&picture, false, 0).unwrap();
		let mut target = software_only(CAMERA);
		let Implementation::Stable(stable) = &mut target.implementation else {
			unreachable!("software test constructor selects Stable")
		};
		let timestamps = [u32::MAX - 2999, 0, 3000, 6000];
		for timestamp in timestamps {
			stable.submit_timestamp(timestamp).unwrap();
		}
		assert!(stable.submit_timestamp(9000).is_err());
		assert_eq!(stable.timeline.len(), MAX_STABLE_PENDING_PICTURES);
		for timestamp in timestamps {
			let output = stable
				.finish_hardware((packet.data.clone(), packet.keyframe), 90_000, true)
				.unwrap();
			assert_eq!(output.timestamp, timestamp);
		}
		assert!(stable.timeline.is_empty());
		stable.submit_timestamp(12000).unwrap();
		assert!(
			stable
				.finish_hardware((vec![1, 2, 3], false), 15000, true)
				.is_err()
		);
		assert_eq!(stable.timeline.front(), Some(&12000));
	}

	#[test]
	fn output_only_native_call_keeps_only_submitted_timestamps() {
		let mut encoder = software_only(CAMERA);
		let picture = vec![128; picture_bytes(CAMERA).unwrap()];
		let packet = encoder.encode_at(&picture, true, 3000).unwrap();
		let Implementation::Stable(stable) = &mut encoder.implementation else {
			unreachable!()
		};
		stable.submit_timestamp(3000).unwrap();
		stable.submit_timestamp(6000).unwrap();
		stable.submit_timestamp(9000).unwrap();
		let drained = stable
			.finish_hardware((packet.data.clone(), true), 9000, false)
			.unwrap();
		assert_eq!(drained.timestamp, 3000);
		assert_eq!(stable.timeline.iter().copied().collect::<Vec<_>>(), [6000]);
		stable.submit_timestamp(12000).unwrap();
		let next = stable
			.finish_hardware((packet.data, true), 12000, true)
			.unwrap();
		assert_eq!(next.timestamp, 6000);
		assert_eq!(stable.timeline.iter().copied().collect::<Vec<_>>(), [12000]);
	}

	#[test]
	fn stable_software_restart_and_rate_replacement_start_a_new_idr_epoch() {
		let config = Config {
			profile: Profile::Main,
			..CAMERA
		};
		let picture = vec![128; picture_bytes(config).unwrap()];
		let mut encoder = software_only(config);
		let first = encoder.encode_at(&picture, false, u32::MAX - 2999).unwrap();
		assert!(first.keyframe && !encoder.pending());
		assert_eq!((first.timestamp, first.epoch), (u32::MAX - 2999, 0));
		encoder.restart().unwrap();
		let second = encoder.encode_at(&picture, false, 0).unwrap();
		assert!(second.keyframe && !encoder.hardware());
		assert_eq!((second.timestamp, second.epoch), (0, 1));
		encoder
			.reconfigure(Config {
				bit_rate: 450_000,
				..config
			})
			.unwrap();
		let third = encoder.encode_at(&picture, false, 3000).unwrap();
		assert!(third.keyframe && !encoder.hardware() && !encoder.pending());
		assert_eq!((third.timestamp, third.epoch), (3000, 2));
	}

	#[test]
	fn stable_adapter_change_and_epoch_exhaustion_preserve_the_active_encoder() {
		let picture = vec![128; picture_bytes(CAMERA).unwrap()];
		let mut encoder = software_only(CAMERA);
		assert_eq!(
			encoder.reconfigure(Config {
				adapter: Some(model::VideoAdapter::default()),
				..CAMERA
			}),
			Err("Restart video to change its rendering GPU")
		);
		assert_eq!(encoder.epoch(), 0);
		let Implementation::Stable(stable) = &mut encoder.implementation else {
			unreachable!("software test constructor selects Stable")
		};
		stable.epoch = u64::MAX;
		assert_eq!(encoder.restart(), Err("Stable encoder epoch exhausted"));
		let packet = encoder.encode_at(&picture, false, 9000).unwrap();
		assert!(packet.keyframe && !encoder.hardware());
		assert_eq!((packet.timestamp, packet.epoch), (9000, u64::MAX));
	}

	#[cfg(not(target_os = "macos"))]
	#[test]
	fn stable_native_pending_output_cannot_accumulate_unbounded_timestamps() {
		let mut encoder = software_only(CAMERA);
		let Implementation::Stable(stable) = &mut encoder.implementation else {
			unreachable!("software test constructor selects Stable")
		};
		for index in 0..MAX_STABLE_PENDING_PICTURES {
			stable.submit_timestamp(index as u32 * 3000).unwrap();
			let pending = stable.finish_hardware((Vec::new(), false), 9000, true);
			assert_eq!(pending.is_ok(), index + 1 < MAX_STABLE_PENDING_PICTURES);
		}
		assert_eq!(stable.timeline.len(), MAX_STABLE_PENDING_PICTURES);
		stable.replace(CAMERA).unwrap();
		assert!(stable.timeline.is_empty() && !stable.produced_output);
		assert_eq!(stable.epoch, 1);
	}

	#[test]
	fn stable_camera_frames_are_bounded_and_independently_decodable() {
		let mut encoder = software_only(CAMERA);
		let mut picture = vec![128; picture_bytes(CAMERA).unwrap()];
		for luma in [16, 126, 235] {
			picture[..640 * 480].fill(luma);
			let (frame, keyframe) = encoder.encode(&picture, false).unwrap();
			assert!(keyframe && frame.len() <= CAMERA.max_bytes);
			let mut decoder = openh264::decoder::Decoder::new().unwrap();
			assert_eq!(
				decoder.decode(&frame).unwrap().unwrap().dimensions(),
				(640, 480)
			);
		}
		encoder
			.reconfigure(Config {
				bit_rate: 450_000,
				..CAMERA
			})
			.unwrap();
		assert!(
			!encoder.hardware(),
			"software fallback must survive rate changes"
		);
		let (frame, keyframe) = encoder.encode(&picture, false).unwrap();
		assert!(keyframe && crate::video_receive::has_parameter_sets(&frame));
	}

	#[test]
	fn stable_screen_uses_delta_frames_and_can_restart_at_a_forced_idr() {
		let config = Config {
			profile: Profile::Main,
			..CAMERA
		};
		let mut encoder = software_only(config);
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		let mut picture = vec![128; picture_bytes(config).unwrap()];
		for index in 0..8 {
			// Preserve Stable's scene-change detection: vary one pixel, not the whole scene.
			picture[0] = 128 + index;
			let force = index == 0 || index == 5;
			let (frame, keyframe) = encoder.encode(&picture, force).unwrap();
			assert_eq!(keyframe, force);
			assert_eq!(
				decoder.decode(&frame).unwrap().unwrap().dimensions(),
				(640, 480)
			);
			if force {
				let mut fresh = openh264::decoder::Decoder::new().unwrap();
				assert_eq!(
					fresh.decode(&frame).unwrap().unwrap().dimensions(),
					(640, 480)
				);
			}
		}
	}

	#[test]
	fn stable_rejects_other_codecs_and_malformed_input_without_losing_its_encoder() {
		let mut encoder = software_only(CAMERA);
		for codec in [VideoCodec::H265, VideoCodec::Av1] {
			let config = Config { codec, ..CAMERA };
			assert!(matches!(
				Encoder::new(config, VideoBackend::Stable),
				Err("Stable video encoding supports H.264 only")
			));
			assert_eq!(
				encoder.reconfigure(config),
				Err("Stable video encoding supports H.264 only")
			);
		}
		for length in [0, 640 * 480 * 3 / 2 - 1, 640 * 480 * 3 / 2 + 1] {
			assert!(encoder.encode(&vec![0; length], true).is_err());
		}
		let picture = vec![128; picture_bytes(CAMERA).unwrap()];
		assert!(encoder.encode(&picture, true).unwrap().1);
		let mut capped = software_only(Config {
			max_bytes: 1,
			..CAMERA
		});
		assert!(capped.encode(&picture, true).is_err());
	}

	#[test]
	fn native_color_adapters_preserve_bt601_limited_range_and_plane_order() {
		let config = Config {
			width: 2,
			height: 2,
			..CAMERA
		};
		let mut bgra = Vec::new();
		for (picture, pixel) in [
			([16, 16, 16, 16, 128, 128], [0, 0, 0, 255]),
			([235, 235, 235, 235, 128, 128], [255, 255, 255, 255]),
			([81, 81, 81, 81, 90, 240], [0, 0, 255, 255]),
		] {
			i420_to_bgra(&picture, config, &mut bgra).unwrap();
			assert!(
				bgra.as_chunks::<4>()
					.0
					.iter()
					.all(|output| *output == pixel)
			);
		}
		let mut nv12 = [0; 12];
		i420_to_nv12(&[1, 2, 3, 4, 5, 6, 7, 8], &[9, 10], &[11, 12], &mut nv12).unwrap();
		assert_eq!(nv12, [1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 10, 12]);
		assert!(i420_to_nv12(&[1; 4], &[2], &[3, 4], &mut nv12).is_err());
		assert!(i420_to_bgra(&[128; 5], config, &mut bgra).is_err());
	}
}
