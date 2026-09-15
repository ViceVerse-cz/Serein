use crate::{
	Controls, Frame, Status,
	crypto::{Dave, Encryption, Identity, MAX_PACKET, MAX_SIGNAL, MODE},
	video_receive::{Encoded, Receivers, VideoSink, is_keyframe, offer, pli, spawn_decoder},
};
use client_core::voice::VoiceConnection;
use futures_util::{SinkExt, StreamExt};
use opus2::{Application, Bitrate, Channels, Encoder};
use serde_json::{Value, json};
use std::{
	net::{IpAddr, SocketAddr},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc::{Receiver, SyncSender},
	},
	time::Duration,
};
use tokio::{
	net::{TcpStream, UdpSocket},
	sync::watch,
	time::{Instant, MissedTickBehavior, timeout},
};
use tokio_tungstenite::{
	MaybeTlsStream, WebSocketStream,
	tungstenite::{Message, protocol::WebSocketConfig},
};
use zeroize::Zeroizing;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
/// Time a sole member gives the roster announcement before waiting for a peer.
const PEER_GRACE: Duration = Duration::from_millis(500);

fn negotiation_timeout(
	hello: bool,
	transport: bool,
	key: bool,
	dave: &Dave,
	resuming: bool,
) -> &'static str {
	if resuming {
		"Discord voice resume acknowledgement timed out; rejoin the call"
	} else if !hello {
		"Discord voice Hello timed out; rejoin the call"
	} else if !transport {
		"Discord voice Ready timed out; rejoin the call"
	} else if !key {
		"Discord voice protocol selection timed out; no transport key was received"
	} else if dave.session.is_ready() && dave.pending.is_some() {
		"Discord DAVE transition execution timed out; no audio was enabled"
	} else {
		"Discord DAVE group negotiation timed out; no accepted commit or welcome was received"
	}
}

fn endpoint(raw: &str) -> Result<String, &'static str> {
	if raw.len() > 256
		|| raw.contains('/')
		|| raw.contains('@')
		|| raw.contains('?')
		|| raw.contains('#')
	{
		return Err("Invalid Discord voice endpoint");
	}
	let url = url::Url::parse(&format!("wss://{raw}/?v=8"))
		.map_err(|_| "Invalid Discord voice endpoint")?;
	let host = url.host_str().ok_or("Missing Discord voice host")?;
	if !host.ends_with(".discord.media") || url.username() != "" || url.password().is_some() {
		return Err("Voice endpoint is outside Discord media");
	}
	Ok(url.into())
}
fn public_ip(ip: IpAddr) -> bool {
	match ip {
		IpAddr::V4(ip) => {
			!ip.is_private()
				&& !ip.is_loopback()
				&& !ip.is_link_local()
				&& !ip.is_broadcast()
				&& !ip.is_multicast()
				&& !ip.is_unspecified()
				&& !ip.is_documentation()
				&& ip.octets()[0] != 0
				&& ip.octets()[0] < 240
				&& !(ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
		}
		IpAddr::V6(ip) => {
			!ip.is_loopback()
				&& !ip.is_unspecified()
				&& !ip.is_multicast()
				&& !ip.is_unique_local()
				&& !ip.is_unicast_link_local()
				&& ip.to_ipv4_mapped().is_none()
				&& !(ip.segments()[0] == 0x2001 && ip.segments()[1] == 0xdb8)
		}
	}
}
async fn send(ws: &mut Socket, message: Message) -> Result<(), &'static str> {
	timeout(Duration::from_secs(5), ws.send(message))
		.await
		.map_err(|_| "Voice signaling write timed out")?
		.map_err(|_| "Voice signaling disconnected")
}
async fn json_send(ws: &mut Socket, value: Value) -> Result<(), &'static str> {
	send(ws, Message::Text(value.to_string().into())).await
}
// Native voice UDP ping: signaling heartbeats alone do not maintain an idle
// media path (notably a receive-only stream or a muted call).
async fn udp_keepalive(socket: &UdpSocket, sequence: &mut u32) -> Result<(), &'static str> {
	*sequence = sequence.wrapping_add(1);
	let mut packet = [0x13, 0x37, 0xca, 0xfe, 0, 0, 0, 0];
	packet[4..].copy_from_slice(&sequence.to_le_bytes());
	socket
		.send(&packet)
		.await
		.map_err(|_| "Voice UDP keepalive failed")?;
	Ok(())
}
fn number(data: &Value, key: &str) -> Result<u64, &'static str> {
	data[key].as_u64().ok_or("Malformed voice signaling field")
}
fn id(data: &Value, key: &str) -> Result<u64, &'static str> {
	data[key]
		.as_str()
		.and_then(|v| v.parse().ok())
		.filter(|v| *v != 0)
		.ok_or("Malformed voice participant")
}
fn transition(data: &Value) -> Result<u16, &'static str> {
	u16::try_from(number(data, "transition_id")?).map_err(|_| "Invalid voice transition")
}
fn h264_negotiated(data: &Value) -> bool {
	data["video_codec"]
		.as_str()
		.is_some_and(|codec| codec.eq_ignore_ascii_case("H264"))
}
/// Bind every video SSRC of a client announcement (opcode 12) to its user; zero clears them.
fn announce_video(receivers: &mut Receivers, user: u64, data: &Value) -> Result<(), &'static str> {
	let ssrc = |value: &Value| value.as_u64().and_then(|v| u32::try_from(v).ok());
	receivers.announce(user, ssrc(&data["video_ssrc"]).unwrap_or(0))?;
	for stream in data["streams"].as_array().into_iter().flatten().take(4) {
		if stream["type"] == "video"
			&& let Some(value) = ssrc(&stream["ssrc"]).filter(|v| *v != 0)
		{
			receivers.announce(user, value)?;
		}
	}
	Ok(())
}
fn discovery(packet: &[u8], ssrc: u32) -> Result<(IpAddr, u16), &'static str> {
	if packet.len() != 74 || packet[..4] != [0, 2, 0, 70] || packet[4..8] != ssrc.to_be_bytes() {
		return Err("Invalid voice UDP discovery reply");
	}
	let end = packet[8..72]
		.iter()
		.position(|b| *b == 0)
		.ok_or("Invalid voice discovery address")?;
	let address = std::str::from_utf8(&packet[8..8 + end])
		.map_err(|_| "Invalid voice discovery address")?
		.parse()
		.map_err(|_| "Invalid voice discovery address")?;
	let port = u16::from_be_bytes([packet[72], packet[73]]);
	if port == 0 {
		return Err("Invalid voice discovery port");
	}
	Ok((address, port))
}

