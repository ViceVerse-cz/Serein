// Offline check of the production aggregation, queue and output limits. No devices/network.
include!("../src/diagnostics.rs");

fn main() {
	let (send, receive) = mpsc::sync_channel(8);
	let mut metrics = Metrics::new(Scope::Audio);
	metrics.send = None;
	assert!(metrics.start().is_none());
	metrics.poll(true, 1, true, 1);
	metrics.add(Stage::VideoSend, Duration::from_micros(10));
	metrics.stream_state([true; 8], 4);
	metrics.checkpoint();
	assert!(receive.try_recv().is_err());
	assert_eq!(metrics.report.wakes, 0);
	assert_eq!(metrics.report.stages, [[0; 3]; 11]);
	assert_eq!(metrics.report.stream_ticks, [0; 8]);
	assert_eq!(metrics.report.queued_audio, 0);
	// Keep the synthetic sender alive for this short-lived debug process.
	metrics.send = Some(Box::leak(Box::new(send)));
	for stage in [
		Stage::EchoRender,
		Stage::EchoCapture,
		Stage::Noise,
		Stage::Encode,
		Stage::Mix,
		Stage::Receive,
		Stage::VideoSend,
		Stage::VideoReceive,
		Stage::CaptureRead,
		Stage::CaptureQueue,
		Stage::CaptureRestart,
	] {
		let start = metrics.start();
		metrics.finish(stage, start);
	}
	assert!(
		metrics
			.report
			.stages
			.iter()
			.all(|s| s[0] == 1 && s[1] == s[2])
	);
	metrics.poll(true, 2, true, 3);
	assert_eq!(
		(
			metrics.report.wakes,
			metrics.report.resets,
			metrics.report.drops,
			metrics.report.stalls,
			metrics.report.noise_frames
		),
		(1, 1, 2, 1, 3)
	);
	metrics.stream_state([true, true, true, false, false, true, false, true], 4);
	metrics.stream_state([true, false, false, false, false, true, true, true], 2);
	metrics.since -= Duration::from_secs(5);
	metrics.poll(false, 0, false, 0);
	let report = receive.try_recv().unwrap();
	assert!(report.window_ms >= 5000);
	assert_eq!(report.stream_ticks, [2, 1, 1, 0, 0, 2, 1, 2]);
	assert_eq!(report.queued_audio, 6);
	assert_eq!(metrics.report.wakes, 0);
	assert_eq!(metrics.report.stages, [[0; 3]; 11]);
	assert_eq!(metrics.report.stream_ticks, [0; 8]);
	assert_eq!(metrics.report.queued_audio, 0);
	metrics.add(Stage::CaptureRead, Duration::from_micros(10));
	metrics.add(Stage::CaptureRead, Duration::from_micros(20));
	metrics.add(Stage::CaptureQueue, Duration::from_micros(5));
	metrics.add(Stage::CaptureRestart, Duration::from_micros(40));
	metrics.poll(true, 2, true, 0);
	metrics.checkpoint();
	let native = receive.try_recv().unwrap();
	assert_eq!(&native.stages[8..], &[[2, 30, 20], [1, 5, 5], [1, 40, 40]]);
	assert_eq!(
		(native.wakes, native.resets, native.drops, native.stalls),
		(1, 1, 2, 1)
	);
	assert_eq!(metrics.report.stages, [[0; 3]; 11]);
	assert_eq!(
		(
			metrics.report.wakes,
			metrics.report.resets,
			metrics.report.drops,
			metrics.report.stalls
		),
		(0, 0, 0, 0)
	);
	for _ in 0..9 {
		metrics.checkpoint();
	}
	assert_eq!(
		receive.try_iter().count(),
		8,
		"full reporter queue drops instead of blocking"
	);
	drop(receive);
	metrics.flush();
	assert!(
		metrics.start().is_none(),
		"disconnected reporter disables timing"
	);

	let mut output = Vec::new();
	let mut bytes = 64 * 1024;
	assert!(write_report(report, &mut bytes, &mut output));
	assert_eq!(bytes, 64 * 1024 - output.len());
	assert!(
		std::str::from_utf8(&output)
			.unwrap()
			.starts_with("[Serein voice Audio]")
	);
	assert!(
		!std::str::from_utf8(&output)
			.unwrap()
			.contains("stream_ticks")
	);
	assert!(!std::str::from_utf8(&output).unwrap().contains("video_send"));
	let mut too_small = output.len() - 1;
	let mut rejected = Vec::new();
	assert!(!write_report(report, &mut too_small, &mut rejected));
	assert!(rejected.is_empty());
	let mut closed = &mut [][..];
	let before = bytes;
	assert!(!write_report(report, &mut bytes, &mut closed));
	assert_eq!(
		before - bytes,
		output.len(),
		"failed writes still consume budget"
	);
	for scope in [
		Scope::Transport,
		Scope::StreamSend,
		Scope::StreamReceive,
		Scope::ScreenAudio,
	] {
		let mut transport = report;
		transport.scope = scope;
		let mut output = Vec::new();
		let before = bytes;
		assert!(write_report(transport, &mut bytes, &mut output));
		assert_eq!(before - bytes, output.len());
		let text = std::str::from_utf8(&output).unwrap();
		let stream = matches!(scope, Scope::StreamSend | Scope::StreamReceive);
		assert_eq!(text.contains("video_send="), stream);
		assert_eq!(text.contains("video_receive="), stream);
		for label in ["capture_read=", "capture_queue=", "capture_restart="] {
			assert_eq!(text.contains(label), matches!(scope, Scope::ScreenAudio));
		}
		assert_eq!(text.contains("stream_ticks: transport_key=2 dave_ready=1 group_ready=1 pending=0 waiting=0 announced=2 capture_ready=1 audio_enabled=2 queued_audio=6"), stream);
		let mut too_small = output.len() - 1;
		let mut rejected = Vec::new();
		assert!(!write_report(transport, &mut too_small, &mut rejected));
		assert!(rejected.is_empty());
		std::io::stderr().write_all(&output).unwrap();
	}
	println!(
		"Offline voice diagnostics check passed: timing and stream-state aggregation, disabled mode, periodic reset/flush, bounded nonblocking queue, byte budget and closed output."
	);
}
