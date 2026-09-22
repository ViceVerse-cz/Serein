//! Desktop ownership for one explicitly authorized voice session.
use client_core::{
	Command, Event, State,
	voice::{self, Phase, Secret},
};
use discord_voice::{
	Controls, Status,
	audio::{Audio, Devices},
};
use eframe::egui;
use model::{Id, notification_preferences::Sound};
use std::{
	sync::{Arc, OnceLock, mpsc},
	time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::watch, task::JoinHandle};
use zeroize::Zeroizing;

fn permission_mutes_microphone(
	state: &State,
	channel: Id,
	push_to_talk: bool,
	ptt_active: bool,
) -> bool {
	!state.can_speak(channel)
		|| (state.permission(channel, model::permissions::USE_VAD) != Some(true)
			&& !(push_to_talk && ptt_active))
}

const DEVICE_OPEN_TIMEOUT: Duration = Duration::from_secs(20);

fn device_wait(
	deadline: &mut Option<Instant>,
	pending: bool,
	now: Instant,
) -> Result<Option<Duration>, &'static str> {
	if !pending {
		*deadline = None;
		return Ok(None);
	}
	deadline
		.get_or_insert(now + DEVICE_OPEN_TIMEOUT)
		.checked_duration_since(now)
		.filter(|remaining| !remaining.is_zero())
		.map(Some)
		.ok_or(
			"Audio device opening timed out; check device selection and system microphone permission",
		)
}

struct Pending {
	generation: u64,
	channel: Id,
	request: u64,
	ring: bool,
	user: Id,
	peer: Option<Id>,
	guild: Option<Id>,
	session: Option<Secret>,
	server: Option<(Secret, String)>,
	started: Instant,
}
enum Notice {
	TransportReady,
	CameraAvailable(bool),
	WaitingForPeer,
	Progress(Phase),
	MediaReady(String),
	DeviceReady,
	RemoteAudio,
}
struct Live {
	generation: u64,
	channel: Id,
	request: u64,
	user: Id,
	peer: Option<Id>,
	ring_pending: bool,
	cues: CallCues,
	session: Zeroizing<String>,
	identity: Arc<discord_voice::Identity>,
	audio: Audio,
	controls: watch::Sender<Controls>,
	events: mpsc::Receiver<Notice>,
	// Terminal errors must survive a full progress queue; retain the first safe reason.
	failure: Arc<OnceLock<&'static str>>,
	speakers: watch::Receiver<[u64; 64]>,
	task: JoinHandle<()>,
	devices: Devices,
	device_deadline: Option<Instant>,
	camera_frames: mpsc::SyncSender<discord_voice::camera_video::Frame>,
	camera_negotiated: bool,
	camera_clock: Instant,
	/// Latest decoded camera picture per remote user, replaced (never queued) by the decoder.
	remote_video: Arc<std::sync::Mutex<RemotePictures>>,
	/// Decoded audio of a watched stream, mixed into this call's playback.
	stream_audio: mpsc::SyncSender<discord_voice::Frame>,
}

#[derive(Default)]
struct CallCues {
	joined: bool,
	peers: Option<[u64; voice::MAX_PARTICIPANTS]>,
}
impl CallCues {
	fn poll(
		&mut self,
		ready: bool,
		gateway_connected: bool,
		owner: Id,
		participants: &[voice::Participant],
	) -> Option<Sound> {
		let joined = !self.joined && ready;
		self.joined |= ready;
		if !self.joined {
			return None;
		}
		let cue = joined.then_some(Sound::UserJoin);
		if !gateway_connected {
			self.peers = None;
			return cue;
		}
		let mut peers = [0; voice::MAX_PARTICIPANTS];
		for (slot, participant) in peers.iter_mut().zip(
			participants
				.iter()
				.filter(|participant| participant.user != owner),
		) {
			*slot = participant.user.0;
		}
		// ponytail: membership scans are capped at 64 IDs; no per-frame set allocation.
		let departed = self.peers.replace(peers).is_some_and(|previous| {
			previous
				.iter()
				.any(|user| *user != 0 && !peers.contains(user))
		});
		cue.or(departed.then_some(Sound::UserLeave))
	}
}
/// Remote cameras kept as textures at once; matches the transport's source limit.
const MAX_REMOTE_VIDEO: usize = 16;
type RemotePictures = Vec<(u64, Arc<egui::ColorImage>, bool)>;
type CameraPicture = Option<(Arc<egui::ColorImage>, bool)>;

#[allow(clippy::chunks_exact_to_as_chunks)] // Matches egui's faster profiled conversion loop.
fn store_remote_frame(
	pictures: &mut RemotePictures,
	frame: discord_voice::RemoteFrame<'_>,
) -> bool {
	if frame.rgba.len() != frame.width as usize * frame.height as usize * 4 {
		return false;
	}
	let entry = if let Some(index) = pictures.iter().position(|(user, _, _)| *user == frame.user) {
		&mut pictures[index]
	} else if pictures.len() < MAX_REMOTE_VIDEO {
		pictures.push((frame.user, Arc::new(egui::ColorImage::default()), false));
		pictures.last_mut().expect("remote frame inserted")
	} else {
		return false;
	};
	let size = [frame.width as usize, frame.height as usize];
	if let Some(image) = Arc::get_mut(&mut entry.1) {
		image.size = size;
		image.source_size = egui::vec2(frame.width as f32, frame.height as f32);
		image.pixels.clear();
		image.pixels.extend(frame.rgba.chunks_exact(4).map(|pixel| {
			egui::Color32::from_rgba_unmultiplied(pixel[0], pixel[1], pixel[2], pixel[3])
		}));
	} else {
		entry.1 = Arc::new(egui::ColorImage::from_rgba_unmultiplied(size, frame.rgba));
	}
	entry.2 = true;
	true
}

#[allow(clippy::chunks_exact_to_as_chunks)] // Matches egui's allocation fallback loop.
fn store_camera_frame(picture: &mut CameraPicture, rgb: &[u8]) -> bool {
	let size = [discord_voice::camera::WIDTH, discord_voice::camera::HEIGHT];
	if rgb.len() != size[0] * size[1] * 3 {
		return false;
	}
	let (image, dirty) =
		picture.get_or_insert_with(|| (Arc::new(egui::ColorImage::default()), false));
	if let Some(image) = Arc::get_mut(image) {
		image.size = size;
		image.source_size = egui::vec2(size[0] as f32, size[1] as f32);
		image.pixels.clear();
		image.pixels.extend(
			rgb.chunks_exact(3)
				.map(|pixel| egui::Color32::from_rgb(pixel[0], pixel[1], pixel[2])),
		);
	} else {
		*image = Arc::new(egui::ColorImage::from_rgb(size, rgb));
	}
	*dirty = true;
	true
}

struct MicPreview {
	audio: Audio,
	devices: Devices,
	failure: Arc<OnceLock<&'static str>>,
	started: Instant,
}

struct CameraTest {
	camera: discord_voice::camera::Camera,
	device: Option<String>,
	picture: Arc<std::sync::Mutex<CameraPicture>>,
}

