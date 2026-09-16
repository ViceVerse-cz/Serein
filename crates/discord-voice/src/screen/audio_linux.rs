//! Isolated application playback through PulseAudio (including PipeWire-Pulse).
//! The ScreenCast portal remote grants video only; this uses the existing Pulse socket.
#![allow(unsafe_code)] // Narrow, worker-owned PulseAudio C boundary; no callbacks outlive their owners.
use super::{AudioChunk, MAX_AUDIO_SAMPLES};
use libpulse_sys as pulse;
use std::{
	cell::Cell,
	collections::VecDeque,
	ffi::{CStr, CString, c_void},
	ptr::{null, null_mut},
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};

const UNAVAILABLE: &str =
	"Isolated application audio is unavailable; check PulseAudio/PipeWire, or share without audio";
const MAX_INPUTS: usize = 32;
const MAX_LISTED: usize = 256;
const FRAME_SAMPLES: usize = 960; // 10 ms, stereo 48 kHz.
const TICK: Duration = Duration::from_millis(10);
const TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct Worker {
	thread: Option<std::thread::JoinHandle<Result<(), &'static str>>>,
	stop: Arc<AtomicBool>,
}

impl Worker {
	pub(super) fn start(
		send: tokio::sync::mpsc::Sender<AudioChunk>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		epoch: Arc<AtomicU64>,
	) -> Result<Self, &'static str> {
		let worker_stop = stop.clone();
		let thread = std::thread::Builder::new()
			.name("screen-audio".into())
			.spawn(move || {
				// No device/server access for a cancelled share. All native work stays off video/UI.
				if worker_stop.load(Ordering::Acquire) || send.is_closed() {
					return Ok(());
				}
				run(send, &worker_stop, &ready, &epoch)
			})
			.map_err(|_| UNAVAILABLE)?;
		Ok(Self {
			thread: Some(thread),
			stop,
		})
	}
	pub(super) fn result(&mut self) -> Option<Result<(), &'static str>> {
		if !self.thread.as_ref()?.is_finished() {
			return None;
		}
		Some(self.thread.take()?.join().unwrap_or(Err(UNAVAILABLE)))
	}
}

impl Drop for Worker {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Release);
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

// Require an identifiable application, not a system/loopback stream that could contain the call.
// Binary matching also covers Flatpak's different host and sandbox PID namespaces.
#[derive(Clone)]
struct OwnApplication {
	pid: u32,
	binary: String,
}
impl OwnApplication {
	fn current() -> Result<Self, &'static str> {
		let path = std::env::current_exe().map_err(|_| UNAVAILABLE)?;
		let binary = path
			.file_name()
			.and_then(|v| v.to_str())
			.filter(|v| v.len() <= 256)
			.ok_or(UNAVAILABLE)?;
		Ok(Self {
			pid: std::process::id(),
			binary: binary.to_owned(),
		})
	}
	fn allows(
		&self,
		pid: Option<&str>,
		binary: Option<&str>,
		name: Option<&str>,
		app_id: Option<&str>,
	) -> bool {
		let Some(pid) = pid
			.filter(|v| v.len() <= 10)
			.and_then(|v| v.parse::<u32>().ok())
		else {
			return false;
		};
		let Some(binary) = binary.filter(|v| !v.is_empty() && v.len() <= 256) else {
			return false;
		};
		pid != 0
			&& pid != self.pid
			&& !binary.eq_ignore_ascii_case(&self.binary)
			&& ![Some(binary), name, app_id].into_iter().flatten().any(|v| {
				v.len() > 256
					|| v.to_ascii_lowercase().contains("serein")
					|| v.eq_ignore_ascii_case("rustcord")
			})
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Input {
	index: u32,
	client: u32,
	sink: u32,
	pid: u32,
	binary: String,
	serial: Option<u64>,
	monitor: Option<CString>,
}

fn property<'a>(info: &'a pulse::pa_sink_input_info, key: &CStr) -> Option<&'a str> {
	let mut data = null();
	let mut size = 0;
	// SAFETY: callback-owned info/proplist stays alive until this function returns; Pulse
	// supplies data and its length, checked before constructing a borrowed byte slice.
	unsafe {
		if info.proplist.is_null()
			|| pulse::pa_proplist_get(info.proplist, key.as_ptr(), &mut data, &mut size) < 0
			|| data.is_null()
			|| size == 0
			|| size > 257
		{
			return None;
		}
		std::str::from_utf8(std::slice::from_raw_parts(data.cast::<u8>(), size).strip_suffix(&[0])?)
			.ok()
	}
}

