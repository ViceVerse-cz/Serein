//! One bounded local sound preparation session: pick, decode, preview and trim. Selecting,
//! previewing or trimming a file never uploads it.
use crate::notification_sounds::{Sounds, Source};
use eframe::egui;
use model::Id;
use std::{
	io::Read,
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
};

type Scope = (u64, Id, u64);
/// Name, duration in milliseconds, waveform peaks (one per 10 ms) and, when the file can be
/// uploaded untouched, its content type and bytes.
type Prepared = (String, u32, Vec<u8>, Option<(String, Vec<u8>)>);
type Selected = Result<Option<Prepared>, &'static str>;
/// A prepared sound with its decoded 48 kHz mono audio.
type Decoded = Result<Option<(Prepared, Arc<[f32]>)>, &'static str>;

/// Largest file opened for trimming.
const MAX_INPUT: u64 = 16 * 1024 * 1024;
/// Five minutes of decoded 48 kHz mono (57.6 MB) are kept while the dialog is open.
const MAX_DECODED: usize = 48_000 * 300;
/// Discord's 5.2-second limit plus codec padding.
const MAX_SAMPLES: usize = 48_000 * 53 / 10;
const PEAK_SAMPLES: usize = 480;

struct Choosing {
	scope: Scope,
	result: mpsc::Receiver<Decoded>,
	cancelled: Arc<AtomicBool>,
}
struct Trimming {
	scope: Scope,
	result: mpsc::Receiver<Result<Vec<u8>, &'static str>>,
}

#[derive(Default)]
pub struct SoundUpload {
	choosing: Option<Choosing>,
	/// Decoded audio of the sound under review.
	session: Option<(Scope, Arc<[f32]>)>,
	trimming: Option<Trimming>,
	player: Sounds,
}

impl SoundUpload {
	pub fn start(
		&mut self,
		scope: Scope,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
		parent: Arc<winit::window::Window>,
	) -> Result<(), &'static str> {
		if self.choosing.is_some() {
			return Err("Close the previous sound picker first");
		}
		self.clear();
		let dialog = platform::save::sound_source(parent);
		let (send, result) = mpsc::sync_channel(1);
		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let context = context.clone();
		runtime.spawn(async move {
			let result = match dialog.await {
				Some(path) if !flag.load(Ordering::Acquire) => {
					let flag = flag.clone();
					tokio::task::spawn_blocking(move || read(path, &flag).map(Some))
						.await
						.unwrap_or(Err("Sound preparation interrupted; choose it again"))
				}
				_ => Ok(None),
			};
			let _ = send.send(if flag.load(Ordering::Acquire) {
				Ok(None)
			} else {
				result
			});
			context.request_repaint();
		});
		self.choosing = Some(Choosing {
			scope,
			result,
			cancelled,
		});
		Ok(())
	}

	pub fn cancel(&self) {
		if let Some(job) = &self.choosing {
			job.cancelled.store(true, Ordering::Release);
		}
	}

	/// Release the decoded audio and stop any preview, e.g. when the review is closed.
	pub fn clear(&mut self) {
		self.session = None;
		self.trimming = None;
		self.player.stop();
	}

	pub fn poll(
		&mut self,
		generation: u64,
		valid: impl FnOnce(Id) -> bool,
	) -> Option<(Scope, Selected)> {
		let job = self.choosing.as_ref()?;
		if job.scope.0 != generation || !valid(job.scope.1) {
			self.cancel();
		}
		let result = match job.result.try_recv() {
			Ok(result) => result,
			Err(mpsc::TryRecvError::Empty) => return None,
			Err(mpsc::TryRecvError::Disconnected) => {
				Err("Sound preparation interrupted; choose it again")
			}
		};
		let job = self.choosing.take()?;
		if job.cancelled.load(Ordering::Acquire) {
			return None;
		}
		Some((
			job.scope,
			result.map(|prepared| {
				prepared.map(|(prepared, pcm)| {
					self.session = Some((job.scope, pcm));
					prepared
				})
			}),
		))
	}

	/// Play or stop the selected part of the sound under review on the default output.
	pub fn preview(
		&mut self,
		scope: Scope,
		selection: Option<(u32, u32, u8)>,
		context: &egui::Context,
	) {
		self.player.stop();
		if let (Some((start, end, volume)), Some((session, pcm))) = (selection, &self.session)
			&& *session == scope
			&& let Some(range) = samples(pcm.len(), start, end)
		{
			self.player
				.play_source(Source::Clip(pcm.clone(), range), volume.min(100), context);
		}
	}

	/// Encode the selected part on a worker; the result arrives through `poll_trim`.
	pub fn trim(&mut self, scope: Scope, start: u32, end: u32, context: &egui::Context) {
		let (send, result) = mpsc::sync_channel(1);
		self.trimming = Some(Trimming { scope, result });
		let job = self
			.session
			.as_ref()
			.filter(|(session, _)| *session == scope)
			.and_then(|(_, pcm)| samples(pcm.len(), start, end).map(|range| (pcm.clone(), range)));
		let Some((pcm, range)) = job else {
			let _ = send.send(Err("Choose the sound again before uploading"));
			return;
		};
		let context = context.clone();
		if std::thread::Builder::new()
			.name("serein-sound-trim".into())
			.spawn(move || {
				let _ = send.send(encode(&pcm[range]));
				context.request_repaint();
			})
			.is_err()
		{
			self.trimming = None;
		}
	}

	pub fn poll_trim(&mut self) -> Option<(Scope, Result<Vec<u8>, &'static str>)> {
		let job = self.trimming.as_ref()?;
		let result = match job.result.try_recv() {
			Ok(result) => result,
			Err(mpsc::TryRecvError::Empty) => return None,
			Err(mpsc::TryRecvError::Disconnected) => Err("Sound could not be encoded"),
		};
		Some((self.trimming.take()?.scope, result))
	}
}

