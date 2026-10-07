//! Offline pinned-dependency reproduction: synthetic Unix sockets, no host/devices/audio.
use pulseaudio::{Client, protocol};
use std::{
	io::{BufReader, Read},
	os::unix::net::UnixStream,
	thread,
	time::{Duration, Instant},
};
const POLLS: usize = 300;

fn fixture(queries: usize) -> (Client, thread::JoinHandle<UnixStream>) {
	let (client_socket, server_socket) = UnixStream::pair().unwrap();
	let server = thread::spawn(move || {
		let mut socket = BufReader::new(server_socket);
		let version = protocol::MAX_VERSION;
		let (seq, command) = protocol::read_command_message(&mut socket, version).unwrap();
		assert!(matches!(command, protocol::Command::Auth(_)));
		protocol::write_reply_message(
			socket.get_mut(),
			seq,
			&protocol::AuthReply {
				version,
				use_memfd: false,
				use_shm: false,
			},
			version,
		)
		.unwrap();
		let (seq, command) = protocol::read_command_message(&mut socket, version).unwrap();
		assert!(matches!(command, protocol::Command::SetClientName(_)));
		protocol::write_reply_message(
			socket.get_mut(),
			seq,
			&protocol::SetClientNameReply { client_id: 42 },
			version,
		)
		.unwrap();
		for _ in 0..queries {
			let (seq, command) = protocol::read_command_message(&mut socket, version).unwrap();
			assert!(matches!(command, protocol::Command::GetSinkInfoList));
			protocol::write_reply_message(
				socket.get_mut(),
				seq,
				&Vec::<protocol::SinkInfo>::new(),
				version,
			)
			.unwrap();
		}
		socket.into_inner()
	});
	let client =
		Client::new_unix(c"serein-offline-host-probe", client_socket, None::<&[u8]>).unwrap();
	(client, server)
}

fn live(sockets: &mut [UnixStream]) -> usize {
	sockets
		.iter_mut()
		.filter(|socket| {
			socket.set_nonblocking(true).unwrap();
			let mut socket = &**socket;
			match socket.read(&mut [0u8; 1]) {
				Ok(0) => false,
				Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => true,
				other => panic!("unexpected socket state {other:?}"),
			}
		})
		.count()
}

fn main() {
	println!(
		"Synthetic PulseAudio 0.3.1 probe: {POLLS} metadata polls; no real audio server or devices."
	);
	let started = Instant::now();
	let mut peers = Vec::with_capacity(POLLS);
	for _ in 0..POLLS {
		let (client, server) = fixture(1);
		assert!(
			futures::executor::block_on(client.list_sinks())
				.unwrap()
				.is_empty()
		);
		peers.push(server.join().unwrap());
		// Let the reactor park after consuming its last reply, as between real 1s polls.
		thread::sleep(Duration::from_millis(2));
		drop(client);
	}
	let baseline_ms = started.elapsed().as_millis();
	thread::sleep(Duration::from_millis(30));
	let baseline_live = live(&mut peers);
	println!(
		"baseline fresh clients: created={POLLS}, still_connected_after_all_clients_dropped={baseline_live}, elapsed_ms={baseline_ms} (includes 600ms deliberate park delay)"
	);
	// Reclaim every synthetic connection before running the reuse case.
	for socket in peers {
		socket.shutdown(std::net::Shutdown::Both).unwrap();
	}
	thread::sleep(Duration::from_millis(30));
	let started = Instant::now();
	let (client, server) = fixture(POLLS);
	for _ in 0..POLLS {
		assert!(
			futures::executor::block_on(client.list_sinks())
				.unwrap()
				.is_empty()
		);
		thread::sleep(Duration::from_millis(2));
	}
	let mut peer = [server.join().unwrap()];
	let after_ms = started.elapsed().as_millis();
	let active_live = live(&mut peer);
	drop(client);
	thread::sleep(Duration::from_millis(30));
	let dropped_live = live(&mut peer);
	println!(
		"reused client: created=1, connected_while_retained={active_live}, still_connected_after_drop={dropped_live}, elapsed_ms={after_ms} (includes 600ms deliberate park delay)"
	);
	for socket in peer {
		socket.shutdown(std::net::Shutdown::Both).unwrap();
	}
	assert_eq!(
		baseline_live, POLLS,
		"expected pinned dependency's idle-drop leak"
	);
	assert_eq!(active_live, 1);
}
