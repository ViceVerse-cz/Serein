//! One bounded model preparation task; inference stays on the existing audio worker.
//! Preparation only sees synthetic PCM. No device, network, persisted probe or PCM queue.
use df::tract::{DfParams, DfTract, FrozenDfTract, RuntimeParams};
use model::voice_settings::NoiseSuppression;
use ndarray::{ArrayView2, ArrayViewMut2};
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
	mpsc,
};
use std::time::{Duration, Instant};

static PREPARING: AtomicBool = AtomicBool::new(false);
const FRAME_BUDGET: Duration = Duration::from_millis(3);
const PROBE_LIMIT: Duration = Duration::from_millis(500);
const PROBE_FRAMES: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Status {
	Off,
	RnNoise,
	WebRtc,
	Loading,
	DeepFilter,
	Limited,
	Unavailable,
}
impl Status {
	const ALL: [Self; 7] = [
		Self::Off,
		Self::RnNoise,
		Self::WebRtc,
		Self::Loading,
		Self::DeepFilter,
		Self::Limited,
		Self::Unavailable,
	];
	/// Inverse of `status as u16`, for the worker's lock-free status atomic.
	pub fn from_repr(value: u16) -> Self {
		Self::ALL
			.into_iter()
			.find(|status| *status as u16 == value)
			.unwrap_or(Self::Off)
	}
	pub fn label(self) -> &'static str {
		match self {
			Self::Off => "voice-suppression-off",
			Self::RnNoise => "voice-suppression-rnnoise",
			Self::WebRtc => "voice-suppression-webrtc",
			Self::Loading => "voice-suppression-loading",
			Self::DeepFilter => "voice-suppression-deepfilter",
			Self::Limited => "voice-suppression-limited",
			Self::Unavailable => "voice-suppression-unavailable",
		}
	}
}
struct Slot;
impl Drop for Slot {
	fn drop(&mut self) {
		PREPARING.store(false, Ordering::Release);
	}
}
struct Pending {
	receive: mpsc::Receiver<Result<Prepared, ()>>,
	cancelled: Arc<AtomicBool>,
}
impl Drop for Pending {
	fn drop(&mut self) {
		self.cancelled.store(true, Ordering::Release);
	}
}
struct Prepared {
	model: FrozenDfTract,
	capable: bool,
}

