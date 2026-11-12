//! Soundboard sounds: bounded CDN download, decode and cache on one lazy worker.
//!
//! Discord sends only `VOICE_CHANNEL_EFFECT_SEND`; the audio never arrives over RTP. Each
//! client fetches `cdn.discordapp.com/soundboard-sounds/{id}` and plays it locally. The CDN
//! serves the uploaded file as is, so the container is sniffed from the bytes (MP3, Ogg Vorbis,
//! Ogg Opus, WAV, FLAC or MP4/AAC), never taken from a header or extension. Received sounds,
//! this account's own sounds and local previews all take this path; network and decoding stay
//! here, off the UI, Gateway and audio-callback threads. Decoded clips go to the active call's
//! transport, which mixes them into playback only.
//!
//! With `SEREIN_VOICE_DIAGNOSTICS=1`, every attempt is traced by number through request,
//! cache, download, format, decode and queue; the transport continues the same attempt through
//! the mixer and the device. Lines name sound IDs, formats, sizes and outcomes only.
use client_core::soundboard::Playback;
use discord_voice::soundboard::{Clip, MAX_CLIP_SAMPLES, trace};
use model::{
	Id,
	soundboard::{MAX_SOUND_FILE_BYTES, MAX_SOUND_SECONDS},
};
use std::{
	collections::VecDeque,
	io::Cursor,
	sync::{
		Arc,
		atomic::{AtomicU32, Ordering},
		mpsc::{self, SyncSender},
	},
	time::{Duration, Instant},
};

/// Decoded sounds kept for replays; Discord sounds are at most about 1.1 MiB as 48 kHz PCM.
const MAX_CACHED: usize = 32;
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
/// A sound Discord refused or that cannot be decoded is not requested again for this long.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(60);
/// A network failure may be transient; the next play retries after this.
const NETWORK_COOLDOWN: Duration = Duration::from_secs(5);
const MAX_FAILURES: usize = 32;
/// Effects waiting for the worker; further ones are dropped rather than queued.
const MAX_JOBS: usize = 8;
/// A received sound that took longer than this to arrive is cached but not played late.
const MAX_LATENESS: Duration = Duration::from_secs(4);
/// A preview was asked for explicitly, so a slow first download still plays.
const MAX_PREVIEW_LATENESS: Duration = Duration::from_secs(12);
/// Redirects are followed only within Discord's CDN hosts.
const MAX_REDIRECTS: usize = 3;
/// Bytes searched for the first MPEG audio frame after ID3 tags.
const MAX_MP3_SYNC_SEARCH: usize = 64 * 1024;

struct Job {
	attempt: u32,
	playback: Playback,
	gain: f32,
	received: Instant,
	sink: SyncSender<Clip>,
	/// The `Player::epoch` this job was queued under; a later `cancel` makes it stale.
	epoch: u32,
}
impl Job {
	/// How long this job may wait before its sound is too late to play.
	fn limit(&self) -> Duration {
		if self.playback.source == client_core::soundboard::Source::Preview {
			MAX_PREVIEW_LATENESS
		} else {
			MAX_LATENESS
		}
	}
}

#[derive(Default)]
pub struct Player {
	jobs: Option<SyncSender<Job>>,
	attempts: u32,
	/// Bumped when a call ends; the worker discards jobs queued under an older value.
	epoch: Arc<AtomicU32>,
}
impl Player {
	/// Discard every queued or not-yet-started sound, for example when the call ends. A
	/// download already running finishes, but its clip is not played.
	pub fn cancel(&self) {
		self.epoch.fetch_add(1, Ordering::Relaxed);
	}
	/// Queue one sound for the call whose transport owns `sink`. A call that ended before the
	/// sound is ready has dropped its receiver, so the late clip goes nowhere.
	pub fn play(
		&mut self,
		playback: Playback,
		soundboard_percent: u16,
		sink: SyncSender<Clip>,
		runtime: &tokio::runtime::Handle,
	) {
		self.attempts = self.attempts.wrapping_add(1).max(1);
		let attempt = self.attempts;
		let effect = playback.effect;
		let gain = effect.volume.clamp(0.0, 1.0) * f32::from(soundboard_percent.min(200)) / 100.0;
		trace(format_args!(
			"attempt={attempt} stage=request source={} sound={} volume={:.2} soundboard={}% gain={gain:.2}",
			playback.source.label(),
			effect.sound,
			effect.volume,
			soundboard_percent
		));
		if gain <= 0.0 {
			trace(format_args!(
				"attempt={attempt} stage=request result=skipped reason=zero-volume"
			));
			return;
		}
		if self.jobs.is_none() {
			let (send, receive) = mpsc::sync_channel(MAX_JOBS);
			let runtime = runtime.clone();
			let epoch = Arc::clone(&self.epoch);
			if std::thread::Builder::new()
				.name("serein-soundboard".into())
				.spawn(move || worker(receive, runtime, epoch))
				.is_err()
			{
				trace(format_args!(
					"attempt={attempt} stage=request result=failed error=worker-unavailable"
				));
				return;
			}
			self.jobs = Some(send);
		}
		let job = Job {
			attempt,
			playback,
			gain,
			received: Instant::now(),
			sink,
			epoch: self.epoch.load(Ordering::Relaxed),
		};
		match self.jobs.as_ref().map(|jobs| jobs.try_send(job)) {
			Some(Ok(())) | None => {}
			Some(Err(mpsc::TrySendError::Full(_))) => trace(format_args!(
				"attempt={attempt} stage=request result=dropped reason=worker-busy"
			)),
			Some(Err(mpsc::TrySendError::Disconnected(_))) => {
				trace(format_args!(
					"attempt={attempt} stage=request result=failed error=worker-stopped"
				));
				self.jobs = None;
			}
		}
	}
}

