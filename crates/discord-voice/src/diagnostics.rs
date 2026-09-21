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
	/// Retransmission packets (payload 102); currently ignored, so a high count means loss.
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
}
const VIDEO_SLOTS: usize = 20;

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
	report: Report,
}

/// Reports and bytes the reporter thread accepts before going quiet. Debug-only opt-in, but
/// still bounded so a forgotten environment variable cannot fill a disk.
const MAX_REPORTS: usize = 8192;
const MAX_REPORT_BYTES: usize = 8 * 1024 * 1024;

fn started() -> Instant {
	static START: OnceLock<Instant> = OnceLock::new();
	*START.get_or_init(Instant::now)
}

impl Metrics {
	pub fn new(scope: Scope) -> Self {
		static REPORTER: OnceLock<Option<mpsc::SyncSender<Report>>> = OnceLock::new();
		let send = REPORTER.get_or_init(|| {
			if std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_none_or(|v| v != "1") {
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
			report: Report {
				scope,
				encoder_counts: [0; 2],
				decoder_counts: [0; 2],
				at_ms: 0,
				window_ms: 0,
				video: [0; VIDEO_SLOTS],
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
			},
		}
	}

	pub fn start(&self) -> Option<Instant> {
		self.send.map(|_| Instant::now())
	}

	/// Snapshot live decoder counts owned by this transport, never process-wide history.
	pub fn decoder_counts(&mut self, hardware: u64, software: u64) {
		self.report.decoder_counts = [hardware, software];
	}

	pub fn finish(&mut self, stage: Stage, start: Option<Instant>) {
		if let Some(start) = start {
			self.add(stage, start.elapsed());
		}
	}

	/// Records one call measured elsewhere; ignored while diagnostics are off.
	pub fn add(&mut self, stage: Stage, elapsed: Duration) {
		if self.send.is_none() {
			return;
		}
		let micros = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
		let [calls, total, max] = &mut self.report.stages[stage as usize];
		*calls = calls.saturating_add(1);
		*total = total.saturating_add(micros);
		*max = (*max).max(micros);
	}

	/// Adds to one remote video counter; ignored while diagnostics are off.
	pub fn video(&mut self, event: Video, count: u64) {
		if self.send.is_none() {
			return;
		}
		let slot = &mut self.report.video[event as usize];
		*slot = slot.saturating_add(count);
	}

	/// Keeps the largest observed value for a maximum-style video counter.
	pub fn video_max(&mut self, event: Video, value: u64) {
		if self.send.is_none() {
			return;
		}
		let slot = &mut self.report.video[event as usize];
		*slot = (*slot).max(value);
	}

	/// Adds to one application-audio capture counter; ignored while diagnostics are off.
	#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
	pub fn capture(&mut self, event: Capture, count: u64) {
		if self.send.is_none() {
			return;
		}
		let slot = &mut self.report.capture[event as usize];
		*slot = slot.saturating_add(count);
	}

	/// Adds to one signaling counter; ignored while diagnostics are off.
	pub fn signal(&mut self, event: Signal, count: u64) {
		if self.send.is_none() {
			return;
		}
		let slot = &mut self.report.signal[event as usize];
		*slot = slot.saturating_add(count);
	}

	pub fn poll(&mut self, reset: bool, drops: u64, stalled: bool, noise_frames: u64) {
		if self.send.is_none() {
			return;
		}
		self.report.wakes = self.report.wakes.saturating_add(1);
		self.report.resets = self.report.resets.saturating_add(u64::from(reset));
		self.report.drops = self.report.drops.saturating_add(drops);
		self.report.stalls = self.report.stalls.saturating_add(u64::from(stalled));
		self.report.noise_frames = self.report.noise_frames.saturating_add(noise_frames);
		if self.since.elapsed() >= Duration::from_secs(5) {
			self.flush();
		}
	}

	/// Counts true state flags per stream tick and observed queued audio chunks.
	/// Flags: transport key, DAVE ready, group ready, pending, waiting, announced,
	/// capture ready, audio enabled.
	pub fn stream_state(&mut self, flags: [bool; 8], queued_audio: usize) {
		if self.send.is_none() {
			return;
		}
		for (ticks, flag) in self.report.stream_ticks.iter_mut().zip(flags) {
			*ticks = ticks.saturating_add(u64::from(flag));
		}
		self.report.queued_audio = self
			.report
			.queued_audio
			.saturating_add(u64::try_from(queued_audio).unwrap_or(u64::MAX));
	}

	/// Queue the current aggregates before a potentially blocking native operation.
	#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
	pub fn checkpoint(&mut self) {
		self.flush();
	}

	fn flush(&mut self) {
		let Some(send) = self.send else { return };
		self.report.encoder_counts = match self.report.scope {
			Scope::StreamSend | Scope::ScreenVideo => Some(&SCREEN_ENCODERS),
			Scope::Transport => Some(&CAMERA_ENCODERS),
			_ => None,
		}
		.map_or([0; 2], |counts| {
			counts.each_ref().map(|count| count.load(Ordering::Relaxed))
		});
		self.report.at_ms = started().elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
		self.report.window_ms = self.since.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
		if let Err(mpsc::TrySendError::Disconnected(_)) = send.try_send(self.report) {
			self.send = None;
		}
		self.since = Instant::now();
		self.report.stages = [[0; 3]; 11];
		self.report.wakes = 0;
		self.report.resets = 0;
		self.report.drops = 0;
		self.report.stalls = 0;
		self.report.noise_frames = 0;
		self.report.stream_ticks = [0; 8];
		self.report.queued_audio = 0;
		self.report.video = [0; VIDEO_SLOTS];
		self.report.signal = [0; SIGNAL_SLOTS];
		self.report.capture = [0; 6];
	}
}

impl Drop for Metrics {
	fn drop(&mut self) {
		self.flush();
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
		] = report.video;
		line.push_str(&format!(
			" video: packets={packets} rtx={rtx} open_failed={open_failed} not_ready={not_ready} unknown_ssrc={unknown_ssrc} incomplete={incomplete} complete={complete} decrypt_failed={decrypt_failed} gated={gated} queue_full={queue_full} keyframes={keyframes} keyframes_without_params={keyframes_without_params} pli_sent={pli_sent} awaiting_ticks={awaiting_ticks} decoder_errors={decoder_errors} pictures={pictures} picture_gap_ms={picture_gap_ms} stall_ticks={stall_ticks} decode_queue_ms={decode_queue_ms} stale_frames={stale_frames}"
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
mod tests {
	use super::*;

	#[test]
	fn video_counters_are_written_only_when_present() {
		let mut report = Report {
			scope: Scope::StreamReceive,
			encoder_counts: [0; 2],
			decoder_counts: [0; 2],
			at_ms: 1234,
			window_ms: 5000,
			video: [0; VIDEO_SLOTS],
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
		};
		let mut bytes = MAX_REPORT_BYTES;
		let mut out = Vec::new();
		assert!(write_report(report, &mut bytes, &mut out));
		let line = String::from_utf8(out).unwrap();
		assert!(line.contains("at_ms=1234"));
		assert!(!line.contains(" video:"));

		report.video[Video::Packets as usize] = 150;
		report.video[Video::Gated as usize] = 40;
		report.video[Video::PictureGapMs as usize] = 1900;
		let mut out = Vec::new();
		assert!(write_report(report, &mut bytes, &mut out));
		let line = String::from_utf8(out).unwrap();
		assert!(line.contains("video: packets=150"));
		assert!(line.contains("gated=40"));
		assert!(line.ends_with("stall_ticks=0 decode_queue_ms=0 stale_frames=0\n"));

		report.signal[Signal::ExecuteTransition as usize] = 2;
		let mut out = Vec::new();
		assert!(write_report(report, &mut bytes, &mut out));
		let line = String::from_utf8(out).unwrap();
		assert!(line.contains("signal: text=0"));
		assert!(line.contains("execute_transition=2"));
	}
}
