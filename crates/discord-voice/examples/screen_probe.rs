//! Screen-capture pipeline probe: opens one ScreenCast portal session and runs a caller-given
//! GStreamer tail on its PipeWire node, reporting delivered pictures per second and per-thread
//! CPU. Nothing is encoded for sending, recorded or uploaded. Use it on a disposable headless
//! compositor (see `tools/screen-lab/`) to compare capture/scale/convert designs quickly.
//!
//! The tail must name its last element `sink`; an appsink is drained like the worker does.
//! Environment knobs for source experiments: `BUFFERS` (PipeWire buffers, default 2 as in
//! Serein), `ALWAYS_COPY=1` (copy each picture and return the buffer at once) and `SETTLE_MS`
//! (delay between the portal's Start and connecting).
//!
//! ```sh
//! cargo run -p discord-voice -p platform --features winit/wayland --example screen_probe -- 8 \
//!   "videoconvertscale ! video/x-raw,format=I420,width=1280,height=720 ! fakesink name=sink"
//! ```
#![cfg(not(test))]
#![allow(dead_code, clippy::print_stdout)]

#[cfg(target_os = "linux")]
#[path = "../src/screen/portal_linux.rs"]
mod portal_linux;

#[cfg(not(target_os = "linux"))]
fn main() {
	eprintln!("The screen probe needs Linux with a ScreenCast portal.");
}

