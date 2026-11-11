use crate::screen::{AudioChunk, MAX_AUDIO_SAMPLES};
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
};
use tokio::sync::mpsc::Sender;

#[cfg(target_os = "windows")]
use std::borrow::Cow;

/// The worker that captures system audio for one screen share. Dropping it stops and joins
/// the worker.
#[cfg(target_os = "windows")]
pub(super) struct Audio {
	thread: Option<std::thread::JoinHandle<()>>,
	stop: Arc<AtomicBool>,
	failed: Arc<AtomicBool>,
	/// First Windows error code that ended capture, 0 when none.
	code: Arc<AtomicI32>,
}

#[cfg(target_os = "windows")]
impl Audio {
	/// Starts capture and waits for the first sign that audio can flow. Sharing a window
	/// captures only that window's application, as Discord does; `window_process` is its
	/// process, and anything else captures every other application. A startup failure
	/// names the Windows error code when the worker recorded one.
	pub(super) fn start(
		send: Sender<AudioChunk>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		audio_epoch: Arc<AtomicU64>,
		window_process: Option<u32>,
	) -> Result<Self, Cow<'static, str>> {
		let failed = Arc::new(AtomicBool::new(false));
		let code = Arc::new(AtomicI32::new(0));
		let input = Samples {
			send,
			stop: stop.clone(),
			ready,
			epoch: audio_epoch,
			failed: failed.clone(),
			code: code.clone(),
		};
		let (started, result) = std::sync::mpsc::sync_channel(1);
		let thread = std::thread::Builder::new()
			.name("screen-audio".into())
			.spawn(move || {
				if let Err(error) = native::run(&input, started, window_process) {
					native::abort(&input, error.code().0);
				}
			})
			.map_err(|_| Cow::Borrowed("System audio worker could not start"))?;
		let audio = Self {
			thread: Some(thread),
			stop,
			failed,
			code,
		};
		match result.recv_timeout(std::time::Duration::from_secs(4)) {
			Ok(()) => Ok(audio),
			// Still starting: the share goes ahead and the audio joins in when Windows is ready.
			// A start that never succeeds ends the share through `failed()` with its reason.
			Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(audio),
			Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
				// The worker gave up and is ending. Dropping `audio` joins it, which makes the
				// error code it recorded final before the code is read.
				let code = audio.code.clone();
				drop(audio);
				Err(native::describe(
					"System audio could not start",
					code.load(Ordering::Acquire),
				))
			}
		}
	}

	/// The status text for a stopped capture, with the Windows error code when one is known.
	pub(super) fn failure(&self) -> Cow<'static, str> {
		native::describe(
			"System audio capture stopped",
			self.code.load(Ordering::Acquire),
		)
	}

	/// Whether capture has failed or its worker has ended.
	pub(super) fn failed(&self) -> bool {
		self.failed.load(Ordering::Acquire)
			|| self
				.thread
				.as_ref()
				.is_some_and(std::thread::JoinHandle::is_finished)
	}
}

#[cfg(target_os = "windows")]
impl Drop for Audio {
	/// Stops the worker and waits for it, so no capture outlives the share.
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Release);
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