fn input(info: &pulse::pa_sink_input_info, own: &OwnApplication) -> Option<Input> {
	let pid = property(info, c"application.process.id");
	let binary = property(info, c"application.process.binary");
	if info.index == pulse::PA_INVALID_INDEX
		|| info.sink == pulse::PA_INVALID_INDEX
		|| info.client == pulse::PA_INVALID_INDEX
		|| !own.allows(
			pid,
			binary,
			property(info, c"application.name"),
			property(info, c"application.id"),
		) || property(info, c"flatpak.app-id")
		.is_some_and(|v| v.eq_ignore_ascii_case("cz.viceverse.serein"))
	{
		return None;
	}
	Some(Input {
		index: info.index,
		client: info.client,
		sink: info.sink,
		pid: pid?.parse().ok()?,
		binary: binary?.to_owned(),
		serial: property(info, c"object.serial").and_then(|v| v.parse().ok()),
		monitor: None,
	})
}

struct Listing {
	own: OwnApplication,
	inputs: Vec<Input>,
	monitors: Vec<(u32, CString)>,
	seen_inputs: usize,
	seen_sinks: usize,
	inputs_done: bool,
	sinks_done: bool,
	failed: bool,
}
struct Enumeration {
	inputs: *mut pulse::pa_operation,
	sinks: *mut pulse::pa_operation,
	listing: Box<Listing>,
	started: Instant,
	revision: u64,
}

fn resolve_monitors(inputs: &mut [Input], monitors: &[(u32, CString)]) -> Result<(), &'static str> {
	for input in inputs {
		input.monitor = monitors
			.iter()
			.find_map(|(sink, monitor)| (*sink == input.sink).then(|| monitor.clone()));
		if input.monitor.is_none() {
			return Err(UNAVAILABLE);
		}
	}
	Ok(())
}

impl Enumeration {
	fn start(native: &Native, own: &OwnApplication, revision: u64) -> Result<Self, &'static str> {
		let mut listing = Box::new(Listing {
			own: own.clone(),
			inputs: Vec::with_capacity(MAX_INPUTS),
			monitors: Vec::with_capacity(MAX_INPUTS),
			seen_inputs: 0,
			seen_sinks: 0,
			inputs_done: false,
			sinks_done: false,
			failed: false,
		});
		// SAFETY: this stable Box lives until Drop cancels/unrefs both operations; callbacks
		// run only during this worker's mainloop dispatch, never concurrently with Rust reads.
		let inputs = unsafe {
			pulse::pa_context_get_sink_input_info_list(
				native.context,
				Some(listed_input),
				(&mut *listing as *mut Listing).cast(),
			)
		};
		if inputs.is_null() {
			return Err(UNAVAILABLE);
		}
		let sinks = unsafe {
			pulse::pa_context_get_sink_info_list(
				native.context,
				Some(listed_sink),
				(&mut *listing as *mut Listing).cast(),
			)
		};
		if sinks.is_null() {
			unsafe {
				pulse::pa_operation_cancel(inputs);
				pulse::pa_operation_unref(inputs);
			}
			return Err(UNAVAILABLE);
		}
		Ok(Self {
			inputs,
			sinks,
			listing,
			started: Instant::now(),
			revision,
		})
	}
	fn take(&mut self) -> Result<Option<Vec<Input>>, &'static str> {
		let listing = &mut self.listing;
		if listing.failed || self.started.elapsed() > TIMEOUT {
			return Err(UNAVAILABLE);
		}
		if !listing.inputs_done || !listing.sinks_done {
			return Ok(None);
		}
		resolve_monitors(&mut listing.inputs, &listing.monitors)?;
		Ok(Some(std::mem::take(&mut listing.inputs)))
	}
}
impl Drop for Enumeration {
	fn drop(&mut self) {
		// SAFETY: cancelling both operations prevents callbacks before userdata is freed.
		unsafe {
			pulse::pa_operation_cancel(self.inputs);
			pulse::pa_operation_unref(self.inputs);
			pulse::pa_operation_cancel(self.sinks);
			pulse::pa_operation_unref(self.sinks);
		}
	}
}

