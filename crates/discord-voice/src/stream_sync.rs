//! Watched-stream timing. Audio is the master; authenticated sender reports relate
//! independent 48 kHz audio and 90 kHz video RTP epochs without trusting wall-clock time.
use crate::presentation::{Clock, Delay, MAX_AHEAD, MAX_LATE, difference, shift};
use std::{
	sync::Arc,
	time::{Duration, Instant},
};

pub(crate) const BUFFER: Duration = crate::presentation::INITIAL;
const MAX_TRACKS: usize = super::video_receive::MAX_SOURCES * 2;
const MAX_REPORT_AGE: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
pub(crate) struct SenderReport {
	pub ssrc: u32,
	pub ntp: u64,
	pub rtp: u32,
}
struct Track {
	output: Arc<Clock>,
	delay: Delay,
	updated: Option<Instant>,
	correction: f64,
	target: Duration,
	user: u64,
	ssrc: u32,
	rate: u32,
	last: Option<(u32, Instant)>,
	report: Option<(SenderReport, Instant)>,
}
struct Reference {
	user: u64,
	ssrc: u32,
	ntp: u64,
	deadline: Instant,
	seen: Instant,
}
#[derive(Default)]
pub(crate) struct Stats {
	pub audio_reports: u64,
	pub video_reports: u64,
	pub video_synced: u64,
	pub offset_ms: u64,
}
#[derive(Default)]
pub(crate) struct Sync {
	tracks: Vec<Track>,
	references: Vec<Reference>,
	stats: Stats,
}
fn ticks(delta: i32, rate: u32) -> i128 {
	i128::from(delta) * 1_000_000_000 / i128::from(rate)
}
impl Sync {
	pub fn take_stats(&mut self) -> Stats {
		std::mem::take(&mut self.stats)
	}
	pub fn clear(&mut self) {
		for track in &self.tracks {
			track.output.reset();
		}
		self.tracks.clear();
		self.references.clear();
	}
	/// A restarted audio buffer must establish a fresh local playback anchor too.
	/// Otherwise a future deadline can survive every packet-pressure reset and
	/// prevent the new buffer from ever reaching its first playable picture/sample.
	pub fn restart_audio(&mut self, ssrc: u32) {
		if let Some(track) = self
			.tracks
			.iter_mut()
			.find(|track| track.ssrc == ssrc && track.rate == 48_000)
		{
			track.last = None;
			track.report = None;
			track.delay = Delay::default();
			track.updated = None;
			track.correction = 0.0;
			track.target = BUFFER;
		}
		// Buffer starvation is not a session/owner change. Already accepted PCM
		// (including normal call voices mixed with it) and queued video remain valid.
		// Retiring the RTP anchor is sufficient to start buffering again; clear/retain
		// still invalidate the shared generation for security and source changes.
		self.references.retain(|reference| reference.ssrc != ssrc);
	}
	pub fn retain(&mut self, mut keep: impl FnMut(u64, u32, u32) -> bool) {
		self.tracks.retain(|track| {
			let retained = keep(track.user, track.ssrc, track.rate);
			if !retained {
				track.output.reset();
			}
			retained
		});
		self.references.retain(|reference| {
			self.tracks.iter().any(|track| {
				track.user == reference.user && track.ssrc == reference.ssrc && track.rate == 48_000
			})
		});
	}
	/// Only the caller's currently announced/authenticated media may register a clock.
	pub fn deadline(
		&mut self,
		user: u64,
		ssrc: u32,
		rate: u32,
		timestamp: u32,
		now: Instant,
	) -> Instant {
		debug_assert!(rate == 48_000 || rate == 90_000);
		if let Some(index) = self.tracks.iter().position(|track| track.ssrc == ssrc)
			&& (self.tracks[index].user != user || self.tracks[index].rate != rate)
		{
			self.tracks[index].output.reset();
			self.tracks.remove(index);
		}
		let index = match self.tracks.iter().position(|track| track.ssrc == ssrc) {
			Some(index) => index,
			None if self.tracks.len() < MAX_TRACKS => {
				let output = self
					.tracks
					.iter()
					.find(|track| track.user == user)
					.map(|track| track.output.clone())
					.unwrap_or_default();
				self.tracks.push(Track {
					output,
					delay: Delay::default(),
					updated: None,
					correction: 0.0,
					target: BUFFER,
					user,
					ssrc,
					rate,
					last: None,
					report: None,
				});
				self.tracks.len() - 1
			}
			None => return now + BUFFER,
		};
		let track = &mut self.tracks[index];
		let elapsed = track
			.updated
			.map_or(Duration::ZERO, |at| now.saturating_duration_since(at))
			.min(Duration::from_millis(100));
		track.updated = Some(now);
		if rate == 90_000 {
			track.delay.observe(timestamp, rate, now);
		}
		if let Some((last, deadline)) = track.last
			&& last == timestamp
		{
			return deadline;
		}
		let previous = track
			.last
			.and_then(|(last, deadline)| {
				let delta = timestamp.wrapping_sub(last);
				(delta < 0x8000_0000)
					.then(|| {
						shift(deadline, ticks(delta as i32, rate)).and_then(|deadline| {
							if deadline <= now + MAX_AHEAD {
								Some(deadline)
							} else if rate == 90_000 && delta <= rate / 2 {
								// Ordinary arrival jitter at the lookahead ceiling must not
								// rewind the video clock by 400 ms behind queued pictures.
								Some(now + MAX_AHEAD)
							} else {
								None
							}
						})
					})
					.flatten()
			})
			.filter(|deadline| now.saturating_duration_since(*deadline) <= MAX_LATE);
		let mut deadline = previous.unwrap_or(now + BUFFER);
		if rate == 90_000 {
			let referenced = self.references.iter().any(|reference| {
				reference.user == user
					&& now.saturating_duration_since(reference.seen) <= MAX_REPORT_AGE
			});
			let target = track.delay.target();
			if referenced {
				track.target = target;
			} else {
				// Releasing the audio reference must not suddenly apply the entire
				// accumulated fallback-target change and rewind queued video.
				let budget = (elapsed.as_nanos() * 60 / 1000) as i128;
				let change = (target.as_nanos() as i128 - track.target.as_nanos() as i128)
					.clamp(-budget, budget);
				deadline = shift(deadline, change).unwrap_or(deadline);
				track.target =
					Duration::from_nanos((track.target.as_nanos() as i128 + change) as u64);
			}
		}
		// Keep audio cadence continuous. Only video slews toward its audio master's
		// sender-clock mapping; acquiring a report must not skip/repeat PCM frames.
		if rate == 90_000
			&& let Some((report, seen)) = track.report
			&& now.saturating_duration_since(seen) <= MAX_REPORT_AGE
			&& let Some(reference) = self.references.iter().find(|reference| {
				reference.user == user
					&& now.saturating_duration_since(reference.seen) <= MAX_REPORT_AGE
			}) {
			let ntp_delta = i128::from(report.ntp.wrapping_sub(reference.ntp) as i64)
				* 1_000_000_000
				/ (1i128 << 32);
			if let Some(desired) = shift(
				reference.deadline,
				ntp_delta + ticks(timestamp.wrapping_sub(report.rtp) as i32, rate),
			) && desired <= now + Duration::from_secs(10)
				&& now.saturating_duration_since(desired) <= MAX_LATE
			{
				// Quantization/jitter can put a valid mapping just beyond the hold
				// horizon. Converge to the nearest supported target instead of ignoring it.
				let desired = desired.min(now + MAX_AHEAD);
				let correction = difference(desired, deadline);
				let alpha = 1.0 - (-elapsed.as_secs_f64() / 0.25).exp();
				track.correction += (correction as f64 - track.correction) * alpha;
				self.stats.video_synced += 1;
				self.stats.offset_ms = self
					.stats
					.offset_ms
					.max((correction.unsigned_abs() / 1_000_000) as u64);
				if track.correction.abs() > 5_000_000.0
					&& (track.correction as i128).signum() == correction.signum()
					&& correction.abs() > 5_000_000
				{
					let budget = ((elapsed.as_nanos() * 60 / 1000) as i128)
						.min(correction.abs() - 5_000_000);
					deadline = shift(deadline, (track.correction as i128).clamp(-budget, budget))
						.unwrap_or(deadline);
				}
			}
		}
		deadline = deadline.min(now + MAX_AHEAD);
		track.last = Some((timestamp, deadline));
		deadline
	}
	/// Anchor a playable audio block to the producer's actual clock. Its RTP
	/// position already includes bounded rate correction; extrapolating the original
	/// nominal anchor forever would accumulate sender/device drift and force resets.
	pub fn audio_frame(
		&mut self,
		user: u64,
		ssrc: u32,
		timestamp: u32,
		now: Instant,
	) -> Option<crate::Presentation> {
		let track = self
			.tracks
			.iter_mut()
			.find(|track| track.user == user && track.ssrc == ssrc && track.rate == 48_000)?;
		track.last = Some((timestamp, now));
		Some(track.output.stamp(now))
	}
	pub fn observe_audio(&mut self, ssrc: u32, timestamp: u32, now: Instant) {
		if let Some(track) = self
			.tracks
			.iter_mut()
			.find(|track| track.ssrc == ssrc && track.rate == 48_000)
		{
			track.output.expect_audio(now);
			track.delay.observe(timestamp, 48_000, now);
		}
	}
	pub fn target(&self, ssrc: u32) -> Duration {
		self.tracks
			.iter()
			.find(|track| track.ssrc == ssrc)
			.map_or(BUFFER, |track| track.delay.target())
	}
	pub fn presentation(&self, user: u64, at: Instant) -> Option<crate::Presentation> {
		self.tracks
			.iter()
			.find(|track| track.user == user)
			.map(|track| track.output.stamp(at))
	}
	pub fn sender_report(&mut self, report: SenderReport, now: Instant) {
		let Some(track) = self
			.tracks
			.iter_mut()
			.find(|track| track.ssrc == report.ssrc)
		else {
			return;
		};
		if report.ntp == 0 {
			return;
		}
		if let Some((last, _)) = track.report
			&& (report.ntp.wrapping_sub(last.ntp) as i64 <= 0)
		{
			return;
		}
		let Some((timestamp, deadline)) = track.last else {
			return;
		};
		let delta = report.rtp.wrapping_sub(timestamp) as i32;
		if i64::from(delta).unsigned_abs() > u64::from(track.rate) * 10 {
			return;
		}
		track.report = Some((report, now));
		if track.rate != 48_000 {
			self.stats.video_reports += 1;
			return;
		}
		self.stats.audio_reports += 1;
		let Some(deadline) = shift(deadline, ticks(delta, track.rate)) else {
			return;
		};
		let reference = Reference {
			user: track.user,
			ssrc: track.ssrc,
			ntp: report.ntp,
			deadline,
			seen: now,
		};
		if let Some(old) = self
			.references
			.iter_mut()
			.find(|reference| reference.user == track.user)
		{
			*old = reference;
		} else if self.references.len() < super::video_receive::MAX_SOURCES {
			self.references.push(reference);
		}
	}
}

