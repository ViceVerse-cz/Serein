//! On-demand Gateway member queries; never requests a complete guild directory.
use client_core::{
	Event,
	auth::Failure,
	member_search::{LIMIT, MAX_BYTES, Request},
};
use discord_protocol::MemberDto;
use model::Id;
use serde::Deserialize;
use std::collections::BTreeMap;
use tokio::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as Frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Purpose {
	Composer,
	Authors,
}
impl Purpose {
	pub fn slot(self) -> usize {
		match self {
			Self::Composer => 0,
			Self::Authors => 1,
		}
	}
}
pub(super) struct Query {
	pub purpose: Purpose,
	pub frame: Frame,
}

#[derive(Deserialize)]
struct Chunk {
	guild_id: Id,
	nonce: Option<String>,
	chunk_index: usize,
	chunk_count: usize,
	members: Vec<MemberDto>,
}
pub(super) struct Search {
	pub queued: [Option<Request>; 2],
	active: [Option<(Request, Option<Instant>)>; 2],
	cooldowns: BTreeMap<Id, Instant>,
	blocked_until: Option<Instant>,
	next_send: Instant,
	seen: [Option<u64>; 2],
	reported: [Option<u64>; 2],
	following: Option<(Id, Id)>,
	watching: bool,
	users: Vec<Id>,
	announced: Option<(Id, Vec<Id>)>,
}
impl Default for Search {
	fn default() -> Self {
		Self {
			queued: Default::default(),
			active: Default::default(),
			cooldowns: BTreeMap::new(),
			blocked_until: None,
			seen: [None; 2],
			reported: [None; 2],
			following: None,
			watching: false,
			users: vec![],
			announced: None,
			next_send: Instant::now(),
		}
	}
}
impl Search {
	pub fn reconnect(&mut self, resumable: bool) {
		self.announced = None;
		self.active = Default::default();
		self.queued = Default::default();
		self.seen = [None; 2];
		self.reported = [None; 2];
		if !resumable {
			self.following = None;
			self.users.clear();
		}
	}
	pub fn watch(&mut self, watch: &Option<crate::MemberWatch>) -> u8 {
		self.watching = true;
		let valid = watch.as_ref().filter(|(guild, channel, users)| {
			guild.0 != 0
				&& channel.0 != 0
				&& users.len() <= 128
				&& users.iter().all(|id| id.0 != 0)
				&& users.windows(2).all(|ids| ids[0] < ids[1])
		});
		let scope = valid.map(|(guild, channel, _)| (*guild, *channel));
		let mut canceled = 0;
		if self.following != scope {
			self.following = scope;
			self.users.clear();
			for slot in 0..2 {
				if self.active[slot]
					.as_ref()
					.is_some_and(|(r, _)| Some((r.guild, r.channel)) != scope)
				{
					self.active[slot] = None;
					canceled |= 1 << slot;
				}
				if self.queued[slot]
					.as_ref()
					.is_some_and(|r| Some((r.guild, r.channel)) != scope)
				{
					self.queued[slot] = None;
					canceled |= 1 << slot;
				}
			}
		}
		if let Some((_, _, users)) = valid
			&& self.users != *users
		{
			self.users.clone_from(users);
		}
		canceled
	}
	pub fn update(&mut self, requests: &[Option<Request>; 2]) -> u8 {
		let scope = if self.watching {
			self.following
		} else {
			requests
				.iter()
				.flatten()
				.filter(|r| r.valid())
				.max_by_key(|r| r.nonce)
				.map(|r| (r.guild, r.channel))
		};
		if self.following != scope {
			self.following = scope;
			self.users.clear();
		}
		for request in requests
			.iter()
			.flatten()
			.filter(|r| Some((r.guild, r.channel)) == scope && r.valid())
		{
			self.follow_users(request.users.iter().copied());
		}
		let mut changed = 0;
		for (slot, request) in requests.iter().enumerate() {
			if self.seen[slot] != request.as_ref().map(|request| request.nonce) {
				self.seen[slot] = request.as_ref().map(|request| request.nonce);
				self.active[slot] = None;
				self.queued[slot] = request.clone();
				changed |= 1 << slot;
			}
		}
		changed
	}
	fn follow_users(&mut self, users: impl IntoIterator<Item = Id>) {
		for user in users {
			self.users.retain(|known| *known != user);
			if self.users.len() == 128 {
				self.users.remove(0);
			}
			self.users.push(user);
		}
	}
	pub fn pending_subscription(&self) -> Option<(Id, Vec<Id>, Frame)> {
		let wanted = self
			.following
			.filter(|_| !self.users.is_empty())
			.map(|scope| scope.0);
		let (guild, users) = match (&self.announced, wanted) {
			(Some((old, _)), next) if Some(*old) != next => (*old, vec![]),
			(Some((_, sent)), Some(_)) if sent == &self.users => return None,
			(_, Some(guild)) => (guild, self.users.clone()),
			_ => return None,
		};
		// A sparse update leaves channel ranges and thread lists intact.
		let ids: Vec<_> = users.iter().map(ToString::to_string).collect();
		let mut setting = serde_json::json!({"members":ids});
		if !users.is_empty() {
			// Large guilds are not subscribed at READY. Member requests take effect only
			// after activating the guild, including when the member sidebar stays closed.
			setting["typing"] = true.into();
		}
		let frame = Frame::Text(
			serde_json::json!({"op":37,"d":{"subscriptions":{guild.to_string():setting}}})
				.to_string()
				.into(),
		);
		Some((guild, users, frame))
	}
	pub fn subscribed(&mut self, guild: Id, users: Vec<Id>) {
		self.announced = (!users.is_empty()).then_some((guild, users));
	}
	pub fn canceled_subscription(&mut self, guild: Id) {
		if self
			.announced
			.as_ref()
			.is_some_and(|(old, _)| *old == guild)
		{
			self.announced = None;
		}
	}
	pub fn report_cooldowns(
		&mut self,
		emit: &impl Fn(Event) -> Result<(), Failure>,
	) -> Result<(), Failure> {
		let now = Instant::now();
		for (slot, request) in self.queued.iter().enumerate() {
			if let Some(request) = request
				&& self.reported[slot] != Some(request.nonce)
				&& (self
					.cooldowns
					.get(&request.guild)
					.is_some_and(|at| *at > now)
					|| self.blocked_until.is_some_and(|at| at > now))
			{
				self.reported[slot] = Some(request.nonce);
				let until = self
					.cooldowns
					.get(&request.guild)
					.copied()
					.into_iter()
					.chain(self.blocked_until)
					.max()
					.unwrap_or(now);
				let seconds = until
					.saturating_duration_since(now)
					.as_millis()
					.div_ceil(1000)
					.min(u32::MAX as u128) as u32;
				emit(Event::MemberSearchCooldown {
					request: request.clone(),
					seconds,
				})?;
			}
		}
		Ok(())
	}
	pub fn deadline(&self) -> Option<Instant> {
		self.active
			.iter()
			.flatten()
			.filter_map(|(_, deadline)| *deadline)
			.chain(
				self.queued
					.iter()
					.flatten()
					.filter(|request| {
						!self.watching || self.following == Some((request.guild, request.channel))
					})
					.map(|request| {
						self.cooldowns
							.get(&request.guild)
							.copied()
							.unwrap_or(self.next_send)
							.max(self.next_send)
							.max(self.blocked_until.unwrap_or(self.next_send))
					}),
			)
			.min()
	}
	pub fn tick(
		&mut self,
		emit: &impl Fn(Event) -> Result<(), Failure>,
	) -> Result<Option<Query>, Failure> {
		self.tick_at(Instant::now(), emit)
	}
	fn tick_at(
		&mut self,
		now: Instant,
		emit: &impl Fn(Event) -> Result<(), Failure>,
	) -> Result<Option<Query>, Failure> {
		self.cooldowns.retain(|_, deadline| *deadline > now);
		for active in &mut self.active {
			if active
				.as_ref()
				.is_some_and(|(_, deadline)| deadline.is_some_and(|at| now >= at))
			{
				let (request, _) = active.take().unwrap();
				emit(Event::MemberSearch {
					request,
					result: Err(Failure::ProtocolAt("Member search timed out; try again")),
				})?;
			}
		}
		if now < self.next_send || self.blocked_until.is_some_and(|at| now < at) {
			return Ok(None);
		}
		let Some(slot) = self.queued.iter().position(|request| {
			request.as_ref().is_some_and(|request| {
				(!self.watching || self.following == Some((request.guild, request.channel)))
					&& self
						.cooldowns
						.get(&request.guild)
						.is_none_or(|at| *at <= now)
			})
		}) else {
			return Ok(None);
		};
		let request = self.queued[slot].take().unwrap();
		if !request.valid() {
			return Ok(None);
		}
		// Only two queued searches, and 64 retained unexpired guild cooldowns.
		// Capacity cannot erase an active cooldown and accidentally permit early traffic.
		if self.cooldowns.len() >= 64 && !self.cooldowns.contains_key(&request.guild) {
			self.blocked_until = self.cooldowns.values().copied().max();
			self.queued[slot] = Some(request);
			return Ok(None);
		}
		self.cooldowns
			.insert(request.guild, now + Duration::from_secs(30));
		let mut data = serde_json::json!({"guild_id":request.guild.to_string(),"limit":LIMIT,"nonce":request.nonce.to_string()});
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
		self.active[slot] = Some((request, None));
		self.next_send = now + Duration::from_secs(1);
		Ok(Some(Query {
			purpose: if slot == 0 {
				Purpose::Composer
			} else {
				Purpose::Authors
			},
			frame: Frame::Text(serde_json::json!({"op":8,"d":data}).to_string().into()),
		}))
	}
	pub fn sent(&mut self, guild: Id, nonce: u64, sent: Instant) {
		// Keep the service session's cooldown even when the old view was replaced
		// before this completion arrived. The writer knows its real send time.
		let until = sent + Duration::from_secs(30);
		if self.cooldowns.len() < 64 || self.cooldowns.contains_key(&guild) {
			self.cooldowns
				.entry(guild)
				.and_modify(|at| *at = (*at).max(until))
				.or_insert(until);
		} else {
			self.blocked_until = Some(self.blocked_until.map_or(until, |at| at.max(until)));
		}
		for (request, deadline) in self.active.iter_mut().flatten() {
			if request.nonce == nonce && deadline.is_none() {
				*deadline = Some(sent + Duration::from_secs(15));
			}
		}
	}
	pub fn limited(
		&mut self,
		guild: Option<Id>,
		nonce: Option<&str>,
		delay: Duration,
		emit: &impl Fn(Event) -> Result<(), Failure>,
	) -> Result<(), Failure> {
		let until = Instant::now() + delay;
		if let Some(guild) = guild {
			if self.cooldowns.len() < 64 || self.cooldowns.contains_key(&guild) {
				self.cooldowns
					.entry(guild)
					.and_modify(|at| *at = (*at).max(until))
					.or_insert(until);
			} else {
				self.blocked_until = Some(self.blocked_until.map_or(until, |at| at.max(until)));
			}
		} else {
			self.blocked_until = Some(self.blocked_until.map_or(until, |at| at.max(until)));
		}
		for active in &mut self.active {
			if active.as_ref().is_some_and(|(request, _)| {
				guild.is_none_or(|guild| guild == request.guild)
					&& nonce.is_none_or(|nonce| nonce == request.nonce.to_string())
			}) {
				let (request, _) = active.take().unwrap();
				emit(Event::MemberSearch {
					request,
					result: Err(Failure::RateLimited),
				})?;
			}
		}
		Ok(())
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
			let mut rows = Vec::with_capacity(chunk.members.len());
			let mut missing = Vec::new();
			for member in chunk.members {
				let fields = model::MissingMemberFields {
					user: member.user.id,
					global_name: model::GlobalNameUpdate::from_patch(&member.user.global_name),
					nick: matches!(member.nick, model::Patch::Absent),
					roles: matches!(member.roles, model::Patch::Absent),
				};
				let Some(member) = member.into_guild_member(request.guild) else {
					continue;
				};
				if fields.needs_metadata() {
					missing.push(fields);
				}
				rows.push(member);
			}
			if rows.iter().any(|m| !m.valid())
				|| rows.iter().map(model::Member::bytes).sum::<usize>()
					+ missing.capacity() * size_of::<model::MissingMemberFields>()
					> MAX_BYTES
			{
				return Err(Failure::Capacity);
			}
			Ok((rows, missing))
		})();
		if self.following == Some((request.guild, request.channel))
			&& let Ok((rows, _)) = &result
		{
			self.follow_users(rows.iter().map(|member| member.user.id));
		}
		Some(match result {
			Ok((rows, missing)) if !missing.is_empty() => Event::MemberSearchPartial {
				request,
				rows,
				missing,
			},
			Ok((rows, _)) => Event::MemberSearch {
				request,
				result: Ok(rows),
			},
			Err(error) => Event::MemberSearch {
				request,
				result: Err(error),
			},
		})
	}
}