extern "C" fn listed_input(
	_: *mut pulse::pa_context,
	info: *const pulse::pa_sink_input_info,
	eol: i32,
	data: *mut c_void,
) {
	// SAFETY: registered with Enumeration's stable Box and cancelled before freeing it.
	let result = unsafe { &mut *data.cast::<Listing>() };
	if eol != 0 {
		result.failed |= eol < 0;
		result.inputs.sort_by_key(|v| v.index);
		result.inputs_done = true;
		return;
	}
	result.seen_inputs = result.seen_inputs.saturating_add(1);
	if result.seen_inputs > MAX_LISTED || info.is_null() {
		result.failed = true;
		return;
	}
	// SAFETY: Pulse guarantees info is valid for the duration of this callback.
	if let Some(input) = input(unsafe { &*info }, &result.own) {
		if result.inputs.len() == MAX_INPUTS {
			result.failed = true;
		} else {
			result.inputs.push(input);
		}
	}
}

extern "C" fn listed_sink(
	_: *mut pulse::pa_context,
	info: *const pulse::pa_sink_info,
	eol: i32,
	data: *mut c_void,
) {
	// SAFETY: registered with Enumeration's stable Box and cancelled before freeing it.
	let result = unsafe { &mut *data.cast::<Listing>() };
	if eol != 0 {
		result.failed |= eol < 0;
		result.monitors.sort_by_key(|v| v.0);
		result.sinks_done = true;
		return;
	}
	result.seen_sinks = result.seen_sinks.saturating_add(1);
	if result.seen_sinks > MAX_LISTED || info.is_null() {
		result.failed = true;
		return;
	}
	// SAFETY: Pulse owns the sink info and monitor name for this callback. Copy a bounded
	// C string because pa_stream_connect_record uses it after enumeration has completed.
	let info = unsafe { &*info };
	if info.index == pulse::PA_INVALID_INDEX || info.monitor_source_name.is_null() {
		return;
	}
	let monitor = unsafe { CStr::from_ptr(info.monitor_source_name) };
	if monitor.to_bytes().is_empty() || monitor.to_bytes().len() > 256 {
		return;
	}
	if result.monitors.len() == MAX_LISTED {
		result.failed = true;
	} else {
		result.monitors.push((info.index, monitor.to_owned()));
	}
}