/// Drop the control sender or abort this future to stop the socket, UDP, codecs and ephemeral keys.
/// PCM queues must contain at most eight 20ms mono48k frames each. No audio device opens here.
#[allow(clippy::too_many_arguments)] // Every media input of one call.
pub async fn run(
	credentials: VoiceConnection,
	capture: Receiver<Frame>,
	playback: SyncSender<Frame>,
	controls: watch::Receiver<Controls>,
	camera: Option<Receiver<crate::camera_video::Frame>>,
	remote_video: Option<VideoSink>,
	stream_audio: Option<Receiver<Frame>>,
	emit: impl Fn(Status) -> Result<(), ()>,
) -> Result<(), &'static str> {
	run_with_identity(
		credentials,
		capture,
		playback,
		controls,
		camera,
		remote_video,
		stream_audio,
		emit,
		Identity::generate(),
	)
	.await
}
/// Run voice media using a call-scoped identity shared with active Go Live streams.
#[allow(clippy::too_many_arguments)] // Every media input of one call plus its identity.
pub async fn run_with_identity(
	credentials: VoiceConnection,
	capture: Receiver<Frame>,
	playback: SyncSender<Frame>,
	controls: watch::Receiver<Controls>,
	camera: Option<Receiver<crate::camera_video::Frame>>,
	remote_video: Option<VideoSink>,
	stream_audio: Option<Receiver<Frame>>,
	emit: impl Fn(Status) -> Result<(), ()>,
	identity: Arc<Identity>,
) -> Result<(), &'static str> {
	let url = endpoint(&credentials.endpoint)?;
	run_inner(
		credentials,
		capture,
		playback,
		controls,
		camera,
		remote_video,
		stream_audio,
		emit,
		identity,
		url,
		false,
	)
	.await
}
#[allow(clippy::too_many_arguments)] // Public media inputs plus the loopback-only test endpoint.
async fn run_inner(
	credentials: VoiceConnection,
	capture: Receiver<Frame>,
	playback: SyncSender<Frame>,
	mut controls: watch::Receiver<Controls>,
	camera: Option<Receiver<crate::camera_video::Frame>>,
	remote_video: Option<VideoSink>,
	stream_audio: Option<Receiver<Frame>>,
	emit: impl Fn(Status) -> Result<(), ()>,
	identity: Arc<Identity>,
	url: String,
	local_test: bool,
) -> Result<(), &'static str> {
	let video_capable = camera.is_some() || remote_video.is_some();
	let (decoder, lost) = match remote_video.map(spawn_decoder).transpose()? {
		Some((decoder, lost)) => (Some(decoder), Some(lost)),
		None => (None, None),
	};
	let mut receivers = Receivers::default();
	let mut next_pli = Instant::now();
	let mut metrics = crate::diagnostics::Metrics::new(crate::diagnostics::Scope::Transport);
	emit(Status::Connecting).map_err(|_| "Call interface closed")?;
	let config = WebSocketConfig::default()
		.max_message_size(Some(MAX_SIGNAL))
		.max_frame_size(Some(MAX_SIGNAL))
		.write_buffer_size(0)
		.max_write_buffer_size(MAX_SIGNAL * 2);
	let (mut ws, _) = timeout(
		Duration::from_secs(15),
		tokio_tungstenite::connect_async_with_config(&url, Some(config), false),
	)
	.await
	.map_err(|_| "Voice connection timed out")?
	.map_err(|_| "Voice TLS connection failed")?;
	// Only voice-scoped credentials go to this validated endpoint; no account Authorization header.
	json_send(&mut ws,json!({"op":0,"d":{"server_id":credentials.guild.unwrap_or(credentials.channel).to_string(),"user_id":credentials.user.to_string(),"session_id":credentials.session.expose(),"token":credentials.token.expose(),"video":video_capable,"streams":if camera.is_some(){vec![json!({"type":"video","rid":"100","quality":100})]}else{vec![]},"max_dave_protocol_version":1}})).await?;
	let mut dave = Dave::with_identity(
		credentials.user.0,
		credentials.peer.map(|peer| peer.0),
		credentials.channel.0,
		identity,
	)?;
	let mut encryption: Option<Encryption> = None;
	// The roster message normally follows the session key within milliseconds; only after
	// this grace does a sole member conclude nobody else is in the call.
	let mut secured_at: Option<Instant> = None;
	let mut udp: Option<UdpSocket> = None;
	let mut next_udp_ping = Instant::now();
	let mut udp_ping_sequence = 0;
	let mut discovering = false;
	let mut discovery_deadline = Instant::now();
	let mut ssrc = 0u32;
	let mut video = crate::camera_video::Sender::default();
	let mut video_tick = tokio::time::interval(Duration::from_millis(2));
	video_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
	let mut mixer = crate::mixer::Mixer::default();
	let mut seq_ack: i64 = -1;
	let mut heartbeat_ms: Option<u64> = None;
	let mut heartbeat_at = Instant::now();
	let mut awaiting_ack = None;
	let mut heartbeat_nonce = 0u64;
	let mut deadline = Some(Instant::now() + Duration::from_secs(90));
	let mut ready_announced = false;
	let mut waiting_announced = false;
	let mut resuming = false;
	let mut resume_attempts = 0u8;
	let mut heard = false;
	let mut speaking = false;
	let mut silence = 0u8;
	// Capture is mono; a mono stream halves Opus work and decodes identically on stereo receivers.
	let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Voip)
		.map_err(|_| "Opus encoder initialization failed")?;
	encoder
		.set_bitrate(Bitrate::Bits(64_000))
		.map_err(|_| "Opus bitrate configuration failed")?;
	let mut random = [0; 6];
	getrandom::fill(&mut random).map_err(|_| "Voice random initialization failed")?;
	let mut sequence = u16::from_be_bytes([random[0], random[1]]);
	let mut timestamp = u32::from_be_bytes(random[2..].try_into().unwrap());
	let mut packet = [0u8; MAX_PACKET + 1];
	let mut encoded = [0u8; 1275];
	let mut mono = [0.0f32; 960];
	let mut tick = tokio::time::interval(Duration::from_millis(20));
	tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
	let mut signal_window = Instant::now();
	let mut signal_count = 0u16;
	let mut capture_pacer = crate::capture::CapturePacer::default();
	let mut capture_at = Instant::now();
	let mut capture_enabled = false;
	let mut capture_reset = true;
	let mut local_activity = 0;
	let mut last_speakers = [0; 64];
	let mut speakers_at = Instant::now();
	loop {
		tokio::select! {
			changed=controls.changed()=>{ if changed.is_err(){return Ok(());} },
			_=video_tick.tick(), if !video.is_empty()=>{
				let generation=controls.borrow().camera;
				if !dave.ready || resuming || generation==0 || generation!=video.generation {video.clear();}
				else if let Some(packet)=video.next() && let Some(socket)=&udp {
					socket.send(&packet).await.map_err(|_|"Camera UDP send failed")?;
				}
			},
			_=tick.tick()=>{
				let now=Instant::now();
				if deadline.is_some_and(|d|now>=d) {return Err(negotiation_timeout(heartbeat_ms.is_some(),udp.is_some(),encryption.is_some(),&dave,resuming));}
				if discovering && now>=discovery_deadline {return Err("Discord voice UDP discovery timed out; check the network firewall");}
				if !discovering && now>=next_udp_ping && let Some(socket)=&udp {
					udp_keepalive(socket,&mut udp_ping_sequence).await?;
					next_udp_ping=now+Duration::from_secs(5);
				}
				if let Some(interval)=heartbeat_ms && now>=heartbeat_at {
					if awaiting_ack.is_some() {return Err("Discord voice heartbeat was not acknowledged; rejoin the call");}
					heartbeat_nonce=heartbeat_nonce.wrapping_add(1);
					json_send(&mut ws,json!({"op":3,"d":{"t":heartbeat_nonce,"seq_ack":seq_ack}})).await?;
					awaiting_ack=Some(heartbeat_nonce);heartbeat_at=now+Duration::from_millis(interval);
				}
				if secured_at.is_some_and(|at| now>=at+PEER_GRACE) && !discovering && !resuming && dave.should_wait_for_peer() {dave.wait_for_peer()?;}
				let enabled=dave.ready && encryption.is_some() && !discovering && !resuming;
				let waiting=dave.waiting && encryption.is_some() && !discovering && !resuming;
				if (!enabled && ready_announced) || (!waiting && waiting_announced) {ready_announced=false;waiting_announced=false;emit(Status::Securing).map_err(|_|"Call interface closed")?;}
				if waiting && !waiting_announced {deadline=None;waiting_announced=true;emit(Status::WaitingForPeer).map_err(|_|"Call interface closed")?;}
				if !waiting {waiting_announced=false;}
				if enabled && !ready_announced {
					deadline=None;ready_announced=true;
					emit(Status::Ready{privacy_code:dave.session.voice_privacy_code().unwrap_or_default().into()}).map_err(|_|"Call interface closed")?;
				}
				// Lost pictures stay frozen until the sender refreshes; ask twice a second at most.
				if enabled && now>=next_pli && let Some(lost)=&lost && let Some(crypto)=encryption.as_mut() && let Some(socket)=&udp {
					receivers.absorb(lost);
					let requests:Vec<u32>=receivers.keyframe_requests().collect();
					for media in requests {let (header,body)=pli(ssrc,media);socket.send(&crypto.seal_rtcp(&header,&body)?).await.map_err(|_|"Voice RTCP send failed")?;}
					next_pli=now+Duration::from_millis(500);
				}
				let control=*controls.borrow();
				let camera_enabled=enabled && video.available() && control.camera!=0;
				if !camera_enabled || video.generation!=control.camera {video.clear();}
				video.generation=control.camera;
				if video.announced && !camera_enabled {
					json_send(&mut ws,video.announcement(ssrc,camera_enabled)).await?;
					video.announced=camera_enabled;
				}
				if let Some(camera)=&camera && let Ok(frame)=camera.try_recv()
					&& camera_enabled && frame.generation==control.camera && video.is_empty()
					&& frame.data.len()<=crate::camera_video::MAX_FRAME_BYTES {
					if !video.announced {
						json_send(&mut ws,video.announcement(ssrc,true)).await?;
						video.announced=true;
					}
					let encrypted=dave.session.encrypt(davey::MediaType::VIDEO,davey::Codec::H264,&frame.data).map_err(|_|"DAVE camera encryption failed")?;
					video.packetize(&encrypted,frame.timestamp,encryption.as_mut().ok_or("Missing camera transport key")?)?;
				}
				// Preserve ordinary callback batches. Only a real stall (four packet
				// intervals) discards queued speech; mute/security gates always flush.
				// Ready can synchronously enqueue the first frame after a delayed tick.
				let resumed_capture = enabled && (!capture_enabled || capture_reset);
				let stalled = !resumed_capture && now.duration_since(capture_at) >= Duration::from_millis(80);
				capture_at = now;
				capture_enabled = enabled;
				capture_reset = false;
				let latest=capture_pacer.next(&capture,enabled && !control.muted && !control.deafened,stalled);
				local_activity=if enabled && !control.muted && !control.deafened && !stalled {
					crate::activity::hold(latest.as_ref().map_or(0.0, |frame| frame.iter().filter(|s| s.is_finite()).map(|s| s*s).sum()),local_activity)
				} else {0};
				let active=enabled && !control.muted && !control.deafened && latest.is_some();
				if active && !speaking {json_send(&mut ws,json!({"op":5,"d":{"speaking":1,"delay":0,"ssrc":ssrc}})).await?;speaking=true;}
				if !active && speaking && silence==0 {silence=5;}
				if enabled && (active || silence>0) {
					let start = metrics.start();
					let data=if active {
						let frame=latest.unwrap();
						for (sample,out) in frame.iter().zip(mono.iter_mut()) {*out=if sample.is_finite(){sample.clamp(-1.0,1.0)}else{0.0};}
						let length=encoder.encode_float(&mono,&mut encoded).map_err(|_|"Opus encoding failed")?;
						dave.session.encrypt_opus(&encoded[..length]).map_err(|_|"DAVE audio encryption failed")?.into_owned()
					} else {silence-=1;davey::OPUS_SILENCE_PACKET.to_vec()};
					let mut header=[0;12];header[0]=0x80;header[1]=120;header[2..4].copy_from_slice(&sequence.to_be_bytes());header[4..8].copy_from_slice(&timestamp.to_be_bytes());header[8..12].copy_from_slice(&ssrc.to_be_bytes());
					let wire=encryption.as_mut().ok_or("Missing voice transport key")?.seal(&header,&data)?;
					metrics.finish(crate::diagnostics::Stage::Encode, start);
					if let Some(socket)=&udp {socket.send(&wire).await.map_err(|_|"Voice UDP send failed")?;}
					sequence=sequence.wrapping_add(1);
					if !active && silence==0 {json_send(&mut ws,json!({"op":5,"d":{"speaking":0,"delay":0,"ssrc":ssrc}})).await?;speaking=false;}
					if active {silence=0;}
				}
				timestamp=timestamp.wrapping_add(960);
				let mut drops = 0;
				if enabled && !control.deafened {
					let start = metrics.start();
					let (mut frame,remote_audio)=mixer.pop_with_volumes(&control.user_volumes);
					// A watched stream's decoded audio joins the same output; one 20ms frame per tick.
					if let Some(aux)=&stream_audio && let Ok(extra)=aux.try_recv() {
						match &mut frame {
							Some(mixed)=>for (out,sample) in mixed.iter_mut().zip(extra.iter()) {*out=(*out+sample).clamp(-1.0,1.0);},
							None=>frame=Some(extra),
						}
					}
					metrics.finish(crate::diagnostics::Stage::Mix, start);
					if let Some(frame)=frame {drops = u64::from(playback.try_send(frame).is_err());}
					if !heard && remote_audio {heard=true;emit(Status::RemoteAudio).map_err(|_|"Call interface closed")?;}
				} else {mixer.clear();if let Some(aux)=&stream_audio {while aux.try_recv().is_ok() {}}}
				metrics.poll(false, drops, stalled, 0);
				if now >= speakers_at {
					let mut users=[0;64];
					users[0]=if local_activity>0 {credentials.user.0} else {0};
					for (slot,user) in users[1..].iter_mut().zip(mixer.speaking()) {*slot=user;}
					if users != last_speakers {
						emit(Status::Speaking(Box::new(users))).map_err(|_|"Call interface closed")?;
						last_speakers=users;
					}
					speakers_at=now+Duration::from_millis(100);
				}
			},
			result=async {match &udp {Some(socket)=>socket.recv(&mut packet).await,None=>std::future::pending().await}}=>{
				let length=result.map_err(|_|"Voice UDP receive failed")?;
				if length>MAX_PACKET {continue;}
				if discovering {
					let (address,port)=discovery(&packet[..length],ssrc)?;
					json_send(&mut ws,json!({"op":1,"d":{"protocol":"udp","data":{"address":address.to_string(),"port":port,"mode":MODE},"codecs":if video_capable{vec![json!({"name":"opus","type":"audio","priority":1000,"payload_type":120}),json!({"name":"H264","type":"video","priority":1000,"payload_type":101,"rtx_payload_type":102,"encode":camera.is_some(),"decode":decoder.is_some()})]}else{vec![json!({"name":"opus","type":"audio","priority":1000,"payload_type":120})]}}})).await?;
					discovering=false;continue;
				}
				let Some(crypto)=&encryption else{continue;};
				let start = metrics.start();
				let Some(rtp)=crypto.open(&packet[..length]) else{continue;};
				if rtp.payload_type==101 {
					let Some(decoder)=&decoder else{continue;};
					if !dave.ready {continue;}
					let Some((user,frame))=receivers.push(rtp.ssrc,rtp.sequence,rtp.timestamp,rtp.marker,&rtp.payload) else{continue;};
					if !dave.contains(user) {continue;}
					let Ok(data)=dave.session.decrypt(user,davey::MediaType::VIDEO,&frame) else{continue;};
					let keyframe=is_keyframe(&data);
					if !receivers.accept(user,keyframe) {continue;}
					if !offer(decoder,Encoded{user,data,keyframe})? {receivers.require_keyframe(user);}
					continue;
				}
				if rtp.payload_type!=120 {continue;}
				let (source,seq,frame)=(rtp.ssrc,rtp.sequence,rtp.payload);
				let Some(user)=mixer.user(source) else{continue;};
				if !dave.ready || !dave.contains(user) || controls.borrow().deafened {continue;}
				let Ok(opus)=dave.session.decrypt(user,davey::MediaType::AUDIO,&frame) else{continue;};
				mixer.push(source,seq,opus);
				metrics.finish(crate::diagnostics::Stage::Receive, start);
			},
			event=ws.next()=>{
				let event=match event {
					// A server crash preserves the voice session. Other close codes remain terminal.
					Some(Ok(message)) if !matches!(&message, Message::Close(Some(frame)) if u16::from(frame.code)==4015)=>message,
					_=>{
						if encryption.is_none() || resume_attempts>=2 {return Err("Voice socket failed; rejoin the call");}
						video.clear();video.announced=false;
						resume_attempts+=1;resuming=true;ready_announced=false;waiting_announced=false;capture_reset=true;
						emit(Status::Securing).map_err(|_|"Call interface closed")?;
						deadline=Some(Instant::now()+Duration::from_secs(30));heartbeat_ms=None;awaiting_ack=None;
						let config=WebSocketConfig::default().max_message_size(Some(MAX_SIGNAL)).max_frame_size(Some(MAX_SIGNAL)).write_buffer_size(0).max_write_buffer_size(MAX_SIGNAL*2);
						let (replacement,_)=timeout(Duration::from_secs(15),tokio_tungstenite::connect_async_with_config(&url,Some(config),false)).await.map_err(|_|"Voice resume timed out; rejoin the call")?.map_err(|_|"Voice resume failed; rejoin the call")?;
						ws=replacement;
						json_send(&mut ws,json!({"op":7,"d":{"server_id":credentials.guild.unwrap_or(credentials.channel).to_string(),"session_id":credentials.session.expose(),"token":credentials.token.expose(),"seq_ack":seq_ack}})).await?;
						continue;
					}
				};
				if signal_window.elapsed()>Duration::from_secs(1){signal_window=Instant::now();signal_count=0;}
				signal_count+=1;if signal_count>256{return Err("Voice signaling exceeded the bounded processing rate");}
				match event {
					Message::Text(text)=>{
						let mut event:Value=serde_json::from_str(&text).map_err(|_|"Invalid voice JSON")?;
						if let Some(seq)=event["seq"].as_i64(){seq_ack=seq;}
						let op=number(&event,"op")?;let data=&mut event["d"];

						match op {
							8=>{
								let interval=data["heartbeat_interval"].as_f64().filter(|v|v.is_finite() && *v>=100.0 && *v<=120_000.0).ok_or("Invalid voice heartbeat interval")? as u64;
								if !(100..=120000).contains(&interval) || heartbeat_ms.is_some(){return Err("Invalid voice heartbeat negotiation");}
								heartbeat_ms=Some(interval.min(5000));heartbeat_at=Instant::now();
							},
							6=>{if awaiting_ack.is_none() || data["t"].as_u64()!=awaiting_ack {return Err("Invalid voice heartbeat acknowledgement");}awaiting_ack=None;},
							2=>{
								if udp.is_some(){return Err("Unexpected voice transport replacement; rejoin the call");}
								ssrc=u32::try_from(number(data,"ssrc")?).map_err(|_|"Invalid voice SSRC")?;
								video.configure(data,ssrc);
								// Video-capable clients announce their audio SSRC even with the camera off.
								if camera.is_some() {json_send(&mut ws,video.announcement(ssrc,false)).await?;}
								let address:IpAddr=data["ip"].as_str().ok_or("Missing voice server address")?.parse().map_err(|_|"Invalid voice server address")?;
								if !public_ip(address) && !(cfg!(test) && local_test && address.is_loopback()){return Err("Voice server advertised a nonpublic address");}
								let port=u16::try_from(number(data,"port")?).ok().filter(|p|*p>0).ok_or("Invalid voice server port")?;
								if !data["modes"].as_array().is_some_and(|m|m.iter().any(|m|m.as_str()==Some(MODE))){return Err("Required voice transport encryption is unavailable");}
								let socket=UdpSocket::bind(if address.is_ipv4(){"0.0.0.0:0"}else{"[::]:0"}).await.map_err(|_|"Could not bind voice UDP socket")?;
								socket.connect(SocketAddr::new(address,port)).await.map_err(|_|"Could not connect voice UDP socket")?;
								let mut probe=[0;74];probe[..4].copy_from_slice(&[0,1,0,70]);probe[4..8].copy_from_slice(&ssrc.to_be_bytes());socket.send(&probe).await.map_err(|_|"Voice UDP discovery failed")?;
								udp=Some(socket);discovering=true;discovery_deadline=Instant::now()+Duration::from_secs(8);
								emit(Status::Discovering).map_err(|_|"Call interface closed")?;
							},
							4=>{
								if encryption.is_some() || udp.is_none() || discovering {return Err("Unexpected voice session description");}
								if data["mode"].as_str()!=Some(MODE) || data["dave_protocol_version"].as_u64()!=Some(1){return Err("Required DAVE version 1 encryption was not negotiated");}
								let values=data["secret_key"].take();let values=values.as_array().ok_or("Missing voice transport key")?;
								if values.len()!=32{return Err("Invalid voice transport key");}
								let mut key=Zeroizing::new([0;32]);for (out,v) in key.iter_mut().zip(values){*out=v.as_u64().and_then(|v|u8::try_from(v).ok()).ok_or("Invalid voice transport key")?;}
								encryption=Some(Encryption::new(&key));secured_at=Some(Instant::now());
								// Register the audio SSRC without opening a microphone or sending media.
								json_send(&mut ws,json!({"op":5,"d":{"speaking":0,"delay":0,"ssrc":ssrc}})).await?;
								video.negotiated=h264_negotiated(data);
								if camera.is_some(){emit(Status::CameraAvailable(video.available())).map_err(|_|"Call interface closed")?;}
								send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;
								emit(Status::TransportReady).map_err(|_|"Call interface closed")?;
								emit(Status::Securing).map_err(|_|"Call interface closed")?;
							},
							5=>{
								let user=id(data,"user_id")?;
								if user!=credentials.user.0 && dave.contains(user) {let value=u32::try_from(number(data,"ssrc")?).map_err(|_|"Invalid voice SSRC")?;mixer.announce(user,value)?;}
							},
							11=>{
								let ids=data["user_ids"].as_array().ok_or("Missing voice participants")?;
								if ids.len()>crate::crypto::MAX_PARTICIPANTS {return Err("Voice channel exceeds the 64 participant limit");}
								let ids=ids.iter().map(|v|v.as_str().and_then(|v|v.parse::<u64>().ok()).filter(|v|*v!=0).ok_or("Malformed voice participant")).collect::<Result<Vec<_>,_>>()?;
								if dave.connect(&ids)? {video.clear();deadline=Some(Instant::now()+Duration::from_secs(90));capture_reset=true;mixer.clear();}
							},
							13=>{
								let user=id(data,"user_id")?;mixer.remove(user);receivers.remove(user);
								if dave.disconnect(user)? {video.clear();deadline=Some(Instant::now()+Duration::from_secs(30));capture_reset=true;mixer.clear();}
							},
							21=>{
								video.clear();
								if number(data,"protocol_version")?!=1 {return Err("Discord requested a voice encryption downgrade; call stopped");}
								capture_reset=true;dave.pending=Some(transition(data)?);
								if dave.pending==Some(0) {if dave.session.is_ready(){dave.execute(0)?;}else if dave.alone(){dave.wait_for_peer()?;}else{dave.pending=None;dave.ready=false;}} else {json_send(&mut ws,json!({"op":23,"d":{"transition_id":dave.pending}})).await?;}
							},
							22=>{video.clear();dave.execute(transition(data)?)?;},
							24=>{
								video.clear();
								if number(data,"protocol_version")?!=1 {return Err("Unsupported DAVE protocol version; call stopped");}
								if number(data,"epoch")?==1 {capture_reset=true;dave.reinitialize()?;deadline=Some(Instant::now()+Duration::from_secs(30));send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;}
							},
							9=>{
								if !resuming{return Err("Unexpected voice resumption");}
								if camera.is_some(){json_send(&mut ws,video.announcement(ssrc,false)).await?;}
								json_send(&mut ws,json!({"op":5,"d":{"speaking":0,"delay":0,"ssrc":ssrc}})).await?;
								speaking=false;silence=0;
								resuming=false;
							},
							12=>{
								let user=id(data,"user_id")?;
								if user!=credentials.user.0 && dave.contains(user) {
									if let Some(value)=data["audio_ssrc"].as_u64().and_then(|v|u32::try_from(v).ok()) {mixer.announce(user,value)?;}
									if decoder.is_some() {announce_video(&mut receivers,user,data)?;}
								}
							},
							14..=20=>{},
							_=>return Err("Unsupported voice signaling opcode; call stopped"),
						}
					},
					Message::Binary(bytes)=>{
						if bytes.len()<3 {return Err("Truncated DAVE signaling");}
						seq_ack=i64::from(u16::from_be_bytes([bytes[0],bytes[1]]));
						let opcode=bytes[2];let data=&bytes[3..];

						match opcode {
							25=>dave.session.set_external_sender(data).map_err(|_|"DAVE external sender validation failed")?,
							27=>{
								if let Some(response)=dave.proposals(data)? {send(&mut ws,Message::Binary(response.into())).await?;}
							},
							29|30=>{
								video.clear();
								ready_announced=false;waiting_announced=false;capture_reset=true;mixer.clear();
								match dave.group_changed(opcode,data){
									Ok(id)=>{if id!=0 {json_send(&mut ws,json!({"op":23,"d":{"transition_id":id}})).await?;}},
									Err(_)=>{
										if data.len()<2 {return Err("Truncated DAVE transition");}
										let id=u16::from_be_bytes([data[0],data[1]]);
										json_send(&mut ws,json!({"op":31,"d":{"transition_id":id}})).await?;
										dave.reset()?;send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;
									}
								}
								if dave.ready {
									deadline=None;
								} else {
									deadline=Some(Instant::now()+Duration::from_secs(30));
									emit(Status::Securing).map_err(|_|"Call interface closed")?;
								}
							},
							_=>return Err("Unsupported DAVE opcode; call stopped"),
						}
					},
					Message::Ping(data)=>send(&mut ws,Message::Pong(data)).await?,
					Message::Close(_)=>return Err("Discord voice connection closed; rejoin the call"),
					Message::Pong(_)=>{},
					_=>return Err("Unsupported voice websocket frame"),
				}
			}
		}
	}
}

