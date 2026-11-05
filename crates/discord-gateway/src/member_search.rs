//! On-demand Gateway member queries; never requests a complete guild directory.
use client_core::{
	Event,
	auth::Failure,
	member_search::{LIMIT, MAX_BYTES, Request},
};
use discord_protocol::{MemberDto, MemberItem};
use model::Id;
use serde::Deserialize;
use tokio::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as Frame;

#[derive(Deserialize)]
struct Chunk {
	guild_id: Id,
	nonce: Option<String>,
	chunk_index: usize,
	chunk_count: usize,
	members: Vec<MemberDto>,
	/// Presences for the returned members, each naming its own user.
	#[serde(default)]
	presences: Vec<Box<serde_json::value::RawValue>>,
}
/// Live patches that arrived after a request was sent, bounded independently per slot.
#[derive(Default)]
struct InFlightPresence {
	updates: std::collections::BTreeMap<Id, client_core::presence::Update>,
	lossy: bool,
}
impl InFlightPresence {
	/// Merge partial fields, marking unreconciled chunk data unknown if either cap is exceeded.
	fn push(&mut self, patch: &client_core::presence::Update) {
		let mut merged = self
			.updates
			.get(&patch.user)
			.cloned()
			.unwrap_or_else(|| patch.clone());
		merged.merge(patch.clone());
		let retained = self
			.updates
			.values()
			.map(|u| u.heap_bytes() + size_of::<Id>() + size_of::<client_core::presence::Update>())
			.sum::<usize>();
		let previous = self.updates.get(&patch.user).map_or(0, |u| {
			u.heap_bytes() + size_of::<Id>() + size_of::<client_core::presence::Update>()
		});
		if (self.updates.len() == LIMIT && previous == 0)
			|| retained - previous
				+ merged.heap_bytes()
				+ size_of::<Id>()
				+ size_of::<client_core::presence::Update>()
				> MAX_BYTES
		{
			self.lossy = true;
			// A rejected newer patch must not leave an older patch authoritative.
			self.updates.remove(&patch.user);
		} else {
			self.updates.insert(patch.user, merged);
		}
	}
}
pub(super) struct Search {
	pub queued: [Option<Request>; 2],
	active: [Option<(Request, Instant)>; 2],
	next_send: Instant,
	seen: [Option<u64>; 2],
	/// Completed lookup snapshots, bounded by the same row/byte caps as their emitted events.
	cached: [Option<(Request, Vec<model::Member>)>; 2],
	presence_deadline: Option<Instant>,
	inflight: [InFlightPresence; 2],
}
impl Default for Search {
	fn default() -> Self {
		Self {
			queued: Default::default(),
			active: Default::default(),
			seen: [None; 2],
			cached: Default::default(),
			presence_deadline: None,
			inflight: Default::default(),
			next_send: Instant::now(),
		}
	}
}
impl Search {
	/// Replace requested slots and discard snapshots and live patches for cancelled queries.
	pub fn update(&mut self, requests: &[Option<Request>; 2]) {
		for (slot, request) in requests.iter().enumerate() {
			if self.active[slot]
				.as_ref()
				.is_some_and(|(old, _)| Some(old) != request.as_ref())
			{
				self.active[slot] = None;
				self.inflight[slot] = Default::default();
			}
			if request.is_none() {
				self.seen[slot] = None;
				self.queued[slot] = None;
			}
			if self.cached[slot]
				.as_ref()
				.is_some_and(|(old, _)| Some(old) != request.as_ref())
			{
				self.cached[slot] = None;
			}
			if let Some(request) = request
				&& self.seen[slot] != Some(request.nonce)
			{
				self.seen[slot] = Some(request.nonce);
				self.queued[slot] = Some(request.clone());
			}
		}
		if self.cached.iter().all(Option::is_none) {
			self.presence_deadline = None;
		}
	}

