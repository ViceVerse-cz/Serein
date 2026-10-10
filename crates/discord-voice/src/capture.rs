//! Pace callback batches into 20 ms packets without discarding normal speech.
use crate::{CapturedFrame, Frame};
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub(crate) struct CapturePacer {
	// One frame of lookahead absorbs callback/worker jitter. The input channel
	// remains capped at eight frames: at most nine frames / 34,560 PCM bytes total.
	pending: Option<CapturedFrame>,
}

impl CapturePacer {
	/// Local detection while alone: consume audio without retaining it for transmission.
	pub fn preview(&mut self, input: &Receiver<CapturedFrame>, generation: u64) -> Option<Frame> {
		self.pending = None;
		let mut latest = None;
		for _ in 0..8 {
			let Ok(frame) = input.try_recv() else {
				break;
			};
			if frame.generation == generation {
				latest = Some(frame.pcm);
			}
		}
		latest
	}

	pub fn next(
		&mut self,
		input: &Receiver<CapturedFrame>,
		enabled: bool,
		stalled: bool,
		generation: u64,
	) -> Option<Frame> {
		if !enabled || stalled {
			self.pending = None;
			for _ in 0..8 {
				if input.try_recv().is_err() {
					break;
				}
			}
			return None;
		}
		let frame = self
			.pending
			.take()
			.filter(|frame| frame.generation == generation);
		for _ in 0..8 {
			let Ok(next) = input.try_recv() else { break };
			if next.generation == generation {
				self.pending = Some(next);
				break;
			}
		}
		frame.map(|frame| frame.pcm)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn coalesced_mute_unmute_discards_pre_transition_capture() {
		let (send, receive) = std::sync::mpsc::sync_channel(8);
		let (controls, mut latest) = tokio::sync::watch::channel(crate::Controls::default());
		let mut pacer = CapturePacer::default();
		send.send(CapturedFrame {
			generation: 0,
			pcm: [0.25; 960],
		})
		.unwrap();
		assert!(pacer.next(&receive, true, false, 0).is_none());
		controls.send_modify(|controls| {
			controls.muted = true;
			controls.capture_generation += 1;
		});
		controls.send_modify(|controls| {
			controls.muted = false;
			controls.capture_generation += 1;
		});
		// A transport tick sees only the latest state even after two control notifications.
		let control = *latest.borrow_and_update();
		assert!(!control.muted && !control.deafened);
		assert!(
			pacer
				.next(
					&receive,
					!control.muted && !control.deafened,
					false,
					control.capture_generation
				)
				.is_none()
		);
		// A late callback/worker send with the old tag cannot repopulate the queue.
		send.send(CapturedFrame {
			generation: 0,
			pcm: [0.5; 960],
		})
		.unwrap();
		send.send(CapturedFrame {
			generation: 2,
			pcm: [0.75; 960],
		})
		.unwrap();
		assert!(pacer.next(&receive, true, false, 2).is_none());
		// Playback-volume changes preserve current microphone PCM and its lookahead.
		controls.send_modify(|controls| controls.stream_volume = 50);
		assert_eq!(
			pacer.next(&receive, true, false, latest.borrow().capture_generation),
			Some([0.75; 960])
		);
	}
}
