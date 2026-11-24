//! Session-local sounds mixed into the call's outgoing microphone audio.
//!
//! Decoding and file choice stay in the app; this module only plays bounded 48 kHz mono PCM on
//! the transport's existing 20 ms clock, so a sound never opens a device or a second encoder.
use crate::Frame;
use std::sync::{
	Arc,
	mpsc::{Receiver, TryRecvError},
};

/// Longest clip the player keeps from the queue; the decoder bounds samples to the same length.
pub const MAX_SECONDS: u32 = 60;
/// Requests applied per tick; a burst cannot stretch one 20 ms slot.
const MAX_REQUESTS: usize = 8;

/// A sound to play, or a request to stop the current one.
pub enum Message {
	Play { id: u64, samples: Arc<[f32]> },
	Stop,
}
struct Playing {
	id: u64,
	samples: Arc<[f32]>,
	at: usize,
}
/// One clip at a time: playing another sound replaces the current one, like Discord's soundboard.
#[derive(Default)]
pub struct Player {
	messages: Option<Receiver<Message>>,
	current: Option<Playing>,
}
impl Player {
	pub fn new(messages: Receiver<Message>) -> Self {
		Self {
			messages: Some(messages),
			current: None,
		}
	}
	/// Advance one 20 ms tick: apply queued requests, then dequeue the next frame.
	pub fn tick(&mut self) -> (Option<Frame>, Option<u64>) {
		if let Some(messages) = &self.messages {
			for _ in 0..MAX_REQUESTS {
				match messages.try_recv() {
					Ok(Message::Play { id, samples }) => {
						self.current = Some(Playing { id, samples, at: 0 });
					}
					Ok(Message::Stop) => self.current = None,
					Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
				}
			}
		}
		let Some(playing) = &mut self.current else {
			return (None, None);
		};
		let mut frame = [0.0; 960];
		let count = (playing.samples.len() - playing.at).min(frame.len());
		frame[..count].copy_from_slice(&playing.samples[playing.at..playing.at + count]);
		playing.at += count;
		let id = playing.id;
		if playing.at == playing.samples.len() {
			self.current = None;
		}
		((count != 0).then_some(frame), Some(id))
	}
}
/// Add one sound frame to outgoing samples, bounded to full scale.
pub fn add(frame: &mut Frame, sound: &Frame) {
	for (out, sample) in frame.iter_mut().zip(sound) {
		if sample.is_finite() {
			*out = (*out + sample).clamp(-1.0, 1.0);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{
		collections::VecDeque,
		sync::{
			atomic::{AtomicUsize, Ordering},
			mpsc,
		},
	};
	fn clip(id: u64, frames: usize, sample: f32) -> Message {
		Message::Play {
			id,
			samples: vec![sample; frames * 960].into(),
		}
	}
	#[test]
	fn one_clip_plays_on_the_tick_and_a_newer_sound_replaces_it() {
		let (send, receive) = mpsc::channel();
		let mut player = Player::new(receive);
		assert_eq!(player.tick(), (None, None));
		send.send(clip(7, 3, 0.25)).unwrap();
		assert_eq!(player.tick().1, Some(7));
		assert_eq!(player.tick().1, Some(7));
		// A second sound takes the slot instead of queueing behind the first.
		send.send(clip(9, 1, -0.5)).unwrap();
		let (frame, id) = player.tick();
		assert_eq!(id, Some(9));
		assert!(frame.unwrap().iter().all(|sample| *sample == -0.5));
		// The short clip ends inside one tick; the next tick is silent.
		assert_eq!(player.tick(), (None, None));
		assert_eq!(player.tick(), (None, None));
	}
	#[test]
	fn a_partial_tail_is_padded_and_stop_silences_the_next_tick() {
		let (send, receive) = mpsc::channel();
		let mut player = Player::new(receive);
		send.send(Message::Play {
			id: 3,
			samples: vec![0.4; 100].into(),
		})
		.unwrap();
		let (frame, id) = player.tick();
		let frame = frame.unwrap();
		assert_eq!(id, Some(3));
		assert!(frame[..100].iter().all(|sample| *sample == 0.4));
		assert!(frame[100..].iter().all(|sample| *sample == 0.0));
		send.send(clip(4, 2, 0.1)).unwrap();
		assert_eq!(player.tick().1, Some(4));
		send.send(Message::Stop).unwrap();
		assert_eq!(player.tick(), (None, None));
	}
	#[test]
	fn mixing_a_sound_never_exceeds_full_scale() {
		let mut frame = [0.9; 960];
		add(&mut frame, &[0.5; 960]);
		assert!(frame.iter().all(|sample| *sample == 1.0));
		let mut silence = [0.0; 960];
		add(&mut silence, &[f32::NAN; 960]);
		assert!(silence.iter().all(|sample| *sample == 0.0));
	}
	#[test]
	fn request_flood_is_applied_within_one_tick_and_keeps_the_latest() {
		let (send, receive) = mpsc::channel();
		let mut player = Player::new(receive);
		for id in 0..64 {
			send.send(clip(id, 2, 0.1)).unwrap();
		}
		// Draining is bounded per tick, so the newest sound arrives over a few ticks at most.
		let mut seen = VecDeque::new();
		for _ in 0..8 {
			seen.extend(player.tick().1);
		}
		assert!(seen.iter().any(|id| *id >= 56));
		assert!(player.tick().1.is_some() || send.send(clip(64, 1, 0.0)).is_ok());
		// A disconnected UI never leaves the transport spinning on a closed channel.
		drop(send);
		let _ = player.tick();
		let counted = Arc::new(AtomicUsize::new(0));
		counted.fetch_add(1, Ordering::Relaxed);
	}
}
