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
	event(&mut ws, json!({"op":8,"d":{"heartbeat_interval":5000}})).await;
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
	assert_eq!(selected["d"]["codecs"][1]["payload_type"], 101);
	ws.send(Message::Binary(
		[&[0, 1, 25], delivery.external.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	event(&mut ws, json!({"op":11,"d":{"user_ids":["1","2"]}})).await;
	event(&mut ws, json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":1,"video_codec":"H264"}})).await;
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
async fn local_stream_sender_and_viewer_deliver_audio_and_video() {
	timeout(Duration::from_secs(15), exchange())
		.await
		.expect("Synthetic Go Live exchange timed out");
}

async fn exchange() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("ws://{}", listener.local_addr().unwrap());
	let (frames_tx, frames) = tokio::sync::mpsc::channel(3);
	let (audio_tx, audio) = tokio::sync::mpsc::channel(4);
	let ready = Arc::new(AtomicBool::new(false));
	let keyframe = Arc::new(AtomicBool::new(true));
	let epoch = Arc::new(AtomicU64::new(0));
	let video = Video {
		settings: Settings {
			source: SourceId::Display(1),
			width: 320,
			height: 240,
			fps: 30,
			cursor: false,
			audio: true,
		},
		frames,
		ready: ready.clone(),
		keyframe: keyframe.clone(),
		audio: Some(audio),
		audio_epoch: epoch.clone(),
	};
	let sender_url = url.clone();
	let sender = tokio::spawn(async move {
		run_stream_inner(
			credentials(1),
			Identity::generate(),
			Some(video),
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
		connect(&listener, &delivery, 1).await;
	let (playback_tx, playback_rx) = std::sync::mpsc::sync_channel(8);
	let (picture_tx, picture_rx) = std::sync::mpsc::sync_channel(1);
	let sink: VideoSink = Arc::new(move |frame| {
		let _ = picture_tx.try_send((frame.user, frame.width, frame.height, frame.rgba.len()));
	});
	let viewer = tokio::spawn(async move {
		run_stream_inner(
			credentials(2),
			Identity::generate(),
			None,
			Some(sink),
			Some(playback_tx),
			|_| Ok(()),
			url,
			true,
		)
		.await
	});
	let (mut view_ws, view_udp, view_addr, package, _) = connect(&listener, &delivery, 2).await;
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
	let source = openh264::formats::YUVBuffer::new(320, 240);
	let mut encoded = Vec::new();
	encoder.encode(&source).unwrap().write_vec(&mut encoded);
	assert!(is_keyframe(&encoded));
	let mut tick = tokio::time::interval(Duration::from_millis(20));
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
				let _ = frames_tx.try_send(EncodedFrame { data: encoded.clone(), timestamp, keyframe: true });
				timestamp += 1800;
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
					101 => { assert_eq!(rtp.ssrc, 51); video_packets += 1; }
					other => panic!("Unexpected RTP payload type: {other}"),
				}
				view_udp.send_to(&packet[..length], view_addr).await.unwrap();
			}
			_ = message(&mut send_ws) => {},
			_ = message(&mut view_ws) => {},
		}
	}
	assert!(audio_packets > 0 && video_packets > 0);
	assert_eq!(picture.unwrap(), (1, 320, 240, 320 * 240 * 4));
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
