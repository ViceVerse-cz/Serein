//! Offline regressions for large, valid login metadata without voice participants.
use super::*;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
	WebSocketStream, accept_async,
	tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
};

async fn send(socket: &mut WebSocketStream<TcpStream>, value: Value) {
	socket
		.send(Frame::Text(value.to_string().into()))
		.await
		.unwrap();
}

async fn packet(socket: &mut WebSocketStream<TcpStream>) -> Value {
	let Frame::Text(text) = socket.next().await.unwrap().unwrap() else {
		panic!("expected a synthetic gateway JSON packet");
	};
	serde_json::from_str(&text).unwrap()
}

async fn login(users: Vec<Value>, guilds: Vec<Value>, supplemental: Option<Value>) {
	login_metadata(users, guilds, supplemental, json!({}), Default::default()).await;
}

async fn login_metadata(
	users: Vec<Value>,
	guilds: Vec<Value>,
	supplemental: Option<Value>,
	metadata: Value,
	warnings: model::account::Warnings,
) {
	timeout(Duration::from_secs(10), async {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
		let ready = AtomicBool::new(false);
		let expected_guilds = guilds.len();
		let expected_channels: usize = guilds
			.iter()
			.map(|g| g["channels"].as_array().map_or(0, Vec::len))
			.sum();
		let state = std::sync::Mutex::new(client_core::State::default());
		let server = async {
			let (stream, _) = listener.accept().await.unwrap();
			let mut socket = accept_async(stream).await.unwrap();
			send(
				&mut socket,
				json!({"op":10,"d":{"heartbeat_interval":1000}}),
			)
			.await;
			assert_eq!(packet(&mut socket).await["op"], 2);
			let mut initial = json!({"op":0,"t":"READY","s":1,"d":{
				"user":{"id":"1","username":"Synthetic owner"},
				"session_id":"synthetic-large-login",
				"resume_gateway_url":"wss://gateway.discord.gg/",
				"users":users,"guilds":guilds,"private_channels":[]
			}});
			initial["d"]
				.as_object_mut()
				.unwrap()
				.extend(metadata.as_object().unwrap().clone());
			send(&mut socket, initial).await;
			let sequence = if let Some(supplemental) = supplemental {
				send(
					&mut socket,
					json!({"op":0,"t":"READY_SUPPLEMENTAL","s":2,"d":supplemental}),
				)
				.await;
				2
			} else {
				1
			};
			send(&mut socket, json!({"op":1,"d":null})).await;
			// A reflected dispatch sequence proves startup continued past metadata processing.
			loop {
				let heartbeat = packet(&mut socket).await;
				assert_eq!(heartbeat["op"], 1);
				send(&mut socket, json!({"op":11,"d":null})).await;
				if heartbeat["d"] == sequence {
					break;
				}
			}
			socket
				.close(Some(CloseFrame {
					code: CloseCode::Library(4004),
					reason: "synthetic stop".into(),
				}))
				.await
				.unwrap();
		};
		let client = run_inner(
			Arc::new(
				SessionSecret::from_owner_input("synthetic-large-login-secret".into()).unwrap(),
			),
			"wss://gateway.discord.gg/".into(),
			watch::channel(None).1,
			mpsc::channel(1).1,
			None,
			|event| {
				let mut state = state.lock().unwrap();
				let startup = matches!(&event, Event::Startup(_));
				if let Event::Startup(snapshot) = &event {
					assert_eq!(snapshot.guilds.len(), expected_guilds);
					assert_eq!(snapshot.channels.len(), expected_channels);
					assert_eq!(snapshot.warnings, warnings);
					ready.store(true, Ordering::Relaxed);
				}
				let generation = state.generation;
				state.apply(client_core::Envelope { generation, event });
				if startup {
					assert_eq!(
						state.auth,
						client_core::auth::AuthState::Authenticated,
						"{}",
						state.status
					);
					assert_eq!(state.channels.len(), expected_channels);
					assert_eq!(state.guilds.len(), expected_guilds);
					assert_eq!(state.startup_warnings, warnings);
					for channel in &state.channels {
						assert!(state.can_read_history(channel.id));
					}
				}
				Ok(())
			},
			Some(&endpoint),
		);
		let ((), result) = tokio::join!(server, client);
		assert_eq!(
			result,
			Err(Failure::Expired),
			"only the synthetic close may terminate login"
		);
		assert!(ready.load(Ordering::Relaxed), "login must emit Ready");
	})
	.await
	.expect("synthetic login exceeded its bounded deadline");
}

fn large_guilds(count: u64) -> Vec<Value> {
	(0..count)
		.map(|index| {
			let guild = 10 + index;
			json!({"id":guild.to_string(),"name":"Synthetic server","owner_id":"1",
			"roles":[{"id":guild.to_string(),"permissions":"1024"}],
			"channels":(0..100).map(|channel| json!({
				"id":(1000+index*100+channel).to_string(), "type":0,
				"name":format!("synthetic-{channel}"),"permission_overwrites":[]
			})).collect::<Vec<_>>()})
		})
		.collect()
}

#[tokio::test]
async fn large_accounts_keep_all_navigation_and_permissions() {
	for count in [70, 96, 200] {
		let start = std::time::Instant::now();
		login(Vec::new(), large_guilds(count), None).await;
		eprintln!(
			"synthetic startup: {count} guilds, {} channels, {:?}",
			count * 100,
			start.elapsed()
		);
	}
}

