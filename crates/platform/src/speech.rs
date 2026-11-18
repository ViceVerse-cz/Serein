//! Local LavaSRv2 inference. Called only by the disposable speech worker, never a device callback.
use ort::{session::Session, value::Tensor};
use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use sha2::{Digest, Sha256};
use std::{
	fs::File,
	io::Read,
	path::{Path, PathBuf},
	sync::Arc,
};

pub const SAMPLE_RATE: usize = 48_000;
pub const WINDOW: usize = 47_616; // 15,872 samples at 16 kHz, exactly 63 denoiser frames.
pub const ENHANCER_WINDOW: usize = 16_384;
// Run inference every 80 ms rather than every 40 ms. Each invocation still
// analyzes the full model window; this gives real speech processing headroom.
pub const HOP: usize = 3_840;
pub const LOOKAHEAD: usize = 1_920;
pub const OVERLAP: usize = 960;
pub const MODELS: [(&str, u64, &str); 3] = [
	(
		"backbone.onnx",
		51_761_958,
		"0bc915649f765d40ca52bd3a2272a28e8f7636b09c2ee3148662a69041f28cd9",
	),
	(
		"spec_head.onnx",
		4_208_522,
		"a54a5d5d652aceb76b2f48cf9f0ab3f4def0d44427dc7af641fd24284342a48a",
	),
	(
		"denoiser_core.onnx",
		1_173_265,
		"515a1be1eea4b1108cc6a758198d726fc506e68ec53d7e522bc2618ff2602175",
	),
];
type Result<T> = std::result::Result<T, &'static str>;

pub fn library_name() -> &'static str {
	if cfg!(target_os = "windows") {
		"onnxruntime.dll"
	} else if cfg!(target_os = "macos") {
		"libonnxruntime.1.23.2.dylib"
	} else {
		"libonnxruntime.so.1.23.2"
	}
}

pub struct Assets {
	pub directory: PathBuf,
	pub library: PathBuf,
}
impl Assets {
	pub fn in_directory(directory: PathBuf) -> Self {
		let library = directory.join(library_name());
		Self { directory, library }
	}
	pub fn packaged() -> Result<Self> {
		#[cfg(debug_assertions)]
		if let Some(directory) = std::env::var_os("SEREIN_LAVASR_DIR") {
			return Ok(Self::in_directory(directory.into()));
		}
		let exe = std::env::current_exe().map_err(|_| "Speech worker location is unavailable")?;
		Self::packaged_for_exe(&exe)
	}
	fn packaged_for_exe(exe: &Path) -> Result<Self> {
		// macOS can report the launcher symlink instead of the executable inside the bundle.
		let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_owned());
		let parent = exe
			.parent()
			.ok_or("Speech worker location is unavailable")?;
		let direct = Self::in_directory(parent.join("voice-enhancement"));
		if direct.directory.is_dir() {
			return Ok(direct);
		}
		if cfg!(target_os = "macos") {
			Ok(Self {
				directory: parent.join("../Resources/voice-enhancement"),
				library: parent.join("../Frameworks").join(library_name()),
			})
		} else {
			Ok(Self::in_directory(
				parent.join("../lib/serein/voice-enhancement"),
			))
		}
	}
	pub fn verify(&self) -> Result<()> {
		for (name, size, expected) in MODELS {
			let mut file = File::open(self.directory.join(name))
				.map_err(|_| "LavaSRv2 model is unavailable")?;
			if file
				.metadata()
				.map_err(|_| "LavaSRv2 model is unavailable")?
				.len() != size
			{
				return Err("LavaSRv2 model has the wrong size");
			}
			let mut hash = Sha256::new();
			let mut buffer = [0u8; 65536];
			let mut count = 0u64;
			loop {
				let read = file
					.read(&mut buffer)
					.map_err(|_| "LavaSRv2 model could not be read")?;
				if read == 0 {
					break;
				}
				count += read as u64;
				if count > size {
					return Err("LavaSRv2 model grew during loading");
				}
				hash.update(&buffer[..read]);
			}
			if count != size || format!("{:x}", hash.finalize()) != expected {
				return Err("LavaSRv2 model verification failed");
			}
		}
		if !self.library.is_file() {
			return Err("LavaSRv2 runtime is unavailable");
		}
		Ok(())
	}
}

