//! Session-only screen selection through the desktop portal; never enumerates other apps.

use std::{
	collections::HashMap,
	future::Future,
	sync::atomic::{AtomicBool, Ordering},
	time::Duration,
};

use futures_util::{FutureExt, StreamExt};
use zbus::{
	Connection, MatchRule, Message, MessageStream, Proxy,
	proxy::CacheProperties,
	zvariant::{OwnedFd, OwnedObjectPath, OwnedValue, Value},
};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
const UNAVAILABLE: &str = "Linux screen sharing requires a working ScreenCast desktop portal.";
const INVALID: &str = "The screen-sharing portal returned an invalid response.";
const CANCELLED: &str = "Screen sharing was cancelled.";
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const PICKER_TIMEOUT: Duration = Duration::from_secs(180);
const RESPONSE_BYTES: usize = 64 * 1024;

type Properties = HashMap<String, OwnedValue>;

pub(super) struct Portal {
	connection: Connection,
	owner: String,
	session: OwnedObjectPath,
	closed: MessageStream,
	closed_seen: bool,
	pub(super) node_id: u32,
	pub(super) pipewire_serial: Option<u64>,
}

impl Portal {
	pub(super) async fn open(cursor: bool, stop: &AtomicBool) -> Result<Self, &'static str> {
		if stop.load(Ordering::Acquire) {
			return Err(CANCELLED);
		}
		// This connection belongs to this capture only. Disconnecting also revokes the session
		// when a portal request fails before its session handle can be returned.
		let builder = zbus::connection::Builder::session()
			.map_err(|_| UNAVAILABLE)?
			.max_queued(2)
			.method_timeout(CALL_TIMEOUT);
		let connection = wait(builder.build(), stop, CALL_TIMEOUT)
			.await?
			.map_err(|_| UNAVAILABLE)?;
		match Self::start(&connection, cursor, stop).await {
			Ok(portal) => Ok(portal),
			Err(error) => {
				let _ = tokio::time::timeout(Duration::from_secs(1), connection.close()).await;
				Err(error)
			}
		}
	}

	async fn start(
		connection: &Connection,
		cursor: bool,
		stop: &AtomicBool,
	) -> Result<Self, &'static str> {
		let initial_proxy = proxy(connection, DESTINATION, DESKTOP, SCREENCAST).await?;
		let types: u32 = wait(
			initial_proxy.get_property("AvailableSourceTypes"),
			stop,
			CALL_TIMEOUT,
		)
		.await?
		.map_err(|_| UNAVAILABLE)?;
		if types & 3 == 0 {
			return Err("The desktop portal cannot share a screen or window.");
		}
		let modes: u32 = wait(
			initial_proxy.get_property("AvailableCursorModes"),
			stop,
			CALL_TIMEOUT,
		)
		.await?
		.unwrap_or(1);

		// Pin the service's unique owner: a well-known sender match alone cannot authenticate
		// an unsolicited unicast signal in zbus. Reading the property above activates the portal.
		let dbus = proxy(
			connection,
			"org.freedesktop.DBus",
			"/org/freedesktop/DBus",
			"org.freedesktop.DBus",
		)
		.await?;
		let owner: String = wait(
			dbus.call("GetNameOwner", &(DESTINATION,)),
			stop,
			CALL_TIMEOUT,
		)
		.await?
		.map_err(|_| UNAVAILABLE)?;
		let portal = proxy(connection, &owner, DESKTOP, SCREENCAST).await?;
		let sender = connection.unique_name().ok_or(UNAVAILABLE)?.as_str()[1..].replace('.', "_");
		let session = OwnedObjectPath::try_from(format!(
			"/org/freedesktop/portal/desktop/session/{sender}/serein"
		))
		.map_err(|_| INVALID)?;
		let closed = signals(
			connection,
			&owner,
			session.as_str(),
			SESSION,
			"Closed",
			stop,
		)
		.await?;
		let options = HashMap::from([
			("handle_token", Value::from("serein_create")),
			("session_handle_token", Value::from("serein")),
		]);
		let response = request(
			connection,
			&owner,
			&sender,
			"serein_create",
			stop,
			portal.call_method("CreateSession", &options),
		)
		.await?;
		// The portal specification deliberately uses a string for session_handle.
		let returned = response
			.get("session_handle")
			.and_then(|value| <&str>::try_from(value).ok());
		if returned != Some(session.as_str()) {
			return Err(INVALID);
		}

		let cursor_mode = if cursor && modes & 2 != 0 { 2u32 } else { 1u32 };
		if modes & cursor_mode == 0 {
			return Err("The desktop portal cannot provide the requested cursor mode.");
		}
		let options = HashMap::from([
			("handle_token", Value::from("serein_select")),
			("types", Value::from(types & 3)),
			("multiple", Value::from(false)),
			("cursor_mode", Value::from(cursor_mode)),
			("persist_mode", Value::from(0u32)),
		]);
		request(
			connection,
			&owner,
			&sender,
			"serein_select",
			stop,
			portal.call_method("SelectSources", &(&session, &options)),
		)
		.await?;
		let options = HashMap::from([("handle_token", Value::from("serein_start"))]);
		let mut response = request(
			connection,
			&owner,
			&sender,
			"serein_start",
			stop,
			portal.call_method("Start", &(&session, "", &options)),
		)
		.await?;
		let streams: Vec<(u32, Properties)> = response
			.remove("streams")
			.ok_or(INVALID)?
			.try_into()
			.map_err(|_| INVALID)?;
		if streams.len() != 1 || streams[0].0 == 0 {
			return Err(INVALID);
		}
		let node_id = streams[0].0;
		let pipewire_serial = streams[0]
			.1
			.get("pipewire-serial")
			.map(u64::try_from)
			.transpose()
			.map_err(|_| INVALID)?;
		drop(portal);
		let mut result = Self {
			connection: connection.clone(),
			owner,
			session,
			closed,
			closed_seen: false,
			node_id,
			pipewire_serial,
		};
		if result.is_closed() {
			return Err("The desktop stopped screen sharing.");
		}
		Ok(result)
	}

	/// Each pipeline needs a fresh PipeWire connection, within the same approved session.
	pub(super) async fn open_remote(&self, stop: &AtomicBool) -> Result<OwnedFd, &'static str> {
		if self.closed_seen || self.connection.closed().now_or_never().is_some() {
			return Err("The desktop stopped screen sharing.");
		}
		let portal = proxy(&self.connection, &self.owner, DESKTOP, SCREENCAST).await?;
		let options: HashMap<&str, Value<'_>> = HashMap::new();
		let message = wait(
			portal.call_method("OpenPipeWireRemote", &(&self.session, &options)),
			stop,
			CALL_TIMEOUT,
		)
		.await?
		.map_err(|_| UNAVAILABLE)?;
		if message.body().len() > RESPONSE_BYTES {
			return Err(INVALID);
		}
		message.body().deserialize().map_err(|_| INVALID)
	}

	pub(super) fn is_closed(&mut self) -> bool {
		self.closed_seen |= self.connection.closed().now_or_never().is_some()
			|| self.closed.next().now_or_never().is_some();
		self.closed_seen
	}

	pub(super) async fn close(self) {
		close_object(
			&self.connection,
			&self.owner,
			self.session.as_str(),
			SESSION,
		)
		.await;
		let _ = tokio::time::timeout(Duration::from_secs(1), self.connection.close()).await;
	}
}