struct StreamReady(Arc<AtomicBool>);
impl StreamReady {
	fn new(ready: Arc<AtomicBool>) -> Self {
		ready.store(false, Ordering::Release);
		Self(ready)
	}
	fn set(&self, value: bool) {
		self.0.store(value, Ordering::Release);
	}
}
impl Drop for StreamReady {
	fn drop(&mut self) {
		self.set(false);
	}
}
const STREAM_AUDIO_FRAME: usize = 1920;
// 100 ms stereo PCM, reserved once (38,400 bytes); trim before extending.
const STREAM_AUDIO_PENDING: usize = STREAM_AUDIO_FRAME * 5;

struct StreamAudio {
	encoder: Encoder,
	pending: Vec<f32>,
	speaking: bool,
	last_tick: Instant,
}
impl StreamAudio {
	fn clear(&mut self) {
		self.pending.clear();
		self.speaking = false;
	}
	fn next(
		&mut self,
		source: &mut tokio::sync::mpsc::Receiver<crate::screen::AudioChunk>,
		epoch: u64,
		secure: bool,
		now: Instant,
	) -> Option<[f32; STREAM_AUDIO_FRAME]> {
		let enabled = secure && now.duration_since(self.last_tick) < Duration::from_millis(100);
		self.last_tick = now;
		if !enabled {
			self.clear();
		}
		// Snapshot the queue length: a busy capture producer cannot starve signaling.
		for _ in 0..source.len().min(source.max_capacity()) {
			let Ok(chunk) = source.try_recv() else { break };
			if chunk.epoch != epoch {
				continue;
			}
			let chunk = chunk.samples;
			if !enabled
				|| chunk.is_empty()
				|| chunk.len() > crate::screen::MAX_AUDIO_SAMPLES
				|| !chunk.len().is_multiple_of(2)
				|| !chunk.iter().all(|sample| sample.is_finite())
			{
				continue;
			}
			let chunk = &chunk[chunk.len().saturating_sub(STREAM_AUDIO_PENDING)..];
			let excess = (self.pending.len() + chunk.len()).saturating_sub(STREAM_AUDIO_PENDING);
			self.pending.drain(..excess);
			self.pending
				.extend(chunk.iter().map(|sample| sample.clamp(-1.0, 1.0)));
		}
		if self.pending.len() < STREAM_AUDIO_FRAME {
			return None;
		}
		let frame = std::array::from_fn(|i| self.pending[i]);
		self.pending.drain(..STREAM_AUDIO_FRAME);
		Some(frame)
	}
}

