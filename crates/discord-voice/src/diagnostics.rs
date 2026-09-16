// Opt-in fixed-size aggregates. No strings or media enter the reporter queue.
use std::{
	io::Write,
	sync::{OnceLock, mpsc},
	time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Scope {
	Audio,
	Transport,
	StreamSend,
	StreamReceive,
	#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
	ScreenAudio,
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
	#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
	CaptureRead,
	#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
	CaptureQueue,
	#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
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
}
const VIDEO_SLOTS: usize = 17;

#[derive(Clone, Copy)]
struct Report {
	scope: Scope,
	at_ms: u64,
	window_ms: u64,
	video: [u64; VIDEO_SLOTS],
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
				at_ms: 0,
				window_ms: 0,
				video: [0; VIDEO_SLOTS],
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
	if matches!(report.scope, Scope::Transport | Scope::StreamReceive)
		&& report.video.iter().any(|count| *count != 0)
	{
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
		] = report.video;
		line.push_str(&format!(
			" video: packets={packets} rtx={rtx} open_failed={open_failed} not_ready={not_ready} unknown_ssrc={unknown_ssrc} incomplete={incomplete} complete={complete} decrypt_failed={decrypt_failed} gated={gated} queue_full={queue_full} keyframes={keyframes} keyframes_without_params={keyframes_without_params} pli_sent={pli_sent} awaiting_ticks={awaiting_ticks} decoder_errors={decoder_errors} pictures={pictures} picture_gap_ms={picture_gap_ms}"
		));
	}
	if matches!(report.scope, Scope::ScreenAudio) {
		line.push_str(&format!(
			" capture_read={:?} capture_queue={:?} capture_restart={:?}",
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
			at_ms: 1234,
			window_ms: 5000,
			video: [0; VIDEO_SLOTS],
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
		assert!(line.ends_with("picture_gap_ms=1900\n"));
	}
}