	/// Earliest lookup timeout, send opportunity, or coalesced presence refresh.
	pub fn deadline(&self) -> Option<Instant> {
		self.active
			.iter()
			.flatten()
			.map(|(_, deadline)| *deadline)
			.chain(
				self.queued
					.iter()
					.any(Option::is_some)
					.then_some(self.next_send),
			)
			.chain(self.presence_deadline)
			.min()
	}
	/// Flush due refreshes/timeouts and return at most one rate-limited lookup packet.
	pub fn tick(
		&mut self,
		emit: &impl Fn(Event) -> Result<(), Failure>,
	) -> Result<Option<Frame>, Failure> {
		let now = Instant::now();
		if self
			.presence_deadline
			.is_some_and(|deadline| now >= deadline)
		{
			self.presence_deadline = None;
			for (request, rows) in self.cached.iter().flatten() {
				emit(Event::MemberSearch {
					request: request.clone(),
					result: Ok(rows.clone()),
				})?;
			}
		}
		for (slot, active) in self.active.iter_mut().enumerate() {
			if active
				.as_ref()
				.is_some_and(|(_, deadline)| now >= *deadline)
			{
				let (request, _) = active.take().unwrap();
				self.inflight[slot] = Default::default();
				emit(Event::MemberSearch {
					request,
					result: Err(Failure::ProtocolAt("Member search timed out; try again")),
				})?;
			}
		}
		if now < self.next_send {
			return Ok(None);
		}
		let Some(slot) = self.queued.iter().position(Option::is_some) else {
			return Ok(None);
		};
		let request = self.queued[slot].take().unwrap();
		if !request.valid() {
			return Ok(None);
		}
		let mut data = serde_json::json!({"guild_id": request.guild.to_string(), "limit": LIMIT, "presences": true, "nonce": request.nonce.to_string()});
		if !request.users.is_empty() {
			data["user_ids"] = serde_json::json!(
				request
					.users
					.iter()
					.map(ToString::to_string)
					.collect::<Vec<_>>()
			);
		} else if let Ok(id) = request.query.parse::<u64>()
			&& id != 0
		{
			data["user_ids"] = serde_json::json!([id.to_string()]);
		} else {
			data["query"] = serde_json::json!(request.query);
		}
		self.inflight[slot] = Default::default();
		self.active[slot] = Some((request, now + Duration::from_secs(15)));
		self.next_send = now + Duration::from_secs(1);
		Ok(Some(Frame::Text(
			serde_json::json!({"op":8,"d":data}).to_string().into(),
		)))
	}
	/// Retain in-flight guild patches and reconcile completed lookup rows, batching repaint events.
	pub fn presence(&mut self, update: &discord_protocol::presence::PresenceUpdate, now: Instant) {
		let active = self
			.active
			.iter()
			.flatten()
			.any(|(request, _)| update.guild == Some(request.guild));
		if !active
			&& !self.cached.iter().flatten().any(|(request, rows)| {
				update.guild == Some(request.guild)
					&& rows.iter().any(|row| row.user.id == update.user)
			}) {
			return;
		}
		let patch = client_core::presence::Update {
			user: update.user,
			avatar: update.avatar.clone(),
			status: update.status.clone(),
			custom_status: update.custom_status.clone(),
			activities: update.activities.clone(),
			clients: update.clients.clone(),
		};
		for (slot, active) in self.active.iter().enumerate() {
			if active.as_ref().is_some_and(|(request, _)| {
				update.guild == Some(request.guild)
					&& (request.users.is_empty() || request.users.contains(&update.user))
			}) {
				self.inflight[slot].push(&patch);
			}
		}
		for (request, rows) in self.cached.iter_mut().flatten() {
			if update.guild != Some(request.guild) {
				continue;
			}
			let bytes = rows.iter().map(model::Member::bytes).sum::<usize>();
			let Some(row) = rows.iter_mut().find(|row| row.user.id == patch.user) else {
				continue;
			};
			let previous = model::MemberPresence {
				user: row.user.id,
				status: row.status.clone(),
				custom_status: row.custom_status.clone(),
				activities: row.activities.clone(),
				clients: row.clients,
			};
			let resolved = patch.resolve(Some(&previous));
			if resolved == previous {
				continue;
			}
			if bytes - row.bytes() + client_core::presence::projected_row_bytes(row, &resolved)
				> MAX_BYTES
			{
				// Forget stale presence rather than retain a misleading dot or exceed the cache cap.
				row.status = None;
				row.custom_status = None;
				row.activities.clear();
				row.clients = Default::default();
			} else {
				row.status = resolved.status;
				row.custom_status = resolved.custom_status;
				row.activities = resolved.activities;
				row.clients = resolved.clients;
			}
			self.presence_deadline
				.get_or_insert(now + Duration::from_millis(100));
		}
	}

