//! Desktop ownership for one explicitly requested screen share.
use client_core::{
	Command, Event, State,
	screen::{self, Settings},
	voice,
};
use discord_voice::{Identity, Status, screen::Worker};
use eframe::egui;
use model::Id;
use std::{
	sync::{Arc, mpsc},
	time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::watch, task::JoinHandle};
use zeroize::Zeroizing;

const SIGNAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Names why a share ended, under the same opt-in variable as the voice reports. A share
/// that stops by itself otherwise leaves only the latest status, which the stop overwrites.
fn note(event: &str, reason: &str) {
	if std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|value| value == "1") {
		eprintln!("[Serein voice Screen] {event}={reason}");
	}
}

pub(super) struct Call<'a> {
	pub generation: u64,
	pub channel: Id,
	pub request: u64,
	pub user: Id,
	pub peer: Option<Id>,
	pub session: &'a str,
	pub identity: Arc<Identity>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Context {
	generation: u64,
	channel: Id,
	request: u64,
	stream_request: u64,
}
struct Pending {
	context: Context,
	settings: Settings,
	user: Id,
	peer: Option<Id>,
	session: Zeroizing<String>,
	identity: Arc<Identity>,
	rtc: Option<(Id, Id)>,
	server: Option<(voice::Secret, String)>,
	started: Instant,
}
#[derive(Clone, Copy)]
enum Notice {
	Status(&'static str),
	Failed(&'static str),
}
struct Live {
	context: Context,
	worker: Worker,
	task: JoinHandle<()>,
	events: watch::Receiver<Option<Notice>>,
}
struct Closing {
	context: Context,
	started: Instant,
	timed_out: bool,
}
struct SourceScan {
	generation: u64,
	context: Option<(u64, Id, u64)>,
	receive: mpsc::Receiver<Result<Vec<screen::Source>, &'static str>>,
}

#[derive(Default)]
pub(super) struct Screen {
	pending: Option<Pending>,
	live: Option<Live>,
	closing: Option<Closing>,
	retiring: Option<mpsc::Receiver<Result<(), &'static str>>>,
	source_scan: Option<SourceScan>,
	command: Option<Command>,
	sequence: u64,
	status: &'static str,
}
impl Screen {
	pub fn stop(&mut self) {
		self.pending = None;
		self.command = None;
		self.closing = None;
		// Keep one in-flight OS enumeration tracked until it completes.
		self.retire_live();
		self.status = "Screen sharing stopped";
	}

	fn retire_live(&mut self) {
		if let Some(live) = self.live.take() {
			live.task.abort();
			self.retiring = Some(live.worker.shutdown());
		}
	}

	fn context(&self) -> Option<Context> {
		self.pending
			.as_ref()
			.map(|pending| pending.context)
			.or_else(|| self.live.as_ref().map(|live| live.context))
			.or_else(|| self.closing.as_ref().map(|closing| closing.context))
	}

	fn request_stop(&mut self, message: &'static str) {
		note("share_stopped", message);
		let Some(context) = self.context() else {
			self.status = message;
			return;
		};
		self.close(context, message);
	}

	fn close(&mut self, context: Context, message: &'static str) {
		self.pending = None;
		self.retire_live();
		if self.closing.is_none() {
			self.closing = Some(Closing {
				context,
				started: Instant::now(),
				timed_out: false,
			});
			self.command = Some(Command::Voice(voice::Command::StopStream {
				channel: context.channel,
				request: context.request,
				stream_request: context.stream_request,
			}));
		}
		self.status = message;
	}

	/// Take matching stream secrets before core reduction. Nothing is persisted.
	pub fn observe(&mut self, state: &State, event: &mut Event) {
		let Event::Voice(voice::Event::Stream {
			channel,
			request,
			stream_request,
			event,
		}) = event
		else {
			return;
		};
		let incoming = Context {
			generation: state.generation,
			channel: *channel,
			request: *request,
			stream_request: *stream_request,
		};
		if self.context() != Some(incoming) {
			return;
		}
		match event {
			screen::Event::Created {
				rtc_server,
				rtc_channel,
			} => {
				if let Some(pending) = &mut self.pending {
					pending.rtc = Some((*rtc_server, *rtc_channel));
				}
			}
			screen::Event::Server { token, endpoint } => {
				if self.live.is_some() {
					let _ = token.take();
					let _ = endpoint.take();
					self.request_stop("Discord changed the active screen-share server");
					return;
				}
				let Some(pending) = &mut self.pending else {
					return;
				};
				let Some(endpoint) = endpoint.take() else {
					let _ = token.take();
					self.request_stop("Discord screen-share server is unavailable");
					return;
				};
				let Some(token) = token.take() else {
					self.request_stop("Discord omitted the screen-share token");
					return;
				};
				pending.server = Some((token, endpoint));
			}
			screen::Event::Deleted { reason } => {
				self.pending = None;
				self.retire_live();
				self.closing = None;
				// A server-side end names its cause, so it is never mistaken for a local stop.
				self.status = reason.unwrap_or("Screen sharing stopped");
			}
			screen::Event::Failed(message) => self.request_stop(message),
		}
	}

	pub fn poll(
		&mut self,
		runtime: &Runtime,
		state: &State,
		ui: &mut ui::MessagingUi,
		ctx: &egui::Context,
		call: Option<Call<'_>>,
	) -> Option<Command> {
		if self
			.retiring
			.as_ref()
			.is_some_and(|done| !matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)))
		{
			self.retiring = None;
		}
		if state.demo {
			if self.context().is_some() || self.live.is_some() {
				self.stop();
			}
			ui.screen.refresh_requested = false;
			ui.screen.request = None;
			ui.screen.busy = false;
			ui.screen.supported = false;
			ui.screen.preview = None;
			ui.screen.capture_status = None;
			return None;
		}
		ui.screen.supported = discord_voice::screen::supported();
		if !ui.screen.open && ui.screen.request.is_none() && self.context().is_none() {
			ui.screen.sources.clear();
			ui.screen.selected = None;
		}

		if ui.screen.refresh_requested {
			ui.screen.refresh_requested = false;
			if !state.demo && self.source_scan.is_none() && ui.screen.supported {
				let (send, receive) = mpsc::sync_channel(1);
				let wake = ctx.clone();
				self.status = "Looking for screens and windows…";
				match std::thread::Builder::new()
					.name("screen-sources".into())
					.spawn(move || {
						let _ = send.send(discord_voice::screen::sources());
						wake.request_repaint();
					}) {
					Ok(_) => {
						self.source_scan = Some(SourceScan {
							generation: state.generation,
							context: ui.screen.context,
							receive,
						})
					}
					Err(_) => self.status = "Could not start screen source discovery",
				}
			} else if !ui.screen.supported {
				self.status = "Screen sharing is unavailable on this platform";
			}
		}
		if let Some(scan) = &self.source_scan {
			match scan.receive.try_recv() {
				Ok(_)
					if !ui.screen.open
						|| scan.generation != state.generation
						|| scan.context != ui.screen.context =>
				{
					self.source_scan = None;
					if ui.screen.open {
						ui.screen.refresh_requested = true;
						ctx.request_repaint();
					}
				}
				Ok(Ok(sources)) => {
					ui.screen.sources = sources;
					ui.screen.selected = ui
						.screen
						.selected
						.filter(|selected| {
							ui.screen
								.sources
								.iter()
								.any(|source| source.id == *selected)
						})
						.or_else(|| ui.screen.sources.first().map(|source| source.id));
					self.status = if ui.screen.sources.is_empty() {
						"No shareable screens or windows were found"
					} else if cfg!(target_os = "linux") {
						"Choose the system picker or, on X11, explicitly share the entire desktop"
					} else {
						"Choose a screen or window"
					};
					self.source_scan = None;
				}
				Ok(Err(error)) => {
					self.status = error;
					self.source_scan = None;
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					self.status = "Screen source discovery stopped";
					self.source_scan = None;
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}

		let call_context = call
			.as_ref()
			.map(|call| (call.generation, call.channel, call.request));
		if let Some(current) = self.context()
			&& call_context != Some((current.generation, current.channel, current.request))
		{
			self.stop();
		}
		if let Some(context) = self
			.pending
			.as_ref()
			.map(|pending| pending.context)
			.or_else(|| self.live.as_ref().map(|live| live.context))
			&& !state.can_stream(context.channel)
		{
			self.request_stop("Screen-share permission was removed; stopping sharing");
		}

		if let Some(request) = ui.screen.request.take() {
			match request {
				ui::screen::Request::Start(settings) => {
					let requested = ui.screen.context;
					let valid = settings
						.valid()
						.then_some(())
						.and(call.as_ref())
						.filter(|call| {
							requested == Some((call.generation, call.channel, call.request))
								&& state.generation == call.generation
								&& state.can_stream(call.channel)
								&& state.voice.active.as_ref().is_some_and(|active| {
									active.channel == call.channel
										&& active.request == call.request
										&& matches!(
											active.phase,
											voice::Phase::Connected | voice::Phase::Waiting
										)
								})
						});
					if self.context().is_some() || self.retiring.is_some() {
						self.status = "Previous screen share is still closing";
					} else if let Some(call) = valid {
						self.sequence = self.sequence.wrapping_add(1).max(1);
						let context = Context {
							generation: call.generation,
							channel: call.channel,
							request: call.request,
							stream_request: self.sequence,
						};
						self.pending = Some(Pending {
							context,
							settings,
							user: call.user,
							peer: call.peer,
							session: Zeroizing::new(call.session.to_owned()),
							identity: call.identity.clone(),
							rtc: None,
							server: None,
							started: Instant::now(),
						});
						self.status = "Requesting screen-share connection…";
						self.command = Some(Command::Voice(voice::Command::StartStream {
							channel: context.channel,
							request: context.request,
							stream_request: context.stream_request,
						}));
					} else {
						self.status = "Screen sharing requires the current connected call";
					}
				}
				ui::screen::Request::Stop => self.request_stop("Stopping screen sharing…"),
			}
		}

		if self
			.pending
			.as_ref()
			.is_some_and(|pending| pending.started.elapsed() >= SIGNAL_TIMEOUT)
		{
			self.request_stop(
				"Discord did not provide screen-share connection details within 30 seconds",
			);
		}
		if let Some(pending) = &self.pending {
			ctx.request_repaint_after(SIGNAL_TIMEOUT.saturating_sub(pending.started.elapsed()));
		}

		if self
			.pending
			.as_ref()
			.is_some_and(|pending| pending.rtc.is_some() && pending.server.is_some())
		{
			self.finish_start(runtime, ctx);
		}

		let mut failure = None;
		if let Some(live) = &mut self.live {
			match *live.events.borrow_and_update() {
				Some(Notice::Status(status)) => self.status = status,
				Some(Notice::Failed(error)) => failure = Some(error),
				None => {}
			}
			if failure.is_none() {
				failure = live
					.worker
					.result()
					.map(|result| result.err().unwrap_or("Screen capture stopped"));
			}
			if live.task.is_finished() && failure.is_none() {
				failure = Some("Screen-share connection ended");
			}
		}
		if let Some(error) = failure {
			self.request_stop(error);
		}
		if let Some(live) = &self.live {
			live.worker.set_preview_visible(
				ui.screen.preview.is_none()
					|| (state.selected == Some(live.context.channel)
						&& !ctx.input(|input| input.viewport().minimized.unwrap_or(false))),
			);
			if let Some(frame) = live.worker.take_preview() {
				let image = egui::ColorImage::from_rgba_unmultiplied(
					[frame.width() as usize, frame.height() as usize],
					frame.as_raw(),
				);
				if let Some(texture) = &mut ui.screen.preview {
					texture.set(image, egui::TextureOptions::LINEAR);
				} else {
					ui.screen.preview =
						Some(ctx.load_texture("local-screen", image, egui::TextureOptions::LINEAR));
					let cue = model::notification_preferences::Sound::ScreenShareOn;
					if ui.notification_options.allows(cue) {
						ui.notification_preview = Some(cue);
						ctx.request_repaint();
					}
				}
			}
		} else {
			ui.screen.preview = None;
		}

		if let Some(closing) = &mut self.closing {
			if !closing.timed_out && closing.started.elapsed() >= SIGNAL_TIMEOUT {
				closing.timed_out = true;
				self.status = "Discord did not acknowledge stopping screen share; leave the call before sharing again";
			}
			if !closing.timed_out {
				ctx.request_repaint_after(SIGNAL_TIMEOUT.saturating_sub(closing.started.elapsed()));
			}
		}
		ui.screen.busy = self.pending.is_some()
			|| self.live.is_some()
			|| self.closing.is_some()
			|| self.retiring.is_some();
		ui.screen.status = self.status;
		ui.screen.capture_status = self
			.live
			.as_ref()
			.and_then(|live| live.worker.capture_status());
		self.command.take()
	}
	fn finish_start(&mut self, runtime: &Runtime, ctx: &egui::Context) {
		let pending = self.pending.take().expect("complete screen negotiation");
		let context = pending.context;
		if let Err(error) = self.start(runtime, pending, ctx) {
			self.close(context, error);
		}
	}

	fn start(
		&mut self,
		runtime: &Runtime,
		mut pending: Pending,
		ctx: &egui::Context,
	) -> Result<(), &'static str> {
		let (rtc_server, rtc_channel) = pending.rtc.take().ok_or("Missing screen RTC identity")?;
		let (token, endpoint) = pending.server.take().ok_or("Missing screen server")?;
		let session = voice::Secret::new(pending.session.to_string())
			.map_err(|_| "Invalid screen voice session")?;
		let credentials = voice::VoiceConnection {
			channel: rtc_channel,
			guild: Some(rtc_server),
			user: pending.user,
			peer: pending.peer,
			session,
			token,
			endpoint,
			request: pending.context.stream_request,
		};
		let wake = ctx.clone();
		let (worker, video) = Worker::start(pending.settings, move || wake.request_repaint())?;
		let (send, events) = watch::channel(None);
		let wake = ctx.clone();
		let identity = pending.identity;
		let task = runtime.spawn(async move {
			let result = discord_voice::run_stream(credentials, identity, video, |event| {
				let status = match event {
					Status::Connecting => "Connecting screen-share transport…",
					Status::Discovering => "Checking screen-share network…",
					Status::TransportReady | Status::Securing => "Securing screen video…",
					Status::WaitingForPeer => "Screen preview · waiting for others",
					Status::Ready { .. } => "Sharing your screen",
					Status::RemoteAudio | Status::Speaking(_) | Status::CameraAvailable(_) => {
						return Ok(());
					}
				};
				send.send_replace(Some(Notice::Status(status)));
				wake.request_repaint();
				Ok(())
			})
			.await;
			match result {
				Ok(()) => note("stream_transport_stopped", "ok"),
				Err(error) => {
					note("stream_transport_stopped", error);
					send.send_replace(Some(Notice::Failed(error)));
				}
			}
			wake.request_repaint();
		});
		self.status = "Connecting screen-share transport…";
		self.live = Some(Live {
			context: pending.context,
			worker,
			task,
			events,
		});
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn waiting_dm_and_guild_calls_can_request_sharing_without_opening_capture() {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		let ctx = egui::Context::default();
		for channel in [Id(22), Id(25)] {
			let mut state = test_support::demo_state();
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
			state.start_call(channel, false).unwrap();
			let active = state.voice.active.as_mut().unwrap();
			active.phase = voice::Phase::Waiting;
			let request = active.request;
			let mut ui = ui::MessagingUi::default();
			ui.screen.context = Some((state.generation, channel, request));
			ui.screen.request = Some(ui::screen::Request::Start(Settings {
				source: screen::SourceId::Display(1),
				width: 1280,
				height: 720,
				fps: 30,
				cursor: true,
				audio: false,
			}));
			let call = Call {
				generation: state.generation,
				channel,
				request,
				user: Id(1),
				peer: (channel == Id(22)).then_some(Id(2)),
				session: "synthetic-session",
				identity: Identity::generate(),
			};
			let mut screen = Screen::default();
			assert!(matches!(
				screen.poll(&runtime, &state, &mut ui, &ctx, Some(call)),
				Some(Command::Voice(voice::Command::StartStream { .. }))
			));
			assert!(screen.pending.is_some());
			assert!(screen.live.is_none());
		}
	}

	fn pending(context: Context, settings: Settings) -> Pending {
		Pending {
			context,
			settings,
			user: Id(1),
			peer: None,
			session: Zeroizing::new("synthetic-session".into()),
			identity: Identity::generate(),
			rtc: Some((Id(30), Id(31))),
			server: Some((
				voice::Secret::new("synthetic-token".into()).unwrap(),
				"voice.discord.media:443".into(),
			)),
			started: Instant::now(),
		}
	}

	#[test]
	fn startup_failure_and_permission_revocation_request_stream_cleanup() {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		let ctx = egui::Context::default();
		let context = Context {
			generation: 0,
			channel: Id(20),
			request: 7,
			stream_request: 8,
		};
		let mut screen = Screen {
			pending: Some(pending(
				context,
				Settings {
					source: screen::SourceId::Display(1),
					width: 1,
					height: 1,
					fps: 30,
					cursor: true,
					audio: false,
				},
			)),
			..Screen::default()
		};
		screen.finish_start(&runtime, &ctx);
		assert!(screen.pending.is_none() && screen.closing.is_some());
		assert!(matches!(
			screen.command.take(),
			Some(Command::Voice(voice::Command::StopStream {
				stream_request: 8,
				..
			}))
		));

		let mut state = State::default();
		state.voice.active = Some(voice::Call {
			channel: Id(20),
			guild: Some(Id(10)),
			connected_at: Some(Instant::now()),
			server_muted: false,
			server_deafened: false,
			request: 7,
			phase: voice::Phase::Connected,
			muted: false,
			deafened: false,
			participants: vec![],
			error: None,
			camera: false,
			watching: None,
		});
		let mut screen = Screen {
			pending: Some(Pending {
				rtc: None,
				server: None,
				..pending(
					context,
					Settings {
						source: screen::SourceId::Display(1),
						width: 1280,
						height: 720,
						fps: 30,
						cursor: true,
						audio: false,
					},
				)
			}),
			..Screen::default()
		};
		let mut ui = ui::MessagingUi::default();
		let call = Call {
			generation: 0,
			channel: Id(20),
			request: 7,
			user: Id(1),
			peer: None,
			session: "synthetic-session",
			identity: Identity::generate(),
		};
		assert!(matches!(
			screen.poll(&runtime, &state, &mut ui, &ctx, Some(call)),
			Some(Command::Voice(voice::Command::StopStream {
				stream_request: 8,
				..
			}))
		));
		assert!(ui.screen.busy);
		assert!(ui.screen.status.contains("permission"));
	}
}
