use discord_protocol::rpc::{self, Activity};
use eframe::egui;
use model::User;
use std::{io, time::Duration};
use tokio::{
	io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
	sync::{mpsc, watch},
	task::JoinSet,
	time::{Instant, timeout},
};

pub type Detection = Result<Option<model::RichActivity>, &'static str>;
const MAX_CLIENTS: usize = 8;
type Update = (usize, Option<Activity>);

/// Sharing owns the listener and all clients. Dropping it cancels every pending IPC/HTTP operation.
pub async fn run(
	enabled: watch::Receiver<bool>,
	activity: watch::Sender<Option<Activity>>,
	report: watch::Sender<Detection>,
	ctx: egui::Context,
	user: User,
) {
	run_enabled(enabled, &activity, &report, &ctx, || {
		listen(&activity, &report, &ctx, &user)
	})
	.await;
}
async fn run_enabled<F, Fut>(
	mut enabled: watch::Receiver<bool>,
	activity: &watch::Sender<Option<Activity>>,
	report: &watch::Sender<Detection>,
	ctx: &egui::Context,
	start: F,
) where
	F: Fn() -> Fut,
	Fut: std::future::Future<Output = Result<(), &'static str>>,
{
	loop {
		activity.send_replace(None);
		let _ = report.send_replace(Ok(None));
		ctx.request_repaint();
		while !*enabled.borrow_and_update() {
			if enabled.changed().await.is_err() {
				return;
			}
		}
		tokio::select! {
			biased;
			_ = enabled.changed() => {},
			result = start() => {
				if let Err(error) = result {
					activity.send_replace(None);
					let _ = report.send_replace(Err(error));
					ctx.request_repaint();
				}
				// Do not repeatedly bind or retry failed metadata/service operations.
				if enabled.changed().await.is_err() { return; }
			}
		}
		if enabled.has_changed().is_err() {
			activity.send_replace(None);
			let _ = report.send_replace(Ok(None));
			return;
		}
	}
}

async fn listen(
	activity: &watch::Sender<Option<Activity>>,
	report: &watch::Sender<Detection>,
	ctx: &egui::Context,
	user: &User,
) -> Result<(), &'static str> {
	let mut listener = platform::game_activity::Listener::bind().map_err(
		|_| "Game activity is unavailable. Close other Discord clients, then turn sharing off and on.",
	)?;
	let client = discord_api::rpc::client()?;
	let cooldown = std::sync::Arc::new(tokio::sync::Mutex::new(Instant::now()));
	let (send, mut receive) = mpsc::channel::<Update>(16);
	let mut workers = JoinSet::new();
	let mut slots = [false; MAX_CLIENTS];
	let mut values: [Option<(Instant, Activity)>; MAX_CLIENTS] = std::array::from_fn(|_| None);
	let mut next_accept = Instant::now();
	loop {
		tokio::select! {
			// An admission interval also bounds credential-free metadata requests (two per client).
			accepted = async {
				tokio::time::sleep_until(next_accept).await;
				listener.accept().await
			}, if workers.len() < MAX_CLIENTS => {
				let stream = accepted.map_err(|_| "Game activity stopped. Turn sharing off and on to retry.")?;
				next_accept = Instant::now() + Duration::from_secs(5);
				let slot = slots.iter().position(|used| !used).expect("bounded IPC slots");
				slots[slot] = true;
				let send = send.clone(); let user = user.clone(); let client = client.clone(); let cooldown = cooldown.clone();
				workers.spawn(async move {
					let result = serve(stream, &user, send, slot, |id| async move { discord_api::rpc::metadata(&client, &mut *cooldown.lock().await, id).await }).await;
					(slot, result)
				});
			},
			Some((slot, value)) = receive.recv() => {
				values[slot] = value.map(|value| (Instant::now(), value));
				publish(&values, activity, report, ctx);
			},
			Some(result) = workers.join_next(), if !workers.is_empty() => {
				let (slot, result) = result.map_err(|_| "Game activity stopped. Turn sharing off and on to retry.")?;
				// Drain accepted updates before retiring the slot so an old update cannot revive it.
				while let Ok((index, value)) = receive.try_recv() {
					values[index] = value.map(|value| (Instant::now(), value));
				}
				slots[slot] = false;
				values[slot] = None;
				publish(&values, activity, report, ctx);
				if let Err(error) = result && activity.borrow().is_none() {
					let _ = report.send_replace(Err(error)); ctx.request_repaint();
				}
			}
		}
	}
}
fn publish(
	values: &[Option<(Instant, Activity)>; MAX_CLIENTS],
	activity: &watch::Sender<Option<Activity>>,
	report: &watch::Sender<Detection>,
	ctx: &egui::Context,
) {
	let latest = values
		.iter()
		.flatten()
		.max_by_key(|(at, _)| *at)
		.map(|(_, value)| value.clone());
	let display = latest.as_ref().map(display_activity);
	activity.send_if_modified(|current| {
		if *current == latest {
			return false;
		}
		*current = latest;
		true
	});
	if report.send_if_modified(|current| {
		if *current == Ok(display.clone()) {
			return false;
		}
		*current = Ok(display);
		true
	}) {
		ctx.request_repaint();
	}
}