// Two capture strategies keep Serein's own playback out of the shared audio:
//
// * Windows build 20348+ activates one process loopback that excludes Serein's process tree.
// * Older builds (Windows 10) cannot exclude a tree, so a supervisor lists the applications
//   that currently own audio sessions, skips Serein's tree and anything Serein descends from,
//   and runs one include-mode loopback per remaining application, activating them in
//   parallel and recycling a lane that goes quiet while its application plays. A mixer sums
//   their lanes into the same 48 kHz stereo stream. The whole output is never captured.
//
// WASAPI owns each packet until ReleaseBuffer. COM and capture objects stay on the worker
// that created them.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
	use super::*;
	use crate::diagnostics::{Capture, Metrics, Scope, Stage};
	use std::{
		collections::{HashMap, HashSet, VecDeque},
		mem::{ManuallyDrop, size_of},
		os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
		sync::{
			Mutex, PoisonError,
			mpsc::{SyncSender, sync_channel},
		},
		time::{Duration, Instant},
	};
	use windows::{
		Win32::{
			Foundation::{
				E_ACCESSDENIED, E_FAIL, ERROR_NO_MORE_FILES, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
			},
			Media::Audio::*,
			System::{
				Com::{
					BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
					CoUninitialize,
					StructuredStorage::{
						PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
					},
				},
				Diagnostics::ToolHelp::{
					CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
					TH32CS_SNAPPROCESS,
				},
				Threading::{
					CreateEventW, OpenProcess, PROCESS_NAME_WIN32,
					PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
					WaitForSingleObject,
				},
				Variant::VT_BLOB,
			},
		},
		core::{HRESULT, Interface, PWSTR, Ref, Result, implement},
	};

	/// First Windows build whose process loopback can exclude a process tree.
	const EXCLUDE_MIN_BUILD: u32 = 20348;
	/// The mixer emits 10 ms of 48 kHz stereo per tick, like a WASAPI shared-mode period.
	const MIX_TICK: Duration = Duration::from_millis(10);
	/// Interleaved stereo samples in one mixer tick.
	const TICK_SAMPLES: usize = 480 * 2;
	/// Each application lane buffers at most 100 ms; older samples are dropped, never queued.
	const LANE_CAP_SAMPLES: usize = 4800 * 2;
	/// Most applications captured at once; playing ones replace idle ones beyond this.
	const MAX_LANES: usize = 16;
	/// How often the supervisor lists audio sessions.
	const SESSION_POLL: Duration = Duration::from_millis(250);
	/// Samples a lane holds before it is mixed, 20 ms: absorbs the jitter between the capture
	/// events and the mixer clock so a short buffer does not leave gaps in the stream.
	const LANE_PRIME_SAMPLES: usize = TICK_SAMPLES * 2;
	/// A lane that ended is restarted after this, doubling per failure in a row up to the
	/// maximum, so a recovering application returns quickly and a failing one cannot spin.
	const LANE_RETRY_MIN: Duration = Duration::from_millis(250);
	const LANE_RETRY_MAX: Duration = Duration::from_secs(2);
	/// How long one process loopback activation may take. Windows 10 before build 20348 can take
	/// many seconds, and a request given up early stays queued in the audio service, so waiting it
	/// out is faster than asking again.
	const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(20);
	/// How long audio may keep failing to start for no specific reason before the share stops.
	const STARTUP_PATIENCE: Duration = Duration::from_secs(45);
	/// Capture is only declared failed after this long with failures pending and no lane
	/// working, so a restart after a device change or an app's own stream restart recovers.
	const ACTIVATION_GRACE: Duration = Duration::from_secs(8);
	/// Lanes activate in parallel, so an application whose activation hangs gives up sooner.
	const LANE_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(10);
	/// A playing application whose lane delivered nothing for this long gets a fresh capture.
	const LANE_STALL: Duration = Duration::from_secs(4);

	/// The `OSVERSIONINFOW` layout that `RtlGetVersion` fills.
	#[repr(C)]
	struct OsVersionInfo {
		size: u32,
		major: u32,
		minor: u32,
		build: u32,
		platform: u32,
		csd: [u16; 128],
	}

	#[link(name = "ntdll")]
	unsafe extern "system" {
		/// Fills `info` with the true OS version and returns an NTSTATUS, 0 on success.
		fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
	}

	/// The real build number; unlike GetVersionEx it is not shimmed by application manifests.
	fn build_number() -> u32 {
		let mut info = OsVersionInfo {
			size: size_of::<OsVersionInfo>() as u32,
			major: 0,
			minor: 0,
			build: 0,
			platform: 0,
			csd: [0; 128],
		};
		// RtlGetVersion fills the structure and returns STATUS_SUCCESS (0).
		if unsafe { RtlGetVersion(&mut info) } == 0 {
			info.build
		} else {
			0
		}
	}

	/// Records the first native error and stops the share's audio, so a mixer that is still
	/// running cannot keep sending silence after capture has failed.
	pub(super) fn abort(input: &Samples, code: i32) {
		// Keep the first specific cause; an earlier failure may already have set one.
		let _ = input
			.code
			.compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
		input.failed.store(true, Ordering::Release);
		input.stop.store(true, Ordering::Release);
	}

	/// `what`, then the Windows error code when one is known, then how to get audio working.
	pub(super) fn describe(what: &str, code: i32) -> Cow<'static, str> {
		const ADVICE: &str = "check your output device or share without audio";
		// E_FAIL is this module's "no specific cause", so it says nothing worth showing.
		if code == 0 || code == E_FAIL.0 {
			Cow::Owned(format!("{what}; {ADVICE}"))
		} else {
			Cow::Owned(format!("{what} (error 0x{:08X}); {ADVICE}", code as u32))
		}
	}

	/// The native error behind a failed thread spawn, or `E_FAIL` when Windows gave none.
	fn spawn_failure(error: &std::io::Error) -> windows::core::Error {
		error
			.raw_os_error()
			.map_or(E_FAIL, |code| HRESULT::from_win32(code as u32))
			.into()
	}

	/// The executable file name of `pid`, for the opt-in diagnostics line only. Never a path.
	fn image_name(pid: u32) -> String {
		let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
		else {
			return "unknown".into();
		};
		// The owned handle closes the process handle on every path.
		let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
		let mut buffer = [0_u16; 260];
		let mut length = buffer.len() as u32;
		let read = unsafe {
			QueryFullProcessImageNameW(
				HANDLE(process.as_raw_handle()),
				PROCESS_NAME_WIN32,
				PWSTR(buffer.as_mut_ptr()),
				&mut length,
			)
		};
		if read.is_err() {
			return "unknown".into();
		}
		let path = String::from_utf16_lossy(&buffer[..length as usize]);
		path.rsplit('\\').next().unwrap_or("unknown").to_owned()
	}

	/// Whether `pid` is the Windows audio engine, which hosts every application's audio
	/// processing and is not an application itself. Its session is never captured.
	fn is_audio_engine(pid: u32) -> bool {
		image_name(pid).eq_ignore_ascii_case("audiodg.exe")
	}

	/// Where capture buffers go and when a capture must stop.
	trait Sink {
		/// Whether capture must end: the share stopped, failed or went away.
		fn stopped(&self) -> bool;
		/// The current encryption generation; audio is only accepted for the generation it was
		/// captured under.
		fn epoch(&self) -> u64;
		/// Hands over captured samples, returning whether they were accepted.
		fn push(&self, data: &[f32], epoch: u64) -> bool;
		/// The capture client started delivering.
		fn running(&self) {}
		/// Whether capture must be recreated when the encryption generation changes. A lane
		/// must not: recreating a process loopback of the same application fails on Windows 10
		/// before build 20348, and the mixer already labels audio with the current generation.
		fn restarts_on_epoch(&self) -> bool {
			true
		}
		/// How long activating the capture may take before it counts as failed.
		fn activation_timeout(&self) -> Duration {
			ACTIVATION_TIMEOUT
		}
		/// Whether activations must wait their turn. Only the single whole-share capture needs
		/// it; lanes activate in parallel so one stuck application cannot delay the others.
		fn serial_activation(&self) -> bool {
			true
		}
	}

	/// A share's audio that is captured by one client for as long as it lives: it is not
	/// recreated when the encryption generation changes, because each packet is labeled with the
	/// generation current when it arrives. Creating a process loopback again fails or stalls on
	/// Windows 10 before build 20348.
	struct Steady<'a> {
		inner: &'a Samples,
		timeout: Duration,
	}

	impl Sink for Steady<'_> {
		fn stopped(&self) -> bool {
			self.inner.stopped()
		}
		fn epoch(&self) -> u64 {
			self.inner.epoch()
		}
		fn push(&self, data: &[f32], epoch: u64) -> bool {
			self.inner.push_at(data, epoch)
		}
		fn restarts_on_epoch(&self) -> bool {
			false
		}
		fn activation_timeout(&self) -> Duration {
			self.timeout
		}
	}

	/// Opt-in diagnostics line saying which capture strategy a share ended up with.
	fn note_mode(mode: &str) {
		if std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|value| value == "1") {
			eprintln!("[Serein voice Screen] audio_mode={mode}");
		}
	}

	impl Sink for Samples {
		/// Whether the share's audio was stopped or its receiver closed.
		fn stopped(&self) -> bool {
			self.stop.load(Ordering::Acquire) || self.send.is_closed()
		}
		/// The share's current encryption generation.
		fn epoch(&self) -> u64 {
			self.epoch.load(Ordering::Acquire)
		}
		/// Sends samples straight to the stream.
		fn push(&self, data: &[f32], epoch: u64) -> bool {
			self.push_at(data, epoch)
		}
	}

	// pure-begin
	/// Interleaved stereo samples captured for one encryption generation.
	struct Buffer {
		epoch: u64,
		samples: VecDeque<f32>,
		/// Whether enough has been captured to play; cleared again by an underrun.
		primed: bool,
	}

	impl Buffer {
		/// An empty buffer that takes the epoch of its first samples.
		fn new() -> Self {
			Self {
				epoch: 0,
				samples: VecDeque::new(),
				primed: false,
			}
		}

		/// Appends capture data, dropping the oldest samples beyond the latency bound.
		fn append(&mut self, data: &[f32], epoch: u64, cap: usize) {
			if self.epoch != epoch {
				self.samples.clear();
				self.primed = false;
				self.epoch = epoch;
			}
			self.samples.extend(data.iter().copied());
			let excess = self.samples.len().saturating_sub(cap);
			if excess > 0 {
				self.samples.drain(..excess);
			}
		}

		/// Adds up to one tick into `frame`. Playback starts once `prime` samples are held and
		/// stops again when the buffer runs dry, so a lane that delivers in bursts is heard
		/// without gaps and an idle lane adds silence. Samples captured under another epoch are
		/// discarded so audio is never relabeled across an encryption transition.
		fn mix_into(&mut self, frame: &mut [f32], epoch: u64, prime: usize) {
			if self.epoch != epoch {
				self.samples.clear();
				self.primed = false;
				return;
			}
			if !self.primed {
				if self.samples.len() < prime.max(1) {
					return;
				}
				self.primed = true;
			}
			let count = self.samples.len().min(frame.len());
			for (slot, sample) in frame.iter_mut().zip(self.samples.drain(..count)) {
				*slot += sample;
			}
			if count < frame.len() {
				self.primed = false;
			}
		}
	}

	/// A process and every ancestor the snapshot's parent links lead to, nearest first. The walk
	/// ends at a process with no parent link, or before it would revisit one. Windows keeps
	/// stale parent ids, so after id reuse the links can loop, and a loop lists each member once.
	fn lineage(pid: u32, parents: &HashMap<u32, u32>) -> Vec<u32> {
		let mut chain = vec![pid];
		let mut current = pid;
		while let Some(&parent) = parents.get(&current) {
			if parent == 0 || chain.contains(&parent) {
				break;
			}
			chain.push(parent);
			current = parent;
		}
		chain
	}

	/// Whether `pid` is `ancestor` or one of its descendants, by the snapshot's parent links.
	fn within_tree(pid: u32, ancestor: u32, parents: &HashMap<u32, u32>) -> bool {
		lineage(pid, parents).contains(&ancestor)
	}

	/// Picks the processes to capture. Include mode takes a process and all its children, so:
	/// Serein and its children are never captured; a process that Serein descends from is
	/// skipped because its tree would carry Serein's playback; and a process inside another
	/// selected tree is left to that tree's capture, which avoids mixing it twice.
	///
	/// Stale parent links can form a loop. A loop above Serein excludes all of its members,
	/// because none can be shown to be unrelated to Serein. Anywhere else, two processes that
	/// each appear inside the other are a loop and not a nesting, so both stay captured instead
	/// of silently dropping each other.
	fn select_roots(sessions: &[u32], ours: u32, parents: &HashMap<u32, u32>) -> Vec<u32> {
		let ours_lineage = lineage(ours, parents);
		let mut roots: Vec<u32> = sessions
			.iter()
			.copied()
			.filter(|&pid| {
				pid != 0 && !ours_lineage.contains(&pid) && !within_tree(pid, ours, parents)
			})
			.collect();
		roots.sort_unstable();
		roots.dedup();
		let all = roots.clone();
		roots.retain(|&pid| {
			!all.iter().any(|&other| {
				other != pid
					&& within_tree(pid, other, parents)
					&& !within_tree(other, pid, parents)
			})
		});
		roots
	}

	/// Records every process that descends from Serein, directly or through one already recorded.
	/// A recorded process stays excluded after its parent exits and Windows leaves its parent link
	/// stale, which would otherwise make it look unrelated. Entries are dropped once the process
	/// itself is gone, so a reused id is not excluded for long.
	fn track_descendants(ours: u32, parents: &HashMap<u32, u32>, known: &mut HashSet<u32>) {
		for &pid in parents.keys() {
			if pid != ours
				&& lineage(pid, parents)
					.iter()
					.skip(1)
					.any(|ancestor| *ancestor == ours || known.contains(ancestor))
			{
				known.insert(pid);
			}
		}
		known.retain(|pid| parents.contains_key(pid));
	}
	// pure-end

	/// One application's include-mode capture, drained by the mixer.
	struct Lane {
		input: Samples,
		/// The application this lane captures, and its executable name for diagnostics.
		pid: u32,
		name: String,
		/// Loudest sample since the last diagnostics report, as `f32` bits (non-negative).
		peak: std::sync::atomic::AtomicU32,
		/// Capture packets received since the last diagnostics report.
		packets: AtomicU64,
		retired: AtomicBool,
		running: AtomicBool,
		/// The error that ended this lane, 0 when it ended normally.
		error: AtomicI32,
		buffer: Mutex<Buffer>,
		born: Instant,
		/// Milliseconds after `born` when this lane last delivered a packet or started.
		seen: AtomicU64,
	}

	impl Sink for Lane {
		/// Whether this lane was retired or the whole share's audio stopped.
		fn stopped(&self) -> bool {
			self.retired.load(Ordering::Acquire) || self.input.stopped()
		}
		/// The share's current encryption generation.
		fn epoch(&self) -> u64 {
			self.input.epoch()
		}
		/// Appends samples to this lane's bounded buffer for the mixer, returning whether the
		/// lane took them.
		fn push(&self, data: &[f32], epoch: u64) -> bool {
			// Any native packet proves the capture is alive, even one refused below because the
			// share is not ready yet, so the stall watchdog never retires a lane that is delivering.
			self.seen
				.store(self.born.elapsed().as_millis() as u64, Ordering::Relaxed);
			if self.stopped()
				|| !self.input.ready.load(Ordering::Acquire)
				|| epoch != self.epoch()
				|| data.len() > MAX_AUDIO_SAMPLES
				|| !data.len().is_multiple_of(2)
			{
				return false;
			}
			self.packets.fetch_add(1, Ordering::Relaxed);
			let loudest = data
				.iter()
				.fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
			if loudest.is_finite() {
				self.peak.fetch_max(loudest.to_bits(), Ordering::Relaxed);
			}
			self.buffer
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.append(data, epoch, LANE_CAP_SAMPLES);
			true
		}
		/// Marks the lane as delivering, which the supervisor reads as a working capture.
		fn running(&self) {
			self.seen
				.store(self.born.elapsed().as_millis() as u64, Ordering::Relaxed);
			self.running.store(true, Ordering::Release);
		}
		/// A lane keeps capturing across encryption transitions; see `Sink`.
		fn restarts_on_epoch(&self) -> bool {
			false
		}
		fn activation_timeout(&self) -> Duration {
			LANE_ACTIVATION_TIMEOUT
		}
		fn serial_activation(&self) -> bool {
			false
		}
	}

	/// A lane and the thread that captures into it.
	struct Running {
		lane: Arc<Lane>,
		thread: std::thread::JoinHandle<()>,
	}

	/// The lanes the mixer sums, shared between the supervisor and the mixer.
	type Lanes = Mutex<Vec<Arc<Lane>>>;

	/// Signals that an asynchronous audio interface activation has completed.
	#[implement(IActivateAudioInterfaceCompletionHandler)]
	struct Completion {
		send: SyncSender<()>,
		// Keep the activation blob alive even if cancellation wins the completion race.
		_params: Arc<AUDIOCLIENT_ACTIVATION_PARAMS>,
	}

	impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
		/// Wakes the waiting activation; the result itself is read from the operation.
		fn ActivateCompleted(
			&self,
			_: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
		) -> Result<()> {
			let _ = self.send.try_send(());
			Ok(())
		}
	}

	/// Process loopback activation parameters for `pid` in the given include or exclude mode.
	fn activation_params(pid: u32, mode: PROCESS_LOOPBACK_MODE) -> AUDIOCLIENT_ACTIVATION_PARAMS {
		AUDIOCLIENT_ACTIVATION_PARAMS {
			ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
			Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
				ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
					TargetProcessId: pid,
					ProcessLoopbackMode: mode,
				},
			},
		}
	}

	/// Captures system audio for the share until it stops, using whichever strategy this
	/// Windows build supports. An error means capture failed; the caller records it.
	pub(super) fn run(
		input: &Samples,
		started: SyncSender<()>,
		window_process: Option<u32>,
	) -> Result<()> {
		// This dedicated thread initializes and releases all COM objects in one MTA.
		unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
		let result = if let Some(pid) = window_process.filter(|&pid| shareable_window(pid)) {
			// A window share carries that application's audio and nothing else.
			application_tree(input, started, pid)
		} else if build_number() >= EXCLUDE_MIN_BUILD {
			note_mode("exclude");
			exclude_tree(input, started)
		} else {
			system_audio(input, started)
		};
		unsafe { CoUninitialize() };
		result
	}

	/// Whether a window's process can be captured alone: it is neither Serein, nor something
	/// Serein runs under, nor inside Serein, so its tree cannot carry Serein's own playback. When
	/// the process list cannot be read this is false and the share captures other applications.
	/// Store apps' windows belong to ApplicationFrameHost, which plays nothing itself, so they
	/// also capture other applications instead of sharing silence.
	fn shareable_window(pid: u32) -> bool {
		if image_name(pid).eq_ignore_ascii_case("ApplicationFrameHost.exe") {
			return false;
		}
		let Ok(parents) = process_parents() else {
			return false;
		};
		let ours = std::process::id();
		pid != 0 && !lineage(ours, &parents).contains(&pid) && !within_tree(pid, ours, &parents)
	}

	/// Captures one application's process tree for a window share, restarting across encryption
	/// generations. Once capture has worked, a failure is retried: the application may restart.
	fn application_tree(input: &Samples, started: SyncSender<()>, pid: u32) -> Result<()> {
		let mut started = Some(started);
		let begun = Instant::now();
		let steady = Steady {
			inner: input,
			timeout: ACTIVATION_TIMEOUT,
		};
		note_mode("application");
		while !input.stopped() {
			match capture(
				&steady,
				pid,
				PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
				&mut started,
			) {
				Ok(()) => {}
				// A specific refusal is final, but only while capture has never started: `started`
				// is read after the call, so a failure after a successful start is retried.
				// A slow or busy audio service is waited out, up to the patience, because
				// activation succeeds on a later try.
				Err(error)
					if started.is_some()
						&& (error.code() != E_FAIL || begun.elapsed() >= STARTUP_PATIENCE) =>
				{
					return Err(error);
				}
				// Later failures wait for the application to come back.
				Err(_) => std::thread::sleep(Duration::from_millis(250)),
			}
		}
		Ok(())
	}

	/// One loopback of everything except Serein's process tree (Windows build 20348+).
	fn exclude_tree(input: &Samples, started: SyncSender<()>) -> Result<()> {
		let mut started = Some(started);
		while !input.stopped() {
			capture(
				input,
				std::process::id(),
				PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
				&mut started,
			)?;
		}
		Ok(())
	}

	/// Captures everything except Serein on Windows 10 before build 20348, where Windows cannot
	/// exclude a process tree. Each application is captured on its own, as Discord does, and the
	/// lanes are mixed. There is no exclude probe: on these builds it can stall for many seconds
	/// and leave abandoned requests queued in the audio service.
	fn system_audio(input: &Samples, started: SyncSender<()>) -> Result<()> {
		note_mode("per_application");
		per_application(input, started)
	}

	/// Mixes one include-mode loopback per application (Windows 10 before build 20348).
	fn per_application(input: &Samples, started: SyncSender<()>) -> Result<()> {
		let lanes: Arc<Lanes> = Arc::new(Mutex::new(Vec::new()));
		let halt = Arc::new(AtomicBool::new(false));
		let (ready_send, ready) = sync_channel(1);
		let supervisor = {
			let (input, lanes, halt) = (input.clone(), lanes.clone(), halt.clone());
			std::thread::Builder::new()
				.name("screen-audio-sessions".into())
				.spawn(move || supervise_thread(&input, &lanes, &halt, ready_send))
				.map_err(|error| spawn_failure(&error))?
		};
		// The share is usable once audio sessions can be listed, even if nothing is playing yet.
		let listed = ready.recv_timeout(Duration::from_secs(10)).is_ok();
		if listed {
			let _ = started.try_send(());
			mix(input, &lanes, &supervisor);
		}
		halt.store(true, Ordering::Release);
		let supervised = supervisor.join().unwrap_or_else(|_| Err(E_FAIL.into()));
		match supervised {
			// The supervisor's own error names why it failed, so it outranks a generic one.
			Err(error) => Err(error),
			Ok(()) if listed => Ok(()),
			// Sessions could not be listed in time, and nothing says why.
			Ok(()) => Err(E_FAIL.into()),
		}
	}

	/// Sums every lane into one stream on a steady 10 ms clock until the share stops or the
	/// supervisor ends. Without the supervisor no lane is ever replaced, so the mixer must not
	/// outlive it and send silence. Chunks the stream refuses are counted as mixer drops, apart
	/// from the lane writes that capture reports.
	fn mix(input: &Samples, lanes: &Lanes, supervisor: &std::thread::JoinHandle<Result<()>>) {
		let mut metrics = Metrics::new(Scope::ScreenAudio);
		let mut frame = [0.0_f32; TICK_SAMPLES];
		let mut next = Instant::now() + MIX_TICK;
		let diagnostics =
			std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|value| value == "1");
		let mut ticks = 0_u32;
		while !input.stopped() && !supervisor.is_finished() {
			let now = Instant::now();
			if now < next {
				std::thread::sleep(next - now);
			} else if now - next > Duration::from_millis(100) {
				// Far behind (suspend or a stalled system): resume the clock instead of bursting.
				next = now;
			}
			next += MIX_TICK;
			let epoch = input.epoch();
			frame.fill(0.0);
			let active: Vec<Arc<Lane>> =
				lanes.lock().unwrap_or_else(PoisonError::into_inner).clone();
			for lane in &active {
				lane.buffer
					.lock()
					.unwrap_or_else(PoisonError::into_inner)
					.mix_into(&mut frame, epoch, LANE_PRIME_SAMPLES);
			}
			ticks = ticks.wrapping_add(1);
			if diagnostics && ticks.is_multiple_of(500) {
				// Which application carried sound in the last 5 s, to find where a voice comes from.
				for lane in &active {
					let peak = f32::from_bits(lane.peak.swap(0, Ordering::Relaxed));
					let packets = lane.packets.swap(0, Ordering::Relaxed);
					eprintln!(
						"[Serein voice Screen] lane pid={} exe={} peak={peak:.4} packets={packets}",
						lane.pid, lane.name
					);
				}
			}
			// Silence is sent too, so the stream keeps its cadence between sounds.
			let handed = input.push_at(&frame, epoch);
			if handed {
				metrics.capture(Capture::Chunks, 1);
			}
			// Before the share is ready the stream takes nothing, which is not a drop.
			metrics.poll(
				false,
				u64::from(!handed && input.ready.load(Ordering::Acquire)),
				false,
				0,
			);
		}
	}

	/// Runs the session supervisor in its own COM apartment. Any failure stops the share's
	/// audio before returning, so the mixer cannot keep sending silence without lanes.
	fn supervise_thread(
		input: &Samples,
		lanes: &Lanes,
		halt: &AtomicBool,
		ready: SyncSender<()>,
	) -> Result<()> {
		let result = match unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
			Ok(()) => {
				let result = supervise(input, lanes, halt, ready);
				unsafe { CoUninitialize() };
				result
			}
			Err(error) => Err(error),
		};
		if let Err(error) = &result {
			abort(input, error.code().0);
		}
		result
	}

	/// Sleeps one session poll, waking early once `halt` is set.
	fn nap(halt: &AtomicBool) {
		let slice = Duration::from_millis(25);
		for _ in 0..(SESSION_POLL.as_millis() / slice.as_millis()) {
			if halt.load(Ordering::Acquire) {
				return;
			}
			std::thread::sleep(slice);
		}
	}

	/// Ends a lane: its capture thread stops, and the mixer no longer sums it.
	fn retire(entry: &Running, lanes: &Lanes) {
		entry.lane.retired.store(true, Ordering::Release);
		lanes
			.lock()
			.unwrap_or_else(PoisonError::into_inner)
			.retain(|lane| !Arc::ptr_eq(lane, &entry.lane));
	}

	/// Starts a lane for each application that owns an audio session and stops lanes whose
	/// application is gone. Ending here ends every lane before returning. An error is one that
	/// persisted, and the caller stops the share's audio with it.
	///
	/// Playing applications are started first and may take the lane of an idle one when all
	/// lanes are in use. Only playing applications count as failures, and only while no lane
	/// works: one lane that fails to restart while others deliver is left to recover.
	fn supervise(
		input: &Samples,
		lanes: &Lanes,
		halt: &AtomicBool,
		ready: SyncSender<()>,
	) -> Result<()> {
		let ours = std::process::id();
		let mut ready = Some(ready);
		let mut running: HashMap<u32, Running> = HashMap::new();
		let mut retiring: Vec<std::thread::JoinHandle<()>> = Vec::new();
		let mut cooldown: HashMap<u32, Instant> = HashMap::new();
		// Failures in a row per application, which set how long its restart waits.
		let mut failures: HashMap<u32, u32> = HashMap::new();
		let mut errors = 0_u32;
		// Playing applications whose capture failed for a reason other than access, with the
		// error, and since when nothing has worked while some of them were failing.
		let mut blamed: HashMap<u32, i32> = HashMap::new();
		let mut failing_since: Option<Instant> = None;
		// Whether any lane has delivered during this share. After that, a lane that fails is
		// retried for as long as the share lasts instead of closing the stream.
		let mut delivered = false;
		// Since when no lane thread could be created.
		let mut unstartable: Option<Instant> = None;
		// Applications already named in the opt-in report, so a restarting lane names itself once.
		let mut announced: HashSet<u32> = HashSet::new();
		// Processes that descend from Serein, kept across polls so orphans stay excluded.
		let mut descendants: HashSet<u32> = HashSet::new();
		// Playing applications that found every lane taken by another playing one.
		let mut over_capacity: HashSet<u32> = HashSet::new();
		let result = (|| -> Result<()> {
			while !halt.load(Ordering::Acquire) && !input.stopped() {
				let wanted = match applications(ours, &mut descendants) {
					Ok(wanted) => {
						errors = 0;
						wanted
					}
					Err(error) => {
						errors += 1;
						// A failure before the first listing means audio cannot start at all. Later
						// ones may be momentary, so lanes that work keep running meanwhile.
						if ready.is_some() || errors >= 20 {
							return Err(error);
						}
						nap(halt);
						continue;
					}
				};
				if let Some(ready) = ready.take() {
					let _ = ready.try_send(());
				}
				let gone: Vec<u32> = running
					.keys()
					.copied()
					.filter(|pid| !wanted.contains_key(pid))
					.collect();
				for pid in gone {
					if let Some(entry) = running.remove(&pid) {
						retire(&entry, lanes);
						retiring.push(entry.thread);
					}
				}
				let ended: Vec<u32> = running
					.iter()
					.filter(|(_, entry)| entry.thread.is_finished())
					.map(|(pid, _)| *pid)
					.collect();
				for pid in ended {
					if let Some(entry) = running.remove(&pid) {
						retire(&entry, lanes);
						let _ = entry.thread.join();
						let code = entry.lane.error.load(Ordering::Acquire);
						// Elevated or protected processes refuse capture; that says nothing about support.
						if code != 0 && code != E_ACCESSDENIED.0 {
							blamed.entry(pid).or_insert(code);
						}
						let streak = if entry.lane.running.load(Ordering::Acquire) {
							// It delivered before ending: an application restart, not a failure.
							failures.remove(&pid);
							0
						} else {
							let count = failures.entry(pid).or_insert(0);
							*count += 1;
							*count - 1
						};
						let wait = (LANE_RETRY_MIN * 2_u32.saturating_pow(streak.min(8)))
							.min(LANE_RETRY_MAX);
						cooldown.insert(pid, Instant::now() + wait);
					}
				}
				// A playing application whose lane has gone quiet is captured afresh.
				let stalled: Vec<u32> = running
					.iter()
					.filter(|(pid, entry)| {
						wanted.get(*pid) == Some(&true)
							&& entry.lane.running.load(Ordering::Acquire)
							&& (entry.lane.born.elapsed().as_millis() as u64)
								.saturating_sub(entry.lane.seen.load(Ordering::Relaxed))
								>= LANE_STALL.as_millis() as u64
					})
					.map(|(pid, _)| *pid)
					.collect();
				for pid in stalled {
					if let Some(entry) = running.remove(&pid) {
						retire(&entry, lanes);
						retiring.push(entry.thread);
						cooldown.insert(pid, Instant::now() + LANE_RETRY_MIN);
					}
				}
				// Only a lane of a playing application counts as delivering: an idle one has
				// nothing to capture, so it says nothing about whether playing audio works.
				let healthy = running.iter().any(|(pid, entry)| {
					wanted.get(pid) == Some(&true) && entry.lane.running.load(Ordering::Acquire)
				});
				// A playing application whose lane is still activating is pending; idle lanes
				// are not, so they cannot postpone reporting a failure.
				let playing_pending = running.keys().any(|pid| wanted.get(pid) == Some(&true));
				// An idle application has nothing to capture, so its failure is not one.
				blamed.retain(|pid, _| wanted.get(pid) == Some(&true));
				delivered |= healthy;
				if healthy || blamed.is_empty() || delivered {
					// Something works, or nothing playing has failed: any later failure starts afresh.
					failing_since = None;
					if healthy || delivered {
						blamed.clear();
					}
				} else {
					let since = *failing_since.get_or_insert_with(Instant::now);
					// Failures return instantly while a working capture takes a moment to start,
					// so judge only after a grace period, with nothing still starting.
					// A generic failure is usually a slow or busy audio service, so it gets longer.
					let grace = if blamed.values().all(|&code| code == E_FAIL.0) {
						STARTUP_PATIENCE
					} else {
						ACTIVATION_GRACE
					};
					if !playing_pending && since.elapsed() >= grace {
						// Playing applications cannot be captured; report why instead of sharing silence.
						// Prefer a specific cause over the generic E_FAIL of a timed-out activation.
						let code = blamed
							.iter()
							.min_by_key(|(pid, code)| (**code == E_FAIL.0, **pid))
							.map_or(E_FAIL.0, |(_, &code)| code);
						return Err(HRESULT(code).into());
					}
				}
				let now = Instant::now();
				cooldown.retain(|_, until| *until > now);
				// Playing applications first, so idle sessions cannot hold every lane.
				let mut order: Vec<(u32, bool)> =
					wanted.iter().map(|(&pid, &active)| (pid, active)).collect();
				order.sort_by_key(|&(pid, active)| (!active, pid));
				let mut spawn_error = None;
				for (pid, active) in order {
					if running.contains_key(&pid) || cooldown.contains_key(&pid) {
						continue;
					}
					// Idle applications start only once playing ones are up, so activating them
					// cannot delay the audio that is actually playing.
					if !active
						&& running
							.values()
							.any(|entry| !entry.lane.running.load(Ordering::Acquire))
					{
						continue;
					}
					if running.len() >= MAX_LANES {
						// Only a playing application may take the lane of an idle one.
						if !active {
							break;
						}
						let Some(idle) = running
							.keys()
							.copied()
							.find(|other| wanted.get(other) != Some(&true))
						else {
							// Every lane carries a playing application. This one is not captured, so
							// say so in the opt-in report instead of leaving it silently out.
							if over_capacity.insert(pid)
								&& std::env::var_os("SEREIN_VOICE_DIAGNOSTICS")
									.is_some_and(|value| value == "1")
							{
								eprintln!(
									"[Serein voice Screen] lane_limit={MAX_LANES} not_captured pid={pid} exe={}",
									image_name(pid)
								);
							}
							continue;
						};
						if let Some(entry) = running.remove(&idle) {
							retire(&entry, lanes);
							retiring.push(entry.thread);
						}
					}
					let lane = Arc::new(Lane {
						input: input.clone(),
						pid,
						name: image_name(pid),
						peak: std::sync::atomic::AtomicU32::new(0),
						packets: AtomicU64::new(0),
						retired: AtomicBool::new(false),
						running: AtomicBool::new(false),
						error: AtomicI32::new(0),
						buffer: Mutex::new(Buffer::new()),
						born: Instant::now(),
						seen: AtomicU64::new(0),
					});
					let worker = lane.clone();
					let thread = match std::thread::Builder::new()
						.name("screen-audio-app".into())
						.spawn(move || lane_worker(&worker, pid))
					{
						Ok(thread) => thread,
						Err(error) => {
							// Another attempt would fail the same way; wait for the next poll.
							spawn_error = Some(spawn_failure(&error));
							break;
						}
					};
					lanes
						.lock()
						.unwrap_or_else(PoisonError::into_inner)
						.push(lane.clone());
					let name = lane.name.clone();
					over_capacity.remove(&pid);
					running.insert(pid, Running { lane, thread });
					if announced.insert(pid)
						&& std::env::var_os("SEREIN_VOICE_DIAGNOSTICS")
							.is_some_and(|value| value == "1")
					{
						eprintln!(
							"[Serein voice Screen] app_capture pid={pid} exe={} playing={active}",
							name
						);
					}
				}
				match spawn_error {
					// With no lane working, threads that can never be created are a failure.
					Some(error) if !healthy => {
						let since = *unstartable.get_or_insert_with(Instant::now);
						if since.elapsed() >= ACTIVATION_GRACE {
							return Err(error);
						}
					}
					// Another lane keeps the share's audio going; try again on the next poll.
					_ => unstartable = None,
				}
				let (done, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut retiring)
					.into_iter()
					.partition(std::thread::JoinHandle::is_finished);
				retiring = pending;
				for thread in done {
					let _ = thread.join();
				}
				nap(halt);
			}
			Ok(())
		})();
		for (_, entry) in running.drain() {
			retire(&entry, lanes);
			retiring.push(entry.thread);
		}
		for thread in retiring {
			let _ = thread.join();
		}
		result
	}

	/// Captures one application into its lane, restarting across encryption generations, until
	/// the lane is retired or capture fails. A failure is left in `lane.error`.
	fn lane_worker(lane: &Lane, pid: u32) {
		if let Err(error) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
			lane.error.store(error.code().0, Ordering::Release);
			return;
		}
		while !lane.stopped() {
			if let Err(error) = capture(
				lane,
				pid,
				PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
				&mut None,
			) {
				if !lane.stopped() {
					lane.error.store(error.code().0, Ordering::Release);
				}
				break;
			}
		}
		unsafe { CoUninitialize() };
	}

	/// The applications to capture now, each with whether it is playing audio. Serein, what it
	/// started and what started it are left out, so its own playback is never captured. Not
	/// learning the sessions or the process tree is an error: capturing without the tree
	/// could pick up Serein's own audio.
	fn applications(ours: u32, descendants: &mut HashSet<u32>) -> Result<HashMap<u32, bool>> {
		let mut sessions = render_sessions()?;
		let parents = process_parents()?;
		let mut engines: HashMap<u32, bool> = HashMap::new();
		sessions.retain(|&(pid, _)| !*engines.entry(pid).or_insert_with(|| is_audio_engine(pid)));
		// What Serein started stays excluded after its parent exits and orphans it.
		track_descendants(ours, &parents, descendants);
		sessions.retain(|&(pid, _)| !descendants.contains(&pid));
		let pids: Vec<u32> = sessions.iter().map(|&(pid, _)| pid).collect();
		Ok(select_roots(&pids, ours, &parents)
			.into_iter()
			.map(|root| {
				// A tree is playing when any session inside it is.
				let playing = sessions
					.iter()
					.any(|&(pid, active)| active && within_tree(pid, root, &parents));
				(root, playing)
			})
			.collect())
	}

	/// Process ids that own an audio session on any active output, each with whether that
	/// session is playing now. Expired sessions are left out. An output that cannot be
	/// inspected is skipped while another one can, but failing every output is an error: it
	/// would otherwise read as nothing playing. An output with no sessions is inspected fine.
	fn render_sessions() -> Result<Vec<(u32, bool)>> {
		let enumerator: IMMDeviceEnumerator =
			unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
		let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
		let mut sessions = Vec::new();
		let (mut inspected, mut failure) = (0_u32, None);
		for index in 0..unsafe { devices.GetCount()? } {
			match endpoint_sessions(&devices, index) {
				Ok(found) => {
					inspected += 1;
					sessions.extend(found);
				}
				Err(error) => failure = Some(error),
			}
		}
		match failure {
			Some(error) if inspected == 0 => Err(error),
			_ => Ok(sessions),
		}
	}

	/// The sessions of one output as process id and whether the session is playing now.
	fn endpoint_sessions(devices: &IMMDeviceCollection, index: u32) -> Result<Vec<(u32, bool)>> {
		let device = unsafe { devices.Item(index)? };
		let manager = unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)? };
		let sessions = unsafe { manager.GetSessionEnumerator()? };
		let mut found = Vec::new();
		for session in 0..unsafe { sessions.GetCount()? } {
			// A session can end between the count and the read, so one that cannot be read is skipped.
			let Ok(control) = (unsafe { sessions.GetSession(session) }) else {
				continue;
			};
			let Ok(control) = control.cast::<IAudioSessionControl2>() else {
				continue;
			};
			// S_OK means the shared system-sounds session, which has no application.
			if unsafe { control.IsSystemSoundsSession() }.0 == 0 {
				continue;
			}
			let state = unsafe { control.GetState() }.ok();
			if state == Some(AudioSessionStateExpired) {
				continue;
			}
			if let Ok(pid) = unsafe { control.GetProcessId() } {
				found.push((pid, state == Some(AudioSessionStateActive)));
			}
		}
		Ok(found)
	}

	/// A snapshot of every process's parent id. A snapshot that cannot be taken or read in full
	/// is an error, because a missing link could let Serein's own audio through.
	fn process_parents() -> Result<HashMap<u32, u32>> {
		let mut parents = HashMap::new();
		let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? };
		// The owned handle closes the snapshot on every path.
		let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.0) };
		let handle = HANDLE(snapshot.as_raw_handle());
		// PROCESSENTRY32W is plain data for which all-zero is valid; dwSize is the version tag.
		let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
		entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
		unsafe { Process32FirstW(handle, &mut entry)? };
		loop {
			parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
			match unsafe { Process32NextW(handle, &mut entry) } {
				Ok(()) => {}
				// The list ends with ERROR_NO_MORE_FILES; any other error is a truncated snapshot.
				Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) => break,
				Err(error) => return Err(error),
			}
		}
		Ok(parents)
	}

	/// Activates a process loopback audio client, waiting up to `ACTIVATION_TIMEOUT` and giving up early when the
	/// sink stops. The Windows error from a refused activation is returned unchanged.
	fn activate(sink: &dyn Sink, pid: u32, mode: PROCESS_LOOPBACK_MODE) -> Result<IAudioClient> {
		// The whole-share capture activates alone. Lanes do not wait for each other: a stuck
		// application would otherwise hold up the one that is actually playing.
		static ACTIVATING: Mutex<()> = Mutex::new(());
		let _turn = if sink.serial_activation() {
			Some(loop {
				match ACTIVATING.try_lock() {
					Ok(guard) => break guard,
					Err(std::sync::TryLockError::Poisoned(poisoned)) => {
						break poisoned.into_inner();
					}
					Err(std::sync::TryLockError::WouldBlock) => {
						if sink.stopped() {
							return Err(E_FAIL.into());
						}
						std::thread::sleep(Duration::from_millis(20));
					}
				}
			})
		} else {
			None
		};
		let params = Arc::new(activation_params(pid, mode));
		let (send, done) = sync_channel(1);
		let completion: IActivateAudioInterfaceCompletionHandler = Completion {
			send,
			_params: params.clone(),
		}
		.into();
		// The blob points into `params`; PROPVARIANT's Rust Drop would call
		// PropVariantClear and try to free that Arc allocation with CoTaskMemFree.
		// The completion handler owns `params` for the whole async activation.
		let variant = ManuallyDrop::new(PROPVARIANT {
			Anonymous: PROPVARIANT_0 {
				Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
					vt: VT_BLOB,
					Anonymous: PROPVARIANT_0_0_0 {
						blob: BLOB {
							cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
							pBlobData: Arc::as_ptr(&params).cast_mut().cast(),
						},
					},
					..Default::default()
				}),
			},
		});
		// The native operation retains the agile completion handler and its immutable blob.
		let operation = unsafe {
			ActivateAudioInterfaceAsync(
				VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
				&IAudioClient::IID,
				Some(&*variant),
				&completion,
			)?
		};
		let deadline = Instant::now() + sink.activation_timeout();
		loop {
			if sink.stopped() || Instant::now() >= deadline {
				return Err(E_FAIL.into());
			}
			if done.recv_timeout(Duration::from_millis(50)).is_ok() {
				break;
			}
		}
		let mut status = HRESULT(0);
		let mut activated = None;
		// Completion has fired; GetActivateResult returns an owned COM reference.
		unsafe { operation.GetActivateResult(&mut status, &mut activated)? };
		drop(operation);
		status.ok()?;
		activated.ok_or(E_FAIL)?.cast()
	}

	/// Captures one process loopback into the sink until the sink stops or its encryption
	/// generation changes, which returns `Ok` so the caller can start a fresh capture.
	fn capture(
		sink: &dyn Sink,
		pid: u32,
		mode: PROCESS_LOOPBACK_MODE,
		started: &mut Option<SyncSender<()>>,
	) -> Result<()> {
		if sink.stopped() {
			return Err(E_FAIL.into());
		}
		let event = unsafe { CreateEventW(None, false, false, None)? };
		// Created before the client so it stays open until the client's COM references are released.
		let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
		let client = activate(sink, pid, mode)?;
		if sink.stopped() {
			return Err(E_FAIL.into());
		}
		let format = WAVEFORMATEX {
			wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
			nChannels: 2,
			nSamplesPerSec: 48_000,
			nAvgBytesPerSec: 48_000 * 8,
			nBlockAlign: 8,
			wBitsPerSample: 32,
			cbSize: 0,
		};
		// Native conversion gives stereo 48 kHz independently of physical output formats.
		unsafe {
			client.Initialize(
				AUDCLNT_SHAREMODE_SHARED,
				AUDCLNT_STREAMFLAGS_LOOPBACK
					| AUDCLNT_STREAMFLAGS_EVENTCALLBACK
					| AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
				0,
				0,
				&format,
				None,
			)?;
		}
		if unsafe { client.GetBufferSize()? } as usize > MAX_AUDIO_SAMPLES / 2 {
			return Err(E_FAIL.into());
		}
		let capture: IAudioCaptureClient = unsafe { client.GetService()? };
		unsafe { client.SetEventHandle(HANDLE(event.as_raw_handle()))? };
		if sink.stopped() {
			return Err(E_FAIL.into());
		}
		let epoch = sink.epoch();
		let mut metrics = Metrics::new(Scope::ScreenAudio);
		metrics.checkpoint();
		let start = metrics.start();
		unsafe { client.Start()? };
		metrics.finish(Stage::CaptureRestart, start);
		metrics.checkpoint();
		sink.running();
		if let Some(started) = started.take() {
			let _ = started.try_send(());
		}
		let result = packets(sink, &capture, &event, epoch, &mut metrics);
		// Always stop before releasing the capture client and event, including packet errors.
		let stopped = unsafe { client.Stop() };
		result.and(stopped)
	}

	/// Drains capture packets into the sink in bounded batches until it stops or the epoch
	/// changes. A malformed packet is an error.
	fn packets(
		sink: &dyn Sink,
		capture: &IAudioCaptureClient,
		event: &OwnedHandle,
		epoch: u64,
		metrics: &mut Metrics,
	) -> Result<()> {
		while !sink.stopped() {
			if sink.restarts_on_epoch() && sink.epoch() != epoch {
				// Stop/Reset/Start can leave process loopback permanently empty on Windows.
				// Recreate it instead; the old client's immutable epoch prevents buffered
				// audio from being relabeled across the encryption transition.
				metrics.poll(true, 0, false, 0);
				metrics.checkpoint();
				return Ok(());
			}
			match unsafe { WaitForSingleObject(HANDLE(event.as_raw_handle()), 50) } {
				WAIT_TIMEOUT => {
					metrics.poll(false, 0, true, 0);
					continue;
				}
				WAIT_OBJECT_0 => {}
				_ => return Err(E_FAIL.into()),
			}
			let mut dropped = 0;
			// Bound work per wakeup as well as the bytes in each packet.
			for _ in 0..32 {
				if sink.stopped() || unsafe { capture.GetNextPacketSize()? } == 0 {
					break;
				}
				let start = metrics.start();
				let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0, 0);
				unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)? };
				metrics.finish(Stage::CaptureRead, start);
				let count = (frames as usize).saturating_mul(2);
				let valid = count <= MAX_AUDIO_SAMPLES
					&& (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0
						|| (!data.is_null() && data.align_offset(align_of::<f32>()) == 0));
				// A lane that never restarts labels each packet with the generation current now.
				let epoch = if sink.restarts_on_epoch() {
					epoch
				} else {
					sink.epoch()
				};
				if valid && count != 0 {
					let start = metrics.start();
					let queued = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
						sink.push(&[0.0; MAX_AUDIO_SAMPLES][..count], epoch)
					} else {
						// WASAPI guarantees frames of the initialized format until ReleaseBuffer.
						sink.push(
							unsafe { std::slice::from_raw_parts(data.cast::<f32>(), count) },
							epoch,
						)
					};
					if queued {
						metrics.finish(Stage::CaptureQueue, start);
					} else {
						dropped += 1;
					}
				}
				unsafe { capture.ReleaseBuffer(frames)? };
				if !valid {
					return Err(E_FAIL.into());
				}
			}
			metrics.poll(false, dropped, false, 0);
		}
		Ok(())
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		/// A parent map from `(process, parent)` pairs.
		fn parents(links: &[(u32, u32)]) -> HashMap<u32, u32> {
			links.iter().copied().collect()
		}

		/// Include and exclude requests must reach the activation parameters unchanged.
		#[test]
		fn activation_selects_the_requested_loopback_mode() {
			let params = activation_params(42, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE);
			assert_eq!(
				params.ActivationType,
				AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK
			);
			let process = unsafe { params.Anonymous.ProcessLoopbackParams };
			assert_eq!(process.TargetProcessId, 42);
			assert_eq!(
				process.ProcessLoopbackMode,
				PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
			);
			let include = activation_params(7, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE);
			let process = unsafe { include.Anonymous.ProcessLoopbackParams };
			assert_eq!(
				process.ProcessLoopbackMode,
				PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
			);
		}

		/// Serein, what it started and what started it are never selected for capture.
		#[test]
		fn application_capture_never_includes_serein_or_its_ancestors() {
			// 1 launcher -> 10 Serein -> 11 webview; 2 browser -> 3 browser helper; 4 game.
			let tree = parents(&[(10, 1), (11, 10), (3, 2), (2, 0), (4, 0), (1, 0)]);
			// Serein, its child, and the launcher Serein descends from are all skipped.
			assert_eq!(select_roots(&[10, 11, 1, 4, 0], 10, &tree), vec![4]);
			// A helper inside an already selected browser tree is not captured twice.
			assert_eq!(select_roots(&[3, 2, 4], 10, &tree), vec![2, 4]);
			// Without its parent selected, the helper is its own root.
			assert_eq!(select_roots(&[3], 10, &tree), vec![3]);
		}

		/// Stale parent ids can loop after id reuse. Unrelated applications in a loop must stay
		/// captured, and a loop above Serein must never let one of its members through.
		#[test]
		fn stale_parent_cycles_neither_hang_nor_hide_applications() {
			let tree = parents(&[(5, 6), (6, 5)]);
			assert_eq!(select_roots(&[5, 6], 10, &tree), vec![5, 6]);
			// Serein 10 descends from 1, which sits in a loop with 2: both are ancestors.
			let tree = parents(&[(10, 1), (1, 2), (2, 1), (4, 0)]);
			assert_eq!(select_roots(&[1, 2, 4], 10, &tree), vec![4]);
		}

		/// A descendant of Serein stays excluded after its parent exits and orphans it.
		#[test]
		fn orphaned_descendants_stay_excluded() {
			let mut known = HashSet::new();
			// 10 is Serein; 11 its helper; 12 started by the helper; 4 is unrelated.
			track_descendants(
				10,
				&parents(&[(10, 1), (11, 10), (12, 11), (4, 0)]),
				&mut known,
			);
			assert_eq!(known, HashSet::from([11, 12]));
			// The helper exits: 12 keeps its stale link to 11, and 13 starts under 12.
			track_descendants(
				10,
				&parents(&[(10, 1), (12, 11), (13, 12), (4, 0)]),
				&mut known,
			);
			assert_eq!(known, HashSet::from([12, 13]));
			// An unrelated process never joins.
			assert!(!known.contains(&4));
			// A recorded process that has exited is forgotten.
			track_descendants(10, &parents(&[(10, 1), (4, 0)]), &mut known);
			assert!(known.is_empty());
		}

		/// Lanes sum into one frame, and audio from another epoch is discarded.
		#[test]
		fn mixed_lanes_sum_and_discard_other_epochs() {
			let mut first = Buffer::new();
			let mut second = Buffer::new();
			first.append(&[0.25, -0.25, 0.5, 0.5], 3, 8);
			second.append(&[0.25, 0.25], 3, 8);
			let mut frame = [0.0; 4];
			first.mix_into(&mut frame, 3, 0);
			second.mix_into(&mut frame, 3, 0);
			assert_eq!(frame, [0.5, 0.0, 0.5, 0.5]);
			// Audio captured before an encryption transition never leaves under the new epoch.
			let mut stale = Buffer::new();
			stale.append(&[1.0, 1.0], 3, 8);
			let mut frame = [0.0; 2];
			stale.mix_into(&mut frame, 4, 0);
			assert_eq!(frame, [0.0, 0.0]);
			assert!(stale.samples.is_empty());
			// A lane waits for its fill before playing, and again after running dry.
			let mut lane = Buffer::new();
			let mut frame = [0.0; 2];
			lane.append(&[1.0, 1.0], 3, 16);
			lane.mix_into(&mut frame, 3, 4);
			assert_eq!(frame, [0.0, 0.0]);
			lane.append(&[2.0, 2.0], 3, 16);
			lane.mix_into(&mut frame, 3, 4);
			assert_eq!(frame, [1.0, 1.0]);
			let mut frame = [0.0; 2];
			lane.mix_into(&mut frame, 3, 4);
			assert_eq!(frame, [2.0, 2.0]);
			let mut frame = [0.0; 2];
			lane.mix_into(&mut frame, 3, 4);
			assert_eq!(frame, [0.0, 0.0]);
			assert!(!lane.primed);
			// A lane that falls behind keeps only the newest samples.
			let mut behind = Buffer::new();
			behind.append(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0], 3, 4);
			assert_eq!(behind.samples, [2.0, 2.0, 3.0, 3.0]);
		}
	}
}

