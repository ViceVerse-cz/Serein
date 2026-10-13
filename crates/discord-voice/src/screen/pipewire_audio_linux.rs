//! Native PipeWire application outputs that are absent from PulseAudio's sink-input list.
use super::{FRAME_SAMPLES, MAX_AUDIO_SAMPLES, MAX_INPUTS, OwnApplication, Pending, UNAVAILABLE};
use ::gstreamer as gst;
use gst::prelude::*;
use gstreamer_app as app;
use std::{
	io::Read,
	os::unix::process::CommandExt,
	process::{Command, Stdio},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
	time::{Duration, Instant},
};

const MAX_GRAPH_BYTES: usize = 1024 * 1024;
const MAX_GRAPH_OBJECTS: usize = 4096;
const MAX_HELPER_ADDRESS_SPACE: libc::rlim_t = 128 * 1024 * 1024;
const GRAPH_TIMEOUT: Duration = Duration::from_secs(2);

struct Discovery {
	stop: Arc<AtomicBool>,
	thread: Option<std::thread::JoinHandle<()>>,
	receive: mpsc::Receiver<Result<Vec<u64>, &'static str>>,
}

impl Discovery {
	fn start(own: OwnApplication) -> Option<Self> {
		let stop = Arc::new(AtomicBool::new(false));
		let stopped = stop.clone();
		let (send, receive) = mpsc::sync_channel(1);
		let thread = std::thread::Builder::new()
			.name("screen-audio-graph".into())
			.spawn(move || {
				while !stopped.load(Ordering::Acquire) {
					let outputs = snapshot(&own, &stopped);
					let _ = send.try_send(outputs);
					std::thread::park_timeout(super::LISTING);
				}
			})
			.ok()?;
		Some(Self {
			stop,
			thread: Some(thread),
			receive,
		})
	}
}

impl Drop for Discovery {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Release);
		if let Some(thread) = self.thread.take() {
			thread.thread().unpark();
			let _ = thread.join();
		}
	}
}

fn snapshot(own: &OwnApplication, stop: &AtomicBool) -> Result<Vec<u64>, &'static str> {
	if stop.load(Ordering::Acquire) {
		return Err(UNAVAILABLE);
	}
	let mut command = Command::new("pw-dump");
	command.arg("--no-colors");
	collect_graph(command, own, stop, GRAPH_TIMEOUT)
}

fn collect_graph(
	mut command: Command,
	own: &OwnApplication,
	stop: &AtomicBool,
	timeout: Duration,
) -> Result<Vec<u64>, &'static str> {
	if stop.load(Ordering::Acquire) {
		return Err(UNAVAILABLE);
	}
	command
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::null());
	// SAFETY: the fork child only invokes async-signal-safe setrlimit before exec;
	// no shared state or graph parsing runs here. The helper's native graph cache is bounded too.
	unsafe {
		command.pre_exec(|| {
			let limit = libc::rlimit {
				rlim_cur: MAX_HELPER_ADDRESS_SPACE,
				rlim_max: MAX_HELPER_ADDRESS_SPACE,
			};
			if libc::setrlimit(libc::RLIMIT_AS, &limit) == 0 {
				Ok(())
			} else {
				Err(std::io::Error::last_os_error())
			}
		});
	}
	let mut child = command.spawn().map_err(|_| UNAVAILABLE)?;
	let stdout = child.stdout.take().ok_or(UNAVAILABLE)?;
	let reader = std::thread::Builder::new()
		.name("screen-audio-graph-read".into())
		.spawn(move || {
			let mut bytes = Vec::new();
			stdout
				.take((MAX_GRAPH_BYTES + 1) as u64)
				.read_to_end(&mut bytes)
				.map(|_| bytes)
		});
	let Ok(reader) = reader else {
		let _ = child.kill();
		let _ = child.wait();
		return Err(UNAVAILABLE);
	};
	let started = Instant::now();
	let success = loop {
		if stop.load(Ordering::Acquire) || started.elapsed() >= timeout {
			let _ = child.kill();
			let _ = child.wait();
			break false;
		}
		match child.try_wait() {
			Ok(Some(status)) => break status.success(),
			Ok(None) => std::thread::sleep(Duration::from_millis(10)),
			Err(_) => {
				let _ = child.kill();
				let _ = child.wait();
				break false;
			}
		}
	};
	let bytes = reader
		.join()
		.map_err(|_| UNAVAILABLE)?
		.map_err(|_| UNAVAILABLE)?;
	if !success {
		return Err(UNAVAILABLE);
	}
	parse_outputs(&bytes, own)
}

