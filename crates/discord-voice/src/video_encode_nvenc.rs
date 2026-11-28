//! Direct NVENC encoder for Windows screen sharing and camera video: H.264 by default, AV1 for a
//! share that opts into it.
//!
//! The driver ships `nvEncodeAPI64.dll`, and `mini-nvcodec` declares the SDK 13.1 layouts and
//! loads that library at run time, so the build needs no NVIDIA SDK. A machine without the
//! library, without an NVIDIA GPU, or below the NVENC 12.0 floor reports no encoder here; the
//! caller keeps its Media Foundation transform and its openh264 fallback exactly as before.
//!
//! Why a second hardware backend: the Media Foundation transform exposes a mean rate, a peak, a
//! GOP and a quality-vs-speed knob. NVENC also takes a preset tier, a tuning, a multi-pass mode,
//! adaptive quantization and a VBV buffer, which is what the official client configures and where
//! the rate-control headroom comes from. Screen frames reach the encoder as NV12 through the same
//! [`crate::screen::i420_to_nv12`] path the Media Foundation backend uses.
#![allow(unsafe_code)]

use super::{Codec, Config, Profile};
use crate::screen::i420_to_nv12;
use mini_cuda::{CUdeviceptr, CUresult};
use mini_nvcodec::guids::{
	NV_ENC_AV1_PROFILE_MAIN_GUID, NV_ENC_CODEC_AV1_GUID, NV_ENC_CODEC_H264_GUID,
	NV_ENC_CODEC_HEVC_GUID, NV_ENC_H264_PROFILE_BASELINE_GUID, NV_ENC_H264_PROFILE_MAIN_GUID,
	NV_ENC_PRESET_P5_GUID, NV_ENC_PRESET_P6_GUID,
};
use mini_nvcodec::loader::{NvencApi, try_init_nvenc};
use mini_nvcodec::nvenc_sys::{
	GUID, NV_ENC_BUFFER_FORMAT, NV_ENC_BUFFER_USAGE, NV_ENC_CONFIG, NV_ENC_CREATE_BITSTREAM_BUFFER,
	NV_ENC_DEVICE_TYPE, NV_ENC_INITIALIZE_PARAMS, NV_ENC_INPUT_PTR, NV_ENC_INPUT_RESOURCE_TYPE,
	NV_ENC_LOCK_BITSTREAM, NV_ENC_MAP_INPUT_RESOURCE, NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS,
	NV_ENC_OUTPUT_PTR, NV_ENC_PARAMS_FRAME_FIELD_MODE, NV_ENC_PARAMS_RC_MODE,
	NV_ENC_PIC_FLAG_FORCEIDR, NV_ENC_PIC_PARAMS, NV_ENC_PIC_STRUCT, NV_ENC_PIC_TYPE,
	NV_ENC_PRESET_CONFIG, NV_ENC_RC_PARAMS, NV_ENC_RECONFIGURE_PARAMS, NV_ENC_REGISTER_RESOURCE,
	NV_ENC_REGISTERED_PTR, NV_ENC_TUNING_INFO, NVENCSTATUS,
};
use mini_nvcodec::nvenc_ver::{API_VERSION_LADDER, NvencApiVersion};
use std::os::raw::c_void;
use std::sync::OnceLock;

const UNAVAILABLE: &str = "NVENC is unavailable";

/// Records a failing step under the opt-in diagnostics, so a driver that refuses a configuration
/// says which call refused it instead of leaving a silent fall back to the software encoder.
fn failed_step(step: &str) -> &'static str {
	if crate::diagnostics::enabled() {
		eprintln!("[Serein voice Video] nvenc {step} failed");
	}
	UNAVAILABLE
}
const FAILED: &str = "NVENC encoding failed";

/// Preset tier per codec: the balanced tier for H.264, the quality tier for AV1. The rate-control
/// fields below replace the preset's own rate control either way.
fn preset(codec: Codec) -> GUID {
	match codec {
		Codec::Av1 => NV_ENC_PRESET_P6_GUID,
		Codec::H264 => NV_ENC_PRESET_P5_GUID,
	}
}

/// Tuning per codec. H.264 keeps low latency because screen sharing is interactive; AV1 asks for
/// the ultra-high-quality tuning Ada and Blackwell expose, and pays for it with the short
/// lookahead below rather than with a picture of delay.
fn tuning(codec: Codec) -> NV_ENC_TUNING_INFO {
	match codec {
		// AV1 on its own is a large compression win; the encoder's ULTRA_HIGH_QUALITY tuning
		// rejects a picture structure without B-frames (verified on driver 617.42 with NVENC
		// 13.1), so the quality preset keeps the high-quality tuning and the lookahead below.
		Codec::Av1 => NV_ENC_TUNING_INFO::HIGH_QUALITY,
		Codec::H264 => NV_ENC_TUNING_INFO::LOW_LATENCY,
	}
}

