//! Bounded Stable GStreamer VA-API/NVENC H.264 encoder shared by camera and screens.
//! Native capture supplies validated I420; Baseline pictures are independently decodable,
//! while Main screen pictures use reference frames and explicit IDR requests.

use super::{Config, Profile};
use gstreamer as gst;
use gstreamer::glib;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
};

const UNAVAILABLE: &str = "Linux hardware video encoding is unavailable";
const FAILED: &str = "Linux hardware video encoding failed";
/// Pictures allowed inside the encoder before its output is considered broken.
const MAX_IN_FLIGHT: u32 = 4;

/// Hardware encoder elements tried in order; the first that builds wins.
const BACKENDS: [&str; 3] = ["vah264enc", "vaapih264enc", "nvh264enc"];

pub(super) struct Encoder {
	pipeline: gst::Pipeline,
	source: gst_app::AppSrc,
	frames: gst_app::AppSink,
	/// Pictures pushed but not yet returned.
	in_flight: u32,
	pictures: u64,
	config: Config,
	backend: &'static str,
	encoder: gst::Element,
	produced_output: bool,
	failed: Arc<AtomicBool>,
}

impl Drop for Encoder {
	fn drop(&mut self) {
		let _ = self.pipeline.set_state(gst::State::Null);
	}
}

