//! The existing Discord adapters run independently of GPUI's render thread.
use client_core::{
	COMMAND_SLOTS, Command, EVENT_SLOTS, Envelope, Event, MAX_EVENT_BYTES,
	auth::{AuthProvider, Failure, SessionSecret},
};
use std::{
	sync::{Arc, LazyLock, Mutex},
	time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};

static CREDENTIAL_GATE: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));
/// Wakes the UI when an event or status is ready; one stored permit coalesces bursts.
const TYPING_SLOTS: usize = 8;
pub static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();

pub struct Backend {
	pub commands: mpsc::Sender<Command>,
	pub events: Events,
	pub status: watch::Receiver<&'static str>,
	cancel: watch::Sender<bool>,
}

pub struct Events {
	receive: mpsc::Receiver<(Envelope, OwnedSemaphorePermit)>,
	/// Ephemeral typing signals have their own few slots and are dropped under pressure.
	pub typing: mpsc::Receiver<Envelope>,
	terminal: watch::Receiver<Option<Failure>>,
	terminal_delivered: bool,
}
impl Events {
	pub fn try_recv(&mut self) -> Result<Envelope, mpsc::error::TryRecvError> {
		match self.receive.try_recv() {
			Ok((event, _permit)) => Ok(event),
			Err(error) => {
				if !self.terminal_delivered
					&& let Some(failure) = *self.terminal.borrow()
				{
					self.terminal_delivered = true;
					return Ok(Envelope {
						generation: 1,
						event: Event::Failure(failure),
					});
				}
				Err(error)
			}
		}
	}
}

struct Output {
	events: mpsc::Sender<(Envelope, OwnedSemaphorePermit)>,
	typing: mpsc::Sender<Envelope>,
	bytes: Arc<Semaphore>,
	startup: Arc<Semaphore>,
}
impl Output {
	fn emit(&self, event: Event) -> Result<(), Failure> {
		// Never let ephemeral typing crowd out messages: it has separate, droppable slots.
		if matches!(&event, Event::Typing(_)) {
			if self
				.typing
				.try_send(Envelope {
					generation: 1,
					event,
				})
				.is_ok()
			{
				WAKE.notify_one();
			}
			return Ok(());
		}
		let startup = event.ready_navigation().is_some();
		let bytes = event
			.bytes()
			.saturating_add(size_of::<(Envelope, OwnedSemaphorePermit)>() - size_of::<Event>());
		let limit = if startup {
			model::account::MAX_BYTES
		} else {
			MAX_EVENT_BYTES
		};
		if bytes > limit {
			return Err(Failure::Capacity);
		}
		let permit = if startup {
			self.startup.clone().try_acquire_owned()
		} else {
			self.bytes.clone().try_acquire_many_owned(bytes as u32)
		}
		.map_err(|_| Failure::Capacity)?;
		self.events
			.try_send((
				Envelope {
					generation: 1,
					event,
				},
				permit,
			))
			.map_err(|error| match error {
				mpsc::error::TrySendError::Full(_) => Failure::Capacity,
				mpsc::error::TrySendError::Closed(_) => Failure::Network,
			})?;
		WAKE.notify_one();
		Ok(())
	}
}

impl Backend {
	pub fn start(demo: bool) -> Self {
		Self::launch(demo, None)
	}

	/// No connection: used while the hosted login page is open.
	pub fn idle() -> Self {
		let mut backend = Self::launch(true, None);
		let (_, status) = watch::channel("");
		backend.status = status;
		backend
	}

	pub fn with_secret(secret: SessionSecret) -> Self {
		Self::launch(false, Some(secret))
	}

