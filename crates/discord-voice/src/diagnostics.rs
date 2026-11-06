// Opt-in fixed-size aggregates. No strings or media enter the reporter queue.
use std::{
	io::Write,
	sync::{
		OnceLock,
		atomic::{AtomicU64, Ordering},
		mpsc,
	},
	time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Scope {
	Audio,
	Transport,
	StreamSend,
	StreamReceive,
	#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
	ScreenAudio,
	/// Linux screen capture: `receive` counts pictures taken from the pipeline, `encode`
	/// times the software encoder, `drops` counts pictures left in the pipeline while the
	/// transport was behind, and `stalls` counts passes where the pipeline had none.
	#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
	ScreenVideo,
}

#[derive(Clone, Copy)]
pub(crate) enum Stage {
	EchoRender,
	EchoCapture,
	Noise,
	Encode,
	Mix,
	Receive,
	VideoSend,
	VideoReceive,
	#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
	CaptureRead,
	#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
	CaptureQueue,
	#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
	CaptureRestart,
}

/// Per-cause remote video counters. Every silent drop in the receive path has a slot, so a
/// frozen viewer can be diagnosed from one report line without any media leaving the process.
#[derive(Clone, Copy)]
pub(crate) enum Video {
	/// Video RTP packets (payload 101) accepted by the transport cipher.
	Packets,
	/// Retransmission packets (payload 102) received before SSRC/sequence restoration.
	Rtx,
	/// Packets that failed the transport AEAD.
	OpenFailed,
	/// Video packets received while the DAVE session was not ready.
	NotReady,
	/// Packets on an SSRC no sender announced.
	UnknownSsrc,
	/// Pictures discarded by the depacketizer: sequence gaps or a missing marker.
	Incomplete,
	/// Intact encrypted access units.
	Complete,
	/// Access units that failed DAVE decryption.
	DecryptFailed,
	/// Predictions rejected while waiting for a keyframe.
	Gated,
	/// Frames dropped because the decoder queue was full.
	QueueFull,
	/// Keyframes handed to the decoder.
	Keyframes,
	/// Keyframes without inline SPS and PPS; a rebuilt decoder cannot use them.
	KeyframesWithoutParams,
	/// Picture Loss Indications sent.
	PliSent,
	/// Ticks spent waiting for at least one sender's keyframe.
	AwaitingTicks,
	/// Decoder failures reported by the decoder thread.
	DecoderErrors,
	/// Decoded pictures delivered to the sink.
	Pictures,
	/// Longest gap in milliseconds between delivered pictures (maximum, not a sum).
	PictureGapMs,
	/// Ticks where video stalled: sources announced but no recent picture.
	StallTicks,
	/// Longest wait in the decoder queue, in milliseconds.
	DecodeQueueMs,
	/// Queued frames discarded after exceeding the latency budget.
	StaleFrames,
	/// Compressed video bytes in complete, decrypted pictures, before decoder admission.
	EncodedBytes,
	/// Authenticated reception reports sent by this watched-stream transport.
	ReceiverReportsSent,
	/// Generic NACK datagrams sent to request missing video packets.
	NacksSent,
	/// Observed sequence holes filled before their repair deadline.
	RepairedPackets,
	/// Watched pictures enqueued after their RTP-clock playback deadline.
	PlayoutLate,
	/// Playback clock anchors cleared after discontinuity/security/pressure.
	PlayoutResets,
	/// Pictures discarded from the bounded watched-playback queue.
	PlayoutDrops,
	/// Maximum watched-playback queue depth during this report.
	PlayoutDepth,
	/// Authenticated empty video RTP payloads handled as padding/keepalives.
	PaddingPackets,
	/// Accepted watched-audio sender clock reports.
	AvAudioReports,
	/// Accepted watched-video sender clock reports.
	AvVideoReports,
	/// Video deadlines related to an audio sender clock.
	AvVideoSynced,
	/// Maximum pre-correction video scheduling offset, not physical output latency.
	AvOffsetMs,
	/// Maximum device/bridge delay estimate, separately from sender-clock correction.
	AvDeviceOffsetMs,
	PlayoutRewinds,
	PlayoutSourceChanges,
	PlayoutExpired,
	PlayoutPressure,
	PlayoutAhead,
	AvTimelineRebases,
	/// Decrypted access units by negotiated codec, not decoder output counts.
	H264Frames,
	H265Frames,
	H264Packets,
	H265Packets,
	PayloadRejected,
	CodecSwitches,
	SessionH264,
	SessionH265,
	/// Sequence holes abandoned after the adaptive repair deadline.
	RepairExpired,
	/// Known expired holes filled later; each hole is counted at most once.
	RepairLate,
	/// Maximum first-NACK-to-hole-fill interval; not a proven network RTT.
	RepairResponseMs,
	/// Maximum current per-source repair allowance, in milliseconds.
	RepairBudgetMs,
	Av1Frames,
	Av1Packets,
	SessionAv1,
	RtpBytes,
	ProbePackets,
	ProbeBytes,
	TwccReceived,
	TwccSent,
	TwccReported,
	TwccMissing,
	TwccDiscarded,
}
const VIDEO_SLOTS: usize = Video::TwccDiscarded as usize + 1;

/// Opt-in only; media callbacks do no additional atomic work when diagnostics are off.
pub(crate) fn enabled() -> bool {
	static ENABLED: OnceLock<bool> = OnceLock::new();
	*ENABLED.get_or_init(|| std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|v| v == "1"))
}

