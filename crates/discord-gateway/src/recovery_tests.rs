//! Offline socket regressions for bounded recovery of an established session.
use super::{tests::*, *};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
	WebSocketStream, accept_async,
	tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
};

struct Gateway {
	initial: TcpListener,
	resume: TcpListener,
	reconnect: Notify,
	finished: Notify,
	manual: AtomicBool,
	subscription: Option<MemberSubscription>,
}

impl Gateway {
	async fn new() -> Self {
		Self {
			initial: TcpListener::bind("127.0.0.1:0").await.unwrap(),
			resume: TcpListener::bind("127.0.0.1:0").await.unwrap(),
			reconnect: Notify::new(),
			finished: Notify::new(),
			manual: AtomicBool::new(false),
			subscription: None,
		}
	}

	async fn run(&self, server: impl std::future::Future<Output = ()>) {
		self.run_with_timeout(Duration::from_secs(10), server).await;
	}

	async fn run_with_timeout(
		&self,
		limit: Duration,
		server: impl std::future::Future<Output = ()>,
	) {
		timeout(limit, async {
			let initial = format!("ws://{}/", self.initial.local_addr().unwrap());
			let resume = format!("ws://{}/", self.resume.local_addr().unwrap());
			let client = async {
				let result = run_recoverable(
					Arc::new(
						SessionSecret::from_owner_input("synthetic-recovery-secret".into())
							.unwrap(),
					),
					"wss://gateway.discord.gg/".into(),
					watch::channel(self.subscription.clone()).1,
					mpsc::channel(1).1,
					None,
					Some(&self.reconnect),
					|event| {
						if matches!(event, Event::Disconnected)
							&& !self.manual.load(Ordering::Relaxed)
						{
							self.reconnect.notify_one();
						}
						Ok(())
					},
					Some((&initial, &resume)),
				)
				.await;
				self.finished.notify_one();
				result
			};
			let ((), result) = tokio::join!(server, client);
			assert_eq!(result, Err(Failure::Expired));
		})
		.await
		.expect("synthetic gateway recovery exceeded its bounded deadline");
	}

	async fn socket(&self, resume: bool, op: Option<u64>) -> WebSocketStream<TcpStream> {
		let (stream, resumed) = tokio::select! {
			result = self.initial.accept() => (result.unwrap().0, false),
			result = self.resume.accept() => (result.unwrap().0, true),
		};
		assert_eq!(resumed, resume, "recovery selected the wrong gateway URL");
		stream.set_nodelay(true).unwrap();
		let mut socket = accept_async(stream).await.unwrap();
		if let Some(op) = op {
			send(
				&mut socket,
				json!({"op":10,"d":{"heartbeat_interval":30000}}),
			)
			.await;
			let handshake = packet(&mut socket).await;
			assert_eq!(handshake["op"], op);
			if op == 6 {
				assert_eq!(handshake["d"]["session_id"], "synthetic-recovery-session");
			} else {
				send(&mut socket, json!({"op":1,"d":null})).await;
				let heartbeat = packet(&mut socket).await;
				assert_eq!(heartbeat["op"], 1);
				assert!(
					heartbeat["d"].is_null(),
					"Identify must clear the resume sequence"
				);
				send(&mut socket, json!({"op":11,"d":null})).await;
			}
		}
		socket
	}

	async fn establish(&self) {
		let mut socket = self.socket(false, Some(2)).await;
		let mut ready = ready(41, "synthetic-recovery-session");
		ready["d"]["resume_gateway_url"] = json!("wss://gateway-resume.discord.gg/");
		send(&mut socket, ready).await;
		acknowledge(&mut socket, 41).await;
	}

	async fn stop(&self, mut socket: WebSocketStream<TcpStream>) {
		close(&mut socket, 4004).await;
		tokio::select! {
			() = self.finished.notified() => {},
			_ = self.initial.accept() => panic!("terminal close must not reconnect"),
			_ = self.resume.accept() => panic!("terminal close must not resume"),
		}
	}
}

async fn close(socket: &mut WebSocketStream<TcpStream>, code: u16) {
	socket
		.send(Frame::Close(Some(CloseFrame {
			code: CloseCode::from(code),
			reason: "synthetic close".into(),
		})))
		.await
		.unwrap();
}