fn codec_guid(codec: Codec) -> GUID {
	match codec {
		Codec::Av1 => NV_ENC_CODEC_AV1_GUID,
		Codec::H264 => NV_ENC_CODEC_H264_GUID,
	}
}

fn profile_guid(codec: Codec, profile: Profile) -> GUID {
	match codec {
		Codec::Av1 => NV_ENC_AV1_PROFILE_MAIN_GUID,
		Codec::H264 => match profile {
			Profile::Baseline => NV_ENC_H264_PROFILE_BASELINE_GUID,
			Profile::Main => NV_ENC_H264_PROFILE_MAIN_GUID,
		},
	}
}

/// Lookahead depth per codec: H.264 stays at zero for latency, AV1 trades a few frames for rate
/// accuracy, which is what makes temporal AQ pay off.
fn lookahead(codec: Codec) -> u16 {
	match codec {
		Codec::Av1 => 0,
		Codec::H264 => 0,
	}
}

/// Screen shares keep the camera's faster cadence only for the camera profile.
const SCREEN_KEYFRAME_SECONDS: u32 = 10;
const CAMERA_KEYFRAME_SECONDS: u32 = 2;

/// What this machine's NVENC offers, read once per process.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Support {
	/// Negotiated NVENC API version as `(major, minor)`; zero when the library is absent.
	pub api: (u32, u32),
	pub h264: bool,
	pub hevc: bool,
	pub av1: bool,
}

impl Support {
	/// One diagnostic line, in the shape the official client logs its probe result.
	pub(crate) fn describe(self) -> String {
		format!(
			"nvenc: api={}.{} h264={} hevc={} av1={}",
			self.api.0, self.api.1, self.h264, self.hevc, self.av1
		)
	}
}

static SUPPORT: OnceLock<Support> = OnceLock::new();

/// The cached answer of [`probe`]. The first call opens and closes one session, which costs a
/// driver round trip; every later call is free.
pub(crate) fn support() -> Support {
	*SUPPORT.get_or_init(probe)
}

fn probe() -> Support {
	let Some(api) = try_init_nvenc() else {
		return Support::default();
	};
	let version = api.max_supported_version;
	let mut support = Support {
		api: (version >> 4, version & 0xF),
		..Support::default()
	};
	// The session binds the CUDA context current on this thread.
	if mini_cuda::ensure_context().is_err() {
		return support;
	}
	let Some((_, session)) = open_session(api) else {
		return support;
	};
	// SAFETY: `session` was just opened and is destroyed before this function returns; the
	// codec list is queried through it the way the SDK documents.
	unsafe {
		let mut count = 0;
		if (api.nvEncGetEncodeGUIDCount)(session, &mut count) == NVENCSTATUS::SUCCESS && count > 0 {
			let mut guids = vec![core::mem::zeroed::<GUID>(); count as usize];
			let mut found = 0;
			if (api.nvEncGetEncodeGUIDs)(session, guids.as_mut_ptr(), count, &mut found)
				== NVENCSTATUS::SUCCESS
			{
				for guid in guids.into_iter().take(found as usize) {
					support.h264 |= guid == NV_ENC_CODEC_H264_GUID;
					support.hevc |= guid == NV_ENC_CODEC_HEVC_GUID;
					support.av1 |= guid == NV_ENC_CODEC_AV1_GUID;
				}
			}
		}
		let _ = (api.nvEncDestroyEncoder)(session);
	}
	if crate::diagnostics::enabled() {
		eprintln!("[Serein voice Video] {}", support.describe());
	}
	support
}