	/// Decode a matching chunk and apply newer in-flight patches before emitting its rows.
	pub fn chunk(&mut self, bytes: &[u8]) -> Option<Event> {
		if bytes.len() > 512 * 1024 {
			return None;
		}
		let chunk: Chunk = discord_protocol::decode(bytes).ok()?;
		let slot = self.active.iter().position(|active| {
			active.as_ref().is_some_and(|(request, _)| {
				request.guild == chunk.guild_id
					&& chunk.nonce.as_deref() == Some(request.nonce.to_string().as_str())
			})
		})?;
		let (request, _) = self.active[slot].take()?;
		let inflight = std::mem::take(&mut self.inflight[slot]);
		let result = (|| {
			if chunk.chunk_index != 0 || chunk.chunk_count != 1 || chunk.members.len() > LIMIT {
				return Err(Failure::Protocol);
			}
			// A lookup answers with each member's presence when it is asked for; keeping it
			// lets a mention suggestion show a status the lazy member window does not hold.
			let presences: std::collections::BTreeMap<
				Id,
				discord_protocol::presence::PresenceUpdate,
			> = chunk
				.presences
				.iter()
				.take(LIMIT)
				.filter_map(|raw| discord_protocol::presence::decode(raw.get().as_bytes()).ok())
				.map(|update| (update.user, update))
				.collect();
			let rows: Vec<_> = chunk
				.members
				.into_iter()
				.filter_map(|member| {
					let mut row = MemberItem::Member {
						presence: model::Patch::Absent,
						member: Box::new(member),
					}
					.into_model()?;
					if let Some(update) = presences.get(&row.user.id) {
						let resolved = client_core::presence::Update {
							user: update.user,
							avatar: update.avatar.clone(),
							status: update.status.clone(),
							custom_status: update.custom_status.clone(),
							activities: update.activities.clone(),
							clients: update.clients.clone(),
						}
						.resolve(None);
						row.status = resolved.status;
						row.custom_status = resolved.custom_status;
						row.activities = resolved.activities;
						row.clients = resolved.clients;
					}
					if inflight.lossy {
						row.status = None;
						row.custom_status = None;
						row.activities = vec![];
						row.clients = Default::default();
					}
					if let Some(patch) = inflight.updates.get(&row.user.id) {
						let previous = model::MemberPresence {
							user: row.user.id,
							status: row.status.clone(),
							custom_status: row.custom_status.clone(),
							activities: row.activities.clone(),
							clients: row.clients,
						};
						let resolved = patch.resolve(Some(&previous));
						row.status = resolved.status;
						row.custom_status = resolved.custom_status;
						row.activities = resolved.activities;
						row.clients = resolved.clients;
					}
					Some(row)
				})
				.collect();
			if rows.iter().any(|m| !m.valid())
				|| rows.iter().map(model::Member::bytes).sum::<usize>() > MAX_BYTES
			{
				return Err(Failure::Capacity);
			}
			Ok(rows)
		})();
		if let Ok(rows) = &result
			&& self.seen[request.slot] == Some(request.nonce)
		{
			self.cached[request.slot] = Some((request.clone(), rows.clone()));
		}
		Some(Event::MemberSearch { request, result })
	}
}