fn soundshare_announcement(audio: &mut Option<StreamAudio>, ssrc: u32) -> Option<Value> {
	let audio = audio.as_mut()?;
	audio.speaking = true;
	Some(json!({"op":5,"d":{"speaking":2,"delay":0,"ssrc":ssrc}}))
}

fn invalidate_stream(video: &mut Option<crate::screen::Video>, audio: &mut Option<StreamAudio>) {
	if let Some(audio) = audio {
		audio.clear();
	}
	if let Some(video) = video {
		video.ready.store(false, Ordering::Release);
		video.audio_epoch.fetch_add(1, Ordering::AcqRel);
		video.keyframe.store(true, Ordering::Release);
		for _ in 0..video.frames.len() {
			let _ = video.frames.try_recv();
		}
		if let Some(source) = &mut video.audio {
			for _ in 0..source.len() {
				let _ = source.try_recv();
			}
		}
	}
}

/// Send one unofficial Discord Go Live H.264 stream on its own voice gateway.
///
/// `credentials.guild` is the stream RTC server ID and `credentials.channel` is
/// its RTC channel ID. Discord has not documented the stream MLS group mapping;
/// the `rtc_server_id - 1` mapping is public implementation evidence only.
pub async fn run_stream(
	credentials: VoiceConnection,
	identity: Arc<Identity>,
	video: crate::screen::Video,
	emit: impl Fn(Status) -> Result<(), ()>,
) -> Result<(), &'static str> {
	let url = endpoint(&credentials.endpoint)?;
	run_stream_inner(
		credentials,
		identity,
		Some(video),
		None,
		None,
		emit,
		url,
		false,
	)
	.await
}
/// Watch another participant's Go Live stream on its own voice gateway; decoded frames
/// reach `sink` from a dedicated thread. Same unofficial identifiers as `run_stream`.
pub async fn watch_stream(
	credentials: VoiceConnection,
	identity: Arc<Identity>,
	sink: VideoSink,
	audio: Option<SyncSender<Frame>>,
	emit: impl Fn(Status) -> Result<(), ()>,
) -> Result<(), &'static str> {
	let url = endpoint(&credentials.endpoint)?;
	run_stream_inner(
		credentials,
		identity,
		None,
		Some(sink),
		audio,
		emit,
		url,
		false,
	)
	.await
}
#[allow(clippy::too_many_arguments)] // Media inputs plus the loopback-only test endpoint.
async fn run_stream_inner(
	credentials: VoiceConnection,
	identity: Arc<Identity>,
	mut video: Option<crate::screen::Video>,
	sink: Option<VideoSink>,
	audio: Option<SyncSender<Frame>>,
	emit: impl Fn(Status) -> Result<(), ()>,
	url: String,
	local_test: bool,
) -> Result<(), &'static str> {
	let (decoder, lost) = match sink.map(spawn_decoder).transpose()? {
		Some((decoder, lost)) => (Some(decoder), Some(lost)),
		None => (None, None),
	};
	let mut receivers = Receivers::default();
	let mut metrics = crate::diagnostics::Metrics::new(if video.is_some() {
		crate::diagnostics::Scope::StreamSend
	} else {
		crate::diagnostics::Scope::StreamReceive
	});
	let mut next_keyframe = Instant::now();
	let mut mixer = crate::mixer::Mixer::default();
	let mut next_pli = Instant::now();
	// Shared system audio: 20 ms stereo Opus frames on the stream's own audio SSRC.
	let mut share_audio = match video.as_ref().and_then(|video| video.audio.as_ref()) {
		Some(_) => {
			let mut encoder = Encoder::new(48_000, Channels::Stereo, Application::Audio)
				.map_err(|_| "Stream audio encoder initialization failed")?;
			encoder
				.set_bitrate(Bitrate::Bits(128_000))
				.map_err(|_| "Stream audio bitrate configuration failed")?;
			Some(StreamAudio {
				encoder,
				pending: Vec::with_capacity(STREAM_AUDIO_PENDING),
				speaking: false,
				last_tick: Instant::now(),
			})
		}
		None => None,
	};
	// Audio has its own RTP sequence space; the video SSRC keeps `sequence`.
	let mut audio_encoded = [0u8; 1275];
	getrandom::fill(&mut audio_encoded[..2]).map_err(|_| "Stream random initialization failed")?;
	let mut audio_sequence = u16::from_be_bytes([audio_encoded[0], audio_encoded[1]]);
	let mut audio_timestamp = 0u32;
	let stream_server = credentials
		.guild
		.ok_or("Stream is missing its RTC server")?
		.0;
	let group = stream_server
		.checked_sub(1)
		.filter(|id| *id != 0)
		.ok_or("Unsupported Discord stream media-session identifier")?;
	let _ready = video
		.as_ref()
		.map(|video| StreamReady::new(video.ready.clone()));
	emit(Status::Connecting).map_err(|_| "Stream interface closed")?;
	let config = WebSocketConfig::default()
		.max_message_size(Some(MAX_SIGNAL))
		.max_frame_size(Some(MAX_SIGNAL))
		.write_buffer_size(0)
		.max_write_buffer_size(MAX_SIGNAL * 2);
	let (mut ws, _) = timeout(
		Duration::from_secs(15),
		tokio_tungstenite::connect_async_with_config(&url, Some(config), false),
	)
	.await
	.map_err(|_| "Stream connection timed out")?
	.map_err(|_| "Stream TLS connection failed")?;
	json_send(
		&mut ws,
		json!({"op":0,"d":{
			"server_id":stream_server.to_string(),"user_id":credentials.user.to_string(),
			"session_id":credentials.session.expose(),"token":credentials.token.expose(),
			"video":true,"streams":if video.is_some(){vec![json!({"type":"video","rid":"100","quality":100})]}else{vec![]},
			"max_dave_protocol_version":1
		}}),
	)
	.await?;
	let mut dave = Dave::with_identity(
		credentials.user.0,
		credentials.peer.map(|id| id.0),
		group,
		identity,
	)?;
	let mut encryption: Option<Encryption> = None;
	let mut secured_at: Option<Instant> = None;
	let mut udp: Option<UdpSocket> = None;
	let mut next_udp_ping = Instant::now();
	let mut udp_ping_sequence = 0;
	let mut discovering = false;
	let mut discovery_deadline = Instant::now();
	let mut audio_ssrc = 0u32;
	let mut video_ssrc = 0u32;
	let mut random = [0; 2];
	getrandom::fill(&mut random).map_err(|_| "Stream random initialization failed")?;
	let mut sequence = u16::from_be_bytes(random);
	let mut heartbeat_ms = None;
	let mut heartbeat_at = Instant::now();
	let mut heartbeat_nonce = 0u64;
	let mut awaiting_ack = None;
	let mut seq_ack = -1i64;
	let mut deadline = Some(Instant::now() + Duration::from_secs(90));
	let mut announced = false;
	let mut waiting_announced = false;
	let mut awaiting_keyframe = true;
	let mut signal_window = Instant::now();
	let mut signal_count = 0u16;
	let mut packet = [0u8; MAX_PACKET + 1];
	let mut tick = tokio::time::interval(Duration::from_millis(20));
	tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
	loop {
		tokio::select! {
			_=tick.tick()=>{
				let now=Instant::now();
				if deadline.is_some_and(|at| now>=at) {return Err(negotiation_timeout(heartbeat_ms.is_some(),udp.is_some(),encryption.is_some(),&dave,false));}
				if discovering && now>=discovery_deadline {return Err("Discord stream UDP discovery timed out");}
				if !discovering && now>=next_udp_ping && let Some(socket)=&udp {
					udp_keepalive(socket,&mut udp_ping_sequence).await?;
					next_udp_ping=now+Duration::from_secs(5);
				}
				if let Some(interval)=heartbeat_ms && now>=heartbeat_at {
					if awaiting_ack.is_some() {return Err("Discord stream heartbeat was not acknowledged");}
					heartbeat_nonce=heartbeat_nonce.wrapping_add(1);
					json_send(&mut ws,json!({"op":3,"d":{"t":heartbeat_nonce,"seq_ack":seq_ack}})).await?;
					awaiting_ack=Some(heartbeat_nonce); heartbeat_at=now+Duration::from_millis(interval);
				}
				if secured_at.is_some_and(|at| now>=at+PEER_GRACE) && !discovering && dave.should_wait_for_peer() {dave.wait_for_peer()?;}
				let waiting=dave.waiting && encryption.is_some() && !discovering;
				if waiting {deadline=None;}
				if waiting!=waiting_announced {
					emit(if waiting {Status::WaitingForPeer} else {Status::Securing}).map_err(|_|"Stream interface closed")?;
					waiting_announced=waiting;
				}
				let secure=dave.ready&&dave.session.is_ready()&&dave.pending.is_none()&&encryption.is_some()&&!discovering;
				if secure && !announced {
					invalidate_stream(&mut video, &mut share_audio);
					if let Some(video)=&video {
						if let Some(event)=soundshare_announcement(&mut share_audio,audio_ssrc) {json_send(&mut ws,event).await?;}
						let streams=json!([{"type":"video","rid":"100","ssrc":video_ssrc,"active":true,"quality":100,"rtx_ssrc":0,"max_bitrate":video.settings.bit_rate(),"max_framerate":video.settings.fps,"max_resolution":{"type":"fixed","width":video.settings.width,"height":video.settings.height}}]);
						json_send(&mut ws,json!({"op":12,"d":{"audio_ssrc":audio_ssrc,"video_ssrc":video_ssrc,"rtx_ssrc":0,"streams":streams}})).await?;
						awaiting_keyframe=true;video.keyframe.store(true, Ordering::Release); video.ready.store(true, Ordering::Release);
					} else {
						json_send(&mut ws,json!({"op":12,"d":{"audio_ssrc":audio_ssrc,"video_ssrc":0,"rtx_ssrc":0,"streams":[]}})).await?;
						json_send(&mut ws,json!({"op":15,"d":{"any":100}})).await?;
						emit(Status::Ready{privacy_code:dave.session.voice_privacy_code().unwrap_or_default().into()}).map_err(|_|"Stream interface closed")?;
					}
					announced=true; deadline=None;
				}
				if !secure && announced {announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);emit(Status::Securing).map_err(|_|"Stream interface closed")?;}
				metrics.stream_state([
					encryption.is_some() && !discovering, dave.ready, dave.session.is_ready(),
					dave.pending.is_some(), waiting, announced,
					video.as_ref().is_some_and(|video|video.ready.load(Ordering::Acquire)),
					share_audio.is_some() || audio.is_some(),
				], video.as_ref().and_then(|video|video.audio.as_ref()).map_or(0,|source|source.len()));
				if let Some(audio)=&audio {
					if secure {
						let start=metrics.start();
						if let (Some(frame),_)=mixer.pop() {
							let dropped=audio.try_send(frame).is_err();
							metrics.finish(crate::diagnostics::Stage::Mix,start);
							metrics.poll(false,u64::from(dropped),false,0);
						}
					} else {mixer.clear();}
				}
				if secure && now>=next_pli && let Some(lost)=&lost && let Some(crypto)=encryption.as_mut() && let Some(socket)=&udp {
					receivers.absorb(lost);
					let requests:Vec<u32>=receivers.keyframe_requests().collect();
					for media in requests {let (header,body)=pli(audio_ssrc,media);socket.send(&crypto.seal_rtcp(&header,&body)?).await.map_err(|_|"Stream RTCP send failed")?;}
					next_pli=now+Duration::from_millis(500);
				}
				if let Some(shared)=&mut share_audio
					&& let Some(video)=video.as_mut()
					&& let Some(source)=video.audio.as_mut()
					&& let Some(frame)=shared.next(source,video.audio_epoch.load(Ordering::Acquire),secure && video.ready.load(Ordering::Acquire),Instant::now()) {
					if !shared.speaking {
						shared.encoder.reset_state().map_err(|_|"Stream audio encoder reset failed")?;
						json_send(&mut ws,json!({"op":5,"d":{"speaking":2,"delay":0,"ssrc":audio_ssrc}})).await?;
						shared.speaking=true;
					}
					let start=metrics.start();
					let length=shared.encoder.encode_float(&frame,&mut audio_encoded).map_err(|_|"Stream audio encoding failed")?;
					let data=dave.session.encrypt_opus(&audio_encoded[..length]).map_err(|_|"DAVE stream audio encryption failed")?.into_owned();
					let mut header=[0;12];header[0]=0x80;header[1]=120;header[2..4].copy_from_slice(&audio_sequence.to_be_bytes());header[4..8].copy_from_slice(&audio_timestamp.to_be_bytes());header[8..12].copy_from_slice(&audio_ssrc.to_be_bytes());
					let crypto=encryption.as_mut().ok_or("Missing stream transport key")?;
					let socket=udp.as_ref().ok_or("Missing stream UDP socket")?;
					socket.send(&crypto.seal_soundshare(&header,&data)?).await.map_err(|_|"Stream audio UDP send failed")?;
					metrics.finish(crate::diagnostics::Stage::Encode,start);
					audio_sequence=audio_sequence.wrapping_add(1);
				}
				audio_timestamp=audio_timestamp.wrapping_add(960);
				metrics.poll(false,0,false,0);
			},
			frame=async {match video.as_mut() {Some(video)=>video.frames.recv().await,None=>std::future::pending().await}}=>{
				let Some(frame)=frame else {return Ok(());};
				if frame.data.len()>2*1024*1024 {return Err("Encoded stream frame exceeds the sharing limit");}
				let secure=announced&&dave.ready&&dave.session.is_ready()&&dave.pending.is_none()&&encryption.is_some()&&!discovering&&video.as_ref().is_some_and(|video|video.ready.load(Ordering::Acquire));
				if !secure {awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);continue;}
				if awaiting_keyframe && !frame.keyframe {continue;}
				let start=metrics.start();
				crate::video::validate_source(&frame.data)?;
				let encrypted=dave.session.encrypt(davey::MediaType::VIDEO,davey::Codec::H264,&frame.data).map_err(|_|"DAVE H264 encryption failed")?;
				let packets=crate::video::packetize(&encrypted,&mut sequence,frame.timestamp,video_ssrc)?;
				let crypto=encryption.as_mut().ok_or("Missing stream transport key")?;
				let socket=udp.as_ref().ok_or("Missing stream UDP socket")?;
				for (index, packet) in packets.into_iter().enumerate() {
					socket.send(&crypto.seal(&packet.header,&packet.payload)?).await.map_err(|_|"Stream UDP send failed")?;
					if index % 32 == 31 {tokio::task::yield_now().await;}
				}
				metrics.finish(crate::diagnostics::Stage::VideoSend,start);
				if frame.keyframe {
					if awaiting_keyframe {emit(Status::Ready{privacy_code:dave.session.voice_privacy_code().unwrap_or_default().into()}).map_err(|_|"Stream interface closed")?;}
					awaiting_keyframe=false;
				}
			},
			result=async {match &udp {Some(socket)=>socket.recv(&mut packet).await,_=>std::future::pending().await}}=>{
				let length=result.map_err(|_|"Stream UDP receive failed")?;
				if discovering {
					let (address,port)=discovery(&packet[..length],audio_ssrc)?;
					json_send(&mut ws,json!({"op":1,"d":{"protocol":"udp","data":{"address":address.to_string(),"port":port,"mode":MODE},"codecs":[{"name":"opus","type":"audio","priority":1000,"payload_type":120},{"name":"H264","type":"video","priority":1000,"payload_type":101,"rtx_payload_type":102,"encode":video.is_some(),"decode":decoder.is_some()}]}})).await?;
					discovering=false;
					continue;
				}
				if length>MAX_PACKET {continue;}
				let Some(crypto)=&encryption else {continue;};
				if let Some(video)=&video && Instant::now()>=next_keyframe && crypto.requests_keyframe(&packet[..length],video_ssrc) {
					video.keyframe.store(true,Ordering::Release);
					next_keyframe=Instant::now()+Duration::from_millis(500);
					continue;
				}
				let Some(rtp)=crypto.open(&packet[..length]) else {continue;};
				if !dave.ready {continue;}
				if rtp.payload_type==120 {
					if audio.is_none() {continue;}
					let Some(user)=mixer.user(rtp.ssrc) else {continue;};
					if !dave.contains(user) {continue;}
					let start=metrics.start();
					let Ok(opus)=dave.session.decrypt(user,davey::MediaType::AUDIO,&rtp.payload) else {metrics.poll(false,1,false,0);continue;};
					metrics.finish(crate::diagnostics::Stage::Receive,start);
					mixer.push(rtp.ssrc,rtp.sequence,opus);
					continue;
				}
				let Some(decoder)=&decoder else {continue;};
				if rtp.payload_type!=101 {continue;}
				let Some((user,frame))=receivers.push(rtp.ssrc,rtp.sequence,rtp.timestamp,rtp.marker,&rtp.payload) else {continue;};
				if !dave.contains(user) {continue;}
				let start=metrics.start();
				let Ok(data)=dave.session.decrypt(user,davey::MediaType::VIDEO,&frame) else {metrics.poll(false,1,false,0);continue;};
				let keyframe=is_keyframe(&data);
				if !receivers.accept(user,keyframe) {continue;}
				if !offer(decoder,Encoded{user,data,keyframe})? {receivers.require_keyframe(user);metrics.poll(false,1,false,0);} else {metrics.finish(crate::diagnostics::Stage::VideoReceive,start);}
			},
			event=ws.next()=>{
				let Some(Ok(event))=event else {return Err("Discord stream socket failed");};
				if signal_window.elapsed()>Duration::from_secs(1){signal_window=Instant::now();signal_count=0;}
				signal_count+=1;if signal_count>256{return Err("Stream signaling exceeded the bounded processing rate");}
				match event {
					Message::Text(text)=>{
						let mut event:Value=serde_json::from_str(&text).map_err(|_|"Invalid stream JSON")?;
						if let Some(seq)=event["seq"].as_i64(){seq_ack=seq;}
						let op=number(&event,"op")?;
						let data=&mut event["d"];
						match op {
							8=>{let interval=data["heartbeat_interval"].as_f64().filter(|v|v.is_finite()&&*v>=100.&&*v<=120000.).ok_or("Invalid stream heartbeat")? as u64; heartbeat_ms=Some(interval.min(5000));heartbeat_at=Instant::now();},
							6=>{if awaiting_ack.is_none()||data["t"].as_u64()!=awaiting_ack{return Err("Invalid stream heartbeat acknowledgement");}awaiting_ack=None;},
							2=>{
								if udp.is_some(){return Err("Unexpected stream transport replacement");}
								audio_ssrc=u32::try_from(number(data,"ssrc")?).map_err(|_|"Invalid stream SSRC")?;
								if video.is_some() {
									let stream=data["streams"].as_array().and_then(|v|v.first()).ok_or("Discord did not assign a stream SSRC")?;
									video_ssrc=u32::try_from(number(stream,"ssrc")?).map_err(|_|"Invalid stream video SSRC")?;
								}
								let address:IpAddr=data["ip"].as_str().ok_or("Missing stream server address")?.parse().map_err(|_|"Invalid stream server address")?;
								if !public_ip(address) && !(cfg!(test) && local_test && address.is_loopback()){return Err("Stream server advertised a nonpublic address");}
								let port=u16::try_from(number(data,"port")?).ok().filter(|port|*port>0).ok_or("Invalid stream server port")?;
								if !data["modes"].as_array().is_some_and(|m|m.iter().any(|mode|mode.as_str()==Some(MODE))){return Err("Required stream transport encryption is unavailable");}
								let socket=UdpSocket::bind(if address.is_ipv4(){"0.0.0.0:0"}else{"[::]:0"}).await.map_err(|_|"Could not bind stream UDP socket")?;
								socket.connect(SocketAddr::new(address,port)).await.map_err(|_|"Could not connect stream UDP socket")?;
								let mut probe=[0;74];probe[..4].copy_from_slice(&[0,1,0,70]);probe[4..8].copy_from_slice(&audio_ssrc.to_be_bytes());socket.send(&probe).await.map_err(|_|"Stream UDP discovery failed")?;
								udp=Some(socket);discovering=true;discovery_deadline=Instant::now()+Duration::from_secs(8);emit(Status::Discovering).map_err(|_|"Stream interface closed")?;
							},
							4=>{
								if encryption.is_some()||udp.is_none()||discovering{return Err("Unexpected stream session description");}
								if data["mode"].as_str()!=Some(MODE)||data["dave_protocol_version"].as_u64()!=Some(1)||!h264_negotiated(data){return Err("Discord did not negotiate DAVE H264 stream media");}
								let values=data["secret_key"].take();let values=values.as_array().ok_or("Missing stream transport key")?;if values.len()!=32{return Err("Invalid stream transport key");}
								let mut key=Zeroizing::new([0;32]);for(out,value)in key.iter_mut().zip(values){*out=value.as_u64().and_then(|value|u8::try_from(value).ok()).ok_or("Invalid stream transport key")?;}
								encryption=Some(Encryption::new(&key));secured_at=Some(Instant::now());send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;emit(Status::TransportReady).map_err(|_|"Stream interface closed")?;emit(Status::Securing).map_err(|_|"Stream interface closed")?;
							},
							11=>{let ids=data["user_ids"].as_array().ok_or("Missing stream participants")?;if ids.len()>crate::crypto::MAX_PARTICIPANTS{return Err("Too many stream participants");}let ids=ids.iter().map(|value|value.as_str().and_then(|value|value.parse().ok()).filter(|id|*id!=0).ok_or("Malformed stream participant")).collect::<Result<Vec<_>,_>>()?;if dave.connect(&ids)?{deadline=Some(Instant::now()+Duration::from_secs(90));announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);}},
							5=>{let user=id(data,"user_id")?;if user!=credentials.user.0 && audio.is_some() && dave.contains(user) {let value=u32::try_from(number(data,"ssrc")?).map_err(|_|"Invalid stream SSRC")?;mixer.announce(user,value)?;}},
							12=>{
								let user=id(data,"user_id")?;
								if user!=credentials.user.0 && dave.contains(user) {
									if decoder.is_some() {announce_video(&mut receivers,user,data)?;}
									if audio.is_some() && let Some(value)=data["audio_ssrc"].as_u64().and_then(|v|u32::try_from(v).ok()).filter(|v|*v!=0) {mixer.announce(user,value)?;}
								}
							},
							13=>{let user=id(data,"user_id")?;receivers.remove(user);mixer.remove(user);if dave.disconnect(user)?{deadline=Some(Instant::now()+Duration::from_secs(30));announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);}},
							21=>{if number(data,"protocol_version")?!=1{return Err("Discord requested a stream encryption downgrade");}announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);dave.pending=Some(transition(data)?);if dave.pending==Some(0){if dave.session.is_ready(){dave.execute(0)?;}else if dave.alone(){dave.wait_for_peer()?;}else{dave.pending=None;dave.ready=false;}}else{json_send(&mut ws,json!({"op":23,"d":{"transition_id":dave.pending}})).await?;}},
							22=>{dave.execute(transition(data)?)?;},
							24=>{if number(data,"protocol_version")?!=1{return Err("Unsupported stream DAVE version");}
							if number(data,"epoch")?==1{announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);dave.reinitialize()?;send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;}},
							// Watching a stream receives signaling the sender never does; unknown
							// opcodes are ignored under the bounded rate above, never fatal.
							_=>{},
						}
					},
					Message::Binary(bytes)=>{if bytes.len()<3{return Err("Truncated stream DAVE signaling");}seq_ack=i64::from(u16::from_be_bytes([bytes[0],bytes[1]]));match bytes[2]{25=>dave.session.set_external_sender(&bytes[3..]).map_err(|_|"Stream DAVE external sender validation failed")?,27=>if let Some(response)=dave.proposals(&bytes[3..])?{send(&mut ws,Message::Binary(response.into())).await?;},29|30=>{announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);match dave.group_changed(bytes[2],&bytes[3..]){Ok(id)=>if id!=0{json_send(&mut ws,json!({"op":23,"d":{"transition_id":id}})).await?;},Err(_)=>{if bytes.len()<5{return Err("Truncated stream DAVE transition");}let id=u16::from_be_bytes([bytes[3],bytes[4]]);json_send(&mut ws,json!({"op":31,"d":{"transition_id":id}})).await?;dave.reset()?;send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;}}},_=>return Err("Unsupported stream DAVE opcode")}},
					Message::Ping(data)=>send(&mut ws,Message::Pong(data)).await?, Message::Close(_)=>return Err("Discord stream connection closed"), _=>{}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use opus2::Decoder;
	async fn receive_media(socket: &UdpSocket, packet: &mut [u8]) -> (usize, SocketAddr) {
		loop {
			let received = socket.recv_from(packet).await.unwrap();
			if received.0 != 8 {
				return received;
			}
			assert_eq!(&packet[..4], &[0x13, 0x37, 0xca, 0xfe]);
		}
	}
	#[test]
	fn stream_audio_is_bounded_paced_and_cleared_on_rekey_or_stall() {
		let start = Instant::now();
		let mut shared = StreamAudio {
			encoder: Encoder::new(48_000, Channels::Stereo, Application::Audio).unwrap(),
			pending: Vec::with_capacity(STREAM_AUDIO_PENDING),
			speaking: true,
			last_tick: start,
		};
		let (send, mut receive) = tokio::sync::mpsc::channel(16);
		let enqueue = |samples| {
			send.try_send(crate::screen::AudioChunk { samples, epoch: 0 })
				.unwrap()
		};
		// Preserve a batched callback in 20 ms frames instead of sending a burst.
		enqueue(
			[
				vec![0.25; STREAM_AUDIO_FRAME],
				vec![0.5; STREAM_AUDIO_FRAME],
			]
			.concat(),
		);
		assert_eq!(
			shared.next(&mut receive, 0, true, start).unwrap(),
			[0.25; STREAM_AUDIO_FRAME]
		);
		assert_eq!(
			shared
				.next(&mut receive, 0, true, start + Duration::from_millis(20))
				.unwrap(),
			[0.5; STREAM_AUDIO_FRAME]
		);
		assert!(
			shared
				.next(&mut receive, 0, true, start + Duration::from_millis(40))
				.is_none()
		);
		// Reject invalid PCM before it can reach Opus or grow the pending allocation.
		for chunk in [
			vec![0.0; 3],
			vec![f32::NAN; 2],
			vec![f32::INFINITY; 2],
			vec![0.0; crate::screen::MAX_AUDIO_SAMPLES + 2],
		] {
			enqueue(chunk);
		}
		assert!(
			shared
				.next(&mut receive, 0, true, start + Duration::from_millis(60))
				.is_none()
		);
		for _ in 0..16 {
			enqueue(vec![2.0; crate::screen::MAX_AUDIO_SAMPLES]);
		}
		assert_eq!(
			shared
				.next(&mut receive, 0, true, start + Duration::from_millis(80))
				.unwrap(),
			[1.0; STREAM_AUDIO_FRAME]
		);
		assert_eq!(
			shared.pending.len(),
			STREAM_AUDIO_PENDING - STREAM_AUDIO_FRAME
		);
		assert_eq!(shared.pending.capacity(), STREAM_AUDIO_PENDING);
		enqueue(vec![0.75; STREAM_AUDIO_FRAME]);
		assert!(
			shared
				.next(&mut receive, 0, true, start + Duration::from_millis(180))
				.is_none()
		);
		assert!(shared.pending.is_empty());
		assert!(receive.is_empty());
		assert!(!shared.speaking);
		enqueue(vec![0.75; STREAM_AUDIO_FRAME]);
		assert!(
			shared
				.next(&mut receive, 0, false, start + Duration::from_millis(200))
				.is_none()
		);
		assert!(receive.is_empty());
		// No tick is required between invalidation and the next secure epoch.
		shared
			.pending
			.extend_from_slice(&[0.75; STREAM_AUDIO_FRAME]);
		shared.speaking = true;
		enqueue(vec![0.75; STREAM_AUDIO_FRAME]);
		let (_, frames) = tokio::sync::mpsc::channel(3);
		let mut video = Some(crate::screen::Video {
			settings: crate::screen::Settings {
				source: crate::screen::SourceId::Display(1),
				width: 1280,
				height: 720,
				fps: 30,
				cursor: true,
				audio: true,
			},
			frames,
			ready: Arc::new(AtomicBool::new(true)),
			keyframe: Arc::new(AtomicBool::new(false)),
			audio_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
			audio: Some(receive),
		});
		let mut shared = Some(shared);
		invalidate_stream(&mut video, &mut shared);
		let video = video.as_mut().unwrap();
		assert!(!video.ready.load(Ordering::Acquire));
		assert!(video.keyframe.load(Ordering::Acquire));
		let shared = shared.as_mut().unwrap();
		assert!(shared.pending.is_empty());
		assert!(!shared.speaking);
		assert!(video.audio.as_ref().unwrap().is_empty());
		let epoch = video.audio_epoch.load(Ordering::Acquire);
		assert_eq!(epoch, 1);
		// An in-flight capture can finish after the flush and readiness resumes.
		video.ready.store(true, Ordering::Release);
		enqueue(vec![0.75; STREAM_AUDIO_FRAME]);
		assert!(
			shared
				.next(
					video.audio.as_mut().unwrap(),
					epoch,
					true,
					start + Duration::from_millis(220)
				)
				.is_none()
		);
		send.try_send(crate::screen::AudioChunk {
			samples: vec![0.25; STREAM_AUDIO_FRAME],
			epoch,
		})
		.unwrap();
		assert_eq!(
			shared
				.next(
					video.audio.as_mut().unwrap(),
					epoch,
					true,
					start + Duration::from_millis(240)
				)
				.unwrap(),
			[0.25; STREAM_AUDIO_FRAME]
		);
	}
	#[test]
	fn soundshare_is_announced_before_captured_audio_is_enabled() {
		let mut audio = Some(StreamAudio {
			encoder: Encoder::new(48_000, Channels::Stereo, Application::Audio).unwrap(),
			pending: Vec::with_capacity(STREAM_AUDIO_PENDING),
			speaking: false,
			last_tick: Instant::now(),
		});
		let event = soundshare_announcement(&mut audio, 42).unwrap();
		assert_eq!(
			event,
			json!({"op":5,"d":{"speaking":2,"delay":0,"ssrc":42}})
		);
		assert!(audio.unwrap().speaking);
	}
	#[test]
	fn negotiation_timeout_distinguishes_missing_group_from_unexecuted_transition() {
		let server = crate::test_mls::Delivery::new();
		let mut alice = Dave::new(1, Some(2), 3).unwrap();
		let mut bob = Dave::new(2, Some(1), 3).unwrap();
		alice.session.set_external_sender(&server.external).unwrap();
		bob.session.set_external_sender(&server.external).unwrap();
		assert_eq!(
			negotiation_timeout(false, false, false, &alice, false),
			"Discord voice Hello timed out; rejoin the call"
		);
		assert_eq!(
			negotiation_timeout(true, false, false, &alice, false),
			"Discord voice Ready timed out; rejoin the call"
		);
		assert_eq!(
			negotiation_timeout(true, true, false, &alice, false),
			"Discord voice protocol selection timed out; no transport key was received"
		);
		assert_eq!(
			negotiation_timeout(true, true, true, &alice, false),
			"Discord DAVE group negotiation timed out; no accepted commit or welcome was received"
		);
		let (_, welcome) = server.add(&mut bob, &alice.key_package().unwrap());
		alice
			.group_changed(30, &[&[0, 7], welcome.as_slice()].concat())
			.unwrap();
		assert!(!alice.ready);
		assert_eq!(
			negotiation_timeout(true, true, true, &alice, false),
			"Discord DAVE transition execution timed out; no audio was enabled"
		);
		assert_eq!(
			negotiation_timeout(false, true, true, &alice, true),
			"Discord voice resume acknowledgement timed out; rejoin the call"
		);
	}
	#[test]
	fn requires_h264_in_the_session_description() {
		assert!(h264_negotiated(&json!({"video_codec":"H264"})));
		assert!(!h264_negotiated(&json!({"video_codec":"VP8"})));
		assert!(!h264_negotiated(
			&json!({"sdp":"a=rtpmap:101 H264/90000\\r\\n"})
		));
	}
	#[test]
	fn validated_endpoints_discovery_and_real_opus() {
		assert!(endpoint("voice-1.discord.media:443").is_ok());
		for bad in [
			"127.0.0.1",
			"voice.discord.media.evil.test",
			"voice.discord.media/path",
			"token@voice.discord.media",
			"voice.discord.media?token=x",
		] {
			assert!(endpoint(bad).is_err());
		}
		assert!(!public_ip("127.0.0.1".parse().unwrap()));
		assert!(!public_ip("192.168.1.1".parse().unwrap()));
		let mut reply = [0; 74];
		reply[..4].copy_from_slice(&[0, 2, 0, 70]);
		reply[4..8].copy_from_slice(&42u32.to_be_bytes());
		reply[8..17].copy_from_slice(b"127.0.0.1");
		reply[72..].copy_from_slice(&1234u16.to_be_bytes());
		assert_eq!(discovery(&reply, 42).unwrap().1, 1234);
		assert!(discovery(&reply, 43).is_err());
		let mut encoder = Encoder::new(48_000, Channels::Stereo, Application::Voip).unwrap();
		let mut decoder = Decoder::new(48_000, Channels::Mono).unwrap();
		let input = std::array::from_fn::<_, 1920, _>(|i| ((i / 2) as f32 * 0.06).sin() * 0.2);
		let mut encoded = [0; 1275];
		let length = encoder.encode_float(&input, &mut encoded).unwrap();
		let mut output = [0.0; 960];
		assert_eq!(
			decoder
				.decode_float(&encoded[..length], &mut output, false)
				.unwrap(),
			960
		);
		assert!(output.iter().any(|sample| sample.abs() > 0.01));
	}
	#[tokio::test]
	async fn local_voice_websocket_udp_dave_and_opus_exchange() {
		local_voice_exchange(false).await;
	}
	#[tokio::test]
	async fn local_guild_voice_waiting_mixed_audio_and_resume() {
		local_voice_exchange(true).await;
	}
	async fn local_voice_exchange(guild: bool) {
		use client_core::voice::Secret;
		use model::Id;
		use tokio::net::TcpListener;
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
		let port = udp.local_addr().unwrap().port();
		let (done_tx, done_rx) = tokio::sync::oneshot::channel();
		let (captured_tx, mut captured_rx) = tokio::sync::oneshot::channel();
		let (waiting_tx, waiting_rx) = tokio::sync::oneshot::channel();
		let (heard_tx, heard_rx) = tokio::sync::oneshot::channel();
		let (resumed_tx, resumed_rx) = tokio::sync::oneshot::channel();
		let server = tokio::spawn(async move {
			let (tcp, _) = listener.accept().await.unwrap();
			let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
			let identify = ws.next().await.unwrap().unwrap().into_text().unwrap();
			let identify: Value = serde_json::from_str(&identify).unwrap();
			assert_eq!(identify["op"], 0);
			assert_eq!(identify["d"]["server_id"], if guild { "30" } else { "3" });
			assert_eq!(identify["d"]["max_dave_protocol_version"], 1);
			assert_eq!(identify["d"]["video"], true);
			ws.send(Message::Text(
				json!({"op":8,"d":{"heartbeat_interval":5000}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();
			ws.send(Message::Text(
				json!({"op":2,"d":{"ssrc":42,"ip":"127.0.0.1","port":port,"modes":[MODE]}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();
			loop {
				let event: Value =
					serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap())
						.unwrap();
				if event["op"] == 3 {
					ws.send(Message::Text(
						json!({"op":6,"d":{"t":event["d"]["t"]}}).to_string().into(),
					))
					.await
					.unwrap();
					continue;
				}
				assert_eq!(event["op"], 12);
				assert_eq!(event["d"]["audio_ssrc"], 42);
				assert_eq!(event["d"]["video_ssrc"], 0);
				assert_eq!(event["d"]["streams"], json!([]));
				break;
			}
			let mut probe = [0; 4096];
			let (n, client) = udp.recv_from(&mut probe).await.unwrap();
			assert_eq!(n, 74);
			probe[..4].copy_from_slice(&[0, 2, 0, 70]);
			probe[8..17].copy_from_slice(b"127.0.0.1");
			probe[72..74].copy_from_slice(&client.port().to_be_bytes());
			udp.send_to(&probe[..74], client).await.unwrap();
			let delivery = crate::test_mls::Delivery::new();
			let mut bob = Dave::new(2, (!guild).then_some(1), 3).unwrap();
			bob.session.set_external_sender(&delivery.external).unwrap();
			let mut charlie = Dave::new(4, None, 3).unwrap();
			if guild {
				bob.connect(&[1, 2, 4]).unwrap();
				charlie
					.session
					.set_external_sender(&delivery.external)
					.unwrap();
				charlie.connect(&[1, 2, 4]).unwrap();
				let (commit, welcome) = delivery.add(&mut bob, &charlie.key_package().unwrap());
				bob.group_changed(29, &[&[0, 0], commit.as_slice()].concat())
					.unwrap();
				charlie
					.group_changed(30, &[&[0, 0], welcome.as_slice()].concat())
					.unwrap();
			}
			loop {
				let message = ws.next().await.unwrap().unwrap();
				let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
				if event["op"] == 3 {
					ws.send(Message::Text(
						json!({"op":6,"d":{"t":event["d"]["t"]}}).to_string().into(),
					))
					.await
					.unwrap();
					continue;
				}
				assert_eq!(event["op"], 1);
				assert_eq!(event["d"]["data"]["mode"], MODE);
				break;
			}
			let mut external = vec![0, 1, 25];
			external.extend(&delivery.external);
			ws.send(Message::Binary(external.into())).await.unwrap();
			ws.send(Message::Text(
				json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":1}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();
			if !guild {
				// Discord announces the already connected DM peer to the joiner before any welcome.
				ws.send(Message::Text(
					json!({"op":11,"d":{"user_ids":["1","2"]}})
						.to_string()
						.into(),
				))
				.await
				.unwrap();
			}
			let mut audio_announced = false;
			let package = loop {
				match ws.next().await.unwrap().unwrap() {
					Message::Binary(bytes) => {
						assert!(audio_announced);
						break bytes;
					}
					Message::Text(text) => {
						let value: Value = serde_json::from_str(&text).unwrap();
						if value["op"] == 5 {
							assert_eq!(value["d"]["speaking"], 0);
							assert_eq!(value["d"]["ssrc"], 42);
							audio_announced = true;
							continue;
						}
						assert_eq!(value["op"], 3);
						ws.send(Message::Text(
							json!({"op":6,"d":{"t":value["d"]["t"]}}).to_string().into(),
						))
						.await
						.unwrap();
					}
					_ => panic!("unexpected test client frame"),
				}
			};
			assert_eq!(package[0], 26);
			if guild {
				ws.send(Message::Text(
					json!({"op":21,"d":{"protocol_version":1,"transition_id":0}})
						.to_string()
						.into(),
				))
				.await
				.unwrap();
				waiting_rx.await.unwrap();
				// Even queued synthetic capture cannot leave while the empty room lacks media keys.
				assert!(
					timeout(Duration::from_millis(80), receive_media(&udp, &mut probe))
						.await
						.is_err()
				);
				ws.send(Message::Text(
					json!({"op":11,"d":{"user_ids":["1","2","4"]}})
						.to_string()
						.into(),
				))
				.await
				.unwrap();
				let proposal = delivery.add_proposal(&bob, &package);
				charlie.proposals(&proposal).unwrap();
			}
			let (commit, welcome) = delivery.add(&mut bob, &package);
			let mut committed = vec![0, 0];
			committed.extend(commit);
			bob.group_changed(29, &committed).unwrap();
			if guild {
				charlie.group_changed(29, &committed).unwrap();
			}
			assert!(bob.ready);
			let mut welcome_frame = vec![0, 2, 30, 0, 0];
			welcome_frame.extend(welcome);
			ws.send(Message::Binary(welcome_frame.into()))
				.await
				.unwrap();
			// Exercise media readiness after a scheduling gap while the fixture
			// supplies continuous capture and fences signaling before UDP.
			std::thread::sleep(Duration::from_millis(100));
			ws.send(Message::Text(
				json!({"op":5,"seq":3,"d":{"user_id":"2","ssrc":43,"speaking":1}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();
			if guild {
				ws.send(Message::Text(
					json!({"op":5,"seq":4,"d":{"user_id":"4","ssrc":44,"speaking":1}})
						.to_string()
						.into(),
				))
				.await
				.unwrap();
			}
			// Fence the SSRC announcements before UDP can race ahead of signaling.
			ws.send(Message::Ping(b"announced".to_vec().into()))
				.await
				.unwrap();
			loop {
				match ws.next().await.unwrap().unwrap() {
					Message::Pong(data) if data.as_ref() == b"announced" => break,
					Message::Text(text) => {
						let event: Value = serde_json::from_str(&text).unwrap();
						if event["op"] == 3 {
							ws.send(Message::Text(
								json!({"op":6,"d":{"t":event["d"]["t"]}}).to_string().into(),
							))
							.await
							.unwrap();
						} else {
							assert_eq!(event["op"], 5);
						}
					}
					_ => panic!("unexpected test client frame before SSRC acknowledgement"),
				}
			}
			let (length, _) = receive_media(&udp, &mut probe).await;
			let mut transport = Encryption::new(&[7; 32]);
			let opened = transport.open(&probe[..length]).unwrap();
			let (ssrc, ciphertext) = (opened.ssrc, opened.payload);
			assert_eq!(ssrc, 42);
			let encoded = bob
				.session
				.decrypt(1, davey::MediaType::AUDIO, &ciphertext)
				.unwrap();
			let mut decoder = Decoder::new(48_000, Channels::Mono).unwrap();
			let mut out = [0.0; 960];
			assert_eq!(
				decoder.decode_float(&encoded, &mut out, false).unwrap(),
				960
			);
			assert!(out.iter().any(|s| s.abs() > 0.01));
			captured_tx.send(()).unwrap();
			let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Voip).unwrap();
			let mut encoded = [0; 1275];
			let length = encoder.encode_float(&out, &mut encoded).unwrap();
			let encrypted = bob.session.encrypt_opus(&encoded[..length]).unwrap();
			let header = [0x80, 120, 0, 1, 0, 0, 0, 1, 0, 0, 0, 43];
			let packet = transport.seal(&header, &encrypted).unwrap();
			udp.send_to(&packet, client).await.unwrap();
			if guild {
				let encrypted = charlie.session.encrypt_opus(&encoded[..length]).unwrap();
				let mut header = header;
				header[11] = 44;
				let packet = transport.seal(&header, &encrypted).unwrap();
				udp.send_to(&packet, client).await.unwrap();
			}
			// Resume only after the client has actually played the initial remote audio.
			heard_rx.await.unwrap();
			if guild {
				ws.send(Message::Close(Some(
					tokio_tungstenite::tungstenite::protocol::CloseFrame {
						code: 4015.into(),
						reason: "synthetic server restart".into(),
					},
				)))
				.await
				.unwrap();
			}
			drop(ws);
			let (tcp, _) = listener.accept().await.unwrap();
			let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
			let resume: Value =
				serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
			assert_eq!(resume["op"], 7);
			assert_eq!(resume["d"]["seq_ack"], if guild { 4 } else { 3 });
			assert_eq!(resume["d"]["server_id"], if guild { "30" } else { "3" });
			ws.send(Message::Text(
				json!({"op":8,"d":{"heartbeat_interval":5000}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();
			ws.send(Message::Text(json!({"op":9,"d":null}).to_string().into()))
				.await
				.unwrap();
			resumed_rx.await.unwrap();
			// A new encrypted frame after resume reuses the live MLS group, never its old nonce.
			let encrypted = bob.session.encrypt_opus(&encoded[..length]).unwrap();
			let mut header = header;
			header[3] = 2;
			let packet = transport.seal(&header, &encrypted).unwrap();
			udp.send_to(&packet, client).await.unwrap();
			done_rx.await.unwrap();
			if guild {
				ws.send(Message::Close(Some(
					tokio_tungstenite::tungstenite::protocol::CloseFrame {
						code: 4014.into(),
						reason: "synthetic terminal disconnect".into(),
					},
				)))
				.await
				.unwrap();
				// A terminal disconnect must never attempt another connection.
				assert!(
					timeout(Duration::from_millis(100), listener.accept())
						.await
						.is_err()
				);
			}
		});
		let credentials = VoiceConnection {
			channel: Id(3),
			user: Id(1),
			peer: (!guild).then_some(Id(2)),
			guild: guild.then_some(Id(30)),
			session: Secret::new("synthetic-session".into()).unwrap(),
			token: Secret::new("synthetic-token".into()).unwrap(),
			endpoint: "not-used-in-test".into(),
			request: 1,
		};
		let (capture_tx, capture) = std::sync::mpsc::sync_channel(8);
		capture_tx.try_send([0.25; 960]).unwrap();
		let (playback, playback_rx) = std::sync::mpsc::sync_channel(8);
		let (control_tx, control_rx) = watch::channel(Controls::default());
		let (_camera_tx, camera_rx) = std::sync::mpsc::sync_channel(1);
		let (status_tx, mut status_rx) = tokio::sync::mpsc::channel(8);
		let task = tokio::spawn(run_inner(
			credentials,
			capture,
			playback,
			control_rx,
			Some(camera_rx),
			None,
			None,
			move |status| status_tx.try_send(status).map_err(|_| ()),
			Identity::generate(),
			format!("ws://{address}"),
			true,
		));
		let pcm = timeout(Duration::from_secs(10), async {
			assert!(matches!(
				status_rx.recv().await.unwrap(),
				Status::Connecting
			));
			assert!(matches!(
				status_rx.recv().await.unwrap(),
				Status::Discovering
			));
			assert!(matches!(status_rx.recv().await.unwrap(), Status::CameraAvailable(false)));
			assert!(matches!(
				status_rx.recv().await.unwrap(),
				Status::TransportReady
			));
			assert!(matches!(status_rx.recv().await.unwrap(), Status::Securing));
			let mut ready = 0;
			let mut heard = false;
			let mut waiting = false;
			let mut captured = false;
			let mut resumed_pcm = None;
			let mut waiting_tx = Some(waiting_tx);
			let mut heard_tx = Some(heard_tx);
			let mut resumed_tx = Some(resumed_tx);
			let mut capture_tick = tokio::time::interval(Duration::from_millis(20));
			capture_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
			loop {
				tokio::select! {
					status = status_rx.recv() => match status.unwrap() {
						Status::Ready { .. } => {
							ready += 1;
							if ready == 2 {
								// Discard first-session playback before allowing the peer's new frame.
								for _ in 0..8 {
									if playback_rx.try_recv().is_err() { break; }
								}
								resumed_tx.take().unwrap().send(()).unwrap();
							}
						}
						Status::RemoteAudio => {
							heard = true;
							if let Some(sender) = heard_tx.take() { sender.send(()).unwrap(); }
						}
						Status::WaitingForPeer => {
							waiting = true;
							if let Some(sender) = waiting_tx.take() { sender.send(()).unwrap(); }
						}
						Status::Connecting | Status::Discovering | Status::Securing
						| Status::TransportReady | Status::Speaking(_) | Status::CameraAvailable(_) => {}
					},
					result = &mut captured_rx, if !captured => {
						result.unwrap();
						captured = true;
					},
					_ = capture_tick.tick(), if ready > 0 => {
						// Model a bounded continuous microphone, not one frame discarded by a stall.
						if !captured {
							match capture_tx.try_send(std::array::from_fn(|i| (i as f32 * 0.06).sin() * 0.3)) {
								Ok(()) | Err(std::sync::mpsc::TrySendError::Full(_)) => {}
								Err(std::sync::mpsc::TrySendError::Disconnected(_)) => panic!("test capture stopped before peer receipt"),
							}
						}
						if ready == 2 {
							resumed_pcm = playback_rx.try_recv().ok();
						}
					}
				}
				if ready == 2 && heard && captured && let Some(pcm) = resumed_pcm.take() {
					assert_eq!(waiting, guild);
					break pcm;
				}
			}
		})
		.await
		.unwrap();
		assert!(pcm.iter().any(|s| s.abs() > 0.01));
		if guild {
			done_tx.send(()).unwrap();
			assert_eq!(
				timeout(Duration::from_secs(2), task)
					.await
					.unwrap()
					.unwrap(),
				Err("Discord voice connection closed; rejoin the call")
			);
			drop(control_tx);
		} else {
			drop(control_tx);
			assert!(task.await.unwrap().is_ok());
			done_tx.send(()).unwrap();
		}
		server.await.unwrap();
	}
}

#[cfg(test)]
#[path = "test_stream.rs"]
mod test_stream;
