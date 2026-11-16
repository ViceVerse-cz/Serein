//! Process-local driver discovery and explicitly requested encoder tests.
use model::voice_settings::{
	DriverCapabilities, EncoderPreset, EncoderPresets, HardwareBackend, HardwareSupport,
	ProbeResult, VideoCapabilities, VideoCodec,
};
use std::{
	ffi::OsString,
	io::{Read, Write},
	path::Path,
	process::{Child, Command, Stdio},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
	thread::JoinHandle,
	time::{Duration, Instant},
};

const QUERY_ARGUMENT: &str = "--query-video-codec";
const TEST_ARGUMENT: &str = "--test-video-encoder";
const ADAPTER_ARGUMENT: &str = "--adapter";
const HELPER_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_HELPER_ERRORS: usize = 4096;
#[cfg(unix)]
use std::os::fd::AsRawFd as PipeHandle;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as PipeHandle;
const UNKNOWN: HardwareSupport = HardwareSupport {
	camera: ProbeResult::Failed,
	screen: ProbeResult::Failed,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
	Discover,
	Test(VideoCodec, EncoderPreset),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Helper {
	Query,
	Test,
}

type HelperRequest = (
	Helper,
	HardwareBackend,
	VideoCodec,
	Option<model::VideoAdapter>,
	EncoderPreset,
);

fn request(mut args: impl Iterator<Item = OsString>) -> Option<Result<HelperRequest, ()>> {
	let helper = match args.next()?.to_str()? {
		QUERY_ARGUMENT => Helper::Query,
		TEST_ARGUMENT => Helper::Test,
		_ => return None,
	};
	let backend = args.next().and_then(|arg| {
		HardwareBackend::ALL
			.into_iter()
			.find(|backend| Some(backend.key()) == arg.to_str())
	});
	let codec = args.next().and_then(|arg| {
		VideoCodec::ALL
			.into_iter()
			.find(|codec| Some(codec.key()) == arg.to_str())
	});
	let mut adapter = None;
	let mut preset = None;
	for _ in 0..=2 {
		let Some(flag) = args.next() else {
			return Some(match (backend, codec) {
				(Some(backend), Some(codec)) => {
					Ok((helper, backend, codec, adapter, preset.unwrap_or_default()))
				}
				_ => Err(()),
			});
		};
		if flag == ADAPTER_ARGUMENT && adapter.is_none() {
			adapter = args
				.next()
				.and_then(|key| model::VideoAdapter::from_helper_key(key.to_str()?));
			if adapter.is_none() {
				return Some(Err(()));
			}
		} else if flag == "--preset" && preset.is_none() && helper == Helper::Test {
			preset = args
				.next()
				.and_then(|key| key.to_str()?.parse::<usize>().ok())
				.and_then(|index| EncoderPreset::ALL.get(index).copied());
			if preset.is_none() {
				return Some(Err(()));
			}
		} else {
			return Some(Err(()));
		}
	}
	Some(Err(()))
}

/// Exit before GUI, credentials, network clients or capture discovery initialize.
pub fn probe_command() -> Option<i32> {
	request(std::env::args_os().skip(1)).map(|request| match request {
		Ok((helper, backend, codec, adapter, preset))
			if discord_voice::video_capabilities::backends().contains(&backend) =>
		{
			match helper {
				Helper::Query => {
					let ordinary = discord_voice::video_capabilities::query_on_adapter(
						backend, codec, adapter,
					);
					let presets = adapter
						.filter(|_| ordinary == ProbeResult::Available)
						.and_then(|adapter| {
							discord_voice::video_capabilities::query_presets(
								backend, codec, adapter,
							)
						});
					println!(
						"{}",
						serde_json::json!({"presets":presets.map(EncoderPresets::bits)})
					);
					if ordinary == ProbeResult::RuntimeUnavailable {
						10
					} else if ordinary == ProbeResult::Unavailable {
						0
					} else if ordinary != ProbeResult::Available {
						2
					} else {
						let high = adapter.map_or(ProbeResult::Failed, |adapter| {
							discord_voice::video_capabilities::query_ten_bit(
								backend, codec, adapter,
							)
						});
						match high {
							ProbeResult::Available => 5,
							ProbeResult::Unavailable => 1,
							_ => 9,
						}
					}
				}
				Helper::Test => {
					let support = discord_voice::video_capabilities::probe_on_adapter_with_preset(
						backend, codec, adapter, preset,
					);
					println!("{{\"presets\":null}}");
					i32::from(support.camera == ProbeResult::Available)
						| (i32::from(support.screen == ProbeResult::Available) << 1)
				}
			}
		}
		_ => 64,
	})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exit {
	Code(Option<i32>),
	Failed,
	TimedOut,
}

fn decode_query(exit: Exit) -> ProbeResult {
	match exit {
		Exit::Code(Some(1 | 5 | 9)) => ProbeResult::Available,
		Exit::Code(Some(0)) => ProbeResult::Unavailable,
		Exit::Code(Some(10)) => ProbeResult::RuntimeUnavailable,
		Exit::TimedOut => ProbeResult::TimedOut,
		_ => ProbeResult::Failed,
	}
}

fn decode_ten_bit(exit: Exit) -> ProbeResult {
	match exit {
		Exit::Code(Some(5)) => ProbeResult::Available,
		Exit::Code(Some(0 | 1)) => ProbeResult::Unavailable,
		Exit::TimedOut => ProbeResult::TimedOut,
		_ => ProbeResult::Failed,
	}
}

fn decode_test(exit: Exit) -> HardwareSupport {
	match exit {
		Exit::Code(Some(code @ 0..=3)) => {
			let available = |bit| {
				if code & bit != 0 {
					ProbeResult::Available
				} else {
					ProbeResult::Unavailable
				}
			};
			HardwareSupport {
				camera: available(1),
				screen: available(2),
			}
		}
		Exit::TimedOut => HardwareSupport {
			camera: ProbeResult::TimedOut,
			screen: ProbeResult::TimedOut,
		},
		_ => UNKNOWN,
	}
}

struct ProbeChild(Child);
impl Drop for ProbeChild {
	fn drop(&mut self) {
		// Reap on success, timeout, cancellation and every error path.
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

fn decode_presets(bytes: &[u8]) -> Option<EncoderPresets> {
	#[derive(serde::Deserialize)]
	struct Report {
		presets: Option<u32>,
	}
	if bytes.len() > 128 {
		return None;
	}
	serde_json::from_slice::<Report>(bytes)
		.ok()?
		.presets
		.map(EncoderPresets::from_bits)
}

#[allow(unsafe_code)] // Nonblocking reads from owned stdout/stderr pipes.
fn helper_bytes(pipe: &mut (impl Read + PipeHandle), bytes: &mut [u8]) -> Option<usize> {
	// A driver may leave an inherited pipe handle open after the owned helper
	// exits. Never let reading its tiny report bypass cancellation/deadlines.
	#[cfg(unix)]
	{
		let fd = pipe.as_raw_fd();
		// SAFETY: pipe owns this live descriptor; only this thread reads it.
		let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
		if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
			return None;
		}
	}
	#[cfg(windows)]
	{
		#[link(name = "kernel32")]
		unsafe extern "system" {
			fn PeekNamedPipe(
				handle: *mut std::ffi::c_void,
				buffer: *mut std::ffi::c_void,
				size: u32,
				read: *mut u32,
				available: *mut u32,
				remaining: *mut u32,
			) -> i32;
		}
		let mut available = 0;
		// SAFETY: a live owned pipe, no destination buffer, one valid output.
		if unsafe {
			PeekNamedPipe(
				pipe.as_raw_handle(),
				std::ptr::null_mut(),
				0,
				std::ptr::null_mut(),
				&mut available,
				std::ptr::null_mut(),
			)
		} == 0 || available == 0
		{
			return None;
		}
	}
	pipe.read(bytes).ok()
}

fn helper_report(stdout: &mut std::process::ChildStdout) -> Option<EncoderPresets> {
	let mut bytes = [0u8; 129];
	let length = helper_bytes(stdout, &mut bytes)?;
	decode_presets(&bytes[..length])
}

#[derive(Default)]
struct HelperErrors {
	emitted: usize,
	truncated: bool,
}

impl HelperErrors {
	fn drain(&mut self, pipe: &mut std::process::ChildStderr, output: &mut impl Write) {
		let mut bytes = [0u8; MAX_HELPER_ERRORS];
		// Keep draining after the print limit so driver output cannot fill its
		// pipe, while bounding work between cancellation/deadline checks.
		for _ in 0..4 {
			let Some(length) = helper_bytes(pipe, &mut bytes).filter(|length| *length != 0) else {
				break;
			};
			let kept = length.min(MAX_HELPER_ERRORS - self.emitted);
			let _ = output.write_all(&bytes[..kept]);
			self.emitted += kept;
			if kept < length && !self.truncated {
				let _ = output.write_all(b"\n[NuSerein] Further video driver output omitted.\n");
				self.truncated = true;
			}
		}
	}
}

fn wait_for_helper(
	command: &mut Command,
	stop: &AtomicBool,
	timeout: Duration,
	presets: &mut Option<EncoderPresets>,
) -> Exit {
	if stop.load(Ordering::Acquire) {
		return Exit::Failed;
	}
	command
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());
	let Ok(child) = command.spawn() else {
		return Exit::Failed;
	};
	let mut child = ProbeChild(child);
	let mut errors = HelperErrors::default();
	let started = Instant::now();
	loop {
		if stop.load(Ordering::Acquire) {
			return Exit::Failed;
		}
		if let Some(pipe) = &mut child.0.stderr {
			errors.drain(pipe, &mut std::io::stderr());
		}
		match child.0.try_wait() {
			Ok(Some(status)) => {
				if let Some(pipe) = &mut child.0.stderr {
					errors.drain(pipe, &mut std::io::stderr());
				}
				*presets = child.0.stdout.as_mut().and_then(helper_report);
				return Exit::Code(status.code());
			}
			Err(_) => return Exit::Failed,
			Ok(None) => {}
		}
		if started.elapsed() >= timeout {
			return Exit::TimedOut;
		}
		std::thread::sleep(POLL_INTERVAL);
	}
}

fn initial_report() -> DriverCapabilities {
	let mut report = DriverCapabilities {
		support: [[ProbeResult::Unavailable; 4]; 3],
		ten_bit: [[ProbeResult::Unavailable; 4]; 3],
		quality_presets: Default::default(),
	};
	for &backend in discord_voice::video_capabilities::backends() {
		for codec in VideoCodec::ALL {
			report.set(backend, codec, ProbeResult::Pending);
			if codec != VideoCodec::H264 {
				report.ten_bit[codec.index()][backend.index()] = ProbeResult::Pending;
			}
		}
	}
	report
}

fn initial_tests() -> VideoCapabilities {
	let unavailable = HardwareSupport {
		camera: ProbeResult::Unavailable,
		screen: ProbeResult::Unavailable,
	};
	let mut report = VideoCapabilities {
		support: [[unavailable; 4]; 3],
	};
	for codec in VideoCodec::ALL {
		reset_test(&mut report, codec);
	}
	report
}

fn reset_test(report: &mut VideoCapabilities, codec: VideoCodec) {
	for &backend in discord_voice::video_capabilities::backends() {
		report.set(backend, codec, HardwareSupport::default());
	}
}

fn finish_pending(report: &mut DriverCapabilities) {
	for results in report.support.iter_mut().chain(report.ten_bit.iter_mut()) {
		for result in results {
			if *result == ProbeResult::Pending {
				*result = ProbeResult::Failed;
			}
		}
	}
}

fn finish_test(report: &mut VideoCapabilities, codec: VideoCodec) {
	for support in &mut report.support[codec.index()] {
		for result in [&mut support.camera, &mut support.screen] {
			if *result == ProbeResult::Pending {
				*result = ProbeResult::Failed;
			}
		}
	}
}

enum Update {
	Driver(
		HardwareBackend,
		VideoCodec,
		ProbeResult,
		ProbeResult,
		Option<EncoderPresets>,
	),
	Test(HardwareBackend, VideoCodec, HardwareSupport),
	Finished,
}

fn scan(
	operation: Operation,
	adapter: Option<model::VideoAdapter>,
	executable: &Path,
	stop: &AtomicBool,
	send: &mpsc::SyncSender<Update>,
	wake: &eframe::egui::Context,
) {
	for &backend in discord_voice::video_capabilities::backends() {
		for codec in VideoCodec::ALL {
			if let Operation::Test(selected, _) = operation
				&& codec != selected
			{
				continue;
			}
			if stop.load(Ordering::Acquire) {
				return;
			}
			let argument = match operation {
				Operation::Discover => QUERY_ARGUMENT,
				Operation::Test(..) => TEST_ARGUMENT,
			};
			let mut command = Command::new(executable);
			command.args([argument, backend.key(), codec.key()]);
			if let Operation::Test(_, preset) = operation {
				command.args(["--preset", &(preset as i32).to_string()]);
			}
			if let Some(adapter) = adapter {
				command.args([ADAPTER_ARGUMENT, &adapter.helper_key()]);
			}
			let mut presets = None;
			let exit = wait_for_helper(&mut command, stop, HELPER_TIMEOUT, &mut presets);
			if stop.load(Ordering::Acquire) {
				return;
			}
			let failed = match operation {
				Operation::Discover => !matches!(exit, Exit::Code(Some(0 | 1 | 5 | 9 | 10))),
				Operation::Test(..) => !matches!(exit, Exit::Code(Some(0..=3))),
			};
			if failed {
				eprintln!(
					"[NuSerein] Video probe {} {}: {exit:?}; selected adapter: {adapter:?}",
					backend.key(),
					codec.key(),
				);
			}
			let update = match operation {
				Operation::Discover => Update::Driver(
					backend,
					codec,
					decode_query(exit),
					decode_ten_bit(exit),
					presets.map(|presets| presets.for_backend(backend)),
				),
				Operation::Test(..) => Update::Test(backend, codec, decode_test(exit)),
			};
			if send.try_send(update).is_err() {
				return;
			}
			wake.request_repaint();
		}
	}
	let _ = send.try_send(Update::Finished);
	wake.request_repaint();
}

fn launch(
	operation: Operation,
	adapter: Option<model::VideoAdapter>,
	stop: Arc<AtomicBool>,
	send: mpsc::SyncSender<Update>,
	wake: eframe::egui::Context,
) -> Option<JoinHandle<()>> {
	let executable = std::env::current_exe().ok()?;
	std::thread::Builder::new()
		.name("video-capabilities".into())
		.spawn(move || scan(operation, adapter, &executable, &stop, &send, &wake))
		.ok()
}

#[derive(Default)]
pub struct Detector {
	receive: Option<mpsc::Receiver<Update>>,
	thread: Option<JoinHandle<()>>,
	stop: Option<Arc<AtomicBool>>,
	operation: Option<Operation>,
}

impl Detector {
	pub fn cancel(&self) {
		if let Some(stop) = &self.stop {
			stop.store(true, Ordering::Release);
		}
	}

	fn discard_cancelled(&mut self, ui: &mut ui::MessagingUi) {
		// Discard the old channel before starting another operation. Its queued
		// partial results must not leak into a replacement report.
		self.receive = None;
		match self.operation.take() {
			Some(Operation::Discover) => {
				ui.video_capabilities = None;
				ui.video_capabilities_loading = false;
			}
			Some(Operation::Test(codec, _)) => {
				if let Some(report) = &mut ui.video_encoder_tests {
					finish_test(report, codec);
				}
				ui.video_encoder_tests_loading = false;
			}
			None => {}
		}
	}

	#[cfg(test)]
	pub fn poll(&mut self, demo: bool, ui: &mut ui::MessagingUi, ctx: &eframe::egui::Context) {
		self.poll_on_adapter(demo, ui, ctx, None);
	}

	pub fn poll_on_adapter(
		&mut self,
		demo: bool,
		ui: &mut ui::MessagingUi,
		ctx: &eframe::egui::Context,
		adapter: Option<model::VideoAdapter>,
	) {
		self.poll_with(demo, ui, ctx, |operation, stop, send, wake| {
			launch(operation, adapter, stop, send, wake)
		});
	}

	fn poll_with(
		&mut self,
		demo: bool,
		ui: &mut ui::MessagingUi,
		ctx: &eframe::egui::Context,
		mut start: impl FnMut(
			Operation,
			Arc<AtomicBool>,
			mpsc::SyncSender<Update>,
			eframe::egui::Context,
		) -> Option<JoinHandle<()>>,
	) {
		let visible = !demo && ui.voice_settings_open() && ui.video_encoding_advanced_visible;
		if !visible {
			self.cancel();
		}
		if self
			.stop
			.as_ref()
			.is_some_and(|stop| stop.load(Ordering::Acquire))
		{
			self.discard_cancelled(ui);
		}
		if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
			self.stop = None;
		}
		let mut finished = false;
		if let Some(receive) = &self.receive {
			// The fixed matrix fits twelve updates and a terminal message.
			for _ in 0..13 {
				match receive.try_recv() {
					Ok(Update::Driver(backend, codec, support, ten_bit, presets)) => {
						if self.operation == Some(Operation::Discover)
							&& let Some(report) = &mut ui.video_capabilities
						{
							report.set(backend, codec, support);
							report.ten_bit[codec.index()][backend.index()] = ten_bit;
							report.quality_presets[codec.index()][backend.index()] =
								if support == ProbeResult::Available {
									presets
								} else {
									None
								};
						}
					}
					Ok(Update::Test(backend, codec, support)) => {
						if matches!(self.operation, Some(Operation::Test(selected, _)) if selected == codec)
							&& let Some(report) = &mut ui.video_encoder_tests
						{
							report.set(backend, codec, support);
						}
					}
					Ok(Update::Finished) | Err(mpsc::TryRecvError::Disconnected) => {
						finished = true;
						break;
					}
					Err(mpsc::TryRecvError::Empty) => break,
				}
			}
		}
		if finished {
			self.receive = None;
			match self.operation.take() {
				Some(Operation::Discover) => {
					ui.video_capabilities_loading = false;
					if let Some(report) = &mut ui.video_capabilities {
						finish_pending(report);
					}
				}
				Some(Operation::Test(codec, _)) => {
					ui.video_encoder_tests_loading = false;
					if let Some(report) = &mut ui.video_encoder_tests {
						finish_test(report, codec);
					}
				}
				None => {}
			}
		}
		if demo {
			// Native offline previews supply explicit fixtures and never query a driver.
			if ui.video_capabilities.is_none() {
				let mut report = initial_report();
				finish_pending(&mut report);
				ui.video_capabilities = Some(report);
			}
			ui.video_capabilities_refresh = false;
			ui.video_capabilities_loading = false;
			ui.video_encoder_tests_request = None;
			ui.video_encoder_tests_loading = false;
			return;
		}
		if !visible {
			ui.video_capabilities_refresh = false;
			ui.video_encoder_tests_request = None;
			return;
		}
		if self.thread.is_some() || self.receive.is_some() {
			return;
		}
		let operation = if ui.video_capabilities_refresh || ui.video_capabilities.is_none() {
			ui.video_capabilities_refresh = false;
			ui.video_capabilities = Some(initial_report());
			ui.video_capabilities_loading = true;
			Operation::Discover
		} else if let Some(codec) = ui.video_encoder_tests_request.take() {
			let report = ui.video_encoder_tests.get_or_insert_with(initial_tests);
			reset_test(report, codec);
			ui.video_encoder_tests_codec = Some(codec);
			ui.video_encoder_tests_loading = true;
			let preset = ui
				.video_encoder_tests_preset
				.unwrap_or(ui.video_settings.quality);
			ui.video_encoder_test_presets[codec.index()] = Some(preset);
			Operation::Test(codec, preset)
		} else {
			return;
		};
		let stop = Arc::new(AtomicBool::new(false));
		let (send, receive) = mpsc::sync_channel(13);
		self.operation = Some(operation);
		if let Some(thread) = start(operation, stop.clone(), send, ctx.clone()) {
			self.receive = Some(receive);
			self.thread = Some(thread);
			self.stop = Some(stop);
		} else {
			match operation {
				Operation::Discover => {
					finish_pending(ui.video_capabilities.as_mut().unwrap());
					ui.video_capabilities_loading = false;
				}
				Operation::Test(codec, _) => {
					finish_test(ui.video_encoder_tests.as_mut().unwrap(), codec);
					ui.video_encoder_tests_loading = false;
				}
			}
			self.operation = None;
		}
	}
}

impl Drop for Detector {
	fn drop(&mut self) {
		self.cancel();
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn settings() -> ui::MessagingUi {
		let mut ui = ui::MessagingUi::default();
		ui.preview_settings("voice");
		ui.video_encoding_advanced_visible = true;
		ui
	}

	#[test]
	fn offline_demo_never_queries_or_encodes_and_preserves_fixtures() {
		let mut detector = Detector::default();
		let mut ui = settings();
		let ctx = eframe::egui::Context::default();
		ui.video_capabilities_refresh = true;
		ui.video_encoder_tests_request = Some(VideoCodec::Av1);
		detector.poll_with(true, &mut ui, &ctx, |_, _, _, _| {
			panic!("demo cannot start a driver")
		});
		assert!(ui.video_capabilities.is_some());
		assert!(!ui.video_capabilities_loading && !ui.video_capabilities_refresh);
		assert!(ui.video_encoder_tests.is_none() && ui.video_encoder_tests_request.is_none());
		let fixture = DriverCapabilities {
			support: [[ProbeResult::Available; 4]; 3],
			ten_bit: [[ProbeResult::Pending; 4]; 3],
			quality_presets: Default::default(),
		};
		ui.video_capabilities = Some(fixture);
		detector.poll(true, &mut ui, &ctx);
		assert_eq!(ui.video_capabilities, Some(fixture));
	}

	#[test]
	fn reopening_cancelled_detection_restarts_and_discards_stale_results() {
		let ctx = eframe::egui::Context::default();
		let mut ui = settings();
		ui.video_capabilities = Some(initial_report());
		ui.video_capabilities_loading = true;
		let backend = discord_voice::video_capabilities::backends()[0];
		let (send, receive) = mpsc::sync_channel(13);
		send.try_send(Update::Driver(
			backend,
			VideoCodec::Av1,
			ProbeResult::Available,
			ProbeResult::Unavailable,
			None,
		))
		.unwrap();
		let stop = Arc::new(AtomicBool::new(false));
		let mut detector = Detector {
			receive: Some(receive),
			stop: Some(stop.clone()),
			operation: Some(Operation::Discover),
			thread: None,
		};
		ui.preview_settings("appearance");
		detector.poll_with(false, &mut ui, &ctx, |_, _, _, _| {
			panic!("closed settings cannot launch")
		});
		assert!(stop.load(Ordering::Acquire));
		assert!(ui.video_capabilities.is_none() && !ui.video_capabilities_loading);
		assert!(send.try_send(Update::Finished).is_err());
		ui.preview_settings("voice");
		let mut starts = Vec::new();
		detector.poll_with(false, &mut ui, &ctx, |operation, _, _, _| {
			starts.push(operation);
			None
		});
		assert_eq!(starts, [Operation::Discover]);
		assert_eq!(
			ui.video_capabilities.unwrap().codec(VideoCodec::Av1)[backend.index()],
			ProbeResult::Failed
		);
	}

	#[test]
	fn cancellation_while_settings_remain_visible_also_restarts_detection() {
		let ctx = eframe::egui::Context::default();
		let mut ui = settings();
		ui.video_capabilities = Some(initial_report());
		let mut detector = Detector {
			receive: None,
			thread: None,
			stop: Some(Arc::new(AtomicBool::new(false))),
			operation: Some(Operation::Discover),
		};
		detector.cancel();
		let mut starts = Vec::new();
		detector.poll_with(false, &mut ui, &ctx, |operation, _, _, _| {
			starts.push(operation);
			None
		});
		assert_eq!(starts, [Operation::Discover]);
	}

	#[test]
	fn quick_reopen_waits_for_cancelled_worker_before_starting_new_detection() {
		let ctx = eframe::egui::Context::default();
		let mut ui = settings();
		ui.video_capabilities = Some(initial_report());
		ui.video_capabilities_loading = true;
		let stop = Arc::new(AtomicBool::new(false));
		let worker_stop = stop.clone();
		let (release, wait) = mpsc::sync_channel(1);
		let thread = std::thread::spawn(move || {
			let _ = wait.recv_timeout(Duration::from_secs(2));
			assert!(worker_stop.load(Ordering::Acquire));
		});
		let mut detector = Detector {
			receive: None,
			thread: Some(thread),
			stop: Some(stop),
			operation: Some(Operation::Discover),
		};
		ui.preview_settings("appearance");
		detector.poll_with(false, &mut ui, &ctx, |_, _, _, _| panic!("closed panel"));
		ui.preview_settings("voice");
		detector.poll_with(false, &mut ui, &ctx, |_, _, _, _| {
			panic!("old worker is still running")
		});
		assert!(ui.video_capabilities.is_none());
		release.send(()).unwrap();
		let mut starts = Vec::new();
		let deadline = Instant::now() + Duration::from_secs(2);
		while starts.is_empty() && Instant::now() < deadline {
			detector.poll_with(false, &mut ui, &ctx, |operation, _, _, _| {
				starts.push(operation);
				None
			});
			std::thread::yield_now();
		}
		assert_eq!(starts, [Operation::Discover]);
	}

	#[test]
	fn cancelled_explicit_test_never_restarts_and_keeps_driver_results() {
		let ctx = eframe::egui::Context::default();
		let mut ui = settings();
		let driver = DriverCapabilities {
			support: [[ProbeResult::Available; 4]; 3],
			ten_bit: [[ProbeResult::Pending; 4]; 3],
			quality_presets: Default::default(),
		};
		ui.video_capabilities = Some(driver);
		ui.video_encoder_tests = Some(initial_tests());
		ui.video_encoder_tests_loading = true;
		ui.video_encoder_tests_codec = Some(VideoCodec::H265);
		let mut detector = Detector {
			receive: None,
			thread: None,
			stop: Some(Arc::new(AtomicBool::new(false))),
			operation: Some(Operation::Test(VideoCodec::H265, EncoderPreset::Automatic)),
		};

		ui.preview_settings("appearance");
		detector.poll_with(false, &mut ui, &ctx, |_, _, _, _| {
			panic!("closed settings cannot launch")
		});

		detector.poll_with(false, &mut ui, &ctx, |_, _, _, _| {
			panic!("optional test cannot restart automatically")
		});
		assert_eq!(ui.video_capabilities, Some(driver));
		assert!(!ui.video_encoder_tests_loading && ui.video_encoder_tests_request.is_none());
		assert_eq!(
			ui.video_encoder_tests.unwrap().codec(VideoCodec::H265)
				[discord_voice::video_capabilities::backends()[0].index()],
			UNKNOWN
		);
	}

	#[test]
	fn automatic_detection_and_completed_unknowns_never_submit_an_encode_test() {
		let mut detector = Detector::default();
		let ctx = eframe::egui::Context::default();
		let mut ui = settings();
		let mut starts = Vec::new();
		detector.poll_with(false, &mut ui, &ctx, |operation, _, _, _| {
			starts.push(operation);
			None
		});
		detector.poll_with(false, &mut ui, &ctx, |operation, _, _, _| {
			starts.push(operation);
			None
		});
		assert_eq!(starts, [Operation::Discover]);
		ui.video_encoder_tests_request = Some(VideoCodec::Av1);
		detector.poll_with(false, &mut ui, &ctx, |operation, _, _, _| {
			starts.push(operation);
			None
		});
		assert_eq!(
			starts,
			[
				Operation::Discover,
				Operation::Test(VideoCodec::Av1, EncoderPreset::Automatic)
			]
		);
		assert!(ui.video_encoder_tests_request.is_none());
	}

	#[test]
	fn failed_test_does_not_overwrite_driver_or_other_codec_results() {
		let mut detector = Detector::default();
		let ctx = eframe::egui::Context::default();
		let mut ui = settings();
		let driver = DriverCapabilities {
			support: [[ProbeResult::Available; 4]; 3],
			ten_bit: [[ProbeResult::Pending; 4]; 3],
			quality_presets: Default::default(),
		};
		ui.video_capabilities = Some(driver);
		let tested = HardwareSupport {
			camera: ProbeResult::Available,
			screen: ProbeResult::Available,
		};
		let mut report = initial_tests();
		report.set(HardwareBackend::Nvenc, VideoCodec::H264, tested);
		ui.video_encoder_tests = Some(report);
		ui.video_encoder_tests_request = Some(VideoCodec::Av1);
		detector.poll_with(false, &mut ui, &ctx, |operation, _, _, _| {
			assert_eq!(
				operation,
				Operation::Test(VideoCodec::Av1, EncoderPreset::Automatic)
			);
			None
		});
		assert_eq!(ui.video_capabilities, Some(driver));
		assert_eq!(
			ui.video_encoder_tests.unwrap().codec(VideoCodec::H264)[HardwareBackend::Nvenc.index()],
			tested
		);
	}

	#[test]
	fn helper_argument_parser_rejects_capture_and_extra_arguments() {
		let args = |values: &[&str]| {
			values
				.iter()
				.map(OsString::from)
				.collect::<Vec<_>>()
				.into_iter()
		};
		assert!(request(args(&["--demo"])).is_none());
		assert_eq!(
			request(args(&[QUERY_ARGUMENT, "amf", "av1"])),
			Some(Ok((
				Helper::Query,
				HardwareBackend::Amf,
				VideoCodec::Av1,
				None,
				EncoderPreset::Automatic
			)))
		);
		assert_eq!(
			request(args(&[TEST_ARGUMENT, "qsv", "h264"])),
			Some(Ok((
				Helper::Test,
				HardwareBackend::Qsv,
				VideoCodec::H264,
				None,
				EncoderPreset::Automatic
			)))
		);
		for argument in [QUERY_ARGUMENT, TEST_ARGUMENT] {
			for values in [
				vec![argument],
				vec![argument, "software", "h264"],
				vec![argument, "nvenc", "vp9"],
				vec![argument, "qsv", "h264", "extra"],
			] {
				assert_eq!(request(args(&values)), Some(Err(())));
			}
		}
	}

	#[test]
	fn helper_adapter_arguments_preserve_the_running_gpu_and_reject_extra_input() {
		let adapter = model::VideoAdapter {
			vendor_id: 0x8086,
			device_id: 0x56a0,
			identity: model::VideoAdapterIdentity::WindowsLuid(0x1234),
		};
		let key = adapter.helper_key();
		let args = [TEST_ARGUMENT, "qsv", "h265", ADAPTER_ARGUMENT, &key];
		assert_eq!(
			request(args.into_iter().map(OsString::from)),
			Some(Ok((
				Helper::Test,
				HardwareBackend::Qsv,
				VideoCodec::H265,
				Some(adapter),
				EncoderPreset::Automatic
			)))
		);
		for args in [
			vec![QUERY_ARGUMENT, "qsv", "h265", ADAPTER_ARGUMENT],
			vec![QUERY_ARGUMENT, "qsv", "h265", ADAPTER_ARGUMENT, "invalid"],
			vec![
				QUERY_ARGUMENT,
				"qsv",
				"h265",
				ADAPTER_ARGUMENT,
				&key,
				"extra",
			],
		] {
			assert_eq!(request(args.into_iter().map(OsString::from)), Some(Err(())));
		}
	}

	#[test]
	fn explicit_test_helper_accepts_one_bounded_preset_only() {
		let parse = |args: &[&str]| request(args.iter().map(OsString::from));
		assert_eq!(
			parse(&[TEST_ARGUMENT, "qsv", "av1", "--preset", "18"]),
			Some(Ok((
				Helper::Test,
				HardwareBackend::Qsv,
				VideoCodec::Av1,
				None,
				EncoderPreset::QsvVeryFast
			)))
		);
		for args in [
			vec![TEST_ARGUMENT, "qsv", "av1", "--preset", "19"],
			vec![TEST_ARGUMENT, "qsv", "av1", "--preset", "-1"],
			vec![TEST_ARGUMENT, "qsv", "av1", "--preset"],
			vec![
				TEST_ARGUMENT,
				"qsv",
				"av1",
				"--preset",
				"1",
				"--preset",
				"2",
			],
			vec![QUERY_ARGUMENT, "qsv", "av1", "--preset", "1"],
		] {
			assert_eq!(parse(&args), Some(Err(())));
		}
	}

	#[test]
	fn driver_and_encode_statuses_are_separate_and_errors_stay_unknown() {
		assert_eq!(decode_query(Exit::Code(Some(1))), ProbeResult::Available);
		assert_eq!(decode_query(Exit::Code(Some(0))), ProbeResult::Unavailable);
		assert_eq!(
			decode_query(Exit::Code(Some(10))),
			ProbeResult::RuntimeUnavailable
		);
		assert_eq!(decode_query(Exit::Code(Some(2))), ProbeResult::Failed);
		assert_eq!(
			decode_test(Exit::Code(Some(1))),
			HardwareSupport {
				camera: ProbeResult::Available,
				screen: ProbeResult::Unavailable
			}
		);
		assert_eq!(
			decode_test(Exit::Code(Some(2))),
			HardwareSupport {
				camera: ProbeResult::Unavailable,
				screen: ProbeResult::Available
			}
		);
		for code in [None, Some(64), Some(70), Some(255)] {
			assert_eq!(decode_query(Exit::Code(code)), ProbeResult::Failed);
			assert_eq!(decode_test(Exit::Code(code)), UNKNOWN);
		}
	}

	#[test]
	fn helper_preset_reports_are_bounded_and_do_not_guess_on_errors() {
		assert_eq!(decode_presets(br#"{"presets": 258}"#).unwrap().bits(), 258);
		for bytes in [
			br#"{"presets":null}"#.as_slice(),
			b"",
			b"{}",
			b"not json",
			br#"{"presets":-1}"#,
			br#"{"presets":4294967296}"#,
			br#"{"presets":2} trailing"#,
			&[b' '; 129],
		] {
			assert_eq!(decode_presets(bytes), None);
		}
	}

	#[test]
	fn explicit_test_preserves_the_requested_preset_and_collapsing_retires_discovery() {
		let mut ui = settings();
		ui.video_capabilities = Some(initial_report());
		ui.video_encoder_tests_request = Some(VideoCodec::Av1);
		ui.video_encoder_tests_preset = Some(EncoderPreset::AmfQuality);
		ui.video_settings.quality = EncoderPreset::Automatic;
		let ctx = eframe::egui::Context::default();
		let mut detector = Detector::default();
		detector.poll_with(false, &mut ui, &ctx, |operation, _, _, _| {
			assert_eq!(
				operation,
				Operation::Test(VideoCodec::Av1, EncoderPreset::AmfQuality)
			);
			None
		});
		ui.video_encoding_advanced_visible = false;
		ui.video_capabilities_refresh = true;
		detector.poll_with(false, &mut ui, &ctx, |_, _, _, _| panic!("collapsed"));
		assert!(!ui.video_capabilities_refresh);
	}

	#[cfg(unix)]
	#[test]
	fn helper_errors_are_visible_bounded_and_keep_draining() {
		let mut command = Command::new("/bin/sh");
		command.args([
			"-c",
			"printf 'AMF Vulkan initialization failed (result=1).\\n' >&2; dd if=/dev/zero bs=4096 count=32 >&2 2>/dev/null",
		]);
		let mut child = ProbeChild(
			command
				.stdout(Stdio::null())
				.stderr(Stdio::piped())
				.spawn()
				.unwrap(),
		);
		let mut errors = HelperErrors::default();
		let mut output = Vec::new();
		let started = Instant::now();
		loop {
			errors.drain(child.0.stderr.as_mut().unwrap(), &mut output);
			if child.0.try_wait().unwrap().is_some() {
				errors.drain(child.0.stderr.as_mut().unwrap(), &mut output);
				break;
			}
			assert!(started.elapsed() < Duration::from_secs(3));
			std::thread::sleep(Duration::from_millis(1));
		}
		assert!(output.starts_with(b"AMF Vulkan initialization failed (result=1)."));
		assert_eq!(errors.emitted, MAX_HELPER_ERRORS);
		assert!(errors.truncated);
		assert!(output.len() <= MAX_HELPER_ERRORS + 64);
	}

	#[cfg(unix)]
	#[test]
	fn helper_report_read_does_not_wait_for_inherited_pipe_handles() {
		let mut command = Command::new("/bin/sh");
		command.args(["-c", "(sleep 0.5) & exit 0"]);
		let now = Instant::now();
		assert_eq!(
			wait_for_helper(
				&mut command,
				&AtomicBool::new(false),
				Duration::from_secs(2),
				&mut None
			),
			Exit::Code(Some(0))
		);
		assert!(now.elapsed() < Duration::from_millis(400));
	}

	#[cfg(unix)]
	#[test]
	fn helper_deadline_and_cancellation_kill_and_reap_processes() {
		let stop = AtomicBool::new(false);
		let started = Instant::now();
		let mut command = Command::new("/bin/sh");
		command.args(["-c", "exec sleep 60"]);
		assert_eq!(
			wait_for_helper(&mut command, &stop, Duration::from_millis(40), &mut None),
			Exit::TimedOut
		);
		assert!(started.elapsed() < Duration::from_secs(2));
		let stop = Arc::new(AtomicBool::new(false));
		let worker_stop = stop.clone();
		let worker = std::thread::spawn(move || {
			std::thread::sleep(Duration::from_millis(40));
			worker_stop.store(true, Ordering::Release);
		});
		assert_eq!(
			wait_for_helper(&mut command, &stop, Duration::from_secs(3), &mut None),
			Exit::Failed
		);
		worker.join().unwrap();
	}
}