async fn proxy<'a>(
	connection: &Connection,
	destination: &'a str,
	path: &'a str,
	interface: &'a str,
) -> Result<Proxy<'a>, &'static str> {
	// No property cache or background PropertiesChanged queue is needed for one capture.
	zbus::proxy::Builder::new(connection)
		.destination(destination)
		.map_err(|_| INVALID)?
		.path(path)
		.map_err(|_| INVALID)?
		.interface(interface)
		.map_err(|_| INVALID)?
		.cache_properties(CacheProperties::No)
		.build()
		.await
		.map_err(|_| UNAVAILABLE)
}

async fn signals(
	connection: &Connection,
	owner: &str,
	path: &str,
	interface: &str,
	member: &str,
	stop: &AtomicBool,
) -> Result<MessageStream, &'static str> {
	let rule = MatchRule::builder()
		.msg_type(zbus::message::Type::Signal)
		.sender(owner)
		.map_err(|_| INVALID)?
		.path(path)
		.map_err(|_| INVALID)?
		.interface(interface)
		.map_err(|_| INVALID)?
		.member(member)
		.map_err(|_| INVALID)?
		.build();
	// One response per request/session. zbus additionally caps each wire message at 128 MiB;
	// only a <=64 KiB body is admitted to our decoded portal state.
	wait(
		MessageStream::for_match_rule(rule, connection, Some(1)),
		stop,
		CALL_TIMEOUT,
	)
	.await?
	.map_err(|_| UNAVAILABLE)
}

async fn request(
	connection: &Connection,
	owner: &str,
	sender: &str,
	token: &str,
	stop: &AtomicBool,
	call: impl Future<Output = zbus::Result<Message>>,
) -> Result<Properties, &'static str> {
	let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
	// Subscribe before invoking the method so even an immediate response is retained.
	let mut responses = signals(connection, owner, &path, REQUEST, "Response", stop).await?;
	let result = wait(
		async {
			let reply = call.await.map_err(|_| UNAVAILABLE)?;
			if reply.body().len() > RESPONSE_BYTES {
				return Err(INVALID);
			}
			let returned: OwnedObjectPath = reply.body().deserialize().map_err(|_| INVALID)?;
			if returned.as_str() != path {
				return Err(INVALID);
			}
			let response = responses
				.next()
				.await
				.ok_or(UNAVAILABLE)?
				.map_err(|_| UNAVAILABLE)?;
			if response.body().len() > RESPONSE_BYTES {
				return Err(INVALID);
			}
			let (status, properties): (u32, Properties) =
				response.body().deserialize().map_err(|_| INVALID)?;
			match status {
				0 => Ok(properties),
				1 => Err(CANCELLED),
				_ => Err("The desktop could not start screen sharing."),
			}
		},
		stop,
		PICKER_TIMEOUT,
	)
	.await
	.and_then(|result| result);
	if result.is_err() {
		close_object(connection, owner, &path, REQUEST).await;
	}
	result
}

async fn close_object(connection: &Connection, owner: &str, path: &str, interface: &str) {
	let _ = tokio::time::timeout(
		Duration::from_millis(500),
		connection.call_method(Some(owner), path, Some(interface), "Close", &()),
	)
	.await;
}

async fn wait<T>(
	future: impl Future<Output = T>,
	stop: &AtomicBool,
	timeout: Duration,
) -> Result<T, &'static str> {
	tokio::pin!(future);
	let deadline = tokio::time::sleep(timeout);
	tokio::pin!(deadline);
	let mut cancellation = tokio::time::interval(Duration::from_millis(50));
	loop {
		if stop.load(Ordering::Acquire) {
			return Err(CANCELLED);
		}
		tokio::select! {
			biased;
			_ = cancellation.tick() => {},
			_ = &mut deadline => return Err("The screen-sharing portal timed out."),
			result = &mut future => return Ok(result),
		}
	}
}
