//! Local playback of soundboard sounds: received effects, this account's own sounds and
//! previews.
//!
//! Discord does not send soundboard audio over RTP: every client downloads the sound and plays
//! it itself. Decoded clips enter the call's playback mix here, on the transport tick, so they
//! use the selected output device, speaker volume and deafen gate like participant audio. They
//! never reach capture, echo processing or the encoder, so a preview stays on this device.
//! The mix runs whenever the call has media, including while this account is the only member
//! and DAVE waits for a peer: soundboard audio needs no remote participant or RTP.
use crate::Frame;
use std::sync::{Arc, mpsc::Receiver};

/// Simultaneous sounds; a newer sound replaces the oldest one past this bound.
pub const MAX_VOICES: usize = 8;
/// Mono 48 kHz samples per clip: Discord limits uploads to 5.2 seconds.
pub const MAX_CLIP_SAMPLES: usize = 48_000 * 6;

/// True when `SEREIN_VOICE_DIAGNOSTICS=1` enabled the opt-in voice diagnostics.
pub fn tracing() -> bool {
	crate::diagnostics::enabled()
}
/// Write one bounded soundboard pipeline line to the opt-in diagnostics. Callers must not
/// include tokens, user or channel IDs, or audio; a disabled or full writer drops the line.
pub fn trace(line: std::fmt::Arguments) {
	if tracing() {
		crate::diagnostics::trace(line);
	}
}

/// One decoded sound, ready to play: mono 48 kHz PCM and its final linear gain.
#[derive(Clone)]
pub struct Clip {
	samples: Arc<[f32]>,
	gain: f32,
	/// Playback attempt number shown in diagnostics; zero when untraced.
	attempt: u32,
}
impl Clip {
	/// `gain` combines Discord's per-sound volume with the local soundboard level; the speaker
	/// level is applied later by the output device, so it is not included here.
	pub fn new(samples: Arc<[f32]>, gain: f32) -> Option<Self> {
		(!samples.is_empty() && samples.len() <= MAX_CLIP_SAMPLES).then(|| Self {
			samples,
			gain: if gain.is_finite() {
				gain.clamp(0.0, 2.0)
			} else {
				0.0
			},
			attempt: 0,
		})
	}
	/// Correlate this clip's mixer and device stages with its download and decode lines.
	#[must_use]
	pub fn traced(mut self, attempt: u32) -> Self {
		self.attempt = attempt;
		self
	}
}

struct Playing {
	clip: Clip,
	position: usize,
	/// 20 ms frames handed to the audio device worker and frames its full queue refused.
	delivered: u32,
	dropped: u32,
}