#[tokio::test]
async fn partial_delivery_waits_for_capacity_then_identifies_and_resynchronizes() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
	let blocked = AtomicBool::new(false);
	let partial = AtomicBool::new(false);
	let delivered = AtomicBool::new(false);
	let diagnostics = Arc::new(Mutex::new(ConnectionDiagnostics::default()));
	let state = Mutex::new(client_core::State::default());
	let client = run_observed(
		Arc::new(SessionSecret::from_owner_input("synthetic-congestion-session".into()).unwrap()),
		"wss://gateway.discord.gg/".into(),
		watch::channel(None).1,
		mpsc::channel(1).1,
		None,
		None,
		None,
		diagnostics.clone(),
		|event, _| {
			if blocked.load(Ordering::Acquire) {
				return Err(Failure::Busy);
			}
			if matches!(
				&event,
				Event::Permissions(client_core::permissions::Event::Channel { .. })
			) && !partial.swap(true, Ordering::AcqRel)
			{
				blocked.store(true, Ordering::Release);
				return Err(Failure::Busy);
			}
			if matches!(
				&event,
				Event::UserAction(client_core::user_actions::Event::NoteChanged { .. })
			) {
				delivered.store(true, Ordering::Release);
			}
			let mut state = state.lock().unwrap();
			let generation = state.generation;
			state.apply(client_core::Envelope { generation, event });
			Ok(())
		},
		Some((&endpoint, &endpoint)),
	);
	let server = async {
		let (stream, _) = listener.accept().await.unwrap();
		stream.set_nodelay(true).unwrap();
		let mut socket = accept_async(stream).await.unwrap();
		send(
			&mut socket,
			json!({"op":10,"d":{"heartbeat_interval":120000}}),
		)
		.await;
		assert_eq!(packet(&mut socket).await["op"], 2);
		send(&mut socket, ready(1, "synthetic-before-congestion")).await;
		send(&mut socket,json!({"op":0,"t":"CHANNEL_CREATE","s":2,"d":{"id":"3","guild_id":"2","type":0,"name":"Synthetic partial","permission_overwrites":[]}})).await;
		for _ in 0..1000 {
			if blocked.load(Ordering::Acquire) {
				break;
			}
			tokio::task::yield_now().await;
		}
		assert!(partial.load(Ordering::Acquire));
		assert!(state.lock().unwrap().channels.iter().any(|c| c.id == Id(3)));
		assert!(
			timeout(Duration::from_millis(300), listener.accept())
				.await
				.is_err(),
			"full delivery queue must not cause a dial loop"
		);
		blocked.store(false, Ordering::Release);
		let (stream, _) = listener.accept().await.unwrap();
		stream.set_nodelay(true).unwrap();
		let mut socket = accept_async(stream).await.unwrap();
		send(
			&mut socket,
			json!({"op":10,"d":{"heartbeat_interval":120000}}),
		)
		.await;
		assert_eq!(
			packet(&mut socket).await["op"],
			2,
			"partially delivered dispatch requires fresh Identify"
		);
		send(&mut socket, ready(1, "synthetic-after-congestion")).await;
		send(
			&mut socket,
			json!({"op":0,"t":"USER_NOTE_UPDATE","s":2,"d":{"id":"9","note":"Synthetic recovered"}}),
		)
		.await;
		while !delivered.load(Ordering::Acquire) {
			tokio::task::yield_now().await;
		}
		assert!(!state.lock().unwrap().channels.iter().any(|c| c.id == Id(3)));
		close(&mut socket, 4004).await;
		for _ in 0..100 {
			tokio::task::yield_now().await;
		}
	};
	let (result, ()) = timeout(Duration::from_secs(5), async {
		tokio::join!(client, server)
	})
	.await
	.unwrap();
	assert_eq!(result, Err(Failure::Expired));
	assert!(
		diagnostics
			.lock()
			.unwrap()
			.summary()
			.contains(ConnectionReason::DeliveryCongestion.label())
	);
}