fn display_activity(activity: &Activity) -> model::RichActivity {
	let text = |value: &Option<String>| {
		value
			.as_deref()
			.map(str::trim)
			.filter(|text| !text.is_empty())
			.map(str::to_owned)
	};
	let image = |id: &String| {
		id.parse().ok().map(|asset| model::ActivityImage::Asset {
			application: activity.application_id,
			asset,
		})
	};
	let assets = activity.assets.as_ref();
	let large = assets
		.and_then(|assets| assets.large_image.as_ref())
		.and_then(image);
	let small = assets
		.and_then(|assets| assets.small_image.as_ref())
		.and_then(image);
	let primary = large
		.clone()
		.or_else(|| small.clone())
		.unwrap_or(model::ActivityImage::Application(activity.application_id));
	let small_image = small
		.or_else(|| {
			(large.is_some() && assets.is_some_and(|assets| assets.small_image.is_none()))
				.then_some(model::ActivityImage::Application(activity.application_id))
		})
		.filter(|small| *small != primary);
	model::RichActivity {
		kind: activity.kind,
		name: activity.name.trim().to_owned(),
		details: text(&activity.details),
		state: text(&activity.state),
		image: Some(primary),
		small_image,
		started_at: activity
			.timestamps
			.as_ref()
			.and_then(|timestamps| timestamps.start),
	}
}

#[cfg(feature = "demo")]
pub fn demo_activity() -> model::RichActivity {
	model::RichActivity {
		kind: 0,
		name: "osu!".into(),
		details: Some("Playing a synthetic beatmap".into()),
		state: Some("Solo".into()),
		image: None,
		small_image: None,
		started_at: None,
	}
}