/// The lazy `serein-soundboard` thread: serves jobs one at a time from the decoded cache or
/// the CDN and hands finished clips to the call that asked. Ends when the player is dropped.
fn worker(jobs: mpsc::Receiver<Job>, runtime: tokio::runtime::Handle, epoch: Arc<AtomicU32>) {
	let client = wreq::Client::builder()
		.https_only(true)
		.no_proxy()
		.redirect(wreq::redirect::Policy::custom(|attempt| {
			// The first entry of `previous` is the original request, not a redirect.
			if attempt.previous.len() > MAX_REDIRECTS {
				attempt.error("too many redirects")
			} else if discord_cdn(&attempt.uri) {
				attempt.follow()
			} else {
				attempt.stop()
			}
		}))
		.timeout(Duration::from_secs(10))
		.connect_timeout(Duration::from_secs(5))
		.build();
	let client = match client {
		Ok(client) => client,
		Err(_) => {
			trace(format_args!("stage=worker result=failed error=http-client"));
			return;
		}
	};
	let mut cache = Cache::default();
	while let Ok(job) = jobs.recv() {
		let attempt = job.attempt;
		let sound = job.playback.effect.sound;
		if job.epoch != epoch.load(Ordering::Relaxed) {
			trace(format_args!(
				"attempt={attempt} stage=queue result=dropped reason=call-ended"
			));
			continue;
		}
		let pcm = match cache.get(sound, Instant::now()) {
			Lookup::Hit(pcm) => {
				trace(format_args!(
					"attempt={attempt} stage=cache result=hit samples={}",
					pcm.len()
				));
				pcm
			}
			Lookup::CoolingDown(error) => {
				trace(format_args!(
					"attempt={attempt} stage=cache result=failed reason=recent-failure error=\"{error}\""
				));
				continue;
			}
			Lookup::Miss => {
				trace(format_args!("attempt={attempt} stage=cache result=miss"));
				// Too late to play already: do not spend a download on it, so a burst on a slow
				// connection cannot delay the sounds queued behind it.
				if job.received.elapsed() > job.limit() {
					trace(format_args!(
						"attempt={attempt} stage=queue result=dropped reason=late-before-download waited_ms={}",
						job.received.elapsed().as_millis()
					));
					continue;
				}
				match load(&client, &runtime, attempt, sound) {
					Ok(pcm) => cache.insert(sound, pcm),
					Err(failure) => {
						cache.fail(sound, failure, Instant::now());
						continue;
					}
				}
			}
		};
		let waited = job.received.elapsed();
		if job.epoch != epoch.load(Ordering::Relaxed) {
			trace(format_args!(
				"attempt={attempt} stage=queue result=dropped reason=call-ended"
			));
			continue;
		}
		if waited > job.limit() {
			trace(format_args!(
				"attempt={attempt} stage=queue result=dropped reason=late waited_ms={}",
				waited.as_millis()
			));
			continue;
		}
		let Some(clip) = Clip::new(pcm, job.gain) else {
			trace(format_args!(
				"attempt={attempt} stage=queue result=failed error=invalid-clip"
			));
			continue;
		};
		match job.sink.try_send(clip.traced(attempt)) {
			Ok(()) => trace(format_args!(
				"attempt={attempt} stage=queue result=queued waited_ms={}",
				waited.as_millis()
			)),
			Err(mpsc::TrySendError::Full(_)) => trace(format_args!(
				"attempt={attempt} stage=queue result=dropped reason=mixer-queue-full"
			)),
			Err(mpsc::TrySendError::Disconnected(_)) => trace(format_args!(
				"attempt={attempt} stage=queue result=dropped reason=call-ended"
			)),
		}
	}
}