#[derive(Default)]
struct Budget {
	micros: u64,
	frames: u16,
	consecutive: u8,
}
impl Budget {
	fn observe(&mut self, elapsed: Duration) -> bool {
		self.micros = self
			.micros
			.saturating_add(elapsed.as_micros().min(u128::from(u64::MAX)) as u64);
		self.frames += 1;
		self.consecutive = if elapsed > Duration::from_millis(7) {
			self.consecutive + 1
		} else {
			0
		};
		if self.consecutive >= 3 {
			return false;
		}
		if self.frames == 50 {
			let healthy = self.micros <= 50 * FRAME_BUDGET.as_micros() as u64;
			self.micros = 0;
			self.frames = 0;
			return healthy;
		}
		true
	}
}
fn frame(model: &mut DfTract, input: &[f32; 480], output: &mut [f32; 480]) -> Result<(), ()> {
	model
		.process(
			ArrayView2::from_shape((1, 480), input).map_err(|_| ())?,
			ArrayViewMut2::from_shape((1, 480), output).map_err(|_| ())?,
		)
		.map_err(|_| ())?;
	if output.iter().any(|x| !x.is_finite()) {
		return Err(());
	}
	Ok(())
}
fn prepare(cancelled: &AtomicBool) -> Result<Prepared, ()> {
	let mut model = Box::new(
		DfTract::new(
			DfParams::embedded().map_err(|_| ())?,
			&RuntimeParams::default(),
		)
		.map_err(|_| ())?,
	);
	if model.sr != 48000
		|| model.ch != 1
		|| model.hop_size != 480
		|| model.fft_size != 960
		|| model.lookahead != 2
	{
		return Err(());
	}
	// Exercise all neural stages, even when the synthetic signal is judged clean/noisy.
	let thresholds = (
		model.min_db_thresh,
		model.max_db_erb_thresh,
		model.max_db_df_thresh,
	);
	model.min_db_thresh = -100.0;
	model.max_db_erb_thresh = 100.0;
	model.max_db_df_thresh = 100.0;
	let mut costs = [Duration::ZERO; PROBE_FRAMES];
	let mut seed = 17_u32;
	let start = Instant::now();
	let mut completed = 0;
	for tick in 0..(16 + PROBE_FRAMES) {
		if cancelled.load(Ordering::Acquire) {
			return Err(());
		}
		let input = std::array::from_fn(|i| {
			seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
			let t = (tick * 480 + i) as f32 / 48000.0;
			0.06 * (t * 140.0 * std::f32::consts::TAU).sin()
				+ 0.03 * (t * 700.0 * std::f32::consts::TAU).sin()
				+ 0.025 * (seed as i32 as f32 / i32::MAX as f32)
		});
		let now = Instant::now();
		frame(&mut model, &input, &mut [0.0; 480])?;
		if tick >= 16 {
			costs[tick - 16] = now.elapsed();
			completed += 1;
		}
		if start.elapsed() > PROBE_LIMIT {
			break;
		}
	}
	costs.sort_unstable();
	let capable = completed == PROBE_FRAMES && costs[45] <= FRAME_BUDGET;
	(
		model.min_db_thresh,
		model.max_db_erb_thresh,
		model.max_db_df_thresh,
	) = thresholds;
	model.reset().map_err(|_| ())?;
	Ok(Prepared {
		model: (*model).freeze(),
		capable,
	})
}