async fn serve<S, F, Fut>(
	mut stream: S,
	user: &User,
	send: mpsc::Sender<Update>,
	slot: usize,
	resolve: F,
) -> Result<(), &'static str>
where
	S: AsyncRead + AsyncWrite + Unpin,
	F: FnOnce(model::Id) -> Fut,
	Fut: std::future::Future<Output = Result<discord_api::rpc::Metadata, &'static str>>,
{
	let invalid = "A game sent activity that could not be read.";
	let (opcode, bytes) = timeout(Duration::from_secs(10), read_frame(&mut stream))
		.await
		.map_err(|_| invalid)?
		.map_err(|_| invalid)?;
	if opcode != 0 {
		return Err(invalid);
	}
	let application = rpc::decode_handshake(&bytes).map_err(|_| invalid)?;
	write_frame(&mut stream, 1, &rpc::ready(user.id, &user.name))
		.await
		.map_err(|_| invalid)?;
	let mut resolve = Some(resolve);
	let mut metadata = None;
	loop {
		let (opcode, bytes) = match read_frame(&mut stream).await {
			Ok(frame) => frame,
			Err(error)
				if matches!(
					error.kind(),
					io::ErrorKind::UnexpectedEof
						| io::ErrorKind::BrokenPipe
						| io::ErrorKind::ConnectionReset
				) =>
			{
				return Ok(());
			}
			Err(_) => return Err(invalid),
		};
		match opcode {
			2 => return Ok(()),
			3 => write_frame(&mut stream, 4, &bytes)
				.await
				.map_err(|_| invalid)?,
			4 => {}
			1 => {
				let command = match rpc::decode_command(&bytes) {
					Ok(command) => command,
					Err(_) => {
						write_frame(&mut stream, 1, &rpc::error_for_payload(&bytes))
							.await
							.map_err(|_| invalid)?;
						tokio::time::sleep(Duration::from_millis(100)).await;
						continue;
					}
				};
				let ack = rpc::acknowledge(&command);
				let value = if let Some(fields) = command.activity {
					if metadata.is_none() {
						metadata = Some(
							resolve.take().expect("one application lookup")(application).await?,
						);
					}
					let metadata = metadata.as_ref().expect("resolved metadata");
					let mut value = fields
						.into_activity(application, metadata.name.clone())
						.map_err(|_| invalid)?;
					if let Some(assets) = value.assets.as_mut() {
						assets.large_image = assets
							.large_image
							.as_deref()
							.and_then(|key| metadata.asset(key));
						assets.small_image = assets
							.small_image
							.as_deref()
							.and_then(|key| metadata.asset(key));
						if assets.large_image.is_none() {
							assets.large_text = None;
						}
						if assets.small_image.is_none() {
							assets.small_text = None;
						}
					}
					Some(value)
				} else {
					None
				};
				// A client that disconnected during metadata lookup must not publish stale activity.
				write_frame(&mut stream, 1, &ack)
					.await
					.map_err(|_| invalid)?;
				send.send((slot, value)).await.map_err(|_| invalid)?;
			}
			_ => return Err(invalid),
		}
		// Backpressure on chatty local senders; no polling when clients are idle.
		tokio::time::sleep(Duration::from_millis(100)).await;
	}
}

async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<(u32, Vec<u8>)> {
	let mut header = [0; 8];
	// Idle clients may publish only once. Once a frame begins it must finish promptly.
	stream.read_exact(&mut header[..1]).await?;
	timeout(Duration::from_secs(5), async {
		stream.read_exact(&mut header[1..]).await?;
		let opcode = u32::from_le_bytes(header[..4].try_into().expect("opcode"));
		let size = u32::from_le_bytes(header[4..].try_into().expect("size")) as usize;
		if size > rpc::MAX_FRAME_BYTES {
			return Err(io::Error::new(
				io::ErrorKind::InvalidData,
				"IPC frame limit",
			));
		}
		let mut bytes = vec![0; size];
		stream.read_exact(&mut bytes).await?;
		Ok((opcode, bytes))
	})
	.await
	.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}
