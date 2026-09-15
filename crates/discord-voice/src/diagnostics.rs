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
}

#[derive(Clone, Copy)]
struct Report {
	scope: Scope,
	window_ms: u64,
	// Each stage: calls, total elapsed microseconds, maximum elapsed microseconds.
	stages: [[u64; 3]; 6],
	wakes: u64,
	resets: u64,
	drops: u64,
	stalls: u64,
	noise_frames: u64,
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
				stages: [[0; 3]; 6],
				wakes: 0,
				resets: 0,
				drops: 0,
				stalls: 0,
				noise_frames: 0,
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

	fn flush(&mut self) {
		let Some(send) = self.send else { return };
		self.report.window_ms = self.since.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
		if let Err(mpsc::TrySendError::Disconnected(_)) = send.try_send(self.report) {
			self.send = None;
		}
		self.since = Instant::now();
		self.report.stages = [[0; 3]; 6];
		self.report.wakes = 0;
		self.report.resets = 0;
		self.report.drops = 0;
		self.report.stalls = 0;
		self.report.noise_frames = 0;
	}
}

impl Drop for Metrics {
	fn drop(&mut self) {
		self.flush();
	}
}

fn write_report(report: Report, bytes: &mut usize, writer: &mut impl Write) -> bool {
	let line = format!(
		"[Serein voice {:?}] debug={} window_ms={} wakes={} resets={} drops={} stalls={} noise_frames={} stages(calls,total_us,max_us): echo_render={:?} echo_capture={:?} noise={:?} encode={:?} mix={:?} receive={:?}\n",
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
	if line.len() > *bytes {
		return false;
	}
	// Charge attempted bytes even on a partial write. Failure never affects the call.
	*bytes -= line.len();
	writer.write_all(line.as_bytes()).is_ok()
}
