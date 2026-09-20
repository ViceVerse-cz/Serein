//! Bounded GStreamer hardware H.264 encoder for Linux camera video.
//!
//! The camera's v4l2 capture path produces packed RGB in Rust, so unlike screen sharing there
//! is no capture element to hang an encoder off. This pushes those pictures through a private
//! `appsrc` into a VA-API or NVENC encoder and pulls Annex B access units back out. Elements
//! are built individually, so a machine without the plugin fails at construction and the
//! caller keeps openh264.
//!
//! Every picture is coded as an IDR, matching the software encoder: the camera sender drops to
//! the latest frame, so inter prediction would freeze a receiver until the next refresh. The
//! keyframe interval is only a request, so each returned access unit is checked and a backend
//! that ignores it is rejected.
//!
//! Property names and types differ between these elements and across their versions, so every
//! property is matched against its own `ParamSpec` before being set. Setting one blindly, or
//! from a string, aborts the process when the name or type does not match.

use crate::video_encode::{Config, Profile};
use gstreamer as gst;
use gstreamer::glib;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

const UNAVAILABLE: &str = "Linux hardware video encoding is unavailable";
const FAILED: &str = "Linux hardware video encoding failed";
/// Pictures allowed inside the encoder before its output is considered broken.
const MAX_IN_FLIGHT: u32 = 6;

/// Hardware encoder elements tried in order; the first that builds wins.
const BACKENDS: [&str; 2] = ["vah264enc", "nvh264enc"];

pub(super) struct Encoder {
	pipeline: gst::Pipeline,
	source: gst_app::AppSrc,
	frames: gst_app::AppSink,
	/// Pictures pushed but not yet returned.
	in_flight: u32,
	pictures: u64,
	config: Config,
}

impl Drop for Encoder {
	fn drop(&mut self) {
		let _ = self.pipeline.set_state(gst::State::Null);
	}
}

impl Encoder {
	pub(super) fn new(config: Config) -> Result<Self, &'static str> {
		gst::init().map_err(|_| UNAVAILABLE)?;
		BACKENDS
			.into_iter()
			.find_map(|backend| Self::start(config, backend).ok())
			.ok_or(UNAVAILABLE)
	}

	fn start(config: Config, backend: &str) -> Result<Self, &'static str> {
		let encoder = make(backend)?;
		Self::assemble(config, encoder)
	}

	fn assemble(config: Config, encoder: gst::Element) -> Result<Self, &'static str> {
		// Both spellings of each knob are offered; only the ones this element declares apply.
		set_number(&encoder, "bitrate", i64::from(config.bit_rate / 1000));
		set_number(&encoder, "key-int-max", 1);
		set_number(&encoder, "gop-size", 1);
		set_number(&encoder, "b-frames", 0);
		set_number(&encoder, "bframes", 0);
		set_number(&encoder, "rc-lookahead", 0);
		set_flag(&encoder, "zerolatency", true);

		let source = gst_app::AppSrc::builder()
			.caps(
				&gst::Caps::builder("video/x-raw")
					.field("format", "RGB")
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
			.ok_or(UNAVAILABLE)?;
		source.set_max_bytes(picture_bytes.saturating_mul(u64::from(MAX_IN_FLIGHT)));

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
		frames.set_max_buffers(MAX_IN_FLIGHT);
		// Encoded pictures are never discarded here; the caller paces what reaches the wire.
		frames.set_drop(false);

		let convert = make("videoconvert")?;
		let parse = make("h264parse")?;
		// Repeat the parameter sets on every access unit so each picture decodes alone.
		set_number(&parse, "config-interval", -1);

		let pipeline = gst::Pipeline::new();
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
		pipeline
			.set_state(gst::State::Playing)
			.map_err(|_| UNAVAILABLE)?;
		Ok(Self {
			pipeline,
			source,
			frames,
			in_flight: 0,
			pictures: 0,
			config,
		})
	}

	/// Pushes one packed RGB picture and returns the next finished access unit, or `None` while
	/// the encoder has not produced one yet.
	pub(super) fn encode(&mut self, rgb: &[u8]) -> Result<Option<Vec<u8>>, &'static str> {
		let expected = (self.config.width as usize)
			.checked_mul(self.config.height as usize)
			.and_then(|pixels| pixels.checked_mul(3))
			.ok_or(FAILED)?;
		if rgb.len() != expected {
			return Err(FAILED);
		}
		// A hardware encoder that gives up reports it on the bus rather than at the push.
		let bus = self.pipeline.bus().ok_or(FAILED)?;
		if bus.pop_filtered(&[gst::MessageType::Error]).is_some() {
			return Err(FAILED);
		}
		self.pictures += 1;
		let mut buffer = gst::Buffer::from_slice(rgb.to_vec());
		buffer.get_mut().ok_or(FAILED)?.set_offset(self.pictures);
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
				Ok(None)
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
		// The stream must stay independently decodable. A delta picture means the element
		// ignored the requested keyframe interval, so this backend is unusable.
		if buffer.flags().contains(gst::BufferFlags::DELTA_UNIT)
			|| !crate::video_receive::is_keyframe(&data)
			|| !crate::video_receive::has_parameter_sets(&data)
		{
			return Err(FAILED);
		}
		Ok(Some(data))
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