impl Drop for Sync {
	fn drop(&mut self) {
		self.clear();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn independent_rtp_epochs_converge_at_30_and_60_fps_without_changing_audio_cadence() {
		let start = Instant::now();
		for fps in [30u32, 60] {
			let mut sync = Sync::default();
			let audio = u32::MAX - 480;
			let video = 800_000;
			assert_eq!(sync.deadline(7, 70, 48_000, audio, start), start + BUFFER);
			sync.deadline(7, 71, 90_000, video, start + Duration::from_millis(60));
			let ntp = u64::MAX - (1 << 31);
			sync.sender_report(
				SenderReport {
					ssrc: 71,
					ntp,
					rtp: video,
				},
				start,
			);
			sync.sender_report(
				SenderReport {
					ssrc: 70,
					ntp,
					rtp: audio,
				},
				start,
			);
			for n in 1..=fps * 3 {
				let elapsed = Duration::from_nanos(u64::from(n) * 1_000_000_000 / u64::from(fps));
				let now = start + elapsed;
				let audio_due =
					sync.deadline(7, 70, 48_000, audio.wrapping_add(n * 48_000 / fps), now);
				// Fractional 30/60 Hz intervals accumulate integer-nanosecond rounding only.
				assert!(difference(audio_due, now + BUFFER).abs() < 1000);
				let due = sync.deadline(7, 71, 90_000, video + n * 90_000 / fps, now);
				if n >= fps * 2 {
					assert!(
						difference(due, now + BUFFER).abs() <= 10_000_000,
						"{fps} fps failed to converge"
					);
				}
			}
		}
	}
	#[test]
	fn unannounced_stale_wrong_owner_and_reordered_reports_cannot_redirect_clocks() {
		let now = Instant::now();
		let mut sync = Sync::default();
		sync.sender_report(
			SenderReport {
				ssrc: 70,
				ntp: 1 << 32,
				rtp: 0,
			},
			now,
		);
		assert!(sync.tracks.is_empty());
		sync.deadline(7, 70, 48_000, 0, now);
		sync.sender_report(
			SenderReport {
				ssrc: 70,
				ntp: 2 << 32,
				rtp: 0,
			},
			now,
		);
		sync.sender_report(
			SenderReport {
				ssrc: 70,
				ntp: 1 << 32,
				rtp: 0,
			},
			now,
		);
		assert_eq!(sync.references[0].ntp, 2 << 32);
		sync.deadline(8, 71, 90_000, 100, now);
		sync.sender_report(
			SenderReport {
				ssrc: 71,
				ntp: 3 << 32,
				rtp: 100,
			},
			now,
		);
		let mut unrelated = Sync::default();
		unrelated.deadline(8, 71, 90_000, 100, now);
		assert_eq!(
			sync.deadline(8, 71, 90_000, 1900, now + Duration::from_millis(20)),
			unrelated.deadline(8, 71, 90_000, 1900, now + Duration::from_millis(20))
		);
		sync.deadline(7, 72, 48_000, 0, now);
		sync.retain(|_, source, _| source != 70);
		assert!(sync.references.is_empty());
		sync.clear();
		assert!(sync.tracks.is_empty());
	}
	#[test]
	fn received_audio_and_video_share_playback_time_after_sender_clock_acquisition() {
		use crate::{
			stream_audio_receive::Audio,
			video_playout::{Playout, Timed},
			video_receive::Encoded,
		};
		use opus2::{Application, Channels, Encoder};
		let start = Instant::now();
		let mut sync = Sync::default();
		let mut audio = Audio::default();
		audio.announce(7, 70).unwrap();
		let mut video = Playout::default();
		let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Audio).unwrap();
		let mut bytes = [0; 1275];
		let length = encoder.encode_float(&[0.1; 960], &mut bytes).unwrap();
		let opus = bytes[..length].to_vec();
		let mut audio_frames = 0;
		let mut video_frames = 0;
		for ms in 0..2300u64 {
			let now = start + Duration::from_millis(ms);
			if ms < 2000 && ms % 20 == 0 {
				audio.push(
					70,
					(ms / 20) as u16,
					500_000 + ms as u32 * 48,
					opus.clone(),
					&mut sync,
					now,
				);
			}
			// The first video packet arrives 40 ms later for the same captured moment.
			if (40..2040).contains(&ms) && (ms - 40) % 20 == 0 {
				let timestamp = 900_000 + (ms - 40) as u32 * 90;
				let due = sync.deadline(7, 71, 90_000, timestamp, now);
				video
					.push_at(
						Timed {
							frame: Encoded {
								user: 7,
								data: vec![1],
								keyframe: true,
							},
							source: 71,
						},
						now,
						Some(due),
					)
					.unwrap();
			}
			if ms == 200 {
				sync.sender_report(
					SenderReport {
						ssrc: 71,
						ntp: 1 << 32,
						rtp: 900_000,
					},
					now,
				);
				sync.sender_report(
					SenderReport {
						ssrc: 70,
						ntp: 1 << 32,
						rtp: 500_000,
					},
					now,
				);
			}
			if ms % 20 == 0 && audio.pop(&mut sync, now).is_some() {
				if audio_frames < 100 {
					assert_eq!(ms, 100 + audio_frames * 20);
				}
				audio_frames += 1;
			}
			if video.pop(now).unwrap().is_some() {
				if video_frames >= 50 {
					assert!((ms as i64 - (100 + video_frames * 20) as i64).abs() <= 10);
				}
				video_frames += 1;
			}
		}
		assert!(audio_frames >= 100);
		assert_eq!(video_frames, 100);
		assert_eq!(video.take_stats().dropped, 0);
	}