#[derive(Default)]
pub struct Voice {
	camera_test: Option<CameraTest>,
	mic_preview: Option<MicPreview>,
	screen: crate::screen::Screen,
	watch: crate::watch::Watch,
	camera: Option<discord_voice::camera::Camera>,
	camera_device: Option<String>,
	camera_generation: u64,
	camera_preview: Option<std::sync::Arc<std::sync::Mutex<CameraPicture>>>,
	pending: Option<Pending>,
	live: Option<Live>,
	retiring: Option<mpsc::Receiver<()>>,
	device_scan: Option<mpsc::Receiver<Result<discord_voice::audio::DeviceList, &'static str>>>,
	camera_scan: Option<mpsc::Receiver<Result<discord_voice::camera::DeviceList, &'static str>>>,
}
impl Voice {
	pub fn stop(&mut self) {
		self.camera_test = None;
		self.mic_preview = None;
		self.screen.stop();
		self.watch.stop();
		self.pending = None;
		self.stop_camera();
		if let Some(live) = self.live.take() {
			live.audio.set_ready(false);
			live.task.abort();
			self.retiring = Some(live.audio.shutdown());
		}
	}
	pub fn stop_camera(&mut self) {
		if let Some(camera) = &self.camera {
			camera.stop();
		}
		self.camera_preview = None;
		if let Some(live) = &self.live {
			live.controls.send_if_modified(|controls| {
				let changed = controls.camera != 0;
				controls.camera = 0;
				changed
			});
		}
	}
	fn reap(&mut self) {
		if self.camera_preview.is_none()
			&& self.camera.as_ref().is_some_and(|camera| camera.stopped())
		{
			self.camera = None;
		}
		if self
			.retiring
			.as_ref()
			.is_some_and(|done| !matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)))
		{
			self.retiring = None;
		}
	}
	pub fn begin(&mut self, state: &State, ring: bool) -> Result<(), &'static str> {
		self.camera_test = None;
		self.mic_preview = None;
		self.reap();
		if self.retiring.is_some() {
			return Err("Previous audio devices are still closing; try again shortly");
		}
		if self.pending.is_some() || self.live.is_some() {
			return Err("A voice call is already active");
		}
		let call = state.voice.active.as_ref().ok_or("No call was requested")?;
		if !state.can_call(call.channel) {
			return Err("Select an existing DM or server voice channel");
		}
		let user = state.user.as_ref().ok_or("Sign in before calling")?.id;
		let channel = state
			.channels
			.iter()
			.find(|c| c.id == call.channel)
			.ok_or("The voice channel is unavailable")?;
		// Only one-to-one DMs pin a peer; group calls use the authenticated voice roster.
		let peer = (channel.guild.is_none() && channel.kind == 1)
			.then(|| channel.recipients.first().map(|u| u.id))
			.flatten();
		if channel.kind == 1 && peer.is_none() {
			return Err("The DM recipient is unavailable");
		}
		self.pending = Some(Pending {
			generation: state.generation,
			channel: call.channel,
			request: call.request,
			user,
			peer,
			guild: channel.guild,
			ring: ring && channel.guild.is_none(),
			session: None,
			server: None,
			started: Instant::now(),
		});
		Ok(())
	}
	/// Take negotiation secrets before reducing the UI event. Nothing is persisted.
	pub fn observe(&mut self, state: &State, event: &mut Event) -> Option<&'static str> {
		self.screen.observe(state, event);
		self.watch.observe(state, event);
		let Event::Voice(event) = event else {
			return None;
		};
		if let Some(live) = &self.live {
			match event {
				voice::Event::State {
					request: Some(request),
					user,
					channel: Some(channel),
					session: Some(session),
					..
				} if live.generation == state.generation
					&& *request == live.request
					&& *channel == live.channel
					&& state.user.as_ref().is_some_and(|owner| owner.id == *user) =>
				{
					if session.expose() != live.session.as_str() {
						return Some("Voice session changed; start a new call");
					}
				}
				voice::Event::Server {
					request, channel, ..
				} if live.generation == state.generation
					&& *request == live.request
					&& *channel == live.channel =>
				{
					return Some("Voice server changed; start a new encrypted call");
				}
				_ => {}
			}
		}
		let pending = self.pending.as_mut()?;
		if pending.generation != state.generation
			|| state.voice.active.as_ref().is_none_or(|c| {
				c.channel != pending.channel
					|| c.request != pending.request
					|| c.phase == Phase::Failed
			}) {
			return None;
		}
		match event {
			voice::Event::State {
				request: Some(request),
				channel: Some(channel),
				user,
				session,
				..
			} if *channel == pending.channel
				&& *request == pending.request
				&& *user == pending.user =>
			{
				if let Some(session) = session.take() {
					if pending
						.session
						.as_ref()
						.is_some_and(|old| old.expose() != session.expose())
					{
						return Some("Voice session changed during connection; try a new call");
					}
					pending.session = Some(session);
				}
			}
			voice::Event::Server {
				request,
				channel,
				token,
				endpoint,
			} if *request == pending.request && *channel == pending.channel => {
				let Some(endpoint) = endpoint.take() else {
					pending.server = None;
					let _ = token.take();
					return None;
				};
				let Some(token) = token.take() else {
					return Some("Discord omitted the voice connection token");
				};
				pending.server = Some((token, endpoint));
			}
			_ => {}
		}
		None
	}
	pub fn fail(&mut self, state: &mut State, message: &'static str) -> Option<Command> {
		self.stop();
		let call = state.voice.active.as_ref()?;
		let (channel, request) = (call.channel, call.request);
		state.apply_voice(voice::Event::Failed {
			channel,
			request,
			message,
		});
		Some(Command::Voice(voice::Command::Leave { channel, request }))
	}
	pub fn poll(
		&mut self,
		runtime: &Runtime,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		ctx: &egui::Context,
	) -> Option<Command> {
		self.reap();
		ui.voice_switch_ready =
			self.pending.is_none() && self.live.is_none() && self.retiring.is_none();
		self.poll_mic_preview(state, ui, ctx);
		self.poll_camera_test(state, ui, ctx);
		ui.voice_speaking.clear();
		ui.voice_microphone_unavailable = false;
		self.poll_camera_devices(state.demo, ui, ctx);
		if ui.voice_refresh_devices {
			ui.voice_refresh_devices = false;
			if !state.demo && self.device_scan.is_none() {
				let (send, receive) = mpsc::sync_channel(1);
				let wake = ctx.clone();
				match std::thread::Builder::new()
					.name("audio-devices".into())
					.spawn(move || {
						let _ = send.send(discord_voice::audio::devices());
						wake.request_repaint();
					}) {
					Ok(_) => {
						self.device_scan = Some(receive);
						ui.voice_device_status = "Looking for audio devices…";
					}
					Err(_) => ui.voice_device_status = "Could not start audio device discovery",
				}
			}
		}
		if let Some(scan) = &self.device_scan {
			match scan.try_recv() {
				Ok(Ok(devices)) => {
					ui.voice_inputs = devices.inputs;
					ui.voice_outputs = devices.outputs;
					ui.voice_device_status =
						"Audio devices loaded · headphones avoid microphone echo";
					self.device_scan = None;
				}
				Ok(Err(error)) => {
					ui.voice_device_status = error;
					self.device_scan = None;
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					ui.voice_device_status = "Audio device discovery stopped";
					self.device_scan = None;
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
		let expected = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.phase != Phase::Failed)
			.map(|call| (state.generation, call.channel, call.request));
		if expected.is_none() {
			ui.voice_camera_available = false;
			ui.voice_camera_preview = None;
			ui.voice_camera_status = "";
			ui.voice_privacy_code = None;
			ui.voice_remote_video.clear();
		}
		let current = self
			.live
			.as_ref()
			.map(|c| (c.generation, c.channel, c.request))
			.or_else(|| {
				self.pending
					.as_ref()
					.map(|c| (c.generation, c.channel, c.request))
			});
		if current.is_some() && current != expected {
			self.stop();
			// Permission/removal failures must leave the service too; never target a new account.
			return current
				.filter(|(generation, _, _)| *generation == state.generation)
				.map(|(_, channel, request)| {
					Command::Voice(voice::Command::Leave { channel, request })
				});
		}
		if let Some(pending) = &self.pending {
			if pending.started.elapsed() >= Duration::from_secs(30) {
				return self.fail(
                    state,
                    "Discord did not provide voice connection details; check Connect permission and channel capacity",
                );
			}
			ctx.request_repaint_after(
				Duration::from_secs(30).saturating_sub(pending.started.elapsed()),
			);
		}
		if self
			.pending
			.as_ref()
			.is_some_and(|p| p.session.is_some() && p.server.is_some())
		{
			let pending = self.pending.take().expect("pending negotiation");
			let listen_only = permission_mutes_microphone(
				state,
				pending.channel,
				ui.voice_push_to_talk,
				ui.voice_ptt_active,
			);
			let input_enabled = state.can_speak(pending.channel);
			if let Err(error) =
				self.start_media(runtime, pending, ui, ctx, listen_only, input_enabled)
			{
				return self.fail(state, error);
			}
		}
		let mut failure = None;
		let mut command = None;
		if let Some(live) = &mut self.live {
			let call = state.voice.active.as_ref().expect("matching active call");
			let deafened = call.deafened || call.server_deafened;
			let muted = call.muted
				|| permission_mutes_microphone(
					state,
					call.channel,
					ui.voice_push_to_talk,
					ui.voice_ptt_active,
				) || call.server_muted
				|| deafened || (ui.voice_push_to_talk && !ui.voice_ptt_active);
			live.audio.set_controls(muted, deafened);
			live.audio.set_processing(ui.voice_processing.effective());
			live.audio.set_input_enabled(state.can_speak(call.channel));
			live.audio
				.set_gain(ui.voice_gain.input_percent, ui.voice_gain.output_percent);
			let user_volumes = ui.voice_user_volumes();
			let stream_volume = ui.voice_stream_volume();
			let activity_threshold_db = ui
				.voice_processing
				.effective()
				.sensitivity_db
				.unwrap_or(-70);
			live.controls.send_if_modified(|control| {
				if control.muted == muted
					&& control.deafened == deafened
					&& control.user_volumes == user_volumes
					&& control.stream_volume == stream_volume
					&& control.activity_threshold_db == activity_threshold_db
				{
					false
				} else {
					control.muted = muted;
					control.deafened = deafened;
					control.user_volumes = user_volumes;
					control.stream_volume = stream_volume;
					control.activity_threshold_db = activity_threshold_db;
					true
				}
			});
			if ui.voice_input != live.devices.input || ui.voice_output != live.devices.output {
				let devices = Devices {
					input: ui.voice_input.clone(),
					output: ui.voice_output.clone(),
				};
				live.audio.set_devices(devices.clone());
				live.devices = devices;
				live.device_deadline = None;
			}
			for _ in 0..8 {
				let Ok(event) = live.events.try_recv() else {
					break;
				};
				match event {
					Notice::CameraAvailable(available) => live.camera_negotiated = available,
					Notice::TransportReady => {
						if live.ring_pending {
							live.ring_pending = false;
							command = Some(Command::Voice(voice::Command::Ring {
								channel: live.channel,
								request: live.request,
							}));
						}
					}
					Notice::WaitingForPeer => {
						ui.voice_privacy_code = None;
						live.audio.set_ready(true);
						live.device_deadline = None;
						state.apply_voice(voice::Event::Progress {
							channel: live.channel,
							request: live.request,
							phase: Phase::Waiting,
						});
					}
					Notice::Progress(phase) => {
						ui.voice_privacy_code = None;
						live.audio.set_ready(false);
						live.device_deadline = None;
						state.apply_voice(voice::Event::Progress {
							channel: live.channel,
							request: live.request,
							phase,
						});
					}
					Notice::MediaReady(code) => {
						ui.voice_privacy_code = Some(code);
						live.audio.set_ready(true);
					}
					// Notices wake the UI; only the current device configuration can be ready.
					Notice::DeviceReady | Notice::RemoteAudio => {}
				}
			}
			failure = live.failure.get().copied();
			let devices_ready = live.audio.is_ready();
			ui.voice_microphone_unavailable = live.audio.microphone_unavailable();
			if ui.voice_settings_open() {
				ui.voice_preview_level = Some(live.audio.preview_level_db());
				ctx.request_repaint_after(Duration::from_millis(50));
			}
			if failure.is_none() {
				let pending = live
					.audio
					.gate
					.ready
					.load(std::sync::atomic::Ordering::Acquire)
					&& !devices_ready;
				match device_wait(&mut live.device_deadline, pending, Instant::now()) {
					Ok(Some(remaining)) => {
						state.apply_voice(voice::Event::Progress {
							channel: live.channel,
							request: live.request,
							phase: if ui.voice_privacy_code.is_some() {
								Phase::OpeningAudio
							} else {
								Phase::Waiting
							},
						});
						ctx.request_repaint_after(remaining);
					}
					Ok(None) => {}
					Err(error) => failure = Some(error),
				}
			}
			if failure.is_none() && devices_ready && ui.voice_privacy_code.is_some() {
				state.apply_voice(voice::Event::Progress {
					channel: live.channel,
					request: live.request,
					phase: Phase::Connected,
				});
			}
			if live.audio.is_stopped() && failure.is_none() {
				failure =
					Some("Audio devices stopped; check microphone permission and device selection");
			}
			if live.task.is_finished() && failure.is_none() {
				failure = Some("Voice connection ended; start a new call explicitly");
			}
			// A worker can finish between draining notices and checking its lifecycle.
			failure = live.failure.get().copied().or(failure);
			if failure.is_none()
				&& !state.demo
				&& state.auth == client_core::auth::AuthState::Authenticated
				&& let Some(call) = &state.voice.active
			{
				// Waiting alone still joins voice, but its audio devices may not be open yet.
				let ready =
					devices_ready && matches!(call.phase, Phase::Connected | Phase::Waiting);
				if let Some(cue) = live.cues.poll(
					ready,
					state.gateway_connected,
					live.user,
					&call.participants,
				) && ui.notification_options.allows(cue)
				{
					ui.notification_preview = Some(cue);
					ctx.request_repaint();
				}
			}
			let pictures = live
				.remote_video
				.try_lock()
				.map(|mut slot| {
					slot.iter_mut()
						.filter_map(|(user, image, dirty)| {
							std::mem::take(dirty).then(|| (*user, image.clone()))
						})
						.collect::<Vec<_>>()
				})
				.unwrap_or_default();
			for (user, image) in pictures {
				if let Some((_, texture)) = ui
					.voice_remote_video
					.iter_mut()
					.find(|(id, _)| id.0 == user)
				{
					texture.set(image, egui::TextureOptions::LINEAR);
				} else if ui.voice_remote_video.len() < MAX_REMOTE_VIDEO {
					ui.voice_remote_video.push((
						Id(user),
						ctx.load_texture(
							format!("remote-camera-{user}"),
							image,
							egui::TextureOptions::LINEAR,
						),
					));
				}
			}
			// Cameras announced off, or participants who left, release their textures.
			let visible: Vec<Id> = state
				.voice
				.active
				.as_ref()
				.map(|call| {
					call.participants
						.iter()
						.filter(|participant| participant.video)
						.map(|participant| participant.user)
						.collect()
				})
				.unwrap_or_default();
			ui.voice_remote_video.retain(|(id, _)| visible.contains(id));
			if let Ok(mut pictures) = live.remote_video.try_lock() {
				pictures.retain(|(id, _, _)| visible.contains(&Id(*id)));
			}
		}
		if failure.is_none()
			&& let Some(live) = &self.live
			&& let Some(call) = &state.voice.active
			&& matches!(call.phase, Phase::Connected | Phase::Waiting)
			&& !call.deafened
			&& !call.server_deafened
		{
			let controls = *live.controls.borrow();
			ui.voice_speaking.extend(
				live.speakers
					.borrow()
					.iter()
					.copied()
					.filter(|user| {
						*user != 0
							&& !(controls.muted
								&& state.user.as_ref().is_some_and(|own| own.id.0 == *user))
					})
					.map(Id),
			);
		}
		let command = if let Some(error) = failure {
			self.fail(state, error)
		} else {
			self.poll_camera(state, ui, ctx).or(command)
		};
		if command.is_some() {
			return command;
		}
		let call = self.live.as_ref().map(|live| crate::screen::Call {
			generation: live.generation,
			channel: live.channel,
			request: live.request,
			user: live.user,
			peer: live.peer,
			session: live.session.as_str(),
			identity: live.identity.clone(),
		});
		let watched = self.live.as_ref().map(|live| crate::screen::Call {
			generation: live.generation,
			channel: live.channel,
			request: live.request,
			user: live.user,
			peer: live.peer,
			session: live.session.as_str(),
			identity: live.identity.clone(),
		});
		let stream_audio = self.live.as_ref().map(|live| live.stream_audio.clone());
		if let Some(command) = self.screen.poll(runtime, state, ui, ctx, call) {
			return Some(command);
		}
		self.watch
			.poll(runtime, state, ui, ctx, watched, stream_audio)
	}
	fn poll_mic_preview(&mut self, state: &State, ui: &mut ui::MessagingUi, ctx: &egui::Context) {
		if state.demo
			|| !ui.voice_available
			|| !ui.voice_settings_open()
			|| state.voice.active.is_some()
			|| self.pending.is_some()
			|| self.live.is_some()
		{
			ui.voice_preview_requested = false;
			ui.voice_preview_status = "";
		}
		if !ui.voice_preview_requested {
			if self.mic_preview.is_some() {
				ui.voice_preview_status = "";
			}
			self.mic_preview = None;
			ui.voice_preview_level = None;
			return;
		}
		if self.mic_preview.is_none() {
			let failure = Arc::new(OnceLock::new());
			let worker_failure = failure.clone();
			let wake = ctx.clone();
			match Audio::preview(
				Devices {
					input: ui.voice_input.clone(),
					output: ui.voice_output.clone(),
				},
				move |result| {
					if let Err(error) = result {
						let _ = worker_failure.set(error);
					}
					wake.request_repaint();
				},
			) {
				Ok(audio) => {
					self.mic_preview = Some(MicPreview {
						audio,
						devices: Devices {
							input: ui.voice_input.clone(),
							output: ui.voice_output.clone(),
						},
						failure,
						started: Instant::now(),
					})
				}
				Err(error) => {
					ui.voice_preview_status = error;
					ui.voice_preview_requested = false;
					return;
				}
			}
		}
		let preview = self.mic_preview.as_mut().expect("preview started");
		let devices = Devices {
			input: ui.voice_input.clone(),
			output: ui.voice_output.clone(),
		};
		if devices != preview.devices {
			preview.audio.set_devices(devices.clone());
			preview.devices = devices;
			preview.started = Instant::now();
		}
		preview
			.audio
			.set_gain(ui.voice_gain.input_percent, ui.voice_gain.output_percent);
		preview
			.audio
			.set_processing(ui.voice_processing.effective());
		preview.audio.set_ready(true);
		let error = preview.failure.get().copied().or_else(|| {
			if preview.audio.is_stopped() {
				Some("Microphone test stopped; try again.")
			} else if !preview.audio.is_ready() && preview.started.elapsed() >= DEVICE_OPEN_TIMEOUT
			{
				Some(
					"Audio devices did not open; check device selection and microphone permission.",
				)
			} else {
				None
			}
		});
		if let Some(error) = error {
			ui.voice_preview_requested = false;
			ui.voice_preview_level = None;
			ui.voice_preview_status = error;
			self.mic_preview = None;
			return;
		}
		ui.voice_preview_level = Some(preview.audio.preview_level_db());
		ui.voice_preview_status = if preview.audio.microphone_unavailable() {
			"Microphone unavailable; check permission or choose another input. Retrying…"
		} else if preview.audio.is_ready() {
			"Playing your microphone through the selected speakers."
		} else {
			"Opening microphone and speakers…"
		};
		ctx.request_repaint_after(Duration::from_millis(50));
	}

	fn poll_camera_test(&mut self, state: &State, ui: &mut ui::MessagingUi, ctx: &egui::Context) {
		ui.camera_test_available = !state.demo
			&& ui.voice_available
			&& discord_voice::camera::SUPPORTED
			&& state.voice.active.is_none()
			&& self.pending.is_none()
			&& self.live.is_none();
		if !ui.camera_test_available || !ui.voice_settings_open() {
			ui.camera_test_requested = false;
			ui.camera_test_status = "";
		}
		if let Some(preview) = &self.camera_test {
			if preview.device != ui.voice_camera_device {
				ui.camera_test_requested = false;
				ui.camera_test_status = "Camera changed. Click Preview camera to use it.";
			} else if let Some(error) = preview.camera.error() {
				ui.camera_test_requested = false;
				ui.camera_test_status = error;
			}
		}
		if !ui.camera_test_requested {
			self.camera_test = None;
			ui.camera_test_texture = None;
			return;
		}
		if self.camera_test.is_none() {
			let picture = Arc::new(std::sync::Mutex::new(None));
			let frames = picture.clone();
			let wake = ctx.clone();
			// No transport sender is captured: these frames can only reach the settings texture.
			let on_frame = Arc::new(move |frame: discord_voice::camera::Frame| {
				if let Ok(mut slot) = frames.try_lock()
					&& store_camera_frame(&mut slot, &frame.rgb)
				{
					wake.request_repaint();
				}
			});
			let wake = ctx.clone();
			match discord_voice::camera::Camera::start(
				ui.voice_camera_device.clone(),
				on_frame,
				Arc::new(move || wake.request_repaint()),
			) {
				Ok(camera) => {
					self.camera_test = Some(CameraTest {
						camera,
						device: ui.voice_camera_device.clone(),
						picture,
					});
					ui.camera_test_status = "Opening camera…";
				}
				Err(error) => {
					ui.camera_test_requested = false;
					ui.camera_test_status = error;
					return;
				}
			}
		}
		let image = self.camera_test.as_ref().and_then(|preview| {
			let mut slot = preview.picture.try_lock().ok()?;
			let (image, dirty) = slot.as_mut()?;
			std::mem::take(dirty).then(|| image.clone())
		});
		if let Some(image) = image {
			if let Some(texture) = &mut ui.camera_test_texture {
				texture.set(image, egui::TextureOptions::LINEAR);
			} else {
				ui.camera_test_texture =
					Some(ctx.load_texture("settings-camera", image, egui::TextureOptions::LINEAR));
			}
			ui.camera_test_status = "Local camera preview · not shared";
		}
	}

	fn poll_camera_devices(&mut self, demo: bool, ui: &mut ui::MessagingUi, ctx: &egui::Context) {
		if demo {
			ui.voice_refresh_cameras = false;
			ui.voice_camera_devices_loading = false;
			self.camera_scan = None;
			return;
		}
		if std::mem::take(&mut ui.voice_refresh_cameras) && self.camera_scan.is_none() {
			let (send, receive) = mpsc::sync_channel(1);
			let wake = ctx.clone();
			match std::thread::Builder::new()
				.name("camera-devices".into())
				.spawn(move || {
					let result = std::panic::catch_unwind(discord_voice::camera::devices)
						.unwrap_or(Err("Camera device discovery failed"));
					let _ = send.send(result);
					wake.request_repaint();
				}) {
				Ok(_) => {
					self.camera_scan = Some(receive);
					ui.voice_camera_devices_loading = true;
					ui.voice_camera_device_status = "Looking for cameras…";
				}
				Err(_) => ui.voice_camera_device_status = "Could not start camera device discovery",
			}
		}
		if let Some(scan) = &self.camera_scan {
			match scan.try_recv() {
				Ok(result) => {
					ui.voice_camera_device_status = match result {
						Ok(devices) => {
							ui.voice_cameras = devices;
							if ui.voice_cameras.is_empty() {
								"No cameras found. Check the camera connection or virtual camera installation, then refresh."
							} else {
								"Cameras loaded"
							}
						}
						Err(error) => error,
					};
					self.camera_scan = None;
					ui.voice_camera_devices_loading = false;
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					ui.voice_camera_device_status = "Camera device discovery stopped";
					ui.voice_camera_devices_loading = false;
					self.camera_scan = None;
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
	}
	fn poll_camera(
		&mut self,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		ctx: &egui::Context,
	) -> Option<Command> {
		let supported = discord_voice::camera::SUPPORTED;
		ui.voice_camera_available = supported
			&& self
				.live
				.as_ref()
				.is_some_and(|live| live.camera_negotiated);
		let requested = state.voice.active.as_ref().is_some_and(|call| call.camera);
		if requested && self.camera.is_some() && self.camera_device != ui.voice_camera_device {
			self.stop_camera();
			ui.voice_camera_preview = None;
			ui.voice_camera_status =
				"Camera device changed. Turn on the camera to use the selected device.";
			return state.set_call_camera(false);
		}
		let preview_allowed = camera_preview_allowed(state);
		let error = self.camera.as_ref().and_then(|camera| camera.error());
		if !requested || !preview_allowed || !ui.voice_camera_available || error.is_some() {
			self.stop_camera();
			ui.voice_camera_preview = None;
			if requested {
				ui.voice_camera_status =
					error.unwrap_or("Camera stopped; reconnect securely before turning it on");
				return state.set_call_camera(false);
			}
			return None;
		}
		if self.camera_preview.is_none() {
			if self.camera.is_some() {
				ui.voice_camera_status = "Camera is still closing; try again shortly";
				return state.set_call_camera(false);
			}
			let live = self.live.as_ref()?;
			self.camera_generation = self.camera_generation.checked_add(1).unwrap_or(1);
			let generation = self.camera_generation;
			let send = live.camera_frames.clone();
			let receive = std::sync::Arc::new(std::sync::Mutex::new(None));
			let preview = receive.clone();
			let start = live.camera_clock;
			let wake = ctx.clone();
			let on_frame = std::sync::Arc::new(move |frame: discord_voice::camera::Frame| {
				let data = frame.h264;
				if data.len() > discord_voice::camera_video::MAX_FRAME_BYTES {
					return;
				}
				let _ = send.try_send(discord_voice::camera_video::Frame {
					generation,
					timestamp: (start.elapsed().as_micros() * 90 / 1000) as u32,
					data,
				});
				if let Ok(mut slot) = preview.try_lock()
					&& store_camera_frame(&mut slot, &frame.rgb)
				{
					wake.request_repaint();
				}
			});
			let wake = ctx.clone();
			match discord_voice::camera::Camera::start(
				ui.voice_camera_device.clone(),
				on_frame,
				std::sync::Arc::new(move || wake.request_repaint()),
			) {
				Ok(camera) => {
					self.camera_device = ui.voice_camera_device.clone();
					self.camera = Some(camera);
					self.camera_preview = Some(receive);
					ui.voice_camera_status = "Opening camera…";
					live.controls
						.send_modify(|controls| controls.camera = generation);
				}
				Err(error) => {
					ui.voice_camera_status = error;
					return state.set_call_camera(false);
				}
			}
		}
		let image = self.camera_preview.as_ref().and_then(|preview| {
			let mut preview = preview.try_lock().ok()?;
			let (image, dirty) = preview.as_mut()?;
			std::mem::take(dirty).then(|| image.clone())
		});
		if let Some(image) = image {
			if let Some(texture) = &mut ui.voice_camera_preview {
				texture.set(image, egui::TextureOptions::LINEAR);
			} else {
				ui.voice_camera_preview =
					Some(ctx.load_texture("local-camera", image, egui::TextureOptions::LINEAR));
				let cue = model::notification_preferences::Sound::CameraOn;
				if ui.notification_options.allows(cue) {
					ui.notification_preview = Some(cue);
					ctx.request_repaint();
				}
			}
			ui.voice_camera_status = "Camera on · local preview";
		}
		None
	}
	fn start_media(
		&mut self,
		runtime: &Runtime,
		pending: Pending,
		ui: &ui::MessagingUi,
		ctx: &egui::Context,
		listen_only: bool,
		input_enabled: bool,
	) -> Result<(), &'static str> {
		let (capture_send, capture) = mpsc::sync_channel(8);
		let (camera_frames, camera_receive) = mpsc::sync_channel(1);
		let (stream_audio, stream_audio_receive) = mpsc::sync_channel(8);
		let (playback, playback_receive) = mpsc::sync_channel(8);
		let (send, events) = mpsc::sync_channel(8);
		let failure = Arc::new(OnceLock::new());
		let audio_failure = failure.clone();
		let (speaking, speakers) = watch::channel([0; 64]);
		let audio_send = send.clone();
		let wake = ctx.clone();
		let devices = Devices {
			input: ui.voice_input.clone(),
			output: ui.voice_output.clone(),
		};
		let audio = Audio::start(
			devices.clone(),
			capture_send,
			playback_receive,
			move |result| {
				match result {
					Ok(()) => {
						let _ = audio_send.try_send(Notice::DeviceReady);
					}
					Err(error) => {
						let _ = audio_failure.set(error);
					}
				}
				wake.request_repaint();
			},
		)?;
		let (controls, control_receive) = watch::channel(Controls {
			activity_threshold_db: ui
				.voice_processing
				.effective()
				.sensitivity_db
				.unwrap_or(-70),
			muted: listen_only || ui.voice_push_to_talk,
			camera: 0,
			deafened: false,
			user_volumes: ui.voice_user_volumes(),
			stream_volume: ui.voice_stream_volume(),
		});
		let remote_video: Arc<std::sync::Mutex<RemotePictures>> =
			Arc::new(std::sync::Mutex::new(Vec::new()));
		let pictures = remote_video.clone();
		let picture_wake = ctx.clone();
		let sink: discord_voice::VideoSink = Arc::new(move |frame: discord_voice::RemoteFrame| {
			if pictures
				.lock()
				.is_ok_and(|mut pictures| store_remote_frame(&mut pictures, frame))
			{
				picture_wake.request_repaint();
			}
		});
		audio.set_controls(listen_only || ui.voice_push_to_talk, false);
		audio.set_input_enabled(input_enabled);
		audio.set_processing(ui.voice_processing.effective());
		audio.set_gain(ui.voice_gain.input_percent, ui.voice_gain.output_percent);
		let session = pending.session.ok_or("Missing voice session")?;
		let session_copy = Zeroizing::new(session.expose().to_owned());
		let (token, endpoint) = pending.server.ok_or("Missing voice server")?;
		let credentials = voice::VoiceConnection {
			channel: pending.channel,
			request: pending.request,
			user: pending.user,
			peer: pending.peer,
			guild: pending.guild,
			session,
			token,
			endpoint,
		};
		let identity = discord_voice::Identity::generate();
		let media_identity = identity.clone();
		let transport_failure = failure.clone();
		let wake = ctx.clone();
		let task = runtime.spawn(async move {
			let status = send.clone();
			let status_wake = wake.clone();
			let result = discord_voice::run_with_identity(
				credentials,
				capture,
				playback,
				control_receive,
				if discord_voice::camera::SUPPORTED {
					Some(camera_receive)
				} else {
					None
				},
				Some(sink),
				Some(stream_audio_receive),
				move |event| {
					let notice = match event {
						Status::TransportReady => Notice::TransportReady,
						Status::CameraAvailable(available) => Notice::CameraAvailable(available),
						Status::Connecting => Notice::Progress(Phase::ConnectingTransport),
						Status::Discovering => Notice::Progress(Phase::Discovering),
						Status::Securing => Notice::Progress(Phase::Securing),
						Status::WaitingForPeer => Notice::WaitingForPeer,
						Status::Ready { privacy_code } => {
							if privacy_code.len() > 256 {
								return Err(());
							}
							Notice::MediaReady(privacy_code)
						}
						Status::RemoteAudio => Notice::RemoteAudio,
						Status::Speaking(users) => {
							speaking.send_replace(*users);
							status_wake.request_repaint();
							return Ok(());
						}
					};
					status.try_send(notice).map_err(|_| ())?;
					status_wake.request_repaint();
					Ok(())
				},
				media_identity,
			)
			.await;
			if let Err(error) = result {
				let _ = transport_failure.set(error);
			}
			wake.request_repaint();
		});
		self.live = Some(Live {
			generation: pending.generation,
			channel: pending.channel,
			request: pending.request,
			user: pending.user,
			peer: pending.peer,
			session: session_copy,
			identity,
			ring_pending: pending.ring,
			cues: CallCues::default(),
			audio,
			controls,
			events,
			failure,
			speakers,
			task,
			devices,
			device_deadline: None,
			camera_frames,
			camera_negotiated: false,
			camera_clock: Instant::now(),
			remote_video,
			stream_audio,
		});
		Ok(())
	}
}
impl Drop for Voice {
	fn drop(&mut self) {
		self.stop();
	}
}

// A peer joining/leaving rekeys media without revoking the local camera gesture.
fn camera_preview_allowed(state: &State) -> bool {
	state.voice.active.as_ref().is_some_and(|call| {
		(matches!(call.phase, Phase::Connected | Phase::Waiting)
			|| (call.connected_at.is_some()
				&& matches!(call.phase, Phase::Securing | Phase::OpeningAudio)))
			&& state.can_camera(call.channel)
	})
}

/// Device-free check of preview rendering and the guards that prevent automatic capture.
#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_mic_preview_check() {
	ui::MessagingUi::debug_call_switch_check(test_support::existing_call_demo_state());
	discord_voice::audio::debug_processing_check();
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	let mut view = ui::MessagingUi::default();
	view.voice_available = true;
	view.preview_settings("voice");
	assert!(view.voice_settings_open());
	for (width, profile) in [
		(480.0, model::voice_settings::InputProfile::VoiceIsolation),
		(1120.0, model::voice_settings::InputProfile::Studio),
		(1120.0, model::voice_settings::InputProfile::Custom),
	] {
		view.voice_processing.profile = profile;
		for _ in 0..3 {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 760.0),
					)),
					..Default::default()
				},
				|ui| {
					let _ = view.show(ui, &mut state);
				},
			)
			.drop_without_applying_deltas();
		}
	}
	assert!(!view.voice_preview_requested);
	assert!(!view.camera_test_requested);
	let mut camera_host = Voice::default();
	view.camera_test_requested = true;
	camera_host.poll_camera_test(&state, &mut view, &ctx);
	assert!(!view.camera_test_requested && camera_host.camera_test.is_none());
	state.demo = false;
	view.preview_settings("appearance");
	view.camera_test_requested = true;
	camera_host.poll_camera_test(&state, &mut view, &ctx);
	assert!(!view.camera_test_requested && camera_host.camera_test.is_none());
	state.demo = true;
	view.preview_settings("voice");
	let legacy: local_store::AppPreferences =
		serde_json::from_str(r#"{"voice_noise_suppression":true}"#).unwrap();
	let mut preferences = crate::app_settings::Settings {
		current: legacy,
		..Default::default()
	};
	preferences.apply(&mut view);
	assert_eq!(
		view.voice_processing.effective().suppression,
		model::voice_settings::NoiseSuppression::RnNoise
	);
	view.voice_processing.custom.sensitivity_db = Some(-63);
	preferences.observe(&view);
	let saved = serde_json::to_string(&preferences.current).unwrap();
	let restored: local_store::AppPreferences = serde_json::from_str(&saved).unwrap();
	assert_eq!(restored.voice_processing, Some(view.voice_processing));
	assert!(restored.is_valid());
	let mut voice = Voice::default();
	view.voice_preview_requested = true;
	voice.poll_mic_preview(&state, &mut view, &ctx);
	assert!(!view.voice_preview_requested && voice.mic_preview.is_none());
	state.demo = false;
	view.preview_settings("appearance");
	view.voice_preview_requested = true;
	voice.poll_mic_preview(&state, &mut view, &ctx);
	assert!(!view.voice_preview_requested && voice.mic_preview.is_none());
	println!(
		"Mic/camera preview debug check passed: settings render, opening settings never starts capture, demo and closed-page guards stop requests. No audio devices opened."
	);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn call_cues_join_once_and_track_remote_departures_without_reconnect_noise() {
		let participant = |id| voice::Participant {
			user: Id(id),
			muted: false,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			video: false,
			streaming: false,
		};
		let owner = participant(1);
		let peer = participant(2);
		let mut cues = CallCues::default();
		assert_eq!(cues.poll(false, true, owner.user, &[owner, peer]), None);
		assert_eq!(
			cues.poll(true, true, owner.user, &[owner, peer]),
			Some(Sound::UserJoin)
		);
		let mut muted_peer = peer;
		muted_peer.muted = true;
		assert_eq!(
			cues.poll(true, true, owner.user, &[muted_peer, owner]),
			None
		);
		// Device reopening and rekeying do not announce this same call again.
		assert_eq!(cues.poll(false, true, owner.user, &[owner, peer]), None);
		assert_eq!(cues.poll(true, true, owner.user, &[owner, peer]), None);
		// Compare identities rather than counts; departures can themselves trigger rekeying.
		let replacement = participant(3);
		assert_eq!(
			cues.poll(false, true, owner.user, &[owner, replacement]),
			Some(Sound::UserLeave)
		);
		assert_eq!(
			cues.poll(true, true, owner.user, &[owner, replacement]),
			None
		);
		assert_eq!(cues.poll(false, false, owner.user, &[]), None);
		assert_eq!(cues.poll(true, true, owner.user, &[owner]), None);
		// Remote joins only update the baseline; the requested join cue is for this device.
		assert_eq!(cues.poll(true, true, owner.user, &[owner, peer]), None);
		assert_eq!(
			cues.poll(true, true, owner.user, &[owner]),
			Some(Sound::UserLeave)
		);
		assert_eq!(cues.poll(true, true, owner.user, &[owner]), None);
		assert_eq!(
			CallCues::default().poll(true, true, owner.user, &[owner]),
			Some(Sound::UserJoin),
			"a new explicitly started call has its own join cue"
		);
	}

	#[test]
	fn optimization_remote_video_reuses_the_latest_frame_buffer() {
		let mut pictures = Vec::new();
		let rgba = [1, 2, 3, 255, 4, 5, 6, 255];
		assert!(store_remote_frame(
			&mut pictures,
			discord_voice::RemoteFrame {
				user: 7,
				width: 2,
				height: 1,
				rgba: &rgba,
			},
		));
		let pixels = pictures[0].1.pixels.as_ptr();
		pictures[0].2 = false;
		assert!(store_remote_frame(
			&mut pictures,
			discord_voice::RemoteFrame {
				user: 7,
				width: 2,
				height: 1,
				rgba: &rgba,
			},
		));
		assert_eq!(pictures[0].1.pixels.as_ptr(), pixels);
		assert!(pictures[0].2);
		let upload = pictures[0].1.clone();
		assert!(store_remote_frame(
			&mut pictures,
			discord_voice::RemoteFrame {
				user: 7,
				width: 1,
				height: 1,
				rgba: &rgba[..4],
			},
		));
		assert!(!Arc::ptr_eq(&pictures[0].1, &upload));
		assert_eq!(upload.size, [2, 1]);
	}

	#[test]
	fn optimization_local_camera_reuses_the_latest_frame_buffer() {
		let mut picture = None;
		let rgb = vec![127; discord_voice::camera::WIDTH * discord_voice::camera::HEIGHT * 3];
		assert!(store_camera_frame(&mut picture, &rgb));
		let pixels = picture.as_ref().unwrap().0.pixels.as_ptr();
		picture.as_mut().unwrap().1 = false;
		assert!(store_camera_frame(&mut picture, &rgb));
		assert_eq!(picture.as_ref().unwrap().0.pixels.as_ptr(), pixels);
		assert!(picture.unwrap().1);
	}

	#[test]
	#[allow(clippy::field_reassign_with_default)] // MessagingUi has private fields in another crate.
	fn camera_discovery_updates_selection_without_starting_capture_and_demo_does_not_scan() {
		let mut voice = Voice::default();
		let mut ui = ui::MessagingUi::default();
		ui.voice_refresh_cameras = true;
		let ctx = egui::Context::default();
		voice.poll_camera_devices(true, &mut ui, &ctx);
		assert!(voice.camera_scan.is_none() && !ui.voice_refresh_cameras);
		let (send, receive) = mpsc::sync_channel(1);
		voice.camera_scan = Some(receive);
		ui.voice_camera_devices_loading = true;
		ui.voice_camera_device = Some("dshow:second".into());
		send.send(Ok(vec![
			("dshow:first".into(), "First camera".into()),
			("dshow:second".into(), "Second camera".into()),
		]))
		.unwrap();
		voice.poll_camera_devices(false, &mut ui, &ctx);
		assert_eq!(ui.voice_cameras.len(), 2);
		assert_eq!(ui.voice_camera_device.as_deref(), Some("dshow:second"));
		assert!(!ui.voice_camera_devices_loading);
		assert!(voice.camera.is_none() && voice.camera_scan.is_none());
		let (send, receive) = mpsc::sync_channel(1);
		voice.camera_scan = Some(receive);
		send.send(Ok(vec![])).unwrap();
		voice.poll_camera_devices(true, &mut ui, &ctx);
		assert_eq!(
			ui.voice_cameras.len(),
			2,
			"Demo must not consume a real device scan"
		);
		assert!(voice.camera_scan.is_none());
	}
	#[test]
	fn local_camera_survives_peer_rekeys_but_requires_join_and_permission() {
		for mut state in [
			test_support::call_demo_state(),
			test_support::voice_demo_state(),
		] {
			state.demo = false;
			let mut role = state.permissions.guilds[&Id(10)].roles.as_ref().unwrap()[0].clone();
			role.bits |= model::permissions::STREAM;
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: Event::Permissions(client_core::permissions::Event::Role {
					guild: Id(10),
					role,
				}),
			});
			for phase in [
				Phase::Waiting,
				Phase::Securing,
				Phase::OpeningAudio,
				Phase::Connected,
				Phase::Waiting,
			] {
				state.voice.active.as_mut().unwrap().phase = phase;
				assert!(
					camera_preview_allowed(&state),
					"phase={phase:?}, channel={:?}, can_call={}, permission={:?}",
					state.voice.active.as_ref().unwrap().channel,
					state.can_call(state.voice.active.as_ref().unwrap().channel),
					state.permission(
						state.voice.active.as_ref().unwrap().channel,
						model::permissions::STREAM
					)
				);
			}
			state.voice.active.as_mut().unwrap().connected_at = None;
			for phase in [
				Phase::Connecting,
				Phase::Securing,
				Phase::OpeningAudio,
				Phase::Failed,
			] {
				state.voice.active.as_mut().unwrap().phase = phase;
				assert!(!camera_preview_allowed(&state));
			}
			state.voice.active.as_mut().unwrap().phase = Phase::Waiting;
			state.gateway_connected = false;
			assert!(!camera_preview_allowed(&state));
			state.voice.active = None;
			assert!(!camera_preview_allowed(&state));
		}
	}

	#[test]
	fn audio_opening_has_a_deadline_that_clears_on_readiness_or_security_pause() {
		let now = Instant::now();
		let mut deadline = None;
		assert_eq!(device_wait(&mut deadline, false, now), Ok(None));
		assert_eq!(
			device_wait(&mut deadline, true, now),
			Ok(Some(DEVICE_OPEN_TIMEOUT))
		);
		assert_eq!(
			device_wait(&mut deadline, true, now + Duration::from_secs(19)),
			Ok(Some(Duration::from_secs(1)))
		);
		assert!(device_wait(&mut deadline, true, now + DEVICE_OPEN_TIMEOUT).is_err());
		assert_eq!(
			device_wait(&mut deadline, false, now + DEVICE_OPEN_TIMEOUT),
			Ok(None)
		);
		assert!(deadline.is_none());
		assert_eq!(
			device_wait(&mut deadline, true, now + DEVICE_OPEN_TIMEOUT),
			Ok(Some(DEVICE_OPEN_TIMEOUT))
		);
	}
	#[test]
	fn microphone_requires_speak_and_focused_push_to_talk_without_vad() {
		use model::permissions as p;
		let mut state = test_support::demo_state();
		let bits = p::VIEW_CHANNEL | p::CONNECT | p::SPEAK;
		state.permissions.guilds.insert(
			Id(10),
			p::Guild {
				id: Id(10),
				owner: Some(Id(999)),
				roles: Some(vec![p::Role {
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
					id: Id(10),
					bits,
				}]),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
			},
		);
		state.permissions.channels.insert(
			Id(25),
			p::Channel {
				id: Id(25),
				guild: Id(10),
				overwrites: Some(vec![]),
			},
		);
		assert!(permission_mutes_microphone(&state, Id(25), false, false));
		assert!(permission_mutes_microphone(&state, Id(25), false, true));
		assert!(permission_mutes_microphone(&state, Id(25), true, false));
		assert!(!permission_mutes_microphone(&state, Id(25), true, true));
		state
			.permissions
			.guilds
			.get_mut(&Id(10))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits |= p::USE_VAD;
		state.permissions.clear_cache();
		assert!(!permission_mutes_microphone(&state, Id(25), false, false));
		state
			.permissions
			.guilds
			.get_mut(&Id(10))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits &= !p::SPEAK;
		state.permissions.clear_cache();
		assert!(permission_mutes_microphone(&state, Id(25), true, true));
		state.permissions.channels.remove(&Id(25));
		state.permissions.clear_cache();
		assert!(permission_mutes_microphone(&state, Id(25), true, true));
	}
	#[test]
	fn group_negotiation_uses_the_voice_roster_and_preserves_one_to_one_peer_pinning() {
		for kind in [1, 3] {
			let mut state = test_support::demo_state();
			state.demo = false;
			let channel = state.channels.iter_mut().find(|c| c.id == Id(22)).unwrap();
			channel.kind = kind;
			let peer = channel.recipients[0].id;
			if kind == 3 {
				let mut other = channel.recipients[0].clone();
				other.id = Id(999);
				channel.recipients.push(other);
			}
			state.start_call(Id(22), true).unwrap();
			let mut manager = Voice::default();
			manager.begin(&state, true).unwrap();
			let pending = manager.pending.as_ref().unwrap();
			assert_eq!(pending.guild, None);
			assert_eq!(pending.peer, (kind == 1).then_some(peer));
			assert!(pending.ring);
			assert!(manager.live.is_none());
		}
	}
	#[test]
	fn guild_negotiation_has_no_dm_peer_or_ringing_and_opens_no_devices() {
		let mut state = test_support::demo_state();
		state.demo = false;
		let command = state.start_call(Id(25), true).unwrap();
		assert!(matches!(
			command,
			Command::Voice(voice::Command::Join { ring: false, .. })
		));
		let mut manager = Voice::default();
		manager.begin(&state, true).unwrap();
		let pending = manager.pending.as_ref().unwrap();
		assert_eq!(pending.guild, Some(Id(10)));
		assert_eq!(pending.peer, None);
		assert!(!pending.ring);
		assert!(manager.live.is_none());
		state.leave_call();
		manager.stop();
		assert!(manager.pending.is_none());
	}
	#[test]
	fn invalidation_leaves_service_but_never_sends_old_account_commands() {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		let mut state = test_support::demo_state();
		state.demo = false;
		state.start_call(Id(25), false).unwrap();
		let mut manager = Voice::default();
		manager.begin(&state, false).unwrap();
		state.disconnect_voice("Discord gateway connection lost; rejoin after reconnecting");
		let mut ui = ui::MessagingUi::default();
		let context = egui::Context::default();
		assert!(matches!(
			manager.poll(&runtime, &mut state, &mut ui, &context),
			Some(Command::Voice(voice::Command::Leave {
				channel: Id(25),
				..
			}))
		));
		assert!(manager.pending.is_none());
		state.leave_call();
		state.start_call(Id(25), false).unwrap();
		manager.begin(&state, false).unwrap();
		state.generation += 1;
		assert!(
			manager
				.poll(&runtime, &mut state, &mut ui, &context)
				.is_none()
		);
		assert!(manager.pending.is_none());
	}
	#[test]
	fn negotiation_requires_matching_request_owner_and_session_without_opening_devices() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.start_call(Id(22), true).unwrap();
		let request = state.voice.active.as_ref().unwrap().request;
		let mut manager = Voice::default();
		manager.begin(&state, true).unwrap();
		let mut stale = Event::Voice(voice::Event::Server {
			channel: Id(22),
			request: request + 1,
			token: Some(Secret::new("synthetic-token".into()).unwrap()),
			endpoint: Some("synthetic.discord.media".into()),
		});
		assert!(manager.observe(&state, &mut stale).is_none());
		assert!(manager.pending.as_ref().unwrap().server.is_none());
		let mut server = Event::Voice(voice::Event::Server {
			channel: Id(22),
			request,
			token: Some(Secret::new("synthetic-token".into()).unwrap()),
			endpoint: Some("synthetic.discord.media".into()),
		});
		assert!(manager.observe(&state, &mut server).is_none());
		assert!(manager.pending.as_ref().unwrap().server.is_some());
		let session = |user, id: &str| {
			Event::Voice(voice::Event::State {
				request: Some(request),
				guild: None,
				member: None,
				server_muted: false,
				server_deafened: false,
				channel: Some(Id(22)),
				user: Id(user),
				session: Some(Secret::new(id.into()).unwrap()),
				muted: false,
				deafened: false,
				video: false,
				streaming: false,
			})
		};
		assert!(
			manager
				.observe(&state, &mut session(2, "other-session"))
				.is_none()
		);
		assert!(manager.pending.as_ref().unwrap().session.is_none());
		assert!(
			manager
				.observe(&state, &mut session(1, "synthetic-session"))
				.is_none()
		);
		assert!(
			manager
				.observe(&state, &mut session(1, "changed-session"))
				.is_some()
		);
		assert!(manager.live.is_none());
		manager.stop();
		assert!(manager.pending.is_none());
	}
}