/// Dimensions of delivered pictures only. The packed key sorts by pixel count and retains
/// both dimensions; live pictures have already passed the shared UHD bound.
#[derive(Clone, Copy, Default)]
struct VideoSizes {
	last: u64,
	min: u64,
	max: u64,
	changes: u64,
}

impl VideoSizes {
	fn dimensions(key: u64) -> (u64, u64) {
		((key >> 12) & 4095, key & 4095)
	}
	fn merge(&mut self, next: Self) {
		if next.last != 0 {
			self.last = next.last;
		}
		if next.min != 0 {
			self.min = if self.min == 0 {
				next.min
			} else {
				self.min.min(next.min)
			};
		}
		self.max = self.max.max(next.max);
		self.changes = self.changes.saturating_add(next.changes);
	}
}

/// Fixed-size decoder-callback counters, drained by its transport without a blocking lock.
pub(crate) struct VideoSizeCounters {
	enabled: bool,
	last: AtomicU64,
	min: AtomicU64,
	max: AtomicU64,
	changes: AtomicU64,
}

impl Default for VideoSizeCounters {
	fn default() -> Self {
		Self {
			enabled: enabled(),
			last: AtomicU64::new(0),
			min: AtomicU64::new(0),
			max: AtomicU64::new(0),
			changes: AtomicU64::new(0),
		}
	}
}

impl VideoSizeCounters {
	pub fn record(&self, width: u32, height: u32) {
		if !self.enabled {
			return;
		}
		debug_assert!(width > 0 && height > 0 && width <= 3840 && height <= 3840);
		let key = ((u64::from(width) * u64::from(height)) << 24)
			| (u64::from(width) << 12)
			| u64::from(height);
		let previous = self.last.swap(key, Ordering::Relaxed);
		if previous != 0 && previous != key {
			self.changes.fetch_add(1, Ordering::Relaxed);
		}
		let _ = self
			.min
			.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |min| {
				Some(if min == 0 { key } else { min.min(key) })
			});
		self.max.fetch_max(key, Ordering::Relaxed);
	}

	fn take(&self) -> VideoSizes {
		VideoSizes {
			last: self.last.load(Ordering::Relaxed),
			min: self.min.swap(0, Ordering::Relaxed),
			max: self.max.swap(0, Ordering::Relaxed),
			changes: self.changes.swap(0, Ordering::Relaxed),
		}
	}
}

/// Voice signaling messages, so a handshake that never completes names its own missing step.
/// Opcodes only; no signaling contents are recorded.
#[derive(Clone, Copy)]
pub(crate) enum Signal {
	/// Text events received.
	Text,
	/// Binary DAVE events received.
	Binary,
	/// Text opcodes with no dedicated slot.
	Other,
	/// Ready (2): our SSRCs and the UDP endpoint.
	Ready,
	/// Session description (4): the transport key.
	Session,
	/// Clients connect (11): the DAVE participant list.
	Clients,
	/// Video (12): a sender announced its SSRCs.
	Sender,
	/// Prepare transition (21).
	PrepareTransition,
	/// Execute transition (22): the only message that makes DAVE ready.
	ExecuteTransition,
	/// Prepare epoch (24).
	PrepareEpoch,
	/// External sender (binary 25).
	ExternalSender,
	/// Proposals (binary 27).
	Proposals,
	/// Commit or welcome (binary 29 and 30).
	Commit,
	/// MLS key packages sent.
	KeyPackageSent,
	/// Transition ready (23) sent.
	TransitionReadySent,
	/// Our SSRC announcement (12) sent.
	SubscribeSent,
	/// Video sink wants (15) sent.
	SinkWantsSent,
}
const SIGNAL_SLOTS: usize = 17;
static SCREEN_ENCODERS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static CAMERA_ENCODERS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// Counts only currently live encoders, separately for screen sharing and camera video.
pub(crate) struct EncoderRegistration {
	counts: &'static [AtomicU64; 2],
	hardware: Option<bool>,
}

impl EncoderRegistration {
	pub(crate) fn new(screen: bool, hardware: bool) -> Self {
		let mut registration = Self {
			counts: if screen {
				&SCREEN_ENCODERS
			} else {
				&CAMERA_ENCODERS
			},
			hardware: None,
		};
		registration.set(Some(hardware));
		registration
	}