	#[test]
	fn delayed_audio_master_does_not_force_sixty_fps_video_into_keyframe_recovery() {
		use crate::{
			stream_audio_receive::Audio,
			video_playout::{Playout, Timed},
			video_receive::Encoded,
		};
		use opus2::{Application, Channels, Encoder};
		let start = Instant::now();
		let mut sync = Sync::default();
		let mut audio = Audio::default();
		audio.announce(7, 70).unwrap();
		let mut video = Playout::default();
		let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Audio).unwrap();
		let mut bytes = [0; 1275];
		let length = encoder.encode_float(&[0.1; 960], &mut bytes).unwrap();
		let opus = bytes[..length].to_vec();
		let mut next_video = 0u32;
		let mut decoded = 0u32;
		let mut audio_frames = 0u64;
		for ms in 0..11_000u64 {
			let now = start + Duration::from_millis(ms);
			// Model independently delayed audio RTP; receiver buffering stays 100 ms.
			if (400..10_400).contains(&ms) && (ms - 400) % 20 == 0 {
				let captured = (ms - 400) as u32;
				audio.push(
					70,
					(captured / 20) as u16,
					captured * 48,
					opus.clone(),
					&mut sync,
					now,
				);
			}
			// A repair releases 120 ms of complete pictures together, in media order.
			while next_video < 600 {
				let captured = u64::from(next_video) * 1000 / 60;
				let arrival = if (6000..6120).contains(&captured) {
					6120
				} else {
					captured
				};
				if arrival > ms {
					break;
				}
				let timestamp = next_video * 1500;
				let due = sync.deadline(7, 71, 90_000, timestamp, now);
				video
					.push_at(
						Timed {
							frame: Encoded {
								user: 7,
								data: vec![1],
								keyframe: next_video.is_multiple_of(120),
							},
							source: 71,
						},
						now,
						Some(due),
					)
					.expect("valid A/V alignment exhausted the picture queue");
				next_video += 1;
			}
			if ms != 0 && ms % 1000 == 0 {
				let ntp = (ms / 1000 + 100) << 32;
				sync.sender_report(
					SenderReport {
						ssrc: 70,
						ntp,
						rtp: ms as u32 * 48,
					},
					now,
				);
				sync.sender_report(
					SenderReport {
						ssrc: 71,
						ntp,
						rtp: ms as u32 * 90,
					},
					now,
				);
			}
			if ms % 20 == 0 && audio.pop(&mut sync, now).is_some() {
				if audio_frames < 500 {
					assert_eq!(ms, 500 + audio_frames * 20);
				}
				audio_frames += 1;
			}
			while video.pop(now).unwrap().is_some() {
				decoded += 1;
			}
		}
		assert_eq!(decoded, 600);
		assert!(audio_frames >= 500);
		let stats = video.take_stats();
		assert_eq!((stats.resets, stats.dropped), (0, 0));
	}

	#[test]
	fn lookahead_ceiling_jitter_does_not_rewind_video_behind_queued_pictures() {
		let start = Instant::now();
		let mut sync = Sync::default();
		sync.deadline(7, 71, 90_000, 0, start);
		sync.deadline(7, 70, 48_000, 0, start + Duration::from_millis(401));
		for ssrc in [70, 71] {
			sync.sender_report(
				SenderReport {
					ssrc,
					ntp: 100 << 32,
					rtp: 0,
				},
				start + Duration::from_millis(401),
			);
		}
		let mut previous = start;
		let mut last_arrival = start;
		for n in 1..=601u32 {
			let now = start + Duration::from_nanos(u64::from(n) * 16_666_667)
				- if n == 601 {
					Duration::from_millis(1)
				} else {
					Duration::ZERO
				};
			let due = sync.deadline(7, 71, 90_000, n * 1500, now);
			assert!(due >= previous, "arrival jitter rewound the playback clock");
			assert!(due <= now + MAX_AHEAD);
			previous = due;
			last_arrival = now;
		}
		assert!(previous.saturating_duration_since(last_arrival) >= Duration::from_millis(490));
	}
	#[test]
	fn an_hour_of_sender_drift_tracks_actual_audio_output_without_clock_resets() {
		let start = Instant::now();
		for ppm in [-500i64, 500] {
			let mut sync = Sync::default();
			sync.deadline(7, 70, 48_000, 0, start);
			for n in 0..180_000u64 {
				let now = start + Duration::from_millis(100 + n * 20);
				let audio = (n * 960 * (1_000_000 + ppm) as u64 / 1_000_000) as u32;
				let stamp = sync.audio_frame(7, 70, audio, now).unwrap();
				stamp.played(now + Duration::from_millis(73));
				let video = (n * 1800) as u32;
				if n % 50 == 0 {
					let ntp = ((100 + n / 50) << 32) | ((n % 50) * (1u64 << 32) / 50);
					sync.sender_report(
						SenderReport {
							ssrc: 70,
							ntp,
							rtp: audio,
						},
						now,
					);
					sync.deadline(7, 71, 90_000, video, now);
					sync.sender_report(
						SenderReport {
							ssrc: 71,
							ntp,
							rtp: video,
						},
						now,
					);
				}
				let due = sync.deadline(7, 71, 90_000, video, now);
				if n > 500 {
					let actual = sync.presentation(7, due).unwrap().deadline(now);
					assert!(
						difference(actual, now + Duration::from_millis(73)).abs() <= 10_000_000,
						"drift={ppm} ppm, tick={n}"
					);
				}
			}
		}
	}
}
