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
pub(super) struct Search {
	pub queued: [Option<Request>; 2],
	active: [Option<(Request, Instant)>; 2],
	next_send: Instant,
	seen: [Option<u64>; 2],
}
impl Default for Search {
	fn default() -> Self {
		Self {
			queued: Default::default(),
			active: Default::default(),
			seen: [None; 2],
			next_send: Instant::now(),
		}
	}
}
impl Search {
	pub fn update(&mut self, requests: &[Option<Request>; 2]) {
		for (slot, request) in requests.iter().enumerate() {
			if let Some(request) = request
				&& self.seen[slot] != Some(request.nonce)
			{
				self.seen[slot] = Some(request.nonce);
				self.queued[slot] = Some(request.clone());
			}
		}
	}

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
			.min()
	}
	pub fn tick(
		&mut self,
		emit: &impl Fn(Event) -> Result<(), Failure>,
	) -> Result<Option<Frame>, Failure> {
		let now = Instant::now();
		for active in &mut self.active {
			if active
				.as_ref()
				.is_some_and(|(_, deadline)| now >= *deadline)
			{
				let (request, _) = active.take().unwrap();
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
		self.active[slot] = Some((request, now + Duration::from_secs(15)));
		self.next_send = now + Duration::from_secs(1);
		Ok(Some(Frame::Text(
			serde_json::json!({"op":8,"d":data}).to_string().into(),
		)))
	}
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
						if let model::Patch::Value(status) = &update.status {
							row.status = Some(status.clone());
						}
						if let model::Patch::Value(custom) = &update.custom_status {
							row.custom_status = Some(custom.clone());
						}
						if let model::Patch::Value(activities) = &update.activities {
							row.activities = activities.clone();
						}
						if let model::Patch::Value(clients) = update.clients {
							row.clients = clients;
						}
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
		Some(Event::MemberSearch { request, result })
	}
}

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
		search.queued[0] = Some(request);
		let frame = search.tick(&|_| Ok(())).unwrap().unwrap();
		let packet: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
		assert_eq!(packet["d"]["presences"], true);
		let chunk = serde_json::json!({
			"guild_id": "1",
			"nonce": "7",
			"chunk_index": 0,
			"chunk_count": 1,
			"members": [
				{"user": {"id": "9", "username": "known"}, "roles": []},
				{"user": {"id": "10", "username": "unknown"}, "roles": []},
			],
			"presences": [{"user": {"id": "9"}, "status": "idle"}],
		});
		let Some(Event::MemberSearch {
			result: Ok(rows), ..
		}) = search.chunk(&serde_json::to_vec(&chunk).unwrap())
		else {
			panic!("the lookup must decode")
		};
		assert_eq!(rows.len(), 2);
		assert_eq!(rows[0].status.as_deref(), Some("idle"));
		assert_eq!(
			rows[1].status, None,
			"a member without a presence stays unknown rather than guessed"
		);
	}
}