impl Drop for SoundUpload {
	fn drop(&mut self) {
		self.cancel();
	}
}

/// The sample range of a selection of at most 5.2 seconds, or `None` when it is invalid.
fn samples(length: usize, start_ms: u32, end_ms: u32) -> Option<std::ops::Range<usize>> {
	let start = start_ms as usize * 48;
	let end = (end_ms as usize * 48).min(length);
	(start < end && end - start <= MAX_SAMPLES).then_some(start..end)
}

/// Fade the cut edges briefly so a trimmed clip does not click, then encode it.
fn encode(pcm: &[f32]) -> Result<Vec<u8>, &'static str> {
	let mut clip = pcm.to_vec();
	let fade = 240.min(clip.len() / 2);
	let length = clip.len();
	for index in 0..fade {
		let gain = index as f32 / fade as f32;
		clip[index] *= gain;
		clip[length - 1 - index] *= gain;
	}
	let file = crate::ogg_opus::encode(&clip)?;
	if file.len() > model::server_admin::MAX_SOUND_FILE_BYTES {
		return Err("Sound could not be encoded");
	}
	Ok(file)
}

fn read(path: PathBuf, cancelled: &AtomicBool) -> Result<(Prepared, Arc<[f32]>), &'static str> {
	if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > 4096 {
		return Err("Choose a local sound with a supported path");
	}
	let metadata =
		std::fs::symlink_metadata(&path).map_err(|_| "Could not open the chosen sound")?;
	if !metadata.is_file()
		|| metadata.file_type().is_symlink()
		|| metadata.len() == 0
		|| metadata.len() > MAX_INPUT
	{
		return Err("Choose a regular MP3, Ogg or WAV file up to 16 MB");
	}
	let mut file = Vec::with_capacity(metadata.len() as usize);
	std::fs::File::open(&path)
		.map_err(|_| "Could not open the chosen sound")?
		.take(MAX_INPUT + 1)
		.read_to_end(&mut file)
		.map_err(|_| "Could not read the chosen sound")?;
	if file.len() as u64 > MAX_INPUT {
		return Err("Choose a regular MP3, Ogg or WAV file up to 16 MB");
	}
	if cancelled.load(Ordering::Acquire) {
		return Err("Sound preparation cancelled");
	}
	// Audio past five minutes is not shown; the selection is at most 5.2 seconds anyway.
	let pcm: Arc<[f32]> = crate::audio::decode_clip(file.clone(), MAX_DECODED)
		.map_err(|_| "This sound could not be decoded; use an MP3, Ogg or WAV file")?
		.into();
	if cancelled.load(Ordering::Acquire) {
		return Err("Sound preparation cancelled");
	}
	let content_type = if file.starts_with(b"OggS") {
		"audio/ogg"
	} else {
		"audio/mpeg"
	};
	// A short MP3 or Ogg file within the service limits is uploaded as it is.
	let original = (pcm.len() <= MAX_SAMPLES
		&& discord_protocol::soundboard::valid_sound_file(content_type, &file))
	.then(|| (content_type.to_owned(), file));
	let peaks = pcm
		.chunks(PEAK_SAMPLES)
		.map(|chunk| {
			let peak = chunk
				.iter()
				.fold(0.0f32, |peak, sample| peak.max(sample.abs()));
			(peak.clamp(0.0, 1.0) * 255.0) as u8
		})
		.collect();
	let mut name: String = path
		.file_stem()
		.and_then(|name| name.to_str())
		.unwrap_or("")
		.trim()
		.chars()
		.filter(|character| !character.is_control())
		.take(32)
		.collect();
	if name.trim().chars().count() < 2 {
		name = "sound".into();
	}
	Ok((
		(
			name.trim().to_owned(),
			(pcm.len() / 48) as u32,
			peaks,
			original,
		),
		pcm,
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn prepared_sounds_keep_short_originals_and_long_or_wav_files_need_a_trim() {
		let root = std::env::temp_dir().join(format!("serein-sound-upload-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let idle = AtomicBool::new(false);
		let tone = include_bytes!("../tests/fixtures/audio-tone.mp3");
		let path = root.join("Synthetic tone with a rather long file name.mp3");
		std::fs::write(&path, tone).unwrap();
		let ((name, duration_ms, peaks, original), pcm) = read(path.clone(), &idle).unwrap();
		assert!((2..=32).contains(&name.chars().count()) && name.trim() == name);
		assert_eq!(duration_ms as usize, pcm.len() / 48);
		assert_eq!(peaks.len(), pcm.len().div_ceil(PEAK_SAMPLES));
		assert!(peaks.iter().any(|peak| *peak > 0));
		// Only a file already within the 5.2-second limit is offered unchanged.
		assert_eq!(
			original
				.as_ref()
				.map(|(kind, file)| (kind.as_str(), &file[..])),
			(pcm.len() <= MAX_SAMPLES).then_some(("audio/mpeg", &tone[..]))
		);
		assert!(read(path, &AtomicBool::new(true)).is_err());

		// A long WAV decodes for trimming but is never uploaded as it is.
		let mut wav = Vec::new();
		let data = vec![0u8; 48_000 * 2 * 8];
		wav.extend_from_slice(b"RIFF");
		wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
		wav.extend_from_slice(b"WAVEfmt ");
		wav.extend_from_slice(&16u32.to_le_bytes());
		wav.extend_from_slice(&1u16.to_le_bytes());
		wav.extend_from_slice(&1u16.to_le_bytes());
		wav.extend_from_slice(&48_000u32.to_le_bytes());
		wav.extend_from_slice(&96_000u32.to_le_bytes());
		wav.extend_from_slice(&2u16.to_le_bytes());
		wav.extend_from_slice(&16u16.to_le_bytes());
		wav.extend_from_slice(b"data");
		wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
		wav.extend_from_slice(&data);
		let long = root.join("eight seconds.wav");
		std::fs::write(&long, wav).unwrap();
		let ((_, duration_ms, peaks, original), pcm) = read(long, &idle).unwrap();
		assert_eq!((duration_ms, pcm.len(), peaks.len()), (8000, 384_000, 800));
		assert!(original.is_none());

		let damaged = root.join("damaged.ogg");
		std::fs::write(&damaged, b"OggS not really a stream").unwrap();
		assert!(read(damaged, &idle).is_err());
		assert!(read(PathBuf::from("relative.mp3"), &idle).is_err());
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn selections_are_bounded_and_trims_encode_a_faded_ogg_clip() {
		assert_eq!(samples(480_000, 1000, 6200), Some(48_000..297_600));
		assert_eq!(samples(480_000, 9000, 20_000), Some(432_000..480_000));
		assert_eq!(samples(480_000, 1000, 7000), None);
		assert_eq!(samples(480_000, 2000, 2000), None);
		assert_eq!(samples(480_000, 20_000, 21_000), None);

		let pcm = vec![0.5f32; 48_000];
		let file = encode(&pcm).unwrap();
		assert!(discord_protocol::soundboard::valid_sound_file(
			"audio/ogg",
			&file
		));
		let decoded = crate::audio::decode_clip(file, MAX_DECODED).unwrap();
		assert_eq!(decoded.len(), pcm.len());
		assert!(encode(&[]).is_err());
	}
}