impl Encoder {
	#[allow(dead_code)] // Legacy standalone/native fixture entry point.
	pub(super) fn new(config: Config) -> Result<Self, &'static str> {
		Self::new_on_adapter(config, None)
	}

	pub(super) fn new_on_adapter(
		config: Config,
		adapter: Option<model::VideoAdapter>,
	) -> Result<Self, &'static str> {
		gst::init().map_err(|_| UNAVAILABLE)?;
		if let Some(adapter) = adapter {
			let cuda = (adapter.vendor_id == 0x10de)
				.then(|| crate::video_gpu::cuda_device(adapter))
				.flatten();
			let drm = matches!(adapter.vendor_id, 0x8086 | 0x1002)
				.then(|| crate::video_gpu::drm_device(adapter))
				.flatten();
			if cuda.is_none() && drm.is_none() {
				return Err(UNAVAILABLE);
			}
			// GStreamer registers separate factories for additional adapters.
			// Their read-only device properties identify the physical target;
			// setting a property on the generic first-device factory cannot do so.
			for factory in gst::ElementFactory::factories_with_type(
				gst::ElementFactoryType::VIDEO_ENCODER,
				gst::Rank::NONE,
			)
			.iter()
			.take(128)
			{
				let name = factory.name();
				let backend = if cuda.is_some() && name.starts_with("nvh264") {
					"nvh264enc"
				} else if drm.is_some() && name.starts_with("va") && name.contains("h264") {
					"vah264enc"
				} else {
					continue;
				};
				let Ok(encoder) = factory.create().build() else {
					continue;
				};
				let matching = if let Some(cuda) = cuda {
					encoder.find_property("cuda-device-id").is_some_and(|spec| {
						spec.flags().contains(glib::ParamFlags::READABLE)
							&& spec.value_type() == u32::static_type()
							&& encoder.property::<u32>("cuda-device-id") == cuda
					})
				} else {
					["device-path", "device"].into_iter().any(|property| {
						encoder.find_property(property).is_some_and(|spec| {
							spec.flags().contains(glib::ParamFlags::READABLE)
								&& spec.value_type() == String::static_type()
								&& encoder.property::<Option<String>>(property).as_ref()
									== drm.as_ref()
						})
					})
				};
				if matching && let Ok(encoder) = Self::assemble(config, encoder, backend) {
					return Ok(encoder);
				}
			}
			return Err(UNAVAILABLE);
		}
		BACKENDS
			.into_iter()
			.find_map(|backend| Self::start(config, backend).ok())
			.ok_or(UNAVAILABLE)
	}

	fn start(config: Config, backend: &'static str) -> Result<Self, &'static str> {
		let encoder = make(backend)?;
		Self::assemble(config, encoder, backend)
	}

	fn assemble(
		config: Config,
		encoder: gst::Element,
		backend: &'static str,
	) -> Result<Self, &'static str> {
		// Both spellings of each knob are offered; only the ones this element declares apply.
		set_number(&encoder, "bitrate", i64::from(config.bit_rate / 1000));
		let gop = if config.profile == Profile::Baseline {
			1
		} else {
			config.fps * 2
		};
		set_number(&encoder, "key-int-max", i64::from(gop));
		set_number(&encoder, "keyframe-period", i64::from(gop));
		set_number(&encoder, "gop-size", i64::from(gop));
		set_number(&encoder, "b-frames", 0);
		set_number(&encoder, "max-bframes", 0);
		set_number(&encoder, "bframes", 0);
		set_number(&encoder, "rc-lookahead", 0);

		let source = gst_app::AppSrc::builder()
			.caps(
				&gst::Caps::builder("video/x-raw")
					.field("format", "I420")
					.field("colorimetry", "bt601")
					.field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
					.field("interlace-mode", "progressive")
					.field("width", config.width as i32)
					.field("height", config.height as i32)
					.field("framerate", gst::Fraction::new(config.fps as i32, 1))
					.build(),
			)
			.build();
		source.set_stream_type(gst_app::AppStreamType::Stream);
		source.set_format(gst::Format::Time);
		source.set_is_live(true);
		source.set_do_timestamp(true);
		source.set_block(false);
		let picture_bytes = (config.width as u64)
			.checked_mul(config.height as u64)
			.and_then(|pixels| pixels.checked_mul(3))
			.map(|bytes| bytes / 2)
			.ok_or(UNAVAILABLE)?;
		source.set_max_bytes(picture_bytes.saturating_mul(u64::from(MAX_IN_FLIGHT)));
		set_number(source.upcast_ref(), "max-buffers", i64::from(MAX_IN_FLIGHT));

		// The profile is negotiated through these caps, keeping the camera's wire format the
		// same as its software encoder. An element that cannot produce it fails to link, and
		// the caller keeps openh264 rather than sending a different profile.
		let frames = gst_app::AppSink::builder()
			.caps(
				&gst::Caps::builder("video/x-h264")
					.field("stream-format", "byte-stream")
					.field("alignment", "au")
					.field(
						"profile",
						match config.profile {
							Profile::Baseline => "constrained-baseline",
							Profile::Main => "main",
						},
					)
					.build(),
			)
			.build();
		frames.set_sync(false);
		frames.set_property("async", false);
		frames.set_property("enable-last-sample", false);
		frames.set_property("wait-on-eos", false);
		frames.set_max_buffers(MAX_IN_FLIGHT);
		// Encoded pictures are never discarded here; the caller paces what reaches the wire.
		frames.set_drop(false);

		let convert = make("videoconvert")?;
		let parse = make("h264parse")?;
		// Repeat parameter sets on every IDR so a new viewer can start without prior packets.
		set_number(&parse, "config-interval", -1);

		let pipeline = gst::Pipeline::new();
		let failed = Arc::new(AtomicBool::new(false));
		pipeline.bus().ok_or(UNAVAILABLE)?.set_sync_handler({
			let failed = failed.clone();
			move |_, message| {
				if matches!(
					message.view(),
					gst::MessageView::Error(_) | gst::MessageView::Eos(_)
				) {
					failed.store(true, Ordering::Release);
				}
				gst::BusSyncReply::Drop
			}
		});
		frames
			.static_pad("sink")
			.ok_or(UNAVAILABLE)?
			.add_probe(gst::PadProbeType::BUFFER, {
				let failed = failed.clone();
				move |_, info| {
					if info
						.buffer()
						.is_none_or(|buffer| buffer.size() > config.max_bytes)
					{
						failed.store(true, Ordering::Release);
						gst::PadProbeReturn::Drop
					} else {
						gst::PadProbeReturn::Ok
					}
				}
			});
		let sink = frames.upcast_ref::<gst::Element>().clone();
		let elements = [
			source.upcast_ref::<gst::Element>(),
			&convert,
			&encoder,
			&parse,
			&sink,
		];
		pipeline.add_many(elements).map_err(|_| UNAVAILABLE)?;
		gst::Element::link_many(elements).map_err(|_| UNAVAILABLE)?;
		let capture = Self {
			pipeline,
			source,
			frames,
			in_flight: 0,
			pictures: 0,
			config,
			backend,
			encoder,
			produced_output: false,
			failed,
		};
		capture
			.pipeline
			.set_state(gst::State::Playing)
			.map_err(|_| UNAVAILABLE)?;
		Ok(capture)
	}

	pub(super) fn label(&self) -> &'static str {
		match self.backend {
			"nvh264enc" => "H.264 · Stable NVENC hardware encoding",
			_ => "H.264 · Stable VA-API hardware encoding",
		}
	}

	#[allow(dead_code)] // Retained native API; stream owners now restart atomically.
	pub(super) fn set_bitrate(&mut self, bitrate: u32) -> Result<(), &'static str> {
		if !self
			.encoder
			.find_property("bitrate")
			.is_some_and(|spec| spec.flags().contains(gst::PARAM_FLAG_MUTABLE_PLAYING))
		{
			return Err(FAILED);
		}
		set_number(&self.encoder, "bitrate", i64::from(bitrate / 1000));
		self.config.bit_rate = bitrate;
		Ok(())
	}

	/// Push one I420 picture; an empty result means bounded native output is still pending.
	pub(super) fn encode(
		&mut self,
		picture: &[u8],
		force: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let expected = (self.config.width as usize)
			.checked_mul(self.config.height as usize)
			.and_then(|pixels| pixels.checked_mul(3))
			.map(|bytes| bytes / 2)
			.ok_or(FAILED)?;
		if picture.len() != expected {
			return Err(FAILED);
		}
		// A hardware encoder that gives up reports it on the bus rather than at the push.
		if self.failed.load(Ordering::Acquire) {
			return Err(FAILED);
		}
		if self.in_flight >= MAX_IN_FLIGHT {
			return Err(FAILED);
		}
		if force && self.produced_output && self.config.profile == Profile::Main {
			let event = gstreamer_video::DownstreamForceKeyUnitEvent::builder()
				.all_headers(true)
				.build();
			if !self
				.source
				.static_pad("src")
				.ok_or(FAILED)?
				.push_event(event)
			{
				return Err(FAILED);
			}
		}
		self.pictures += 1;
		let mut buffer = gst::Buffer::from_slice(picture.to_vec());
		let pixels = self.config.width as usize * self.config.height as usize;
		let buffer_ref = buffer.get_mut().ok_or(FAILED)?;
		buffer_ref.set_offset(self.pictures);
		// I420 is tightly packed, including 854-wide shares. Declare its actual planes
		// rather than letting GStreamer assume four-byte-aligned chroma strides.
		gstreamer_video::VideoMeta::add_full(
			buffer_ref,
			gstreamer_video::VideoFrameFlags::empty(),
			gstreamer_video::VideoFormat::I420,
			self.config.width,
			self.config.height,
			&[0, pixels, pixels + pixels / 4],
			&[
				self.config.width as i32,
				self.config.width as i32 / 2,
				self.config.width as i32 / 2,
			],
		)
		.map_err(|_| FAILED)?;
		self.source.push_buffer(buffer).map_err(|_| FAILED)?;
		self.in_flight += 1;
		// Encoders hold a picture or two, so the first pushes legitimately return nothing.
		// Wait only once the backlog says one is overdue, and give a deep pipeline one last
		// bounded chance before the caller drops to software for good.
		let frame_ms = 1000 / u64::from(self.config.fps.max(1));
		let wait = match self.in_flight {
			0 | 1 => gst::ClockTime::ZERO,
			held if held < MAX_IN_FLIGHT => gst::ClockTime::from_mseconds(frame_ms),
			_ => gst::ClockTime::from_mseconds(frame_ms * 4),
		};
		let Some(sample) = self.frames.try_pull_sample(wait) else {
			return if self.in_flight >= MAX_IN_FLIGHT {
				Err(FAILED)
			} else {
				Ok((Vec::new(), false))
			};
		};
		self.in_flight = self.in_flight.saturating_sub(1);
		let buffer = sample.buffer().ok_or(FAILED)?;
		if buffer.size() == 0 || buffer.size() > self.config.max_bytes {
			return Err(FAILED);
		}
		let map = buffer.map_readable().map_err(|_| FAILED)?;
		let data = map.to_vec();
		crate::video::validate_source(&data).map_err(|_| FAILED)?;
		let keyframe = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT)
			&& crate::video_receive::is_keyframe(&data);
		if (!self.produced_output || self.config.profile == Profile::Baseline)
			&& (!keyframe || !crate::video_receive::has_parameter_sets(&data))
		{
			return Err(FAILED);
		}
		self.produced_output = true;
		Ok((data, keyframe))
	}
}

