//! Device-free Go Live sender/viewer integration; no Discord connection or capture device.
use super::*;
use crate::screen::{AudioChunk, EncodedFrame, Settings, SourceId, Video};
use client_core::voice::Secret;
use model::Id;
use std::sync::atomic::AtomicU64;
use tokio::net::TcpListener;

type TestSocket = WebSocketStream<TcpStream>;

async fn event(ws: &mut TestSocket, value: Value) {
	ws.send(Message::Text(value.to_string().into()))
		.await
		.unwrap();
}

async fn message(ws: &mut TestSocket) -> Message {
	loop {
		let message = ws.next().await.unwrap().unwrap();
		if let Message::Text(text) = &message {
			let value: Value = serde_json::from_str(text).unwrap();
			if value["op"] == 3 {
				event(ws, json!({"op":6,"d":{"t":value["d"]["t"]}})).await;
				continue;
			}
		}
		return message;
	}
}

async fn connect(
	listener: &TcpListener,
	delivery: &crate::test_mls::Delivery,
	user: u64,
	codec: VideoCodec,
	pending: bool,
) -> (TestSocket, UdpSocket, SocketAddr, Vec<u8>, bool) {
	let (tcp, _) = listener.accept().await.unwrap();
	let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
	let identify = message(&mut ws).await;
	let identify: Value = serde_json::from_str(identify.to_text().unwrap()).unwrap();
	assert_eq!(identify["op"], 0);
	assert_eq!(identify["d"]["user_id"], user.to_string());
	assert_eq!(identify["d"]["server_id"], "4");
	assert_eq!(identify["d"]["max_dave_protocol_version"], 1);
	let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
	event(
		&mut ws,
		json!({"op":8,"d":{"heartbeat_interval":if pending {1000} else {5000}}}),
	)
	.await;
	event(&mut ws, json!({"op":2,"d":{"ssrc":40+user,"ip":"127.0.0.1","port":udp.local_addr().unwrap().port(),"modes":[MODE],"streams":[{"ssrc":50+user}]}})).await;
	let mut probe = [0; 74];
	let (length, client) = udp.recv_from(&mut probe).await.unwrap();
	assert_eq!(length, 74);
	probe[..4].copy_from_slice(&[0, 2, 0, 70]);
	probe[8..17].copy_from_slice(b"127.0.0.1");
	probe[72..].copy_from_slice(&client.port().to_be_bytes());
	udp.send_to(&probe, client).await.unwrap();
	let selected = message(&mut ws).await;
	let selected: Value = serde_json::from_str(selected.to_text().unwrap()).unwrap();
	assert_eq!(selected["op"], 1);
	assert_eq!(selected["d"]["codecs"][0]["payload_type"], 120);
	assert_eq!(
		selected["d"]["codecs"][1]["payload_type"],
		if user == 1 {
			crate::video::payload_type(codec)
		} else {
			101
		}
	);
	ws.send(Message::Binary(
		[&[0, 1, 25], delivery.external.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	event(&mut ws, json!({"op":11,"d":{"user_ids":["1","2"]}})).await;
	event(&mut ws, json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":1,"video_codec":crate::video::codec_name(codec)}})).await;
	let mut soundshare = false;
	let package = loop {
		match message(&mut ws).await {
			Message::Binary(package) => break package.to_vec(),
			Message::Text(text) => {
				let value: Value = serde_json::from_str(&text).unwrap();
				assert_eq!(value["op"], 5);
				soundshare |= value["d"]["speaking"] == 2;
			}
			other => panic!("Unexpected negotiation frame: {other:?}"),
		}
	};
	assert_eq!(package[0], 26);
	(ws, udp, client, package, soundshare)
}

fn credentials(user: u64) -> VoiceConnection {
	VoiceConnection {
		channel: Id(3),
		guild: Some(Id(4)),
		user: Id(user),
		peer: Some(Id(3 - user)),
		session: Secret::new("synthetic-session".into()).unwrap(),
		token: Secret::new("synthetic-token".into()).unwrap(),
		endpoint: "voice.discord.media".into(),
		request: 1,
	}
}

#[tokio::test]
async fn source_resize_restarts_even_when_preset_output_dimensions_are_unchanged() {
	timeout(
		Duration::from_secs(15),
		exchange(false, TestPicture::ResizedSameOutput),
	)
	.await
	.unwrap();
}

#[tokio::test]
async fn pending_source_geometry_keeps_heartbeats_and_delivers_native_ultrawide() {
	timeout(
		Duration::from_secs(20),
		exchange(false, TestPicture::PendingUltrawide),
	)
	.await
	.unwrap();
}
#[tokio::test]
async fn pending_source_selection_can_be_cancelled_without_announcing_video() {
	timeout(
		Duration::from_secs(10),
		exchange(false, TestPicture::CancelledPicker),
	)
	.await
	.unwrap();
}
#[tokio::test]
async fn source_resize_announces_new_geometry_before_delivering_new_pictures() {
	timeout(
		Duration::from_secs(15),
		exchange(false, TestPicture::Resized),
	)
	.await
	.unwrap();
}

#[tokio::test]
async fn local_stream_sender_and_viewer_deliver_audio_and_video() {
	timeout(Duration::from_secs(15), exchange(false, TestPicture::Sdr))
		.await
		.expect("Synthetic Go Live exchange timed out");
}

#[tokio::test]
async fn established_streams_bound_missing_rekey_execution_and_welcome() {
	timeout(Duration::from_secs(45), exchange(true, TestPicture::Sdr))
		.await
		.expect("Stream rekey must time out while signaling remains healthy");
}

#[tokio::test]
async fn experimental_stream_rejects_h264_selection_for_h265_or_av1() {
	for codec in [VideoCodec::H265, VideoCodec::Av1] {
		timeout(Duration::from_secs(10), reject_h264_selection(codec))
			.await
			.expect("Codec rejection handshake timed out");
	}
}

async fn reject_h264_selection(codec: VideoCodec) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("ws://{}", listener.local_addr().unwrap());
	let (_frames_tx, frames) = tokio::sync::mpsc::channel(1);
	let video = Video {
		video_settings: VideoSettings {
			backend: model::voice_settings::VideoBackend::Experimental,
			codec,
			..Default::default()
		},
		output: Arc::new(std::sync::Mutex::new(Some(crate::screen::Output {
			source_dimensions: (320, 240),
			settings: Settings {
				source: SourceId::Display(1),
				width: 320,
				height: 240,
				fps: 30,
				cursor: false,
				audio: false,
			},
			generation: 0,
		}))),
		frames,
		ready: Arc::new(AtomicBool::new(false)),
		keyframe: Arc::new(AtomicBool::new(true)),
		bitrate: Arc::new(std::sync::atomic::AtomicU32::new(4_000_000)),
		audio: None,
		audio_epoch: Arc::new(AtomicU64::new(0)),
	};
	let sender = tokio::spawn(run_stream_inner(
		credentials(1),
		Identity::generate(),
		Some(video),
		None,
		None,
		None,
		|_| Ok(()),
		url,
		true,
	));
	let (tcp, _) = listener.accept().await.unwrap();
	let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
	let identify: Value = serde_json::from_str(message(&mut ws).await.to_text().unwrap()).unwrap();
	assert_eq!(identify["op"], 0);
	let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
	event(&mut ws, json!({"op":8,"d":{"heartbeat_interval":5000}})).await;
	event(&mut ws, json!({"op":2,"d":{"ssrc":41,"ip":"127.0.0.1","port":udp.local_addr().unwrap().port(),"modes":[MODE],"streams":[{"ssrc":51,"rtx_ssrc":61}]}})).await;
	let mut packet = [0; MAX_PACKET + 1];
	let (length, client) = udp.recv_from(&mut packet).await.unwrap();
	assert_eq!(length, 74);
	packet[..4].copy_from_slice(&[0, 2, 0, 70]);
	packet[8..17].copy_from_slice(b"127.0.0.1");
	packet[72..74].copy_from_slice(&client.port().to_be_bytes());
	udp.send_to(&packet[..74], client).await.unwrap();
	let selection: Value = serde_json::from_str(message(&mut ws).await.to_text().unwrap()).unwrap();
	assert_eq!(selection["op"], 1);
	let codecs = selection["d"]["codecs"].as_array().unwrap();
	assert_eq!(codecs.len(), 2);
	assert_eq!(codecs[1]["name"], crate::video::codec_name(codec));
	assert_eq!(codecs[1]["payload_type"], crate::video::payload_type(codec));
	assert_eq!(codecs[1]["encode"], true);
	assert_eq!(codecs[1]["decode"], false);
	event(&mut ws, json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":1,"video_codec":"H264"}})).await;
	assert_eq!(sender.await.unwrap(), Err(codec_negotiation_error(codec)));
	// The only remaining datagram may be an idle keepalive, never mislabeled RTP.
	for _ in 0..8 {
		match udp.try_recv_from(&mut packet) {
			Ok((length, _)) => {
				assert_eq!(length, 8);
				assert_eq!(&packet[..4], &[0x13, 0x37, 0xca, 0xfe]);
			}
			Err(error) => {
				assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
				break;
			}
		}
	}
}

#[derive(Clone, Copy)]
enum TestPicture {
	Sdr,
	PendingUltrawide,
	CancelledPicker,
	Resized,
	ResizedSameOutput,
	Ultrawide,
	Hdr(VideoCodec, &'static [u8], u32),
}
async fn exchange(rekey_timeout: bool, picture_case: TestPicture) {
	let ultrawide = matches!(
		picture_case,
		TestPicture::Ultrawide | TestPicture::PendingUltrawide
	);
	let pending = matches!(
		picture_case,
		TestPicture::PendingUltrawide | TestPicture::CancelledPicker
	);
	let same_output = matches!(picture_case, TestPicture::ResizedSameOutput);
	let resized = matches!(
		picture_case,
		TestPicture::Resized | TestPicture::ResizedSameOutput
	);
	let hdr = matches!(picture_case, TestPicture::Hdr(..));
	let codec = match picture_case {
		TestPicture::Hdr(codec, ..) => codec,
		_ => VideoCodec::H264,
	};
	let (mut width, mut height) = if same_output {
		(640, 480)
	} else if ultrawide {
		(6144, 2560)
	} else if hdr {
		(32, 16)
	} else {
		(320, 240)
	};
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("ws://{}", listener.local_addr().unwrap());
	let (frames_tx, frames) = tokio::sync::mpsc::channel(3);
	let (audio_tx, audio) = tokio::sync::mpsc::channel(4);
	let ready = Arc::new(AtomicBool::new(false));
	let keyframe = Arc::new(AtomicBool::new(true));
	let epoch = Arc::new(AtomicU64::new(0));
	let output = Arc::new(std::sync::Mutex::new(if pending {
		None
	} else {
		Some(crate::screen::Output {
			source_dimensions: (width, height),
			settings: Settings {
				source: SourceId::Display(1),
				width,
				height,
				fps: if ultrawide { 60 } else { 30 },
				cursor: false,
				audio: true,
			},
			generation: 0,
		})
	}));
	let geometry_generation = Arc::new(AtomicU64::new(0));
	let video = Video {
		video_settings: VideoSettings {
			codec,
			backend: if hdr {
				model::voice_settings::VideoBackend::Experimental
			} else {
				model::voice_settings::VideoBackend::Stable
			},
			..Default::default()
		},
		output: output.clone(),
		frames,
		ready: ready.clone(),
		keyframe: keyframe.clone(),
		bitrate: Arc::new(std::sync::atomic::AtomicU32::new(4_000_000)),
		audio: Some(audio),
		audio_epoch: epoch.clone(),
	};
	let sender_url = url.clone();
	let mut sender = tokio::spawn(async move {
		run_stream_inner(
			credentials(1),
			Identity::generate(),
			Some(video),
			None,
			None,
			None,
			|_| Ok(()),
			sender_url,
			true,
		)
		.await
	});
	let delivery = crate::test_mls::Delivery::new();
	let (mut send_ws, send_udp, send_addr, _, mut soundshare) =
		connect(&listener, &delivery, 1, codec, pending).await;
	let (playback_tx, playback_rx) = std::sync::mpsc::sync_channel(8);
	let (picture_tx, picture_rx) = std::sync::mpsc::sync_channel(1);
	let sink: VideoSink = Arc::new(move |frame| {
		let (width, height, bytes, format, transfer) = match frame.picture {
			crate::RemotePicture::Ffmpeg { picture, data } => (
				picture.width,
				picture.height,
				data.len(),
				picture.format,
				picture.transfer,
			),
			crate::RemotePicture::Native(picture) => {
				(picture.width, picture.height, picture.data.len(), 0, 2)
			}
		};
		let _ = picture_tx.try_send((frame.user, width, height, bytes, format, transfer));
	});
	let mut viewer = tokio::spawn(async move {
		run_stream_inner(
			credentials(2),
			Identity::generate(),
			None,
			Some(sink),
			Some(playback_tx),
			None,
			|_| Ok(()),
			url,
			true,
		)
		.await
	});
	let (mut view_ws, view_udp, view_addr, package, _) =
		connect(&listener, &delivery, 2, codec, pending).await;
	// An external MLS Add needs only the existing group ID and epoch; both clients
	// negotiate their actual commit/welcome rather than receiving precomputed media keys.
	let mut group = Dave::new(1, Some(2), 3).unwrap();
	group
		.session
		.set_external_sender(&delivery.external)
		.unwrap();
	let proposal = delivery.add_proposal(&group, &package);
	send_ws
		.send(Message::Binary(
			[&[0, 2, 27], proposal.as_slice()].concat().into(),
		))
		.await
		.unwrap();
	let committed = message(&mut send_ws).await.into_data();
	let (commit, welcome) = crate::test_mls::Delivery::split(&committed);
	send_ws
		.send(Message::Binary(
			[&[0, 3, 29, 0, 0], commit.as_slice()].concat().into(),
		))
		.await
		.unwrap();
	view_ws
		.send(Message::Binary(
			[&[0, 3, 30, 0, 0], welcome.as_slice()].concat().into(),
		))
		.await
		.unwrap();
	if pending {
		assert!(!ready.load(Ordering::Acquire));
		assert!(output.lock().unwrap().is_none());
		// A secured transport must heartbeat while source geometry is unknown,
		// without emitting a zero-sized stream announcement.
		let heartbeat = timeout(Duration::from_secs(2), send_ws.next())
			.await
			.unwrap()
			.unwrap()
			.unwrap();
		let heartbeat: Value = serde_json::from_str(heartbeat.to_text().unwrap()).unwrap();
		assert_eq!(heartbeat["op"], 3);
		event(&mut send_ws, json!({"op":6,"d":{"t":heartbeat["d"]["t"]}})).await;
		assert!(!ready.load(Ordering::Acquire));
		if matches!(picture_case, TestPicture::CancelledPicker) {
			sender.abort();
			viewer.abort();
			assert!(sender.await.unwrap_err().is_cancelled());
			assert!(viewer.await.unwrap_err().is_cancelled());
			return;
		}
		let request = crate::screen::RequestSettings {
			source: SourceId::Display(1),
			resolution: client_core::screen::Resolution::Source,
			fps: 60,
			cursor: false,
			audio: true,
		};
		let settings = request.resolve((6144, 2560)).unwrap();
		crate::screen::publish_output(
			&output,
			settings,
			if same_output {
				(1280, 960)
			} else {
				(width, height)
			},
			&ready,
			&keyframe,
			&std::sync::atomic::AtomicU32::new(0),
		)
		.unwrap();
	}

	loop {
		let announcement = message(&mut send_ws).await;
		let mut value: Value = serde_json::from_str(announcement.to_text().unwrap()).unwrap();
		if value["op"] == 5 {
			assert_eq!(value["d"]["speaking"], 2);
			soundshare = true;
		} else {
			assert_eq!(value["op"], 12);
			assert!(
				soundshare,
				"Soundshare must be announced before media capture starts"
			);
			assert_eq!(value["d"]["audio_ssrc"], 41);
			assert_eq!(value["d"]["video_ssrc"], 51);
			assert_eq!(value["d"]["streams"][0]["max_resolution"]["width"], width);
			assert_eq!(value["d"]["streams"][0]["max_resolution"]["height"], height);
			assert_eq!(
				value["d"]["streams"][0]["max_bitrate"],
				output.lock().unwrap().unwrap().settings.bit_rate()
			);
			value["d"]["user_id"] = json!("1");
			event(&mut view_ws, value).await;
			break;
		}
	}
	// Fence the viewer's SSRC mapping before forwarding UDP; arrival order of the
	// independent WebSocket and UDP transports is not otherwise guaranteed.
	let receiver = message(&mut view_ws).await;
	let receiver: Value = serde_json::from_str(receiver.to_text().unwrap()).unwrap();
	assert_eq!(
		receiver,
		json!({"op":12,"d":{"audio_ssrc":42,"video_ssrc":0,"rtx_ssrc":0,"streams":[]}})
	);
	let subscription = message(&mut view_ws).await;
	let subscription: Value = serde_json::from_str(subscription.to_text().unwrap()).unwrap();
	assert_eq!(subscription, json!({"op":15,"d":{"any":100}}));
	view_ws
		.send(Message::Ping(b"mapped".to_vec().into()))
		.await
		.unwrap();
	assert!(
		matches!(message(&mut view_ws).await, Message::Pong(data) if data.as_ref() == b"mapped")
	);
	if resized {
		(width, height) = (640, 480);
		let settings = Settings {
			source: SourceId::Display(1),
			width,
			height,
			fps: 30,
			cursor: false,
			audio: true,
		};
		let current = crate::screen::publish_output(
			&output,
			settings,
			if same_output {
				(1280, 960)
			} else {
				(width, height)
			},
			&ready,
			&keyframe,
			&std::sync::atomic::AtomicU32::new(0),
		)
		.unwrap();
		geometry_generation.store(current.generation, Ordering::Release);
		assert!(!ready.load(Ordering::Acquire));
		let mut announcement: Value = loop {
			let message = message(&mut send_ws).await;
			let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
			if value["op"] == 5 {
				assert_eq!(value["d"]["speaking"], 2);
				continue;
			}
			break value;
		};
		assert_eq!(announcement["op"], 12);
		assert_eq!(
			announcement["d"]["streams"][0]["max_resolution"],
			json!({"type":"fixed","width":width,"height":height})
		);
		announcement["d"]["user_id"] = json!("1");
		event(&mut view_ws, announcement).await;
		assert!(ready.load(Ordering::Acquire));
	}

	// Sender-only sessions must keep receiving authenticated RTCP feedback after
	// discovery. An unrelated media SSRC must not force our encoder's keyframe.
	assert!(ready.load(Ordering::Acquire));
	keyframe.store(false, Ordering::Release);
	let mut feedback = Encryption::new(&[7; 32]);
	for (media, expected, deadline) in [(52, false, 40), (51, true, 1000)] {
		let (header, body) = pli(42, media);
		send_udp
			.send_to(&feedback.seal_rtcp(&header, &body).unwrap(), send_addr)
			.await
			.unwrap();
		let requested = timeout(Duration::from_millis(deadline), async {
			let mut poll = tokio::time::interval(Duration::from_millis(2));
			while !keyframe.load(Ordering::Acquire) {
				poll.tick().await;
			}
		})
		.await;
		assert_eq!(requested.is_ok(), expected, "PLI media SSRC {media}");
	}
	// A receive-only stream must maintain UDP even without outgoing media or
	// keyframe requests. Echo native pong packets before verifying media below.
	let mut ping = [0; MAX_PACKET + 1];
	let mut ping_sequence = 0u32;
	timeout(Duration::from_secs(8), async {
		while ping_sequence < 2 {
			tokio::select! {
				result = view_udp.recv_from(&mut ping) => {
					let (length, address) = result.unwrap();
					assert_eq!(address, view_addr);
					if length != 8 { continue; } // Authenticated RTCP is separate.
					assert_eq!(&ping[..4], &[0x13, 0x37, 0xca, 0xfe]);
					ping_sequence += 1;
					assert_eq!(&ping[4..8], &ping_sequence.to_le_bytes());
					ping[..4].copy_from_slice(&[0x13, 0x37, 0xf0, 0x0d]);
					view_udp.send_to(&ping[..8], view_addr).await.unwrap();
				}
				_ = message(&mut send_ws) => {},
				_ = message(&mut view_ws) => {},
			}
		}
	})
	.await
	.expect("Idle viewer stopped maintaining UDP");
	let mut encoder = openh264::encoder::Encoder::with_api_config(
		openh264::OpenH264API::from_source(),
		openh264::encoder::EncoderConfig::new(),
	)
	.unwrap();
	let source = openh264::formats::YUVBuffer::new(width as usize, height as usize);
	let mut encoded = Vec::new();
	if ultrawide || hdr {
		use std::io::Read;
		let fixture = match picture_case {
			TestPicture::Hdr(_, bytes, _) => bytes,
			_ => ULTRAWIDE_KEYFRAME,
		};
		flate2::read::ZlibDecoder::new(fixture)
			.take(crate::video_receive::MAX_FRAME_BYTES as u64)
			.read_to_end(&mut encoded)
			.unwrap();
	} else {
		encoder.encode(&source).unwrap().write_vec(&mut encoded);
	}
	assert!(crate::video::is_keyframe_for_codec(&encoded, codec));
	let mut tick = tokio::time::interval(if ultrawide {
		Duration::from_nanos(1_000_000_000 / 60)
	} else {
		Duration::from_millis(20)
	});
	tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
	let transport = Encryption::new(&[7; 32]);
	let mut packet = [0; MAX_PACKET + 1];
	let mut timestamp = 0;
	let mut heard = false;
	let mut picture = None;
	let mut audio_packets = 0;
	let mut video_packets = 0;
	let mut previous_audio: Option<(u16, u32)> = None;
	loop {
		tokio::select! {
			_ = tick.tick() => {
				assert!(ready.load(Ordering::Acquire));
				let samples = (0..STREAM_AUDIO_FRAME).map(|i| ((i/2) as f32 * if i%2 == 0 {0.06} else {0.1}).sin() * 0.3).collect();
				let _ = audio_tx.try_send(AudioChunk { samples, epoch: epoch.load(Ordering::Acquire) });
				let _ = frames_tx.try_send(EncodedFrame { data: encoded.clone(), timestamp, keyframe: true, codec, epoch: epoch.load(Ordering::Acquire), reset_generation: epoch.load(Ordering::Acquire), geometry_generation: geometry_generation.load(Ordering::Acquire) });
				timestamp += if ultrawide { 1500 } else { 1800 };
				while let Ok(frame) = playback_rx.try_recv() { heard |= frame.iter().any(|sample| sample.abs() > 0.01); }
				if let Ok(frame) = picture_rx.try_recv() { picture = Some(frame); }
				if heard && picture.is_some() && audio_packets >= 3 { break; }
			}
			result = send_udp.recv_from(&mut packet) => {
				let (length, address) = result.unwrap();
				assert_eq!(address, send_addr);
				if length == 8 {
					assert_eq!(&packet[..4], &[0x13, 0x37, 0xca, 0xfe]);
					continue;
				}
				let rtp = transport.open(&packet[..length]).expect("Authenticated RTP");
				match rtp.payload_type {
					120 => {
						assert_eq!(rtp.ssrc, 41);
						assert_eq!(packet[0] & 0x10, 0x10);
						assert_eq!(&packet[12..16], &[0xbe, 0xde, 0, 1]);
						if let Some((sequence, timestamp)) = previous_audio {
							assert_eq!(rtp.sequence, sequence.wrapping_add(1));
							let elapsed = rtp.timestamp.wrapping_sub(timestamp);
							assert!(elapsed > 0 && elapsed.is_multiple_of(960));
						}
						previous_audio = Some((rtp.sequence, rtp.timestamp));
						audio_packets += 1;
					}
					media if media == crate::video::payload_type(codec) => { assert_eq!(rtp.ssrc, 51); video_packets += 1; }
					other => panic!("Unexpected RTP payload type: {other}"),
				}
				view_udp.send_to(&packet[..length], view_addr).await.unwrap();
			}
			_ = message(&mut send_ws) => {},
			_ = message(&mut view_ws) => {},
		}
	}
	assert!(audio_packets > 0 && video_packets > 0);
	assert_eq!(
		picture.unwrap(),
		if ultrawide {
			(1, 6144, 2560, 6144 * 2560 * 4, 0, 2)
		} else if let TestPicture::Hdr(_, _, transfer) = picture_case {
			(1, 32, 16, 32 * 16 * 3, 1, transfer)
		} else {
			(1, width, height, width as usize * height as usize * 4, 0, 2)
		}
	);
	if rekey_timeout {
		// Both have cleared their initial negotiation deadline. Keep acknowledging
		// heartbeats, but withhold the sender's Execute and viewer's new Welcome.
		event(
			&mut send_ws,
			json!({"op":21,"d":{"protocol_version":1,"transition_id":7}}),
		)
		.await;
		event(
			&mut view_ws,
			json!({"op":24,"d":{"protocol_version":1,"epoch":1}}),
		)
		.await;
		let prepared = message(&mut send_ws).await;
		let prepared: Value = serde_json::from_str(prepared.to_text().unwrap()).unwrap();
		assert_eq!(prepared, json!({"op":23,"d":{"transition_id":7}}));
		// A due opcode 15 refresh (or other JSON) can be queued ahead of the binary key package.
		loop {
			let package = message(&mut view_ws).await;
			if !matches!(package, Message::Text(_)) {
				assert_eq!(package.into_data()[0], 26);
				break;
			}
		}
		let mut sender_done = false;
		let mut viewer_done = false;
		while !sender_done || !viewer_done {
			tokio::select! {
				result = &mut sender, if !sender_done => {
					assert_eq!(result.unwrap(), Err("Discord DAVE transition execution timed out; no audio was enabled"));
					sender_done = true;
				},
				result = &mut viewer, if !viewer_done => {
					assert_eq!(result.unwrap(), Err("Discord DAVE group negotiation timed out; no accepted commit or welcome was received"));
					viewer_done = true;
				},
				frame = send_ws.next(), if !sender_done => {
					if let Some(Ok(Message::Text(text))) = frame {
						let value: Value = serde_json::from_str(&text).unwrap();
						assert_eq!(value["op"], 3);
						event(&mut send_ws, json!({"op":6,"d":{"t":value["d"]["t"]}})).await;
					}
				},
				frame = view_ws.next(), if !viewer_done => {
					if let Some(Ok(Message::Text(text))) = frame {
						let value: Value = serde_json::from_str(&text).unwrap();
						assert_eq!(value["op"], 3);
						event(&mut view_ws, json!({"op":6,"d":{"t":value["d"]["t"]}})).await;
					}
				},
			}
		}
		assert!(!ready.load(Ordering::Acquire));
		return;
	}
	send_ws.close(None).await.unwrap();
	view_ws.close(None).await.unwrap();
	assert_eq!(
		sender.await.unwrap(),
		Err("Discord stream connection closed")
	);
	assert_eq!(
		viewer.await.unwrap(),
		Err("Discord stream connection closed")
	);
}

#[tokio::test]
async fn exact_6144_by_2560_at_60fps_decodes_through_rtp_and_dave() {
	timeout(
		Duration::from_secs(20),
		exchange(false, TestPicture::Ultrawide),
	)
	.await
	.expect("Ultrawide encrypted sender/viewer exchange timed out");
}

// Device-free black H264/60fps fixture; zlib stores it compactly in the existing test file.
// Generated by FFmpeg: color=c=black:s=6144x2560:r=60, one libx264 baseline frame,
// preset ultrafast, keyint=1:slices=1, Annex B. This is test data, not a linked GPL encoder.
const ULTRAWIDE_KEYFRAME: &[u8] = &[
	120, 218, 237, 210, 191, 111, 211, 64, 20, 192, 113, 135, 95, 130, 169, 226, 63, 56, 9, 9, 49,
	180, 137, 157, 68, 33, 141, 240, 80, 170, 138, 46, 72, 76, 221, 144, 117, 177, 47, 177, 21,
	255, 202, 217, 37, 9, 83, 7, 6, 254, 2, 254, 14, 54, 38, 36, 70, 88, 88, 161, 82, 7, 38, 36,
	22, 248, 19, 202, 187, 20, 196, 159, 192, 242, 117, 62, 58, 63, 223, 189, 123, 103, 61, 199,
	243, 188, 206, 252, 241, 135, 71, 23, 157, 179, 206, 193, 219, 187, 158, 119, 221, 235, 184,
	97, 189, 115, 251, 198, 216, 147, 197, 244, 243, 206, 71, 185, 221, 186, 121, 121, 121, 114,
	113, 244, 227, 253, 247, 243, 227, 119, 111, 118, 191, 170, 243, 123, 63, 127, 173, 251, 163,
	161, 218, 83, 113, 101, 141, 10, 36, 180, 131, 192, 31, 171, 65, 96, 130, 253, 217, 190, 44,
	28, 119, 37, 161, 247, 244, 217, 209, 147, 189, 161, 58, 56, 57, 148, 204, 196, 196, 178, 112,
	88, 213, 155, 220, 204, 90, 213, 247, 253, 193, 94, 223, 239, 15, 100, 50, 109, 219, 122, 210,
	235, 173, 86, 171, 238, 139, 44, 49, 85, 174, 203, 110, 101, 231, 61, 119, 74, 55, 109, 139,
	92, 114, 170, 186, 205, 170, 178, 153, 168, 88, 79, 117, 28, 250, 202, 154, 89, 24, 168, 196,
	76, 243, 42, 94, 132, 254, 68, 126, 74, 151, 58, 223, 52, 198, 61, 169, 194, 132, 73, 166, 85,
	115, 58, 149, 200, 87, 117, 179, 145, 116, 25, 35, 155, 132, 65, 215, 151, 20, 25, 84, 145,
	173, 77, 18, 185, 90, 110, 71, 100, 117, 57, 55, 97, 48, 82, 113, 106, 171, 66, 71, 178, 53,
	80, 173, 53, 121, 158, 53, 146, 49, 94, 143, 147, 184, 149, 32, 94, 22, 50, 38, 70, 39, 47,
	171, 210, 132, 253, 96, 55, 8, 212, 76, 55, 109, 84, 55, 139, 172, 150, 77, 127, 10, 44, 235,
	168, 154, 205, 26, 227, 54, 181, 169, 149, 13, 77, 248, 80, 229, 85, 181, 208, 169, 60, 68,
	127, 231, 2, 213, 228, 89, 108, 254, 77, 248, 87, 19, 110, 165, 180, 219, 179, 226, 172, 208,
	173, 123, 159, 172, 108, 141, 205, 181, 100, 203, 252, 52, 63, 181, 122, 19, 197, 85, 81, 235,
	237, 155, 73, 147, 90, 171, 179, 82, 106, 73, 162, 213, 46, 103, 102, 117, 97, 92, 205, 149,
	201, 230, 105, 91, 75, 180, 48, 27, 89, 150, 106, 87, 65, 84, 100, 165, 123, 137, 216, 148, 38,
	62, 117, 133, 182, 155, 93, 103, 172, 105, 82, 215, 239, 56, 140, 237, 76, 21, 83, 105, 135,
	107, 168, 60, 132, 253, 65, 215, 87, 75, 119, 118, 232, 119, 71, 18, 214, 174, 204, 246, 174,
	215, 225, 104, 95, 130, 166, 53, 117, 56, 84, 89, 45, 189, 149, 47, 40, 173, 31, 202, 119, 90,
	134, 190, 119, 38, 127, 46, 243, 250, 213, 228, 254, 3, 239, 206, 181, 79, 92, 255, 243, 250,
	242, 252, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	0, 0, 0, 0, 0, 0, 0, 255, 209, 183, 223, 204, 4, 4, 96,
];

// HDR fixtures are offline reference patches (black, diffuse white, highlights),
// not captured content. FFmpeg generated YUV420P10 at 60 fps with BT2020-NCL
// and explicit PQ/HLG metadata. External test-only libx265/libaom encoders are
// not part of Serein's runtime recipe. Arrays are bounded zlib data.
const HDR_HEVC_PQ: &[u8] = &[
	120, 218, 99, 96, 96, 96, 116, 96, 228, 97, 252, 255, 159, 133, 131, 129, 129, 153, 97, 238,
	10, 16, 201, 240, 127, 23, 147, 3, 3, 80, 202, 137, 145, 17, 85, 124, 129, 147, 160, 89, 244,
	170, 61, 235, 178, 132, 20, 132, 192, 18, 32, 130, 241, 1, 88, 177, 11, 227, 193, 194, 78, 33,
	6, 26, 152, 200, 168, 193, 184, 158, 117, 135, 202, 140, 255, 204, 230, 255, 255, 63, 184, 90,
	35, 62, 155, 233, 82, 93, 200, 15, 29, 40, 236, 175, 64, 129, 207, 50, 110, 188, 16, 96, 175,
	111, 217, 254, 161, 253, 143, 208, 223, 95, 181, 119, 160, 240, 94, 44, 10, 140, 41, 188, 203,
	180, 156, 129, 97, 226, 155, 39, 223, 18, 204, 196, 181, 185, 148, 57, 122, 228, 95, 116, 28,
	82, 158, 245, 86, 242, 79, 150, 229, 186, 139, 243, 143, 237, 118, 91, 92, 216, 192, 224, 255,
	181, 253, 135, 243, 239, 150, 202, 233, 103, 249, 107, 185, 214, 61, 173, 93, 90, 231, 98, 145,
	125, 54, 190, 54, 238, 217, 222, 165, 53, 87, 60, 228, 21, 24, 196, 165, 22, 240, 250, 42, 58,
	246, 133, 156, 121, 114, 206, 231, 201, 57, 95, 199, 16, 81, 40, 10, 126, 41, 156, 192, 32, 51,
	219, 194, 117, 70, 111, 151, 213, 109, 205, 69, 203, 186, 102, 105, 101, 172, 220, 239, 246,
	87, 250, 75, 254, 69, 8, 53, 179, 168, 106, 13, 0, 244, 140, 144, 61,
];
const HDR_HEVC_HLG: &[u8] = &[
	120, 218, 99, 96, 96, 96, 116, 96, 228, 97, 252, 255, 159, 133, 131, 129, 129, 153, 97, 238,
	10, 16, 201, 240, 127, 23, 147, 3, 3, 80, 202, 137, 145, 17, 85, 124, 129, 147, 160, 89, 244,
	170, 61, 235, 178, 132, 84, 132, 192, 18, 32, 130, 241, 1, 88, 177, 11, 227, 193, 194, 78, 33,
	6, 26, 152, 200, 168, 193, 184, 158, 117, 135, 202, 140, 255, 204, 230, 255, 255, 63, 184, 90,
	35, 62, 155, 233, 82, 93, 200, 15, 29, 40, 236, 175, 64, 129, 207, 50, 110, 188, 16, 96, 175,
	79, 206, 127, 19, 247, 155, 99, 111, 105, 108, 56, 20, 70, 218, 162, 64, 155, 194, 187, 76,
	203, 25, 24, 38, 62, 127, 94, 252, 176, 120, 70, 26, 79, 191, 144, 141, 219, 111, 231, 35, 253,
	75, 246, 176, 179, 36, 191, 212, 103, 60, 40, 245, 65, 249, 222, 3, 6, 225, 63, 155, 254, 158,
	173, 59, 58, 143, 251, 67, 251, 109, 206, 239, 27, 222, 251, 213, 126, 219, 206, 124, 40, 63,
	167, 238, 200, 95, 203, 15, 83, 21, 106, 26, 24, 98, 10, 118, 223, 8, 138, 80, 225, 239, 251,
	81, 252, 240, 124, 187, 157, 204, 143, 11, 201, 142, 189, 108, 38, 18, 96, 202, 216, 197, 183,
	129, 161, 40, 87, 42, 172, 238, 154, 107, 247, 210, 231, 249, 115, 247, 149, 253, 188, 253,
	252, 145, 124, 71, 157, 226, 143, 118, 48, 245, 125, 233, 201, 87, 27, 0, 221, 230, 141, 243,
];
const HDR_AV1_PQ: &[u8] = &[
	120, 218, 19, 98, 224, 226, 101, 96, 96, 96, 146, 255, 115, 235, 103, 192, 68, 134, 9, 14, 70,
	194, 34, 12, 10, 12, 12, 119, 234, 195, 47, 70, 157, 50, 47, 48, 247, 96, 99, 139, 61, 0, 0,
	189, 185, 11, 58,
];
const HDR_AV1_HLG: &[u8] = &[
	120, 218, 19, 98, 224, 226, 101, 96, 96, 96, 146, 255, 115, 235, 103, 192, 68, 133, 9, 14, 70,
	194, 34, 12, 10, 12, 12, 119, 234, 195, 143, 220, 183, 49, 47, 48, 55, 225, 98, 155, 123, 0, 0,
	192, 138, 11, 116,
];

#[tokio::test]
async fn pq_and_hlg_hevc_and_av1_decode_through_rtp_and_dave() {
	for (codec, fixture, transfer) in [
		(VideoCodec::H265, HDR_HEVC_PQ, 16),
		(VideoCodec::H265, HDR_HEVC_HLG, 18),
		(VideoCodec::Av1, HDR_AV1_PQ, 16),
		(VideoCodec::Av1, HDR_AV1_HLG, 18),
	] {
		timeout(
			Duration::from_secs(20),
			exchange(false, TestPicture::Hdr(codec, fixture, transfer)),
		)
		.await
		.expect("HDR encrypted sender/viewer exchange timed out");
	}
}
