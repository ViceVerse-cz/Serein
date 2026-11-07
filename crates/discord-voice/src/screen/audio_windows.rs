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
	/// Starts capture and waits for the first sign that audio can flow. A startup failure
	/// names the Windows error code when the worker recorded one.
	pub(super) fn start(
		send: Sender<AudioChunk>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		audio_epoch: Arc<AtomicU64>,
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
				if let Err(error) = native::run(&input, started) {
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
			// Still starting after the deadline: no native error exists to report.
			Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(Cow::Borrowed(
				"System audio could not start; check your output device or share without audio",
			)),
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
//   and runs one include-mode loopback per remaining application. A mixer sums their lanes
//   into the same 48 kHz stereo stream. The whole output is never captured.
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
				Threading::{CreateEventW, WaitForSingleObject},
				Variant::VT_BLOB,
			},
		},
		core::{HRESULT, Interface, Ref, Result, implement},
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
	/// A lane that ended is not restarted for this long, so a failing app cannot spin.
	const LANE_COOLDOWN: Duration = Duration::from_secs(2);
	/// Capture is only declared failed after this long with failures pending and no lane
	/// working, so a restart after a device change or an app's own stream restart recovers.
	const ACTIVATION_GRACE: Duration = Duration::from_secs(8);

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
	}

	impl Buffer {
		/// An empty buffer that takes the epoch of its first samples.
		fn new() -> Self {
			Self {
				epoch: 0,
				samples: VecDeque::new(),
			}
		}

		/// Appends capture data, dropping the oldest samples beyond the latency bound.
		fn append(&mut self, data: &[f32], epoch: u64, cap: usize) {
			if self.epoch != epoch {
				self.samples.clear();
				self.epoch = epoch;
			}
			self.samples.extend(data.iter().copied());
			let excess = self.samples.len().saturating_sub(cap);
			if excess > 0 {
				self.samples.drain(..excess);
			}
		}

		/// Adds up to one tick into `frame`. Samples captured under another epoch are
		/// discarded so audio is never relabeled across an encryption transition.
		fn mix_into(&mut self, frame: &mut [f32], epoch: u64) {
			if self.epoch != epoch {
				self.samples.clear();
				return;
			}
			let count = self.samples.len().min(frame.len());
			for (slot, sample) in frame.iter_mut().zip(self.samples.drain(..count)) {
				*slot += sample;
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
	// pure-end

	/// One application's include-mode capture, drained by the mixer.
	struct Lane {
		input: Samples,
		retired: AtomicBool,
		running: AtomicBool,
		/// The error that ended this lane, 0 when it ended normally.
		error: AtomicI32,
		buffer: Mutex<Buffer>,
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
			if self.stopped()
				|| !self.input.ready.load(Ordering::Acquire)
				|| epoch != self.epoch()
				|| data.len() > MAX_AUDIO_SAMPLES
				|| !data.len().is_multiple_of(2)
			{
				return false;
			}
			self.buffer
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.append(data, epoch, LANE_CAP_SAMPLES);
			true
		}
		/// Marks the lane as delivering, which the supervisor reads as a working capture.
		fn running(&self) {
			self.running.store(true, Ordering::Release);
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
	pub(super) fn run(input: &Samples, started: SyncSender<()>) -> Result<()> {
		// This dedicated thread initializes and releases all COM objects in one MTA.
		unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
		let result = if build_number() >= EXCLUDE_MIN_BUILD {
			exclude_tree(input, started)
		} else {
			per_application(input, started)
		};
		unsafe { CoUninitialize() };
		result
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
		let listed = ready.recv_timeout(Duration::from_secs(3)).is_ok();
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
					.mix_into(&mut frame, epoch);
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
		let mut errors = 0_u32;
		// Playing applications whose capture failed for a reason other than access, with the
		// error, and since when nothing has worked while some of them were failing.
		let mut blamed: HashMap<u32, i32> = HashMap::new();
		let mut failing_since: Option<Instant> = None;
		// Since when no lane thread could be created.
		let mut unstartable: Option<Instant> = None;
		// Applications already named in the opt-in report, so a restarting lane names itself once.
		let mut announced: HashSet<u32> = HashSet::new();
		let result = (|| -> Result<()> {
			while !halt.load(Ordering::Acquire) && !input.stopped() {
				let wanted = match applications(ours) {
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
						cooldown.insert(pid, Instant::now() + LANE_COOLDOWN);
					}
				}
				let healthy = running
					.values()
					.any(|entry| entry.lane.running.load(Ordering::Acquire));
				// An idle application has nothing to capture, so its failure is not one.
				blamed.retain(|pid, _| wanted.get(pid) == Some(&true));
				if healthy || blamed.is_empty() {
					// Something works, or nothing playing has failed: any later failure starts afresh.
					failing_since = None;
					if healthy {
						blamed.clear();
					}
				} else {
					let since = *failing_since.get_or_insert_with(Instant::now);
					// Failures return instantly while a working capture takes a moment to start,
					// so judge only after a grace period, with nothing still starting.
					if running.is_empty() && since.elapsed() >= ACTIVATION_GRACE {
						// Playing applications cannot be captured; report why instead of sharing silence.
						let code = blamed
							.iter()
							.min_by_key(|(pid, _)| **pid)
							.map_or(E_FAIL.0, |(_, &code)| code);
						return Err(HRESULT(code).into());
					}
				}
				let now = Instant::now();
				cooldown.retain(|_, until| *until > now);
				// Playing applications first, so idle sessions cannot hold every lane.
				let mut order: Vec<(u32, bool)> = wanted
					.iter()
					.map(|(&pid, &active)| (pid, active))
					.collect();
				order.sort_by_key(|&(pid, active)| (!active, pid));
				let mut spawn_error = None;
				for (pid, active) in order {
					if running.contains_key(&pid) || cooldown.contains_key(&pid) {
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
							break;
						};
						if let Some(entry) = running.remove(&idle) {
							retire(&entry, lanes);
							retiring.push(entry.thread);
						}
					}
					let lane = Arc::new(Lane {
						input: input.clone(),
						retired: AtomicBool::new(false),
						running: AtomicBool::new(false),
						error: AtomicI32::new(0),
						buffer: Mutex::new(Buffer::new()),
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
					running.insert(pid, Running { lane, thread });
					if announced.insert(pid)
						&& std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|value| value == "1")
					{
						eprintln!("[Serein voice Screen] app_capture pid={pid} playing={active}");
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
	fn applications(ours: u32) -> Result<HashMap<u32, bool>> {
		let sessions = render_sessions()?;
		let parents = process_parents()?;
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

	/// Activates a process loopback audio client, waiting up to 3 s and giving up early when the
	/// sink stops. The Windows error from a refused activation is returned unchanged.
	fn activate(sink: &dyn Sink, pid: u32, mode: PROCESS_LOOPBACK_MODE) -> Result<IAudioClient> {
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
		let deadline = Instant::now() + Duration::from_secs(3);
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
		Ok(activated.ok_or(E_FAIL)?.cast()?)
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
			if sink.epoch() != epoch {
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

		/// Lanes sum into one frame, and audio from another epoch is discarded.
		#[test]
		fn mixed_lanes_sum_and_discard_other_epochs() {
			let mut first = Buffer::new();
			let mut second = Buffer::new();
			first.append(&[0.25, -0.25, 0.5, 0.5], 3, 8);
			second.append(&[0.25, 0.25], 3, 8);
			let mut frame = [0.0; 4];
			first.mix_into(&mut frame, 3);
			second.mix_into(&mut frame, 3);
			assert_eq!(frame, [0.5, 0.0, 0.5, 0.5]);
			// Audio captured before an encryption transition never leaves under the new epoch.
			let mut stale = Buffer::new();
			stale.append(&[1.0, 1.0], 3, 8);
			let mut frame = [0.0; 2];
			stale.mix_into(&mut frame, 4);
			assert_eq!(frame, [0.0, 0.0]);
			assert!(stale.samples.is_empty());
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
