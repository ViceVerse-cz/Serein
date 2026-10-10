//! Process-local driver discovery and explicitly requested encoder tests.
use model::voice_settings::{
	DriverCapabilities, HardwareBackend, HardwareSupport, ProbeResult, VideoBackend,
	VideoCapabilities, VideoCodec,
};
use std::{
	ffi::OsString,
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
const UNKNOWN: HardwareSupport = HardwareSupport {
	camera: ProbeResult::Failed,
	screen: ProbeResult::Failed,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
	Discover,
	Test(VideoCodec),
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
	let adapter = match args.next() {
		None => None,
		Some(flag) if flag == ADAPTER_ARGUMENT => {
			let Some(adapter) = args
				.next()
				.and_then(|key| model::VideoAdapter::from_helper_key(key.to_str()?))
			else {
				return Some(Err(()));
			};
			Some(adapter)
		}
		_ => return Some(Err(())),
	};
	Some(match (backend, codec, args.next()) {
		(Some(backend), Some(codec), None) => Ok((helper, backend, codec, adapter)),
		_ => Err(()),
	})
}

/// Exit before GUI, credentials, network clients or capture discovery initialize.
pub fn probe_command() -> Option<i32> {
	request(std::env::args_os().skip(1)).map(|request| match request {
		Ok((helper, backend, codec, adapter))
			if discord_voice::video_capabilities::backends().contains(&backend) =>
		{
			match helper {
				Helper::Query => match discord_voice::video_capabilities::query_on_adapter(
					backend, codec, adapter,
				) {
					ProbeResult::Available => 1,
					ProbeResult::Unavailable => 0,
					_ => 2,
				},
				Helper::Test => {
					let support = discord_voice::video_capabilities::probe_on_adapter(
						backend, codec, adapter,
					);
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
		Exit::Code(Some(1)) => ProbeResult::Available,
		Exit::Code(Some(0)) => ProbeResult::Unavailable,
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

fn wait_for_helper(command: &mut Command, stop: &AtomicBool, timeout: Duration) -> Exit {
	if stop.load(Ordering::Acquire) {
		return Exit::Failed;
	}
	command
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null());
	let Ok(child) = command.spawn() else {
		return Exit::Failed;
	};
	let mut child = ProbeChild(child);
	let started = Instant::now();
	loop {
		if stop.load(Ordering::Acquire) {
			return Exit::Failed;
		}
		match child.0.try_wait() {
			Ok(Some(status)) => return Exit::Code(status.code()),
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
	};
	for &backend in discord_voice::video_capabilities::backends() {
		for codec in VideoCodec::ALL {
			report.set(backend, codec, ProbeResult::Pending);
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
	for results in &mut report.support {
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
	Driver(HardwareBackend, VideoCodec, ProbeResult),
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
			if let Operation::Test(selected) = operation
				&& codec != selected
			{
				continue;
			}
			if stop.load(Ordering::Acquire) {
				return;
			}
			let argument = match operation {
				Operation::Discover => QUERY_ARGUMENT,
				Operation::Test(_) => TEST_ARGUMENT,
			};
			let mut command = Command::new(executable);
			command.args([argument, backend.key(), codec.key()]);
			if let Some(adapter) = adapter {
				command.args([ADAPTER_ARGUMENT, &adapter.helper_key()]);
			}
			let exit = wait_for_helper(&mut command, stop, HELPER_TIMEOUT);
			if stop.load(Ordering::Acquire) {
				return;
			}
			let update = match operation {
				Operation::Discover => Update::Driver(backend, codec, decode_query(exit)),
				Operation::Test(_) => Update::Test(backend, codec, decode_test(exit)),
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
			Some(Operation::Test(codec)) => {
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
		let visible = !demo
			&& ui.voice_settings_open()
			&& ui.video_settings.backend == VideoBackend::Experimental;
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
					Ok(Update::Driver(backend, codec, support)) => {
						if self.operation == Some(Operation::Discover)
							&& let Some(report) = &mut ui.video_capabilities
						{
							report.set(backend, codec, support);
						}
					}
					Ok(Update::Test(backend, codec, support)) => {
						if self.operation == Some(Operation::Test(codec))
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
				Some(Operation::Test(codec)) => {
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
			Operation::Test(codec)
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
				Operation::Test(codec) => {
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
		ui.video_settings.backend = VideoBackend::Experimental;
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
		};
		ui.video_capabilities = Some(driver);
		ui.video_encoder_tests = Some(initial_tests());
		ui.video_encoder_tests_loading = true;
		ui.video_encoder_tests_codec = Some(VideoCodec::H265);
		let mut detector = Detector {
			receive: None,
			thread: None,
			stop: Some(Arc::new(AtomicBool::new(false))),
			operation: Some(Operation::Test(VideoCodec::H265)),
		};
		ui.video_settings.backend = VideoBackend::Stable;
		detector.poll_with(false, &mut ui, &ctx, |_, _, _, _| {
			panic!("closed settings cannot launch")
		});
		ui.video_settings.backend = VideoBackend::Experimental;
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
			[Operation::Discover, Operation::Test(VideoCodec::Av1)]
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
			assert_eq!(operation, Operation::Test(VideoCodec::Av1));
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
				None
			)))
		);
		assert_eq!(
			request(args(&[TEST_ARGUMENT, "qsv", "h264"])),
			Some(Ok((
				Helper::Test,
				HardwareBackend::Qsv,
				VideoCodec::H264,
				None
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
				Some(adapter)
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
	fn driver_and_encode_statuses_are_separate_and_errors_stay_unknown() {
		assert_eq!(decode_query(Exit::Code(Some(1))), ProbeResult::Available);
		assert_eq!(decode_query(Exit::Code(Some(0))), ProbeResult::Unavailable);
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

	#[cfg(unix)]
	#[test]
	fn helper_deadline_and_cancellation_kill_and_reap_processes() {
		let stop = AtomicBool::new(false);
		let started = Instant::now();
		let mut command = Command::new("/bin/sh");
		command.args(["-c", "exec sleep 60"]);
		assert_eq!(
			wait_for_helper(&mut command, &stop, Duration::from_millis(40)),
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
			wait_for_helper(&mut command, &stop, Duration::from_secs(3)),
			Exit::Failed
		);
		worker.join().unwrap();
	}
}