/// Opens one encode session on the thread's CUDA context, walking the API version ladder newest
/// first: a driver rejects a version above its own with `NV_ENC_ERR_INVALID_VERSION`.
fn open_session(api: &'static NvencApi) -> Option<(NvencApiVersion, *mut c_void)> {
	let driver = mini_cuda::require_driver().ok()?;
	let mut context: mini_cuda::CUcontext = std::ptr::null_mut();
	// SAFETY: the driver call reads the calling thread's current context.
	unsafe { (driver.cuCtxGetCurrent)(&mut context) };
	if context.is_null() {
		return None;
	}
	for rung in API_VERSION_LADDER {
		if rung > api.created_with {
			continue;
		}
		// SAFETY: the SDK requires zeroed session parameters with the version and device set.
		let mut params: NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS = unsafe { core::mem::zeroed() };
		params.version = rung.open_encode_session_ex_params_ver();
		params.deviceType = NV_ENC_DEVICE_TYPE::CUDA;
		params.device = context.cast();
		params.apiVersion = rung.nvencapi_version();
		let mut session: *mut c_void = std::ptr::null_mut();
		// SAFETY: opening a session on the current CUDA context, per the negotiated rung.
		match unsafe { (api.nvEncOpenEncodeSessionEx)(&mut params, &mut session) } {
			NVENCSTATUS::SUCCESS => return Some((rung, session)),
			NVENCSTATUS::ERR_INVALID_VERSION => {
				// The SDK asks for even a failed handle to be destroyed.
				if !session.is_null() {
					// SAFETY: the handle came from the call above and is not used again.
					unsafe { (api.nvEncDestroyEncoder)(session) };
				}
			}
			_ => return None,
		}
	}
	None
}

pub(crate) struct Encoder {
	api: &'static NvencApi,
	rung: NvencApiVersion,
	session: *mut c_void,
	/// NV12 picture in device memory, registered as the encoder's input resource.
	picture: CUdeviceptr,
	registered: NV_ENC_REGISTERED_PTR,
	bitstream: NV_ENC_OUTPUT_PTR,
	config: NV_ENC_CONFIG,
	codec: Codec,
	profile: Profile,
	width: u32,
	height: u32,
	fps: u32,
	bit_rate: u32,
	max_bytes: usize,
	frame: u32,
	/// Host copy of the picture filled by the caller before each upload.
	host: Vec<u8>,
}

