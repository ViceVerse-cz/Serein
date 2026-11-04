//! Viewer decode bench: replays a lab recording (from `screen_bench --out`) at its frame
//! rate through the platform live decoder a watched stream uses, and through OpenH264, then
//! reports decode latency, delivered pictures and per-thread CPU. Offline: no network,
//! Discord connection or display.
//!
//! ```sh
//! cargo run --release -p discord-voice -p platform --features winit/wayland \
//!   --example stream_view_bench -- RECORDING.au --fps 30 [--repeat 2]
//! ```
#![allow(clippy::print_stdout)]

use std::{
	collections::HashMap,
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};

/// Access units of one recording: little-endian u32 length, then Annex B bytes.
fn read(path: &str) -> Vec<Vec<u8>> {
	let bytes = std::fs::read(path).expect("recording");
	let mut units = Vec::new();
	let mut rest = bytes.as_slice();
	while rest.len() >= 4 {
		let length = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
		assert!(
			length <= 4 << 20 && rest.len() >= 4 + length,
			"corrupt recording"
		);
		units.push(rest[4..4 + length].to_vec());
		rest = &rest[4 + length..];
	}
	units
}

fn thread_ticks() -> HashMap<String, u64> {
	let mut threads = HashMap::new();
	for task in std::fs::read_dir("/proc/self/task")
		.into_iter()
		.flatten()
		.flatten()
	{
		let Ok(stat) = std::fs::read_to_string(task.path().join("stat")) else {
			continue;
		};
		let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
			continue;
		};
		let fields: Vec<&str> = stat[close + 2..].split(' ').collect();
		let ticks = fields[11].parse::<u64>().unwrap_or(0) + fields[12].parse::<u64>().unwrap_or(0);
		let role = stat[open + 1..close]
			.trim_end_matches(|c: char| c.is_ascii_digit() || c == ':')
			.to_string();
		*threads.entry(role).or_default() += ticks;
	}
	threads
}

fn report(
	name: &str,
	started: Instant,
	before: HashMap<String, u64>,
	latencies: &mut [f64],
	sent: usize,
) {
	let elapsed = started.elapsed().as_secs_f64();
	let after = thread_ticks();
	let mut rows: Vec<(String, f64)> = after
		.iter()
		.map(|(role, ticks)| {
			(
				role.clone(),
				ticks.saturating_sub(*before.get(role).unwrap_or(&0)) as f64 / elapsed,
			)
		})
		.filter(|(_, percent)| *percent >= 0.5)
		.collect();
	rows.sort_by(|a, b| b.1.total_cmp(&a.1));
	latencies.sort_by(f64::total_cmp);
	let at = |fraction: f64| {
		latencies
			.get(((latencies.len().max(1) - 1) as f64 * fraction) as usize)
			.copied()
			.unwrap_or(0.)
	};
	println!(
		"{name}: {}/{sent} pictures, latency ms p50 {:.1} p95 {:.1} max {:.1}, cpu {:.1}%",
		latencies.len(),
		at(0.5),
		at(0.95),
		latencies.last().copied().unwrap_or(0.),
		rows.iter().map(|row| row.1).sum::<f64>()
	);
	for (role, percent) in rows {
		println!("  {percent:>6.1}%  {role}");
	}
}

fn main() {
	let mut args = std::env::args().skip(1);
	let path = args
		.next()
		.expect("usage: stream_view_bench RECORDING.au [--fps N] [--repeat N]");
	let (mut fps, mut repeat) = (30u32, 1usize);
	while let Some(arg) = args.next() {
		match arg.as_str() {
			"--fps" => fps = args.next().unwrap().parse().unwrap(),
			"--repeat" => repeat = args.next().unwrap().parse().unwrap(),
			other => panic!("unknown option {other}"),
		}
	}
	let units = read(&path);
	let interval = Duration::from_secs_f64(1. / f64::from(fps));
	println!(
		"{} access units from {path} at {fps} fps, {repeat}x",
		units.len()
	);

	// Platform decoder: GStreamer decodebin on Linux, as a watched stream uses first.
	let pushed: Arc<Mutex<HashMap<u64, Instant>>> = Arc::default();
	let latencies: Arc<Mutex<Vec<f64>>> = Arc::default();
	let sink: platform::video::TimedLiveSink = {
		let (pushed, latencies) = (pushed.clone(), latencies.clone());
		Box::new(move |_frame, tag| {
			if let Some(at) = pushed.lock().unwrap().remove(&tag) {
				latencies
					.lock()
					.unwrap()
					.push(at.elapsed().as_secs_f64() * 1000.);
			}
		})
	};
	match platform::video::live::H264Decoder::new_timed(sink) {
		Ok(mut decoder) => {
			let (before, started) = (thread_ticks(), Instant::now());
			let mut next = Instant::now();
			let mut sent = 0;
			for unit in units.iter().cycle().take(units.len() * repeat) {
				std::thread::sleep(next.saturating_duration_since(Instant::now()));
				next += interval;
				sent += 1;
				// Tags travel as millisecond timestamps.
				let tag = sent as u64 * 1000 / u64::from(fps);
				pushed.lock().unwrap().insert(tag, Instant::now());
				if decoder.decode_timed(unit, tag).is_err() {
					println!("native decoder rejected unit {sent}");
				}
			}
			std::thread::sleep(Duration::from_millis(500));
			decoder.flush();
			report(
				"native",
				started,
				before,
				&mut latencies.lock().unwrap(),
				sent,
			);
		}
		Err(error) => println!("native: unavailable ({error})"),
	}

	let mut decoder = openh264::decoder::Decoder::new().unwrap();
	let (before, started) = (thread_ticks(), Instant::now());
	let mut next = Instant::now();
	let mut latencies = Vec::new();
	let mut sent = 0;
	for unit in units.iter().cycle().take(units.len() * repeat) {
		std::thread::sleep(next.saturating_duration_since(Instant::now()));
		next += interval;
		sent += 1;
		let at = Instant::now();
		if let Ok(Some(_)) = decoder.decode(unit) {
			latencies.push(at.elapsed().as_secs_f64() * 1000.);
		}
	}
	report("openh264", started, before, &mut latencies, sent);
}