fn parse_outputs(bytes: &[u8], own: &OwnApplication) -> Result<Vec<u64>, &'static str> {
	if bytes.len() > MAX_GRAPH_BYTES {
		return Err(UNAVAILABLE);
	}
	let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| UNAVAILABLE)?;
	let objects = value
		.as_array()
		.filter(|v| v.len() <= MAX_GRAPH_OBJECTS)
		.ok_or(UNAVAILABLE)?;
	let mut outputs = Vec::with_capacity(MAX_INPUTS);
	for object in objects {
		if object["type"] != "PipeWire:Interface:Node" {
			continue;
		}
		let props = &object["info"]["props"];
		let client = objects.iter().find(|candidate| {
			candidate["type"] == "PipeWire:Interface:Client"
				&& integer(&candidate["id"])
					.is_some_and(|id| Some(id) == integer(&props["client.id"]))
		});
		let get = |key: &str| {
			props[key]
				.as_str()
				.or_else(|| client.and_then(|client| client["info"]["props"][key].as_str()))
		};
		// PipeWire's JACK client-node has no media.class by default and can expose both
		// directions. Only its playback ports are eligible; native device/source classes stay out.
		if get("client.api") == Some("jack")
			&& !integer(&object["info"]["n-output-ports"]).is_some_and(|ports| ports > 0)
		{
			continue;
		}
		let serial = integer(&props["object.serial"]).map(|v| v.to_string());
		let pid = integer(&props["application.process.id"])
			.or_else(|| {
				client
					.and_then(|client| integer(&client["info"]["props"]["application.process.id"]))
			})
			.map(|v| v.to_string());
		if let Some(serial) = allowed(
			own,
			get("media.class"),
			serial.as_deref(),
			get("client.api"),
			pid.as_deref(),
			get("application.process.binary"),
			get("application.name"),
			get("application.id"),
			get("flatpak.app-id"),
		) {
			if outputs.contains(&serial) {
				continue;
			}
			if outputs.len() == MAX_INPUTS {
				return Err(UNAVAILABLE);
			}
			outputs.push(serial);
		}
	}
	Ok(outputs)
}

fn integer(value: &serde_json::Value) -> Option<u64> {
	value.as_u64().or_else(|| {
		value
			.as_str()
			.filter(|v| v.len() <= 20)
			.and_then(|v| v.parse().ok())
	})
}

pub(super) struct Monitor {
	discovery: Discovery,
	outputs: Vec<u64>,
	captures: Vec<Capture>,
}

impl Monitor {
	pub(super) fn new(own: OwnApplication) -> Option<Self> {
		gst::init().ok()?;
		Some(Self {
			discovery: Discovery::start(own)?,
			outputs: Vec::new(),
			captures: Vec::new(),
		})
	}

	pub(super) fn clear(&mut self) {
		self.captures.clear();
	}

	pub(super) fn refresh(&mut self, pulse: &[super::Input], epoch: u64) -> bool {
		let mut available = true;
		match self.discovery.receive.try_recv() {
			Ok(outputs) => {
				available = outputs.is_ok();
				self.outputs = outputs.unwrap_or_default();
			}
			Err(mpsc::TryRecvError::Disconnected) => {
				self.outputs.clear();
				available = false;
			}
			Err(mpsc::TryRecvError::Empty) => {}
		}
		let mut outputs = Vec::with_capacity(MAX_INPUTS);
		for &serial in &self.outputs {
			if pulse.iter().any(|input| input.serial == Some(serial)) {
				continue;
			}
			if outputs.len() == MAX_INPUTS.saturating_sub(pulse.len()) {
				break;
			}
			outputs.push(serial);
		}
		self.captures
			.retain(|capture| outputs.contains(&capture.serial));
		for serial in outputs {
			if !self.captures.iter().any(|capture| capture.serial == serial) {
				match Capture::new(serial, epoch) {
					Ok(capture) => self.captures.push(capture),
					Err(_) => available = false,
				}
			}
		}
		available
	}

	pub(super) fn active(&self) -> bool {
		!self.captures.is_empty()
	}

	pub(super) fn inputs(&self) -> usize {
		self.captures.len()
	}

	pub(super) fn mix(
		&mut self,
		output: &mut [f32; FRAME_SAMPLES],
		epoch: u64,
		stalled: bool,
	) -> usize {
		let before = self.captures.len();
		self.captures.retain_mut(|capture| {
			if stalled {
				capture.pending.samples.clear();
			}
			if capture.read().is_err() {
				return false;
			}
			capture.pending.mix(output, epoch);
			true
		});
		before - self.captures.len()
	}
}