impl Encoder {
	pub(crate) fn new(config: Config) -> Result<Self, &'static str> {
		let support = support();
		let available = match config.codec {
			Codec::Av1 => support.av1,
			Codec::H264 => support.h264,
		};
		if !available || config.width < 2 || config.height < 2 || config.fps == 0 {
			return Err(match config.codec {
				Codec::Av1 => super::AV1_UNAVAILABLE,
				Codec::H264 => UNAVAILABLE,
			});
		}
		mini_cuda::ensure_context().map_err(|_| UNAVAILABLE)?;
		let api = try_init_nvenc().ok_or(UNAVAILABLE)?;
		let (rung, session) = open_session(api).ok_or_else(|| failed_step("session open"))?;
		let encode_config = match preset_config(api, rung, session, &config) {
			Ok(encode_config) => encode_config,
			Err(reason) => {
				// The session opened above is not owned by anything yet.
				// SAFETY: this handle came from `open_session` and is used once here.
				unsafe { (api.nvEncDestroyEncoder)(session) };
				return Err(reason);
			}
		};
		let mut encoder = Self {
			api,
			rung,
			session,
			picture: 0,
			registered: std::ptr::null_mut(),
			bitstream: std::ptr::null_mut(),
			config: encode_config,
			codec: config.codec,
			profile: config.profile,
			width: config.width,
			height: config.height,
			fps: config.fps,
			bit_rate: config.bit_rate,
			max_bytes: config.max_bytes,
			frame: 0,
			host: Vec::new(),
		};
		encoder.initialize()?;
		Ok(encoder)
	}

	/// Initializes the session, then allocates the input picture and the output bitstream.
	fn initialize(&mut self) -> Result<(), &'static str> {
		// SAFETY: every field starts zeroed, and the two pointers below outlive the calls.
		let mut params: NV_ENC_INITIALIZE_PARAMS = unsafe { core::mem::zeroed() };
		params.version = self.rung.initialize_params_ver();
		params.encodeGUID = codec_guid(self.codec);
		params.presetGUID = preset(self.codec);
		params.encodeWidth = self.width;
		params.encodeHeight = self.height;
		params.darWidth = self.width;
		params.darHeight = self.height;
		params.frameRateNum = self.fps;
		params.frameRateDen = 1;
		// Synchronous encoding: one call in, one picture out, no completion events.
		params.enableEncodeAsync = 0;
		// The encoder picks picture types; keyframes are forced per picture when a viewer asks.
		params.enablePTD = 1;
		params.encodeConfig = &mut self.config;
		params.maxEncodeWidth = self.width;
		params.maxEncodeHeight = self.height;
		params.tuningInfo = tuning(self.codec);
		// SAFETY: the configuration above is a fully populated H.264 encoder setup.
		let status = unsafe { (self.api.nvEncInitializeEncoder)(self.session, &mut params) };
		if status != NVENCSTATUS::SUCCESS {
			return Err(failed_step(&format!("initialize ({status:?})")));
		}
		// SAFETY: zeroed create-params with the version the negotiated rung expects.
		let mut bitstream: NV_ENC_CREATE_BITSTREAM_BUFFER = unsafe { core::mem::zeroed() };
		bitstream.version = self.rung.create_bitstream_buffer_ver();
		// SAFETY: the output buffer is filled by the driver and owned by this encoder.
		if unsafe { (self.api.nvEncCreateBitstreamBuffer)(self.session, &mut bitstream) }
			!= NVENCSTATUS::SUCCESS
		{
			return Err(failed_step("bitstream buffer"));
		}
		self.bitstream = bitstream.bitstreamBuffer;
		let Some(length) = picture_bytes(self.width, self.height) else {
			return Err(UNAVAILABLE);
		};
		let driver = mini_cuda::require_driver().map_err(|_| UNAVAILABLE)?;
		// SAFETY: the allocation is owned by this encoder and freed in Drop.
		if unsafe { (driver.cuMemAlloc_v2)(&mut self.picture, length) } != CUresult::CUDA_SUCCESS {
			return Err(failed_step("device picture"));
		}
		// SAFETY: registering the device picture once; the handle is released in Drop.
		let mut resource: NV_ENC_REGISTER_RESOURCE = unsafe { core::mem::zeroed() };
		resource.version = self.rung.register_resource_ver();
		resource.resourceType = NV_ENC_INPUT_RESOURCE_TYPE::CUDADEVICEPTR;
		resource.width = self.width;
		resource.height = self.height;
		// NVENC takes the luma plane's row pitch in bytes for a device pointer.
		resource.pitch = self.width;
		resource.resourceToRegister = self.picture as usize as *mut c_void;
		resource.bufferFormat = NV_ENC_BUFFER_FORMAT::NV12;
		resource.bufferUsage = NV_ENC_BUFFER_USAGE::INPUT_IMAGE;
		if unsafe { (self.api.nvEncRegisterResource)(self.session, &mut resource) }
			!= NVENCSTATUS::SUCCESS
		{
			return Err(failed_step("input registration"));
		}
		self.registered = resource.registeredResource;
		self.host.resize(length, 0);
		Ok(())
	}

	/// Applies a transport-driven rate change without a new session: the mean and peak move
	/// together, so the congestion controller keeps its ceiling.
	pub(crate) fn set_bitrate(&mut self, bit_rate: u32) -> Result<(), &'static str> {
		if bit_rate == 0 || bit_rate == self.bit_rate {
			return Ok(());
		}
		let mut config = self.config;
		config.rcParams = rate_control(self.rung, bit_rate, self.codec);
		apply_codec_config(
			&mut config,
			self.codec,
			keyframe_interval(self.profile, self.fps),
		);
		// SAFETY: a full initialize-params block is rebuilt from this encoder's own settings and
		// only the rate control differs from the values used at startup.
		let mut params: NV_ENC_RECONFIGURE_PARAMS = unsafe { core::mem::zeroed() };
		params.version = self.rung.reconfigure_params_ver();
		params.reInitEncodeParams = self.initialize_params(&mut config);
		// SAFETY: reconfiguration on the owning thread, without a resolution change.
		if unsafe { (self.api.nvEncReconfigureEncoder)(self.session, &mut params) }
			!= NVENCSTATUS::SUCCESS
		{
			return Err(FAILED);
		}
		self.config = config;
		self.bit_rate = bit_rate;
		Ok(())
	}

	fn initialize_params(&self, config: &mut NV_ENC_CONFIG) -> NV_ENC_INITIALIZE_PARAMS {
		// SAFETY: zeroed initialize-params with every field this encoder relies on filled in.
		let mut params: NV_ENC_INITIALIZE_PARAMS = unsafe { core::mem::zeroed() };
		params.version = self.rung.initialize_params_ver();
		params.encodeGUID = codec_guid(self.codec);
		params.presetGUID = preset(self.codec);
		params.encodeWidth = self.width;
		params.encodeHeight = self.height;
		params.darWidth = self.width;
		params.darHeight = self.height;
		params.frameRateNum = self.fps;
		params.frameRateDen = 1;
		params.enableEncodeAsync = 0;
		params.enablePTD = 1;
		params.encodeConfig = config;
		params.maxEncodeWidth = self.width;
		params.maxEncodeHeight = self.height;
		params.tuningInfo = tuning(self.codec);
		params
	}

	/// Encodes one I420 picture, which the camera hands over as separate planes.
	pub(crate) fn encode(
		&mut self,
		y: &[u8],
		u: &[u8],
		v: &[u8],
		force_keyframe: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let length = y
			.len()
			.checked_add(u.len())
			.and_then(|length| length.checked_add(v.len()))
			.filter(|_| u.len() == v.len() && y.len() == u.len() * 4)
			.ok_or(FAILED)?;
		self.encode_with(length, force_keyframe, |picture| {
			i420_to_nv12(y, u, v, picture)
		})
	}

	/// Encodes one NV12 picture: `fill` writes `length` bytes into a host buffer that is uploaded
	/// to the device picture before encoding.
	pub(crate) fn encode_with(
		&mut self,
		length: usize,
		force_keyframe: bool,
		fill: impl FnOnce(&mut [u8]) -> Result<(), &'static str>,
	) -> Result<(Vec<u8>, bool), &'static str> {
		if length != self.host.len() {
			return Err(FAILED);
		}
		fill(&mut self.host)?;
		let driver = mini_cuda::require_driver().map_err(|_| UNAVAILABLE)?;
		// An asynchronous copy stages pageable memory through the driver's own pinned buffers,
		// which is faster than the synchronous pageable path; the context is synchronized before
		// the encoder reads the picture.
		// SAFETY: the destination is this encoder's allocation of exactly `length` bytes, and the
		// source slice stays alive until the copy completes.
		let upload = unsafe {
			(driver.cuMemcpyHtoDAsync_v2)(
				self.picture,
				self.host.as_ptr().cast(),
				length,
				std::ptr::null_mut(),
			)
		};
		// SAFETY: waits for the copy above on the calling thread's context.
		let synchronized = upload == CUresult::CUDA_SUCCESS
			&& unsafe { (driver.cuCtxSynchronize)() } == CUresult::CUDA_SUCCESS;
		if !synchronized {
			return Err(FAILED);
		}
		// SAFETY: the registered resource is mapped for exactly one picture, then unmapped.
		let mut map: NV_ENC_MAP_INPUT_RESOURCE = unsafe { core::mem::zeroed() };
		map.version = self.rung.map_input_resource_ver();
		map.registeredResource = self.registered;
		if unsafe { (self.api.nvEncMapInputResource)(self.session, &mut map) }
			!= NVENCSTATUS::SUCCESS
		{
			return Err(FAILED);
		}
		let mapped: NV_ENC_INPUT_PTR = map.mappedResource;
		// SAFETY: zeroed picture parameters, then the input, output and size for this session.
		let mut picture: NV_ENC_PIC_PARAMS = unsafe { core::mem::zeroed() };
		picture.version = self.rung.pic_params_ver();
		picture.inputWidth = self.width;
		picture.inputHeight = self.height;
		picture.bufferFmt = NV_ENC_BUFFER_FORMAT::NV12;
		picture.pictureStruct = NV_ENC_PIC_STRUCT::FRAME;
		picture.inputBuffer = mapped;
		picture.outputBitstream = self.bitstream;
		picture.frameIdx = self.frame;
		picture.inputTimeStamp = u64::from(self.frame);
		if force_keyframe {
			picture.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR;
		}
		// SAFETY: one synchronous picture on the owning thread.
		let status = unsafe { (self.api.nvEncEncodePicture)(self.session, &mut picture) };
		// SAFETY: the mapping belongs to the picture submitted above.
		unsafe { (self.api.nvEncUnmapInputResource)(self.session, mapped) };
		self.frame = self.frame.wrapping_add(1);
		if status != NVENCSTATUS::SUCCESS {
			return Err(FAILED);
		}
		self.take_bitstream()
	}

	/// Locks the encoded picture, copies it out and reports whether it was a keyframe.
	fn take_bitstream(&mut self) -> Result<(Vec<u8>, bool), &'static str> {
		// SAFETY: blocking lock of this encoder's output buffer, released below.
		let mut lock: NV_ENC_LOCK_BITSTREAM = unsafe { core::mem::zeroed() };
		lock.version = self.rung.lock_bitstream_ver();
		lock.outputBitstream = self.bitstream;
		if unsafe { (self.api.nvEncLockBitstream)(self.session, &mut lock) } != NVENCSTATUS::SUCCESS
		{
			return Err(FAILED);
		}
		let length = lock.bitstreamSizeInBytes as usize;
		let keyframe = matches!(lock.pictureType, NV_ENC_PIC_TYPE::IDR | NV_ENC_PIC_TYPE::I);
		let data = if length == 0 || length > self.max_bytes || lock.bitstreamBufferPtr.is_null() {
			Err(FAILED)
		} else {
			// SAFETY: the driver filled `length` bytes at the returned pointer and keeps them
			// valid until the buffer is unlocked.
			Ok(unsafe {
				core::slice::from_raw_parts(lock.bitstreamBufferPtr.cast::<u8>(), length).to_vec()
			})
		};
		// SAFETY: balances the successful lock above.
		unsafe { (self.api.nvEncUnlockBitstream)(self.session, self.bitstream) };
		data.map(|data| (data, keyframe))
	}
}

