use crate::{
	Controls, Frame, Status,
	crypto::{Dave, Encryption, Identity, MAX_PACKET, MAX_SIGNAL, MODE, is_dave_frame},
	diagnostics::{Signal, Video},
	video_receive::{
		DecoderQueue, Encoded, Receivers, VideoSink, has_parameter_sets, is_keyframe, offer, pli,
		remove as remove_decoder, retain_sources, spawn_decoder,
	},
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

fn receive_media(
	dave: &mut Dave,
	user: u64,
	media: davey::MediaType,
	frame: Vec<u8>,
	passthrough: bool,
	decrypt: bool,
) -> Result<Vec<u8>, ()> {
	if is_dave_frame(&frame) {
		if !decrypt {
			return Err(());
		}
		dave.session.decrypt(user, media, &frame).map_err(|_| ())
	} else if passthrough
		|| (media == davey::MediaType::AUDIO && frame == davey::OPUS_SILENCE_PACKET)
	{
		Ok(frame)
	} else {
		Err(())
	}
}

fn participant_ids(data: &Value) -> Result<Vec<u64>, &'static str> {
	let ids = data["user_ids"]
		.as_array()
		.ok_or("Missing voice participants")?;
	if ids.len() > crate::crypto::MAX_PARTICIPANTS {
		return Err("Voice channel exceeds the 64 participant limit");
	}
	ids.iter()
		.map(|value| {
			value
				.as_str()
				.and_then(|value| value.parse().ok())
				.or_else(|| value.as_u64())
				.filter(|id| *id != 0)
				.ok_or("Malformed voice participant")
		})
		.collect()
}

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
	} else if dave.session.is_ready() && dave.transitioning() {
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
	if (!host.ends_with(".discord.media") && !host.ends_with(".discord.gg"))
		|| url.username() != ""
		|| url.password().is_some()
	{
		return Err("Voice endpoint is outside Discord media");
	}
	// Preserve the port Discord assigned (e.g. 2053, 2083 for large rooms).
	// The voice room instance is allocated on this exact port; connecting to a different
	// port results in close code 4006 (Session is no longer valid).
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
/// A filtered path can accept TLS 1.2 and drop 1.3, so this is only a retry after a rejected
/// handshake, never the default stack.
fn tls12_connector() -> tokio_tungstenite::Connector {
	let mut root_store = rustls::RootCertStore::empty();
	root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
	let config = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
		.with_root_certificates(root_store)
		.with_no_client_auth();
	tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(config))
}

/// Why a signaling dial failed; call sites keep their own wording.
enum ConnectFailure {
	TimedOut,
	Rejected,
}

/// Dials voice signaling with the default TLS stack and retries once with TLS 1.2 only.
async fn connect_socket(url: &str, config: &WebSocketConfig) -> Result<Socket, ConnectFailure> {
	match dial(url, config, None).await {
		// A timeout is not a handshake rejection; repeating it would only double the wait.
		Err(ConnectFailure::TimedOut) => Err(ConnectFailure::TimedOut),
		Err(ConnectFailure::Rejected) => dial(url, config, Some(tls12_connector())).await,
		Ok(socket) => Ok(socket),
	}
}