#[tokio::test]
async fn coalesced_delivery_loss_still_retires_the_socket_generation() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
	let (delivery, observed) = watch::channel(DeliveryState::default());
	let resynchronized = AtomicBool::new(false);
	let delivered = AtomicBool::new(false);
	let client = run_observed(
		Arc::new(SessionSecret::from_owner_input("synthetic-delivery-generation".into()).unwrap()),
		"wss://gateway.discord.gg/".into(),
		watch::channel(None).1,
		mpsc::channel(1).1,
		None,
		None,
		Some(observed),
		Arc::new(Mutex::new(ConnectionDiagnostics::default())),
		|event, epoch| {
			if matches!(&event, Event::Resync) {
				assert_eq!(epoch, 1);
				resynchronized.store(true, Ordering::Release);
			}
			if matches!(
				&event,
				Event::UserAction(client_core::user_actions::Event::NoteChanged { .. })
			) {
				assert_eq!(epoch, 1);
				assert!(resynchronized.load(Ordering::Acquire));
				delivered.store(true, Ordering::Release);
			}
			Ok(())
		},
		Some((&endpoint, &endpoint)),
	);
	let server = async {
		let (stream, _) = listener.accept().await.unwrap();
		let mut socket = accept_async(stream).await.unwrap();
		send(
			&mut socket,
			json!({"op":10,"d":{"heartbeat_interval":120000}}),
		)
		.await;
		assert_eq!(packet(&mut socket).await["op"], 2);
		send(&mut socket, ready(1, "synthetic-old-generation")).await;
		acknowledge(&mut socket, 1).await;
		// No yield between these changes: a boolean watch would lose the failure.
		delivery.send_replace(DeliveryState {
			blocked: true,
			recovering: true,
			generation: 1,
		});
		delivery.send_replace(DeliveryState {
			blocked: false,
			recovering: true,
			generation: 1,
		});
		let (stream, _) = listener.accept().await.unwrap();
		let mut fresh = accept_async(stream).await.unwrap();
		send(
			&mut fresh,
			json!({"op":10,"d":{"heartbeat_interval":120000}}),
		)
		.await;
		assert_eq!(
			packet(&mut fresh).await["op"],
			2,
			"lost delivery requires fresh synchronization"
		);
		send(&mut fresh, ready(1, "synthetic-new-generation")).await;
		send(
			&mut fresh,
			json!({"op":0,"t":"USER_NOTE_UPDATE","s":2,"d":{"id":"9","note":"Synthetic current"}}),
		)
		.await;
		while !delivered.load(Ordering::Acquire) {
			tokio::task::yield_now().await;
		}
		close(&mut fresh, 4004).await;
	};
	let (result, ()) = timeout(Duration::from_secs(5), async {
		tokio::join!(client, server)
	})
	.await
	.unwrap();
	assert_eq!(result, Err(Failure::Expired));
}

#[tokio::test]
async fn three_failed_resumes_identify_at_original_gateway() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			gateway.establish().await;
			drop(gateway.socket(true, None).await);
			for _ in 0..2 {
				drop(gateway.socket(true, Some(6)).await);
			}
			gateway.stop(gateway.socket(false, Some(2)).await).await;
		})
		.await;
}

#[tokio::test]
async fn resumed_session_resets_consecutive_failure_budget() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			gateway.establish().await;
			for _ in 0..2 {
				drop(gateway.socket(true, Some(6)).await);
			}
			let mut socket = gateway.socket(true, Some(6)).await;
			send(&mut socket, json!({"op":0,"t":"RESUMED","s":42,"d":{}})).await;
			acknowledge(&mut socket, 42).await;
			drop(socket);
			for _ in 0..3 {
				drop(gateway.socket(true, Some(6)).await);
			}
			gateway.stop(gateway.socket(false, Some(2)).await).await;
		})
		.await;
}

#[tokio::test]
async fn repeated_resumable_invalid_sessions_fall_back_to_identify() {
	let gateway = Gateway::new().await;
	gateway
		.run_with_timeout(Duration::from_secs(20), async {
			gateway.establish().await;
			for _ in 0..3 {
				let mut socket = gateway.socket(true, Some(6)).await;
				send(&mut socket, json!({"op":9,"d":true})).await;
				// Keep TCP open through the invalid-session grace period. Closing it
				// ourselves could lose the opcode in a race with an unread heartbeat.
				while let Some(Ok(_)) = socket.next().await {}
			}
			gateway.stop(gateway.socket(false, Some(2)).await).await;
		})
		.await;
}