	fn launch(demo: bool, secret: Option<SessionSecret>) -> Self {
		let (commands, receive) = mpsc::channel(COMMAND_SLOTS);
		let (events, incoming) = mpsc::channel(4000 + EVENT_SLOTS);
		let (typing, typing_incoming) = mpsc::channel(TYPING_SLOTS);
		let (report, status) = watch::channel(if demo {
			"Offline demo · synthetic data"
		} else {
			"Checking saved login · allow keychain access if macOS asks…"
		});
		let (cancel, mut cancelled) = watch::channel(false);
		let (finished, terminal) = watch::channel(None);
		let mut statuses = status.clone();
		if !demo {
			std::thread::spawn(move || {
				let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
					.enable_all()
					.build()
				else {
					let _ = report.send("Could not start the connection worker");
					let _ = finished.send(Some(Failure::Network));
					WAKE.notify_one();
					return;
				};
				let output = Output {
					events,
					typing,
					bytes: Arc::new(Semaphore::new(EVENT_SLOTS * MAX_EVENT_BYTES)),
					startup: Arc::new(Semaphore::new(1)),
				};
				let forward = async {
					while statuses.changed().await.is_ok() {
						WAKE.notify_one();
					}
					std::future::pending::<()>().await;
				};
				runtime.block_on(async {
					tokio::select! {
						_ = cancelled.changed() => {},
						_ = forward => {},
						result = connect(receive, &output, &report, secret) => {
							if let Err(failure) = result {
								let _ = report.send(failure.label());
								let _ = finished.send(Some(failure));
							}
						}
					}
				});
				WAKE.notify_one();
				// A blocked OS credential prompt must not keep app shutdown waiting.
				runtime.shutdown_background();
			});
		}
		Self {
			commands,
			events: Events {
				receive: incoming,
				typing: typing_incoming,
				terminal,
				terminal_delivered: false,
			},
			status,
			cancel,
		}
	}
}
impl Drop for Backend {
	fn drop(&mut self) {
		let _ = self.cancel.send(true);
	}
}