struct Transform {
	n: usize,
	window: Vec<f32>,
	forward: Arc<dyn Fft<f32>>,
	inverse: Arc<dyn Fft<f32>>,
}
impl Transform {
	fn new(n: usize) -> Self {
		let mut planner = FftPlanner::new();
		Self {
			n,
			window: (0..n)
				.map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
				.collect(),
			forward: planner.plan_fft_forward(n),
			inverse: planner.plan_fft_inverse(n),
		}
	}
	fn stft(&self, wave: &[f32], hop: usize, pad: usize) -> Vec<Complex32> {
		let frames = (wave.len() + 2 * pad - self.n) / hop + 1;
		let mut result = Vec::with_capacity(frames * (self.n / 2 + 1));
		let mut buffer = vec![Complex32::ZERO; self.n];
		let mut scratch = vec![Complex32::ZERO; self.forward.get_inplace_scratch_len()];
		for frame in 0..frames {
			for (i, value) in buffer.iter_mut().enumerate() {
				let index = frame as isize * hop as isize + i as isize - pad as isize;
				let index = if index < 0 {
					-index
				} else if index >= wave.len() as isize {
					2 * wave.len() as isize - 2 - index
				} else {
					index
				} as usize;
				*value = Complex32::new(wave[index] * self.window[i], 0.0);
			}
			self.forward.process_with_scratch(&mut buffer, &mut scratch);
			result.extend_from_slice(&buffer[..=self.n / 2]);
		}
		result
	}
	fn istft(&self, spectrum: &[Complex32], hop: usize, pad: usize, length: usize) -> Vec<f32> {
		let bins = self.n / 2 + 1;
		let frames = spectrum.len() / bins;
		let total = (frames - 1) * hop + self.n;
		let mut result = vec![0.0; total];
		let mut weights = vec![0.0; total];
		let mut buffer = vec![Complex32::ZERO; self.n];
		let mut scratch = vec![Complex32::ZERO; self.inverse.get_inplace_scratch_len()];
		for frame in 0..frames {
			buffer[..bins].copy_from_slice(&spectrum[frame * bins..(frame + 1) * bins]);
			buffer[0].im = 0.0;
			buffer[self.n / 2].im = 0.0;
			for i in 1..self.n / 2 {
				buffer[self.n - i] = buffer[i].conj();
			}
			self.inverse.process_with_scratch(&mut buffer, &mut scratch);
			for (i, value) in buffer.iter().enumerate() {
				let offset = frame * hop + i;
				result[offset] += value.re / self.n as f32 * self.window[i];
				weights[offset] += self.window[i] * self.window[i];
			}
		}
		result[pad..pad + length]
			.iter()
			.zip(&weights[pad..pad + length])
			.map(|(value, weight)| value / weight.max(1e-8))
			.collect()
	}
}

// Torchaudio's default Hann-windowed sinc kernel, reduced to the rational rate ratio.
fn resample(wave: &[f32], original: usize, new: usize) -> Vec<f32> {
	let base = original.min(new) as f64 * 0.99;
	let width = (6.0 * original as f64 / base).ceil() as isize;
	let kernels: Vec<Vec<f32>> = (0..new)
		.map(|phase| {
			(-width..width + original as isize)
				.map(|index| {
					let t = (index as f64 / original as f64 - phase as f64 / new as f64) * base;
					if t.abs() >= 6.0 {
						return 0.0;
					}
					let x = t * std::f64::consts::PI;
					let sinc = if x.abs() < 1e-12 { 1.0 } else { x.sin() / x };
					(sinc * (x / 12.0).cos().powi(2) * base / original as f64) as f32
				})
				.collect()
		})
		.collect();
	(0..(wave.len() * new).div_ceil(original))
		.map(|index| {
			kernels[index % new]
				.iter()
				.enumerate()
				.map(|(k, coefficient)| {
					let source = (index / new * original) as isize + k as isize - width;
					if source >= 0 && (source as usize) < wave.len() {
						wave[source as usize] * coefficient
					} else {
						0.0
					}
				})
				.sum()
		})
		.collect()
}