/// Download and decode one sound, tracing each stage and the exact failure.
fn load(
	client: &wreq::Client,
	runtime: &tokio::runtime::Handle,
	attempt: u32,
	sound: Id,
) -> Result<Arc<[f32]>, Failure> {
	let started = Instant::now();
	let url = model::soundboard::sound_url(sound);
	let download = runtime.block_on(fetch(client, &url));
	let elapsed = started.elapsed().as_millis();
	let bytes = match download {
		Ok(download) => {
			trace(format_args!(
				"attempt={attempt} stage=download result=ok status={} content_type={} bytes={} ms={elapsed}",
				download.status,
				download.content_type,
				download.bytes.len()
			));
			download.bytes
		}
		Err(failure) => {
			trace(format_args!(
				"attempt={attempt} stage=download result=failed status={} content_type={} bytes={} ms={elapsed} error=\"{}\"",
				failure.status, failure.content_type, failure.bytes, failure.error
			));
			return Err(failure.into());
		}
	};
	let format = match sniff(&bytes) {
		Ok(format) => format,
		Err(error) => {
			trace(format_args!(
				"attempt={attempt} stage=format result=failed error=\"{error}\""
			));
			return Err(Failure::Permanent(error));
		}
	};
	trace(format_args!(
		"attempt={attempt} stage=format result=ok container={} codec={} offset={}",
		format.container.name(),
		format.codec,
		format.offset
	));
	match decode_detected(bytes, format) {
		Ok(decoded) => {
			trace(format_args!(
				"attempt={attempt} stage=decode result=ok rate={} channels={} frames={} skipped_packets={} samples_48k={} peak={:.3}",
				decoded.rate,
				decoded.channels,
				decoded.frames,
				decoded.skipped,
				decoded.pcm.len(),
				decoded
					.pcm
					.iter()
					.fold(0.0f32, |peak, sample| peak.max(sample.abs()))
			));
			Ok(decoded.pcm)
		}
		Err(error) => {
			trace(format_args!(
				"attempt={attempt} stage=decode result=failed container={} codec={} error=\"{error}\"",
				format.container.name(),
				format.codec
			));
			Err(Failure::Permanent(error))
		}
	}
}

/// Discord's CDN hosts, over HTTPS only.
fn discord_cdn(uri: &wreq::Uri) -> bool {
	uri.scheme_str() == Some("https")
		&& uri.host().is_some_and(|host| {
			["discordapp.com", "discordapp.net", "discord.com"]
				.iter()
				.any(|domain| {
					host == *domain
						|| host
							.strip_suffix(domain)
							.is_some_and(|prefix| prefix.ends_with('.'))
				})
		})
}

