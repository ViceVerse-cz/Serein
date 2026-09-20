//! Offline debug check: synthetic delayed speaker echo, followed by near-end speech.
//! Does not open audio devices or connect to Discord.
#![allow(dead_code)]

#[path = "../src/activity.rs"]
mod activity;
#[path = "../src/capture.rs"]
mod capture;
#[path = "../src/audio/echo.rs"]
mod echo;
type Frame = [f32; 960];

fn main() {
	assert_eq!(activity::level_db(&[0.0; 960]), -100.0);
	assert_eq!(activity::level_db(&[1.0; 960]), 0.0);
	assert!((activity::level_db(&[0.1; 960]) + 20.0).abs() < 0.01);
	assert_eq!(activity::level_db(&[f32::NAN; 960]), -100.0);

	assert_eq!(activity::hold(0.0, 0), 0);
	assert_eq!(activity::hold(960.0 * 0.001_f32.powi(2), 0), 0);
	let mut hold = activity::hold(960.0 * 0.02_f32.powi(2), 0);
	assert_eq!(hold, 10);
	for remaining in (0..10).rev() {
		hold = activity::hold(0.0, hold);
		assert_eq!(hold, remaining);
	}
	assert_eq!(activity::hold(f32::NAN, 0), 0);

	// Hardware/worker batches alternate zero and two frames per packet tick.
	// The previous drain-to-latest sender lost half the speech in this case.
	let (send, receive) = std::sync::mpsc::sync_channel(8);
	let mut pacer = capture::CapturePacer::default();
	// Empty-room detection consumes PCM locally, leaving nothing for a later peer.
	send.try_send([0.2; 960]).unwrap();
	assert!(pacer.next(&receive, true, false).is_none());
	send.try_send([0.3; 960]).unwrap();
	let preview = pacer.preview(&receive).unwrap();
	assert_eq!(preview, [0.3; 960]);
	assert!(activity::hold(preview.iter().map(|s| s * s).sum(), 0) > 0);
	assert!(pacer.next(&receive, true, false).is_none());
	assert!(pacer.preview(&receive).is_none());
	for tick in 0..100 {
		if tick % 2 == 0 {
			send.try_send([tick as f32; 960]).unwrap();
			send.try_send([(tick + 1) as f32; 960]).unwrap();
		}
		let frame = pacer.next(&receive, true, false);
		if tick == 0 {
			assert!(frame.is_none());
		} else {
			assert_eq!(frame.unwrap(), [(tick - 1) as f32; 960]);
		}
	}
	assert_eq!(pacer.next(&receive, true, false).unwrap(), [99.0; 960]);
	for (enabled, stalled) in [(false, false), (true, true)] {
		send.try_send([0.5; 960]).unwrap();
		assert!(pacer.next(&receive, true, false).is_none());
		send.try_send([0.5; 960]).unwrap();
		assert!(pacer.next(&receive, enabled, stalled).is_none());
		assert!(pacer.next(&receive, true, false).is_none());
		assert!(pacer.next(&receive, true, false).is_none());
	}
	// Exercise the real AEC -> RNNoise chain with synthetic hiss and short click bursts.
	let mut filtered = echo::Echo::new();
	let mut unfiltered = echo::Echo::new();
	filtered
		.configure(model::voice_settings::VoiceProcessing::from_legacy(true).effective())
		.unwrap();
	let mut seed = 17_u32;
	let mut before = 0.0;
	let mut after = 0.0;
	for tick in 0..120 {
		let mut raw = std::array::from_fn(|i| {
			seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
			let amplitude = if tick % 15 == 0 && i < 240 {
				0.25
			} else {
				0.06
			};
			(seed as i32 as f32 / i32::MAX as f32) * amplitude
		});
		let mut clean = raw;
		filtered.render(&[0.0; 960]).unwrap();
		unfiltered.render(&[0.0; 960]).unwrap();
		filtered.capture(&mut clean, false).unwrap();
		unfiltered.capture(&mut raw, false).unwrap();
		assert!(clean.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
		if tick >= 60 {
			before += raw.iter().map(|s| s * s).sum::<f32>();
			after += clean.iter().map(|s| s * s).sum::<f32>();
		}
	}
	assert!(
		after < before * 0.75,
		"noise suppression should reduce synthetic hiss/click energy: {before} -> {after}"
	);
	// A synthetic voiced vowel must survive; noise-only attenuation is insufficient.
	let mut voiced_before = 0.0;
	let mut voiced_after = 0.0;
	for tick in 0..100 {
		let mut vowel = std::array::from_fn(|i| {
			let t = (tick * 960 + i) as f32 / 48_000.0;
			(1..=24)
				.map(|h| {
					let frequency = h as f32 * 140.0;
					let weight = ((frequency - 700.0) / 180.0).powi(2);
					let weight2 = ((frequency - 1200.0) / 250.0).powi(2);
					(0.035 * (-weight).exp() + 0.02 * (-weight2).exp())
						* (t * frequency * std::f32::consts::TAU).sin()
				})
				.sum::<f32>()
		});
		let mut raw = vowel;
		filtered.render(&[0.0; 960]).unwrap();
		unfiltered.render(&[0.0; 960]).unwrap();
		filtered.capture(&mut vowel, false).unwrap();
		unfiltered.capture(&mut raw, false).unwrap();
		if tick >= 50 {
			voiced_before += raw.iter().map(|s| s * s).sum::<f32>();
			voiced_after += vowel.iter().map(|s| s * s).sum::<f32>();
		}
	}
	assert!(
		voiced_after > voiced_before * 0.1,
		"voiced signal must survive suppression: {voiced_before} -> {voiced_after}"
	);
	filtered
		.configure(model::voice_settings::VoiceProcessing::from_legacy(false).effective())
		.unwrap();
	let mut bypass = [0.2; 960];
	let mut reference = bypass;
	filtered.render(&[0.0; 960]).unwrap();
	unfiltered.render(&[0.0; 960]).unwrap();
	filtered.capture(&mut bypass, false).unwrap();
	unfiltered.capture(&mut reference, false).unwrap();
	assert_eq!(
		bypass, reference,
		"disabling suppression must retain the original AEC state"
	);

	let mut echo = echo::Echo::new();
	let mut history = [[0.0_f32; 960]; 4];
	let mut random = 7_u32;
	let mut smooth = 0.0;
	let mut original_energy = 0.0;
	let mut residual_energy = 0.0;
	for tick in 0..500 {
		let mut render = [0.0; 960];
		for sample in &mut render {
			random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
			smooth = 0.7 * smooth + 0.3 * (random as i32 as f32 / i32::MAX as f32);
			*sample = smooth * 0.5;
		}
		// 60 ms acoustic/device delay with a second reflection 20 ms later.
		let mut capture =
			std::array::from_fn(|i| history[(tick + 1) % 4][i] * 0.6 + history[tick % 4][i] * 0.15);
		history[tick % 4] = render;
		echo.render(&render).unwrap();
		if tick >= 400 {
			original_energy += capture.iter().map(|s| s * s).sum::<f32>();
		}
		echo.capture(&mut capture, false).unwrap();
		assert!(capture.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
		if tick >= 400 {
			residual_energy += capture.iter().map(|s| s * s).sum::<f32>();
		}
	}
	assert!(
		residual_energy < original_energy * 0.1,
		"delayed echo should be reduced"
	);

	// A fresh device/call must also pass near-end speech with no speaker signal.
	echo = echo::Echo::new();
	let mut speech_energy = 0.0;
	for tick in 0..100 {
		let mut capture = std::array::from_fn(|i| ((tick * 960 + i) as f32 * 0.04).sin() * 0.2);
		echo.render(&[0.0; 960]).unwrap();
		echo.capture(&mut capture, false).unwrap();
		if tick >= 50 {
			speech_energy += capture.iter().map(|s| s * s).sum::<f32>();
		}
	}
	assert!(
		speech_energy > 100.0,
		"near-end speech should remain audible"
	);
	println!(
		"Offline audio check passed: noise suppression and live bypass, activity threshold and release, batched capture preserved, mute/stall flushed, echo reduced, near-end signal preserved."
	);
}