#[tokio::test]
async fn invalid_passive_permissions_force_a_fresh_snapshot() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			let mut socket = gateway.socket(false, Some(2)).await;
			let mut ready = ready(41, "synthetic-recovery-session");
			ready["d"]["resume_gateway_url"] = json!("wss://gateway-resume.discord.gg/");
			send(&mut socket, ready.clone()).await;
			acknowledge(&mut socket, 41).await;
			send(&mut socket, json!({"op":0,"t":"PASSIVE_UPDATE_V2","s":42,"d":{
				"guild_id":"2","updated_members":[{"user":{"id":"1","username":"Synthetic"},"roles":["11","11"]}]
			}})).await;
			let mut fresh = gateway.socket(false, Some(2)).await;
			ready["s"] = json!(1);
			send(&mut fresh, ready).await;
			acknowledge(&mut fresh, 1).await;
			gateway.stop(fresh).await;
		})
		.await;
}

#[tokio::test]
async fn manual_recovery_does_not_exhaust_resume_budget() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			gateway.establish().await;
			let mut socket = gateway.socket(true, None).await;
			gateway.manual.store(true, Ordering::Relaxed);
			for op in [Some(6), None, Some(6), Some(6)] {
				gateway.reconnect.notify_one();
				let next = gateway.socket(true, op).await;
				drop(socket);
				socket = next;
			}
			gateway.stop(socket).await;
		})
		.await;
}

#[tokio::test]
async fn expired_session_before_hello_stops_recovery() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			gateway.establish().await;
			gateway.stop(gateway.socket(true, None).await).await;
		})
		.await;
}

#[tokio::test]
async fn invalid_session_before_hello_identifies_at_original_gateway() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			gateway.establish().await;
			let mut socket = gateway.socket(true, None).await;
			close(&mut socket, 4009).await;
			let next = gateway.socket(false, Some(2)).await;
			drop(socket);
			gateway.stop(next).await;
		})
		.await;
}

#[tokio::test]
async fn recovery_probes_a_ready_socket_and_resumes_only_when_it_is_silent() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			let mut socket = gateway.socket(false, Some(2)).await;
			let mut ready = ready(41, "synthetic-recovery-session");
			ready["d"]["resume_gateway_url"] = json!("wss://gateway-resume.discord.gg/");
			send(&mut socket, ready).await;
			acknowledge(&mut socket, 41).await;
			gateway.manual.store(true, Ordering::Relaxed);
			// Timer/requested heartbeats and recovery probes have the same wire payload.
			// Deliberately race a requested heartbeat with the probe and acknowledge both.
			send(&mut socket, json!({"op":1,"d":null})).await;
			gateway.reconnect.notify_one();
			let healthy_until = Instant::now() + PROBE_TIMEOUT * 2;
			let mut heartbeats = 0;
			loop {
				tokio::select! {
					_ = tokio::time::sleep_until(healthy_until) => break,
					packet = packet(&mut socket) => {
						assert_eq!((packet["op"].clone(), packet["d"].clone()), (json!(1), json!(41)));
						send(&mut socket, json!({"op":11,"d":null})).await;
						heartbeats += 1;
					}
					_ = gateway.initial.accept() => panic!("healthy probe must not reconnect"),
					_ = gateway.resume.accept() => panic!("healthy probe must not resume"),
				}
			}
			assert!(
				heartbeats >= 2,
				"requested and probe heartbeats must both arrive"
			);
			// The ordered Pong proves the client consumed every preceding ACK. Otherwise a
			// delayed ACK can clear the silent probe and leave this test waiting for a resume.
			let barrier = b"recovery-ack-barrier";
			socket
				.send(Frame::Ping(barrier.as_slice().into()))
				.await
				.unwrap();
			loop {
				match socket.next().await.unwrap().unwrap() {
					Frame::Pong(data) => {
						assert_eq!(data.as_ref(), barrier);
						break;
					}
					Frame::Text(text) => {
						let packet: serde_json::Value = serde_json::from_str(&text).unwrap();
						assert_eq!(packet["op"], 1);
					}
					other => panic!("unexpected frame at recovery ACK barrier: {other:?}"),
				}
			}
			let silent_since = Instant::now();
			gateway.reconnect.notify_one();
			let probe = packet(&mut socket).await;
			assert_eq!(
				(probe["op"].clone(), probe["d"].clone()),
				(json!(1), json!(41))
			);
			let next = gateway.socket(true, Some(6)).await;
			assert!(silent_since.elapsed() >= PROBE_TIMEOUT);
			drop(socket);
			gateway.stop(next).await;
		})
		.await;
}