#[tokio::test]
async fn optional_metadata_faults_do_not_abort_large_login() {
	use model::account::Warnings;
	let entries = (1000..5101)
		.map(|id| json!({"id":id.to_string(),"last_message_id":"0"}))
		.collect::<Vec<_>>();
	login_metadata(
		Vec::new(),
		large_guilds(96),
		None,
		json!({"read_state":{"entries":entries}}),
		Warnings::default(),
	)
	.await;
	let mut guilds = large_guilds(96);
	guilds[0]["emojis"] = json!([{"id":"9","name":null}]);
	login_metadata(
		Vec::new(),
		guilds,
		None,
		json!({
			"read_state":{"entries":[{"id":"invalid"}]},
			"user_guild_settings":{"entries":[{"guild_id":"10","muted":"invalid"}]},
			"sessions":[{"status":false}], "presences":false
		}),
		Warnings {
			stickers: false,
			read_state: true,
			notifications: true,
			sessions: true,
			presence: true,
			emojis: true,
		},
	)
	.await;
}

#[tokio::test]
async fn ready_accepts_4097_referenced_users_without_voice() {
	let users = (2..4099)
		.map(|id| json!({"id":id.to_string(),"username":"Synthetic user"}))
		.collect();
	login(users, Vec::new(), None).await;
}

#[tokio::test]
async fn ready_accepts_byte_heavy_referenced_users_without_voice() {
	let name = "\u{1f980}".repeat(128);
	let users = (2..3002)
		.map(|id| json!({"id":id.to_string(),"username":name}))
		.collect();
	login(users, Vec::new(), None).await;
}

#[tokio::test]
async fn supplemental_accepts_4097_members_without_voice() {
	let members = |start, end| {
		(start..end)
			.map(
				|id: u64| json!({"user":{"id":id.to_string(),"username":"Synthetic member"},"roles":[]}),
			)
			.collect::<Vec<_>>()
	};
	login(
		Vec::new(),
		Vec::new(),
		Some(json!({
			"guilds":[{"id":"10","members":members(2,3002),"voice_states":[]}],
			"merged_members":[members(3002,4099)]
		})),
	)
	.await;
}

#[tokio::test]
async fn ready_between_4_and_64_mib_logs_in() {
	// A normal account in many servers receives a READY far above the per-event bound.
	let padding = "x".repeat(64 * 1024);
	let guilds = (1..=96)
		.map(|id| {
			json!({"id":id.to_string(),"owner_id":"1","name":"Synthetic","roles":[],"channels":[],
				"synthetic_padding":padding})
		})
		.collect::<Vec<_>>();
	assert!(serde_json::to_vec(&guilds).unwrap().len() > discord_protocol::MAX_WIRE);
	login(Vec::new(), guilds, None).await;
}

#[tokio::test]
async fn oversized_frames_stop_login_during_and_after_hello() {
	for after_hello in [false, true] {
		timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				if after_hello {
					send(
						&mut socket,
						json!({"op":10,"d":{"heartbeat_interval":1000}}),
					)
					.await;
					assert_eq!(packet(&mut socket).await["op"], 2);
				}
				// The client may close as soon as it reads the oversized frame header.
				let _ = socket
					.send(Frame::Text(" ".repeat(MAX_GATEWAY_WIRE + 1).into()))
					.await;
				// Keep the peer open until the client reports the capacity error.
				socket
			};
			let client = run_inner(
				Arc::new(
					SessionSecret::from_owner_input("synthetic-frame-limit-secret".into()).unwrap(),
				),
				"wss://gateway.discord.gg/".into(),
				watch::channel(None).1,
				mpsc::channel(1).1,
				None,
				|event| {
					assert!(
						!matches!(
							event,
							Event::Startup(_) | Event::Ready { .. } | Event::Disconnected
						),
						"oversized login must stop without Ready or reconnecting"
					);
					Ok(())
				},
				Some(&endpoint),
			);
			let (_socket, result) = tokio::join!(server, client);
			assert_eq!(
				result,
				Err(Failure::CapacityAt(
					"Gateway frame exceeds 64 MiB; connection stopped"
				)),
				"after_hello={after_hello}"
			);
		})
		.await
		.expect("oversized synthetic frame exceeded its bounded deadline");
	}
}

#[tokio::test]
async fn invalid_owner_identity_or_session_never_emits_startup() {
	for (field, value) in [
		("username", ""),
		("username", "bad\nname"),
		("session_id", ""),
		("session_id", "bad\nsession"),
	] {
		timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let endpoint = format!("ws://{}/", listener.local_addr().unwrap());
			let server = async {
				let (stream, _) = listener.accept().await.unwrap();
				let mut socket = accept_async(stream).await.unwrap();
				send(
					&mut socket,
					json!({"op":10,"d":{"heartbeat_interval":1000}}),
				)
				.await;
				assert_eq!(packet(&mut socket).await["op"], 2);
				let mut payload = json!({"op":0,"t":"READY","s":1,"d":{
					"user":{"id":"1","username":"Synthetic"},"session_id":"synthetic",
					"resume_gateway_url":"wss://gateway.discord.gg/"}});
				if field == "username" {
					payload["d"]["user"][field] = json!(value);
				} else {
					payload["d"][field] = json!(value);
				}
				send(&mut socket, payload).await;
				socket
			};
			let client = run_inner(
				Arc::new(
					SessionSecret::from_owner_input("synthetic-identity-test".into()).unwrap(),
				),
				"wss://gateway.discord.gg/".into(),
				watch::channel(None).1,
				mpsc::channel(1).1,
				None,
				|event| {
					assert!(event.ready_navigation().is_none());
					Ok(())
				},
				Some(&endpoint),
			);
			let (_socket, result) = tokio::join!(server, client);
			assert_eq!(
				result,
				Err(Failure::ProtocolAt(
					"Gateway login: invalid account identity or session ID"
				))
			);
		})
		.await
		.unwrap();
	}
}
