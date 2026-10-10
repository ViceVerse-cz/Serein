//! Short pre-decoded clips, such as soundboard sounds, mixed into call playback on the
//! transport clock. Local playback only: nothing here is encoded or sent.
use crate::Frame;
use std::sync::{Arc, Mutex};

const MAX_CLIPS: usize = 8;
/// Six seconds of 48 kHz mono; Discord limits a sound to 5.2 seconds.
pub const MAX_CLIP_SAMPLES: usize = 48_000 * 6;

struct Clip {
	pcm: Arc<[f32]>,
	position: usize,
	/// The participant who played the sound and the sound's own volume (0 to 1).
	source: u64,
	volume: f32,
	gain: f32,
}
#[derive(Clone, Default)]
pub struct Effects(Arc<Mutex<Vec<Clip>>>);
impl Effects {
	/// Queue 48 kHz mono PCM from `source` at `volume * gain`, capped at 2. A full mixer
	/// replaces its oldest clip.
	pub fn play(&self, pcm: Arc<[f32]>, source: u64, volume: f32, gain: f32) -> bool {
		let level = volume * gain;
		if pcm.is_empty() || pcm.len() > MAX_CLIP_SAMPLES || !level.is_finite() || level <= 0.0 {
			return false;
		}
		let Ok(mut clips) = self.0.lock() else {
			return false;
		};
		if clips.len() == MAX_CLIPS {
			clips.remove(0);
		}
		clips.push(Clip {
			pcm,
			position: 0,
			source,
			volume,
			gain: level.min(2.0),
		});
		true
	}
	/// Re-apply the listener's current gain for each clip's source, so muting, blocking or
	/// turning a participant down also affects sounds that are already playing. A zero gain
	/// drops the clip.
	pub fn retune(&self, gain: impl Fn(u64) -> f32) {
		let Ok(mut clips) = self.0.lock() else {
			return;
		};
		for clip in clips.iter_mut() {
			clip.gain = (clip.volume * gain(clip.source)).min(2.0);
		}
		clips.retain(|clip| clip.gain.is_finite() && clip.gain > 0.0);
	}
	pub fn clear(&self) {
		if let Ok(mut clips) = self.0.lock() {
			clips.clear();
		}
	}
	/// Add the next 20 ms of every active clip. Called once per transport tick, never from
	/// a device callback.
	pub(crate) fn mix(&self, frame: &mut Option<Frame>) {
		let Ok(mut clips) = self.0.lock() else {
			return;
		};
		if clips.is_empty() {
			return;
		}
		let output = frame.get_or_insert([0.0; 960]);
		for clip in clips.iter_mut() {
			let end = (clip.position + output.len()).min(clip.pcm.len());
			for (mixed, sample) in output.iter_mut().zip(&clip.pcm[clip.position..end]) {
				if sample.is_finite() {
					*mixed += sample * clip.gain;
				}
			}
			clip.position = end;
		}
		clips.retain(|clip| clip.position < clip.pcm.len());
		output
			.iter_mut()
			.for_each(|sample| *sample = sample.clamp(-1.0, 1.0));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn clips_mix_in_20ms_frames_with_gain_limits_and_a_bounded_queue() {
		let effects = Effects::default();
		let mut frame = None;
		effects.mix(&mut frame);
		assert!(frame.is_none(), "an idle mixer never invents playback");

		assert!(!effects.play(Arc::from([]), 1, 1.0, 1.0));
		assert!(!effects.play(vec![0.1; MAX_CLIP_SAMPLES + 1].into(), 1, 1.0, 1.0));
		assert!(!effects.play(vec![0.1; 10].into(), 1, 1.0, 0.0));
		assert!(!effects.play(vec![0.1; 10].into(), 1, f32::NAN, 1.0));

		// 1.5 frames at half gain over existing voice, then a clamped loud clip.
		assert!(effects.play(vec![0.5; 1440].into(), 1, 1.0, 0.5));
		let mut frame = Some([0.25; 960]);
		effects.mix(&mut frame);
		assert!(frame.unwrap().iter().all(|sample| *sample == 0.5));
		assert!(effects.play(vec![f32::NAN, 1.0, 1.0].into(), 1, 1.0, 9.0));
		let mut frame = None;
		effects.mix(&mut frame);
		let mixed = frame.unwrap();
		assert_eq!(mixed[0], 0.25);
		assert_eq!(&mixed[1..3], &[1.0, 1.0]);
		assert_eq!((mixed[479], mixed[480]), (0.25, 0.0));
		let mut frame = None;
		effects.mix(&mut frame);
		assert!(frame.is_none(), "finished clips are released");

		for index in 0..=MAX_CLIPS {
			assert!(effects.play(vec![index as f32 / 100.0; 960].into(), 1, 1.0, 1.0));
		}
		assert_eq!(effects.0.lock().unwrap().len(), MAX_CLIPS);
		assert_eq!(effects.0.lock().unwrap()[0].pcm[0], 0.01);
		effects.clear();
		let mut frame = None;
		effects.mix(&mut frame);
		assert!(frame.is_none());
	}
	#[test]
	fn retuning_applies_current_listener_gain_to_playing_clips() {
		let effects = Effects::default();
		assert!(effects.play(vec![0.5; 1920].into(), 7, 0.5, 1.0));
		assert!(effects.play(vec![0.5; 1920].into(), 9, 1.0, 1.0));
		// Participant 7 turned up, participant 9 muted or blocked mid-sound.
		effects.retune(|source| if source == 7 { 2.0 } else { 0.0 });
		let mut frame = None;
		effects.mix(&mut frame);
		assert!(frame.unwrap().iter().all(|sample| *sample == 0.5));
		assert_eq!(effects.0.lock().unwrap().len(), 1);
		effects.retune(|_| 0.0);
		let mut frame = None;
		effects.mix(&mut frame);
		assert!(frame.is_none(), "silenced clips are released");
	}
}
