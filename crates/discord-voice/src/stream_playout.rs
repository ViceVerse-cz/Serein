//! Bridge stream/call ticks without cutting PCM during ordinary scheduling bursts.
use crate::PlaybackFrame;
use std::{collections::VecDeque, sync::mpsc::Receiver};

const MAX_STREAMS: usize = client_core::voice::MAX_WATCHED_STREAMS;

struct Source {
	presentation: Option<crate::Presentation>,
	pending: VecDeque<PlaybackFrame>,
	started: bool,
}
#[derive(Default)]
pub(crate) struct Playout {
	sources: Vec<Source>,
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
		self.sources.retain(|source| {
			source
				.presentation
				.as_ref()
				.is_none_or(crate::Presentation::is_current)
		});
		for _ in 0..8 * MAX_STREAMS {
			let Ok(frame) = source.try_recv() else { break };
			if !enabled
				|| stalled || volume == 0
				|| frame
					.presentation
					.as_ref()
					.is_some_and(|tag| !tag.is_current())
			{
				continue;
			}
			let index = self.sources.iter().position(|source| {
				match (&source.presentation, &frame.presentation) {
					(Some(a), Some(b)) => a.same_source(b),
					(None, None) => true,
					_ => false,
				}
			});
			let index = match index {
				Some(index) => index,
				None if self.sources.len() < MAX_STREAMS => {
					self.sources.push(Source {
						presentation: frame.presentation.clone(),
						pending: VecDeque::new(),
						started: false,
					});
					self.sources.len() - 1
				}
				None => {
					self.stats[0] += 1;
					continue;
				}
			};
			let pending = &mut self.sources[index].pending;
			if pending.len() == 8 {
				pending.pop_front();
				self.stats[0] += 1;
			}
			pending.push_back(frame);
		}
		if !enabled || stalled || volume == 0 {
			self.sources.clear();
			return None;
		}
		let mut output: Option<PlaybackFrame> = None;
		let mut count = 0;
		for source in &mut self.sources {
			// Each stream retains its own 40 ms scheduling headroom and sample order.
			if !source.started && source.pending.len() < 3 {
				continue;
			}
			let Some(frame) = source.pending.pop_front() else {
				self.stats[1] += u64::from(source.started);
				source.started = false;
				continue;
			};
			source.started = true;
			if let Some(output) = &mut output {
				for (out, sample) in output.samples.iter_mut().zip(frame.samples) {
					*out += if sample.is_finite() { sample } else { 0.0 };
				}
				output.additional_presentations[count - 1] = frame.presentation;
				output.mixed = true;
			} else {
				let mut frame = frame;
				for sample in &mut frame.samples {
					if !sample.is_finite() {
						*sample = 0.0;
					}
				}
				output = Some(frame);
			}
			count += 1;
		}
		let gain = f32::from(volume.min(200)) / 100.0;
		if let Some(output) = &mut output {
			for sample in &mut output.samples {
				*sample = (*sample * gain).clamp(-1.0, 1.0);
			}
		}
		output
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn concurrent_stream_audio_mixes_samples_and_preserves_independent_clocks() {
		use std::{sync::Arc, time::Instant};
		let clocks: Vec<_> = (0..MAX_STREAMS)
			.map(|_| Arc::new(crate::presentation::Clock::default()))
			.collect();
		let (send, receive) = std::sync::mpsc::sync_channel(8 * MAX_STREAMS);
		let mut playout = Playout::default();
		let at = Instant::now();
		for n in 0..3 {
			for (index, clock) in clocks.iter().enumerate() {
				send.send(PlaybackFrame {
					samples: [(index + n + 1) as f32 / 100.0; 960],
					presentation: Some(clock.stamp(at)),
					additional_presentations: Default::default(),
					mixed: false,
				})
				.unwrap();
			}
		}
		let frame = playout.next(&receive, 100, true, false).unwrap();
		assert!((frame.samples[0] - 0.1).abs() < 0.00001);
		let tags: Vec<_> = frame
			.presentation
			.iter()
			.chain(frame.additional_presentations.iter().flatten())
			.collect();
		assert_eq!(tags.len(), MAX_STREAMS);
		for (tag, clock) in tags.iter().zip(&clocks) {
			assert!(tag.same_source(&clock.stamp(at)));
		}
		clocks[1].reset();
		let frame = playout.next(&receive, 100, true, false).unwrap();
		assert!((frame.samples[0] - 0.11).abs() < 0.00001);
		assert_eq!(playout.sources.len(), MAX_STREAMS - 1);
		clocks[0].reset();
		assert!(
			PlaybackFrame::mix(None, Some(frame)).is_some(),
			"retiring the first source must preserve other mixed streams"
		);
		assert!(playout.next(&receive, 0, true, false).is_none());
		assert!(playout.sources.is_empty());
	}

	#[test]
	fn retired_stream_frames_do_not_delay_or_mix_with_the_replacement() {
		let clock = std::sync::Arc::new(crate::presentation::Clock::default());
		let (send, receive) = std::sync::mpsc::sync_channel(8);
		let mut playout = Playout::default();
		for _ in 0..3 {
			send.send(PlaybackFrame {
				samples: [0.8; 960],
				presentation: Some(clock.stamp(std::time::Instant::now())),
				additional_presentations: Default::default(),
				mixed: false,
			})
			.unwrap();
		}
		assert_eq!(
			playout.next(&receive, 100, true, false).unwrap().samples[0],
			0.8
		);
		clock.reset();
		assert!(playout.next(&receive, 100, true, false).is_none());
		for _ in 0..3 {
			send.send([0.2; 960].into()).unwrap();
		}
		assert_eq!(
			playout.next(&receive, 100, true, false).unwrap().samples[0],
			0.2
		);
	}
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
		assert!(playout.sources.is_empty());
		send.send([0.2; 960].into()).unwrap();
		assert!(playout.next(&receive, 100, true, true).is_none());
		assert!(playout.sources.is_empty());
	}
}
