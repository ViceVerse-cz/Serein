//! Device-free suppression workload. Synthetic PCM only; never opens a microphone.
#![allow(dead_code)]
#[path = "../src/audio/echo.rs"]
mod echo;
use model::voice_settings::{NoiseSuppression, Processing};
use std::{hint::black_box, time::Instant};
fn signal(tick: usize) -> [f32; 960] {
	let mut seed = 17_u32.wrapping_add(tick as u32);
	std::array::from_fn(|i| {
		seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
		let t = (tick * 960 + i) as f32 / 48000.0;
		let voiced: f32 = (1..=24)
			.map(|h| {
				let f = h as f32 * 140.0;
				(0.035 * (-((f - 700.0) / 180.0).powi(2)).exp()
					+ 0.02 * (-((f - 1200.0) / 250.0).powi(2)).exp())
					* (t * f * std::f32::consts::TAU).sin()
			})
			.sum();
		voiced + seed as i32 as f32 / i32::MAX as f32 * 0.025
	})
}
fn main() {
	let frames: Vec<_> = (0..300).map(signal).collect();
	for mode in [
		NoiseSuppression::Off,
		NoiseSuppression::RnNoise,
		NoiseSuppression::WebRtc,
		NoiseSuppression::DeepFilterNet,
		NoiseSuppression::Auto,
	] {
		let init = Instant::now();
		let mut dsp = echo::Echo::new();
		let settings = Processing {
			suppression: mode,
			suppression_level: if matches!(
				mode,
				NoiseSuppression::Auto | NoiseSuppression::DeepFilterNet
			) {
				2
			} else {
				0
			},
			..Processing::studio()
		};
		dsp.configure(settings).unwrap();
		while dsp.suppression_status() == echo::Status::Loading {
			assert!(init.elapsed().as_secs() < 30, "model preparation timed out");
			std::thread::sleep(std::time::Duration::from_millis(5));
			dsp.configure(settings).unwrap();
		}
		if mode == NoiseSuppression::DeepFilterNet {
			assert_eq!(
				dsp.suppression_status(),
				echo::Status::DeepFilter,
				"DeepFilterNet must actually run"
			);
		}
		let init_us = init.elapsed().as_micros();
		println!("{mode:?} selected={}", dsp.suppression_status().label());
		for run in 0..6 {
			let mut times = Vec::with_capacity(frames.len());
			for input in &frames {
				let mut frame = *input;
				let now = Instant::now();
				dsp.capture(black_box(&mut frame), false).unwrap();
				times.push(now.elapsed().as_nanos());
				black_box(frame);
			}
			let total: u128 = times.iter().sum();
			times.sort_unstable();
			println!(
				"{mode:?} run={run} init_us={init_us} total_us={} p50_us={} p95_us={} max_us={} frames=300 audio_ms=6000",
				total / 1000,
				times[150] / 1000,
				times[285] / 1000,
				times[299] / 1000
			);
		}
	}
}