#[tokio::test]
async fn server_requested_reconnect_resumes_without_the_failure_backoff() {
	let gateway = Gateway::new().await;
	gateway.manual.store(true, Ordering::Relaxed);
	gateway
		.run(async {
			let mut socket = gateway.socket(false, Some(2)).await;
			let mut ready = ready(41, "synthetic-recovery-session");
			ready["d"]["resume_gateway_url"] = json!("wss://gateway-resume.discord.gg/");
			send(&mut socket, ready).await;
			acknowledge(&mut socket, 41).await;
			let started = Instant::now();
			send(&mut socket, json!({"op":7,"d":null})).await;
			let next = timeout(Duration::from_secs(1), gateway.socket(true, Some(6)))
				.await
				.expect("server migration must not wait for failure backoff");
			assert!(started.elapsed() < Duration::from_secs(1));
			gateway.stop(next).await;
		})
		.await;
}

/// Poll real loopback I/O while keeping the paused runtime runnable. Advancing the
/// clock remains explicit: ordinary packet timeouts cannot advance the test for us.
async fn buffered_packets(socket: &mut WebSocketStream<TcpStream>) -> Vec<serde_json::Value> {
	let mut packets = Vec::new();
	for _ in 0..400 {
		tokio::task::yield_now().await;
		if let Some(frame) = socket.next().now_or_never() {
			match frame.expect("synthetic socket closed").unwrap() {
				Frame::Text(text) => {
					let value: serde_json::Value = serde_json::from_str(&text).unwrap();
					if value["op"] == 1 {
						send(socket, json!({"op":11,"d":null})).await;
					} else {
						packets.push(value);
					}
				}
				Frame::Pong(_) => {}
				other => panic!("unexpected synthetic frame: {other:?}"),
			}
		}
		assert!(packets.len() < 32);
	}
	packets
}

#[tokio::test]
async fn member_subscription_backoff_is_preserved_by_the_production_send_loop() {
	let initial = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let endpoint = format!("ws://{}/", initial.local_addr().unwrap());
	let (_subscriptions, subscriptions) = watch::channel(Some(MemberSubscription {
		thread: false,
		guild: Id(2),
		channel: Id(3),
		request: 1,
		list_id: "everyone".into(),
		ranges: vec![[0, 99]],
	}));
	let client = run_recoverable(
		Arc::new(SessionSecret::from_owner_input("synthetic-retry-secret".into()).unwrap()),
		"wss://gateway.discord.gg/".into(),
		subscriptions,
		mpsc::channel(1).1,
		None,
		None,
		|_| Ok(()),
		Some((&endpoint, &endpoint)),
	);
	let server = async {
		let (stream, _) = initial.accept().await.unwrap();
		stream.set_nodelay(true).unwrap();
		let mut socket = accept_async(stream).await.unwrap();
		send(
			&mut socket,
			json!({"op":10,"d":{"heartbeat_interval":120000}}),
		)
		.await;
		assert_eq!(packet(&mut socket).await["op"], 2);
		send(&mut socket, ready(1, "synthetic-retry-session")).await;
		let mut initial_subscriptions = 0;
		while initial_subscriptions < 2 {
			let value = packet(&mut socket).await;
			if value["op"] == 1 {
				send(&mut socket, json!({"op":11,"d":null})).await;
			} else {
				assert_eq!(value["op"], 37);
				initial_subscriptions += 1;
			}
		}
		for _ in 0..100 {
			tokio::task::yield_now().await;
		}
		tokio::time::pause();
		for delay in [15, 30, 60, 120] {
			tokio::time::advance(Duration::from_secs(delay - 1)).await;
			assert!(
				buffered_packets(&mut socket).await.is_empty(),
				"member retry fired before its increasing deadline"
			);
			tokio::time::advance(Duration::from_secs(1)).await;
			// Tokio rounds timer-wheel deadlines up to a millisecond. Cross that
			// resolution explicitly without changing the intended retry intervals.
			tokio::time::advance(Duration::from_millis(1)).await;
			let retry = buffered_packets(&mut socket).await;
			assert_eq!(
				retry.len(),
				2,
				"each retry must reset then resubscribe, once"
			);
			assert!(retry.iter().all(|value| value["op"] == 37));
			assert_eq!(
				retry[1]["d"]["subscriptions"]["2"]["channels"],
				json!({"3":[[0,99]]})
			);
		}
		tokio::time::resume();
		close(&mut socket, 4004).await;
		// Retain TCP until the client reads Close, avoiding an unread-heartbeat reset race.
		for _ in 0..100 {
			tokio::task::yield_now().await;
		}
	};
	let (result, ()) = tokio::join!(client, server);
	assert_eq!(result, Err(Failure::Expired));
}