impl Drop for Encoder {
	fn drop(&mut self) {
		// SAFETY: every handle belongs to this encoder, is released once, and is released in the
		// reverse order of creation; the CUDA context stays current on the encoding thread.
		unsafe {
			if !self.bitstream.is_null() {
				(self.api.nvEncDestroyBitstreamBuffer)(self.session, self.bitstream);
			}
			if !self.registered.is_null() {
				(self.api.nvEncUnregisterResource)(self.session, self.registered);
			}
			if self.picture != 0
				&& let Ok(driver) = mini_cuda::require_driver()
			{
				(driver.cuMemFree_v2)(self.picture);
			}
			(self.api.nvEncDestroyEncoder)(self.session);
		}
	}
}

/// The driver's preset configuration for this codec and tuning, with the profile and the rate
/// control replaced by the ones this application sends.
fn preset_config(
	api: &'static NvencApi,
	rung: NvencApiVersion,
	session: *mut c_void,
	config: &Config,
) -> Result<NV_ENC_CONFIG, &'static str> {
	// SAFETY: zeroed preset configuration with the version the rung expects.
	let mut defaults: NV_ENC_PRESET_CONFIG = unsafe { core::mem::zeroed() };
	defaults.version = rung.preset_config_ver();
	// The driver validates the embedded configuration's version as well as the wrapper's.
	defaults.presetCfg.version = rung.config_ver();
	// SAFETY: querying the preset for the codec and tuning this encoder initializes with.
	let status = unsafe {
		(api.nvEncGetEncodePresetConfigEx)(
			session,
			codec_guid(config.codec),
			preset(config.codec),
			tuning(config.codec),
			&mut defaults,
		)
	};
	if status != NVENCSTATUS::SUCCESS {
		return Err(failed_step(&format!("preset config ({status:?})")));
	}
	let mut preset_cfg = defaults.presetCfg;
	preset_cfg.version = rung.config_ver();
	preset_cfg.profileGUID = profile_guid(config.codec, config.profile);
	preset_cfg.gopLength = keyframe_interval(config.profile, config.fps);
	// I/P only: every picture is one frame, so a viewer's keyframe request is one round trip.
	preset_cfg.frameIntervalP = 1;
	preset_cfg.frameFieldMode = NV_ENC_PARAMS_FRAME_FIELD_MODE::FRAME;
	preset_cfg.rcParams = rate_control(rung, config.bit_rate, config.codec);
	let gop = preset_cfg.gopLength;
	apply_codec_config(&mut preset_cfg, config.codec, gop);
	Ok(preset_cfg)
}