async fn dial(
	url: &str,
	config: &WebSocketConfig,
	connector: Option<tokio_tungstenite::Connector>,
) -> Result<Socket, ConnectFailure> {
	timeout(
		Duration::from_secs(15),
		tokio_tungstenite::connect_async_tls_with_config(url, Some(*config), false, connector),
	)
	.await
	.map_err(|_| ConnectFailure::TimedOut)?
	.map(|(socket, _)| socket)
	.map_err(|_| ConnectFailure::Rejected)
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
/// Media datagrams are lossy by design. A full send buffer, a network roam or a stray
/// ICMP unreachable (which Windows reports on the next send of a connected socket) drops
/// one packet; only a path that keeps failing for `UDP_OUTAGE` ends the call.
/// Never wait for socket capacity here: that would stall audio, signaling and feedback.
const UDP_OUTAGE: Duration = Duration::from_secs(10);
#[derive(Default)]
struct UdpFailures(Option<Instant>);
impl UdpFailures {
	fn send(
		&mut self,
		socket: &UdpSocket,
		data: &[u8],
		outage: &'static str,
	) -> Result<(), &'static str> {
		if socket.try_send(data).is_ok() {
			self.0 = None;
			return Ok(());
		}
		let now = Instant::now();
		if now.duration_since(*self.0.get_or_insert(now)) >= UDP_OUTAGE {
			return Err(outage);
		}
		Ok(())
	}
}
/// Receive errors that describe one lost or rejected datagram rather than a dead socket.
fn transient_receive(error: &std::io::Error) -> bool {
	// WSAEMSGSIZE: Windows fails an oversized datagram instead of truncating it.
	const WSAEMSGSIZE: i32 = 10040;
	matches!(
		error.kind(),
		std::io::ErrorKind::ConnectionReset
			| std::io::ErrorKind::ConnectionRefused
			| std::io::ErrorKind::Interrupted
	) || (cfg!(windows) && error.raw_os_error() == Some(WSAEMSGSIZE))
}
// Native voice UDP ping: signaling heartbeats alone do not maintain an idle
// media path (notably a receive-only stream or a muted call).
fn udp_keepalive(
	socket: &UdpSocket,
	failures: &mut UdpFailures,
	sequence: &mut u32,
) -> Result<(), &'static str> {
	*sequence = sequence.wrapping_add(1);
	let mut packet = [0x13, 0x37, 0xca, 0xfe, 0, 0, 0, 0];
	packet[4..].copy_from_slice(&sequence.to_le_bytes());
	failures.send(socket, &packet, "Voice UDP keepalive failed")
}
fn number(data: &Value, key: &str) -> Result<u64, &'static str> {
	data[key].as_u64().ok_or("Malformed voice signaling field")
}
fn id(data: &Value, key: &str) -> Result<u64, &'static str> {
	data[key]
		.as_str()
		.and_then(|v| v.parse().ok())
		.or_else(|| data[key].as_u64())
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
pub(super) fn announce_video(
	receivers: &mut Receivers,
	decoder: &DecoderQueue,
	user: u64,
	data: &Value,
) -> Result<(), &'static str> {
	let ssrc = |value: &Value| value.as_u64().and_then(|v| u32::try_from(v).ok());
	let streams = data
		.get("streams")
		.map(|value| value.as_array().ok_or("Invalid video stream announcement"))
		.transpose()?;
	let primary = data
		.get("video_ssrc")
		.map(|value| ssrc(value).ok_or("Invalid video SSRC"))
		.transpose()?;
	let mut sources = Vec::with_capacity(5);
	if let Some(primary) = primary.filter(|value| *value != 0) {
		sources.push((primary, ssrc(&data["rtx_ssrc"])));
	}
	for stream in streams.into_iter().flatten().take(4) {
		if (stream.get("type").is_none() || stream["type"] == "video")
			&& let Some(value) = ssrc(&stream["ssrc"]).filter(|v| *v != 0)
		{
			sources.push((value, ssrc(&stream["rtx_ssrc"])));
		}
	}
	if streams.is_some() {
		// A present list is the current snapshot; absent lists are partial updates.
		let keep: Vec<_> = sources.iter().map(|(ssrc, _)| *ssrc).collect();
		receivers.retain_user_sources(user, &keep);
	} else if primary == Some(0) {
		receivers.remove(user);
	}
	for (ssrc, rtx) in sources {
		receivers.announce(user, ssrc)?;
		if let Some(rtx) = rtx {
			receivers.announce_rtx(ssrc, rtx)?;
		}
	}
	retain_sources(decoder, receivers);
	Ok(())
}
fn discovery(packet: &[u8], ssrc: u32) -> Result<(IpAddr, u16), &'static str> {
	if packet.len() < 74 || packet[..4] != [0, 2, 0, 70] || packet[4..8] != ssrc.to_be_bytes() {
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
fn stream_discovery(
	packet: &[u8],
	audio_ssrc: u32,
	video_ssrc: u32,
) -> Result<(IpAddr, u16), &'static str> {
	if packet.len() < 74 || packet[..4] != [0, 2, 0, 70] {
		return Err("Invalid stream UDP discovery reply");
	}
	let packet_ssrc = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
	if packet_ssrc != 0
		&& packet_ssrc != audio_ssrc
		&& (video_ssrc == 0 || packet_ssrc != video_ssrc)
	{
		// Tolerant: socket is connected to Discord media server exclusively.
	}
	let end = packet[8..72]
		.iter()
		.position(|b| *b == 0)
		.ok_or("Invalid stream discovery address")?;
	let address = std::str::from_utf8(&packet[8..8 + end])
		.map_err(|_| "Invalid stream discovery address")?
		.parse()
		.map_err(|_| "Invalid stream discovery address")?;
	let port = u16::from_be_bytes([packet[72], packet[73]]);
	if port == 0 {
		return Err("Invalid stream discovery port");
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
	emit: impl Fn(Status) -> Result<(), ()> + Send + 'static,
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
	emit: impl Fn(Status) -> Result<(), ()> + Send + 'static,
	identity: Arc<Identity>,
) -> Result<(), &'static str> {
	let url = endpoint(&credentials.endpoint)?;
	crate::timer::isolated("serein-voice", move || {
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
	})
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
	let mut next_sink_wants = Instant::now();
	let mut watch = VideoWatch::new();
	let mut metrics = crate::diagnostics::Metrics::new(crate::diagnostics::Scope::Transport);
	emit(Status::Connecting).map_err(|_| "Call interface closed")?;
	let config = WebSocketConfig::default()
		.max_message_size(Some(MAX_SIGNAL))
		.max_frame_size(Some(MAX_SIGNAL))
		.write_buffer_size(0)
		.max_write_buffer_size(MAX_SIGNAL * 2);
	let mut ws = connect_socket(&url, &config)
		.await
		.map_err(|failure| match failure {
			ConnectFailure::TimedOut => "Voice connection timed out",
			ConnectFailure::Rejected => "Voice TLS connection failed",
		})?;
	// Only voice-scoped credentials go to this validated endpoint; no account Authorization header.
	json_send(&mut ws,json!({"op":0,"d":{"server_id":credentials.guild.unwrap_or(credentials.channel).to_string(),"user_id":credentials.user.to_string(),"session_id":credentials.session.expose(),"token":credentials.token.expose(),"video":video_capable,"streams":if camera.is_some(){vec![json!({"type":"video","rid":"100","quality":100})]}else{vec![]},"max_dave_protocol_version":1}})).await?;
	let mut dave = Dave::with_identity(
		credentials.user.0,
		credentials.peer.map(|peer| peer.0),
		credentials.channel.0,
		identity,
	)?;
	let mut dave_enabled = true;
	// Receive passthrough may start before the send-side transition executes.
	let mut receive_passthrough = false;
	let mut receive_keys_until: Option<Instant> = None;
	let mut pending_downgrade: Option<u16> = None;
	let mut encryption: Option<Encryption> = None;
	// The roster message normally follows the session key within milliseconds; only after
	// this grace does a sole member conclude nobody else is in the call.
	let mut secured_at: Option<Instant> = None;
	let mut udp: Option<UdpSocket> = None;
	let mut next_udp_ping = Instant::now();
	let mut udp_ping_sequence = 0;
	let mut udp_failures = UdpFailures::default();
	let mut discovering = false;
	let mut discovery_deadline = Instant::now();
	let mut next_discovery_probe = Instant::now();
	let mut ssrc = 0u32;
	let mut video = crate::camera_video::Sender::default();
	let mut video_tick = tokio::time::interval(Duration::from_millis(2));
	video_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
	let mut mixer = crate::mixer::Mixer::default();
	let mut stream_playout = crate::stream_playout::Playout::default();
	let mut seq_ack: i64 = -1;
	let mut heartbeat_ms: Option<u64> = None;
	let mut heartbeat_at = Instant::now();
	let mut awaiting_ack = None;
	let mut heartbeat_nonce = 0u64;
	let mut missed_acks = 0u8;
	// Set when the signaling socket died or went silent; the next loop turn resumes it.
	let mut reconnect: Option<&'static str> = None;
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
		if let Some(reason) = reconnect.take() {
			if encryption.is_none() || resume_attempts >= 2 {
				return Err(reason);
			}
			video.clear();
			video.announced = false;
			resume_attempts += 1;
			resuming = true;
			ready_announced = false;
			waiting_announced = false;
			capture_reset = true;
			emit(Status::Securing).map_err(|_| "Call interface closed")?;
			deadline = Some(Instant::now() + Duration::from_secs(30));
			heartbeat_ms = None;
			awaiting_ack = None;
			missed_acks = 0;
			let config = WebSocketConfig::default()
				.max_message_size(Some(MAX_SIGNAL))
				.max_frame_size(Some(MAX_SIGNAL))
				.write_buffer_size(0)
				.max_write_buffer_size(MAX_SIGNAL * 2);
			let replacement =
				connect_socket(&url, &config)
					.await
					.map_err(|failure| match failure {
						ConnectFailure::TimedOut => "Voice resume timed out; rejoin the call",
						ConnectFailure::Rejected => "Voice resume failed; rejoin the call",
					})?;
			ws = replacement;
			json_send(&mut ws,json!({"op":7,"d":{"server_id":credentials.guild.unwrap_or(credentials.channel).to_string(),"session_id":credentials.session.expose(),"token":credentials.token.expose(),"seq_ack":seq_ack}})).await?;
		}
		tokio::select! {
			changed=controls.changed()=>{ if changed.is_err(){return Ok(());} },
			_=video_tick.tick(), if !video.is_empty()=>{
				let generation=controls.borrow().camera;
				if (dave_enabled && !dave.ready) || resuming || generation==0 || generation!=video.generation {video.clear();}
				else if let Some(packet)=video.next() && let Some(socket)=&udp {
					udp_failures.send(socket,&packet,"Camera UDP send failed")?;
				}
			},
			_=tick.tick()=>{
				let now=Instant::now();
				if deadline.is_some_and(|d|now>=d) {
					return Err(negotiation_timeout(heartbeat_ms.is_some(),udp.is_some(),encryption.is_some(),&dave,resuming));
				}
				if discovering {
					if now>=discovery_deadline {
						return Err("Discord voice UDP discovery timed out; check the network firewall");
					}
					if now>=next_discovery_probe && let Some(socket)=&udp {
						let mut probe=[0;74];probe[..4].copy_from_slice(&[0,1,0,70]);probe[4..8].copy_from_slice(&ssrc.to_be_bytes());
						let _ = socket.send(&probe).await;
						next_discovery_probe=now+Duration::from_millis(300);
					}
				}
				if !discovering && now>=next_udp_ping && let Some(socket)=&udp {
					udp_keepalive(socket,&mut udp_failures,&mut udp_ping_sequence)?;
					next_udp_ping=now+Duration::from_secs(5);
				}
				if let Some(interval)=heartbeat_ms && now>=heartbeat_at {
					// A delayed wake can find the acknowledgement still unread; only a second
					// silent interval marks the socket as zombied, and Discord expects a resume.
					if awaiting_ack.is_some() {
						missed_acks+=1;
						if missed_acks>=3 {reconnect=Some("Discord voice heartbeat was not acknowledged; rejoin the call");continue;}
					}
					heartbeat_nonce=heartbeat_nonce.wrapping_add(1);
					json_send(&mut ws,json!({"op":3,"d":{"t":heartbeat_nonce,"seq_ack":seq_ack}})).await?;
					awaiting_ack=Some(heartbeat_nonce);heartbeat_at=now+Duration::from_millis(interval);
				}
				if dave_enabled && secured_at.is_some_and(|at| now>=at+PEER_GRACE) && !discovering && !resuming && dave.should_wait_for_peer() {dave.enter_sole_member_waiting()?;}
				let waiting=dave_enabled && dave.waiting && !dave.transitioning() && encryption.is_some() && !discovering && !resuming;
				let enabled=(!dave_enabled || dave.ready) && encryption.is_some() && !discovering && !resuming;
				if (!enabled && !waiting) || dave.transitioning() {deadline.get_or_insert(now+Duration::from_secs(30));}
				if (!enabled && ready_announced) || (!waiting && waiting_announced) {
					ready_announced=false;
					waiting_announced=false;
					emit(Status::Securing).map_err(|_|"Call interface closed")?;
				}
				if waiting && !waiting_announced {deadline=None;waiting_announced=true;emit(Status::WaitingForPeer).map_err(|_|"Call interface closed")?;}
				if !waiting {waiting_announced=false;}
				if enabled && !ready_announced {
					if !dave.transitioning() {deadline=None;}ready_announced=true;
					emit(Status::Ready{privacy_code:if dave_enabled {dave.session.voice_privacy_code().unwrap_or_default().into()} else {String::new()}}).map_err(|_|"Call interface closed")?;
				}
				let stalled = watch.tick(&mut metrics, &mut receivers, decoder.as_ref(), now);
				if enabled && decoder.is_some() && now >= next_sink_wants {
					json_send(&mut ws, json!({"op": 15, "d": {"any": 100}})).await?;
					metrics.signal(Signal::SinkWantsSent, 1);
					next_sink_wants = now + if stalled { SINK_WANTS_STALLED_INTERVAL } else { SINK_WANTS_INTERVAL };
				}
				// Lost or stalled pictures stay frozen until the sender refreshes; ask twice a second at most.
				if enabled && now>=next_pli && let Some(lost)=&lost && let Some(crypto)=encryption.as_mut() && let Some(socket)=&udp {
					receivers.absorb(lost);
					let requests:Vec<u32>=receivers.keyframe_requests().collect();
					metrics.video(Video::PliSent,requests.len() as u64);
					for media in requests {let (header,body)=pli(ssrc,media);udp_failures.send(socket,&crypto.seal_rtcp(&header,&body)?,"Voice RTCP send failed")?;}
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
					let normalized=crate::video_sps::normalize(&frame.data)?;
					let encrypted=if dave_enabled && dave.ready {
						dave.session
							.encrypt(davey::MediaType::VIDEO,davey::Codec::H264,&normalized)
							.map_err(|_| "DAVE camera encryption failed")?
							.into_owned()
					} else {
						normalized.to_vec()
					};
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
				let latest=if waiting {
					capture_pacer.preview(&capture)
				} else {
					capture_pacer.next(&capture,enabled && !control.muted && !control.deafened,stalled)
				};
				local_activity=if (enabled || waiting) && !control.muted && !control.deafened && !stalled {
					crate::activity::hold_at(latest.as_ref().map_or(0.0, |frame| frame.iter().filter(|s| s.is_finite()).map(|s| s*s).sum()),local_activity,control.activity_threshold_db)
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
						if dave_enabled && (dave.ready || dave.session.is_ready()) {
							dave.session
								.encrypt_opus(&encoded[..length])
								.map_err(|_| "DAVE audio encryption failed")?
								.into_owned()
						} else {
							encoded[..length].to_vec()
						}
					} else {silence-=1;davey::OPUS_SILENCE_PACKET.to_vec()};
					let mut header=[0;12];header[0]=0x80;header[1]=120;header[2..4].copy_from_slice(&sequence.to_be_bytes());header[4..8].copy_from_slice(&timestamp.to_be_bytes());header[8..12].copy_from_slice(&ssrc.to_be_bytes());
					let wire=encryption.as_mut().ok_or("Missing voice transport key")?.seal(&header,&data)?;
					metrics.finish(crate::diagnostics::Stage::Encode, start);
					if let Some(socket)=&udp {udp_failures.send(socket,&wire,"Voice UDP send failed")?;}
					sequence=sequence.wrapping_add(1);
					if !active && silence==0 {json_send(&mut ws,json!({"op":5,"d":{"speaking":0,"delay":0,"ssrc":ssrc}})).await?;speaking=false;}
					if active {silence=0;}
				}
				timestamp=timestamp.wrapping_add(960);
				let mut drops = 0;
				if enabled && !control.deafened {
					if stalled {mixer.clear();}
					let start = metrics.start();
					let (mut frame,remote_audio)=mixer.pop_with_volumes(&control.user_volumes);
					// Keep the auxiliary stream close to live even if this clock misses a tick.
					if let Some(aux)=&stream_audio && let Some(extra)=stream_playout.next(aux,control.stream_volume,true,stalled) {
						match &mut frame {
							Some(mixed)=>for (out,sample) in mixed.iter_mut().zip(extra.iter()) {*out=(*out+sample).clamp(-1.0,1.0);},
							None=>frame=Some(extra),
						}
					}
					metrics.finish(crate::diagnostics::Stage::Mix, start);
					if let Some(frame)=frame {drops = u64::from(playback.try_send(frame).is_err());}
					if !heard && remote_audio {heard=true;emit(Status::RemoteAudio).map_err(|_|"Call interface closed")?;}
				} else {mixer.clear();if let Some(aux)=&stream_audio {let _=stream_playout.next(aux,0,false,false);}}
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
				let length=match result {
					Ok(length)=>length,
					Err(error) if transient_receive(&error)=>continue,
					Err(_)=>return Err("Voice UDP receive failed"),
				};
				if length>MAX_PACKET {continue;}
				if discovering {
					let Ok((address, port)) = discovery(&packet[..length], ssrc) else {
						continue;
					};
					json_send(&mut ws,json!({"op":1,"d":{"protocol":"udp","data":{"address":address.to_string(),"port":port,"mode":MODE},"codecs":if video_capable{vec![json!({"name":"opus","type":"audio","priority":1000,"payload_type":120}),json!({"name":"H264","type":"video","priority":1000,"payload_type":101,"rtx_payload_type":102,"encode":camera.is_some(),"decode":decoder.is_some()})]}else{vec![json!({"name":"opus","type":"audio","priority":1000,"payload_type":120})]}}})).await?;
					discovering=false;continue;
				}
				let Some(crypto)=&encryption else{continue;};
				let start = metrics.start();
				let Some(mut rtp)=crypto.open(&packet[..length]) else{metrics.video(Video::OpenFailed,1);continue;};
				if rtp.payload_type==102 {metrics.video(Video::Rtx,1);let Some((media,sequence))=receivers.restore_rtx(rtp.ssrc,&mut rtp.payload) else{continue;};rtp.ssrc=media;rtp.sequence=sequence;rtp.payload_type=101;}
				if rtp.payload_type==101 {
					metrics.video(Video::Packets,1);
					let Some(decoder)=&decoder else{continue;};
					let Some((user,frame))=receivers.push(rtp.ssrc,rtp.sequence,rtp.timestamp,rtp.marker,&rtp.payload) else{continue;};
					if dave_enabled && !dave.session.is_ready() {metrics.video(Video::NotReady,1);}
					let data = match receive_media(&mut dave, user, davey::MediaType::VIDEO, frame, receive_passthrough, dave_enabled || receive_keys_until.is_some_and(|until| Instant::now() < until)) {
						Ok(data) => data,
						Err(()) => { receivers.require_keyframe(user);metrics.video(Video::DecryptFailed,1);continue; }
					};
					let keyframe=is_keyframe(&data);
					if keyframe {metrics.video(Video::Keyframes,1);metrics.video(Video::KeyframesWithoutParams,u64::from(!has_parameter_sets(&data)));}
					if !receivers.accept(user,keyframe) {metrics.video(Video::Gated,1);continue;}
					if !offer(decoder,Encoded{user,data,keyframe})? {receivers.require_keyframe(user);metrics.video(Video::QueueFull,1);}
					continue;
				}
				if rtp.payload_type!=120 {continue;}
				let (source,seq,frame)=(rtp.ssrc,rtp.sequence,rtp.payload);
				let Some(user)=mixer.user(source) else{continue;};
				if controls.borrow().deafened {continue;}
				let opus = match receive_media(&mut dave, user, davey::MediaType::AUDIO, frame, receive_passthrough, dave_enabled || receive_keys_until.is_some_and(|until| Instant::now() < until)) {
					Ok(data) => data,
					Err(()) => { continue; }
				};
				mixer.push(source,seq,opus);
				metrics.finish(crate::diagnostics::Stage::Receive, start);
			},
			event=ws.next()=>{
				let event=match event {
					// A server crash preserves the voice session. Other close codes remain terminal.
					Some(Ok(message)) if !matches!(&message, Message::Close(Some(frame)) if u16::from(frame.code)==4015)=>message,
					_=>{reconnect=Some("Voice socket failed; rejoin the call");continue;}
				};
				if signal_window.elapsed()>Duration::from_secs(1){signal_window=Instant::now();signal_count=0;}
				signal_count+=1;if signal_count>256{return Err("Voice signaling exceeded the bounded processing rate");}
				match event {
					Message::Text(text)=>{
						let mut event:Value=serde_json::from_str(&text).map_err(|_|"Invalid voice JSON")?;
						if let Some(seq)=event["seq"].as_i64(){seq_ack=seq;}
						let op=number(&event,"op")?;let data=&mut event["d"];
						metrics.signal(Signal::Text,1);
						metrics.signal(signal_of(op),1);
						match op {
							8=>{
								let interval=data["heartbeat_interval"].as_f64().filter(|v|v.is_finite() && *v>=100.0 && *v<=120_000.0).ok_or("Invalid voice heartbeat interval")? as u64;
								if !(100..=120000).contains(&interval) || heartbeat_ms.is_some(){return Err("Invalid voice heartbeat negotiation");}
								heartbeat_ms=Some(if cfg!(test) { interval.min(5000) } else { interval });heartbeat_at=Instant::now();
							},
							6=>{
								let nonce = data.as_u64().or_else(|| data["t"].as_u64());
								if acknowledges(awaiting_ack, missed_acks, nonce) {
									awaiting_ack = None;
									missed_acks = 0;
									// A healthy resumed socket renews the consecutive resume budget.
									if !resuming { resume_attempts = 0; }
								}
							},
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
								udp=Some(socket);discovering=true;discovery_deadline=Instant::now()+Duration::from_secs(8);next_discovery_probe=Instant::now()+Duration::from_millis(300);
								emit(Status::Discovering).map_err(|_|"Call interface closed")?;
							},
							4=>{
								if encryption.is_some() || udp.is_none() || discovering {return Err("Unexpected voice session description");}
								if data["mode"].as_str()!=Some(MODE){return Err("Required voice transport encryption was not negotiated");}
								let values=data["secret_key"].take();let values=values.as_array().ok_or("Missing voice transport key")?;
								if values.len()!=32{return Err("Invalid voice transport key");}
								let mut key=Zeroizing::new([0;32]);for (out,v) in key.iter_mut().zip(values){*out=v.as_u64().and_then(|v|u8::try_from(v).ok()).ok_or("Invalid voice transport key")?;}
								encryption=Some(Encryption::new(&key));secured_at=Some(Instant::now());deadline=Some(Instant::now()+Duration::from_secs(30));
								// Register the audio SSRC without opening a microphone or sending media.
								json_send(&mut ws,json!({"op":5,"d":{"speaking":0,"delay":0,"ssrc":ssrc}})).await?;
								video.negotiated=h264_negotiated(data);
								if camera.is_some(){emit(Status::CameraAvailable(video.available())).map_err(|_|"Call interface closed")?;}
								let dave_ver = data
									.get("dave_protocol_version")
									.map(|v| v.as_u64().ok_or("Invalid DAVE protocol version"))
									.transpose()?
									.unwrap_or(0);
								if dave_ver == 0 {
									dave_enabled = false;
									receive_passthrough = true;
									receive_keys_until = None;
								} else if dave_ver != 1 {
									return Err("Unsupported voice DAVE version");
								} else {
									dave_enabled = true;
									receivers.retain(|u| dave.contains(u));
									mixer.retain(|u| dave.contains(u));
									if let Some(decoder) = decoder.as_ref() {
										retain_sources(decoder, &receivers);
									}
									send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;
								}
								emit(Status::TransportReady).map_err(|_|"Call interface closed")?;
								emit(Status::Securing).map_err(|_|"Call interface closed")?;
							},
							5=>{
								let user=id(data,"user_id")?;
								if user!=credentials.user.0 {
									let was_ready = dave.ready;
									match dave.connect(&[user]) {
										Ok(_) => {
											if was_ready || dave.session.is_ready() { dave.ready = true; }
										}
										Err("Voice channel exceeds the 64 participant limit") if !dave_enabled || encryption.is_none() => {}
										Err(_) => continue,
									}
									let value=u32::try_from(number(data,"ssrc")?).map_err(|_|"Invalid voice SSRC")?;
									let _ = mixer.announce(user,value);
								}
							},
							11=>{
								let ids=participant_ids(data)?;
								if dave_enabled {
									let was_ready=dave.ready;
									if dave.connect(&ids)? {
										if was_ready || dave.session.is_ready() {
											dave.ready=true;
										} else {
											video.clear();
											capture_reset=true;
											mixer.clear();
										}
									}
								}
							},
							13=>{
								let user=id(data,"user_id")?;mixer.remove(user);receivers.remove(user);if let Some(decoder)=decoder.as_ref(){remove_decoder(decoder,user);}
								if !dave_enabled {
									let _ = dave.disconnect(user);
								} else {
									let was_group_member=dave.is_group_member(user);
									let was_ready=dave.ready;
									if dave.disconnect(user)? {
										if dave.alone() {
											video.clear();
											capture_reset=true;
											mixer.clear();
											dave.enter_sole_member_waiting()?;
											deadline=None;
											waiting_announced=true;
											ready_announced=false;
											emit(Status::WaitingForPeer).map_err(|_|"Call interface closed")?;
										} else if was_ready {
											// The remaining members still share the current epoch until the removal commit.
											dave.ready=true;
											deadline=None;
										} else if was_group_member {
											video.clear();
											capture_reset=true;
											mixer.clear();
										}
									}
								}
							},
							21=>{
								let (Ok(proto), Ok(id)) = (number(data, "protocol_version"), transition(data)) else { continue; };
								if proto != 0 && proto != 1 { return Err("Discord requested an unsupported DAVE protocol version"); }
								video.clear();
								capture_reset=true;
								if proto==0 {
									receive_passthrough=true;
									dave.prepare_protocol(id);
									if id==0 {
										dave.execute(0)?;
										dave_enabled=false;
										receive_keys_until=Some(Instant::now()+Duration::from_secs(10));
										dave.ready=false;
										ready_announced=false;
										pending_downgrade=None;
									} else {
										pending_downgrade=Some(id);
										json_send(&mut ws,json!({"op":23,"d":{"transition_id":id}})).await?;
									}
								} else if proto==1 {
									ready_announced=false;
									pending_downgrade=None;
									if id==0 {dave_enabled=true;receive_passthrough=false;receive_keys_until=None;}
									else {receive_keys_until=Some(Instant::now()+Duration::from_secs(30));}
									dave.prepare_protocol(id);
									if id==0 {
										if dave.session.is_ready(){
											dave.execute(0)?;
										} else if dave.alone(){
											dave.enter_sole_member_waiting()?;
											deadline=None;
											waiting_announced=true;
											ready_announced=false;
											emit(Status::WaitingForPeer).map_err(|_|"Call interface closed")?;
										} else {
											dave.pending_protocol=None;
											dave.ready=false;
										}
									} else {
										json_send(&mut ws,json!({"op":23,"d":{"transition_id":id}})).await?;
									}
								}
							},
							22=>{
								video.clear();
								let tid = transition(data)?;
								let protocol_change = dave.pending_protocol == Some(tid);
								dave.execute(tid)?;
								capture_reset=true;
								if pending_downgrade == Some(tid) {
									pending_downgrade = None;
									dave_enabled = false;
									receive_passthrough = true;
									receive_keys_until = Some(Instant::now()+Duration::from_secs(10));
									dave.ready = false;
									ready_announced = false;
								} else if protocol_change {
									ready_announced=false;
									dave_enabled=true;receive_passthrough=false;receive_keys_until=None;
								}
							},
							24=>{
								video.clear();
								if number(data,"epoch").unwrap_or(0)==1 {
									capture_reset=true;
									dave.reinitialize()?;
									let pkg = dave.key_package()?;
									send(&mut ws, Message::Binary(pkg.into())).await?;
								}
							},							9=>{
								if !resuming{return Err("Unexpected voice resumption");}
								if camera.is_some(){json_send(&mut ws,video.announcement(ssrc,false)).await?;}
								json_send(&mut ws,json!({"op":5,"d":{"speaking":0,"delay":0,"ssrc":ssrc}})).await?;
								speaking=false;silence=0;
								resuming=false;
							},
							12=>{
								let user=id(data,"user_id")?;
								if user!=credentials.user.0 {
									let was_ready = dave.ready;
									match dave.connect(&[user]) {
										Ok(_) => {
											if was_ready || dave.session.is_ready() { dave.ready = true; }
										}
										Err("Voice channel exceeds the 64 participant limit") if !dave_enabled || encryption.is_none() => {}
										Err(_) => continue,
									}
									if let Some(value)=data["audio_ssrc"].as_u64().and_then(|v|u32::try_from(v).ok()) {let _ = mixer.announce(user,value);}
									if let Some(decoder) = decoder.as_ref() {
										let _ = announce_video(&mut receivers, decoder, user, data);
										next_sink_wants = Instant::now();
									}
								}
							},
							18 | 20 => {
								if let Ok(user) = id(data, "user_id")
									&& user != credentials.user.0
								{
									let was_ready = dave.ready;
									let _ = dave.connect(&[user]);
									if was_ready || dave.session.is_ready() { dave.ready = true; }
								}
							},
							14..=17 | 19 => {},
							_=>{},
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
								ready_announced=false;waiting_announced=false;
								match dave.group_changed(opcode,data){
									Ok(id)=>{if id!=0 {json_send(&mut ws,json!({"op":23,"d":{"transition_id":id}})).await?;}},
									Err(_)=>{
										if data.len()>=2 {
											let id=u16::from_be_bytes([data[0],data[1]]);
											json_send(&mut ws,json!({"op":31,"d":{"transition_id":id}})).await?;
											dave.reset()?;
											let pkg = dave.key_package()?;
											send(&mut ws, Message::Binary(pkg.into())).await?;
											deadline=Some(Instant::now()+Duration::from_secs(30));
										}
									}
								}
								// A live call only re-announces Ready with the new privacy code.
								if dave.ready && !dave.transitioning() {
									deadline=None;
								}
							},
							_=>return Err("Unsupported DAVE opcode; call stopped"),
						}
					},
					Message::Ping(data)=>send(&mut ws,Message::Pong(data)).await?,
					// 4014: kicked, moved or the channel was deleted; Discord says not to reconnect.
					Message::Close(Some(frame)) if u16::from(frame.code)==4014=>return Err(DISCONNECTED_BY_DISCORD),
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

/// The voice server ended this device's session on purpose; rejoining automatically is wrong.
pub const DISCONNECTED_BY_DISCORD: &str = "Discord disconnected this device from the call";

/// Whether a heartbeat acknowledgement names the latest outstanding nonce or one of the
/// `missed` heartbeats sent before it; a late acknowledgement still proves liveness.
/// Others are stale duplicates of an already answered heartbeat and are ignored.
fn acknowledges(awaiting: Option<u64>, missed: u8, nonce: Option<u64>) -> bool {
	awaiting
		.zip(nonce)
		.is_some_and(|(latest, nonce)| latest.wrapping_sub(nonce) <= u64::from(missed))
}

/// Maps a voice signaling opcode to its diagnostics slot. Opcodes without a dedicated slot
/// are counted together; no signaling contents are recorded.
fn signal_of(op: u64) -> Signal {
	match op {
		2 => Signal::Ready,
		4 => Signal::Session,
		11 => Signal::Clients,
		12 => Signal::Sender,
		21 => Signal::PrepareTransition,
		22 => Signal::ExecuteTransition,
		24 => Signal::PrepareEpoch,
		_ => Signal::Other,
	}
}

/// Video stops that no loss explains still need a keyframe request, so recovery cannot depend
/// on the depacketizer noticing a gap. Ask again after this long without a decoded picture.
const VIDEO_STALL: Duration = Duration::from_secs(1);
/// Discord stops forwarding video when a viewer's sink wants lapse, so refresh them while
/// watching, and sooner while video is stalled.
const SINK_WANTS_INTERVAL: Duration = Duration::from_secs(5);
const SINK_WANTS_STALLED_INTERVAL: Duration = Duration::from_secs(1);

/// Folds remote video state into the diagnostics report once per tick.
struct VideoWatch {
	last_picture_at: Instant,
	pictures: u64,
	errors: u64,
	stale: u64,
}

impl VideoWatch {
	fn new() -> Self {
		Self {
			last_picture_at: Instant::now(),
			pictures: 0,
			errors: 0,
			stale: 0,
		}
	}

	/// Returns true while video is stalled: a source is announced but no picture arrived
	/// recently. Every announced sender is asked for a keyframe for as long as that holds.
	fn tick(
		&mut self,
		metrics: &mut crate::diagnostics::Metrics,
		receivers: &mut Receivers,
		decoder: Option<&DecoderQueue>,
		now: Instant,
	) -> bool {
		let stats = receivers.take_stats();
		metrics.video(Video::UnknownSsrc, stats.unknown_ssrc);
		metrics.video(Video::Incomplete, stats.incomplete);
		metrics.video(Video::Complete, stats.complete);
		metrics.video(Video::AwaitingTicks, u64::from(receivers.awaiting()));
		let Some(decoder) = decoder else { return false };
		metrics.decoder_counts(
			decoder.counters.hardware.load(Ordering::Relaxed),
			decoder.counters.software.load(Ordering::Relaxed),
		);
		metrics.video_max(
			Video::DecodeQueueMs,
			decoder.counters.queue_ms.swap(0, Ordering::Relaxed),
		);
		let stale = decoder.counters.stale.load(Ordering::Relaxed);
		metrics.video(Video::StaleFrames, stale.wrapping_sub(self.stale));
		self.stale = stale;
		let pictures = decoder
			.counters
			.pictures
			.load(std::sync::atomic::Ordering::Relaxed);
		if pictures != self.pictures {
			metrics.video(Video::Pictures, pictures.wrapping_sub(self.pictures));
			self.pictures = pictures;
			self.last_picture_at = now;
		}
		let errors = decoder
			.counters
			.errors
			.load(std::sync::atomic::Ordering::Relaxed);
		metrics.video(Video::DecoderErrors, errors.wrapping_sub(self.errors));
		self.errors = errors;
		let gap = now.saturating_duration_since(self.last_picture_at);
		if self.pictures > 0 {
			metrics.video_max(
				Video::PictureGapMs,
				gap.as_millis().min(u128::from(u64::MAX)) as u64,
			);
		}
		// A clean stop leaves nothing marked lost, so only elapsed time can reveal it.
		let stalled = receivers.has_sources() && gap >= VIDEO_STALL;
		if stalled {
			receivers.require_all_keyframes();
			metrics.video(Video::StallTicks, 1);
		}
		stalled
	}
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
	emit: impl Fn(Status) -> Result<(), ()> + Send + 'static,
) -> Result<(), &'static str> {
	let url = endpoint(&credentials.endpoint)?;
	crate::timer::isolated("serein-stream", move || {
		run_stream_inner(
			credentials,
			identity,
			None,
			Some(video),
			None,
			None,
			emit,
			url,
			false,
		)
	})
	.await
}
/// Watch another participant's Go Live stream on its own voice gateway; decoded frames
/// reach `sink` from a dedicated thread. Same unofficial identifiers as `run_stream`.
pub async fn watch_stream(
	credentials: VoiceConnection,
	identity: Arc<Identity>,
	streamer: Option<model::Id>,
	sink: VideoSink,
	audio: Option<SyncSender<Frame>>,
	emit: impl Fn(Status) -> Result<(), ()> + Send + 'static,
) -> Result<(), &'static str> {
	let url = endpoint(&credentials.endpoint)?;
	let streamer_id = streamer.map(|id| id.0);
	crate::timer::isolated("serein-watch", move || {
		run_stream_inner(
			credentials,
			identity,
			streamer_id,
			None,
			Some(sink),
			audio,
			emit,
			url,
			false,
		)
	})
	.await
}
#[allow(clippy::too_many_arguments)] // Media inputs plus the loopback-only test endpoint.
async fn run_stream_inner(
	credentials: VoiceConnection,
	identity: Arc<Identity>,
	streamer: Option<u64>,
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
	let mut next_sink_wants = Instant::now();
	let mut watch = VideoWatch::new();
	let mut receive_tick = Instant::now();
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
	let mut ws = connect_socket(&url, &config)
		.await
		.map_err(|failure| match failure {
			ConnectFailure::TimedOut => "Stream connection timed out",
			ConnectFailure::Rejected => "Stream TLS connection failed",
		})?;
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
	let mut dave_enabled = true;
	// Receive passthrough may start before the send-side transition executes.
	let mut receive_passthrough = false;
	let mut receive_keys_until: Option<Instant> = None;
	let mut pending_downgrade: Option<u16> = None;
	let mut encryption: Option<Encryption> = None;
	let mut secured_at: Option<Instant> = None;
	let mut udp: Option<UdpSocket> = None;
	let mut next_udp_ping = Instant::now();
	let mut udp_ping_sequence = 0;
	let mut udp_failures = UdpFailures::default();
	let mut discovering = false;
	let mut discovery_deadline = Instant::now();
	let mut next_discovery_probe = Instant::now();
	let mut discovery_probe_alt = false;
	let mut audio_ssrc = 0u32;
	let mut video_ssrc = 0u32;
	let mut rtx_ssrc = 0u32;
	let mut random = [0; 4];
	getrandom::fill(&mut random).map_err(|_| "Stream random initialization failed")?;
	let mut sequence = u16::from_be_bytes([random[0], random[1]]);
	let mut rtx_sequence = u16::from_be_bytes([random[2], random[3]]);
	let mut history = crate::video::History::default();
	let mut rate = crate::video::Rate::new(
		video
			.as_ref()
			.map_or(250_000, |video| video.settings.bit_rate()),
		Instant::now(),
	);
	let mut heartbeat_ms = None;
	let mut heartbeat_at = Instant::now();
	let mut heartbeat_nonce = 0u64;
	let mut missed_acks = 0u8;
	let mut awaiting_ack = None;
	let mut seq_ack = -1i64;
	let mut deadline = Some(Instant::now() + Duration::from_secs(90));
	let mut announced = false;
	let mut waiting_announced = false;
	let mut awaiting_keyframe = true;
	let mut outgoing = crate::video::Pacer::new();
	let mut outgoing_keyframe = false;
	let mut signal_window = Instant::now();
	let mut signal_count = 0u16;
	let mut packet = [0u8; MAX_PACKET + 1];
	let mut tick = tokio::time::interval(Duration::from_millis(20));
	tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
	loop {
		// A transition must discard the rest of an access unit encrypted with the old
		// epoch before any further packet is sent, including between paced batches.
		if !announced
			|| (dave_enabled && (!dave.ready || dave.transitioning() || !dave.session.is_ready()))
			|| video
				.as_ref()
				.is_none_or(|video| !video.ready.load(Ordering::Acquire))
		{
			outgoing.clear();
			history.clear();
			outgoing_keyframe = false;
		}
		if outgoing.stale(Instant::now()) {
			outgoing.clear();
			history.clear();
			outgoing_keyframe = false;
			awaiting_keyframe = true;
			if let Some(video) = &mut video {
				for _ in 0..video.frames.len() {
					let _ = video.frames.try_recv();
				}
				video.keyframe.store(true, Ordering::Release);
			}
		}
		tokio::select! {
			_=tokio::time::sleep_until(outgoing.deadline), if !outgoing.is_empty() || history.has_pending()=>{
				let crypto=encryption.as_mut().ok_or("Missing stream transport key")?;
				let socket=udp.as_ref().ok_or("Missing stream UDP socket")?;
				let start=metrics.start();
				let now=Instant::now();
				if history.has_pending() && outgoing.allow_repair(now,rate.target)
					&& let Some(packet)=history.repair(rtx_ssrc,&mut rtx_sequence,now) {
					// The original DAVE ciphertext is reused; the transport nonce is always fresh.
					udp_failures.send(socket,&crypto.seal(&packet.header,&packet.payload)?,"Stream retransmission failed")?;
				}
				for packet in outgoing.next_batch(now,rate.target) {
					udp_failures.send(socket,&crypto.seal(&packet.header,&packet.payload)?,"Stream UDP send failed")?;
					if rtx_ssrc!=0 {history.remember(packet,now);}
				}
				metrics.finish(crate::diagnostics::Stage::VideoSend,start);
				if outgoing.is_empty() && outgoing_keyframe {
					if awaiting_keyframe {emit(Status::Ready{privacy_code:if dave_enabled {dave.session.voice_privacy_code().unwrap_or_default().into()} else {String::new()}}).map_err(|_|"Stream interface closed")?;}
					awaiting_keyframe=false;
				}
			},
			_=tick.tick()=>{
				let now=Instant::now();
				history.expire(now);
				if deadline.is_some_and(|at| now>=at) {return Err(negotiation_timeout(heartbeat_ms.is_some(),udp.is_some(),encryption.is_some(),&dave,false));}
				if discovering {
					if now>=discovery_deadline {return Err("Discord stream UDP discovery timed out");}
					if now>=next_discovery_probe && let Some(socket)=&udp {
						let probe_ssrc = if discovery_probe_alt && video_ssrc != 0 {
							video_ssrc
						} else {
							audio_ssrc
						};
						discovery_probe_alt = !discovery_probe_alt;
						let mut probe=[0;74];probe[..4].copy_from_slice(&[0,1,0,70]);probe[4..8].copy_from_slice(&probe_ssrc.to_be_bytes());
						let _ = socket.send(&probe).await;
						next_discovery_probe=now+Duration::from_millis(300);
					}
				}
				if !discovering && now>=next_udp_ping && let Some(socket)=&udp {
					udp_keepalive(socket,&mut udp_failures,&mut udp_ping_sequence)?;
					next_udp_ping=now+Duration::from_secs(5);
				}
				if let Some(interval)=heartbeat_ms && now>=heartbeat_at {
					if awaiting_ack.is_some() {
						missed_acks+=1;
						if missed_acks>=3 {return Err("Discord stream heartbeat was not acknowledged");}
					}
					heartbeat_nonce=heartbeat_nonce.wrapping_add(1);
					json_send(&mut ws,json!({"op":3,"d":{"t":heartbeat_nonce,"seq_ack":seq_ack}})).await?;
					awaiting_ack=Some(heartbeat_nonce); heartbeat_at=now+Duration::from_millis(interval);
				}
				if dave_enabled && secured_at.is_some_and(|at| now>=at+PEER_GRACE) && !discovering && dave.should_wait_for_peer() {dave.enter_sole_member_waiting()?;}
				let waiting=dave_enabled && dave.waiting && !dave.transitioning() && encryption.is_some() && !discovering;
				if waiting {deadline=None;}
				if waiting!=waiting_announced {
					emit(if waiting {Status::WaitingForPeer} else {Status::Securing}).map_err(|_|"Stream interface closed")?;
					waiting_announced=waiting;
				}
				let secure=(!dave_enabled || (dave.ready&&dave.session.is_ready()&&!dave.transitioning()))&&encryption.is_some()&&!discovering;
				// A rekey can begin after initial readiness cleared the allocation deadline.
				if (!secure && !waiting) || dave.transitioning() {deadline.get_or_insert(now+Duration::from_secs(30));}
				if secure && let Some(video)=&video && let Some(target)=rate.tick(now) {
					video.bitrate.store(target,Ordering::Release);
				}
				if secure && !announced {
					invalidate_stream(&mut video, &mut share_audio);
					if let Some(video)=&video {
						if let Some(event)=soundshare_announcement(&mut share_audio,audio_ssrc) {json_send(&mut ws,event).await?;}
						let streams=json!([{"type":"video","rid":"100","ssrc":video_ssrc,"active":true,"quality":100,"rtx_ssrc":rtx_ssrc,"max_bitrate":video.settings.bit_rate(),"max_framerate":video.settings.fps,"max_resolution":{"type":"fixed","width":video.settings.width,"height":video.settings.height}}]);
						json_send(&mut ws,json!({"op":12,"d":{"audio_ssrc":audio_ssrc,"video_ssrc":video_ssrc,"rtx_ssrc":rtx_ssrc,"streams":streams}})).await?;
						awaiting_keyframe=true;video.keyframe.store(true, Ordering::Release); video.ready.store(true, Ordering::Release);
					} else {
						json_send(&mut ws,json!({"op":12,"d":{"audio_ssrc":audio_ssrc,"video_ssrc":0,"rtx_ssrc":0,"streams":[]}})).await?;
						metrics.signal(Signal::SubscribeSent,1);
						json_send(&mut ws,json!({"op":15,"d":{"any":100}})).await?;
						metrics.signal(Signal::SinkWantsSent,1);
						next_sink_wants=now+SINK_WANTS_INTERVAL;
						emit(Status::Ready{privacy_code:if dave_enabled {dave.session.voice_privacy_code().unwrap_or_default().into()} else {String::new()}}).map_err(|_|"Stream interface closed")?;
					}
					announced=true; if !dave.transitioning() {deadline=None;}
				}
				if !secure && announced {announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);emit(Status::Securing).map_err(|_|"Stream interface closed")?;}
				metrics.stream_state([
					encryption.is_some() && !discovering, dave.ready, dave.session.is_ready(),
					dave.transitioning(), waiting, announced,
					video.as_ref().is_some_and(|video|video.ready.load(Ordering::Acquire)),
					share_audio.is_some() || audio.is_some(),
				], video.as_ref().and_then(|video|video.audio.as_ref()).map_or(0,|source|source.len()));
				if let Some(audio)=&audio {
					if now.saturating_duration_since(receive_tick)>=Duration::from_millis(80) {mixer.clear();}
					receive_tick=now;
					if secure {
						let start=metrics.start();
						if let (Some(frame),_)=mixer.pop() {
							let dropped=audio.try_send(frame).is_err();
							metrics.finish(crate::diagnostics::Stage::Mix,start);
							metrics.poll(false,u64::from(dropped),false,0);
						}
					} else {mixer.clear();}
				}
				let stalled=watch.tick(&mut metrics,&mut receivers,decoder.as_ref(),now);
				// A viewer's subscription lapses silently and video stops with no loss to
				// observe; refresh the sink wants while watching, and sooner while stalled.
				if secure && announced && video.is_none() && now>=next_sink_wants {
					json_send(&mut ws,json!({"op":15,"d":{"any":100}})).await?;
					metrics.signal(Signal::SinkWantsSent,1);
					next_sink_wants=now+if stalled {SINK_WANTS_STALLED_INTERVAL} else {SINK_WANTS_INTERVAL};
				}
				if secure && now>=next_pli && let Some(lost)=&lost && let Some(crypto)=encryption.as_mut() && let Some(socket)=&udp {
					receivers.absorb(lost);
					let requests:Vec<u32>=receivers.keyframe_requests().collect();
					metrics.video(Video::PliSent,requests.len() as u64);
					for media in requests {let (header,body)=pli(audio_ssrc,media);udp_failures.send(socket,&crypto.seal_rtcp(&header,&body)?,"Stream RTCP send failed")?;}
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
					let data=if dave_enabled && dave.ready {
						dave.session
							.encrypt_opus(&audio_encoded[..length])
							.map_err(|_| "DAVE screen audio encryption failed")?
							.into_owned()
					} else {
						audio_encoded[..length].to_vec()
					};
					let mut header=[0;12];header[0]=0x80;header[1]=120;header[2..4].copy_from_slice(&audio_sequence.to_be_bytes());header[4..8].copy_from_slice(&audio_timestamp.to_be_bytes());header[8..12].copy_from_slice(&audio_ssrc.to_be_bytes());
					let crypto=encryption.as_mut().ok_or("Missing stream transport key")?;
					let socket=udp.as_ref().ok_or("Missing stream UDP socket")?;
					udp_failures.send(socket,&crypto.seal_soundshare(&header,&data)?,"Stream audio UDP send failed")?;
					metrics.finish(crate::diagnostics::Stage::Encode,start);
					audio_sequence=audio_sequence.wrapping_add(1);
				}
				audio_timestamp=audio_timestamp.wrapping_add(960);
				metrics.poll(false,0,false,0);
			},
			frame=async {match video.as_mut() {Some(video)=>video.frames.recv().await,None=>std::future::pending().await}}, if outgoing.is_empty()=>{
				let Some(frame)=frame else {return Ok(());};
				if frame.data.len()>2*1024*1024 {return Err("Encoded stream frame exceeds the sharing limit");}
				let secure=announced&&(!dave_enabled || (dave.ready&&dave.session.is_ready()&&!dave.transitioning()))&&encryption.is_some()&&!discovering&&video.as_ref().is_some_and(|video|video.ready.load(Ordering::Acquire));
				if !secure {awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);continue;}
				if awaiting_keyframe && !frame.keyframe {continue;}
				let start=metrics.start();
				let normalized=crate::video_sps::normalize(&frame.data)?;
				let encrypted=if dave_enabled {
					dave.session.encrypt(davey::MediaType::VIDEO,davey::Codec::H264,&normalized).map_err(|_|"DAVE H264 encryption failed")?
				} else {
					normalized
				};
				let packets=crate::video::packetize(&encrypted,&mut sequence,frame.timestamp,video_ssrc)?;
				outgoing.queue(packets,Instant::now());
				outgoing_keyframe=frame.keyframe;
				metrics.finish(crate::diagnostics::Stage::VideoSend,start);
			},
			result=async {match &udp {Some(socket)=>socket.recv(&mut packet).await,_=>std::future::pending().await}}=>{
				let length=match result {Ok(length)=>length,Err(error) if transient_receive(&error)=>continue,Err(_)=>return Err("Stream UDP receive failed")};
				if discovering {
					let Ok((address, port)) = stream_discovery(&packet[..length], audio_ssrc, video_ssrc) else {
						continue;
					};
					json_send(&mut ws,json!({"op":1,"d":{"protocol":"udp","data":{"address":address.to_string(),"port":port,"mode":MODE},"codecs":[{"name":"opus","type":"audio","priority":1000,"payload_type":120},{"name":"H264","type":"video","priority":1000,"payload_type":101,"rtx_payload_type":102,"encode":video.is_some(),"decode":decoder.is_some()}]}})).await?;
					discovering=false;
					continue;
				}
				if length>MAX_PACKET {continue;}
				let Some(crypto)=&encryption else {continue;};
				if let Some(video)=&video && let Some(feedback)=crypto.feedback(&packet[..length],video_ssrc) {
					if announced && (!dave_enabled || (dave.ready && !dave.transitioning())) && video.ready.load(Ordering::Acquire) {
						let now=Instant::now();
						rate.observe(feedback.loss,feedback.bitrate);
						let missing=if rtx_ssrc!=0 {history.request(&feedback.nacks,now)} else {!feedback.nacks.is_empty()};
						if (feedback.keyframe || missing) && now>=next_keyframe {
							video.keyframe.store(true,Ordering::Release);
							next_keyframe=now+Duration::from_millis(500);
						}
					}
					continue;
				}
				let Some(mut rtp)=crypto.open(&packet[..length]) else {metrics.video(Video::OpenFailed,1);continue;};
				if rtp.payload_type==102 {metrics.video(Video::Rtx,1);let Some((media,sequence))=receivers.restore_rtx(rtp.ssrc,&mut rtp.payload) else{continue;};rtp.ssrc=media;rtp.sequence=sequence;rtp.payload_type=101;}
				if rtp.payload_type==101 {metrics.video(Video::Packets,1);}
				if rtp.payload_type==120 {
					if audio.is_none() {continue;}
					let Some(user)=mixer.user(rtp.ssrc) else {continue;};
					let start=metrics.start();
					let opus = match receive_media(&mut dave, user, davey::MediaType::AUDIO, rtp.payload, receive_passthrough, dave_enabled || receive_keys_until.is_some_and(|until| Instant::now() < until)) {
						Ok(data) => data,
						Err(()) => { continue; }
					};
					metrics.finish(crate::diagnostics::Stage::Receive,start);
					mixer.push(rtp.ssrc,rtp.sequence,opus);
					continue;
				}
				let Some(decoder)=&decoder else {continue;};
				if rtp.payload_type!=101 {continue;}
				let Some((user,frame))=receivers.push(rtp.ssrc,rtp.sequence,rtp.timestamp,rtp.marker,&rtp.payload) else {continue;};
				if dave_enabled && !dave.session.is_ready() {metrics.video(Video::NotReady,1);}
				let start=metrics.start();
				let data = match receive_media(&mut dave, user, davey::MediaType::VIDEO, frame, receive_passthrough, dave_enabled || receive_keys_until.is_some_and(|until| Instant::now() < until)) {
					Ok(data) => data,
					Err(()) => { receivers.require_keyframe(user);metrics.video(Video::DecryptFailed,1);continue; }
				};
				let keyframe=is_keyframe(&data);
				if keyframe {metrics.video(Video::Keyframes,1);metrics.video(Video::KeyframesWithoutParams,u64::from(!has_parameter_sets(&data)));}
				if !receivers.accept(user,keyframe) {metrics.video(Video::Gated,1);continue;}
				if !offer(decoder,Encoded{user,data,keyframe})? {receivers.require_keyframe(user);metrics.poll(false,1,false,0);metrics.video(Video::QueueFull,1);} else {metrics.finish(crate::diagnostics::Stage::VideoReceive,start);}
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
						metrics.signal(Signal::Text,1);
						metrics.signal(signal_of(op),1);
						match op {
							8=>{let interval=data["heartbeat_interval"].as_f64().filter(|v|v.is_finite()&&*v>=100.&&*v<=120000.).ok_or("Invalid stream heartbeat")? as u64; heartbeat_ms=Some(if cfg!(test) { interval.min(5000) } else { interval });heartbeat_at=Instant::now();},
							6=>{let nonce = data.as_u64().or_else(|| data["t"].as_u64()); if acknowledges(awaiting_ack,missed_acks,nonce) {awaiting_ack=None;missed_acks=0;}},
							2=>{
								if udp.is_some(){return Err("Unexpected stream transport replacement");}
								audio_ssrc=u32::try_from(number(data,"ssrc")?).ok().filter(|ssrc|*ssrc!=0).ok_or("Invalid stream SSRC")?;
								if let Some(stream) = data["streams"].as_array().and_then(|v| v.first()) {
									let ssrc = u32::try_from(number(stream, "ssrc")?).map_err(|_| "Invalid stream video SSRC")?;
									let rtx = stream.get("rtx_ssrc").map(|value| value.as_u64().and_then(|value| u32::try_from(value).ok()).ok_or("Invalid stream retransmission SSRC")).transpose()?.unwrap_or(0);
									if ssrc == 0 || ssrc == audio_ssrc || (rtx != 0 && (rtx == ssrc || rtx == audio_ssrc)) {
										return Err("Invalid stream video SSRC assignment");
									}
									video_ssrc = ssrc;
									rtx_ssrc = rtx;
								} else if video.is_some() {
									return Err("Discord did not assign a stream SSRC");
								}
								let address:IpAddr=data["ip"].as_str().ok_or("Missing stream server address")?.parse().map_err(|_|"Invalid stream server address")?;
								if !public_ip(address) && !(cfg!(test) && local_test && address.is_loopback()){return Err("Stream server advertised a nonpublic address");}
								let port=u16::try_from(number(data,"port")?).ok().filter(|port|*port>0).ok_or("Invalid stream server port")?;
								if !data["modes"].as_array().is_some_and(|m|m.iter().any(|mode|mode.as_str()==Some(MODE))){return Err("Required stream transport encryption is unavailable");}
								let socket=UdpSocket::bind(if address.is_ipv4(){"0.0.0.0:0"}else{"[::]:0"}).await.map_err(|_|"Could not bind stream UDP socket")?;
								socket.connect(SocketAddr::new(address,port)).await.map_err(|_|"Could not connect stream UDP socket")?;
								let mut probe=[0;74];probe[..4].copy_from_slice(&[0,1,0,70]);probe[4..8].copy_from_slice(&audio_ssrc.to_be_bytes());socket.send(&probe).await.map_err(|_|"Stream UDP discovery failed")?;
								udp=Some(socket);discovering=true;discovery_deadline=Instant::now()+Duration::from_secs(15);next_discovery_probe=Instant::now()+Duration::from_millis(300);discovery_probe_alt=true;emit(Status::Discovering).map_err(|_|"Stream interface closed")?;
							},
							4=>{
								if encryption.is_some()||udp.is_none()||discovering{return Err("Unexpected stream session description");}
								if data["mode"].as_str()!=Some(MODE)||(video.is_some()&&!h264_negotiated(data)){return Err("Discord did not negotiate stream transport media");}
								let values=data["secret_key"].take();let values=values.as_array().ok_or("Missing stream transport key")?;if values.len()!=32{return Err("Invalid stream transport key");}
								let mut key=Zeroizing::new([0;32]);for(out,value)in key.iter_mut().zip(values){*out=value.as_u64().and_then(|value|u8::try_from(value).ok()).ok_or("Invalid stream transport key")?;}
								encryption=Some(Encryption::new(&key));secured_at=Some(Instant::now());
								let dave_ver = data
									.get("dave_protocol_version")
									.map(|v| v.as_u64().ok_or("Invalid DAVE protocol version"))
									.transpose()?
									.unwrap_or(0);
								if dave_ver == 0 {
									dave_enabled = false;
									receive_passthrough = true;
									receive_keys_until = None;
								} else if dave_ver != 1 {
									return Err("Unsupported stream DAVE version");
								} else {
									dave_enabled = true;
									receivers.retain(|u| dave.contains(u));
									mixer.retain(|u| dave.contains(u));
									if let Some(decoder) = decoder.as_ref() {
										retain_sources(decoder, &receivers);
									}
									send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;
									metrics.signal(Signal::KeyPackageSent,1);
								}
								emit(Status::TransportReady).map_err(|_|"Stream interface closed")?;emit(Status::Securing).map_err(|_|"Stream interface closed")?;
							},
							11=>{
								let ids=participant_ids(data)?;
								if dave_enabled {
									let was_ready=dave.ready;
									if dave.connect(&ids)? {
										if was_ready || dave.session.is_ready() {
											dave.ready=true;
										} else {
											announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);
										}
									}
								}
							},
							5=>{
								let user = if data.get("user_id").is_some() {
									Some(id(data, "user_id")?)
								} else {
									streamer
								};
								if let Some(user) = user
									&& user!=credentials.user.0 {
									let was_ready = dave.ready;
									match dave.connect(&[user]) {
										Ok(_) => {
											if was_ready || dave.session.is_ready() { dave.ready = true; }
										}
										Err("Voice channel exceeds the 64 participant limit") if !dave_enabled || encryption.is_none() => {}
										Err(_) => continue,
									}
									if audio.is_some() {
										let value=u32::try_from(number(data,"ssrc")?).map_err(|_|"Invalid stream SSRC")?;
										let _ = mixer.announce(user,value);
									}
								}
							},
							12=>{
								let user = if data.get("user_id").is_some() {
									Some(id(data, "user_id")?)
								} else {
									streamer
								};
								if let Some(user) = user
									&& user!=credentials.user.0 {
									let was_ready = dave.ready;
									match dave.connect(&[user]) {
										Ok(_) => {
											if was_ready || dave.session.is_ready() { dave.ready = true; }
										}
										Err("Voice channel exceeds the 64 participant limit") if !dave_enabled || encryption.is_none() => {}
										Err(_) => continue,
									}
									if let Some(decoder) = decoder.as_ref() {
										let _ = announce_video(&mut receivers, decoder, user, data);
										next_sink_wants = Instant::now();
									}
									if audio.is_some() && let Some(value)=data["audio_ssrc"].as_u64().and_then(|v|u32::try_from(v).ok()).filter(|v|*v!=0) {let _ = mixer.announce(user,value);}
								}
							},
							13=>{let user=id(data,"user_id")?;receivers.remove(user);if let Some(decoder)=decoder.as_ref(){remove_decoder(decoder,user);}mixer.remove(user);if !dave_enabled {let _ = dave.disconnect(user);} else {let was_group_member=dave.is_group_member(user);let was_ready=dave.ready;if dave.disconnect(user)?{if dave.alone(){announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);dave.enter_sole_member_waiting()?;deadline=None;}else if was_ready{dave.ready=true;deadline=None;}else if was_group_member{announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);}}}},
							21=>{
								let (Ok(proto), Ok(id)) = (number(data, "protocol_version"), transition(data)) else { continue; };
								if proto != 0 && proto != 1 { return Err("Discord requested an unsupported DAVE protocol version"); }
								announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);
								if proto==0 {
									receive_passthrough=true;
									dave.prepare_protocol(id);
									if id==0 {
										dave.execute(0)?;
										dave_enabled=false;
										receive_keys_until=Some(Instant::now()+Duration::from_secs(10));
										dave.ready=false;
										pending_downgrade=None;
									} else {
										pending_downgrade=Some(id);
										json_send(&mut ws,json!({"op":23,"d":{"transition_id":id}})).await?;
									}
								} else if proto==1 {
									pending_downgrade=None;
									if id==0 {dave_enabled=true;receive_passthrough=false;receive_keys_until=None;}
									else {receive_keys_until=Some(Instant::now()+Duration::from_secs(30));}
									dave.prepare_protocol(id);
									if id==0{if dave.session.is_ready(){dave.execute(0)?;}else if dave.alone(){dave.enter_sole_member_waiting()?;deadline=None;}else{dave.pending_protocol=None;dave.ready=false;}}else{json_send(&mut ws,json!({"op":23,"d":{"transition_id":id}})).await?;}
								}
							},
							22=>{
								let tid = transition(data)?;
								let protocol_change = dave.pending_protocol == Some(tid);
								dave.execute(tid)?;
								announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);
								if pending_downgrade == Some(tid) {
									pending_downgrade = None;
									dave_enabled = false;
									receive_passthrough = true;
									receive_keys_until = Some(Instant::now()+Duration::from_secs(10));
									dave.ready = false;
								} else if protocol_change {
									dave_enabled=true;receive_passthrough=false;receive_keys_until=None;
								}
							},
							24=>{if number(data,"protocol_version")?!=1{return Err("Unsupported stream DAVE version");}
							if number(data,"epoch")?==1{announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);dave.reinitialize()?;send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;}},
							// Watching a stream receives signaling the sender never does; unknown
							// opcodes are ignored under the bounded rate above, never fatal.
							_=>{},
						}
					},
					Message::Binary(bytes)=>{if bytes.len()<3{return Err("Truncated stream DAVE signaling");}seq_ack=i64::from(u16::from_be_bytes([bytes[0],bytes[1]]));metrics.signal(Signal::Binary,1);metrics.signal(match bytes[2]{25=>Signal::ExternalSender,27=>Signal::Proposals,29|30=>Signal::Commit,_=>Signal::Other},1);match bytes[2]{25=>dave.session.set_external_sender(&bytes[3..]).map_err(|_|"Stream DAVE external sender validation failed")?,27=>if let Some(response)=dave.proposals(&bytes[3..])?{send(&mut ws,Message::Binary(response.into())).await?;},29|30=>{announced=false;awaiting_keyframe=true;invalidate_stream(&mut video, &mut share_audio);match dave.group_changed(bytes[2],&bytes[3..]){Ok(id)=>if id!=0{json_send(&mut ws,json!({"op":23,"d":{"transition_id":id}})).await?;metrics.signal(Signal::TransitionReadySent,1);},Err(_)=>{if bytes.len()<5{return Err("Truncated stream DAVE transition");}let id=u16::from_be_bytes([bytes[3],bytes[4]]);json_send(&mut ws,json!({"op":31,"d":{"transition_id":id}})).await?;dave.reset()?;send(&mut ws,Message::Binary(dave.key_package()?.into())).await?;metrics.signal(Signal::KeyPackageSent,1);}}},_=>return Err("Unsupported stream DAVE opcode")}},
					Message::Ping(data)=>send(&mut ws,Message::Pong(data)).await?, Message::Close(_)=>return Err("Discord stream connection closed"), _=>{}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn participant_roster_keeps_the_last_member_and_rejects_invalid_input() {
		let ids: Vec<_> = (1..=64).map(|id| id.to_string()).collect();
		let parsed = participant_ids(&json!({"user_ids": ids})).unwrap();
		let mut dave = Dave::new(1, None, 3).unwrap();
		dave.connect(&parsed).unwrap();
		assert!(dave.contains(64));
		assert!(participant_ids(&json!({"user_ids": vec![1; 65]})).is_err());
		assert!(participant_ids(&json!({"user_ids": [1, 0]})).is_err());
		assert!(participant_ids(&json!({"user_ids": ["invalid"]})).is_err());
	}

	#[test]
	fn incoming_media_authenticates_dave_even_when_plaintext_is_permitted() {
		let server = crate::test_mls::Delivery::new();
		let mut alice = Dave::new(1, Some(2), 3).unwrap();
		let mut bob = Dave::new(2, Some(1), 3).unwrap();
		alice.session.set_external_sender(&server.external).unwrap();
		bob.session.set_external_sender(&server.external).unwrap();
		let (commit, welcome) = server.add(&mut alice, &bob.key_package().unwrap());
		alice
			.group_changed(29, &[&[0, 0], commit.as_slice()].concat())
			.unwrap();
		bob.group_changed(30, &[&[0, 0], welcome.as_slice()].concat())
			.unwrap();
		let clear = b"synthetic Opus";
		let encrypted = bob.session.encrypt_opus(clear).unwrap().into_owned();
		assert_eq!(
			super::receive_media(
				&mut alice,
				2,
				davey::MediaType::AUDIO,
				encrypted.clone(),
				false,
				true
			)
			.unwrap(),
			clear
		);
		assert!(
			super::receive_media(
				&mut alice,
				2,
				davey::MediaType::AUDIO,
				clear.to_vec(),
				false,
				true
			)
			.is_err()
		);
		assert_eq!(
			super::receive_media(
				&mut alice,
				2,
				davey::MediaType::AUDIO,
				clear.to_vec(),
				true,
				true
			)
			.unwrap(),
			clear
		);
		assert!(
			super::receive_media(
				&mut alice,
				2,
				davey::MediaType::AUDIO,
				encrypted.clone(),
				true,
				false
			)
			.is_err()
		);
		let mut forged = encrypted;
		forged[0] ^= 1;
		assert!(
			super::receive_media(&mut alice, 2, davey::MediaType::AUDIO, forged, true, true)
				.is_err()
		);
		assert_eq!(
			super::receive_media(
				&mut alice,
				2,
				davey::MediaType::AUDIO,
				vec![1, 0xfa, 0xfa],
				true,
				false
			)
			.unwrap(),
			[1, 0xfa, 0xfa]
		);
		assert!(
			super::receive_media(
				&mut alice,
				2,
				davey::MediaType::VIDEO,
				clear.to_vec(),
				false,
				true
			)
			.is_err()
		);
	}
	use crate::video_receive::Receivers;
	use opus2::Decoder;

	#[tokio::test]
	async fn udp_send_errors_drop_datagrams_without_waiting_and_recover() {
		let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
		let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
		sender
			.connect(receiver.local_addr().unwrap())
			.await
			.unwrap();
		sender.writable().await.unwrap();
		let mut failures = UdpFailures::default();
		// An oversized datagram gives a real socket error without needing a live outage.
		let invalid = vec![0; 65_536];
		assert!(failures.send(&sender, &invalid, "outage").is_ok());
		assert!(failures.0.is_some());
		failures.0 = Some(Instant::now() - UDP_OUTAGE);
		assert_eq!(failures.send(&sender, &invalid, "outage"), Err("outage"));
		assert!(failures.send(&sender, b"recovered", "outage").is_ok());
		assert!(failures.0.is_none());
		let mut packet = [0; 16];
		let length = timeout(Duration::from_secs(1), receiver.recv(&mut packet))
			.await
			.unwrap()
			.unwrap();
		assert_eq!(&packet[..length], b"recovered");
	}

	#[test]
	fn decoder_cleanup_announcement_preserves_streams_and_partial_updates() {
		let (decoder, _) = spawn_decoder(Arc::new(|_| {})).unwrap();
		let mut receivers = Receivers::default();
		let streams = json!({"video_ssrc": 0, "streams": [
			{"ssrc": 700, "rtx_ssrc": 701},
			{"ssrc": 710, "rtx_ssrc": 711}
		]});
		announce_video(&mut receivers, &decoder, 7, &streams).unwrap();
		assert!(receivers.has_sources());
		assert_eq!(
			receivers.push(700, 1, 90, true, &[0x65, 1]),
			Some((7, vec![0, 0, 0, 1, 0x65, 1]))
		);
		// An absent list is a partial update, so other sources stay bound.
		announce_video(
			&mut receivers,
			&decoder,
			7,
			&json!({"video_ssrc": 710, "rtx_ssrc": 711}),
		)
		.unwrap();
		assert!(receivers.push(700, 2, 180, true, &[0x65, 3]).is_some());
		assert_eq!(
			receivers.restore_rtx(711, &mut vec![0, 4, 0x65]),
			Some((710, 4))
		);
		assert_eq!(
			receivers.restore_rtx(701, &mut vec![0, 4, 0x65]),
			Some((700, 4))
		);
		assert!(
			receivers
				.push(710, 1, 90, false, &[0x7c, 0x85, 1])
				.is_none()
		);
		announce_video(
			&mut receivers,
			&decoder,
			7,
			&json!({"video_ssrc": 710, "streams": [{"ssrc": 710, "rtx_ssrc": 711}]}),
		)
		.unwrap();
		assert!(receivers.push(700, 3, 270, true, &[0x65, 4]).is_none());
		assert!(receivers.restore_rtx(701, &mut vec![0, 4, 0x65]).is_none());
		assert_eq!(
			receivers.push(710, 2, 90, true, &[0x7c, 0x45, 2]),
			Some((7, vec![0, 0, 0, 1, 0x65, 1, 2]))
		);
		assert_eq!(receivers.keyframe_requests().collect::<Vec<_>>(), vec![710]);
		announce_video(
			&mut receivers,
			&decoder,
			7,
			&json!({"video_ssrc": 720, "streams": []}),
		)
		.unwrap();
		assert!(receivers.push(710, 3, 180, true, &[0x65, 4]).is_none());
		assert_eq!(receivers.keyframe_requests().collect::<Vec<_>>(), vec![720]);
		announce_video(
			&mut receivers,
			&decoder,
			7,
			&json!({"video_ssrc": 0, "streams": []}),
		)
		.unwrap();
		assert!(!receivers.has_sources());
		assert!(receivers.push(710, 5, 270, true, &[0x65, 4]).is_none());
	}

	#[test]
	fn a_stall_asks_every_announced_sender_for_a_keyframe() {
		let mut metrics =
			crate::diagnostics::Metrics::new(crate::diagnostics::Scope::StreamReceive);
		let mut watch = VideoWatch::new();
		let mut receivers = Receivers::default();
		let (decoder, _lost) =
			crate::video_receive::spawn_decoder(std::sync::Arc::new(|_| {})).expect("decoder");
		let later = Instant::now() + VIDEO_STALL * 2;
		// No announced source yet: elapsed time alone must not manufacture a request.
		assert!(!watch.tick(&mut metrics, &mut receivers, Some(&decoder), later));
		assert!(!receivers.awaiting());
		// Without a decoder there is nothing to keep alive, so no request is made either.
		receivers.announce(9, 900).unwrap();
		assert!(!watch.tick(&mut metrics, &mut receivers, None, later));
		receivers.remove(9);
		receivers.announce(7, 700).unwrap();
		receivers.announce(8, 800).unwrap();
		// A delivered keyframe from each sender clears what announce owed.
		for ssrc in [700, 800] {
			assert!(receivers.push(ssrc, 1, 900, true, &[0x65, 1]).is_some());
		}
		assert!(receivers.accept(7, true) && receivers.accept(8, true));
		assert!(!receivers.awaiting());
		// Video stops with nothing marked lost; only the elapsed-time path can recover it.
		assert!(watch.tick(&mut metrics, &mut receivers, Some(&decoder), later));
		assert_eq!(
			receivers.keyframe_requests().collect::<Vec<_>>(),
			vec![700, 800]
		);
	}

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
			bitrate: Arc::new(std::sync::atomic::AtomicU32::new(4_000_000)),
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

		{
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
	}
	#[test]
	fn validated_endpoints_discovery_and_real_opus() {
		assert_eq!(
			endpoint("voice-1.discord.media:443").unwrap(),
			"wss://voice-1.discord.media/?v=8"
		);
		assert_eq!(
			endpoint("c-bru01-67e5a38a.discord.media:2053").unwrap(),
			"wss://c-bru01-67e5a38a.discord.media:2053/?v=8"
		);
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
		let mut padded_reply = [0; 82];
		padded_reply[..74].copy_from_slice(&reply);
		assert_eq!(discovery(&padded_reply, 42).unwrap().1, 1234);
		assert_eq!(stream_discovery(&reply, 42, 99).unwrap().1, 1234);
		assert_eq!(stream_discovery(&reply, 99, 42).unwrap().1, 1234);
		assert_eq!(stream_discovery(&reply, 99, 100).unwrap().1, 1234);
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

		{
			assert!(h264_negotiated(&json!({"video_codec":"H264"})));
			assert!(!h264_negotiated(&json!({"video_codec":"VP8"})));
			assert!(!h264_negotiated(
				&json!({"sdp":"a=rtpmap:101 H264/90000\\r\\n"})
			));
		}
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
		let (downgraded_tx, mut downgraded_rx) = tokio::sync::oneshot::channel();
		let (upgraded_tx, mut upgraded_rx) = tokio::sync::oneshot::channel();
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
			if !guild {
				for (version, transition) in [(0, 10), (1, 11)] {
					ws.send(Message::Text(json!({"op":21,"d":{"protocol_version":version,"transition_id":transition}}).to_string().into())).await.unwrap();
					loop {
						let message = ws.next().await.unwrap().unwrap();
						let Ok(text) = message.to_text() else {
							continue;
						};
						let event: Value = serde_json::from_str(text).unwrap();
						if event["op"] == 3 {
							ws.send(Message::Text(
								json!({"op":6,"d":{"t":event["d"]["t"]}}).to_string().into(),
							))
							.await
							.unwrap();
						} else if event["op"] == 23 {
							assert_eq!(event["d"]["transition_id"], transition);
							break;
						}
					}
					ws.send(Message::Text(
						json!({"op":22,"d":{"transition_id":transition}})
							.to_string()
							.into(),
					))
					.await
					.unwrap();
					if version == 0 {
						(&mut downgraded_rx).await.unwrap();
					} else {
						(&mut upgraded_rx).await.unwrap();
					}
				}
			}
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
			let expected_ready = if guild { 2 } else { 3 };
			let mut privacy_code_before = None;
			let mut downgraded_tx = Some(downgraded_tx);
			let mut upgraded_tx = Some(upgraded_tx);
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
						Status::Ready { privacy_code } => {
							if privacy_code.is_empty() {
								assert!(!guild);
								if let Some(tx) = downgraded_tx.take() { tx.send(()).unwrap(); }
								continue;
							}
							if !guild {
								if let Some(before) = &privacy_code_before { assert_eq!(before, &privacy_code); }
								else { privacy_code_before = Some(privacy_code); }
							}
							ready += 1;
							if !guild && ready == 2 { upgraded_tx.take().unwrap().send(()).unwrap(); }
							if ready == expected_ready {
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
						if ready == expected_ready {
							resumed_pcm = playback_rx.try_recv().ok();
						}
					}
				}
				if ready == expected_ready && heard && captured && let Some(pcm) = resumed_pcm.take() {
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
				Err(DISCONNECTED_BY_DISCORD)
			);
			drop(control_tx);
		} else {
			drop(control_tx);
			assert!(task.await.unwrap().is_ok());
			done_tx.send(()).unwrap();
		}
		server.await.unwrap();
	}

	#[tokio::test]
	async fn local_voice_peer_disconnect_enters_waiting_without_timeout() {
		use client_core::voice::Secret;
		use model::Id;
		use tokio::net::TcpListener;
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
		let port = udp.local_addr().unwrap().port();
		let (waiting_peer_tx, waiting_peer_rx) = tokio::sync::oneshot::channel();
		let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

		let server = tokio::spawn(async move {
			let (tcp, _) = listener.accept().await.unwrap();
			let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
			let _identify = ws.next().await.unwrap().unwrap();
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
			let mut bob = Dave::new(2, Some(1), 3).unwrap();
			bob.session.set_external_sender(&delivery.external).unwrap();

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
			ws.send(Message::Text(
				json!({"op":11,"d":{"user_ids":["1","2"]}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();

			let package = loop {
				match ws.next().await.unwrap().unwrap() {
					Message::Binary(bytes) => break bytes,
					Message::Text(text) => {
						let value: Value = serde_json::from_str(&text).unwrap();
						if value["op"] == 3 {
							ws.send(Message::Text(
								json!({"op":6,"d":{"t":value["d"]["t"]}}).to_string().into(),
							))
							.await
							.unwrap();
						}
					}
					_ => {}
				}
			};
			let (commit, welcome) = delivery.add(&mut bob, &package);
			let mut committed = vec![0, 0];
			committed.extend(commit);
			bob.group_changed(29, &committed).unwrap();
			assert!(bob.ready);
			let mut welcome_frame = vec![0, 2, 30, 0, 0];
			welcome_frame.extend(welcome);
			ws.send(Message::Binary(welcome_frame.into()))
				.await
				.unwrap();

			// Wait until the client has reached Ready.
			ready_rx.await.unwrap();

			// Bob departs (leaves the DM call). Serein is now the sole member.
			ws.send(Message::Text(
				json!({"op":13,"d":{"user_id":"2"}}).to_string().into(),
			))
			.await
			.unwrap();

			// Wait until client transitions to WaitingForPeer.
			waiting_peer_rx.await.unwrap();

			// Also send an Opcode 11 and 13 for an unannounced / non-MLS participant (e.g. quick connect/disconnect)
			// to verify it does not trigger a negotiation timeout or crash.
			ws.send(Message::Text(
				json!({"op":13,"d":{"user_id":"999"}}).to_string().into(),
			))
			.await
			.unwrap();
		});

		let credentials = VoiceConnection {
			channel: Id(3),
			user: Id(1),
			peer: Some(Id(2)),
			guild: None,
			session: Secret::new("synthetic-session".into()).unwrap(),
			token: Secret::new("synthetic-token".into()).unwrap(),
			endpoint: "not-used-in-test".into(),
			request: 1,
		};
		let (_capture_tx, capture) = std::sync::mpsc::sync_channel(8);
		let (playback, _playback_rx) = std::sync::mpsc::sync_channel(8);
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

		let test = timeout(Duration::from_secs(5), async {
			let mut ready_sent = false;
			let mut ready_tx = Some(ready_tx);
			let mut waiting_peer_tx = Some(waiting_peer_tx);
			while let Some(status) = status_rx.recv().await {
				match status {
					Status::Ready { .. } => {
						if !ready_sent {
							ready_sent = true;
							ready_tx.take().unwrap().send(()).unwrap();
						}
					}
					Status::WaitingForPeer => {
						if let Some(tx) = waiting_peer_tx.take() {
							tx.send(()).unwrap();
							break;
						}
					}
					_ => {}
				}
			}
		});
		test.await.unwrap();
		drop(control_tx);
		let res = task.await.unwrap();
		assert!(res.is_ok());
		server.await.unwrap();
	}
	#[tokio::test]
	async fn local_voice_dave_downgrade_transition_disables_dave_and_announces_ready() {
		local_voice_dave_transition(true).await;
	}

	#[tokio::test]
	async fn local_voice_dave_negotiation_has_a_deadline_after_transport_ready() {
		local_voice_dave_transition(false).await;
	}

	async fn local_voice_dave_transition(execute: bool) {
		use client_core::voice::Secret;
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
		let udp_address = udp.local_addr().unwrap();

		let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
		let (prepared_tx, prepared_rx) = tokio::sync::oneshot::channel::<()>();
		let (execute_tx, execute_rx) = tokio::sync::oneshot::channel::<()>();

		let server = tokio::spawn(async move {
			let (stream, _) = listener.accept().await.unwrap();
			let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
			let _identify = ws.next().await.unwrap().unwrap();
			ws.send(Message::Text(
				json!({"op":8,"d":{"heartbeat_interval":5000}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();
			let ready = json!({
				"op": 2,
				"d": {
					"ssrc": 42,
					"ip": "127.0.0.1",
					"port": udp_address.port(),
					"modes": [MODE]
				}
			});
			ws.send(Message::Text(ready.to_string().into()))
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
				break;
			}

			let mut probe = [0u8; 4096];
			let (n, client) = udp.recv_from(&mut probe).await.unwrap();
			assert_eq!(n, 74);
			probe[..4].copy_from_slice(&[0, 2, 0, 70]);
			probe[8..17].copy_from_slice(b"127.0.0.1");
			probe[72..74].copy_from_slice(&client.port().to_be_bytes());
			udp.send_to(&probe[..74], client).await.unwrap();

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

			// Server announces Op 4 with dave_protocol_version: 1 (DAVE enabled)
			ws.send(Message::Text(
				json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":1}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();

			// Read key package (binary) and Op 5 from client
			let mut saw_key_pkg = false;
			let mut saw_op5 = false;
			while !saw_key_pkg || !saw_op5 {
				let message = ws.next().await.unwrap().unwrap();
				match message {
					Message::Binary(_) => saw_key_pkg = true,
					Message::Text(text) => {
						let event: Value = serde_json::from_str(&text).unwrap();
						if event["op"] == 5 {
							saw_op5 = true;
						}
					}
					_ => {}
				}
			}

			// Server sends Op 21 preparing downgrade to non-DAVE with transition_id: 10
			ws.send(Message::Text(
				json!({"op":21,"d":{"protocol_version":0,"transition_id":10}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();

			// Client must acknowledge transition with Op 23
			loop {
				let message = ws.next().await.unwrap().unwrap();
				let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
				if event["op"] == 23 {
					assert_eq!(event["d"]["transition_id"], 10);
					break;
				}
			}

			prepared_tx.send(()).unwrap();
			execute_rx.await.unwrap();

			if execute {
				// Server sends Op 22 executing the matching downgrade transition 10
				ws.send(Message::Text(
					json!({"op":22,"d":{"transition_id":10}}).to_string().into(),
				))
				.await
				.unwrap();
			}

			let _ = ready_rx.await;
		});

		let credentials = VoiceConnection {
			channel: model::Id(3),
			user: model::Id(1),
			peer: None,
			guild: Some(model::Id(100)),
			session: Secret::new("synthetic-session".into()).unwrap(),
			token: Secret::new("synthetic-token".into()).unwrap(),
			endpoint: "not-used-in-test".into(),
			request: 1,
		};
		let (_capture_tx, capture) = std::sync::mpsc::sync_channel(8);
		let (playback, _playback_rx) = std::sync::mpsc::sync_channel(8);
		let (control_tx, control_rx) = watch::channel(Controls::default());
		let (_camera_tx, camera_rx) = std::sync::mpsc::sync_channel(1);
		let (status_tx, mut status_rx) = tokio::sync::mpsc::channel(8);
		let mut task = tokio::spawn(run_inner(
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

		let test = timeout(Duration::from_secs(45), async {
			prepared_rx.await.unwrap();
			// Advance the media tick before allowing execute; no real-time wait is needed.
			tokio::time::pause();
			tokio::time::advance(if execute {
				Duration::from_millis(40)
			} else {
				Duration::from_secs(31)
			})
			.await;
			tokio::task::yield_now().await;
			while let Ok(status) = status_rx.try_recv() {
				assert!(
					!matches!(status, Status::Ready { .. }),
					"Ready before execute"
				);
			}
			tokio::time::resume();
			execute_tx.send(()).unwrap();
			if !execute {
				let reason = (&mut task).await.unwrap().unwrap_err();
				assert!(
					reason.contains("DAVE group negotiation timed out"),
					"{reason}"
				);
				let _ = ready_tx.send(());
				return;
			}
			let mut saw_ready = false;
			while let Some(status) = status_rx.recv().await {
				if let Status::Ready { privacy_code } = status {
					assert!(privacy_code.is_empty());
					saw_ready = true;
					let _ = ready_tx.send(());
					break;
				}
			}
			assert!(saw_ready);
		});
		test.await.unwrap();
		drop(control_tx);
		if execute {
			let res = task.await.unwrap();
			assert!(res.is_ok());
		}
		server.await.unwrap();
	}

	#[tokio::test]
	async fn local_voice_non_dave_channel_ready_immediately() {
		use client_core::voice::Secret;
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
		let udp_address = udp.local_addr().unwrap();

		let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

		let server = tokio::spawn(async move {
			let (stream, _) = listener.accept().await.unwrap();
			let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
			let _identify = ws.next().await.unwrap().unwrap();
			ws.send(Message::Text(
				json!({"op":8,"d":{"heartbeat_interval":5000}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();
			let ready = json!({
				"op": 2,
				"d": {
					"ssrc": 42,
					"ip": "127.0.0.1",
					"port": udp_address.port(),
					"modes": [MODE]
				}
			});
			ws.send(Message::Text(ready.to_string().into()))
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
				break;
			}

			let mut probe = [0u8; 4096];
			let (n, client) = udp.recv_from(&mut probe).await.unwrap();
			assert_eq!(n, 74);
			probe[..4].copy_from_slice(&[0, 2, 0, 70]);
			probe[8..17].copy_from_slice(b"127.0.0.1");
			probe[72..74].copy_from_slice(&client.port().to_be_bytes());
			udp.send_to(&probe[..74], client).await.unwrap();

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

			// Server announces Op 4 with dave_protocol_version: 0 (non-DAVE channel)
			ws.send(Message::Text(
				json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":0}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();

			// Read Op 5 (speaking) from client
			loop {
				let message = ws.next().await.unwrap().unwrap();
				let text = message.to_text().unwrap();
				let event: Value = serde_json::from_str(text).unwrap();
				if event["op"] == 5 {
					break;
				}
			}

			// Server sends Op 21 with protocol_version: 0 and transition_id: 10
			ws.send(Message::Text(
				json!({"op":21,"d":{"protocol_version":0,"transition_id":10}})
					.to_string()
					.into(),
			))
			.await
			.unwrap();

			// Client must acknowledge transition with Op 23
			loop {
				let message = ws.next().await.unwrap().unwrap();
				let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
				if event["op"] == 23 {
					assert_eq!(event["d"]["transition_id"], 10);
					break;
				}
			}
			let _ = ready_rx.await;
		});

		let credentials = VoiceConnection {
			channel: model::Id(3),
			user: model::Id(1),
			peer: None,
			guild: Some(model::Id(100)),
			session: Secret::new("synthetic-session".into()).unwrap(),
			token: Secret::new("synthetic-token".into()).unwrap(),
			endpoint: "not-used-in-test".into(),
			request: 1,
		};
		let (_capture_tx, capture) = std::sync::mpsc::sync_channel(8);
		let (playback, _playback_rx) = std::sync::mpsc::sync_channel(8);
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

		let test = timeout(Duration::from_secs(5), async {
			let mut saw_ready = false;
			while let Some(status) = status_rx.recv().await {
				if let Status::Ready { privacy_code } = status {
					assert!(privacy_code.is_empty());
					saw_ready = true;
					let _ = ready_tx.send(());
					break;
				}
			}
			assert!(saw_ready);
		});
		test.await.unwrap();
		drop(control_tx);
		let res = task.await.unwrap();
		assert!(res.is_ok());
		server.await.unwrap();
	}
	#[test]
	fn voice_remote_video_sends_sink_wants_when_ready() {
		use client_core::voice::Secret;
		use tokio::net::TcpListener;

		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		runtime.block_on(async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let address = listener.local_addr().unwrap();
			let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
			let udp_addr = udp.local_addr().unwrap();
			let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
			let (sink_wants_tx, sink_wants_rx) = tokio::sync::oneshot::channel();

			let server = tokio::spawn(async move {
				let (tcp, _) = listener.accept().await.unwrap();
				let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
				let handshake: Value =
					serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
				assert_eq!(handshake["op"], 0);

				ws.send(Message::Text(
					json!({"op":8,"d":{"heartbeat_interval":5000}}).to_string().into(),
				))
				.await
				.unwrap();

				ws.send(Message::Text(
					json!({"op":2,"d":{"ssrc":42,"ip":udp_addr.ip().to_string(),"port":udp_addr.port(),"modes":[MODE]}})
						.to_string()
						.into(),
				))
				.await
				.unwrap();

				let mut probe = [0u8; 4096];
				let (n, client) = udp.recv_from(&mut probe).await.unwrap();
				assert_eq!(n, 74);
				probe[..4].copy_from_slice(&[0, 2, 0, 70]);
				probe[8..17].copy_from_slice(b"127.0.0.1");
				probe[72..74].copy_from_slice(&client.port().to_be_bytes());
				udp.send_to(&probe[..74], client).await.unwrap();

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
					break;
				}

				ws.send(Message::Text(
					json!({"op":4,"d":{"mode":MODE,"secret_key":vec![7;32],"dave_protocol_version":0}})
						.to_string()
						.into(),
				))
				.await
				.unwrap();

				ws.send(Message::Text(
					json!({"op":21,"d":{"protocol_version":0,"transition_id":10}})
						.to_string()
						.into(),
				))
				.await
				.unwrap();

				let mut saw_sink_wants = false;
				let mut sink_wants_tx = Some(sink_wants_tx);
				while let Some(msg) = ws.next().await {
					let message = msg.unwrap();
					if let Ok(text) = message.to_text()
						&& let Ok(event) = serde_json::from_str::<Value>(text)
						&& event["op"] == 15
					{
						assert_eq!(event["d"]["any"], 100);
						saw_sink_wants = true;
						if let Some(tx) = sink_wants_tx.take() {
							let _ = tx.send(());
						}
						break;
					}
				}
				assert!(saw_sink_wants);
				let _ = ready_rx.await;
			});

			let credentials = VoiceConnection {
				channel: model::Id(3),
				user: model::Id(1),
				peer: None,
				guild: Some(model::Id(100)),
				session: Secret::new("synthetic-session".into()).unwrap(),
				token: Secret::new("synthetic-token".into()).unwrap(),
				endpoint: "not-used-in-test".into(),
				request: 1,
			};
			let (_capture_tx, capture) = std::sync::mpsc::sync_channel(8);
			let (playback, _playback_rx) = std::sync::mpsc::sync_channel(8);
			let (control_tx, control_rx) = watch::channel(Controls::default());
			let (status_tx, mut status_rx) = tokio::sync::mpsc::channel(8);
			let remote_video: VideoSink = Arc::new(|_| {});
			let task = tokio::spawn(run_inner(
				credentials,
				capture,
				playback,
				control_rx,
				None,
				Some(remote_video),
				None,
				move |status| status_tx.try_send(status).map_err(|_| ()),
				Identity::generate(),
				format!("ws://{address}"),
				true,
			));

			let test = timeout(Duration::from_secs(5), async {
				while let Some(status) = status_rx.recv().await {
					if let Status::Ready { .. } = status {
						break;
					}
				}
				sink_wants_rx.await.unwrap();
				let _ = ready_tx.send(());
			});
			test.await.unwrap();
			drop(control_tx);
			let res = task.await.unwrap();
			assert!(res.is_ok());
			server.await.unwrap();
		});
	}
}

#[cfg(test)]
#[path = "test_stream.rs"]
mod test_stream;
