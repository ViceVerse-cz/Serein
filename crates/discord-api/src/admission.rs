//! One in-flight request per normalized route or learned service bucket. Feedback is
//! applied before the next request is admitted; unrelated routes keep their concurrency.
//! Only fixed-size hashes are retained, never URLs, tokens or response header strings.
use client_core::auth::Failure;
use std::{
	hash::{Hash, Hasher},
	sync::Mutex,
	time::Duration,
};
use tokio::{
	sync::Notify,
	time::{Instant, sleep_until},
};
use wreq::{Method, header::HeaderMap};

const MAX_ENTRIES: usize = 128;
const MAX_BUCKET_BYTES: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Route {
	name: u64,
	major: u64,
}

fn hash(value: impl Hash) -> u64 {
	let mut hasher = std::collections::hash_map::DefaultHasher::new();
	value.hash(&mut hasher);
	hasher.finish()
}

fn route(method: &Method, path: &str) -> Route {
	let mut name = std::collections::hash_map::DefaultHasher::new();
	method.hash(&mut name);
	let mut major = std::collections::hash_map::DefaultHasher::new();
	let path = path.split('?').next().unwrap_or(path);
	let mut previous = "";
	for part in path.split('/').filter(|part| !part.is_empty()) {
		if matches!(previous, "channels" | "guilds" | "webhooks") {
			(previous, part).hash(&mut major);
		}
		if part.bytes().all(|byte| byte.is_ascii_digit()) {
			"{id}".hash(&mut name);
		} else {
			part.hash(&mut name);
		}
		previous = part;
	}
	Route {
		name: name.finish(),
		major: major.finish(),
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
	Route(Route),
	Bucket(u64, u64),
}

struct Slot {
	key: Key,
	busy: usize,
	until: Instant,
}

struct State {
	routes: Vec<(Route, Key)>,
	slots: Vec<Slot>,
}

const _: () = assert!(MAX_ENTRIES * (size_of::<(Route, Key)>() + size_of::<Slot>()) <= 16 * 1024);

impl Default for State {
	fn default() -> Self {
		Self {
			routes: Vec::with_capacity(MAX_ENTRIES),
			slots: Vec::with_capacity(MAX_ENTRIES),
		}
	}
}

impl State {
	fn collect(&mut self) {
		self.slots
			.retain(|slot| slot.busy != 0 || self.routes.iter().any(|(_, key)| *key == slot.key));
	}

	fn index(&mut self, route: Route, now: Instant) -> Result<usize, Failure> {
		if let Some((_, key)) = self.routes.iter().find(|(value, _)| *value == route) {
			return self
				.slots
				.iter()
				.position(|slot| slot.key == *key)
				.ok_or(Failure::Protocol);
		}
		if self.routes.len() == MAX_ENTRIES {
			let index = self
				.routes
				.iter()
				.position(|(_, key)| {
					self.slots
						.iter()
						.any(|slot| slot.key == *key && slot.busy == 0 && slot.until <= now)
				})
				.ok_or(Failure::Capacity)?;
			self.routes.remove(index);
			self.collect();
		}
		if self.slots.len() == MAX_ENTRIES {
			return Err(Failure::Capacity);
		}
		let key = Key::Route(route);
		self.routes.push((route, key));
		self.slots.push(Slot {
			key,
			busy: 0,
			until: now,
		});
		Ok(self.slots.len() - 1)
	}
}

#[derive(Default)]
pub(crate) struct Admission {
	state: Mutex<State>,
	changed: Notify,
}

pub(crate) struct Ticket<'a> {
	admission: &'a Admission,
	route: Route,
	key: Key,
}

impl Admission {
	pub(crate) async fn acquire(
		&self,
		method: &Method,
		path: &str,
		speculative: bool,
	) -> Result<Ticket<'_>, Failure> {
		let route = route(method, path);
		loop {
			// Register before inspecting state: a response/drop must not lose its wakeup.
			let changed = self.changed.notified();
			tokio::pin!(changed);
			changed.as_mut().enable();
			let until = {
				let now = Instant::now();
				let mut state = self.state.lock().map_err(|_| Failure::Protocol)?;
				let index = state.index(route, now)?;
				let slot = &mut state.slots[index];
				if slot.busy == 0 && slot.until <= now {
					slot.busy = 1;
					return Ok(Ticket {
						admission: self,
						route,
						key: slot.key,
					});
				}
				if speculative {
					return Err(Failure::Capacity);
				}
				(slot.until > now).then_some(slot.until)
			};
			match until {
				Some(until) => {
					tokio::select! { () = &mut changed => {}, () = sleep_until(until) => {} }
				}
				None => changed.await,
			}
		}
	}
}

impl Ticket<'_> {
	/// Merge aliases using the service's bucket id plus major resource. Unknown or
	/// malformed metadata retains route serialization rather than inventing a quota.
	pub(crate) fn feedback(&mut self, headers: &HeaderMap) {
		let Ok(mut state) = self.admission.state.lock() else {
			return;
		};
		let bucket = headers
			.get("x-ratelimit-bucket")
			.filter(|value| !value.is_empty() && value.len() <= MAX_BUCKET_BYTES)
			.map(|value| Key::Bucket(hash(value.as_bytes()), self.route.major));
		if let Some(key) = bucket.filter(|key| *key != self.key) {
			let target = state
				.slots
				.iter()
				.position(|slot| slot.key == key)
				.or_else(|| {
					(state.slots.len() < MAX_ENTRIES).then(|| {
						state.slots.push(Slot {
							key,
							busy: 0,
							until: Instant::now(),
						});
						state.slots.len() - 1
					})
				});
			if let Some(target) = target {
				let old = state
					.slots
					.iter()
					.position(|slot| slot.key == self.key)
					.expect("active admission slot");
				let until = state.slots[old].until;
				state.slots[old].busy -= 1;
				state.slots[target].busy += 1;
				state.slots[target].until = state.slots[target].until.max(until);
				if let Some((_, mapped)) = state
					.routes
					.iter_mut()
					.find(|(route, _)| *route == self.route)
				{
					*mapped = key;
				}
				self.key = key;
				state.collect();
			}
		}
		if headers
			.get("x-ratelimit-remaining")
			.is_some_and(|value| value == "0")
			&& let Some(delay) = headers
				.get("x-ratelimit-reset-after")
				.and_then(|value| value.to_str().ok())
				.and_then(|value| value.parse::<f64>().ok())
				.filter(|value| value.is_finite() && (0.0..=86400.0).contains(value))
			&& let Some(slot) = state.slots.iter_mut().find(|slot| slot.key == self.key)
		{
			slot.until = slot
				.until
				.max(Instant::now() + Duration::from_secs_f64(delay.max(0.05)));
		}
		self.admission.changed.notify_waiters();
	}
}