#[derive(Default)]
struct Events {
	revision: Cell<u64>,
	subscribed: Cell<Option<bool>>,
}
struct Native {
	mainloop: *mut pulse::pa_mainloop,
	context: *mut pulse::pa_context,
	subscription: *mut pulse::pa_operation,
	events: Box<Events>,
}
impl Native {
	fn new() -> Result<Self, &'static str> {
		// SAFETY: all native handles are exclusively owned on this worker; Drop releases
		// partially initialized objects too. No Pulse daemon is spawned by this connection.
		unsafe {
			let mut native = Self {
				mainloop: pulse::pa_mainloop_new(),
				context: null_mut(),
				subscription: null_mut(),
				events: Box::default(),
			};
			if native.mainloop.is_null() {
				return Err(UNAVAILABLE);
			}
			native.context = pulse::pa_context_new(
				pulse::pa_mainloop_get_api(native.mainloop),
				c"Serein screen audio".as_ptr(),
			);
			if native.context.is_null()
				|| pulse::pa_context_connect(
					native.context,
					null(),
					pulse::PA_CONTEXT_NOAUTOSPAWN,
					null(),
				) < 0
			{
				return Err(UNAVAILABLE);
			}
			pulse::pa_context_set_subscribe_callback(
				native.context,
				Some(changed),
				(&mut *native.events as *mut Events).cast(),
			);
			Ok(native)
		}
	}
	fn poll(&mut self, timeout_ms: i32) -> Result<pulse::pa_context_state_t, &'static str> {
		// SAFETY: the mainloop/context remain valid; only this call dispatches callbacks.
		unsafe {
			if pulse::pa_mainloop_prepare(self.mainloop, timeout_ms) < 0
				|| pulse::pa_mainloop_poll(self.mainloop) < 0
				|| pulse::pa_mainloop_dispatch(self.mainloop) < 0
			{
				return Err(UNAVAILABLE);
			}
			Ok(pulse::pa_context_get_state(self.context))
		}
	}
	fn subscribe(&mut self) -> Result<(), &'static str> {
		if self.subscription.is_null() {
			// SAFETY: stable userdata is detached/cancelled in Drop before freeing events.
			self.subscription = unsafe {
				pulse::pa_context_subscribe(
					self.context,
					pulse::PA_SUBSCRIPTION_MASK_SINK_INPUT,
					Some(subscribed),
					(&mut *self.events as *mut Events).cast(),
				)
			};
			if self.subscription.is_null() {
				return Err(UNAVAILABLE);
			}
		}
		Ok(())
	}
}
impl Drop for Native {
	fn drop(&mut self) {
		// SAFETY: streams/enumerations have already been dropped by the worker. Cancel
		// and detach callbacks before releasing userdata and free the mainloop last.
		unsafe {
			if !self.subscription.is_null() {
				pulse::pa_operation_cancel(self.subscription);
				pulse::pa_operation_unref(self.subscription);
			}
			if !self.context.is_null() {
				pulse::pa_context_set_subscribe_callback(self.context, None, null_mut());
				pulse::pa_context_disconnect(self.context);
				pulse::pa_context_unref(self.context);
			}
			if !self.mainloop.is_null() {
				pulse::pa_mainloop_free(self.mainloop);
			}
		}
	}
}
extern "C" fn changed(
	_: *mut pulse::pa_context,
	_: pulse::pa_subscription_event_type_t,
	_: u32,
	data: *mut c_void,
) {
	// SAFETY: Native owns this stable userdata and detaches the callback before Drop.
	let events = unsafe { &*data.cast::<Events>() };
	events.revision.set(events.revision.get().wrapping_add(1));
}
extern "C" fn subscribed(_: *mut pulse::pa_context, ok: i32, data: *mut c_void) {
	// SAFETY: Native owns this userdata and cancels the operation before Drop.
	unsafe { &*data.cast::<Events>() }
		.subscribed
		.set(Some(ok != 0));
}