#[derive(Clone, Copy)]
enum Failure {
	/// The network or CDN may recover shortly.
	Transient(&'static str),
	/// Discord refused the sound or its bytes cannot be played.
	Permanent(&'static str),
}

struct Download {
	status: u16,
	content_type: String,
	bytes: Vec<u8>,
}
struct DownloadFailure {
	status: u16,
	content_type: String,
	bytes: usize,
	error: &'static str,
	transient: bool,
}
impl From<DownloadFailure> for Failure {
	/// Throttling, server errors and dropped connections are worth a quick retry; anything the
	/// response itself rules out is not.
	fn from(failure: DownloadFailure) -> Self {
		if failure.transient {
			Self::Transient(failure.error)
		} else {
			Self::Permanent(failure.error)
		}
	}
}

/// A short printable media type for diagnostics; never the full header.
fn content_type(response: &wreq::Response) -> String {
	let value = response
		.headers()
		.get(wreq::header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.unwrap_or("none");
	let value: String = value
		.chars()
		.filter(|c| c.is_ascii_alphanumeric() || "/.+-;=".contains(*c))
		.take(48)
		.collect();
	if value.is_empty() {
		"none".into()
	} else {
		value
	}
}

/// Download one sound file, refusing compression and anything over the size bound.
/// Redirects are left to the client's policy.
async fn fetch(client: &wreq::Client, url: &str) -> Result<Download, DownloadFailure> {
	let failure = |status: u16, content_type: String, bytes, error, transient| DownloadFailure {
		status,
		content_type,
		bytes,
		error,
		transient,
	};
	let response = client_core::fingerprint::discord_request(client.get(url), url)
		.header(wreq::header::ACCEPT_ENCODING, "identity")
		.send()
		.await
		.map_err(|error| {
			failure(
				0,
				"none".into(),
				0,
				if error.is_timeout() {
					"Sound download timed out"
				} else if error.is_redirect() {
					"Sound redirected too often"
				} else {
					"Sound download failed"
				},
				true,
			)
		})?;
	let status = response.status();
	let kind = content_type(&response);
	if status != wreq::StatusCode::OK {
		// Throttling and server errors are worth another try; anything else is the sound's.
		let transient = status == wreq::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
		return Err(failure(
			status.as_u16(),
			kind,
			0,
			if status.is_redirection() {
				"Sound redirected outside Discord's CDN"
			} else {
				"Sound unavailable"
			},
			transient,
		));
	}
	if response
		.headers()
		.get(wreq::header::CONTENT_ENCODING)
		.is_some_and(|value| value != "identity")
	{
		return Err(failure(200, kind, 0, "Sound was compressed", false));
	}
	let expected = response.content_length();
	if expected.is_some_and(|length| length > MAX_SOUND_FILE_BYTES as u64) {
		return Err(failure(200, kind, 0, "Sound too large", false));
	}
	let mut bytes = Vec::new();
	let mut body = std::pin::pin!(response.bytes_stream());
	loop {
		match futures_util::StreamExt::next(&mut body).await.transpose() {
			Ok(Some(chunk)) => {
				if bytes.len() + chunk.len() > MAX_SOUND_FILE_BYTES {
					return Err(failure(200, kind, bytes.len(), "Sound too large", false));
				}
				bytes.extend_from_slice(&chunk);
			}
			Ok(None) => break,
			// A body that cannot be decoded will not improve on retry; a dropped one may.
			Err(error) if error.is_decode() => {
				return Err(failure(
					200,
					kind,
					bytes.len(),
					"Sound body is damaged",
					false,
				));
			}
			Err(_) => {
				return Err(failure(
					200,
					kind,
					bytes.len(),
					"Sound download interrupted",
					true,
				));
			}
		}
	}
	if expected.is_some_and(|length| length != bytes.len() as u64) {
		return Err(failure(
			200,
			kind,
			bytes.len(),
			"Sound download incomplete",
			true,
		));
	}
	if bytes.is_empty() {
		return Err(failure(200, kind, 0, "Sound is empty", false));
	}
	Ok(Download {
		status: 200,
		content_type: kind,
		bytes,
	})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
	Mp3,
	Ogg,
	Wav,
	Flac,
	Mp4,
}
impl Container {
	/// Lower-case container name used in diagnostics.
	fn name(self) -> &'static str {
		match self {
			Self::Mp3 => "mpeg",
			Self::Ogg => "ogg",
			Self::Wav => "wav",
			Self::Flac => "flac",
			Self::Mp4 => "mp4",
		}
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Format {
	container: Container,
	codec: &'static str,
	/// Where the audio starts: after ID3 tags and padding for MP3, otherwise zero.
	offset: usize,
}

/// Identify the container and codec from the file's own bytes.
fn sniff(bytes: &[u8]) -> Result<Format, &'static str> {
	let format = |container, codec| {
		Ok(Format {
			container,
			codec,
			offset: 0,
		})
	};
	if bytes.starts_with(b"OggS") {
		// The first packet of the first page names the codec.
		let segments = usize::from(*bytes.get(26).ok_or("Ogg header cut off")?);
		let packet = bytes.get(27 + segments..).ok_or("Ogg header cut off")?;
		return if packet.starts_with(b"OpusHead") {
			format(Container::Ogg, "opus")
		} else if packet.starts_with(b"\x01vorbis") {
			format(Container::Ogg, "vorbis")
		} else if packet.starts_with(b"\x7fFLAC") {
			format(Container::Ogg, "flac")
		} else {
			Err("Ogg stream with an unsupported codec")
		};
	}
	if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
		return format(Container::Wav, "pcm");
	}
	if bytes.starts_with(b"fLaC") {
		return format(Container::Flac, "flac");
	}
	if bytes.get(4..8) == Some(b"ftyp") {
		return format(Container::Mp4, "aac/alac");
	}
	// MP3: skip ID3v2 tags, then find two consecutive valid MPEG audio frame headers.
	let mut offset = 0;
	for _ in 0..16 {
		let Some(header) = bytes
			.get(offset..offset + 10)
			.filter(|h| h.starts_with(b"ID3"))
		else {
			break;
		};
		if header[6..10].iter().any(|byte| byte & 0x80 != 0) {
			return Err("Damaged ID3 tag");
		}
		let len = header[6..10]
			.iter()
			.fold(0usize, |size, byte| (size << 7) | usize::from(*byte));
		let footer = usize::from(header[3] == 4 && header[5] & 0x10 != 0) * 10;
		offset = offset
			.checked_add(10 + len + footer)
			.filter(|end| *end < bytes.len())
			.ok_or("ID3 tag runs past the file")?;
	}
	let end = bytes.len().min(offset.saturating_add(MAX_MP3_SYNC_SEARCH));
	(offset..end)
		.find(|&start| {
			mpeg_frame_len(&bytes[start..]).is_some_and(|len| {
				// A single frame filling the rest of the file is accepted on its own.
				start + len == bytes.len()
					|| bytes.get(start + len..).and_then(mpeg_frame_len).is_some()
			})
		})
		.map(|start| Format {
			container: Container::Mp3,
			codec: "mp3",
			offset: start,
		})
		.ok_or("Unrecognized audio format")
}

/// The length of the MPEG audio frame whose header starts `bytes`, if it is a valid header.
fn mpeg_frame_len(bytes: &[u8]) -> Option<usize> {
	let &[0xff, b1, b2, ..] = bytes else {
		return None;
	};
	if b1 & 0xe0 != 0xe0 {
		return None;
	}
	let version = (b1 >> 3) & 3; // 0: MPEG 2.5, 1: reserved, 2: MPEG 2, 3: MPEG 1
	let layer = (b1 >> 1) & 3; // 0: reserved, 1: III, 2: II, 3: I
	let bitrate = usize::from(b2 >> 4);
	let rate = usize::from((b2 >> 2) & 3);
	if version == 1 || layer == 0 || bitrate == 0 || bitrate == 15 || rate == 3 {
		return None;
	}
	const MPEG1: [[u16; 15]; 3] = [
		[
			0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
		],
		[
			0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
		],
		[
			0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
		],
	];
	const MPEG2: [[u16; 15]; 2] = [
		[
			0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
		],
		[0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
	];
	let layer_index = usize::from(3 - layer); // 0: I, 1: II, 2: III
	let kbps = usize::from(if version == 3 {
		MPEG1[layer_index][bitrate]
	} else {
		MPEG2[layer_index.min(1)][bitrate]
	});
	let sample_rate = [44_100, 48_000, 32_000][rate]
		>> match version {
			3 => 0,
			2 => 1,
			_ => 2,
		};
	let padding = usize::from((b2 >> 1) & 1);
	let bits = kbps * 1000;
	Some(match layer_index {
		0 => (12 * bits / sample_rate + padding) * 4,
		2 if version != 3 => 72 * bits / sample_rate + padding,
		_ => 144 * bits / sample_rate + padding,
	})
	.filter(|len| *len > 4)
}

struct Decoded {
	pcm: Arc<[f32]>,
	rate: u32,
	channels: usize,
	frames: usize,
	skipped: u32,
}

/// Decode a sniffed file into mono 48 kHz PCM, keeping at most `MAX_SOUND_SECONDS`; longer
/// files are cut rather than rejected. Damaged frames are skipped within a bound.
fn decode_detected(mut bytes: Vec<u8>, format: Format) -> Result<Decoded, &'static str> {
	const FULL: &str = "full";
	let current = || true;
	match format.container {
		Container::Ogg => {
			let complete = crate::audio::check_ogg_headers_lenient(&bytes, &current)?;
			bytes.truncate(complete);
		}
		Container::Wav => crate::audio::prepare_media(&mut bytes)?,
		Container::Mp3 => {
			bytes.drain(..format.offset);
			// A trailing ID3v1 tag is not audio.
			if bytes.len() > 128 && bytes[bytes.len() - 128..].starts_with(b"TAG") {
				bytes.truncate(bytes.len() - 128);
			}
		}
		Container::Flac | Container::Mp4 => {}
	}
	let mut mono = Vec::new();
	let mut source = (0, 0);
	let result = crate::audio::decode_stream_lenient(
		Box::new(Cursor::new(bytes)),
		&current,
		&mut |chunk, channels, rate, _| {
			if source.0 != 0 && source != (rate, channels) {
				return Err("Sound changed sample rate or channels");
			}
			source = (rate, channels);
			let limit = rate as usize * MAX_SOUND_SECONDS;
			for frame in chunk.chunks_exact(channels) {
				if mono.len() == limit {
					return Err(FULL);
				}
				// Downmix any layout to mono by averaging the channels.
				let sum: f32 = frame
					.iter()
					.map(|sample| if sample.is_finite() { *sample } else { 0.0 })
					.sum();
				mono.push((sum / channels as f32).clamp(-1.0, 1.0));
			}
			Ok(())
		},
	);
	let skipped = match result {
		Ok(skipped) => skipped,
		Err(error) if error == FULL => 0,
		Err(error) => return Err(error),
	};
	let (rate, channels) = source;
	if mono.is_empty() || rate == 0 {
		return Err("Sound is empty");
	}
	Ok(Decoded {
		frames: mono.len(),
		pcm: resample(&mono, rate),
		rate,
		channels,
		skipped,
	})
}

/// Decode a downloaded file of any supported format; used by tests and fixtures.
#[cfg(test)]
fn decode(bytes: Vec<u8>) -> Result<Arc<[f32]>, &'static str> {
	let format = sniff(&bytes)?;
	decode_detected(bytes, format).map(|decoded| decoded.pcm)
}

/// Linear conversion to 48 kHz, matching the bundled notification cues.
fn resample(pcm: &[f32], rate: u32) -> Arc<[f32]> {
	if rate == 48_000 {
		return pcm[..pcm.len().min(MAX_CLIP_SAMPLES)].into();
	}
	let frames = pcm.len();
	let length = (frames * 48_000)
		.div_ceil(rate as usize)
		.min(MAX_CLIP_SAMPLES);
	(0..length)
		.map(|frame| {
			let position = frame as f64 * f64::from(rate) / 48_000.0;
			let index = (position as usize).min(frames - 1);
			let next = (index + 1).min(frames - 1);
			let a = pcm[index];
			a + (pcm[next] - a) * (position - index as f64) as f32
		})
		.collect()
}

enum Lookup {
	Hit(Arc<[f32]>),
	Miss,
	CoolingDown(&'static str),
}
/// Most recently used last. Bounded by entry count and decoded bytes.
#[derive(Default)]
struct Cache {
	clips: VecDeque<(Id, Arc<[f32]>)>,
	/// Failed sounds with the time they may be tried again and the reason.
	failures: VecDeque<(Id, Instant, &'static str)>,
}
impl Cache {
	/// Look `sound` up, marking a hit as most recently used. A sound that failed recently reports
	/// its error until its cooldown passes; anything else is a miss to download.
	fn get(&mut self, sound: Id, now: Instant) -> Lookup {
		if let Some(index) = self.clips.iter().position(|(id, _)| *id == sound) {
			let entry = self.clips.remove(index).expect("cached sound");
			let pcm = entry.1.clone();
			self.clips.push_back(entry);
			return Lookup::Hit(pcm);
		}
		self.failures.retain(|(_, retry, _)| now < *retry);
		match self.failures.iter().find(|(id, _, _)| *id == sound) {
			Some((_, _, error)) => Lookup::CoolingDown(error),
			None => Lookup::Miss,
		}
	}
	/// Cache freshly decoded PCM and return it, evicting least recently used sounds past either
	/// bound.
	fn insert(&mut self, sound: Id, pcm: Arc<[f32]>) -> Arc<[f32]> {
		self.clips.retain(|(id, _)| *id != sound);
		self.clips.push_back((sound, pcm.clone()));
		while self.clips.len() > MAX_CACHED || self.bytes() > MAX_CACHE_BYTES {
			self.clips.pop_front();
		}
		pcm
	}
	/// Remember a failure so the sound is not downloaded again before its cooldown: short for a
	/// transient failure, long for one the file itself caused.
	fn fail(&mut self, sound: Id, failure: Failure, now: Instant) {
		let (cooldown, error) = match failure {
			Failure::Transient(error) => (NETWORK_COOLDOWN, error),
			Failure::Permanent(error) => (FAILURE_COOLDOWN, error),
		};
		self.failures.retain(|(id, _, _)| *id != sound);
		if self.failures.len() == MAX_FAILURES {
			self.failures.pop_front();
		}
		self.failures.push_back((sound, now + cooldown, error));
	}
	/// Decoded PCM bytes held, for the byte bound.
	fn bytes(&self) -> usize {
		self.clips
			.iter()
			.map(|(_, pcm)| pcm.len() * size_of::<f32>())
			.sum()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A synthetic 16-bit PCM WAV sine tone.
	fn wav(rate: u32, channels: u16, seconds: f32) -> Vec<u8> {
		let frames = (rate as f32 * seconds) as usize;
		let mut data = Vec::new();
		for frame in 0..frames {
			let sample = ((frame as f32 * 0.05).sin() * 8000.0) as i16;
			for _ in 0..channels {
				data.extend_from_slice(&sample.to_le_bytes());
			}
		}
		let mut bytes = b"RIFF".to_vec();
		bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
		bytes.extend_from_slice(b"WAVEfmt ");
		bytes.extend_from_slice(&16u32.to_le_bytes());
		bytes.extend_from_slice(&1u16.to_le_bytes());
		bytes.extend_from_slice(&channels.to_le_bytes());
		bytes.extend_from_slice(&rate.to_le_bytes());
		bytes.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
		bytes.extend_from_slice(&(channels * 2).to_le_bytes());
		bytes.extend_from_slice(&16u16.to_le_bytes());
		bytes.extend_from_slice(b"data");
		bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
		bytes.extend_from_slice(&data);
		bytes
	}

	/// Channel downmix, resampling, the six-second cut and refusal of empty or damaged files.
	#[test]
	fn sounds_decode_to_bounded_mono_48k_and_reject_damaged_files() {
		let stereo = decode(wav(24_000, 2, 1.0)).unwrap();
		assert_eq!(stereo.len(), 48_000);
		assert!(stereo.iter().any(|sample| sample.abs() > 0.1));
		// A file longer than Discord allows is cut, not decoded in full.
		assert_eq!(decode(wav(48_000, 1, 9.0)).unwrap().len(), MAX_CLIP_SAMPLES);
		assert!(Clip::new(decode(wav(44_100, 1, 5.2)).unwrap(), 1.0).is_some());
		for damaged in [Vec::new(), b"not audio".to_vec(), wav(48_000, 1, 0.0)] {
			assert!(decode(damaged).is_err());
		}
		let mut truncated = wav(48_000, 1, 0.5);
		truncated[24..28].copy_from_slice(&0u32.to_le_bytes());
		assert!(decode(truncated).is_err());
	}

	/// Ogg page checksum (CRC-32, polynomial 0x04c11db7, no reflection).
	fn ogg_crc(page: &[u8]) -> u32 {
		page.iter().fold(0u32, |crc, byte| {
			(0..8).fold(crc ^ (u32::from(*byte) << 24), |crc, _| {
				if crc & 0x8000_0000 != 0 {
					(crc << 1) ^ 0x04c1_1db7
				} else {
					crc << 1
				}
			})
		})
	}

	/// Regression: files with defects common in uploaded sounds were silently dropped. Each
	/// defect is injected into a bundled fixture and must still decode.
	#[test]
	fn real_world_mp3_and_ogg_quirks_still_play() {
		let mp3 = include_bytes!("../tests/fixtures/audio-tone.mp3").to_vec();
		let ogg = include_bytes!("../tests/fixtures/voice-message.ogg").to_vec();
		assert_eq!(sniff(&mp3).unwrap().container, Container::Mp3);
		assert_eq!(sniff(&ogg).unwrap().codec, "opus");
		let reference = decode(mp3.clone()).unwrap();
		assert!(reference.iter().any(|sample| sample.abs() > 0.05));
		// Encoder padding between the ID3 tag and the first frame, a cut-off final frame and
		// a trailing ID3v1 tag: the strict attachment path rejects this file, a clip must not.
		let tag = 10
			+ mp3[6..10]
				.iter()
				.fold(0usize, |size, byte| (size << 7) | usize::from(*byte));
		let mut quirky = mp3[..tag].to_vec();
		quirky.extend_from_slice(&[0; 37]);
		quirky.extend_from_slice(&mp3[tag..mp3.len() - 100]);
		quirky.extend_from_slice(b"TAG");
		quirky.extend_from_slice(&[b' '; 125]);
		assert!(crate::audio::prepare_media(&mut quirky.clone()).is_err());
		assert_eq!(sniff(&quirky).unwrap().offset, tag + 37);
		let quirky = decode(quirky).unwrap();
		assert!(quirky.len() > reference.len() / 2);
		// A damaged frame in the middle is skipped, not fatal.
		let mut damaged = mp3.clone();
		let middle = sniff(&mp3).unwrap().offset + 3 * mpeg_frame_len(&mp3[tag..]).unwrap();
		damaged[middle + 4..middle + 40].fill(0xff);
		assert!(decode(damaged).is_ok());
		// An Ogg Opus clip without its end-of-stream page still plays.
		let mut cut = ogg.clone();
		cut.truncate(ogg.len() - 100);
		assert!(crate::audio::check_ogg_headers(&cut, &|| true).is_err());
		assert!(decode(cut).is_ok());
		// Channel mapping family 1 describing one plain stream is the same audio.
		let segments = usize::from(ogg[26]);
		let head = &ogg[27 + segments..27 + segments + usize::from(ogg[27])];
		let channels = head[9];
		let mut family1 = head.to_vec();
		family1[18] = 1;
		family1.extend_from_slice(&[1, channels - 1]);
		family1.extend(0..channels);
		let mut page = ogg[..26].to_vec();
		page.extend_from_slice(&[1, family1.len() as u8]);
		page.extend_from_slice(&family1);
		page[22..26].fill(0);
		let crc = ogg_crc(&page);
		page[22..26].copy_from_slice(&crc.to_le_bytes());
		page.extend_from_slice(&ogg[27 + segments + usize::from(ogg[27])..]);
		assert!(decode(page).is_ok());
		// Unknown bytes are reported as such instead of being fed to a decoder.
		assert_eq!(
			sniff(b"<html>nope</html>"),
			Err("Unrecognized audio format")
		);
	}

	/// Recency order, both cache bounds and the two failure cooldowns.
	#[test]
	fn cache_replays_recent_sounds_within_count_and_byte_bounds() {
		let now = Instant::now();
		let mut cache = Cache::default();
		assert!(matches!(cache.get(Id(1), now), Lookup::Miss));
		cache.insert(Id(1), vec![0.5; 4].into());
		cache.insert(Id(2), vec![0.0; 4].into());
		// Replaying Id(1) makes Id(2) the least recently used sound.
		assert!(matches!(cache.get(Id(1), now), Lookup::Hit(pcm) if pcm.len() == 4));
		for id in 3..=MAX_CACHED as u64 + 1 {
			cache.insert(Id(id), vec![0.0; 4].into());
		}
		assert_eq!(cache.clips.len(), MAX_CACHED);
		assert!(matches!(cache.get(Id(2), now), Lookup::Miss));
		assert!(matches!(cache.get(Id(1), now), Lookup::Hit(_)));
		for id in 100..120 {
			cache.insert(Id(id), vec![0.0; MAX_CLIP_SAMPLES].into());
		}
		assert!(cache.bytes() <= MAX_CACHE_BYTES);
		// A refused or undecodable sound waits out its cooldown; a network failure retries soon.
		cache.fail(Id(7), Failure::Permanent("Sound unavailable"), now);
		cache.fail(Id(8), Failure::Transient("Sound download failed"), now);
		assert!(matches!(cache.get(Id(7), now), Lookup::CoolingDown(_)));
		assert!(matches!(cache.get(Id(8), now), Lookup::CoolingDown(_)));
		assert!(matches!(
			cache.get(Id(8), now + NETWORK_COOLDOWN),
			Lookup::Miss
		));
		assert!(matches!(
			cache.get(Id(7), now + FAILURE_COOLDOWN),
			Lookup::Miss
		));
	}

	/// Only a plain 200 within the size bound is accepted; refusals are classified as permanent.
	#[tokio::test]
	async fn downloads_are_bounded_and_require_a_plain_ok_response() {
		use tokio::{
			io::{AsyncReadExt, AsyncWriteExt},
			net::TcpListener,
		};
		/// Answer one request on `listener` with the raw `response` bytes.
		async fn serve(listener: TcpListener, response: Vec<u8>) {
			let (mut stream, _) = listener.accept().await.unwrap();
			let mut request = Vec::new();
			while !request.windows(4).any(|w| w == b"\r\n\r\n") {
				let mut chunk = [0; 1024];
				let count = stream.read(&mut chunk).await.unwrap();
				assert!(count > 0);
				request.extend_from_slice(&chunk[..count]);
			}
			let _ = stream.write_all(&response).await;
		}
		let client = wreq::Client::builder()
			.no_proxy()
			.redirect(wreq::redirect::Policy::none())
			.build()
			.unwrap();
		let body = wav(48_000, 1, 0.1);
		let oversized = vec![0u8; MAX_SOUND_FILE_BYTES + 1];
		for (head, body, ok) in [
			("HTTP/1.1 200 OK", body.clone(), true),
			("HTTP/1.1 404 Not Found", body.clone(), false),
			(
				"HTTP/1.1 302 Found\r\nLocation: http://example.invalid/",
				Vec::new(),
				false,
			),
			("HTTP/1.1 200 OK\r\nContent-Encoding: gzip", body, false),
			("HTTP/1.1 200 OK", oversized, false),
		] {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let url = format!(
				"http://{}/soundboard-sounds/1",
				listener.local_addr().unwrap()
			);
			let mut response = format!(
				"{head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
				body.len()
			)
			.into_bytes();
			response.extend_from_slice(&body);
			let server = tokio::spawn(serve(listener, response));
			match fetch(&client, &url).await {
				Ok(download) => {
					assert!(ok, "{head}");
					assert!(decode(download.bytes).is_ok());
				}
				Err(failure) => assert!(!ok && !failure.transient, "{head}"),
			}
			server.abort();
		}
	}
}