fn mel_filters() -> Vec<Vec<(usize, f32)>> {
	fn to_mel(hz: f64) -> f64 {
		if hz < 1000.0 {
			hz / (200.0 / 3.0)
		} else {
			15.0 + (hz / 1000.0).ln() / (6.4f64.ln() / 27.0)
		}
	}
	fn to_hz(mel: f64) -> f64 {
		if mel < 15.0 {
			mel * (200.0 / 3.0)
		} else {
			1000.0 * ((mel - 15.0) * (6.4f64.ln() / 27.0)).exp()
		}
	}
	let points: Vec<_> = (0..82)
		.map(|i| to_hz(to_mel(8000.0) * i as f64 / 81.0))
		.collect();
	(0..80)
		.map(|mel| {
			(0..1025)
				.filter_map(|bin| {
					// Preserve the published v2 checkpoint's 44.1-kHz mel configuration even though
					// its waveform wrapper and output run at 48 kHz.
					let frequency = bin as f64 * 44100.0 / 2048.0;
					let low = (frequency - points[mel]) / (points[mel + 1] - points[mel]);
					let high = (points[mel + 2] - frequency) / (points[mel + 2] - points[mel + 1]);
					let weight = low.min(high).max(0.0) * 2.0 / (points[mel + 2] - points[mel]);
					(weight > 0.0).then_some((bin, weight as f32))
				})
				.collect()
		})
		.collect()
}