/// Sets a boolean property when the element declares it as one.
fn set_flag(element: &gst::Element, name: &str, value: bool) {
	if element
		.find_property(name)
		.is_some_and(|spec| spec.value_type() == glib::Type::BOOL)
	{
		element.set_property(name, value);
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
	};

	/// Exercises the real pipeline with whichever H.264 encoder this machine has. Development
	/// machines without VA-API or NVENC still cover the appsrc, property, pacing and Annex B
	/// paths through `x264enc`; production only ever tries the two hardware elements.
	fn any_encoder() -> Option<Encoder> {
		gst::init().ok()?;
		BACKENDS
			.into_iter()
			.chain(std::iter::once("x264enc"))
			.find_map(|backend| {
				let encoder = make(backend).ok()?;
				// The software stand-in buffers many frames by default, unlike the real-time
				// hardware elements. These values belong to x264: NVENC also exposes
				// `tune`, but with different enum values.
				if backend == "x264enc" {
					encoder.set_property_from_str("tune", "zerolatency");
					encoder.set_property_from_str("speed-preset", "ultrafast");
				}
				let encoder = Encoder::assemble(CAMERA, encoder).ok()?;
				eprintln!("Synthetic camera encoder test using {backend}");
				Some(encoder)
			})
	}

	#[test]
	fn rejects_unbounded_pictures_and_stays_independently_decodable() {
		let Some(mut encoder) = any_encoder() else {
			return;
		};
		for length in [0, 640 * 480 * 3 - 1, 640 * 480 * 3 + 1] {
			assert!(encoder.encode(&vec![0; length]).is_err());
		}
		let mut coded = 0;
		for value in [0u8, 32, 96, 160, 255, 8, 200, 64] {
			let mut rgb = vec![value; 640 * 480 * 3];
			for (index, pixel) in rgb.as_chunks_mut::<3>().0.iter_mut().take(640).enumerate() {
				*pixel = [(index % 251) as u8, value, (index % 97) as u8];
			}
			let Some(data) = encoder.encode(&rgb).expect("hardware encode") else {
				continue;
			};
			coded += 1;
			assert!(data.len() <= CAMERA.max_bytes);
			assert!(crate::video_receive::is_keyframe(&data));
			assert!(crate::video_receive::has_parameter_sets(&data));
		}
		assert!(coded > 0, "the pipeline returned no encoded picture");
	}

	#[test]
	fn missing_elements_report_unavailable_rather_than_panicking() {
		assert!(gst::init().is_ok());
		// An element this machine does not have must fail cleanly, never abort.
		assert_eq!(make("serein-no-such-encoder").unwrap_err(), UNAVAILABLE);
		// Property helpers must ignore names and types an element does not declare.
		let convert = make("videoconvert").expect("videoconvert");
		set_number(&convert, "serein-no-such-property", 1);
		set_flag(&convert, "serein-no-such-property", true);
		// A real property of the wrong type is left alone rather than aborting.
		set_number(&convert, "qos", 1);
		set_flag(&convert, "qos", true);
		assert!(convert.property::<bool>("qos"));
	}
}