/// The share's audio handoff: the stream sender plus the flags that gate and end capture.
#[derive(Clone)]
struct Samples {
	send: Sender<AudioChunk>,
	stop: Arc<AtomicBool>,
	ready: Arc<AtomicBool>,
	epoch: Arc<AtomicU64>,
	failed: Arc<AtomicBool>,
	/// First Windows error code that ended capture, 0 when none.
	#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
	code: Arc<AtomicI32>,
}

impl Samples {
	/// Pushes under the current epoch, for tests.
	#[cfg(test)]
	fn push(&self, data: &[f32]) {
		let epoch = self.epoch.load(Ordering::Acquire);
		self.push_at(data, epoch);
	}

	/// Sanitizes and sends one chunk if the share is ready, running and still on `epoch`.
	/// Malformed sizes end capture. Returns whether the chunk was sent; a full or stale
	/// stream refuses it without queueing.
	fn push_at(&self, data: &[f32], epoch: u64) -> bool {
		if self.stop.load(Ordering::Acquire)
			|| !self.ready.load(Ordering::Acquire)
			|| epoch != self.epoch.load(Ordering::Acquire)
		{
			return false;
		}
		if data.len() > MAX_AUDIO_SAMPLES || !data.len().is_multiple_of(2) {
			self.failed.store(true, Ordering::Release);
			self.stop.store(true, Ordering::Release);
			return false;
		}
		if data.is_empty() {
			return false;
		}
		// Reserve first: a stalled transport must not allocate for dropped packets.
		if let Ok(permit) = self.send.try_reserve() {
			let samples = data
				.iter()
				.map(|&sample| {
					if sample.is_finite() {
						sample.clamp(-1.0, 1.0)
					} else {
						0.0
					}
				})
				.collect();
			if !self.stop.load(Ordering::Acquire)
				&& self.ready.load(Ordering::Acquire)
				&& epoch == self.epoch.load(Ordering::Acquire)
			{
				permit.send(AudioChunk { samples, epoch });
				return true;
			}
		}
		false
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Samples are dropped until ready, sanitized, and bounded in size.
	#[test]
	fn loopback_samples_are_gated_sanitized_and_bounded() {
		let (send, mut receive) = tokio::sync::mpsc::channel(1);
		let samples = Samples {
			send,
			stop: Arc::new(AtomicBool::new(false)),
			ready: Arc::new(AtomicBool::new(false)),
			epoch: Arc::new(AtomicU64::new(7)),
			failed: Arc::new(AtomicBool::new(false)),
			code: Arc::new(AtomicI32::new(0)),
		};
		assert!(!samples.push_at(&[0.25, -0.25], 7));
		assert!(receive.try_recv().is_err());
		samples.ready.store(true, Ordering::Release);
		assert!(samples.push_at(&[f32::NAN, f32::INFINITY, -2.0, 2.0], 7));
		assert!(!samples.push_at(&[0.5, -0.5], 7)); // Full queue drops the new chunk.
		samples.epoch.store(8, Ordering::Release);
		let chunk = receive.try_recv().unwrap();
		assert_eq!(chunk.samples, vec![0.0, 0.0, -1.0, 1.0]);
		assert_eq!(chunk.epoch, 7); // Queued samples retain their capture generation.
		assert!(receive.try_recv().is_err());
		assert!(!samples.push_at(&[0.25, -0.25], 7)); // A native wakeup across rekey is stale.
		assert!(receive.try_recv().is_err());
		samples.push(&vec![0.0; MAX_AUDIO_SAMPLES]);
		let chunk = receive.try_recv().unwrap();
		assert_eq!(chunk.samples.len(), MAX_AUDIO_SAMPLES);
		assert_eq!(chunk.epoch, 8);
		samples.stop.store(true, Ordering::Release);
		samples.push(&[0.25, -0.25]);
		assert!(receive.try_recv().is_err());
		samples.stop.store(false, Ordering::Release);
		samples.push(&vec![0.0; MAX_AUDIO_SAMPLES + 2]);
		assert!(samples.failed.load(Ordering::Acquire));
		assert!(samples.stop.load(Ordering::Acquire));
		assert!(receive.try_recv().is_err());
	}
}
