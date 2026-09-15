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
}

#[derive(Clone, Copy)]
struct Report {
	scope: Scope,
	window_ms: u64,
	// Each stage: calls, total elapsed microseconds, maximum elapsed microseconds.
	stages: [[u64; 3]; 8],
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
					let mut bytes = 64 * 1024;
					for report in receive.iter().take(128) {
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
				window_ms: 0,
				stages: [[0; 3]; 8],
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

	fn flush(&mut self) {
		let Some(send) = self.send else { return };
		self.report.window_ms = self.since.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
		if let Err(mpsc::TrySendError::Disconnected(_)) = send.try_send(self.report) {
			self.send = None;
		}
		self.since = Instant::now();
		self.report.stages = [[0; 3]; 8];
		self.report.wakes = 0;
		self.report.resets = 0;
		self.report.drops = 0;
		self.report.stalls = 0;
		self.report.noise_frames = 0;
		self.report.stream_ticks = [0; 8];
		self.report.queued_audio = 0;
	}
}

impl Drop for Metrics {
	fn drop(&mut self) {
		self.flush();
	}
}

fn write_report(report: Report, bytes: &mut usize, writer: &mut impl Write) -> bool {
	let mut line = format!(
		"[Serein voice {:?}] debug={} window_ms={} wakes={} resets={} drops={} stalls={} noise_frames={} stages(calls,total_us,max_us): echo_render={:?} echo_capture={:?} noise={:?} encode={:?} mix={:?} receive={:?}",
		report.scope,
		cfg!(debug_assertions),
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
	line.push('\n');
	if line.len() > *bytes {
		return false;
	}
	// Charge attempted bytes even on a partial write. Failure never affects the call.
	*bytes -= line.len();
	writer.write_all(line.as_bytes()).is_ok()
}