impl Drop for Monitor {
	fn drop(&mut self) {
		self.clear();
	}
}

#[allow(clippy::too_many_arguments)] // Exact native identity fields; no unbounded property map is retained.
fn allowed(
	own: &OwnApplication,
	class: Option<&str>,
	serial: Option<&str>,
	api: Option<&str>,
	pid: Option<&str>,
	binary: Option<&str>,
	name: Option<&str>,
	app_id: Option<&str>,
	flatpak: Option<&str>,
) -> Option<u64> {
	let output = class == Some("Stream/Output/Audio")
		|| (api == Some("jack") && matches!(class, None | Some("Audio/Duplex")));
	if !output
		|| api == Some("pipewire-pulse")
		|| flatpak.is_some_and(|v| v.eq_ignore_ascii_case("cz.viceverse.serein"))
		|| !own.allows(pid, binary, name, app_id)
	{
		return None;
	}
	serial
		.filter(|v| v.len() <= 20)?
		.parse::<u64>()
		.ok()
		.filter(|v| *v != 0)
}

struct Capture {
	serial: u64,
	pipeline: gst::Pipeline,
	sink: app::AppSink,
	pending: Pending,
	failed: Arc<AtomicBool>,
}

impl Capture {
	fn new(serial: u64, epoch: u64) -> Result<Self, &'static str> {
		let source = gst::ElementFactory::make("pipewiresrc")
			.build()
			.map_err(|_| UNAVAILABLE)?;
		if source.find_property("target-object").is_none()
			|| source.find_property("max-buffers").is_none()
		{
			return Err(UNAVAILABLE);
		}
		// An immutable serial cannot silently retarget a recycled node ID to this client's audio.
		source.set_property("target-object", serial.to_string());
		source.set_property("max-buffers", 2i32);
		source.set_property(
			"stream-properties",
			gst::Structure::builder("properties")
				.field("node.passive", true)
				.field("node.dont-reconnect", true)
				.build(),
		);
		Self::from_source(source, serial, epoch)
	}

	fn from_source(source: gst::Element, serial: u64, epoch: u64) -> Result<Self, &'static str> {
		let convert = gst::ElementFactory::make("audioconvert")
			.build()
			.map_err(|_| UNAVAILABLE)?;
		let resample = gst::ElementFactory::make("audioresample")
			.build()
			.map_err(|_| UNAVAILABLE)?;
		let sink = app::AppSink::builder()
			.caps(
				&gst::Caps::builder("audio/x-raw")
					.field("format", "F32LE")
					.field("layout", "interleaved")
					.field("channels", 2i32)
					.field("rate", 48_000i32)
					.build(),
			)
			.build();
		sink.set_sync(false);
		sink.set_max_buffers(2);
		if sink.find_property("max-bytes").is_none() {
			return Err(UNAVAILABLE);
		}
		sink.set_property("max-bytes", (MAX_AUDIO_SAMPLES * 4 * 2) as u64);
		sink.set_drop(true);
		let pipeline = gst::Pipeline::new();
		let elements = [&source, &convert, &resample, sink.upcast_ref()];
		pipeline.add_many(elements).map_err(|_| UNAVAILABLE)?;
		gst::Element::link_many(elements).map_err(|_| UNAVAILABLE)?;
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
		let capture = Self {
			serial,
			pipeline,
			sink,
			pending: Pending::new(epoch),
			failed,
		};
		capture
			.pipeline
			.set_state(gst::State::Playing)
			.map_err(|_| UNAVAILABLE)?;
		Ok(capture)
	}

	fn read(&mut self) -> Result<(), &'static str> {
		if self.failed.load(Ordering::Acquire) {
			return Err(UNAVAILABLE);
		}
		// Two queued buffers at most; processing cannot monopolize the shared audio worker.
		for _ in 0..2 {
			let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::ZERO) else {
				break;
			};
			let buffer = sample.buffer().ok_or(UNAVAILABLE)?;
			if buffer.size() > MAX_AUDIO_SAMPLES * 4 {
				return Err(UNAVAILABLE);
			}
			let bytes = buffer.map_readable().map_err(|_| UNAVAILABLE)?;
			self.pending.push(bytes.as_slice())?;
		}
		Ok(())
	}
}