#[cfg(debug_assertions)]
pub fn debug_check(request: Request) -> Event {
	let mut search = Search::default();
	search.queued[request.slot] = Some(request.clone());
	let frame = search.tick(&|_| Ok(())).unwrap().unwrap().frame;
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
	let user = request.users.first().copied().unwrap_or(Id(987654321));
	let mut chunk = serde_json::json!({"guild_id":request.guild.to_string(),"nonce":"wrong","chunk_index":0,"chunk_count":1,"members":[{"user":{"id":user.to_string(),"username":"remote-person","global_name":"remote-person"},"nick":"OutsideFirstHundred","roles":[]}]});
	assert!(search.chunk(&serde_json::to_vec(&chunk).unwrap()).is_none());
	chunk["nonce"] = serde_json::json!(request.nonce.to_string());
	let event = search.chunk(&serde_json::to_vec(&chunk).unwrap()).unwrap();
	assert!(
		matches!(&event, Event::MemberSearch { result: Ok(rows), .. } if rows.len() == 1 && rows[0].user.id == user)
	);
	assert!(search.chunk(&serde_json::to_vec(&chunk).unwrap()).is_none());
	search.next_send = Instant::now();
	search.cooldowns.clear();
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
	fn request(guild: u64, nonce: u64) -> Request {
		Request {
			guild: Id(guild),
			channel: Id(guild + 100),
			query: "synthetic".into(),
			users: vec![],
			nonce,
			slot: 0,
		}
	}
	#[test]
	fn member_queries_do_not_turn_omitted_fields_into_removals() {
		use model::GlobalNameUpdate::{Cleared, Set, Unchanged};
		for (member, global_name, missing_nick, missing_roles) in [
			(
				serde_json::json!({"user":{"id":"7","username":"account"},"roles":[]}),
				Unchanged,
				true,
				false,
			),
			(
				serde_json::json!({"user":{"id":"7","username":"account"},"nick":"Nickname"}),
				Unchanged,
				false,
				true,
			),
			(
				serde_json::json!({"user":{"id":"7","username":"account"},"nick":null,"roles":[]}),
				Unchanged,
				false,
				false,
			),
			(
				serde_json::json!({"user":{"id":"7","username":"account","global_name":null},"nick":null,"roles":[]}),
				Cleared,
				false,
				false,
			),
			(
				serde_json::json!({"user":{"id":"7","username":"account","global_name":"Display"},"nick":null,"roles":[]}),
				Set,
				false,
				false,
			),
		] {
			let mut search = Search::default();
			search.update(&[Some(request(1, 1)), None]);
			search.tick(&|_| Ok(())).unwrap().unwrap();
			let wire = serde_json::json!({"guild_id":"1","nonce":"1","chunk_index":0,"chunk_count":1,"members":[member]});
			match search.chunk(&serde_json::to_vec(&wire).unwrap()).unwrap() {
				Event::MemberSearch {
					result: Ok(rows), ..
				} if global_name == Set && !missing_nick && !missing_roles => {
					assert_eq!(rows.len(), 1);
					assert!(rows[0].nick.is_none() && rows[0].roles.is_empty());
				}
				Event::MemberSearchPartial { rows, missing, .. } => {
					assert_eq!(rows.len(), 1);
					assert_eq!(
						missing,
						vec![model::MissingMemberFields {
							global_name,
							user: Id(7),
							nick: missing_nick,
							roles: missing_roles
						}]
					);
				}
				_ => panic!("member result"),
			}
		}
	}
	#[test]
	fn mixed_wire_query_results_reach_core_with_live_updates_preserved() {
		use client_core::{Command, Envelope, State};
		use model::Patch;
		let mut state = State::default();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Ready {
				user: model::SavedAccount {
					id: Id(1),
					name: "Owner".into(),
					display: None,
					avatar: None,
					discriminator: 0,
					has_token: false,
				}
				.user(),
				guilds: vec![model::Guild {
					id: Id(10),
					name: "Server".into(),
					icon: None,
					emojis: None,
					stickers: None,
					default_message_notifications: None,
				}],
				channels: vec![model::Channel {
					id: Id(101),
					guild: Some(Id(10)),
					parent_id: None,
					position: 0,
					name: "chat".into(),
					kind: 0,
					recipients: vec![],
					member_list_id: None,
					message_count: None,
					tags: None,
					icon: None,
					last_message: None,
				}],
				permissions: model::permissions::Snapshot {
					guilds: vec![model::permissions::Guild {
						id: Id(10),
						owner: Some(Id(1)),
						roles: None,
						member: None,
					}],
					channels: vec![],
				},
			},
		});
		state.select(Id(101));
		let identity = |user, nick, roles| model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			guild: Id(10),
			user: Id(user),
			profile: None,
			avatar: Patch::Absent,
			nick,
			roles,
			departed: false,
		};
		for user in [7, 8, 9] {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::MemberIdentity(Box::new(identity(
					user,
					Patch::Value("Before".into()),
					Patch::Value(vec![Id(11)]),
				))),
			});
		}
		let mut profile = state.user.clone().unwrap();
		profile.id = Id(7);
		profile.name = "Known display".into();
		profile.username = Some("one".into());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::MemberIdentity(Box::new(model::MemberIdentity {
				global_name: model::GlobalNameUpdate::Set,
				guild: Id(10),
				user: Id(7),
				profile: Some(profile),
				avatar: Patch::Absent,
				nick: Patch::Absent,
				roles: Patch::Absent,
				departed: false,
			})),
		});
		let Some(Command::MemberSearch(request)) =
			state.request_author_members(&[Id(7), Id(8), Id(9)])
		else {
			panic!("author request")
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::MemberIdentity(Box::new(identity(
				7,
				Patch::Value("Live".into()),
				Patch::Value(vec![]),
			))),
		});
		let mut search = Search::default();
		search.update(&[None, Some(request.clone())]);
		search.tick(&|_| Ok(())).unwrap().unwrap();
		let wire = serde_json::json!({"guild_id":"10","nonce":request.nonce.to_string(),"chunk_index":0,"chunk_count":1,"members":[
			{"user":{"id":"7","username":"one"},"roles":["11"]},
			{"user":{"id":"8","username":"two"},"nick":"Current"},
			{"user":{"id":"9","username":"three"},"nick":null,"roles":[]}
		]});
		let event = search.chunk(&serde_json::to_vec(&wire).unwrap()).unwrap();
		assert!(matches!(event, Event::MemberSearchPartial { .. }));
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
		let rows = &state.member_search[1].rows;
		assert_eq!(rows.len(), 3);
		assert_eq!(rows[0].user.name, "Known display");
		assert_eq!(rows[0].nick.as_deref(), Some("Live"));
		assert!(rows[0].roles.is_empty());
		assert_eq!(rows[1].nick.as_deref(), Some("Current"));
		assert_eq!(rows[1].roles, vec![Id(11)]);
		assert!(rows[2].nick.is_none() && rows[2].roles.is_empty());

		let Some(Command::MemberSearch(next)) = state.search_members(Id(101), "one", 0) else {
			panic!("follow-up query");
		};
		let mut follow_up = Search::default();
		follow_up.update(&[Some(next.clone()), None]);
		follow_up.tick(&|_| Ok(())).unwrap().unwrap();
		let cleared = serde_json::json!({"guild_id":"10","nonce":next.nonce.to_string(),"chunk_index":0,"chunk_count":1,"members":[
			{"user":{"id":"7","username":"one","global_name":null},"nick":null,"roles":[]}
		]});
		state.apply(Envelope {
			generation: state.generation,
			event: follow_up
				.chunk(&serde_json::to_vec(&cleared).unwrap())
				.unwrap(),
		});
		let mut renamed = state.user.clone().unwrap();
		renamed.id = Id(7);
		renamed.name = "renamed".into();
		renamed.username = Some("renamed".into());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::MemberIdentity(Box::new(model::MemberIdentity {
				global_name: model::GlobalNameUpdate::Unchanged,
				guild: Id(10),
				user: Id(7),
				profile: Some(renamed),
				avatar: Patch::Absent,
				nick: Patch::Absent,
				roles: Patch::Absent,
				departed: false,
			})),
		});
		assert_eq!(state.member_search[0].rows[0].user.name, "renamed");

		let Some(Command::MemberSearch(stale)) = state.search_members(Id(101), "renamed", 0) else {
			panic!("stale query");
		};
		let mut newest = state.user.clone().unwrap();
		newest.id = Id(7);
		newest.name = "Newest display".into();
		newest.username = Some("renamed".into());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::MemberIdentity(Box::new(model::MemberIdentity {
				global_name: model::GlobalNameUpdate::Set,
				guild: Id(10),
				user: Id(7),
				profile: Some(newest),
				avatar: Patch::Absent,
				nick: Patch::Absent,
				roles: Patch::Absent,
				departed: false,
			})),
		});
		let mut stale_search = Search::default();
		stale_search.update(&[Some(stale.clone()), None]);
		stale_search.tick(&|_| Ok(())).unwrap().unwrap();
		let stale_wire = serde_json::json!({"guild_id":"10","nonce":stale.nonce.to_string(),"chunk_index":0,"chunk_count":1,"members":[
			{"user":{"id":"7","username":"renamed","global_name":null},"nick":null,"roles":[]}
		]});
		state.apply(Envelope {
			generation: state.generation,
			event: stale_search
				.chunk(&serde_json::to_vec(&stale_wire).unwrap())
				.unwrap(),
		});
		assert_eq!(state.member_search[0].rows[0].user.name, "Newest display");
		let mut renamed_again = state.user.clone().unwrap();
		renamed_again.id = Id(7);
		renamed_again.name = "account_two".into();
		renamed_again.username = Some("account_two".into());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::MemberIdentity(Box::new(model::MemberIdentity {
				global_name: model::GlobalNameUpdate::Unchanged,
				guild: Id(10),
				user: Id(7),
				profile: Some(renamed_again),
				avatar: Patch::Absent,
				nick: Patch::Absent,
				roles: Patch::Absent,
				departed: false,
			})),
		});
		assert_eq!(state.member_search[0].rows[0].user.name, "Newest display");
	}
	#[test]
	fn cached_member_watches_subscribe_without_a_query_and_cancel_old_scope() {
		let mut search = Search::default();
		search.watch(&Some((Id(1), Id(101), vec![Id(7), Id(8)])));
		assert!(search.tick(&|_| Ok(())).unwrap().is_none());
		let (guild, users, frame) = search.pending_subscription().unwrap();
		assert_eq!((guild, users.clone()), (Id(1), vec![Id(7), Id(8)]));
		let packet: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
		assert_eq!(packet["op"], 37);
		assert!(packet["d"]["subscriptions"]["1"].get("channels").is_none());
		search.subscribed(guild, users);
		search.watch(&Some((Id(1), Id(102), vec![Id(7), Id(8)])));
		assert!(search.pending_subscription().is_none());
		search.reconnect(true);
		search.watch(&Some((Id(1), Id(102), vec![Id(7), Id(8)])));
		assert_eq!(search.pending_subscription().unwrap().1, vec![Id(7), Id(8)]);
		search.subscribed(Id(1), vec![Id(7), Id(8)]);
		search.watch(&Some((Id(2), Id(202), vec![Id(9)])));
		let (old, users, _) = search.pending_subscription().unwrap();
		assert_eq!(old, Id(1));
		assert!(users.is_empty());
		search.subscribed(old, users);
		assert_eq!(search.pending_subscription().unwrap().0, Id(2));
		search.subscribed(Id(2), vec![Id(9)]);
		search.watch(&None);
		assert!(search.pending_subscription().unwrap().1.is_empty());
	}
	#[test]
	fn invalid_watches_cannot_retain_access_or_unbound_subscriptions() {
		let mut search = Search::default();
		for users in [
			vec![Id(0)],
			vec![Id(8), Id(7)],
			vec![Id(7), Id(7)],
			(1..=129).map(Id).collect(),
		] {
			search.watch(&Some((Id(1), Id(101), users)));
			assert!(search.pending_subscription().is_none());
			assert!(search.following.is_none());
		}
	}
	#[test]
	fn foreground_query_wins_queued_author_query_and_reports_real_cooldown() {
		let now = Instant::now();
		let mut search = Search {
			next_send: now,
			..Default::default()
		};
		let mut author = request(1, 1);
		author.slot = 1;
		author.query.clear();
		author.users = vec![Id(7)];
		search.update(&[Some(request(1, 2)), Some(author)]);
		assert_eq!(
			search.tick_at(now, &|_| Ok(())).unwrap().unwrap().purpose,
			Purpose::Composer
		);
		assert!(
			search
				.tick_at(now + Duration::from_secs(1), &|_| Ok(()))
				.unwrap()
				.is_none()
		);
		let reports = std::sync::Mutex::new(vec![]);
		search
			.report_cooldowns(&|event| {
				reports.lock().unwrap().push(event);
				Ok(())
			})
			.unwrap();
		assert!(matches!(
			reports.lock().unwrap().first(),
			Some(Event::MemberSearchCooldown {
				seconds: 1..=30,
				..
			})
		));
		search.update(&[None, None]);
		assert!(search.queued.iter().all(Option::is_none));
		assert!(
			search
				.tick_at(now + Duration::from_secs(30), &|_| Ok(()))
				.unwrap()
				.is_none()
		);
	}
	#[test]
	fn retired_watch_discards_query_before_send_without_losing_service_cooldown() {
		let now = Instant::now();
		let mut search = Search {
			next_send: now,
			..Default::default()
		};
		search.watch(&Some((Id(1), Id(101), vec![Id(7)])));
		search.update(&[Some(request(1, 1)), None]);
		search.tick_at(now, &|_| Ok(())).unwrap().unwrap();
		search.sent(Id(1), 1, now);
		assert_eq!(search.watch(&None), 1 << Purpose::Composer.slot());
		assert!(search.active.iter().all(Option::is_none));
		assert!(search.cooldowns.contains_key(&Id(1)));
		search.update(&[Some(request(1, 2)), None]);
		assert!(search.deadline().is_none());
		assert!(
			search
				.tick_at(now + Duration::from_secs(31), &|_| Ok(()))
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn resolved_members_receive_bounded_sparse_live_subscriptions() {
		let mut search = Search::default();
		let lookup = request(1, 1);
		search.update(&[Some(lookup), None]);
		assert!(search.pending_subscription().is_none());
		search.tick(&|_| Ok(())).unwrap().unwrap();
		let event = search.chunk(br#"{"guild_id":"1","nonce":"1","chunk_index":0,"chunk_count":1,"members":[{"user":{"id":"7","username":"account","global_name":"account"},"nick":"Nickname","roles":[]}]}"#);
		assert!(matches!(
			event,
			Some(Event::MemberSearch { result: Ok(_), .. })
		));
		let (guild, users, frame) = search.pending_subscription().unwrap();
		let packet: serde_json::Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
		assert_eq!(
			packet,
			serde_json::json!({"op":37,"d":{"subscriptions":{"1":{"typing":true,"members":["7"]}}}})
		);
		search.subscribed(guild, users);
		assert!(search.pending_subscription().is_none());
		search.follow_users((1..=1000).map(Id));
		assert_eq!(search.users.len(), 128);
		assert_eq!(search.users.first(), Some(&Id(873)));
		assert_eq!(search.users.last(), Some(&Id(1000)));
		assert!(search.pending_subscription().unwrap().2.len() < 4096);
	}
	#[test]
	fn identity_subscriptions_retire_and_resume_without_crossing_guilds() {
		let mut search = Search::default();
		let mut author = request(1, 1);
		author.slot = 1;
		author.query.clear();
		author.users = vec![Id(7)];
		search.update(&[None, Some(author.clone())]);
		let (guild, users, _) = search.pending_subscription().unwrap();
		search.subscribed(guild, users);
		search.reconnect(true);
		assert_eq!(search.pending_subscription().unwrap().1, vec![Id(7)]);
		search.subscribed(Id(1), vec![Id(7)]);
		author.guild = Id(2);
		author.channel = Id(102);
		author.nonce = 2;
		author.users = vec![Id(8)];
		search.update(&[None, Some(author)]);
		let (old, users, _) = search.pending_subscription().unwrap();
		assert_eq!(old, Id(1));
		assert!(users.is_empty());
		search.subscribed(old, users);
		let (guild, users, _) = search.pending_subscription().unwrap();
		assert_eq!((guild, users.clone()), (Id(2), vec![Id(8)]));
		search.subscribed(guild, users);
		search.canceled_subscription(Id(2));
		assert_eq!(search.pending_subscription().unwrap().1, vec![Id(8)]);
		search.reconnect(false);
		assert!(search.pending_subscription().is_none());
	}
	#[test]
	fn query_purpose_follows_its_slot_when_both_requests_encode_user_ids() {
		let mut search = Search::default();
		let now = Instant::now();
		search.next_send = now;
		let mut numeric = request(1, 1);
		numeric.query = "123456789".into();
		let mut authors = request(2, 2);
		authors.slot = 1;
		authors.query.clear();
		authors.users = vec![Id(123456789)];
		search.update(&[Some(numeric), Some(authors)]);
		for (offset, purpose) in [(0, Purpose::Composer), (1, Purpose::Authors)] {
			let query = search
				.tick_at(now + Duration::from_secs(offset), &|_| Ok(()))
				.unwrap()
				.unwrap();
			assert_eq!(query.purpose, purpose);
			let packet: serde_json::Value =
				serde_json::from_str(query.frame.to_text().unwrap()).unwrap();
			assert_eq!(packet["d"]["user_ids"], serde_json::json!(["123456789"]));
		}
	}
	#[test]
	fn per_guild_cooldown_retains_latest_query_and_uses_actual_send_time() {
		let now = Instant::now();
		let mut search = Search {
			next_send: now,
			..Default::default()
		};
		search.update(&[Some(request(1, 1)), None]);
		assert!(search.tick_at(now, &|_| Ok(())).unwrap().is_some());
		assert!(search.active[0].as_ref().unwrap().1.is_none());
		search.sent(Id(1), 1, now + Duration::from_secs(5));
		search.update(&[Some(request(1, 2)), None]);
		assert!(
			search
				.tick_at(now + Duration::from_secs(30), &|_| Ok(()))
				.unwrap()
				.is_none()
		);
		assert_eq!(search.queued[0].as_ref().unwrap().nonce, 2);
		assert!(
			search
				.tick_at(now + Duration::from_secs(35), &|_| Ok(()))
				.unwrap()
				.is_some()
		);
		assert_eq!(search.active[0].as_ref().unwrap().0.nonce, 2);
	}
	#[test]
	fn resume_and_fresh_identify_preserve_member_cooldowns() {
		let mut search = Search::default();
		let now = Instant::now();
		search.next_send = now;
		search.update(&[Some(request(1, 1)), None]);
		search.tick_at(now, &|_| Ok(())).unwrap();
		for (resumable, nonce) in [(true, 2), (false, 3)] {
			search.reconnect(resumable);
			search.update(&[Some(request(1, nonce)), None]);
			assert!(
				search
					.tick_at(now + Duration::from_secs(2), &|_| Ok(()))
					.unwrap()
					.is_none()
			);
		}
		assert!(
			search
				.tick_at(now + Duration::from_secs(30), &|_| Ok(()))
				.unwrap()
				.is_some()
		);
	}
	#[test]
	fn send_receipt_cannot_shorten_an_existing_service_penalty() {
		let mut search = Search::default();
		let now = search.next_send;
		search.update(&[Some(request(1, 1)), None]);
		search.tick_at(now, &|_| Ok(())).unwrap();
		search
			.limited(Some(Id(1)), Some("0"), Duration::from_secs(60), &|_| Ok(()))
			.unwrap();
		let until = search.cooldowns[&Id(1)];
		search.sent(Id(1), 1, now + Duration::from_secs(1));
		assert_eq!(search.cooldowns[&Id(1)], until);
		assert_eq!(
			search.active[0].as_ref().unwrap().1,
			Some(now + Duration::from_secs(16))
		);
	}
}
