//! Watched-stream timing. Audio is the master; authenticated sender reports relate
//! independent 48 kHz audio and 90 kHz video RTP epochs without trusting wall-clock time.
use std::time::{Duration, Instant};

pub(crate) const BUFFER: Duration = Duration::from_millis(100);
const MAX_TRACKS: usize = super::video_receive::MAX_SOURCES * 2;
const MAX_REPORT_AGE: Duration = Duration::from_secs(30);
const MAX_CORRECTION_NS: i128 = 2_000_000;

#[derive(Clone, Copy)]
pub(crate) struct SenderReport {
	pub ssrc: u32,
	pub ntp: u64,
	pub rtp: u32,
}
struct Track {
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
fn shift(at: Instant, nanos: i128) -> Option<Instant> {
	let duration = Duration::from_nanos(u64::try_from(nanos.unsigned_abs()).ok()?);
	if nanos < 0 {
		at.checked_sub(duration)
	} else {
		at.checked_add(duration)
	}
}
fn ticks(delta: i32, rate: u32) -> i128 {
	i128::from(delta) * 1_000_000_000 / i128::from(rate)
}
impl Sync {
	pub fn take_stats(&mut self) -> Stats {
		std::mem::take(&mut self.stats)
	}
	pub fn has_audio(&self, user: u64) -> bool {
		self.tracks
			.iter()
			.any(|track| track.user == user && track.rate == 48_000)
	}
	pub fn clear(&mut self) {
		self.tracks.clear();
		self.references.clear();
	}
	pub fn retain(&mut self, mut keep: impl FnMut(u64, u32, u32) -> bool) {
		self.tracks
			.retain(|track| keep(track.user, track.ssrc, track.rate));
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
			self.tracks.remove(index);
		}
		let index = match self.tracks.iter().position(|track| track.ssrc == ssrc) {
			Some(index) => index,
			None if self.tracks.len() < MAX_TRACKS => {
				self.tracks.push(Track {
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
					.then(|| shift(deadline, ticks(delta as i32, rate)))
					.flatten()
			})
			.filter(|deadline| {
				*deadline <= now + Duration::from_millis(500)
					&& now.saturating_duration_since(*deadline) <= Duration::from_millis(250)
			});
		let mut deadline = previous.unwrap_or(now + BUFFER);
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
			) && desired <= now + Duration::from_millis(500)
				&& now.saturating_duration_since(desired) <= Duration::from_millis(250)
			{
				let correction = if desired >= deadline {
					desired.duration_since(deadline).as_nanos() as i128
				} else {
					-(deadline.duration_since(desired).as_nanos() as i128)
				};
				self.stats.video_synced += 1;
				self.stats.offset_ms = self
					.stats
					.offset_ms
					.max((correction.unsigned_abs() / 1_000_000) as u64);
				deadline = shift(
					deadline,
					correction.clamp(-MAX_CORRECTION_NS, MAX_CORRECTION_NS),
				)
				.unwrap_or(deadline);
			}
		}
		track.last = Some((timestamp, deadline));
		deadline
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

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn independent_rtp_epochs_converge_to_audio_without_changing_audio_cadence() {
		let now = Instant::now();
		let mut sync = Sync::default();
		let audio = u32::MAX - 480;
		let video = 800_000;
		assert_eq!(sync.deadline(7, 70, 48_000, audio, now), now + BUFFER);
		assert_eq!(
			sync.deadline(7, 71, 90_000, video, now + Duration::from_millis(60)),
			now + BUFFER + Duration::from_millis(60)
		);
		let ntp = u64::MAX - (1 << 31);
		sync.sender_report(
			SenderReport {
				ssrc: 71,
				ntp,
				rtp: video,
			},
			now,
		);
		sync.sender_report(
			SenderReport {
				ssrc: 70,
				ntp,
				rtp: audio,
			},
			now,
		);
		for n in 1..=60 {
			let at = now + Duration::from_millis(n * 20);
			assert_eq!(
				sync.deadline(7, 70, 48_000, audio.wrapping_add(n as u32 * 960), at),
				at + BUFFER
			);
			let due = sync.deadline(7, 71, 90_000, video + n as u32 * 1800, at);
			if n >= 30 {
				assert_eq!(due, at + BUFFER);
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
		assert_eq!(
			sync.deadline(8, 71, 90_000, 1900, now + Duration::from_millis(20)),
			now + BUFFER + Duration::from_millis(20)
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
							timestamp,
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
					assert_eq!(ms, 100 + video_frames * 20);
				}
				video_frames += 1;
			}
		}
		assert!(audio_frames >= 100);
		assert_eq!(video_frames, 100);
		assert_eq!(video.take_stats().dropped, 0);
	}
}
