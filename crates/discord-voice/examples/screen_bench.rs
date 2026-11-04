//! Local screen-share sender bench: runs the production capture/encode worker against the
//! desktop's ScreenCast portal, consumes encoded frames the way the stream transport does
//! and reports throughput, pacing and per-thread CPU. No Discord connection, microphone or
//! network is used; captured pictures stay in memory unless `--out` writes the H.264 stream
//! as a lab recording: each access unit as a little-endian u32 length and its Annex B bytes,
//! which `stream_view_bench` and the desktop demo replay read back.
//!
//! Intended for a disposable headless compositor (see `tools/screen-lab/`), where the portal
//! picks the only output without a dialog. On a real desktop the system picker opens.
//!
//! ```sh
//! cargo run --release -p discord-voice --example screen_bench -- --seconds 20 --size 1080 --fps 30
//! ```
#![allow(clippy::print_stdout)]

#[cfg(not(target_os = "linux"))]
fn main() {
	eprintln!("The screen bench needs Linux with a ScreenCast portal.");
}

#[cfg(target_os = "linux")]
fn main() {
	linux::main();
}

#[cfg(target_os = "linux")]
mod linux {
	use discord_voice::screen::{Settings, SourceId, Worker};
	use std::{
		collections::HashMap,
		io::Write,
		sync::atomic::Ordering,
		time::{Duration, Instant},
	};

	struct Args {
		seconds: u64,
		settings: Settings,
		out: Option<std::path::PathBuf>,
		decode: bool,
		preview: bool,
		/// Simulated transport feedback: a target bitrate applied after warmup.
		bitrate: Option<u32>,
		/// Forced keyframe requests per second, like viewer PLIs.
		pli: f64,
	}

	fn args() -> Args {
		let mut args = Args {
			seconds: 15,
			settings: Settings {
				source: SourceId::Portal,
				width: 1280,
				height: 720,
				fps: 30,
				cursor: true,
				audio: false,
			},
			out: None,
			decode: false,
			preview: true,
			bitrate: None,
			pli: 0.,
		};
		let mut list = std::env::args().skip(1);
		while let Some(arg) = list.next() {
			let mut value = || list.next().expect("missing option value");
			match arg.as_str() {
				"--seconds" => args.seconds = value().parse().expect("seconds"),
				"--fps" => args.settings.fps = value().parse().expect("fps"),
				"--size" => {
					(args.settings.width, args.settings.height) = match value().as_str() {
						"480" => (854, 480),
						"720" => (1280, 720),
						"1080" => (1920, 1080),
						_ => panic!("--size is 480, 720 or 1080"),
					}
				}
				"--out" => args.out = Some(value().into()),
				"--decode" => args.decode = true,
				"--no-preview" => args.preview = false,
				"--x11" => args.settings.source = SourceId::X11Desktop,
				"--no-cursor" => args.settings.cursor = false,
				"--bitrate" => args.bitrate = Some(value().parse().expect("bitrate")),
				"--pli" => args.pli = value().parse().expect("pli per second"),
				other => panic!("unknown option {other}"),
			}
		}
		assert!(args.settings.valid(), "unsupported size/fps");
		args
	}