#[cfg(target_os = "linux")]
fn main() {
	use gst::prelude::*;
	use gstreamer as gst;
	use std::{
		os::fd::AsRawFd,
		sync::{
			Arc,
			atomic::{AtomicBool, AtomicU64, Ordering},
		},
		time::{Duration, Instant},
	};
	let mut args = std::env::args().skip(1);
	let seconds: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(8);
	let tail = args
		.next()
		.unwrap_or_else(|| "fakesink name=sink sync=false".into());
	gst::init().unwrap();
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap();
	runtime.block_on(async {
		let stop = AtomicBool::new(false);
		let portal = portal_linux::Portal::open(true, &stop)
			.await
			.expect("portal session");
		let settle: u64 = std::env::var("SETTLE_MS")
			.ok()
			.and_then(|v| v.parse().ok())
			.unwrap_or(0);
		tokio::time::sleep(Duration::from_millis(settle)).await;
		let remote = portal.open_remote(&stop).await.expect("pipewire remote");
		// Mirror the production order: configure the source before it joins a pipeline.
		let source = gst::ElementFactory::make("pipewiresrc")
			.name("src")
			.build()
			.unwrap();
		source.set_property("fd", remote.as_raw_fd());
		match portal
			.pipewire_serial
			.filter(|_| source.find_property("target-object").is_some())
		{
			Some(serial) => source.set_property("target-object", serial.to_string()),
			None => source.set_property("path", portal.node_id.to_string()),
		}
		source.set_property("do-timestamp", true);
		source.set_property("keepalive-time", 1000i32);
		let buffers: i32 = std::env::var("BUFFERS")
			.ok()
			.and_then(|v| v.parse().ok())
			.unwrap_or(2);
		source.set_property("min-buffers", buffers);
		if std::env::var_os("ALWAYS_COPY").is_some() {
			source.set_property("always-copy", true);
		}
		source.set_property("max-buffers", buffers + 2);
		// pipewiresrc offers no format to a sink accepting anything.
		let bin = gst::parse::bin_from_description(
			&format!("capsfilter caps=video/x-raw(ANY) ! {tail}"),
			true,
		)
		.expect("tail");
		let pipeline = gst::Pipeline::new();
		pipeline.add_many([&source, bin.upcast_ref()]).unwrap();
		source.link(&bin).unwrap();
		let counted = Arc::new(AtomicU64::new(0));
		let source_count = Arc::new(AtomicU64::new(0));
		let sink = bin
			.by_name("sink")
			.expect("tail needs an element named sink");
		let count = counted.clone();
		sink.static_pad("sink")
			.unwrap()
			.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
				count.fetch_add(1, Ordering::Relaxed);
				gst::PadProbeReturn::Ok
			});
		let count = source_count.clone();
		let caps_seen = Arc::new(AtomicBool::new(false));
		let seen = caps_seen.clone();
		source
			.static_pad("src")
			.unwrap()
			.add_probe(gst::PadProbeType::BUFFER, move |pad, info| {
				count.fetch_add(1, Ordering::Relaxed);
				if !seen.swap(true, Ordering::Relaxed) {
					println!(
						"source caps: {:?} memory: {:?}",
						pad.current_caps().map(|caps| caps.to_string()),
						info.buffer()
							.and_then(|buffer| buffer.peek_memory(0).allocator())
							.map(|allocator| allocator.type_())
					);
				}
				gst::PadProbeReturn::Ok
			});
		// Drain an appsink the way the worker does, so it never pins a PipeWire buffer.
		if let Ok(appsink) = sink.clone().downcast::<gstreamer_app::AppSink>() {
			std::thread::spawn(move || {
				while !appsink.is_eos() {
					let _ = appsink.try_pull_sample(gst::ClockTime::from_mseconds(100));
				}
			});
		}
		pipeline.set_state(gst::State::Playing).unwrap();
		let bus = pipeline.bus().unwrap();
		let started = Instant::now();
		let mut measured = None;
		let mut previous = (0, 0);
		while started.elapsed() < Duration::from_secs(seconds) {
			tokio::time::sleep(Duration::from_secs(1)).await;
			while let Some(message) = bus.pop() {
				if let gst::MessageView::Error(error) = message.view() {
					panic!("pipeline error: {} ({:?})", error.error(), error.debug());
				}
			}
			let (sources, sinks) = (
				source_count.load(Ordering::Relaxed),
				counted.load(Ordering::Relaxed),
			);
			println!(
				"[{:>4.1}s] source {:>3}/s  sink {:>3}/s",
				started.elapsed().as_secs_f64(),
				sources - previous.0,
				sinks - previous.1
			);
			previous = (sources, sinks);
			if measured.is_none() && started.elapsed() >= Duration::from_secs(2) {
				measured = Some((Instant::now(), cpu(), sources, sinks));
			}
		}
		let (at, start, sources, sinks) = measured.expect("probe ran at least two seconds");
		let elapsed = at.elapsed().as_secs_f64();
		let end = cpu();
		let mut rows: Vec<(String, f64)> = end
			.iter()
			.map(|(name, ticks)| {
				let used = ticks.saturating_sub(*start.get(name).unwrap_or(&0));
				(name.clone(), used as f64 / elapsed) // ticks are 1/100 s, so this is percent
			})
			.filter(|(_, percent)| *percent >= 0.5)
			.collect();
		rows.sort_by(|a, b| b.1.total_cmp(&a.1));
		println!(
			"--- source {:.1}/s sink {:.1}/s cpu {:.1}%",
			(source_count.load(Ordering::Relaxed) - sources) as f64 / elapsed,
			(counted.load(Ordering::Relaxed) - sinks) as f64 / elapsed,
			rows.iter().map(|row| row.1).sum::<f64>()
		);
		for (name, percent) in rows {
			println!("  {percent:>6.1}%  {name}");
		}
		pipeline.set_state(gst::State::Null).unwrap();
		drop(remote);
		portal.close().await;
	});
}

/// Clock ticks (1/100 s) per thread role.
#[cfg(target_os = "linux")]
fn cpu() -> std::collections::HashMap<String, u64> {
	let mut threads = std::collections::HashMap::new();
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
		let ticks: u64 =
			fields[11].parse::<u64>().unwrap_or(0) + fields[12].parse::<u64>().unwrap_or(0);
		let role = stat[open + 1..close]
			.trim_end_matches(|c: char| c.is_ascii_digit() || c == ':')
			.to_string();
		*threads.entry(role).or_default() += ticks;
	}
	threads
}