	pub(crate) fn set(&mut self, hardware: Option<bool>) {
		if let Some(previous) = self.hardware {
			self.counts[usize::from(!previous)].fetch_sub(1, Ordering::Relaxed);
		}
		if let Some(current) = hardware {
			self.counts[usize::from(!current)].fetch_add(1, Ordering::Relaxed);
		}
		self.hardware = hardware;
	}
}

impl Drop for EncoderRegistration {
	fn drop(&mut self) {
		self.set(None);
	}
}

fn codec_label([hardware, software]: [u64; 2]) -> &'static str {
	match (hardware > 0, software > 0) {
		(true, false) => "hardware",
		(false, true) => "software",
		(true, true) => "mixed",
		(false, false) => "unknown",
	}
}

/// Linux application-audio capture counters, reported under `Scope::ScreenAudio`.
#[derive(Clone, Copy)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum Capture {
	/// Applications the enumeration allowed (after excluding this client).
	Inputs,
	/// Monitor captures started.
	Started,
	/// Applications excluded after their capture failed.
	Excluded,
	/// Mixed 20 ms chunks handed to the stream.
	Chunks,
	/// Attachments confirmed by a later listing and mixed from.
	Ready,
	/// Attachments dropped because they failed or never connected.
	Dropped,
}

#[derive(Clone, Copy)]
struct Report {
	scope: Scope,
	encoder_counts: [u64; 2],
	decoder_counts: [u64; 2],
	at_ms: u64,
	window_ms: u64,
	video: [u64; VIDEO_SLOTS],
	video_sizes: VideoSizes,
	stream_audio: [u64; 10],
	device_playback: [u64; 2],
	signal: [u64; SIGNAL_SLOTS],
	/// Application-audio capture (Linux): inputs allowed, captures started, applications
	/// excluded after a failure, and mixed 20 ms chunks handed to the stream.
	capture: [u64; 6],
	// Each stage: calls, total elapsed microseconds, maximum elapsed microseconds.
	stages: [[u64; 3]; 11],
	wakes: u64,
	resets: u64,
	drops: u64,
	stalls: u64,
	noise_frames: u64,
	stream_ticks: [u64; 8],
	queued_audio: u64,
}

pub(crate) struct Metrics {
	send: Option<&'static mpsc::SyncSender<Report>>,
	since: Instant,
	scope: Scope,
	recording_session: u64,
	report: Option<Box<Report>>,
}

/// Reports and bytes the reporter thread accepts before going quiet. Debug-only opt-in, but
/// still bounded so a forgotten environment variable cannot fill a disk.
const MAX_REPORTS: usize = 8192;
const MAX_REPORT_BYTES: usize = 8 * 1024 * 1024;

fn started() -> Instant {
	static START: OnceLock<Instant> = OnceLock::new();
	*START.get_or_init(Instant::now)
}

fn empty_report(scope: Scope) -> Report {
	Report {
		scope,
		encoder_counts: [0; 2],
		decoder_counts: [0; 2],
		at_ms: 0,
		window_ms: 0,
		video: [0; VIDEO_SLOTS],
		video_sizes: VideoSizes::default(),
		stream_audio: [0; 10],
		device_playback: [0; 2],
		signal: [0; SIGNAL_SLOTS],
		capture: [0; 6],
		stages: [[0; 3]; 11],
		wakes: 0,
		resets: 0,
		drops: 0,
		stalls: 0,
		noise_frames: 0,
		stream_ticks: [0; 8],
		queued_audio: 0,
	}
}

impl Metrics {
	fn enabled(&self) -> bool {
		self.send.is_some() || diagnostics::is_enabled(diagnostics::Level::Debug)
	}

	fn active(&mut self) -> bool {
		self.active_when(self.enabled(), diagnostics::recording_session())
	}

	fn active_when(&mut self, enabled: bool, session: u64) -> bool {
		if self.recording_session != session {
			self.recording_session = session;
			self.report = None;
		}
		if !enabled {
			self.report = None;
			return false;
		}
		if self.report.is_none() {
			self.since = Instant::now();
			self.report = Some(Box::new(empty_report(self.scope)));
		}
		true
	}

	pub fn new(scope: Scope) -> Self {
		static REPORTER: OnceLock<Option<mpsc::SyncSender<Report>>> = OnceLock::new();
		let send = REPORTER.get_or_init(|| {
			if !enabled() {
				return None;
			}
			let (send, receive) = mpsc::sync_channel::<Report>(8);
			std::thread::Builder::new()
				.name("voice-diagnostics".into())
				.spawn(move || {
					let mut bytes = MAX_REPORT_BYTES;
					for report in receive.iter().take(MAX_REPORTS) {
						if !write_report(report, &mut bytes, &mut std::io::stderr()) {
							break;
						}
					}
				})
				.ok()?;
			Some(send)
		});
		Self {
			send: send.as_ref(),
			since: Instant::now(),
			scope,
			recording_session: diagnostics::recording_session(),
			report: None,
		}
	}