	/// Clock ticks of CPU time per thread name, summed across same-named threads.
	fn thread_cpu() -> HashMap<String, u64> {
		let mut threads = HashMap::new();
		let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
			return threads;
		};
		for task in tasks.flatten() {
			let Ok(stat) = std::fs::read_to_string(task.path().join("stat")) else {
				continue;
			};
			let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
				continue;
			};
			let name = stat[open + 1..close].to_string();
			let fields: Vec<&str> = stat[close + 2..].split(' ').collect();
			let ticks = fields
				.get(11)
				.and_then(|v| v.parse::<u64>().ok())
				.unwrap_or(0)
				+ fields
					.get(12)
					.and_then(|v| v.parse::<u64>().ok())
					.unwrap_or(0);
			// Collapse numbered GStreamer/worker threads into one row per role.
			let role = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ':');
			*threads.entry(role.to_string()).or_default() += ticks;
		}
		threads
	}

	fn status_kib(field: &str) -> u64 {
		std::fs::read_to_string("/proc/self/status")
			.ok()
			.and_then(|status| {
				status.lines().find_map(|line| {
					line.strip_prefix(field)?
						.trim()
						.trim_end_matches("kB")
						.trim()
						.parse()
						.ok()
				})
			})
			.unwrap_or(0)
	}

	fn percentile(sorted: &[f64], fraction: f64) -> f64 {
		if sorted.is_empty() {
			return 0.;
		}
		sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
	}

	pub fn main() {
		let args = args();
		let settings = args.settings;
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.thread_name("bench-runtime")
			.enable_all()
			.build()
			.unwrap();
		let (worker, mut video) = Worker::start(settings, || {}).expect("start screen worker");
		// The transport sets this once DAVE is ready; the bench is always "secure".
		video.ready.store(true, Ordering::Release);
		video.keyframe.store(true, Ordering::Release);
		let mut out = args
			.out
			.as_ref()
			.map(|path| std::io::BufWriter::new(std::fs::File::create(path).expect("output file")));
		let (decode_send, decode_receive) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
		let decoder = args.decode.then(|| {
			std::thread::Builder::new()
				.name("bench-decode".into())
				.spawn(move || {
					let mut decoder = openh264::decoder::Decoder::new().unwrap();
					let (mut ok, mut failed) = (0u64, 0u64);
					while let Ok(frame) = decode_receive.recv() {
						match decoder.decode(&frame) {
							Ok(_) => ok += 1,
							Err(_) => failed += 1,
						}
					}
					(ok, failed)
				})
				.unwrap()
		});

		let started = Instant::now();
		let deadline = started + Duration::from_secs(args.seconds);
		let warmup = Duration::from_secs(2);
		let mut measuring: Option<(Instant, HashMap<String, u64>)> = None;
		let mut first_frame = None;
		let mut last_frame: Option<Instant> = None;
		let mut gaps = Vec::new();
		let (mut frames, mut bytes, mut keyframes, mut previews) = (0u64, 0u64, 0u64, 0u64);
		let (mut second_frames, mut second_bytes) = (0u64, 0u64);
		let mut second = Instant::now();
		let mut last_status = None;
		let mut next_pli = Instant::now();
		let mut failure = None;
		let mut largest = 0usize;
		println!(
			"bench: {}x{}@{} target {} kbps for {} s",
			settings.width,
			settings.height,
			settings.fps,
			settings.bit_rate() / 1000,
			args.seconds
		);
		while Instant::now() < deadline {
			if let Some(result) = worker.result() {
				failure = Some(result.err().unwrap_or("worker stopped"));
				break;
			}
			let status = worker.capture_status();
			if status != last_status {
				println!(
					"[{:>6.2}s] status: {}",
					started.elapsed().as_secs_f64(),
					status.unwrap_or("-")
				);
				last_status = status;
			}
			if args.preview && worker.take_preview().is_some() {
				previews += 1;
			}
			let frame = runtime.block_on(async {
				tokio::time::timeout(Duration::from_millis(20), video.frames.recv()).await
			});
			let now = Instant::now();
			if args.pli > 0. && first_frame.is_some() && now >= next_pli {
				video.keyframe.store(true, Ordering::Release);
				next_pli = now + Duration::from_secs_f64(1. / args.pli);
			}
			if measuring.is_none() && first_frame.is_some_and(|at: Instant| at.elapsed() >= warmup)
			{
				if let Some(target) = args.bitrate {
					video.bitrate.store(target, Ordering::Release);
				}
				measuring = Some((now, thread_cpu()));
				gaps.clear();
				(frames, bytes, keyframes, previews) = (0, 0, 0, 0);
			}
			let Ok(Some(frame)) = frame else {
				if let Ok(None) = frame {
					failure = Some("encoded stream closed");
					break;
				}
				continue;
			};
			if first_frame.is_none() {
				println!(
					"[{:>6.2}s] first encoded frame ({} bytes, keyframe={})",
					started.elapsed().as_secs_f64(),
					frame.data.len(),
					frame.keyframe
				);
				first_frame = Some(now);
			}
			if let Some(last) = last_frame {
				gaps.push(now.duration_since(last).as_secs_f64() * 1000.);
			}
			last_frame = Some(now);
			frames += 1;
			second_frames += 1;
			bytes += frame.data.len() as u64;
			second_bytes += frame.data.len() as u64;
			keyframes += u64::from(frame.keyframe);
			largest = largest.max(frame.data.len());
			if let Some(out) = &mut out {
				out.write_all(&(frame.data.len() as u32).to_le_bytes())
					.unwrap();
				out.write_all(&frame.data).unwrap();
			}
			if decoder.is_some() {
				let _ = decode_send.try_send(frame.data);
			}
			if second.elapsed() >= Duration::from_secs(1) {
				println!(
					"[{:>6.2}s] {:>3} fps {:>6} kbps",
					started.elapsed().as_secs_f64(),
					second_frames,
					second_bytes * 8 / 1000
				);
				second = Instant::now();
				second_frames = 0;
				second_bytes = 0;
			}
		}
		let cpu_end = thread_cpu();
		let peak_rss = status_kib("VmHWM:");
		let rss = status_kib("VmRSS:");
		drop(video);
		let done = worker.shutdown();
		let stopped = Instant::now();
		let clean = done.recv_timeout(Duration::from_secs(5));
		let teardown = stopped.elapsed();
		drop(decode_send);
		let decoded = decoder.map(|thread| thread.join().unwrap());
		if let Some(out) = &mut out {
			out.flush().unwrap();
		}

		let Some((measured_at, cpu_start)) = measuring else {
			eprintln!(
				"bench failed before warmup finished: {}",
				failure.unwrap_or("no encoded frames")
			);
			std::process::exit(1);
		};
		let elapsed = measured_at.elapsed().as_secs_f64();
		let ticks = 100.; // USER_HZ on Linux.
		let mut rows: Vec<(String, f64)> = cpu_end
			.iter()
			.map(|(name, end)| {
				let used = end.saturating_sub(*cpu_start.get(name).unwrap_or(&0));
				(name.clone(), used as f64 / ticks / elapsed * 100.)
			})
			.filter(|(_, percent)| *percent >= 0.5)
			.collect();
		rows.sort_by(|a, b| b.1.total_cmp(&a.1));
		let total: f64 = rows
			.iter()
			.filter(|(name, _)| name != "bench-decode")
			.map(|(_, percent)| percent)
			.sum();
		gaps.sort_by(f64::total_cmp);
		println!("---");
		if let Some(error) = failure {
			println!("failure: {error}");
		}
		println!(
			"measured: {elapsed:.1} s after {} s warmup",
			warmup.as_secs()
		);
		println!(
			"frames: {frames} ({:.1} fps of {}), keyframes {keyframes}, previews {previews} ({:.1}/s)",
			frames as f64 / elapsed,
			settings.fps,
			previews as f64 / elapsed
		);
		println!(
			"bitrate: {:.0} kbps (target {} kbps), largest frame {} KiB",
			bytes as f64 * 8. / 1000. / elapsed,
			video_target(&args) / 1000,
			largest / 1024
		);
		println!(
			"frame gap ms: p50 {:.1} p95 {:.1} p99 {:.1} max {:.1}",
			percentile(&gaps, 0.5),
			percentile(&gaps, 0.95),
			percentile(&gaps, 0.99),
			gaps.last().copied().unwrap_or(0.)
		);
		println!("cpu: {total:.1}% of one core (excluding bench-decode)");
		for (name, percent) in rows {
			println!("  {percent:>6.1}%  {name}");
		}
		println!("rss: {} MiB now, {} MiB peak", rss / 1024, peak_rss / 1024);
		println!(
			"teardown: {:.0} ms ({})",
			teardown.as_secs_f64() * 1000.,
			match clean {
				Ok(Ok(())) => "ok".to_string(),
				Ok(Err(error)) => error.to_string(),
				Err(_) => "timed out".to_string(),
			}
		);
		if let Some((ok, failed)) = decoded {
			println!("decode check: {ok} decoded, {failed} failed");
		}
	}

	fn video_target(args: &Args) -> u32 {
		args.bitrate.unwrap_or(args.settings.bit_rate())
	}
}
