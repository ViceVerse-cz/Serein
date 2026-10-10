//! Soundboard playback: credential-free CDN download, bounded decode and a session-only
//! PCM cache. Clips are mixed into the connected call's playback; nothing is written to disk.
use discord_voice::{Effects, MAX_CLIP_SAMPLES};
use model::Id;
use std::{
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};

/// Discord caps an uploaded sound at 512 KiB; the margin covers its bundled defaults.
const MAX_SOUND_BYTES: usize = 1024 * 1024;
const MAX_CACHED: usize = 16;
/// Ten full-length clips: at most 11.5 MiB of retained PCM.
const MAX_CACHED_SAMPLES: usize = MAX_CLIP_SAMPLES * 10;
const MAX_LOADS: usize = 4;
/// A sound that finishes loading later than this is kept for next time, not played.
const MAX_LATENESS: Duration = Duration::from_secs(3);

type Clip = Arc<[f32]>;
#[derive(Default)]
struct Shared {
	/// Least recently played first.
	clips: Vec<(Id, Clip)>,
	loading: Vec<Id>,
}
impl Shared {
	fn insert(&mut self, sound: Id, clip: Clip) {
		self.clips.retain(|(id, _)| *id != sound);
		self.clips.push((sound, clip));
		while self.clips.len() > MAX_CACHED
			|| self.clips.iter().map(|(_, clip)| clip.len()).sum::<usize>() > MAX_CACHED_SAMPLES
		{
			self.clips.remove(0);
		}
	}
}
#[derive(Default)]
pub struct Soundboard {
	shared: Arc<Mutex<Shared>>,
	client: Option<reqwest::Client>,
}
impl Soundboard {
	/// Mix `sound` played by `source` into `effects` at `volume * gain`, downloading it first
	/// when it is not cached. Best effort: a failed or slow download plays nothing.
	pub fn play(
		&mut self,
		runtime: &tokio::runtime::Runtime,
		effects: &Effects,
		sound: Id,
		source: Id,
		volume: f32,
		gain: f32,
	) {
		if sound.0 == 0 || !(volume * gain).is_finite() || volume * gain <= 0.0 {
			return;
		}
		let Ok(mut shared) = self.shared.lock() else {
			return;
		};
		if let Some(index) = shared.clips.iter().position(|(id, _)| *id == sound) {
			let entry = shared.clips.remove(index);
			effects.play(entry.1.clone(), source.0, volume, gain);
			shared.clips.push(entry);
			return;
		}
		if shared.loading.contains(&sound) || shared.loading.len() >= MAX_LOADS {
			return;
		}
		if self.client.is_none() {
			self.client = reqwest::Client::builder()
				.https_only(true)
				.no_proxy()
				.redirect(reqwest::redirect::Policy::none())
				.timeout(Duration::from_secs(10))
				.connect_timeout(Duration::from_secs(5))
				.pool_max_idle_per_host(1)
				.build()
				.ok();
		}
		let Some(client) = self.client.clone() else {
			return;
		};
		shared.loading.push(sound);
		drop(shared);
		let shared = self.shared.clone();
		let effects = effects.clone();
		let requested = Instant::now();
		runtime.spawn(async move {
			let clip = load(&client, sound).await;
			let Ok(mut shared) = shared.lock() else {
				return;
			};
			shared.loading.retain(|id| *id != sound);
			if let Ok(clip) = clip {
				shared.insert(sound, clip.clone());
				if requested.elapsed() <= MAX_LATENESS {
					effects.play(clip, source.0, volume, gain);
				}
			}
		});
	}
}
async fn load(client: &reqwest::Client, sound: Id) -> Result<Clip, &'static str> {
	// The path holds only the numeric sound ID; no credential or account data is sent.
	let mut response = client
		.get(format!(
			"https://cdn.discordapp.com/soundboard-sounds/{sound}"
		))
		.send()
		.await
		.map_err(|_| "Sound download failed")?;
	if response.status() != reqwest::StatusCode::OK
		|| response
			.content_length()
			.is_some_and(|size| size > MAX_SOUND_BYTES as u64)
	{
		return Err("Sound unavailable");
	}
	let mut bytes = Vec::new();
	while let Some(chunk) = response
		.chunk()
		.await
		.map_err(|_| "Sound download interrupted")?
	{
		if chunk.len() > MAX_SOUND_BYTES - bytes.len() {
			return Err("Sound is too large");
		}
		bytes.extend_from_slice(&chunk);
	}
	// Decoding is CPU work: keep it off the async workers and the UI thread.
	tokio::task::spawn_blocking(move || crate::audio::decode_clip(bytes, MAX_CLIP_SAMPLES))
		.await
		.map_err(|_| "Sound decoding stopped")?
		.map(Arc::from)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn cache_keeps_recent_clips_within_item_and_sample_limits() {
		let mut shared = Shared::default();
		for id in 1..=MAX_CACHED as u64 + 2 {
			shared.insert(Id(id), vec![0.0; 8].into());
		}
		assert_eq!(shared.clips.len(), MAX_CACHED);
		assert_eq!(shared.clips[0].0, Id(3));
		// Replacing an entry never duplicates it; full-length clips evict by samples.
		shared.insert(Id(3), vec![0.0; 16].into());
		assert_eq!(shared.clips.len(), MAX_CACHED);
		assert_eq!(shared.clips.last().unwrap().1.len(), 16);
		for id in 100..112 {
			shared.insert(Id(id), vec![0.0; MAX_CLIP_SAMPLES].into());
		}
		assert_eq!(shared.clips.len(), 10);
		assert!(shared.clips.iter().all(|(id, _)| id.0 >= 102));
	}
	#[test]
	fn clips_decode_to_bounded_48khz_mono_and_reject_damaged_audio() {
		for bytes in [
			include_bytes!("../tests/fixtures/audio-tone.mp3").to_vec(),
			include_bytes!("../tests/fixtures/voice-message.ogg").to_vec(),
		] {
			let clip = crate::audio::decode_clip(bytes.clone(), MAX_CLIP_SAMPLES).unwrap();
			assert!(!clip.is_empty() && clip.len() <= MAX_CLIP_SAMPLES);
			assert!(clip.iter().all(|sample| sample.abs() <= 1.0));
			assert!(clip.iter().any(|sample| sample.abs() > 0.001));
			// A longer sound is truncated to the budget, never rejected or over-allocated.
			let short = crate::audio::decode_clip(bytes, 4800).unwrap();
			assert_eq!(short.len(), 4800);
			assert_eq!(short[..100], clip[..100]);
		}
		assert!(crate::audio::decode_clip(Vec::new(), MAX_CLIP_SAMPLES).is_err());

		// A file longer than the streaming decoder accepts still yields its start: 601 s of
		// 8 kHz mono exceeds the ten-minute limit that rejects such files elsewhere.
		let data = vec![128u8; 8000 * 601];
		let mut wav = Vec::with_capacity(44 + data.len());
		wav.extend_from_slice(b"RIFF");
		wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
		wav.extend_from_slice(b"WAVEfmt ");
		for field in [16u32, 1 | 1 << 16, 8000, 8000, 1 | 8 << 16] {
			wav.extend_from_slice(&field.to_le_bytes());
		}
		wav.extend_from_slice(b"data");
		wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
		wav.extend_from_slice(&data);
		let clip = crate::audio::decode_clip(wav, 48_000).unwrap();
		assert_eq!(clip.len(), 48_000);
		assert!(crate::audio::decode_clip(vec![0x42; 4096], MAX_CLIP_SAMPLES).is_err());
	}
}