	pub fn device_playback(&mut self, values: [u64; 2]) {
		if !self.active() {
			return;
		}
		for (slot, value) in self
			.report
			.as_mut()
			.expect("active metrics")
			.device_playback
			.iter_mut()
			.zip(values)
		{
			*slot = slot.saturating_add(value);
		}
	}

	/// Fixed aggregates only: no PCM, source identities or raw RTP timestamps.
	pub fn stream_audio(&mut self, values: [u64; 10]) {
		if !self.active() {
			return;
		}
		for (index, value) in values.into_iter().enumerate() {
			let slot = &mut self.report.as_mut().expect("active metrics").stream_audio[index];
			*slot = if (6..8).contains(&index) {
				(*slot).max(value)
			} else {
				slot.saturating_add(value)
			};
		}
	}

	pub fn start(&self) -> Option<Instant> {
		self.enabled().then(Instant::now)
	}

	/// Snapshot live decoder counts owned by this transport, never process-wide history.
	pub fn decoder_counts(&mut self, hardware: u64, software: u64) {
		if !self.active() {
			return;
		}
		self.report.as_mut().expect("active metrics").decoder_counts = [hardware, software];
	}

	/// Aggregate dimensions independently of the decoder's short polling interval.
	pub fn video_sizes(&mut self, counters: &VideoSizeCounters) {
		if self.active() {
			self.report
				.as_mut()
				.expect("active metrics")
				.video_sizes
				.merge(counters.take());
		}
	}

	pub fn finish(&mut self, stage: Stage, start: Option<Instant>) {
		if let Some(start) = start {
			self.add(stage, start.elapsed());
		}
	}

	/// Records one call measured elsewhere; ignored while diagnostics are off.
	pub fn add(&mut self, stage: Stage, elapsed: Duration) {
		if !self.active() {
			return;
		}
		let micros = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
		let [calls, total, max] =
			&mut self.report.as_mut().expect("active metrics").stages[stage as usize];
		*calls = calls.saturating_add(1);
		*total = total.saturating_add(micros);
		*max = (*max).max(micros);
	}

	/// Adds to one remote video counter; ignored while diagnostics are off.
	pub fn video(&mut self, event: Video, count: u64) {
		if !self.active() {
			return;
		}
		let slot = &mut self.report.as_mut().expect("active metrics").video[event as usize];
		*slot = slot.saturating_add(count);
	}

	/// Keeps the largest observed value for a maximum-style video counter.
	pub fn video_max(&mut self, event: Video, value: u64) {
		if !self.active() {
			return;
		}
		let slot = &mut self.report.as_mut().expect("active metrics").video[event as usize];
		*slot = (*slot).max(value);
	}

	/// Adds to one application-audio capture counter; ignored while diagnostics are off.
	#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
	pub fn capture(&mut self, event: Capture, count: u64) {
		if !self.active() {
			return;
		}
		let slot = &mut self.report.as_mut().expect("active metrics").capture[event as usize];
		*slot = slot.saturating_add(count);
	}

	/// Adds to one signaling counter; ignored while diagnostics are off.
	pub fn signal(&mut self, event: Signal, count: u64) {
		if !self.active() {
			return;
		}
		let slot = &mut self.report.as_mut().expect("active metrics").signal[event as usize];
		*slot = slot.saturating_add(count);
	}

	pub fn poll(&mut self, reset: bool, drops: u64, stalled: bool, noise_frames: u64) {
		if !self.active() {
			return;
		}
		self.report.as_mut().expect("active metrics").wakes = self
			.report
			.as_mut()
			.expect("active metrics")
			.wakes
			.saturating_add(1);
		self.report.as_mut().expect("active metrics").resets = self
			.report
			.as_mut()
			.expect("active metrics")
			.resets
			.saturating_add(u64::from(reset));
		self.report.as_mut().expect("active metrics").drops = self
			.report
			.as_mut()
			.expect("active metrics")
			.drops
			.saturating_add(drops);
		self.report.as_mut().expect("active metrics").stalls = self
			.report
			.as_mut()
			.expect("active metrics")
			.stalls
			.saturating_add(u64::from(stalled));
		self.report.as_mut().expect("active metrics").noise_frames = self
			.report
			.as_mut()
			.expect("active metrics")
			.noise_frames
			.saturating_add(noise_frames);
		if self.since.elapsed() >= Duration::from_secs(5) {
			self.flush();
		}
	}