struct Capture {
	stream: *mut pulse::pa_stream,
	pending: Pending,
	input: Input,
}
impl Capture {
	fn start(native: &Native, input: Input, epoch: u64) -> Result<Self, &'static str> {
		if input.index == pulse::PA_INVALID_INDEX
			|| input.sink == pulse::PA_INVALID_INDEX
			|| input.monitor.is_none()
		{
			return Err(UNAVAILABLE);
		}
		let spec = pulse::pa_sample_spec {
			format: pulse::PA_SAMPLE_FLOAT32LE,
			channels: 2,
			rate: 48_000,
		};
		// SAFETY: Pulse copies the spec and supplies a stereo default map; Native outlives
		// this stream, and Capture immediately owns the returned handle on every path.
		let stream = unsafe {
			pulse::pa_stream_new(
				native.context,
				c"Serein isolated screen audio".as_ptr(),
				&spec,
				null(),
			)
		};
		if stream.is_null() {
			return Err(UNAVAILABLE);
		}
		let capture = Self {
			stream,
			pending: Pending::new(epoch),
			input,
		};
		let attr = pulse::pa_buffer_attr {
			maxlength: (MAX_AUDIO_SAMPLES * 4) as u32,
			fragsize: (FRAME_SAMPLES * 4) as u32,
			tlength: u32::MAX,
			prebuf: u32::MAX,
			minreq: u32::MAX,
		};
		// Pulse requires the sink's monitor source before it can isolate one sink input.
		// DONT_MOVE forbids policy/device fallback to a microphone or another monitor.
		// SAFETY: valid unconnected stream. Set monitor BEFORE connecting; failure returns
		// without retrying a default source, so no microphone or whole-output fallback.
		unsafe {
			if pulse::pa_stream_set_monitor_stream(stream, capture.input.index) < 0
				|| pulse::pa_stream_connect_record(
					stream,
					capture.input.monitor.as_ref().unwrap().as_ptr(),
					&attr,
					pulse::PA_STREAM_DONT_MOVE | pulse::PA_STREAM_ADJUST_LATENCY,
				) < 0
			{
				return Err(UNAVAILABLE);
			}
		}
		Ok(capture)
	}
	fn state(&self) -> pulse::pa_stream_state_t {
		// SAFETY: self owns this live stream until Drop.
		unsafe { pulse::pa_stream_get_state(self.stream) }
	}
	fn read(&mut self) -> Result<(), &'static str> {
		// SAFETY: stream is Ready; returned attributes and peeked memory remain borrowed
		// from it until the next native operation. Lengths are checked before making slices.
		unsafe {
			let attr = pulse::pa_stream_get_buffer_attr(self.stream);
			if pulse::pa_stream_get_monitor_stream(self.stream) != self.input.index
				|| attr.is_null()
				|| (*attr).maxlength as usize > MAX_AUDIO_SAMPLES * 4
			{
				return Err(UNAVAILABLE);
			}
			for _ in 0..4 {
				let mut data = null();
				let mut size = 0;
				if pulse::pa_stream_peek(self.stream, &mut data, &mut size) < 0 {
					return Err(UNAVAILABLE);
				}
				if size == 0 {
					break;
				}
				if size > MAX_AUDIO_SAMPLES * 4 || !size.is_multiple_of(8) {
					return Err(UNAVAILABLE);
				}
				if data.is_null() {
					self.pending.hole(size)?;
				} else {
					self.pending
						.push(std::slice::from_raw_parts(data.cast::<u8>(), size))?;
				}
				if pulse::pa_stream_drop(self.stream) < 0 {
					return Err(UNAVAILABLE);
				}
			}
			Ok(())
		}
	}
}
impl Drop for Capture {
	fn drop(&mut self) {
		// SAFETY: uniquely owned stream; Native/context outlives it.
		unsafe {
			pulse::pa_stream_disconnect(self.stream);
			pulse::pa_stream_unref(self.stream);
		}
	}
}

struct Pending {
	samples: VecDeque<f32>,
	epoch: u64,
}
impl Pending {
	fn new(epoch: u64) -> Self {
		Self {
			samples: VecDeque::with_capacity(MAX_AUDIO_SAMPLES),
			epoch,
		}
	}
	fn reserve(&mut self, bytes: usize) -> Result<usize, &'static str> {
		if bytes > MAX_AUDIO_SAMPLES * 4 || !bytes.is_multiple_of(8) {
			return Err(UNAVAILABLE);
		}
		let samples = bytes / 4;
		let excess = (self.samples.len() + samples).saturating_sub(MAX_AUDIO_SAMPLES);
		self.samples.drain(..excess);
		Ok(samples)
	}
	fn hole(&mut self, bytes: usize) -> Result<(), &'static str> {
		let samples = self.reserve(bytes)?;
		self.samples.extend(std::iter::repeat_n(0.0, samples));
		Ok(())
	}
	fn push(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
		self.reserve(bytes.len())?;
		self.samples
			.extend(bytes.as_chunks::<4>().0.iter().map(|b| {
				let value = f32::from_le_bytes(*b);
				if value.is_finite() {
					value.clamp(-1.0, 1.0)
				} else {
					0.0
				}
			}));
		Ok(())
	}
	fn mix(&mut self, output: &mut [f32; FRAME_SAMPLES], epoch: u64) {
		if self.epoch != epoch {
			self.samples.clear();
			return;
		}
		for out in output {
			*out += self.samples.pop_front().unwrap_or(0.0);
		}
	}
}