fn make(name: &str) -> Result<gst::Element, &'static str> {
	gst::ElementFactory::make(name)
		.build()
		.map_err(|_| UNAVAILABLE)
}

/// Sets a numeric property when the element declares it with a matching integer type.
fn set_number(element: &gst::Element, name: &str, value: i64) {
	let Some(spec) = element.find_property(name) else {
		return;
	};
	let kind = spec.value_type();
	if kind == glib::Type::I32
		&& let Ok(value) = i32::try_from(value)
	{
		element.set_property(name, value);
	} else if kind == glib::Type::U32
		&& let Ok(value) = u32::try_from(value)
	{
		element.set_property(name, value);
	} else if kind == glib::Type::I64 {
		element.set_property(name, value);
	} else if kind == glib::Type::U64
		&& let Ok(value) = u64::try_from(value)
	{
		element.set_property(name, value);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::voice_settings::VideoCodec;
	use openh264::formats::YUVSource;

	#[test]
	fn compact_i420_chroma_strides_encode_an_854_pixel_picture() {
		gst::init().unwrap();
		let Ok(stand_in) = make("openh264enc") else {
			eprintln!(
				"GStreamer OpenH264 fixture unavailable; native stride encode remains unverified"
			);
			return;
		};
		let config = Config {
			width: 854,
			height: 480,
			fps: 30,
			bit_rate: 2_000_000,
			max_bytes: 2 * 1024 * 1024,
			profile: Profile::Baseline,
			codec: VideoCodec::H264,
			adapter: None,
		};
		let mut encoder = Encoder::assemble(config, stand_in, "openh264enc").unwrap();
		let mut picture = vec![128; 854 * 480 * 3 / 2];
		picture[..854 * 480].fill(96);
		for length in [0, picture.len() - 1, picture.len() + 1] {
			assert!(encoder.encode(&vec![0; length], true).is_err());
		}
		let mut encoded = 0;
		for _ in 0..8 {
			let (frame, keyframe) = encoder.encode(&picture, true).unwrap();
			if frame.is_empty() {
				continue;
			}
			assert!(keyframe && frame.len() <= config.max_bytes);
			let mut decoder = openh264::decoder::Decoder::new().unwrap();
			let decoded = decoder.decode(&frame).unwrap().unwrap();
			assert_eq!(decoded.dimensions(), (854, 480));
			encoded += 1;
		}
		assert!(
			encoded > 0,
			"the bounded I420 fixture produced no access unit"
		);
	}
}