pub(super) struct DeepFilter {
	mode: NoiseSuppression,
	pending: Option<Pending>,
	ready: Option<Box<DfTract>>,
	attempted: bool,
	status: Status,
	budget: Budget,
	capable: bool,
	attenuation: Option<u8>,
}
impl Default for DeepFilter {
	fn default() -> Self {
		Self {
			mode: NoiseSuppression::Off,
			pending: None,
			ready: None,
			attempted: false,
			status: Status::Off,
			budget: Budget::default(),
			capable: false,
			attenuation: None,
		}
	}
}
impl DeepFilter {
	pub fn status(&self) -> Status {
		self.status
	}
	pub fn configure(&mut self, mode: NoiseSuppression, strength: u8) {
		let wanted = matches!(
			mode,
			NoiseSuppression::Auto | NoiseSuppression::DeepFilterNet
		);
		if self.mode != mode {
			self.mode = mode;
			self.budget = Budget::default();
			self.attempted = self.pending.is_some() || self.ready.is_some();
			self.reset();
			if mode == NoiseSuppression::Auto && self.ready.is_some() && !self.capable {
				self.ready = None;
				self.status = Status::Limited;
			}
		}
		if !wanted {
			self.pending = None;
			self.ready = None;
			self.attenuation = None;
			self.attempted = false;
			self.status = match mode {
				NoiseSuppression::RnNoise => Status::RnNoise,
				NoiseSuppression::WebRtc => Status::WebRtc,
				_ => Status::Off,
			};
			return;
		}
		if !self.attempted {
			self.status = Status::Loading;
			if PREPARING
				.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
				.is_ok()
			{
				let slot = Slot;
				let (send, receive) = mpsc::sync_channel(1);
				let cancelled = Arc::new(AtomicBool::new(false));
				let cancel = cancelled.clone();
				self.attempted = true;
				match std::thread::Builder::new()
					.name("voice-noise-prepare".into())
					.spawn(move || {
						let _slot = slot;
						let result = prepare(&cancel);
						if !cancel.load(Ordering::Acquire) {
							let _ = send.try_send(result);
						}
					}) {
					Ok(_) => self.pending = Some(Pending { receive, cancelled }),
					Err(_) => self.status = Status::Unavailable,
				}
			}
		}
		if let Some(pending) = &self.pending {
			match pending.receive.try_recv() {
				Ok(Ok(prepared)) => {
					self.capable = prepared.capable;
					self.attenuation = None;
					if mode == NoiseSuppression::Auto && !prepared.capable {
						self.status = Status::Limited;
					} else {
						self.ready = Some(Box::new(prepared.model.unfreeze()));
						self.status = Status::DeepFilter;
					}
					self.pending = None;
				}
				Ok(Err(())) | Err(mpsc::TryRecvError::Disconnected) => {
					self.status = Status::Unavailable;
					self.pending = None;
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
		if self.attenuation != Some(strength)
			&& let Some(model) = &mut self.ready
		{
			model.set_atten_lim([6.0, 12.0, 24.0, 100.0][usize::from(strength.min(3))]);
			self.attenuation = Some(strength);
		}
	}
	pub fn reset(&mut self) {
		if self
			.ready
			.as_mut()
			.is_some_and(|model| model.reset().is_err())
		{
			self.ready = None;
			self.status = Status::Unavailable;
		}
		// Keep an automatic downgrade for the call; mute/PTT must not repeatedly reprobe.
	}
	/// False requests RNNoise for this block. Never exposes unprocessed PCM on failure.
	pub fn process(&mut self, samples: &mut [f32; 480]) -> bool {
		let Some(model) = &mut self.ready else {
			return false;
		};
		let mut output = [0.0; 480];
		let start = Instant::now();
		if frame(model, samples, &mut output).is_err() {
			self.ready = None;
			self.status = Status::Unavailable;
			return false;
		}
		let healthy = self.mode != NoiseSuppression::Auto || self.budget.observe(start.elapsed());
		if !healthy {
			self.ready = None;
			self.status = Status::Limited;
			return false;
		}
		*samples = output;
		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn cancelled_or_failed_preparation_cannot_reenable_suppression() {
		let (send, receive) = mpsc::sync_channel(1);
		let cancelled = Arc::new(AtomicBool::new(false));
		let mut filter = DeepFilter {
			mode: NoiseSuppression::Auto,
			attempted: true,
			status: Status::Loading,
			pending: Some(Pending {
				receive,
				cancelled: cancelled.clone(),
			}),
			..DeepFilter::default()
		};
		filter.configure(NoiseSuppression::Off, 2);
		assert!(cancelled.load(Ordering::Acquire));
		assert!(send.try_send(Err(())).is_err());
		assert!(filter.pending.is_none() && filter.ready.is_none());
		assert!(filter.status() == Status::Off);
		let (send, receive) = mpsc::sync_channel(1);
		filter.mode = NoiseSuppression::Auto;
		filter.attempted = true;
		filter.pending = Some(Pending {
			receive,
			cancelled: Arc::new(AtomicBool::new(false)),
		});
		assert!(send.try_send(Err(())).is_ok());
		filter.configure(NoiseSuppression::Auto, 2);
		assert!(filter.status() == Status::Unavailable);
		filter.reset();
		filter.configure(NoiseSuppression::Auto, 2);
		assert!(filter.status() == Status::Unavailable && filter.pending.is_none());
	}
	#[test]
	fn automatic_budget_tolerates_one_spike_and_bounds_sustained_load() {
		let mut budget = Budget::default();
		assert!(budget.observe(Duration::from_millis(12)));
		for _ in 0..49 {
			assert!(budget.observe(Duration::from_millis(1)));
		}
		for _ in 0..49 {
			assert!(budget.observe(Duration::from_millis(4)));
		}
		assert!(!budget.observe(Duration::from_millis(4)));
		let mut budget = Budget::default();
		assert!(budget.observe(Duration::from_millis(8)));
		assert!(budget.observe(Duration::from_millis(8)));
		assert!(!budget.observe(Duration::from_millis(8)));
	}
}