	/// Counts true state flags per stream tick and observed queued audio chunks.
	/// Flags: transport key, DAVE ready, group ready, pending, waiting, announced,
	/// capture ready, audio enabled.
	pub fn stream_state(&mut self, flags: [bool; 8], queued_audio: usize) {
		if !self.active() {
			return;
		}
		for (ticks, flag) in self
			.report
			.as_mut()
			.expect("active metrics")
			.stream_ticks
			.iter_mut()
			.zip(flags)
		{
			*ticks = ticks.saturating_add(u64::from(flag));
		}
		self.report.as_mut().expect("active metrics").queued_audio = self
			.report
			.as_ref()
			.expect("active metrics")
			.queued_audio
			.saturating_add(u64::try_from(queued_audio).unwrap_or(u64::MAX));
	}

	/// Queue the current aggregates before a potentially blocking native operation.
	#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
	pub fn checkpoint(&mut self) {
		self.flush();
	}

	fn flush(&mut self) {
		if !self.active() {
			return;
		}
		self.report.as_mut().expect("active metrics").encoder_counts =
			match self.report.as_mut().expect("active metrics").scope {
				Scope::StreamSend | Scope::ScreenVideo => Some(&SCREEN_ENCODERS),
				Scope::Transport => Some(&CAMERA_ENCODERS),
				_ => None,
			}
			.map_or([0; 2], |counts| {
				counts.each_ref().map(|count| count.load(Ordering::Relaxed))
			});
		self.report.as_mut().expect("active metrics").at_ms =
			started().elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
		self.report.as_mut().expect("active metrics").window_ms =
			self.since.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
		if diagnostics::is_enabled(diagnostics::Level::Debug) {
			shared_report(self.report.as_ref().expect("active metrics"));
		}
		if let Some(send) = self.send
			&& let Err(mpsc::TrySendError::Disconnected(_)) =
				send.try_send(**self.report.as_ref().expect("active metrics"))
		{
			self.send = None;
		}
		if !self.enabled() {
			self.report = None;
			return;
		}
		self.reset();
	}

	fn reset(&mut self) {
		let Some(report) = self.report.as_mut() else {
			return;
		};
		self.since = Instant::now();
		report.stages = [[0; 3]; 11];
		report.wakes = 0;
		report.resets = 0;
		report.drops = 0;
		report.stalls = 0;
		report.noise_frames = 0;
		report.stream_ticks = [0; 8];
		report.queued_audio = 0;
		report.video = [0; VIDEO_SLOTS];
		report.video_sizes = VideoSizes::default();
		report.stream_audio = [0; 10];
		report.device_playback = [0; 2];
		report.signal = [0; SIGNAL_SLOTS];
		report.capture = [0; 6];
	}
}

impl Drop for Metrics {
	fn drop(&mut self) {
		self.flush();
	}
}

