//! Desktop ownership for watching one other participant's screen share. The choice is an
//! explicit click; frames are decoded off the render thread and never written to disk.
use client_core::{Command, Event, State, screen, voice};
use discord_voice::{Identity, Status};
use eframe::egui;
use model::Id;
use std::{
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};
use tokio::{runtime::Runtime, sync::watch, task::JoinHandle};
use zeroize::Zeroizing;

const SIGNAL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Context {
	generation: u64,
	channel: Id,
	request: u64,
	stream_request: u64,
	streamer: Id,
}
struct Pending {
	context: Context,
	user: Id,
	peer: Option<Id>,
	session: Zeroizing<String>,
	identity: Arc<Identity>,
	audio: Option<std::sync::mpsc::SyncSender<discord_voice::Frame>>,
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
	task: JoinHandle<()>,
	events: watch::Receiver<Option<Notice>>,
	frames: Arc<Mutex<Option<egui::ColorImage>>>,
}

#[derive(Default)]
pub(super) struct Watch {
	pending: Option<Pending>,
	live: Option<Live>,
	/// The streamer stopped or Discord failed the view; the state choice is cleared next poll.
	ended: Option<&'static str>,
	sequence: u64,
	status: &'static str,
	/// Why the last view stopped; shown as a stage notice until the next request or hang-up.
	notice: &'static str,
}
impl Watch {
	pub fn stop(&mut self) {
		self.pending = None;
		self.ended = None;
		if let Some(live) = self.live.take() {
			live.task.abort();
		}
	}
	fn context(&self) -> Option<Context> {
		self.pending
			.as_ref()
			.map(|pending| pending.context)
			.or_else(|| self.live.as_ref().map(|live| live.context))
	}
	/// Take negotiation secrets before the UI reduces the event. Nothing is persisted.
	pub fn observe(&mut self, state: &State, event: &mut Event) {
		let Event::Voice(voice::Event::Watch {
			channel,
			request,
			stream_request,
			streamer,
			event,
		}) = event
		else {
			return;
		};
		let Some(context) = self.context() else {
			return;
		};
		if context.generation != state.generation
			|| (
				context.channel,
				context.request,
				context.stream_request,
				context.streamer,
			) != (*channel, *request, *stream_request, *streamer)
		{
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
				if let Some(pending) = &mut self.pending {
					match (token.take(), endpoint.take()) {
						(Some(token), Some(endpoint)) => pending.server = Some((token, endpoint)),
						// A null endpoint means Discord is still allocating the stream server.
						(_, None) => pending.server = None,
						(None, Some(_)) => {
							self.ended = Some("Discord omitted the stream connection token");
						}
					}
				}
			}
			screen::Event::Deleted { reason } => {
				self.ended = Some(reason.unwrap_or("The stream ended"));
			}
			screen::Event::Failed(message) => self.ended = Some(message),
		}
	}
	pub fn poll(
		&mut self,
		runtime: &Runtime,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		ctx: &egui::Context,
		call: Option<super::screen::Call<'_>>,
		audio: Option<std::sync::mpsc::SyncSender<discord_voice::Frame>>,
	) -> Option<Command> {
		let wanted = state.voice.active.as_ref().and_then(|active| {
			let streamer = active.watching?;
			matches!(
				active.phase,
				voice::Phase::Connected | voice::Phase::Waiting
			)
			.then_some((state.generation, active.channel, active.request, streamer))
		});
		let current = self.context();
		let mut command = None;
		if let Some(context) = current
			&& (state.demo
				|| wanted
					!= Some((
						context.generation,
						context.channel,
						context.request,
						context.streamer,
					))) {
			self.stop();
			self.status = "Stopped watching";
			if context.generation == state.generation && !state.demo {
				command = Some(Command::Voice(voice::Command::StopWatching {
					channel: context.channel,
					request: context.request,
					stream_request: context.stream_request,
				}));
			}
		}
		if state.voice.active.is_none() {
			self.notice = "";
		}
		if let Some(message) = self.ended.take() {
			let context = self.context();
			self.stop();
			self.status = message;
			self.notice = message;
			state.stop_watching();
			ui.voice_stream_view = None;
			ui.voice_stream_status = self.status;
			return command.or_else(|| {
				let context = context.filter(|context| context.generation == state.generation)?;
				Some(Command::Voice(voice::Command::StopWatching {
					channel: context.channel,
					request: context.request,
					stream_request: context.stream_request,
				}))
			});
		}
		if state.demo {
			ui.voice_stream_status = if wanted.is_some() {
				"Offline preview · no stream is received"
			} else {
				""
			};
			return command;
		}
		if let Some((generation, channel, request, streamer)) = wanted
			&& self.context().is_none()
			&& command.is_none()
		{
			let Some(call) = call.filter(|call| {
				call.generation == generation && call.channel == channel && call.request == request
			}) else {
				ui.voice_stream_status = "Connect the call before watching a stream";
				return None;
			};
			self.sequence = self.sequence.wrapping_add(1);
			let context = Context {
				generation,
				channel,
				request,
				stream_request: self.sequence,
				streamer,
			};
			self.pending = Some(Pending {
				context,
				user: call.user,
				peer: call.peer,
				session: Zeroizing::new(call.session.to_owned()),
				identity: call.identity,
				audio,
				rtc: None,
				server: None,
				started: Instant::now(),
			});
			self.status = "Requesting the stream…";
			self.notice = "";
			command = Some(Command::Voice(voice::Command::WatchStream {
				channel,
				request,
				stream_request: context.stream_request,
				streamer,
			}));
		}
		if let Some(pending) = &self.pending {
			if pending.started.elapsed() >= SIGNAL_TIMEOUT {
				self.ended = Some("Discord did not provide the stream connection; try again");
				ctx.request_repaint();
			} else {
				ctx.request_repaint_after(SIGNAL_TIMEOUT.saturating_sub(pending.started.elapsed()));
			}
		}
		if self
			.pending
			.as_ref()
			.is_some_and(|pending| pending.rtc.is_some() && pending.server.is_some())
		{
			let pending = self.pending.take().expect("complete stream negotiation");
			if let Err(error) = self.start(runtime, pending, ctx) {
				self.ended = Some(error);
			}
		}
		if let Some(live) = &mut self.live {
			match *live.events.borrow_and_update() {
				Some(Notice::Status(status)) => self.status = status,
				Some(Notice::Failed(error)) => self.ended = Some(error),
				None => {}
			}
			if live.task.is_finished() && self.ended.is_none() {
				self.ended = Some("The stream connection ended");
			}
			let image = live.frames.try_lock().ok().and_then(|mut slot| slot.take());
			if let Some(image) = image {
				if let Some(texture) = &mut ui.voice_stream_view {
					texture.set(image, egui::TextureOptions::LINEAR);
				} else {
					ui.voice_stream_view = Some(ctx.load_texture(
						"remote-stream",
						image,
						egui::TextureOptions::LINEAR,
					));
				}
				self.status = "Watching the stream";
			}
		} else {
			ui.voice_stream_view = None;
		}
		ui.voice_stream_status = if wanted.is_some() {
			self.status
		} else {
			self.notice
		};
		command
	}
	fn start(
		&mut self,
		runtime: &Runtime,
		mut pending: Pending,
		ctx: &egui::Context,
	) -> Result<(), &'static str> {
		let (rtc_server, rtc_channel) = pending.rtc.take().ok_or("Missing stream RTC identity")?;
		let (token, endpoint) = pending.server.take().ok_or("Missing stream server")?;
		let session = voice::Secret::new(pending.session.to_string())
			.map_err(|_| "Invalid stream voice session")?;
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
		let frames = Arc::new(Mutex::new(None));
		let slot = frames.clone();
		let wake = ctx.clone();
		let streamer = pending.context.streamer.0;
		let sink: discord_voice::VideoSink = Arc::new(move |frame: discord_voice::RemoteFrame| {
			if frame.user != streamer
				|| frame.rgba.len() != frame.width as usize * frame.height as usize * 4
			{
				return;
			}
			let image = egui::ColorImage::from_rgba_unmultiplied(
				[frame.width as usize, frame.height as usize],
				frame.rgba,
			);
			if let Ok(mut slot) = slot.lock() {
				*slot = Some(image);
			}
			wake.request_repaint();
		});
		let (send, events) = watch::channel(None);
		let wake = ctx.clone();
		let identity = pending.identity;
		let audio = pending.audio;
		let task = runtime.spawn(async move {
			let result = discord_voice::watch_stream(credentials, identity, sink, audio, |event| {
				let status = match event {
					Status::Connecting => "Connecting to the stream…",
					Status::Discovering => "Checking the stream network…",
					Status::TransportReady | Status::Securing => "Securing the stream…",
					Status::WaitingForPeer => "Waiting for the streamer…",
					Status::Ready { .. } => "Stream secured · waiting for video",
					Status::RemoteAudio | Status::Speaking(_) | Status::CameraAvailable(_) => {
						return Ok(());
					}
				};
				send.send_replace(Some(Notice::Status(status)));
				wake.request_repaint();
				Ok(())
			})
			.await;
			if let Err(error) = result {
				send.send_replace(Some(Notice::Failed(error)));
			}
			wake.request_repaint();
		});
		self.status = "Connecting to the stream…";
		self.live = Some(Live {
			context: pending.context,
			task,
			events,
			frames,
		});
		Ok(())
	}
}
impl Drop for Watch {
	fn drop(&mut self) {
		self.stop();
	}
}
