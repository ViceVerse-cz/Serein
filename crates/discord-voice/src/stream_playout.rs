//! Bridge stream/call ticks without cutting PCM during ordinary scheduling bursts.
use crate::PlaybackFrame;
use std::{collections::VecDeque, sync::mpsc::Receiver};

#[derive(Default)]
pub(crate) struct Playout {
	pending: VecDeque<PlaybackFrame>,
	started: bool,
	stats: [u64; 2],
}
impl Playout {
	pub fn take_stats(&mut self) -> [u64; 2] {
		std::mem::take(&mut self.stats)
	}
	pub fn next(
		&mut self,
		source: &Receiver<PlaybackFrame>,
		volume: u16,
		enabled: bool,
		stalled: bool,
	) -> Option<PlaybackFrame> {
		// Both channels are bounded to eight 20 ms frames. Start with 40 ms of
		// headroom so independently scheduled transport ticks can catch up without
		// cutting an entire PCM frame or inserting silence. Device feedback includes
		// this bridge delay when video follows audio. Real stalls still flush.
		for _ in 0..8 {
			let Ok(frame) = source.try_recv() else { break };
			if self.pending.len() == 8 {
				self.pending.pop_front();
				self.stats[0] += 1;
			}
			self.pending.push_back(frame);
		}
		if !enabled || stalled || volume == 0 {
			self.pending.clear();
			self.started = false;
			return None;
		}
		if !self.started && self.pending.len() < 3 {
			return None;
		}
		let Some(mut frame) = self.pending.pop_front() else {
			self.stats[1] += u64::from(self.started);
			self.started = false;
			return None;
		};
		self.started = true;
		let gain = f32::from(volume.min(200)) / 100.0;
		for sample in &mut frame.samples {
			*sample = if sample.is_finite() {
				(*sample * gain).clamp(-1.0, 1.0)
			} else {
				0.0
			};
		}
		Some(frame)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn scheduler_bursts_preserve_sample_order_and_a_continuous_output_cadence() {
		let (send, receive) = std::sync::mpsc::sync_channel(8);
		let mut playout = Playout::default();
		let mut expected = 0;
		for tick in 0..100u32 {
			let first = if tick % 10 == 7 { tick - 2 } else { tick };
			if !(5..7).contains(&(tick % 10)) {
				for n in first..=tick {
					send.send(([n as f32 / 100.0; 960]).into()).unwrap();
				}
			}
			let frame = playout.next(&receive, 100, true, false);
			if let Some(frame) = frame {
				assert_eq!((frame.samples[0] * 100.0).round() as u32, expected);
				expected += 1;
			} else {
				assert!(tick < 2, "a sub-stall scheduling burst interrupted PCM");
			}
		}
		assert!(playout.next(&receive, 0, false, false).is_none());
		assert!(playout.pending.is_empty());
		send.send([0.2; 960].into()).unwrap();
		assert!(playout.next(&receive, 100, true, true).is_none());
		assert!(playout.pending.is_empty());
	}
}