async fn write_frame(
	stream: &mut (impl AsyncWrite + Unpin),
	opcode: u32,
	bytes: &[u8],
) -> io::Result<()> {
	if bytes.len() > rpc::MAX_FRAME_BYTES {
		return Err(io::Error::from(io::ErrorKind::InvalidData));
	}
	// Some game SDKs parse each pipe read as a complete frame; a separate
	// opcode/header write makes them reject READY and disconnect immediately.
	let mut frame = Vec::with_capacity(8 + bytes.len());
	frame.extend_from_slice(&opcode.to_le_bytes());
	frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
	frame.extend_from_slice(bytes);
	timeout(Duration::from_secs(5), async {
		stream.write_all(&frame).await?;
		stream.flush().await
	})
	.await
	.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

#[cfg(test)]
mod tests {
	use super::*;
	fn user() -> User {
		User {
			id: model::Id(1),
			name: "Synthetic user".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		}
	}
	#[tokio::test]
	async fn ipc_handshake_rich_update_clear_ping_and_disconnect() {
		let (mut game, server) = tokio::io::duplex(32 * 1024);
		let (send, mut receive) = mpsc::channel(16);
		let worker = tokio::spawn(async move {
			serve(server, &user(), send, 0, |_| async {
				Ok(discord_api::rpc::Metadata {
					name: "A game outside the old list".into(),
					assets: vec![discord_api::rpc::Asset {
						id: model::Id(9),
						name: "map".into(),
					}],
				})
			})
			.await
		});
		write_frame(&mut game, 0, br#"{"v":1,"client_id":"7"}"#)
			.await
			.unwrap();
		let ready = read_frame(&mut game).await.unwrap();
		assert_eq!(ready.0, 1);
		assert!(String::from_utf8(ready.1).unwrap().contains("READY"));
		write_frame(
			&mut game,
			1,
			br#"{"cmd":"SUBSCRIBE","evt":"ACTIVITY_JOIN","nonce":"subscription"}"#,
		)
		.await
		.unwrap();
		let error = read_frame(&mut game).await.unwrap();
		assert!(String::from_utf8(error.1).unwrap().contains("subscription"));
		write_frame(&mut game, 3, b"ping").await.unwrap();
		assert_eq!(read_frame(&mut game).await.unwrap(), (4, b"ping".to_vec()));
		write_frame(&mut game, 1, br#"{"cmd":"SET_ACTIVITY","nonce":"one","args":{"pid":123,"activity":{"details":"Level 4","state":"In match","timestamps":{"start":1700000000},"assets":{"large_image":"map","small_image":"https://localhost/private"},"secrets":{"join":"synthetic-not-forwarded"}}}}"#).await.unwrap();
		let ack = read_frame(&mut game).await.unwrap();
		assert!(String::from_utf8(ack.1).unwrap().contains("one"));
		let (_, value) = receive.recv().await.unwrap();
		let value = value.unwrap();
		assert_eq!(value.application_id, model::Id(7));
		assert_eq!(value.name, "A game outside the old list");
		assert_eq!(value.details.as_deref(), Some("Level 4"));
		assert_eq!(value.state.as_deref(), Some("In match"));
		assert_eq!(value.timestamps.unwrap().start, Some(1700000000000));
		let assets = value.assets.unwrap();
		assert_eq!(assets.large_image.as_deref(), Some("9"));
		assert!(assets.small_image.is_none());
		for clear in [
			br#"{"cmd":"SET_ACTIVITY","nonce":"clear","args":{"pid":123,"activity":null}}"#
				.as_slice(),
			br#"{"cmd":"SET_ACTIVITY","nonce":"legacy-clear","args":{"pid":123}}"#,
		] {
			write_frame(&mut game, 1, clear).await.unwrap();
			read_frame(&mut game).await.unwrap();
			assert_eq!(receive.recv().await.unwrap(), (0, None));
		}
		drop(game);
		timeout(Duration::from_secs(2), worker)
			.await
			.unwrap()
			.unwrap()
			.unwrap();
	}
	#[tokio::test]
	async fn disconnect_during_metadata_lookup_never_queues_activity() {
		let (mut game, server) = tokio::io::duplex(4096);
		let (send, mut receive) = mpsc::channel(16);
		let (begun, started) = tokio::sync::oneshot::channel();
		let (finish, finished) = tokio::sync::oneshot::channel();
		let worker = tokio::spawn(async move {
			serve(server, &user(), send, 0, |_| async {
				begun.send(()).unwrap();
				finished.await.unwrap();
				Ok(discord_api::rpc::Metadata {
					name: "Synthetic game".into(),
					assets: vec![],
				})
			})
			.await
		});
		write_frame(&mut game, 0, br#"{"v":1,"client_id":"7"}"#)
			.await
			.unwrap();
		read_frame(&mut game).await.unwrap();
		write_frame(
			&mut game,
			1,
			br#"{"cmd":"SET_ACTIVITY","nonce":"one","args":{"pid":123,"activity":{}}}"#,
		)
		.await
		.unwrap();
		started.await.unwrap();
		drop(game);
		finish.send(()).unwrap();
		assert!(worker.await.unwrap().is_err());
		assert!(receive.recv().await.is_none());
	}

	#[tokio::test]
	#[cfg(windows)]
	async fn replies_reach_pending_game_reads_as_complete_frames() {
		use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
		let name = format!(
			r"\\.\pipe\serein-test-reply-{}-{}",
			std::process::id(),
			getrandom::u64().unwrap()
		);
		let mut server = ServerOptions::new()
			.first_pipe_instance(true)
			.create(&name)
			.unwrap();
		let mut game = ClientOptions::new().open(&name).unwrap();
		server.connect().await.unwrap();
		let reply = rpc::ready(user().id, &user().name);
		let mut buffer = vec![0; rpc::MAX_FRAME_BYTES + 8];
		let read = game.read(&mut buffer);
		tokio::pin!(read);
		// osu!'s SDK parses each completed pipe read as a whole frame. Start its
		// read first so a separately written header cannot hide in buffered data.
		tokio::select! {
			biased;
			result = &mut read => panic!("unexpected read before reply: {result:?}"),
			_ = tokio::task::yield_now() => {}
		}
		write_frame(&mut server, 1, &reply).await.unwrap();
		let length = timeout(Duration::from_secs(2), read)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(length, reply.len() + 8);
		assert_eq!(&buffer[..4], &1u32.to_le_bytes());
		assert_eq!(&buffer[4..8], &(reply.len() as u32).to_le_bytes());
		assert_eq!(&buffer[8..length], reply);
	}
	#[tokio::test]
	async fn frames_bound_before_allocation_and_handle_partial_reads() {
		let (mut client, mut server) = tokio::io::duplex(64);
		client.write_all(&1u32.to_le_bytes()).await.unwrap();
		client
			.write_all(&(rpc::MAX_FRAME_BYTES as u32 + 1).to_le_bytes())
			.await
			.unwrap();
		assert_eq!(
			read_frame(&mut server).await.unwrap_err().kind(),
			io::ErrorKind::InvalidData
		);
		let worker = tokio::spawn(async move {
			for byte in [1, 0, 0, 0, 2, 0, 0, 0, b'{', b'}'] {
				client.write_all(&[byte]).await.unwrap();
				tokio::task::yield_now().await;
			}
		});
		assert_eq!(read_frame(&mut server).await.unwrap(), (1, b"{}".to_vec()));
		worker.await.unwrap();
	}
	#[test]
	fn latest_game_falls_back_and_clears_without_polling() {
		let (activity, _) = watch::channel(None);
		let (report, _) = watch::channel(Ok(None));
		let mut values = std::array::from_fn(|_| None);
		let first = rpc::ActivityFields::default()
			.into_activity(model::Id(7), "First game".into())
			.unwrap();
		let second = rpc::ActivityFields::default()
			.into_activity(model::Id(8), "Second game".into())
			.unwrap();
		values[0] = Some((Instant::now(), first.clone()));
		values[1] = Some((Instant::now() + Duration::from_millis(1), second.clone()));
		publish(&values, &activity, &report, &egui::Context::default());
		assert_eq!(*activity.borrow(), Some(second));
		values[1] = None;
		publish(&values, &activity, &report, &egui::Context::default());
		assert_eq!(*activity.borrow(), Some(first));
		let first = values[0].as_mut().unwrap();
		first.1.details = Some("  Next beatmap  ".into());
		first.1.state = Some(" ".into());
		first.1.assets = Some(rpc::Assets {
			large_image: Some("99".into()),
			small_image: Some("98".into()),
			..Default::default()
		});
		first.1.timestamps = Some(rpc::Timestamps {
			start: Some(1_700_000_000_000),
			end: None,
		});
		publish(&values, &activity, &report, &egui::Context::default());
		let display = report.borrow().as_ref().unwrap().clone().unwrap();
		assert!(display.valid());
		assert_eq!(display.summary(), "Playing First game");
		assert_eq!(display.details.as_deref(), Some("Next beatmap"));
		assert!(display.state.is_none());
		assert_eq!(
			display.image,
			Some(model::ActivityImage::Asset {
				application: model::Id(7),
				asset: model::Id(99),
			})
		);
		assert_eq!(
			display.small_image,
			Some(model::ActivityImage::Asset {
				application: model::Id(7),
				asset: model::Id(98),
			})
		);
		assert_eq!(display.started_at, Some(1_700_000_000_000));
		let first = &mut values[0].as_mut().unwrap().1;
		first.assets.as_mut().unwrap().small_image = None;
		assert_eq!(
			display_activity(first).small_image,
			Some(model::ActivityImage::Application(model::Id(7)))
		);
		first.assets.as_mut().unwrap().large_image = None;
		first.assets.as_mut().unwrap().small_image = Some("98".into());
		assert!(display_activity(first).small_image.is_none());
		values[0] = None;
		publish(&values, &activity, &report, &egui::Context::default());
		assert!(activity.borrow().is_none());
		assert_eq!(*report.borrow(), Ok(None));
	}
	#[tokio::test]
	async fn disable_and_sender_close_cancel_the_session_and_clear_latest() {
		use std::sync::{
			Arc,
			atomic::{AtomicUsize, Ordering},
		};
		struct Running(Arc<AtomicUsize>);
		impl Drop for Running {
			fn drop(&mut self) {
				self.0.fetch_sub(1, Ordering::SeqCst);
			}
		}
		let (enabled, receiver) = watch::channel(false);
		let (activity, mut changes) = watch::channel(None);
		let (report, _) = watch::channel(Ok(None));
		let running = Arc::new(AtomicUsize::new(0));
		let count = running.clone();
		let worker = tokio::spawn(async move {
			run_enabled(
				receiver,
				&activity,
				&report,
				&egui::Context::default(),
				|| async {
					count.fetch_add(1, Ordering::SeqCst);
					let _running = Running(count.clone());
					activity.send_replace(Some(
						rpc::ActivityFields::default()
							.into_activity(model::Id(7), "Synthetic game".into())
							.unwrap(),
					));
					std::future::pending().await
				},
			)
			.await;
		});
		tokio::task::yield_now().await;
		assert_eq!(running.load(Ordering::SeqCst), 0);
		for stop_with_drop in [false, true] {
			enabled.send(true).unwrap();
			timeout(Duration::from_secs(2), async {
				loop {
					changes.changed().await.unwrap();
					if changes.borrow_and_update().is_some() {
						break;
					}
				}
			})
			.await
			.unwrap();
			assert_eq!(running.load(Ordering::SeqCst), 1);
			if stop_with_drop {
				break;
			}
			enabled.send(false).unwrap();
			timeout(Duration::from_secs(2), async {
				loop {
					changes.changed().await.unwrap();
					if changes.borrow_and_update().is_none() {
						break;
					}
				}
			})
			.await
			.unwrap();
			assert_eq!(running.load(Ordering::SeqCst), 0);
		}
		drop(enabled);
		timeout(Duration::from_secs(2), worker)
			.await
			.unwrap()
			.unwrap();
		assert!(changes.borrow().is_none());
		assert_eq!(running.load(Ordering::SeqCst), 0);
	}

	#[test]
	fn choice_survives_late_load_and_failed_load_never_enables_sharing() {
		let mut settings = crate::toggle_setting::Settings::default();
		settings.restore(Err(local_store::StoreError::Unavailable));
		assert!(!settings.enabled);
		assert!(settings.failed);
		settings.observe(true);
		settings.restore(Ok(false));
		assert!(settings.enabled && settings.dirty && !settings.failed);
		settings.saving = true;
		settings.dirty = false;
		settings.observe(false);
		assert!(settings.dirty && settings.saving && !settings.enabled);
	}
}