async fn connect(
	mut commands: mpsc::Receiver<Command>,
	output: &Output,
	status: &watch::Sender<&'static str>,
	supplied: Option<SessionSecret>,
) -> Result<(), Failure> {
	let remember = supplied.is_some();
	let secret = if let Some(secret) = supplied {
		Arc::new(secret)
	} else {
		let Ok(permit) = CREDENTIAL_GATE.clone().try_acquire_owned() else {
			let _ = status.send("Credential store busy. Close its prompt before retrying.");
			return Ok(());
		};
		let loaded = tokio::time::timeout(
			// This executable is new to the keychain; leave time to answer the macOS access prompt.
			Duration::from_secs(60),
			tokio::task::spawn_blocking(move || {
				let _permit = permit;
				platform::load_session()
			}),
		)
		.await;

		match loaded {
			Ok(Ok(Ok(Some(secret)))) => Arc::new(secret),
			Ok(Ok(Ok(None))) => {
				let _ = status.send("No saved login. Choose Sign in with Discord.");
				return Ok(());
			}
			Err(_) => {
				let _ = status.send(
					"Saved-login check timed out. Retry after closing the credential prompt.",
				);
				return Ok(());
			}
			_ => {
				let _ =
					status.send("Saved login unavailable. Choose Sign in with Discord or retry.");
				return Ok(());
			}
		}
	};
	let _ = status.send("Authenticating · connecting…");
	let mut api = discord_api::DiscordApi::new(secret.clone())?;
	let user = api.authenticate().await?;
	let gateway = api.gateway_url().await?;
	let (subscriptions, subscription) = watch::channel(None);
	let (ready_send, ready_receive) = tokio::sync::oneshot::channel();
	let ready_send = Mutex::new(Some(ready_send));
	let save_secret = secret.clone();
	let stream = discord_gateway::run(secret, gateway, subscription, |event| {
		if let Some((owner, _, _)) = event.ready_navigation()
			&& owner.id != user.id
		{
			return Err(Failure::InvalidCredential);
		}
		if matches!(&event, Event::Disconnected | Event::Resync) {
			let _ = status.send("Reconnecting…");
		}
		if matches!(&event, Event::Resumed) {
			let _ = status.send("Connected");
		}
		let ready = event.ready_navigation().is_some();
		output.emit(event)?;
		if ready {
			let _ = status.send("Connected");
			if let Some(send) = ready_send.lock().map_err(|_| Failure::Protocol)?.take() {
				let _ = send.send(());
			}
		}
		Ok(())
	});
	let writes = async {
		// ponytail: serialize REST work; split history from writes if switching latency matters.
		while let Some(command) = commands.recv().await {
			// Dropping the in-flight search is enough; no request goes to Discord.
			if matches!(command, Command::CancelSearch) {
				continue;
			}
			// Member lists are gateway subscriptions, not REST requests.
			if let Command::Members {
				guild,
				channel,
				request,
				list_id,
				thread,
				ranges,
			} = command
			{
				let member = match (guild, channel, list_id) {
					(Some(guild), Some(channel), list_id) if thread || list_id.is_some() => {
						Some(discord_gateway::MemberSubscription {
							guild,
							channel,
							request,
							thread,
							list_id: list_id.unwrap_or_default(),
							ranges,
						})
					}
					_ => None,
				};
				subscriptions.send(member).map_err(|_| Failure::Network)?;
				continue;
			}
			let history = match &command {
				Command::History {
					channel, request, ..
				} => Some((*channel, *request)),
				_ => None,
			};
			let mut event = api.execute(command).await;
			if let Some((channel, request)) = history {
				event = match event {
					Event::Unavailable(_) => Event::HistoryFailed {
						channel,
						request,
						failure: Failure::Forbidden,
					},
					Event::Failure(failure) if !failure.ends_session() => Event::HistoryFailed {
						channel,
						request,
						failure,
					},
					other => other,
				};
			}
			let terminal = match &event {
				Event::Failure(failure)
				| Event::SendResult {
					result: Err(failure),
					..
				} if failure.ends_session() => Some(*failure),
				_ => None,
			};
			output.emit(event)?;
			if let Some(failure) = terminal {
				return Err(failure);
			}
		}
		Ok(())
	};
	let save = async {
		if remember && ready_receive.await.is_ok() {
			let saved = if let Ok(permit) = CREDENTIAL_GATE.clone().try_acquire_owned() {
				tokio::task::spawn_blocking(move || {
					let _permit = permit;
					platform::save_session(&save_secret)?;
					platform::save_account_session(user.id, &save_secret)
				})
				.await
			} else {
				let _ = status.send("Connected · credential store busy; login was not saved");
				std::future::pending().await
			};
			if !matches!(saved, Ok(Ok(()))) {
				let _ =
					status.send("Connected · login could not be saved to the OS credential store");
			}
		}
		std::future::pending::<()>().await;
	};
	let result = tokio::select! {
		result = stream => result.and(Err(Failure::Network)),
		result = writes => result,
		_ = save => Ok(()),
	};
	api.stop();
	result
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn queued_events_release_their_byte_budget() {
		let (send, receive) = mpsc::channel(1);
		let (typing, typing_incoming) = mpsc::channel(1);
		let output = Output {
			events: send,
			typing,
			bytes: Arc::new(Semaphore::new(MAX_EVENT_BYTES)),
			startup: Arc::new(Semaphore::new(1)),
		};
		let (finished, terminal) = watch::channel(None);
		let mut events = Events {
			receive,
			typing: typing_incoming,
			terminal,
			terminal_delivered: false,
		};
		output.emit(Event::Disconnected).unwrap();
		assert!(output.bytes.available_permits() < MAX_EVENT_BYTES);
		assert_eq!(output.emit(Event::Disconnected), Err(Failure::Capacity));
		finished.send(Some(Failure::Capacity)).unwrap();
		assert!(matches!(
			events.try_recv().unwrap().event,
			Event::Disconnected
		));
		assert!(matches!(
			events.try_recv().unwrap().event,
			Event::Failure(Failure::Capacity)
		));
		assert!(events.try_recv().is_err());
		assert_eq!(output.bytes.available_permits(), MAX_EVENT_BYTES);
	}
}