/// Three fixed, hash-pinned v2 graphs. CPU sessions use at most two inference threads.
pub struct Enhancer {
	denoise: Session,
	backbone: Session,
	head: Session,
	small: Transform,
	large: Transform,
	merge: Transform,
	mel: Vec<Vec<(usize, f32)>>,
	_environment: ort::environment::Environment,
}
impl Enhancer {
	pub fn new(assets: &Assets) -> Result<Self> {
		assets.verify()?;
		let environment = ort::init_from(&assets.library)
			.map_err(|_| "LavaSRv2 runtime could not load")?
			.with_name("serein-speech")
			.with_telemetry(false)
			.build()
			.map_err(|_| "LavaSRv2 environment could not initialize")?;
		fn session(path: &Path, environment: &ort::environment::Environment) -> Result<Session> {
			Session::builder(environment)
				.map_err(|_| "LavaSRv2 runtime could not initialize")?
				.with_intra_threads(2)
				.map_err(|_| "LavaSRv2 thread limits failed")?
				.with_inter_threads(1)
				.map_err(|_| "LavaSRv2 thread limits failed")?
				.with_intra_op_spinning(false)
				.map_err(|_| "LavaSRv2 thread limits failed")?
				.with_inter_op_spinning(false)
				.map_err(|_| "LavaSRv2 thread limits failed")?
				.commit_from_file(path)
				.map_err(|_| "LavaSRv2 model could not load")
		}
		Ok(Self {
			denoise: session(&assets.directory.join("denoiser_core.onnx"), &environment)?,
			backbone: session(&assets.directory.join("backbone.onnx"), &environment)?,
			head: session(&assets.directory.join("spec_head.onnx"), &environment)?,
			small: Transform::new(512),
			large: Transform::new(2048),
			merge: Transform::new(ENHANCER_WINDOW),
			mel: mel_filters(),
			_environment: environment,
		})
	}
	pub fn enhance(&mut self, pcm: &[f32]) -> Result<Vec<f32>> {
		if pcm.len() != WINDOW || pcm.iter().any(|x| !x.is_finite() || x.abs() > 1.0) {
			return Err("Invalid speech input window");
		}
		let wave = resample(pcm, 3, 1);
		let spectrum = self.small.stft(&wave, 256, 256);
		let mut input = Vec::with_capacity(2 * spectrum.len());
		input.extend(spectrum.iter().map(|value| value.re));
		input.extend(spectrum.iter().map(|value| value.im));
		let input = Tensor::from_array(([1usize, 2, 63, 257], input))
			.map_err(|_| "LavaSRv2 input failed")?;
		let outputs = self
			.denoise
			.run(ort::inputs!["spec_ri" => input])
			.map_err(|_| "LavaSRv2 denoising failed")?;
		let (shape, data) = outputs[0]
			.try_extract_tensor::<f32>()
			.map_err(|_| "LavaSRv2 denoiser output failed")?;
		if shape[..] != [1, 2, 63, 257]
			|| data.len() != 2 * spectrum.len()
			|| data.iter().any(|x| !x.is_finite())
		{
			return Err("Invalid LavaSRv2 denoiser output");
		}
		let clean: Vec<_> = (0..spectrum.len())
			.map(|i| Complex32::new(data[i], data[spectrum.len() + i]))
			.collect();
		drop(outputs);
		let clean = self.small.istft(&clean, 256, 256, wave.len());
		let clean = resample(&clean, 1, 3);
		let original = &clean[clean.len() - ENHANCER_WINDOW..];
		let spectrum = self.large.stft(original, 512, 768);
		let frames = spectrum.len() / 1025;
		let mel: Vec<f32> = self
			.mel
			.iter()
			.flat_map(|filter| {
				(0..frames).map(|frame| {
					filter
						.iter()
						.map(|(bin, weight)| spectrum[frame * 1025 + bin].norm() * weight)
						.sum::<f32>()
						.max(1e-5)
						.ln()
				})
			})
			.collect();
		let input = Tensor::from_array(([1usize, 80, frames], mel))
			.map_err(|_| "LavaSRv2 mel input failed")?;
		let hidden = self
			.backbone
			.run(ort::inputs!["mel" => input])
			.map_err(|_| "LavaSRv2 enhancement failed")?;
		let (shape, data) = hidden[0]
			.try_extract_tensor::<f32>()
			.map_err(|_| "Invalid LavaSRv2 hidden output")?;
		if shape[..] != [1, frames as i64, 512]
			|| data.len() != frames * 512
			|| data.iter().any(|x| !x.is_finite())
		{
			return Err("Invalid LavaSRv2 hidden dimensions");
		}
		let input = Tensor::from_array(([1usize, frames, 512], data.to_vec()))
			.map_err(|_| "LavaSRv2 synthesis input failed")?;
		drop(hidden);
		let output = self
			.head
			.run(ort::inputs!["hidden" => input])
			.map_err(|_| "LavaSRv2 synthesis failed")?;
		let (real_shape, real) = output[0]
			.try_extract_tensor::<f32>()
			.map_err(|_| "Invalid LavaSRv2 spectrum")?;
		let (imag_shape, imag) = output[1]
			.try_extract_tensor::<f32>()
			.map_err(|_| "Invalid LavaSRv2 spectrum")?;
		if real_shape[..] != [1, 1025, frames as i64]
			|| real_shape != imag_shape
			|| real.len() != 1025 * frames
			|| imag.len() != real.len()
			|| real.iter().chain(imag).any(|x| !x.is_finite())
		{
			return Err("Invalid LavaSRv2 spectrum dimensions");
		}
		let mut spectrum = Vec::with_capacity(real.len());
		for frame in 0..frames {
			for bin in 0..1025 {
				spectrum.push(Complex32::new(
					real[bin * frames + frame],
					imag[bin * frames + frame],
				));
			}
		}
		drop(output);
		let restored = self.large.istft(&spectrum, 512, 768, ENHANCER_WINDOW);
		let mut a: Vec<_> = restored.iter().map(|x| Complex32::new(*x, 0.0)).collect();
		let mut b: Vec<_> = original.iter().map(|x| Complex32::new(*x, 0.0)).collect();
		self.merge.forward.process(&mut a);
		self.merge.forward.process(&mut b);
		let bins = ENHANCER_WINDOW / 2 + 1;
		let cutoff = bins / 3;
		let start = cutoff - 512;
		let end = cutoff + 512;
		for i in 0..bins {
			let t = ((i.saturating_sub(start)) as f32 / 1023.0).clamp(0.0, 1.0);
			let mask = if i < start {
				0.0
			} else if i >= end {
				1.0
			} else {
				3.0 * t * t - 2.0 * t * t * t
			};
			let original = b[i];
			b[i] += (a[i] - original) * mask;
		}
		for i in 1..ENHANCER_WINDOW / 2 {
			b[ENHANCER_WINDOW - i] = b[i].conj();
		}
		self.merge.inverse.process(&mut b);
		let result: Vec<_> = b
			.iter()
			.map(|x| (x.re / ENHANCER_WINDOW as f32).clamp(-1.0, 1.0))
			.collect();
		if result.iter().any(|x| !x.is_finite()) {
			return Err("Non-finite LavaSRv2 audio");
		}
		Ok(result)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[cfg(unix)]
	#[test]
	fn speech_assets_follow_the_executable_behind_a_launcher_symlink() {
		let root = tempfile::tempdir().unwrap();
		let launcher = root.path().join("bin/serein");
		let executable = root
			.path()
			.join("Applications/Serein.app/Contents/MacOS/serein");
		std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
		std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
		std::fs::write(&executable, b"fixture").unwrap();
		std::os::unix::fs::symlink(&executable, &launcher).unwrap();
		let direct = Assets::packaged_for_exe(&executable).unwrap();
		let linked = Assets::packaged_for_exe(&launcher).unwrap();
		assert_eq!(linked.directory, direct.directory);
		assert_eq!(linked.library, direct.library);
		let parent = executable.canonicalize().unwrap();
		let parent = parent.parent().unwrap();
		if cfg!(target_os = "macos") {
			assert_eq!(
				linked.directory,
				parent.join("../Resources/voice-enhancement")
			);
			assert_eq!(
				linked.library,
				parent.join("../Frameworks").join(library_name())
			);
		} else {
			assert_eq!(
				linked.directory,
				parent.join("../lib/serein/voice-enhancement")
			);
		}
	}

	#[test]
	fn speech_transforms_preserve_samples_and_suppress_aliasing() {
		for (n, length, hop, pad) in [
			(512, WINDOW / 3, 256, 256),
			(2048, ENHANCER_WINDOW, 512, 768),
		] {
			let fft = Transform::new(n);
			let input: Vec<_> = (0..length).map(|i| (i as f32 * 0.23).sin() * 0.1).collect();
			let result = fft.istft(&fft.stft(&input, hop, pad), hop, pad, length);
			assert_eq!(result.len(), input.len());
			assert!(input.iter().zip(result).all(|(a, b)| (a - b).abs() < 2e-5));
		}
		let tone = |hz: f32| {
			(0..WINDOW)
				.map(|i| {
					(2.0 * std::f32::consts::PI * hz * i as f32 / SAMPLE_RATE as f32).sin() * 0.1
				})
				.collect::<Vec<_>>()
		};
		let input = tone(1000.0);
		let result = resample(&resample(&input, 3, 1), 1, 3);
		assert_eq!(result.len(), input.len());
		assert!(
			input[100..WINDOW - 100]
				.iter()
				.zip(&result[100..WINDOW - 100])
				.all(|(a, b)| (a - b).abs() < 0.001)
		);
		let high = resample(&tone(20_000.0), 3, 1);
		assert!(
			(high[100..high.len() - 100]
				.iter()
				.map(|x| x * x)
				.sum::<f32>()
				/ (high.len() - 200) as f32)
				.sqrt() < 0.001
		);
	}
	#[test]
	#[ignore = "requires the separately staged hash-pinned speech models and CPU runtime"]
	fn actual_v2_graphs_denoise_and_restore_a_bounded_window() {
		let assets = Assets::in_directory(
			std::env::var_os("SEREIN_LAVASR_DIR")
				.expect("staged model directory")
				.into(),
		);
		let mut model = Enhancer::new(&assets).unwrap();
		let input: Vec<_> = (0..WINDOW)
			.map(|i| (i as f32 * 0.13).sin() * 0.05)
			.collect();
		let output = model.enhance(&input).unwrap();
		assert_eq!(output.len(), ENHANCER_WINDOW);
		assert!(output.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
		// Independent Torch/torchaudio reference: published v2 mel configuration,
		// periodic Hann ULUNAS/Vocos transforms and FastLRMerge (8 kHz, 1024 bins).
		// Same pinned ONNX graphs; this checks DSP parity, not subjective speech quality.
		for (index, expected) in [
			(0, -3.899462e-3),
			(1, 2.895139e-5),
			(100, -1.606909e-4),
			(511, -1.999233e-4),
			(1024, -1.877677e-4),
			(2048, -2.512133e-4),
			(4096, 1.293711e-5),
			(8192, 1.380311e-4),
			(10000, 1.555865e-4),
			(12000, -2.132_93e-4),
			(14000, 5.668185e-5),
			(15000, 9.165028e-5),
			(16000, -4.619121e-4),
			(16383, -1.182138e-4),
		] {
			// Keep the streaming crop strict. FP32 FFT/CPU kernels vary slightly at
			// synthesized window edges, which the helper's lookahead/overlap discard.
			let tolerance = if (512..ENHANCER_WINDOW - 512).contains(&index) {
				1e-6
			} else {
				1e-4
			};
			assert!(
				(output[index] - expected).abs() < tolerance,
				"reference sample {index}: {}",
				output[index]
			);
		}
		let rms = (output.iter().map(|x| x * x).sum::<f32>() / output.len() as f32).sqrt();
		assert!((rms - 5.227617e-4).abs() < 1e-5);
		assert!(model.enhance(&input[..WINDOW - 1]).is_err());
	}
}