#[tokio::test]
async fn buffered_ack_survives_a_scheduler_pause_without_replacing_the_socket() {
	let initial = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let endpoint = format!("ws://{}/", initial.local_addr().unwrap());
	let disconnected = std::sync::atomic::AtomicUsize::new(0);
	let client = run_recoverable(
		Arc::new(SessionSecret::from_owner_input("synthetic-paused-heartbeat".into()).unwrap()),
		"wss://gateway.discord.gg/".into(),
		watch::channel(None).1,
		mpsc::channel(1).1,
		None,
		None,
		|event| {
			if matches!(event, Event::Disconnected) {
				disconnected.fetch_add(1, Ordering::Relaxed);
			}
			Ok(())
		},
		Some((&endpoint, &endpoint)),
	);
	let server = async {
		let (stream, _) = initial.accept().await.unwrap();
		stream.set_nodelay(true).unwrap();
		let mut socket = accept_async(stream).await.unwrap();
		send(
			&mut socket,
			json!({"op":10,"d":{"heartbeat_interval":1000}}),
		)
		.await;
		assert_eq!(packet(&mut socket).await["op"], 2);
		send(&mut socket, ready(1, "synthetic-paused-session")).await;
		let heartbeat = packet(&mut socket).await;
		assert_eq!(heartbeat["op"], 1);
		for _ in 0..20 {
			tokio::task::yield_now().await;
		}
		tokio::time::pause();
		for burst in 0..20 {
			// Real data already queued ahead of an ACK must be drained in bounded
			// batches after the scheduler resumes, without declaring a false loss.
			for offset in 0..128 {
				send(
					&mut socket,
					json!({"op":0,"t":"SYNTHETIC_UNSUPPORTED","s":2+burst*128+offset,"d":{}}),
				)
				.await;
			}
			// Queue the ACK before resuming a timer which is already overdue.
			send(&mut socket, json!({"op":11,"d":null})).await;
			tokio::time::advance(Duration::from_secs(2)).await;
			assert!(buffered_packets(&mut socket).await.is_empty());
			assert_eq!(
				disconnected.load(Ordering::Relaxed),
				0,
				"buffered ACK lost to expired timer"
			);
		}
		tokio::time::resume();
		close(&mut socket, 4004).await;
		for _ in 0..100 {
			tokio::task::yield_now().await;
		}
	};
	let (result, ()) = tokio::join!(client, server);
	assert_eq!(result, Err(Failure::Expired));
}

