//! Keep the independent stream/call audio clocks from accumulating stale queued PCM.
use crate::Frame;
use std::{collections::VecDeque, sync::mpsc::Receiver};

#[derive(Default)]
pub(crate) struct Playout {
	pending: VecDeque<Frame>,
}
impl Playout {
	pub fn next(
		&mut self,
		source: &Receiver<Frame>,
		volume: u16,
		enabled: bool,
		stalled: bool,
	) -> Option<Frame> {
		// ponytail: bound clock drift by trimming PCM; sender-clock A/V sync needs RTCP timing.
		// The source queue holds eight 20 ms frames. Retain at most two: absorb small
		// scheduling bursts without playing the full backlog after a delayed tick.
		for _ in 0..8 {
			let Ok(frame) = source.try_recv() else { break };
			if self.pending.len() == 2 {
				self.pending.pop_front();
			}
			self.pending.push_back(frame);
		}
		if !enabled || stalled || volume == 0 {
			self.pending.clear();
			return None;
		}
		let mut frame = self.pending.pop_front()?;
		let gain = f32::from(volume.min(200)) / 100.0;
		for sample in &mut frame {
			*sample = if sample.is_finite() {
				(*sample * gain).clamp(-1.0, 1.0)
			} else {
				0.0
			};
		}
		Some(frame)
	}
}