/// Synthetic nonce, request-shape, presence attachment and row-limit regression.
#[cfg(debug_assertions)]
pub fn debug_check(request: Request) -> Event {
	let mut search = Search::default();
	search.queued[request.slot] = Some(request.clone());
	let frame = search.tick(&|_| Ok(())).unwrap().unwrap();
	let packet: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
	assert_eq!(packet["op"], 8);
	if request.users.is_empty() {
		assert_eq!(packet["d"]["query"], request.query);
	} else {
		assert_eq!(
			packet["d"]["user_ids"],
			serde_json::json!(
				request
					.users
					.iter()
					.map(ToString::to_string)
					.collect::<Vec<_>>()
			)
		);
		assert!(packet["d"].get("query").is_none());
	}
	assert_eq!(packet["d"]["limit"], 100);
	assert_eq!(packet["d"]["presences"], true);
	let mut chunk = serde_json::json!({"guild_id":request.guild.to_string(),"nonce":"wrong","chunk_index":0,"chunk_count":1,"members":[{"user":{"id":"987654321","username":"remote-person"},"nick":"OutsideFirstHundred","roles":[]}],"presences":[{"user":{"id":"987654321"},"status":"online"}]});
	assert!(search.chunk(&serde_json::to_vec(&chunk).unwrap()).is_none());
	chunk["nonce"] = serde_json::json!(request.nonce.to_string());
	let event = search.chunk(&serde_json::to_vec(&chunk).unwrap()).unwrap();
	assert!(
		matches!(&event, Event::MemberSearch { result: Ok(rows), .. } if rows.len() == 1 && rows[0].user.id == Id(987654321) && rows[0].status.as_deref() == Some("online")),
		"a lookup keeps the presence it asked for"
	);
	assert!(search.chunk(&serde_json::to_vec(&chunk).unwrap()).is_none());
	search.next_send = Instant::now();
	search.queued[request.slot] = Some(request.clone());
	search.tick(&|_| Ok(())).unwrap();
	chunk["members"] = serde_json::Value::Array(vec![chunk["members"][0].clone(); LIMIT + 1]);
	assert!(matches!(
		search.chunk(&serde_json::to_vec(&chunk).unwrap()),
		Some(Event::MemberSearch { result: Err(_), .. })
	));
	event
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::member_search::Request;

	/// Regression: a name lookup keeps the presence it asked Discord for, so a mention
	/// suggestion can show a status the lazy member window no longer holds.
	#[test]
	fn a_member_lookup_attaches_the_presence_it_requested() {
		let request = Request {
			guild: Id(1),
			channel: Id(2),
			query: "remote-person".into(),
			users: vec![],
			nonce: 7,
			slot: 0,
		};
		let mut search = Search::default();
		search.update(&[Some(request), None]);
		let frame = search.tick(&|_| Ok(())).unwrap().unwrap();
		let packet: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
		assert_eq!(packet["d"]["presences"], true);
		search.presence(
			&discord_protocol::presence::decode(
				br#"{"guild_id":"1","user":{"id":"9"},"status":"offline"}"#,
			)
			.unwrap(),
			Instant::now(),
		);
		let chunk = serde_json::json!({
			"guild_id": "1",
			"nonce": "7",
			"chunk_index": 0,
			"chunk_count": 1,
			"members": [
				{"user": {"id": "9", "username": "known"}, "roles": []},
				{"user": {"id": "10", "username": "unknown"}, "roles": []},
				{"user": {"id": "11", "username": "offline"}, "roles": []},
				{"user": {"id": "12", "username": "null-status"}, "roles": []},
			],
			"presences": [
				{"user": {"id": "9"}, "status": "idle"},
				{"user": {"id": "11"}, "status": "offline", "client_status":{"mobile":"online"}, "activities":[{"type":0,"name":"Old game"},{"type":4,"state":"Old custom status"}]},
				{"user": {"id": "12"}, "status": null, "client_status":{"mobile":"online"}, "activities":[{"type":0,"name":"Old game"},{"type":4,"state":"Old custom status"}]},
			],
		});
		let Some(Event::MemberSearch {
			result: Ok(rows), ..
		}) = search.chunk(&serde_json::to_vec(&chunk).unwrap())
		else {
			panic!("the lookup must decode")
		};
		assert_eq!(rows.len(), 4);
		for row in &rows[2..] {
			assert!(row.activities.is_empty());
			assert!(row.custom_status.is_none());
			assert!(!row.clients.any());
		}
		assert_eq!(rows[0].status.as_deref(), Some("offline"));
		assert!(search.inflight[0].updates.is_empty());
		assert_eq!(
			rows[1].status, None,
			"a member without a presence stays unknown rather than guessed"
		);
		let now = Instant::now();
		let decode = |guild: &str, status: &str| {
			discord_protocol::presence::decode(
				serde_json::json!({"guild_id":guild,"user":{"id":"9"},"status":status})
					.to_string()
					.as_bytes(),
			)
			.unwrap()
		};
		search.presence(&decode("99", "offline"), now);
		assert!(
			search.presence_deadline.is_none(),
			"other guilds cannot change lookup rows"
		);
		search.presence(&decode("1", "online"), now);
		search.presence(&decode("1", "offline"), now);
		assert_eq!(
			search.presence_deadline,
			Some(now + Duration::from_millis(100))
		);
		search.presence_deadline = Some(now);
		let events = std::cell::RefCell::new(Vec::new());
		search
			.tick(&|event| {
				events.borrow_mut().push(event);
				Ok(())
			})
			.unwrap();
		let events = events.into_inner();
		assert!(
			matches!(&events[..], [Event::MemberSearch { result: Ok(rows), .. }]
			if rows[0].status.as_deref() == Some("offline") && rows[1].status.is_none())
		);
		// Fill a still-valid snapshot to its byte budget; a larger live patch must forget
		// stale status without growing the cache. Roles make this independent of UI copy.
		let (_, cached) = search.cached[0].as_mut().unwrap();
		let bytes = cached.iter().map(model::Member::bytes).sum::<usize>();
		cached[1].roles = vec![Id(1); (MAX_BYTES - bytes) / size_of::<Id>()];
		let large = discord_protocol::presence::decode(
			br#"{"guild_id":"1","user":{"id":"9"},"status":"online","activities":[{"type":4,"state":"Bounded synthetic custom presence"}]}"#
		).unwrap();
		search.presence(&large, now);
		let (_, cached) = search.cached[0].as_ref().unwrap();
		assert!(cached[0].status.is_none());
		assert!(cached.iter().all(model::Member::valid));
		assert!(cached.iter().map(model::Member::bytes).sum::<usize>() <= MAX_BYTES);
		search.update(&[None, None]);
		search.presence(&decode("1", "online"), now);
		assert!(search.cached.iter().all(Option::is_none));
		assert!(search.presence_deadline.is_none());
	}
	/// Both pending-patch caps degrade to unknown status and release state after completion.
	#[test]
	fn inflight_presence_overflow_forgets_unreconciled_status() {
		let request = Request {
			guild: Id(1),
			channel: Id(2),
			query: "known".into(),
			users: vec![],
			nonce: 8,
			slot: 0,
		};
		let mut search = Search::default();
		search.update(&[Some(request), None]);
		search.tick(&|_| Ok(())).unwrap();
		for user in 1000..1102 {
			let update = discord_protocol::presence::decode(serde_json::json!({"guild_id":"1","user":{"id":user.to_string()},"status":"online"}).to_string().as_bytes()).unwrap();
			search.presence(&update, Instant::now());
		}
		assert!(search.inflight[0].lossy);
		assert!(search.inflight[0].updates.len() <= LIMIT);
		// A smaller number of activity-rich patches must hit the byte cap independently.
		search.inflight[0] = Default::default();
		let activities: Vec<_> = (0..model::MAX_RICH_ACTIVITIES).map(|index| serde_json::json!({"type":0,"name":format!("{index}{}", "a".repeat(127)),"details":"b".repeat(128),"state":"c".repeat(128)})).collect();
		for user in 2000..2080 {
			let wire = serde_json::json!({"guild_id":"1","user":{"id":user.to_string()},"status":"online","activities":activities});
			search.presence(
				&discord_protocol::presence::decode(wire.to_string().as_bytes()).unwrap(),
				Instant::now(),
			);
		}
		assert!(search.inflight[0].lossy);
		assert!(
			search.inflight[0].updates.len() < LIMIT,
			"byte cap applies before the item cap"
		);
		assert!(
			search.inflight[0]
				.updates
				.values()
				.map(|u| u.heap_bytes()
					+ size_of::<Id>()
					+ size_of::<client_core::presence::Update>())
				.sum::<usize>()
				<= MAX_BYTES
		);
		let chunk = br#"{"guild_id":"1","nonce":"8","chunk_index":0,"chunk_count":1,"members":[{"user":{"id":"9","username":"known"},"roles":[]}],"presences":[{"user":{"id":"9"},"status":"online"}]}"#;
		let Some(Event::MemberSearch {
			result: Ok(rows), ..
		}) = search.chunk(chunk)
		else {
			panic!("bounded lookup result");
		};
		assert!(
			rows[0].status.is_none(),
			"dropped live patches cannot make old chunk status authoritative"
		);
		assert!(search.inflight[0].updates.is_empty());
		assert!(!search.inflight[0].lossy);
	}
}