#[tokio::test]
async fn gateway_rate_limit_reports_disconnect_immediately_and_manual_recovery_cannot_skip_cooldown()
 {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
	let disconnected = AtomicBool::new(false);
	let reconnect = Notify::new();
	let diagnostics = Arc::new(Mutex::new(ConnectionDiagnostics::default()));
	let client = run_observed(
		Arc::new(SessionSecret::from_owner_input("synthetic-rate-secret".into()).unwrap()),
		"wss://gateway.discord.gg/".into(),
		watch::channel(None).1,
		mpsc::channel(1).1,
		None,
		Some(&reconnect),
		None,
		diagnostics.clone(),
		|event, _| {
			if matches!(event, Event::Disconnected) {
				disconnected.store(true, Ordering::Relaxed);
			}
			Ok(())
		},
		Some((&endpoint, &endpoint)),
	);
	let server = async {
		let (stream, _) = listener.accept().await.unwrap();
		stream.set_nodelay(true).unwrap();
		let mut first = accept_async(stream).await.unwrap();
		send(
			&mut first,
			json!({"op":10,"d":{"heartbeat_interval":120000}}),
		)
		.await;
		assert_eq!(packet(&mut first).await["op"], 2);
		send(&mut first, ready(1, "synthetic-rate-session")).await;
		acknowledge(&mut first, 1).await;
		close(&mut first, 4008).await;
		for _ in 0..1000 {
			if disconnected.load(Ordering::Relaxed) {
				break;
			}
			tokio::task::yield_now().await;
		}
		assert!(
			disconnected.load(Ordering::Relaxed),
			"cooldown must not leave the UI claiming connected"
		);
		assert_eq!(
			diagnostics.lock().unwrap().last_reason(),
			Some(ConnectionReason::RateLimited.label())
		);
		tokio::time::pause();
		for _ in 0..20 {
			reconnect.notify_one();
		}
		tokio::time::advance(Duration::from_secs(59)).await;
		for _ in 0..100 {
			tokio::task::yield_now().await;
		}
		assert!(
			listener.accept().now_or_never().is_none(),
			"manual recovery bypassed the Gateway cooldown"
		);
		tokio::time::advance(Duration::from_secs(2)).await;
		tokio::time::resume();
		let (stream, _) = timeout(Duration::from_secs(2), listener.accept())
			.await
			.unwrap()
			.unwrap();
		stream.set_nodelay(true).unwrap();
		let mut next = accept_async(stream).await.unwrap();
		send(
			&mut next,
			json!({"op":10,"d":{"heartbeat_interval":120000}}),
		)
		.await;
		assert_eq!(packet(&mut next).await["op"], 6);
		close(&mut next, 4004).await;
		for _ in 0..100 {
			tokio::task::yield_now().await;
		}
	};
	let (result, ()) = tokio::join!(client, server);
	assert_eq!(result, Err(Failure::Expired));
	let history = diagnostics.lock().unwrap().summary();
	assert!(history.contains("code=4008"));
	assert!(!history.contains("synthetic"));
}

#[tokio::test]
async fn protocol_errors_from_dispatch_are_retained_by_the_diagnostic_boundary() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
	let diagnostics = Arc::new(Mutex::new(ConnectionDiagnostics::default()));
	let client = run_observed(
		Arc::new(SessionSecret::from_owner_input("synthetic-diagnostic-secret".into()).unwrap()),
		"wss://gateway.discord.gg/".into(),
		watch::channel(None).1,
		mpsc::channel(1).1,
		None,
		None,
		None,
		diagnostics.clone(),
		|_, _| Ok(()),
		Some((&endpoint, &endpoint)),
	);
	let server = async {
		let (stream, _) = listener.accept().await.unwrap();
		stream.set_nodelay(true).unwrap();
		let mut socket = accept_async(stream).await.unwrap();
		send(
			&mut socket,
			json!({"op":10,"d":{"heartbeat_interval":120000}}),
		)
		.await;
		assert_eq!(packet(&mut socket).await["op"], 2);
		send(&mut socket, ready(1, "synthetic-sensitive-session")).await;
		acknowledge(&mut socket, 1).await;
		send(
			&mut socket,
			json!({"op":99,"d":{"private":"SYNTHETIC_PRIVATE_PAYLOAD"}}),
		)
		.await;
		for _ in 0..100 {
			tokio::task::yield_now().await;
		}
	};
	let (result, ()) = tokio::join!(client, server);
	assert_eq!(result, Err(Failure::Protocol));
	let history = diagnostics.lock().unwrap();
	assert_eq!(
		history.last_reason(),
		Some(ConnectionReason::Protocol.label())
	);
	assert!(!history.summary().contains("SYNTHETIC"));
	assert!(!history.summary().contains("synthetic"));
}