#[derive(Default)]
pub(crate) struct Mixer {
	playing: Vec<Playing>,
}
impl Mixer {
	/// Admit clips queued since the previous tick; the oldest sound yields past the voice bound.
	pub fn admit(&mut self, source: &Receiver<Clip>) {
		for _ in 0..MAX_VOICES {
			let Ok(clip) = source.try_recv() else { break };
			if clip.gain == 0.0 {
				trace(format_args!(
					"attempt={} stage=mix result=muted",
					clip.attempt
				));
				continue;
			}
			if self.playing.len() == MAX_VOICES {
				let oldest = self.playing.remove(0);
				oldest.finish("replaced");
			}
			trace(format_args!(
				"attempt={} stage=mix result=started samples={} gain={:.2}",
				clip.attempt,
				clip.samples.len(),
				clip.gain
			));
			self.playing.push(Playing {
				clip,
				position: 0,
				delivered: 0,
				dropped: 0,
			});
		}
	}
	/// Add the next 20 ms of every playing sound to the call frame, or make a frame for them.
	pub fn mix(&mut self, call: Option<Frame>) -> Option<Frame> {
		if self.playing.is_empty() {
			return call;
		}
		let mut frame = call.unwrap_or([0.0; 960]);
		for playing in &mut self.playing {
			let samples = &playing.clip.samples;
			let end = (playing.position + frame.len()).min(samples.len());
			for (out, sample) in frame.iter_mut().zip(&samples[playing.position..end]) {
				let sample = if sample.is_finite() { *sample } else { 0.0 };
				*out += sample * playing.clip.gain;
			}
			playing.position = end;
		}
		for sample in &mut frame {
			*sample = sample.clamp(-1.0, 1.0);
		}
		Some(frame)
	}
	/// Record whether the frame carrying the current sounds reached the device worker, then
	/// retire sounds that ended. Called once per tick after `mix`.
	pub fn delivered(&mut self, accepted: bool) {
		self.playing.retain_mut(|playing| {
			if accepted {
				playing.delivered = playing.delivered.saturating_add(1);
			} else {
				playing.dropped = playing.dropped.saturating_add(1);
			}
			let done = playing.position >= playing.clip.samples.len();
			if done {
				playing.finish("finished");
			}
			!done
		});
	}
	/// Deafening or leaving drops sounds already playing, as Discord does.
	pub fn clear(&mut self, source: Option<&Receiver<Clip>>) {
		for playing in self.playing.drain(..) {
			playing.finish("stopped");
		}
		if let Some(source) = source {
			while let Ok(clip) = source.try_recv() {
				trace(format_args!(
					"attempt={} stage=mix result=discarded reason=speaker-gate-closed",
					clip.attempt
				));
			}
		}
	}
}
impl Playing {
	/// Trace the clip's last stage with how many of its frames reached the device worker.
	fn finish(&self, result: &str) {
		trace(format_args!(
			"attempt={} stage=device result={result} frames_delivered={} frames_dropped={}",
			self.clip.attempt, self.delivered, self.dropped
		));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Clips add to participant audio, end on their own, clip instead of wrapping and silence
	/// invalid samples.
	#[test]
	fn overlapping_sounds_mix_with_speech_and_finish_independently() {
		let (send, receive) = std::sync::mpsc::sync_channel(MAX_VOICES * 2);
		let mut mixer = Mixer::default();
		// No sound: participant frames and silence pass through untouched.
		assert_eq!(mixer.mix(None), None);
		send.send(Clip::new(vec![0.25; 1200].into(), 1.0).unwrap())
			.unwrap();
		send.send(Clip::new(vec![0.5; 960].into(), 0.5).unwrap())
			.unwrap();
		mixer.admit(&receive);
		let first = mixer.mix(Some([0.1; 960])).unwrap();
		mixer.delivered(true);
		assert!((first[0] - 0.6).abs() < 1e-6);
		// The shorter clip ended; the longer one supplies its last 240 samples alone.
		let second = mixer.mix(None).unwrap();
		mixer.delivered(false);
		assert!((second[0] - 0.25).abs() < 1e-6 && second[240] == 0.0);
		assert_eq!(mixer.mix(None), None);
		// Loud overlaps clip instead of wrapping, and invalid samples are silent.
		for _ in 0..3 {
			send.send(Clip::new(vec![0.9; 960].into(), 1.0).unwrap())
				.unwrap();
		}
		send.send(Clip::new(vec![f32::NAN; 960].into(), 1.0).unwrap())
			.unwrap();
		mixer.admit(&receive);
		assert_eq!(mixer.mix(None).unwrap()[0], 1.0);
	}

	/// The voice and clip-length bounds, and deafening flushing queued clips.
	#[test]
	fn voices_and_clip_lengths_are_bounded_and_deafen_flushes() {
		assert!(Clip::new(vec![0.0; MAX_CLIP_SAMPLES + 1].into(), 1.0).is_none());
		assert!(Clip::new(Vec::new().into(), 1.0).is_none());
		assert_eq!(
			Clip::new(vec![0.1].into(), f32::INFINITY).unwrap().gain,
			0.0
		);
		let (send, receive) = std::sync::mpsc::sync_channel(64);
		let mut mixer = Mixer::default();
		for index in 0..MAX_VOICES + 2 {
			send.send(Clip::new(vec![index as f32 / 100.0; 4800].into(), 1.0).unwrap())
				.unwrap();
		}
		mixer.admit(&receive);
		mixer.admit(&receive);
		assert_eq!(mixer.playing.len(), MAX_VOICES);
		// The two oldest sounds yielded to the newest ones.
		assert_eq!(mixer.playing[0].clip.samples[0], 0.02);
		send.send(Clip::new(vec![0.3; 960].into(), 1.0).unwrap())
			.unwrap();
		mixer.clear(Some(&receive));
		mixer.admit(&receive);
		assert_eq!(mixer.mix(None), None);
	}
}