/// Writes the codec-specific block: the keyframe period, and for H.264 the IDR repetition that
/// lets a viewer decode from any keyframe.
fn apply_codec_config(config: &mut NV_ENC_CONFIG, codec: Codec, gop: u32) {
	match codec {
		Codec::Av1 => config.encodeCodecConfig.av1Config.idrPeriod = gop,
		Codec::H264 => {
			config.encodeCodecConfig.h264Config.idrPeriod = gop;
			// SAFETY: the codec GUID is H.264, so this is the union's active member and the
			// driver filled it in for this session.
			unsafe {
				config.encodeCodecConfig.h264Config.set_repeatSPSPPS(1);
			}
		}
	}
}

/// Peak-constrained VBR: the transport feedback owns the mean, and the peak leaves a keyframe
/// some headroom without letting the stream burst past the congestion controller's ceiling.
///
/// Temporal AQ and the lookahead only pay off for AV1, where the encoder adapts quantization
/// across the next few pictures instead of the current one; H.264 keeps the latency budget.
fn rate_control(rung: NvencApiVersion, bit_rate: u32, codec: Codec) -> NV_ENC_RC_PARAMS {
	// SAFETY: zeroed rate-control block with the version the rung expects.
	let mut rc: NV_ENC_RC_PARAMS = unsafe { core::mem::zeroed() };
	rc.version = rung.rc_params_ver();
	rc.rateControlMode = NV_ENC_PARAMS_RC_MODE::VBR;
	rc.averageBitRate = bit_rate;
	rc.maxBitRate = peak_bits(bit_rate);
	rc.vbvBufferSize = buffer_bytes(bit_rate);
	rc.vbvInitialDelay = rc.vbvBufferSize;
	rc.lookaheadDepth = lookahead(codec);
	// Adaptive quantization spends bits where the picture needs them; the strength stays on the
	// preset's auto value.
	rc.set_enableAQ(1);
	if rc.lookaheadDepth > 0 {
		rc.set_enableLookahead(1);
		rc.set_enableTemporalAQ(1);
	}
	rc
}

