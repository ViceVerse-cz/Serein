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
}

impl Gateway {
	async fn new() -> Self {
		Self {
			initial: TcpListener::bind("127.0.0.1:0").await.unwrap(),
			resume: TcpListener::bind("127.0.0.1:0").await.unwrap(),
			reconnect: Notify::new(),
			finished: Notify::new(),
			manual: AtomicBool::new(false),
		}
	}

	async fn run(&self, server: impl std::future::Future<Output = ()>) {
		timeout(Duration::from_secs(10), async {
			let initial = format!("ws://{}/", self.initial.local_addr().unwrap());
			let resume = format!("ws://{}/", self.resume.local_addr().unwrap());
			let client = async {
				let result = run_recoverable(
					Arc::new(
						SessionSecret::from_owner_input("synthetic-recovery-secret".into())
							.unwrap(),
					),
					"wss://gateway.discord.gg/".into(),
					watch::channel(None).1,
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