#[tokio::test]
async fn split_reader_sends_one_automatic_pong_and_keeps_requested_heartbeats() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			let mut socket = gateway.socket(false, Some(2)).await;
			send(&mut socket, ready(41, "synthetic-recovery-session")).await;
			acknowledge(&mut socket, 41).await;
			socket
				.send(Frame::Ping(b"synthetic-pong-barrier".as_slice().into()))
				.await
				.unwrap();
			send(&mut socket, json!({"op":1,"d":null})).await;
			let mut pongs = 0;
			let mut heartbeat = false;
			while pongs == 0 || !heartbeat {
				match timeout(Duration::from_secs(1), socket.next())
					.await
					.unwrap()
					.unwrap()
					.unwrap()
				{
					Frame::Pong(bytes) => {
						assert_eq!(bytes.as_ref(), b"synthetic-pong-barrier");
						pongs += 1;
						assert_eq!(pongs, 1);
					}
					Frame::Text(text) => {
						let value: serde_json::Value = serde_json::from_str(&text).unwrap();
						assert_eq!(value["op"], 1);
						send(&mut socket, json!({"op":11,"d":null})).await;
						heartbeat = true;
					}
					other => panic!("unexpected synthetic frame: {other:?}"),
				}
			}
			send(&mut socket, json!({"op":1,"d":null})).await;
			assert_eq!(
				packet(&mut socket).await["op"],
				1,
				"a duplicate Pong must not precede the next heartbeat"
			);
			send(&mut socket, json!({"op":11,"d":null})).await;
			gateway.stop(socket).await;
		})
		.await;
}

#[tokio::test]
async fn older_dispatch_cannot_rewind_cursor_or_replace_a_newer_note() {
	let gateway = Gateway::new().await;
	gateway
		.run(async {
			let mut socket = gateway.socket(false, Some(2)).await;
			send(&mut socket, ready(40, "synthetic-recovery-session")).await;
			for (sequence, note) in [(42, "new"), (41, "old"), (42, "duplicate")] {
				send(
					&mut socket,
					json!({"op":0,"t":"USER_NOTE_UPDATE","s":sequence,"d":{"id":"9","note":note}}),
				)
				.await;
			}
			send(&mut socket, json!({"op":1,"d":null})).await;
			loop {
				let value = packet(&mut socket).await;
				if value["op"] == 1 {
					assert_eq!(value["d"], 42);
					send(&mut socket, json!({"op":11,"d":null})).await;
					break;
				}
			}
			gateway.stop(socket).await;
		})
		.await;
}
#[test]
fn delayed_ack_keeps_a_later_probe_outstanding() {
	let now = Instant::now();
	let mut heartbeat = Heartbeat::default();
	heartbeat
		.pending
		.extend([(1, now), (2, now + Duration::from_secs(1))]);
	assert_eq!(heartbeat.ack(), Some(1));
	assert_eq!(
		heartbeat.probe_deadline(Some(2)),
		Some(now + Duration::from_secs(1) + PROBE_TIMEOUT)
	);
	assert_eq!(heartbeat.ack(), Some(2));
	assert!(heartbeat.pending.is_empty());
}
#[tokio::test]
async fn resume_preserves_an_active_subscription_cooldown() {
	let mut gateway = Gateway::new().await;
	gateway.subscription = Some(MemberSubscription {
		thread: false,
		guild: Id(2),
		channel: Id(3),
		request: 1,
		list_id: "everyone".into(),
		ranges: vec![[0, 99]],
	});
	gateway.run(async {
		let mut socket=gateway.socket(false,Some(2)).await;
		let mut ready=ready(41,"synthetic-recovery-session");
		ready["d"]["resume_gateway_url"]=json!("wss://gateway-resume.discord.gg/");
		send(&mut socket,ready).await;
		let mut subscriptions=0;
		while subscriptions<2 {
			let value=packet(&mut socket).await;
			if value["op"]==37 {subscriptions+=1;}
			if value["op"]==1 {send(&mut socket,json!({"op":11,"d":null})).await;}
		}
		send(&mut socket,json!({"op":0,"t":"RATE_LIMITED","s":42,"d":{"opcode":37,"retry_after":60,"meta":{"guild_id":"2"}}})).await;
		send(&mut socket,json!({"op":7,"d":null})).await;
		let mut socket=gateway.socket(true,Some(6)).await;
		send(&mut socket,json!({"op":0,"t":"RESUMED","s":43,"d":{}})).await;
		assert!(timeout(Duration::from_millis(100),async {
			loop {let value=packet(&mut socket).await;
				if value["op"]==37 {break;}
				if value["op"]==1 {send(&mut socket,json!({"op":11,"d":null})).await;}
			}
		}).await.is_err(),"RESUME sent before the active cooldown ended");
		gateway.stop(socket).await;
	}).await;
}