fn run(
	send: tokio::sync::mpsc::Sender<AudioChunk>,
	stop: &AtomicBool,
	ready: &AtomicBool,
	epoch: &AtomicU64,
) -> Result<(), &'static str> {
	let own = OwnApplication::current()?;
	let mut native = Native::new()?;
	let mut listing: Option<Enumeration> = None;
	let mut captures: Vec<Capture> = Vec::new();
	let mut expected = Vec::new();
	// Applications whose monitor failed. Their audio is dropped rather than ending the share.
	let mut excluded: Vec<Input> = Vec::new();
	let mut active_revision = u64::MAX;
	let mut active_epoch = epoch.load(Ordering::Acquire);
	let mut verifying = false;
	let mut verified = false;
	let started = Instant::now();
	let mut connecting = started;
	let mut last_tick = started;
	while !stop.load(Ordering::Acquire) && !send.is_closed() {
		// Advance a fixed cadence instead of accumulating scheduler delay. A late tick
		// catches up one frame per dispatch; >=100 ms stalls discard queued samples.
		let timeout_ms = if verified && captures.is_empty() {
			100
		} else if verified {
			TICK.saturating_sub(last_tick.elapsed())
				.as_micros()
				.div_ceil(1000) as i32
		} else {
			10
		};
		match native.poll(timeout_ms)? {
			pulse::PA_CONTEXT_READY => {}
			pulse::PA_CONTEXT_FAILED | pulse::PA_CONTEXT_TERMINATED => return Err(UNAVAILABLE),
			_ if started.elapsed() > TIMEOUT => return Err(UNAVAILABLE),
			_ => continue,
		}
		native.subscribe()?;
		let revision = &native.events.revision;
		match native.events.subscribed.get() {
			Some(true) => {}
			Some(false) => return Err(UNAVAILABLE),
			None if started.elapsed() > TIMEOUT => return Err(UNAVAILABLE),
			None => continue,
		}
		let current_epoch = epoch.load(Ordering::Acquire);
		if !ready.load(Ordering::Acquire)
			|| current_epoch != active_epoch
			|| revision.get() != active_revision
		{
			captures.clear();
			expected.clear();
			verified = false;
			verifying = false;
			active_epoch = current_epoch;
			active_revision = revision.get();
		}
		// Coalesce events while one bounded request completes. Do not accumulate callbacks.
		if let Some(request) = listing.as_mut()
			&& let Some(inputs) = request.take()?
		{
			let fresh = request.revision == active_revision && ready.load(Ordering::Acquire);
			listing = None;
			if fresh {
				// Filter before both the verify comparison and the start loop, so an excluded
				// application stays out of the expected set too.
				let inputs: Vec<Input> = inputs
					.into_iter()
					.filter(|input| !excluded.contains(input))
					.collect();
				if verifying {
					// Re-check after native attachment: indices may have been removed/reused.
					verified = inputs == expected;
					verifying = false;
					if !verified {
						revision.set(revision.get().wrapping_add(1));
						continue;
					}
				} else {
					// One application refusing to be captured must not end the whole share;
					// drop just that application's audio and keep the rest.
					expected = Vec::with_capacity(inputs.len());
					for input in inputs {
						match Capture::start(&native, input.clone(), active_epoch) {
							Ok(capture) => {
								captures.push(capture);
								expected.push(input);
							}
							Err(_) => exclude(&mut excluded, input),
						}
					}
					connecting = Instant::now();
					verifying = true;
				}
			}
		}
		if !ready.load(Ordering::Acquire) {
			continue;
		}
		if listing.is_none() && !verified {
			if verifying {
				if let Some(capture) = captures.iter().find(|v| {
					matches!(
						v.state(),
						pulse::PA_STREAM_FAILED | pulse::PA_STREAM_TERMINATED
					)
				}) {
					exclude(&mut excluded, capture.input.clone());
					revision.set(revision.get().wrapping_add(1));
					continue;
				}
				if captures.iter().any(|v| v.state() != pulse::PA_STREAM_READY) {
					if connecting.elapsed() > TIMEOUT {
						return Err(UNAVAILABLE);
					}
					continue;
				}
			}
			listing = Some(Enumeration::start(&native, &own, active_revision)?);
		}
		if !verified || captures.is_empty() || last_tick.elapsed() < TICK {
			continue;
		}
		let stalled = last_tick.elapsed() >= Duration::from_millis(100);
		last_tick = if stalled {
			Instant::now()
		} else {
			last_tick + TICK
		};
		let mut mixed = [0.0; FRAME_SAMPLES];
		for capture in &mut captures {
			if capture.state() != pulse::PA_STREAM_READY {
				exclude(&mut excluded, capture.input.clone());
				revision.set(revision.get().wrapping_add(1));
				break;
			}
			capture.read()?;
			if stalled {
				capture.pending.samples.clear();
			}
			capture.pending.mix(&mut mixed, active_epoch);
		}
		if revision.get() == active_revision
			&& ready.load(Ordering::Acquire)
			&& epoch.load(Ordering::Acquire) == active_epoch
			&& !stop.load(Ordering::Acquire)
			&& let Ok(permit) = send.try_reserve()
		{
			permit.send(AudioChunk {
				samples: mixed.into_iter().map(|v| v.clamp(-1.0, 1.0)).collect(),
				epoch: active_epoch,
			});
		}
	}
	Ok(())
}