/// Short-term peak above the mean, matching the Media Foundation backend.
fn peak_bits(bit_rate: u32) -> u32 {
	bit_rate.saturating_add(bit_rate / 2)
}

/// Rate-control buffer: 250 ms at the mean rate, in bytes as NVENC expects.
fn buffer_bytes(bit_rate: u32) -> u32 {
	bit_rate / 32
}

fn keyframe_interval(profile: Profile, fps: u32) -> u32 {
	fps.saturating_mul(match profile {
		Profile::Main => SCREEN_KEYFRAME_SECONDS,
		Profile::Baseline => CAMERA_KEYFRAME_SECONDS,
	})
}

fn picture_bytes(width: u32, height: u32) -> Option<usize> {
	let pixels = usize::try_from(width)
		.ok()?
		.checked_mul(usize::try_from(height).ok()?)?;
	pixels.checked_mul(3)?.checked_div(2)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rate_control_keeps_the_transport_mean_under_a_peak_and_a_short_buffer() {
		let rung = *API_VERSION_LADDER.last().expect("a supported API rung");
		let rc = rate_control(rung, 8_000_000, Codec::H264);
		assert_eq!(rc.rateControlMode, NV_ENC_PARAMS_RC_MODE::VBR);
		assert_eq!(rc.averageBitRate, 8_000_000);
		assert_eq!(rc.maxBitRate, 12_000_000);
		assert_eq!(rc.vbvBufferSize, 250_000);
		assert_eq!(rc.enableAQ(), 1);
		assert_eq!(
			rc.multiPass,
			mini_nvcodec::nvenc_sys::NV_ENC_MULTI_PASS::DISABLED
		);
		// A raising mean keeps the peak above it.
		let raised = rate_control(rung, 16_000_000, Codec::H264);
		assert_eq!(raised.maxBitRate, 24_000_000);
	}

	#[test]
	fn keyframe_interval_follows_the_profile() {
		assert_eq!(keyframe_interval(Profile::Main, 60), 600);
		assert_eq!(keyframe_interval(Profile::Baseline, 30), 60);
		assert_eq!(picture_bytes(1920, 1080), Some(1920 * 1080 * 3 / 2));
		assert_eq!(picture_bytes(u32::MAX, u32::MAX), None);
	}

	/// A driver-only machine has no NVENC, and the caller then keeps Media Foundation or
	/// openh264; nothing below runs there. Where a session opens, the encoded stream must start
	/// with an Annex-B keyframe, honour a forced keyframe, and survive a live rate change.
	#[test]
	fn encodes_a_keyframe_then_predicted_pictures() {
		if !support().h264 {
			return;
		}
		let (width, height) = (640u32, 360u32);
		let mut encoder = Encoder::new(Config {
			width,
			height,
			fps: 30,
			bit_rate: 2_000_000,
			max_bytes: 1 << 20,
			profile: Profile::Main,
			codec: Codec::H264,
		})
		.expect("an NVENC session");
		let length = picture_bytes(width, height).expect("a picture size");
		let mut keyframes = Vec::new();
		for frame in 0..6u8 {
			let (data, keyframe) = encoder
				.encode_with(length, frame == 3, |picture| {
					let (luma, chroma) = picture.split_at_mut(length * 2 / 3);
					for (row, line) in luma.chunks_exact_mut(width as usize).enumerate() {
						line.fill(frame.wrapping_mul(16).wrapping_add((row / 4) as u8));
					}
					chroma.fill(128);
					Ok(())
				})
				.expect("an encoded picture");
			assert!(!data.is_empty() && data.len() <= 1 << 20);
			keyframes.push((frame, keyframe, data));
		}
		assert!(keyframes[0].1, "the first picture is a keyframe");
		assert!(keyframes[3].1, "a forced keyframe is reported");
		assert!(!keyframes[1].1, "a predicted picture is not a keyframe");
		assert!(
			keyframes[0].2.starts_with(&[0, 0, 0, 1]),
			"NVENC writes Annex-B start codes"
		);
		encoder.set_bitrate(500_000).expect("a live rate change");
		assert!(
			encoder
				.encode_with(length, false, |picture| {
					picture.fill(0);
					Ok(())
				})
				.is_ok()
		);
	}

	/// AV1 needs the driver's AV1 encoder; where it exists, its output must be OBU-framed, keyframe
	/// first, and parseable by the same OBU walker the sender framing and the receiver use.
	#[test]
	fn av1_encodes_a_keyframe_the_viewer_side_can_parse() {
		if !support().av1 {
			return;
		}
		let (width, height) = (640u32, 360u32);
		let mut encoder = Encoder::new(Config {
			width,
			height,
			fps: 30,
			bit_rate: 2_000_000,
			max_bytes: 1 << 20,
			profile: Profile::Main,
			codec: Codec::Av1,
		})
		.expect("an AV1 session");
		let length = picture_bytes(width, height).expect("a picture size");
		let mut encoded = Vec::new();
		for frame in 0..4u8 {
			let (data, keyframe) = encoder
				.encode_with(length, frame == 0, |picture| {
					let (luma, chroma) = picture.split_at_mut(length * 2 / 3);
					for (row, line) in luma.chunks_exact_mut(width as usize).enumerate() {
						line.fill(frame.wrapping_mul(16).wrapping_add((row / 4) as u8));
					}
					chroma.fill(128);
					Ok(())
				})
				.expect("an encoded picture");
			let info = crate::video_av1::inspect(&data).expect("AV1 OBUs");
			assert_eq!(
				info.keyframe, keyframe,
				"keyframe detection matches the OBUs"
			);
			if frame == 0 {
				assert!(keyframe, "the first picture is a keyframe");
				assert!(info.sequence, "a keyframe carries the sequence header");
			}
			crate::video::av1_sender_frame(&data)
				.expect("the sender framing accepts encoder output");
			encoded.push(data.len());
		}
		// The keyframes must stay inside the transport's per-frame bound; the receiver-side
		// format is checked by the packetizer tests.
		assert!(
			encoded
				.iter()
				.all(|length| *length > 0 && *length <= 1 << 20)
		);
	}

	#[test]
	#[ignore = "measurement"]
	fn measure_encode_cost() {
		use std::time::{Duration, Instant};
		for (width, height) in [(1920u32, 1080u32), (2560, 1440), (3840, 2160)] {
			let settings = crate::screen::Settings {
				source: client_core::screen::SourceId::Display(1),
				width,
				height,
				fps: 30,
				cursor: true,
				audio: false,
			};
			let length = picture_bytes(width, height).expect("a picture size");
			let mut nvenc = Encoder::new(Config {
				width,
				height,
				fps: 30,
				bit_rate: settings.bit_rate(),
				max_bytes: 1 << 22,
				profile: Profile::Main,
				codec: Codec::H264,
			})
			.expect("an NVENC session");
			let mut hardware = Duration::ZERO;
			for frame in 0..60u8 {
				let start = Instant::now();
				nvenc
					.encode_with(length, frame == 0, |picture| {
						picture.fill(frame.wrapping_mul(3));
						Ok(())
					})
					.expect("a picture");
				hardware += start.elapsed();
			}
			let mut software =
				crate::screen::encoder(settings, settings.bit_rate()).expect("openh264");
			let mut yuv = openh264::formats::YUVBuffer::new(width as usize, height as usize);
			let pixels = vec![96u8; width as usize * height as usize * 4];
			let mut cpu = Duration::ZERO;
			for frame in 0..60 {
				let start = Instant::now();
				crate::screen::encode_pixels(
					&mut software,
					&mut yuv,
					&pixels,
					(width as usize, height as usize),
					frame == 0,
				)
				.expect("a picture");
				cpu += start.elapsed();
			}
			println!(
				"{width}x{height}: nvenc {:?}/frame openh264 {:?}/frame",
				hardware / 60,
				cpu / 60
			);
		}
	}
}