impl Drop for Capture {
	fn drop(&mut self) {
		let _ = self.pipeline.set_state(gst::State::Null);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn collector_bounds_fixture_output_deadline_and_cancellation_without_a_server() {
		let own = OwnApplication {
			pid: 42,
			binary: "serein".into(),
		};
		let stop = AtomicBool::new(false);
		let fixture = |script: &str| {
			let mut command = Command::new("/bin/sh");
			command.args(["-c", script]);
			command
		};
		assert_eq!(
			collect_graph(fixture("printf '[]'"), &own, &stop, GRAPH_TIMEOUT),
			Ok(vec![])
		);
		assert!(
			collect_graph(
				fixture("exec head -c 1048577 /dev/zero"),
				&own,
				&stop,
				GRAPH_TIMEOUT
			)
			.is_err()
		);
		let started = Instant::now();
		assert!(
			collect_graph(
				fixture("exec sleep 60"),
				&own,
				&stop,
				Duration::from_millis(40)
			)
			.is_err()
		);
		assert!(started.elapsed() < Duration::from_secs(2));
		stop.store(true, Ordering::Release);
		assert!(
			collect_graph(
				Command::new("serein-fixture-missing-helper"),
				&own,
				&stop,
				GRAPH_TIMEOUT
			)
			.is_err()
		);
	}

	#[test]
	#[ignore = "run only in a runtime without pipewiresrc; never opens a native stream"]
	fn absent_pipewire_plugin_reports_attachment_failure() {
		gst::init().unwrap();
		assert!(gst::ElementFactory::find("pipewiresrc").is_none());
		assert!(Capture::new(91, 7).is_err());
	}
	#[test]
	#[ignore = "requires installed GStreamer audiotestsrc; synthetic PCM only, no audio device"]
	fn synthetic_pipeline_mixes_stereo_and_discards_an_old_capture_generation() {
		gst::init().unwrap();
		let source = gst::ElementFactory::make("audiotestsrc")
			.property("is-live", true)
			.property("samplesperbuffer", 480i32)
			.property("num-buffers", 20i32)
			.build()
			.unwrap();
		let mut capture = Capture::from_source(source, 91, 7).unwrap();
		let started = Instant::now();
		while capture.pending.samples.len() < FRAME_SAMPLES
			&& started.elapsed() < Duration::from_secs(2)
		{
			capture.read().unwrap();
			std::thread::sleep(Duration::from_millis(10));
		}
		assert!(capture.pending.samples.len() >= FRAME_SAMPLES);
		let mut output = [0.0; FRAME_SAMPLES];
		capture.pending.mix(&mut output, 7);
		assert!(output.iter().any(|sample| sample.abs() > 0.01));
		assert!(
			output
				.chunks_exact(2)
				.all(|pair| (pair[0] - pair[1]).abs() < 0.001)
		);
		capture.pending.mix(&mut [0.0; FRAME_SAMPLES], 8);
		assert!(capture.pending.samples.is_empty());
	}
	#[test]
	fn graph_uses_client_identity_and_rejects_microphones_self_and_oversized_snapshots() {
		let own = OwnApplication {
			pid: 42,
			binary: "serein".into(),
		};
		let graph = serde_json::json!([
			{"id": 7, "type": "PipeWire:Interface:Client", "info": {"props": {"application.process.id": 123, "application.process.binary": "ardour", "client.api": "jack"}}},
			{"type": "PipeWire:Interface:Node", "info": {"n-output-ports": 2, "props": {"client.id": "7", "object.serial": 91}}},
			{"type": "PipeWire:Interface:Node", "info": {"props": {"client.id": 7, "object.serial": "92", "media.class": "Audio/Source"}}},
			{"type": "PipeWire:Interface:Node", "info": {"n-output-ports": 2, "props": {"client.id": 7, "object.serial": 93, "media.class": "Stream/Output/Audio", "application.process.id": 42}}},
			{"type": "PipeWire:Interface:Node", "info": {"props": {"client.id": 7, "object.serial": 94, "media.class": "Stream/Output/Audio", "client.api": "pipewire-pulse"}}}
		]);
		assert_eq!(
			parse_outputs(&serde_json::to_vec(&graph).unwrap(), &own),
			Ok(vec![91])
		);
		assert!(parse_outputs(&vec![0; MAX_GRAPH_BYTES + 1], &own).is_err());
		assert!(
			parse_outputs(
				&serde_json::to_vec(&vec![serde_json::Value::Null; MAX_GRAPH_OBJECTS + 1]).unwrap(),
				&own
			)
			.is_err()
		);
		assert!(parse_outputs(b"not JSON", &own).is_err());
	}
}