/// Drops one application's audio after its monitor failed, keeping the rest of the share.
/// The list is bounded, so a machine that keeps failing simply ends up sharing no audio.
fn exclude(excluded: &mut Vec<Input>, input: Input) {
	if excluded.len() < MAX_INPUTS && !excluded.contains(&input) {
		excluded.push(input);
	}
}

/// Device-free check shared with the Linux video debug example.
#[allow(dead_code)] // Called by the standalone offline example and the unit test.
pub(super) fn check_isolation() {
	let own = OwnApplication {
		pid: 42,
		binary: "serein".into(),
	};
	assert!(!own.allows(Some("42"), Some("renamed-client"), None, None));
	assert!(!own.allows(Some("999"), Some("serein"), None, None));
	assert!(!own.allows(Some("999"), Some("other"), Some("Serein call"), None));
	assert!(!own.allows(
		Some("999"),
		Some("other"),
		None,
		Some("cz.viceverse.serein")
	));
	assert!(!own.allows(None, Some("game"), None, None));
	assert!(!own.allows(Some("123"), None, None, None));
	assert!(own.allows(Some("123"), Some("game"), None, None));
	// Synthetic Pulse metadata only: allocating a proplist never opens a server/device.
	// SAFETY: pa_sink_input_info consists of C integers/enums/pointers for which zero is
	// valid; its proplist is owned here and freed after all borrowed callback accesses.
	unsafe {
		let props = pulse::pa_proplist_new();
		assert!(!props.is_null());
		assert_eq!(
			pulse::pa_proplist_sets(props, c"application.process.id".as_ptr(), c"123".as_ptr()),
			0
		);
		assert_eq!(
			pulse::pa_proplist_sets(
				props,
				c"application.process.binary".as_ptr(),
				c"game".as_ptr()
			),
			0
		);
		let mut info: pulse::pa_sink_input_info = std::mem::zeroed();
		info.index = 7;
		info.client = 4;
		info.sink = 2;
		info.proplist = props;
		let mut list = Listing {
			own: own.clone(),
			inputs: Vec::new(),
			monitors: Vec::new(),
			seen_inputs: 0,
			seen_sinks: 0,
			inputs_done: false,
			sinks_done: false,
			failed: false,
		};
		listed_input(null_mut(), &info, 0, (&mut list as *mut Listing).cast());
		assert_eq!(list.inputs.len(), 1);
		let mut sink: pulse::pa_sink_info = std::mem::zeroed();
		sink.index = 2;
		sink.monitor_source_name = c"game-output.monitor".as_ptr();
		listed_sink(null_mut(), &sink, 0, (&mut list as *mut Listing).cast());
		resolve_monitors(&mut list.inputs, &list.monitors).unwrap();
		assert_eq!(
			list.inputs[0].monitor.as_deref(),
			Some(c"game-output.monitor")
		);
		let selected = list.inputs[0].clone();
		// A new Serein playback stream reusing a former game index is never admitted.
		list.inputs.clear();
		pulse::pa_proplist_sets(
			props,
			c"application.process.binary".as_ptr(),
			c"serein".as_ptr(),
		);
		listed_input(null_mut(), &info, 0, (&mut list as *mut Listing).cast());
		assert!(list.inputs.is_empty());
		pulse::pa_proplist_sets(
			props,
			c"application.process.binary".as_ptr(),
			c"game".as_ptr(),
		);
		info.client = 5;
		listed_input(null_mut(), &info, 0, (&mut list as *mut Listing).cast());
		assert_ne!(
			list.inputs[0], selected,
			"post-attachment identity must detect index reuse"
		);
		info.index = pulse::PA_INVALID_INDEX;
		assert!(
			input(&info, &own).is_none(),
			"invalid monitor index would reset isolation"
		);
		info.index = 7;
		info.sink = pulse::PA_INVALID_INDEX;
		assert!(input(&info, &own).is_none());
		info.sink = 2;
		for _ in 0..MAX_INPUTS {
			listed_input(null_mut(), &info, 0, (&mut list as *mut Listing).cast());
		}
		assert!(list.failed);
		assert_eq!(list.inputs.len(), MAX_INPUTS);
		let mut missing = vec![Input {
			index: 9,
			client: 1,
			sink: 99,
			pid: 123,
			binary: "game".into(),
			serial: None,
			monitor: None,
		}];
		assert!(resolve_monitors(&mut missing, &list.monitors).is_err());
		pulse::pa_proplist_free(props);
	}
	let mut first = Pending::new(0);
	let mut second = Pending::new(0);
	let bytes = [0.25f32.to_le_bytes(), 0.5f32.to_le_bytes()]
		.concat()
		.repeat(FRAME_SAMPLES / 2);
	first.push(&bytes).unwrap();
	second.push(&bytes).unwrap();
	let mut mixed = [0.0; FRAME_SAMPLES];
	first.mix(&mut mixed, 0);
	second.mix(&mut mixed, 0);
	assert_eq!(&mixed[..4], &[0.5, 1.0, 0.5, 1.0]);
	for _ in 0..20 {
		first.push(&bytes).unwrap();
	}
	assert_eq!(first.samples.len(), MAX_AUDIO_SAMPLES);
	let mut after_rekey = [0.0; FRAME_SAMPLES];
	first.mix(&mut after_rekey, 1);
	assert!(first.samples.is_empty());
	assert!(after_rekey.iter().all(|v| *v == 0.0));
	assert!(first.hole(MAX_AUDIO_SAMPLES * 4 + 8).is_err());
	assert!(first.push(&[0; 7]).is_err());
	first.samples.clear();
	first
		.push(&[f32::NAN.to_le_bytes(), 4.0f32.to_le_bytes()].concat())
		.unwrap();
	assert_eq!(
		first.samples.iter().copied().collect::<Vec<_>>(),
		[0.0, 1.0]
	);
}

#[cfg(test)]
mod tests {
	#[test]
	fn application_isolation_and_bounded_stereo_mix() {
		super::check_isolation();
	}
}