// Called only by the media processing worker, never by a native audio callback. No received
// values, identities, device labels or media bytes enter the shared report.
fn shared_report(report: &Report) {
	use diagnostics::{Category, Value};
	let scope = match report.scope {
		Scope::Audio => "audio",
		Scope::Transport => "transport",
		Scope::StreamSend => "stream_send",
		Scope::StreamReceive => "stream_receive",
		Scope::ScreenAudio => "screen_audio",
		Scope::ScreenVideo => "screen_video",
	};
	let record = |label, values: &[(&'static str, Value)]| {
		diagnostics::summary(Category::Voice, label, values);
	};
	record(
		"media summary",
		&[
			("scope", scope.into()),
			("window_ms", report.window_ms.into()),
			("wakes", report.wakes.into()),
			("drops", report.drops.into()),
		],
	);
	record(
		"media recovery",
		&[
			("scope", scope.into()),
			("resets", report.resets.into()),
			("stalls", report.stalls.into()),
			("noise_frames", report.noise_frames.into()),
		],
	);
	let stages = [
		"echo_render",
		"echo_capture",
		"noise",
		"encode",
		"mix",
		"receive",
		"video_send",
		"video_receive",
		"capture_read",
		"capture_queue",
		"capture_restart",
	];
	for (stage, [calls, total, max]) in stages.into_iter().zip(report.stages) {
		if calls > 0 {
			record(
				"media stage",
				&[
					("scope", scope.into()),
					("stage", stage.into()),
					("calls", calls.into()),
					("total_us", total.into()),
					("max_us", max.into()),
				],
			);
		}
	}
	let video = [
		(Video::Packets, "packets"),
		(Video::Rtx, "rtx"),
		(Video::OpenFailed, "open_failed"),
		(Video::NotReady, "not_ready"),
		(Video::UnknownSsrc, "unknown_sender"),
		(Video::Incomplete, "incomplete"),
		(Video::Complete, "complete"),
		(Video::DecryptFailed, "decrypt_failed"),
		(Video::Gated, "gated"),
		(Video::QueueFull, "queue_full"),
		(Video::Keyframes, "keyframes"),
		(Video::KeyframesWithoutParams, "keyframes_without_params"),
		(Video::PliSent, "pli_sent"),
		(Video::AwaitingTicks, "awaiting_ticks"),
		(Video::DecoderErrors, "decoder_errors"),
		(Video::Pictures, "pictures"),
		(Video::PictureGapMs, "picture_gap_ms"),
		(Video::StallTicks, "stall_ticks"),
		(Video::DecodeQueueMs, "decode_queue_ms"),
		(Video::StaleFrames, "stale_frames"),
	];
	for (variant, counter) in video {
		let value = report.video[variant as usize];
		if value > 0 {
			record(
				"video counter",
				&[
					("scope", scope.into()),
					("counter", counter.into()),
					("value", value.into()),
				],
			);
		}
	}
	let signal = [
		(Signal::Text, "text"),
		(Signal::Binary, "binary"),
		(Signal::Other, "other"),
		(Signal::Ready, "ready"),
		(Signal::Session, "session"),
		(Signal::Clients, "clients"),
		(Signal::Sender, "sender"),
		(Signal::PrepareTransition, "prepare_transition"),
		(Signal::ExecuteTransition, "execute_transition"),
		(Signal::PrepareEpoch, "prepare_epoch"),
		(Signal::ExternalSender, "external_sender"),
		(Signal::Proposals, "proposals"),
		(Signal::Commit, "commit"),
		(Signal::KeyPackageSent, "key_package_sent"),
		(Signal::TransitionReadySent, "transition_ready_sent"),
		(Signal::SubscribeSent, "subscribe_sent"),
		(Signal::SinkWantsSent, "sink_wants_sent"),
	];
	for (variant, counter) in signal {
		let value = report.signal[variant as usize];
		if value > 0 {
			record(
				"voice signaling counter",
				&[
					("scope", scope.into()),
					("counter", counter.into()),
					("value", value.into()),
				],
			);
		}
	}
	let capture = [
		"inputs", "started", "excluded", "chunks", "ready", "dropped",
	];
	for (counter, value) in capture.into_iter().zip(report.capture) {
		if value > 0 {
			record(
				"capture counter",
				&[
					("scope", scope.into()),
					("counter", counter.into()),
					("value", value.into()),
				],
			);
		}
	}
	record(
		"video codecs",
		&[
			("scope", scope.into()),
			("encode_hardware", report.encoder_counts[0].into()),
			("encode_software", report.encoder_counts[1].into()),
			("decode_software", report.decoder_counts[1].into()),
		],
	);
	let states = [
		"transport_key",
		"dave_ready",
		"group_ready",
		"pending",
		"waiting",
		"announced",
		"capture_ready",
		"audio_enabled",
	];
	for (state, ticks) in states.into_iter().zip(report.stream_ticks) {
		if ticks > 0 {
			record(
				"stream state",
				&[
					("scope", scope.into()),
					("state", state.into()),
					("ticks", ticks.into()),
				],
			);
		}
	}
}

fn write_report(report: Report, bytes: &mut usize, writer: &mut impl Write) -> bool {
	let mut line = format!(
		"[Serein voice {:?}] debug={} at_ms={} window_ms={} wakes={} resets={} drops={} stalls={} noise_frames={} stages(calls,total_us,max_us): echo_render={:?} echo_capture={:?} noise={:?} encode={:?} mix={:?} receive={:?}",
		report.scope,
		cfg!(debug_assertions),
		report.at_ms,
		report.window_ms,
		report.wakes,
		report.resets,
		report.drops,
		report.stalls,
		report.noise_frames,
		report.stages[0],
		report.stages[1],
		report.stages[2],
		report.stages[3],
		report.stages[4],
		report.stages[5],
	);
	if matches!(
		report.scope,
		Scope::Transport | Scope::StreamSend | Scope::ScreenVideo
	) {
		line.push_str(&format!(" encoder={}", codec_label(report.encoder_counts)));
	}
	if matches!(report.scope, Scope::Transport | Scope::StreamReceive) {
		line.push_str(&format!(" decoder={}", codec_label(report.decoder_counts)));
	}
	if matches!(report.scope, Scope::StreamSend | Scope::StreamReceive) {
		let [
			transport_key,
			dave_ready,
			group_ready,
			pending,
			waiting,
			announced,
			capture_ready,
			audio_enabled,
		] = report.stream_ticks;
		line.push_str(&format!(
			" video_send={:?} video_receive={:?} stream_ticks: transport_key={transport_key} dave_ready={dave_ready} group_ready={group_ready} pending={pending} waiting={waiting} announced={announced} capture_ready={capture_ready} audio_enabled={audio_enabled} queued_audio={}",
			report.stages[6], report.stages[7], report.queued_audio,
		));
	}
	if report.video.iter().any(|count| *count != 0) {
		let [
			packets,
			rtx,
			open_failed,
			not_ready,
			unknown_ssrc,
			incomplete,
			complete,
			decrypt_failed,
			gated,
			queue_full,
			keyframes,
			keyframes_without_params,
			pli_sent,
			awaiting_ticks,
			decoder_errors,
			pictures,
			picture_gap_ms,
			stall_ticks,
			decode_queue_ms,
			stale_frames,
			encoded_bytes,
			receiver_reports_sent,
			nacks_sent,
			repaired_packets,
			playout_late,
			playout_resets,
			playout_drops,
			playout_depth,
			padding_packets,
			av_audio_reports,
			av_video_reports,
			av_video_synced,
			av_offset_ms,
			av_device_offset_ms,
			playout_rewinds,
			playout_source_changes,
			playout_expired,
			playout_pressure,
			playout_ahead,
			av_timeline_rebases,
			h264_frames,
			h265_frames,
			h264_packets,
			h265_packets,
			payload_rejected,
			codec_switches,
			session_h264,
			session_h265,
			repair_expired,
			repair_late,
			repair_response_ms,
			repair_budget_ms,
			av1_frames,
			av1_packets,
			session_av1,
			video_rtp_bytes,
			probe_packets,
			probe_bytes,
			twcc_received,
			twcc_sent,
			twcc_reported,
			twcc_missing,
			twcc_discarded,
		] = report.video;
		let encoded_bps = encoded_bytes
			.saturating_mul(8000)
			.checked_div(report.window_ms)
			.unwrap_or(0);
		let video_rtp_bps = video_rtp_bytes
			.saturating_mul(8000)
			.checked_div(report.window_ms)
			.unwrap_or(0);
		line.push_str(&format!(" receive_transport: video_rtp_bytes={video_rtp_bytes} video_rtp_bps={video_rtp_bps} probe_packets={probe_packets} probe_bytes={probe_bytes} twcc_received={twcc_received} twcc_sent={twcc_sent} twcc_reported={twcc_reported} twcc_missing={twcc_missing} twcc_discarded={twcc_discarded}"));
		line.push_str(&format!(
			" av1_frames={av1_frames} av1_packets={av1_packets} session_av1={session_av1} repair: repair_expired={repair_expired} repair_late={repair_late} repair_response_ms={repair_response_ms} repair_budget_ms={repair_budget_ms}"
		));
		line.push_str(&format!(
			" video: packets={packets} rtx={rtx} open_failed={open_failed} not_ready={not_ready} unknown_ssrc={unknown_ssrc} incomplete={incomplete} complete={complete} decrypt_failed={decrypt_failed} gated={gated} queue_full={queue_full} keyframes={keyframes} keyframes_without_params={keyframes_without_params} pli_sent={pli_sent} awaiting_ticks={awaiting_ticks} decoder_errors={decoder_errors} pictures={pictures} picture_gap_ms={picture_gap_ms} stall_ticks={stall_ticks} decode_queue_ms={decode_queue_ms} stale_frames={stale_frames} encoded_bytes={encoded_bytes} encoded_bps={encoded_bps} receiver_reports_sent={receiver_reports_sent} nacks_sent={nacks_sent} repaired_packets={repaired_packets} playout_late={playout_late} playout_resets={playout_resets} playout_drops={playout_drops} playout_depth={playout_depth} padding_packets={padding_packets} av_audio_reports={av_audio_reports} av_video_reports={av_video_reports} av_video_synced={av_video_synced} av_offset_ms={av_offset_ms} av_device_offset_ms={av_device_offset_ms} playout_rewinds={playout_rewinds} playout_source_changes={playout_source_changes} playout_expired={playout_expired} playout_pressure={playout_pressure} playout_ahead={playout_ahead} av_timeline_rebases={av_timeline_rebases} h264_frames={h264_frames} h265_frames={h265_frames} h264_packets={h264_packets} h265_packets={h265_packets} payload_rejected={payload_rejected} codec_switches={codec_switches} session_h264={session_h264} session_h265={session_h265}"
		));
	}
	if report.device_playback.iter().any(|value| *value != 0) {
		let [empty_frames, stale_frames] = report.device_playback;
		line.push_str(&format!(
			" device_playback: device_empty_frames={empty_frames} device_stale_frames={stale_frames}"
		));
	}
	if report.stream_audio.iter().any(|value| *value != 0) {
		let [
			decoded_samples,
			concealed_samples,
			decode_errors,
			rebufferings,
			rate_up_samples,
			rate_down_samples,
			buffer_ms,
			target_ms,
			bridge_drops,
			bridge_underruns,
		] = report.stream_audio;
		line.push_str(&format!(" stream_audio: decoded_samples={decoded_samples} concealed_samples={concealed_samples} audio_decode_errors={decode_errors} audio_rebufferings={rebufferings} rate_up_samples={rate_up_samples} rate_down_samples={rate_down_samples} audio_buffer_ms={buffer_ms} audio_target_ms={target_ms} bridge_drops={bridge_drops} bridge_underruns={bridge_underruns}"));
	}
	if report.video_sizes.last != 0 {
		let (width, height) = VideoSizes::dimensions(report.video_sizes.last);
		let (min_width, min_height) = VideoSizes::dimensions(report.video_sizes.min);
		let (max_width, max_height) = VideoSizes::dimensions(report.video_sizes.max);
		line.push_str(&format!(
			" video_size: last={width}x{height} min={min_width}x{min_height} max={max_width}x{max_height} changes={}", report.video_sizes.changes
		));
	}
	if report.signal.iter().any(|count| *count != 0) {
		let [
			text,
			binary,
			other,
			ready,
			session,
			clients,
			sender,
			prepare_transition,
			execute_transition,
			prepare_epoch,
			external_sender,
			proposals,
			commit,
			key_package_sent,
			transition_ready_sent,
			subscribe_sent,
			sink_wants_sent,
		] = report.signal;
		line.push_str(&format!(
			" signal: text={text} binary={binary} other={other} ready={ready} session={session} clients={clients} sender={sender} prepare_transition={prepare_transition} execute_transition={execute_transition} prepare_epoch={prepare_epoch} external_sender={external_sender} proposals={proposals} commit={commit} key_package_sent={key_package_sent} transition_ready_sent={transition_ready_sent} subscribe_sent={subscribe_sent} sink_wants_sent={sink_wants_sent}"
		));
	}
	if matches!(report.scope, Scope::ScreenAudio) {
		let [inputs, started, excluded, chunks, ready, dropped] = report.capture;
		line.push_str(&format!(
			" capture_read={:?} capture_queue={:?} capture_restart={:?} app_inputs={inputs} app_captures={started} app_excluded={excluded} app_chunks={chunks} app_ready={ready} app_dropped={dropped}",
			report.stages[8], report.stages[9], report.stages[10],
		));
	}
	line.push('\n');
	if line.len() > *bytes {
		return false;
	}
	// Charge attempted bytes even on a partial write. Failure never affects the call.
	*bytes -= line.len();
	writer.write_all(line.as_bytes()).is_ok()
}

#[cfg(test)]
mod shared_tests {
	use super::*;

	#[test]
	fn shared_measurements_keep_both_worker_scopes() {
		diagnostics::install_mode(diagnostics::Level::Debug, std::path::PathBuf::new(), false);
		let mut report = empty_report(Scope::Audio);
		report.stages[Stage::Encode as usize] = [1, 10, 10];
		report.signal[Signal::Text as usize] = 1;
		report.stream_ticks[0] = 1;
		shared_report(&report);
		report.scope = Scope::Transport;
		shared_report(&report);
		let text = diagnostics::recent(100);
		let latest = text.split("Latest subsystem summaries:").nth(1).unwrap();
		for scope in ["audio", "transport"] {
			for event in ["media stage", "voice signaling counter", "stream state"] {
				assert!(
					latest.contains(&format!("{event} scope={scope} ")),
					"{event} lost {scope}"
				);
			}
		}
		diagnostics::install_mode(diagnostics::Level::Off, std::path::PathBuf::new(), false);
	}

	#[test]
	fn toggling_recording_allocates_lazily_and_discards_previous_counters() {
		let mut metrics = Metrics {
			send: None,
			since: Instant::now(),
			scope: Scope::Audio,
			recording_session: 0,
			report: None,
		};
		assert!(!metrics.active_when(false, 0));
		assert!(metrics.report.is_none());
		assert!(metrics.active_when(true, 0));
		metrics.report.as_mut().unwrap().drops = 7;
		assert!(!metrics.active_when(false, 0));
		assert!(metrics.report.is_none());
		assert!(metrics.active_when(true, 0));
		assert_eq!(metrics.report.as_ref().unwrap().drops, 0);
		metrics.report.as_mut().unwrap().drops = 9;
		metrics.since = Instant::now() - Duration::from_secs(10);
		let before = Instant::now();
		assert!(
			metrics.active_when(true, 1),
			"off/on between media ticks starts a new session"
		);
		assert_eq!(metrics.report.as_ref().unwrap().drops, 0);
		assert!(metrics.since >= before);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn uhd_dimensions_survive_packing_and_pixel_count_range_selection() {
		let sizes = VideoSizeCounters {
			enabled: true,
			..VideoSizeCounters::default()
		};
		for (width, height) in [(2560, 1440), (3840, 2160), (1920, 1080), (2160, 3840)] {
			sizes.record(width, height);
			assert_eq!(
				VideoSizes::dimensions(sizes.last.load(Ordering::Relaxed)),
				(u64::from(width), u64::from(height))
			);
		}
		let range = sizes.take();
		assert_eq!(VideoSizes::dimensions(range.min), (1920, 1080));
		assert_eq!(VideoSizes::dimensions(range.max), (3840, 2160));
		assert_eq!(VideoSizes::dimensions(range.last), (2160, 3840));
	}
}
