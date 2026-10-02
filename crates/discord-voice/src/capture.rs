//! Pace callback batches into 20 ms packets without discarding normal speech.
use crate::Frame;
use std::sync::mpsc::Receiver;

pub(crate) struct CapturePacer<T = Frame> {
	// One frame of lookahead absorbs callback/worker jitter. The input channel
	// remains capped at eight frames: at most nine frames; at most 70 KiB for fixed stereo-capable CapturedFrame payloads.
	pending: Option<T>,
}

impl<T> Default for CapturePacer<T> {
	fn default() -> Self {
		Self { pending: None }
	}
}

impl<T> CapturePacer<T> {
	/// Local detection while alone: consume audio without retaining it for transmission.
	pub fn preview(&mut self, input: &Receiver<T>) -> Option<T> {
		self.pending = None;
		let mut latest = None;
		for _ in 0..8 {
			let Ok(frame) = input.try_recv() else {
				break;
			};
			latest = Some(frame);
		}
		latest
	}

	pub fn next(&mut self, input: &Receiver<T>, enabled: bool, stalled: bool) -> Option<T> {
		if !enabled || stalled {
			self.pending = None;
			for _ in 0..8 {
				if input.try_recv().is_err() {
					break;
				}
			}
			return None;
		}
		let frame = self.pending.take();
		self.pending = input.try_recv().ok();
		frame
	}
}
