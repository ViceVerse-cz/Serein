//! Device-free Go Live sender/viewer integration; no Discord connection or capture device.
use super::*;
use crate::screen::{AudioChunk, Codec, EncodedFrame, Settings, SourceId, Video};
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
	connect_codec(listener, delivery, user, Codec::H264, Codec::H264).await
}

async fn connect_codec(
	listener: &TcpListener,
	delivery: &crate::test_mls::Delivery,
	user: u64,
	available: Codec,
	negotiated: Codec,
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
	let mut expected = vec![
		json!({"name":"opus","type":"audio","priority":1000,"payload_type":120}),
		json!({"name":"H264","type":"video","priority":1000,"payload_type":101,"rtx_payload_type":102,"encode":user==1,"decode":user==2}),
	];
	if available == Codec::Av1 {
		expected.push(json!({"name":"AV1","type":"video","priority":2000,"payload_type":105,"rtx_payload_type":106,"encode":true,"decode":false}));
	}
	assert_eq!(selected["d"]["codecs"], Value::Array(expected));
	ws.send(Message::Binary(
		[&[0, 1, 25], delivery.external.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	event(&mut ws, json!({"op":11,"d":{"user_ids":["1","2"]}})).await;
	event(&mut ws, json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":1,"video_codec":negotiated.name()}})).await;
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
		codec: tokio::sync::watch::channel(Some(Codec::H264)).1,
		codec_selection: tokio::sync::watch::channel(None).0,
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
		bitrate: Arc::new(std::sync::atomic::AtomicU32::new(4_000_000)),
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
				let _ = frames_tx.try_send(EncodedFrame { codec: Codec::H264, data: encoded.clone(), timestamp, keyframe: true });
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

#[tokio::test]
async fn local_av1_sender_negotiates_encrypts_and_delivers_valid_frame() {
	timeout(Duration::from_secs(10), codec_sender(Codec::Av1))
		.await
		.expect("Synthetic AV1 sender timed out");
}

#[tokio::test]
async fn local_av1_sender_waits_for_h264_encoder_on_server_fallback() {
	timeout(Duration::from_secs(10), codec_sender(Codec::H264))
		.await
		.expect("Synthetic H264 fallback sender timed out");
}

/// Actual transport and MLS exchange, with a mock capture worker and a local peer.
/// This never probes a GPU, captures a screen, or connects to Discord.
async fn codec_sender(negotiated: Codec) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("ws://{}", listener.local_addr().unwrap());
	let (codec_tx, codec) = tokio::sync::watch::channel(None);
	let (codec_selection, mut selection) = tokio::sync::watch::channel(None);
	let (frames_tx, frames) = tokio::sync::mpsc::channel(3);
	let ready = Arc::new(AtomicBool::new(false));
	let video = Video {
		codec,
		codec_selection,
		settings: Settings {
			source: SourceId::Display(1),
			width: 64,
			height: 64,
			fps: 30,
			cursor: false,
			audio: false,
		},
		frames,
		ready: ready.clone(),
		keyframe: Arc::new(AtomicBool::new(true)),
		bitrate: Arc::new(std::sync::atomic::AtomicU32::new(4_000_000)),
		audio: None,
		audio_epoch: Arc::new(AtomicU64::new(0)),
	};
	let sender = tokio::spawn(async move {
		run_stream_inner(
			credentials(1),
			Identity::generate(),
			Some(video),
			None,
			None,
			|_| Ok(()),
			url,
			true,
		)
		.await
	});
	// Capability is supplied only after an encoder has passed its offline probe.
	assert!(
		timeout(Duration::from_millis(30), listener.accept())
			.await
			.is_err(),
		"Transport started before validated encoder capability"
	);
	codec_tx.send_replace(Some(Codec::Av1));
	let delivery = crate::test_mls::Delivery::new();
	let (mut ws, udp, client, _, soundshare) =
		connect_codec(&listener, &delivery, 1, Codec::Av1, negotiated).await;
	assert!(!soundshare);
	selection.wait_for(|codec| codec.is_some()).await.unwrap();
	assert_eq!(*selection.borrow(), Some(negotiated));
	// Hold the capture worker on a codec different from the service selection.
	// AV1-capable workers must finish an H264 restart before fallback can send.
	codec_tx.send_replace(Some(if negotiated == Codec::Av1 {
		Codec::H264
	} else {
		Codec::Av1
	}));
	let mut peer = Dave::new(2, Some(1), 3).unwrap();
	peer.session
		.set_external_sender(&delivery.external)
		.unwrap();
	let mut creator = Dave::new(1, Some(2), 3).unwrap();
	creator
		.session
		.set_external_sender(&delivery.external)
		.unwrap();
	let proposal = delivery.add_proposal(&creator, &peer.key_package().unwrap());
	ws.send(Message::Binary(
		[&[0, 2, 27], proposal.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	let response = message(&mut ws).await.into_data();
	let (commit, welcome) = crate::test_mls::Delivery::split(&response);
	peer.group_changed(30, &[&[0, 0], welcome.as_slice()].concat())
		.unwrap();
	ws.send(Message::Binary(
		[&[0, 3, 29, 0, 0], commit.as_slice()].concat().into(),
	))
	.await
	.unwrap();
	// Fence the commit handler before checking the independent encoder readiness gate.
	ws.send(Message::Ping(b"secure".to_vec().into()))
		.await
		.unwrap();
	assert!(matches!(message(&mut ws).await, Message::Pong(data) if data.as_ref()==b"secure"));
	assert!(!ready.load(Ordering::Acquire));
	frames_tx
		.send(EncodedFrame {
			codec: negotiated,
			data: vec![1], // Must be discarded before parsing while codec readiness is false.
			timestamp: 0,
			keyframe: true,
		})
		.await
		.unwrap();
	let mut datagram = [0; MAX_PACKET + 1];
	let silence = tokio::time::sleep(Duration::from_millis(60));
	tokio::pin!(silence);
	loop {
		tokio::select! {
			_ = &mut silence => break,
			result = udp.recv_from(&mut datagram) => {
				let (length,address)=result.unwrap();
				assert_eq!(address,client);
				assert_eq!(length,8,"Media sent before encoder matched selected codec");
				assert_eq!(&datagram[..4],&[0x13,0x37,0xca,0xfe]);
			},
			unexpected = message(&mut ws) => panic!("Announced media before matching encoder: {unexpected:?}"),
		}
	}
	assert!(!ready.load(Ordering::Acquire));
	// Mock the capture worker finishing the exact service-selected encoder restart.
	codec_tx.send_replace(Some(negotiated));
	let announcement = message(&mut ws).await;
	let announcement: Value = serde_json::from_str(announcement.to_text().unwrap()).unwrap();
	assert_eq!(announcement["op"], 12);
	assert_eq!(announcement["d"]["video_ssrc"], 51);
	assert!(ready.load(Ordering::Acquire));
	// A repeated session update must preserve the already selected encoder.
	event(
		&mut ws,
		json!({"op":14,"d":{"video_codec":negotiated.name()}}),
	)
	.await;
	ws.send(Message::Ping(b"same-codec".to_vec().into()))
		.await
		.unwrap();
	assert!(matches!(message(&mut ws).await, Message::Pong(data) if data.as_ref()==b"same-codec"));
	assert!(ready.load(Ordering::Acquire));
	let encoded = match negotiated {
		Codec::Av1 => {
			// A real 64×64 libaom keyframe, encoded and decoded offline via GStreamer.
			vec![
				0x12, 0x00, 0x0a, 0x0b, 0x00, 0x00, 0x00, 0x02, 0xaf, 0xff, 0xf0, 0x36, 0xbe, 0x40,
				0x10, 0x32, 0x10, 0x10, 0x80, 0x80, 0x01, 0x00, 0x00, 0xb4, 0x51, 0xb4, 0xf0, 0xa1,
				0xf1, 0x97, 0xe0, 0x13, 0x24,
			]
		}
		Codec::H264 => {
			let mut encoder = openh264::encoder::Encoder::with_api_config(
				openh264::OpenH264API::from_source(),
				openh264::encoder::EncoderConfig::new(),
			)
			.unwrap();
			let source = openh264::formats::YUVBuffer::new(64, 64);
			let mut encoded = Vec::new();
			encoder.encode(&source).unwrap().write_vec(&mut encoded);
			encoded
		}
	};
	let expected = match negotiated {
		Codec::Av1 => crate::video_av1::normalize_source(&encoded).unwrap(),
		Codec::H264 => crate::video_sps::normalize(&encoded).unwrap().into_owned(),
	};
	frames_tx
		.send(EncodedFrame {
			codec: negotiated,
			data: encoded,
			timestamp: 90_000,
			keyframe: true,
		})
		.await
		.unwrap();
	let transport = Encryption::new(&[7; 32]);
	let mut packets = Vec::new();
	loop {
		let (length, address) = udp.recv_from(&mut datagram).await.unwrap();
		assert_eq!(address, client);
		if length == 8 {
			continue;
		}
		let rtp = transport
			.open(&datagram[..length])
			.expect("Authenticated RTP");
		assert_eq!(rtp.payload_type, negotiated.payload_type());
		assert_eq!(rtp.ssrc, 51);
		assert_eq!(rtp.timestamp, 90_000);
		if let Some(previous) = packets.last() {
			let previous: &crate::crypto::Rtp = previous;
			assert_eq!(rtp.sequence, previous.sequence.wrapping_add(1));
		}
		let marker = rtp.marker;
		packets.push(rtp);
		assert!(packets.len() <= crate::video::MAX_FRAGMENTS);
		if marker {
			break;
		}
	}
	let frame = reconstruct_codec_frame(negotiated, &packets);
	assert_eq!(
		peer.session
			.decrypt(1, davey::MediaType::VIDEO, &frame)
			.unwrap(),
		expected
	);
	// A later participant can require a different codec. End this share visibly
	// rather than continuing to send a codec that the service no longer selected.
	let changed = if negotiated == Codec::Av1 {
		Codec::H264
	} else {
		Codec::Av1
	};
	event(&mut ws, json!({"op":14,"d":{"video_codec":changed.name()}})).await;
	assert_eq!(
		sender.await.unwrap(),
		Err("Discord changed the screen-share codec; stop and start sharing again")
	);
	assert!(!ready.load(Ordering::Acquire));
}

fn reconstruct_codec_frame(codec: Codec, packets: &[crate::crypto::Rtp]) -> Vec<u8> {
	let mut output = Vec::new();
	if codec == Codec::H264 {
		for packet in packets {
			let payload = &packet.payload;
			if payload[0] & 31 == 28 {
				if payload[1] & 128 != 0 {
					output.extend([0, 0, 0, 1, (payload[0] & 0xe0) | (payload[1] & 31)]);
				}
				output.extend_from_slice(&payload[2..]);
			} else {
				output.extend([0, 0, 0, 1]);
				output.extend_from_slice(payload);
			}
		}
		return output;
	}
	let mut obus = Vec::<Vec<u8>>::new();
	let mut fragmented = false;
	for packet in packets {
		let header = packet.payload[0];
		assert_eq!(header & 0x30, 0x10);
		assert_eq!(header & 0x80 != 0, fragmented);
		if !fragmented {
			obus.push(Vec::new());
		}
		obus.last_mut()
			.unwrap()
			.extend_from_slice(&packet.payload[1..]);
		fragmented = header & 0x40 != 0;
	}
	assert!(!fragmented);
	for (index, obu) in obus.iter().enumerate() {
		let last = index + 1 == obus.len();
		let header_len = 1 + usize::from(obu[0] & 4 != 0);
		output.push(obu[0] | if last { 0 } else { 2 });
		output.extend_from_slice(&obu[1..header_len]);
		if !last {
			let mut size = obu.len() - header_len;
			loop {
				output.push((size & 127) as u8 | if size >= 128 { 128 } else { 0 });
				size >>= 7;
				if size == 0 {
					break;
				}
			}
		}
		output.extend_from_slice(&obu[header_len..]);
	}
	output
}