impl Drop for Ticket<'_> {
	fn drop(&mut self) {
		if let Ok(mut state) = self.admission.state.lock()
			&& let Some(slot) = state.slots.iter_mut().find(|slot| slot.key == self.key)
		{
			slot.busy -= 1;
		}
		self.admission.changed.notify_waiters();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::DiscordApi;
	use client_core::auth::SessionSecret;
	use std::{
		future::{Future, poll_fn},
		pin::Pin,
		sync::Arc,
		task::Poll,
	};

	async fn pending(mut future: Pin<&mut impl Future>) -> bool {
		poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx).is_pending())).await
	}

	#[tokio::test]
	async fn security_stop_wakes_route_and_cooldown_waiters_without_dispatch() {
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_SECRET_MARKER".into()).unwrap(),
		))
		.unwrap();
		// A broken admission path must still never send a synthetic credential online.
		api.base = "http://127.0.0.1:9".into();
		let _held = api
			.admission
			.acquire(&Method::GET, "/users/@me", false)
			.await
			.unwrap();
		let request = api.request(Method::GET, "/users/@me", None);
		tokio::pin!(request);
		assert!(pending(request.as_mut()).await);
		*api.cooldown.lock().await = Instant::now() + Duration::from_secs(3600);
		let cooldown = api.wait_cooldown(false);
		tokio::pin!(cooldown);
		assert!(pending(cooldown.as_mut()).await);
		api.stop_with(Failure::AccountRestricted);
		assert_eq!(request.await, Err(Failure::AccountRestricted));
		assert_eq!(cooldown.await, Err(Failure::AccountRestricted));
	}

	#[tokio::test]
	async fn discovery_aliases_major_resources_and_cancellation() {
		let admission = Admission::default();
		let mut first = admission
			.acquire(&Method::GET, "/channels/1/messages?limit=50", false)
			.await
			.unwrap();
		let next = admission.acquire(&Method::GET, "/channels/1/messages?limit=10", false);
		tokio::pin!(next);
		assert!(pending(next.as_mut()).await);
		let headers = HeaderMap::from_iter([(
			"x-ratelimit-bucket".parse().unwrap(),
			"shared".parse().unwrap(),
		)]);
		first.feedback(&headers);
		drop(first);
		let next = next.await.unwrap();
		drop(next);
		let mut alias = admission
			.acquire(&Method::POST, "/channels/1/messages", false)
			.await
			.unwrap();
		alias.feedback(&headers);
		assert!(
			admission
				.acquire(&Method::GET, "/channels/1/messages", true)
				.await
				.is_err()
		);
		// The same server bucket does not merge different major resources.
		let mut other = admission
			.acquire(&Method::GET, "/channels/2/messages", false)
			.await
			.unwrap();
		other.feedback(&headers);
		drop(other);
		drop(alias);
		assert!(
			admission
				.acquire(&Method::GET, "/channels/1/messages", true)
				.await
				.is_ok()
		);
	}

	#[tokio::test]
	async fn bounded_tracking_does_not_evict_a_live_cooldown() {
		let admission = Admission::default();
		let mut first = admission
			.acquire(&Method::GET, "/channels/1/messages", false)
			.await
			.unwrap();
		// Oversized bucket ids and non-finite waits cannot create a new bucket or
		// pin this route forever. A subsequent valid response must still apply.
		first.feedback(&HeaderMap::from_iter([
			(
				"x-ratelimit-bucket".parse().unwrap(),
				"x".repeat(MAX_BUCKET_BYTES + 1).parse().unwrap(),
			),
			(
				"x-ratelimit-remaining".parse().unwrap(),
				"0".parse().unwrap(),
			),
			(
				"x-ratelimit-reset-after".parse().unwrap(),
				"NaN".parse().unwrap(),
			),
		]));
		drop(first);
		let mut first = admission
			.acquire(&Method::GET, "/channels/1/messages", true)
			.await
			.unwrap();
		let headers = HeaderMap::from_iter([
			(
				"x-ratelimit-remaining".parse().unwrap(),
				"0".parse().unwrap(),
			),
			(
				"x-ratelimit-reset-after".parse().unwrap(),
				"60".parse().unwrap(),
			),
		]);
		first.feedback(&headers);
		drop(first);
		for channel in 2..=300 {
			drop(
				admission
					.acquire(
						&Method::GET,
						&format!("/channels/{channel}/messages"),
						false,
					)
					.await
					.unwrap(),
			);
		}
		assert!(
			admission
				.acquire(&Method::GET, "/channels/1/messages", true)
				.await
				.is_err()
		);
		let state = admission.state.lock().unwrap();
		assert!(state.routes.len() <= MAX_ENTRIES && state.slots.len() <= MAX_ENTRIES);
	}
}
