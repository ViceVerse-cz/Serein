//! Frequently used emoji kept in the account's unofficial frecency settings: one ranking for
//! messages and one for reactions, read with the GIF favorites. Uses are batched and saved at
//! most once per [`SAVE_DELAY`]; a failed save is dropped, never replayed.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const MAX_FREQUENT_EMOJI: usize = 32;
pub const MAX_EMOJI_KEY: usize = 64;
/// Distinct emoji awaiting a save; later new ones wait for the next batch.
const MAX_PENDING: usize = 64;
/// Use times kept per pending emoji; the service keeps only the newest few.
const MAX_TIMES: usize = 10;
pub const SAVE_DELAY: Duration = Duration::from_secs(15);

/// A custom emoji id or a Unicode emoji's Discord name.
pub fn valid_key(key: &str) -> bool {
	!key.is_empty() && key.len() <= MAX_EMOJI_KEY && !key.chars().any(char::is_control)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiUse {
	pub key: String,
	pub reaction: bool,
	pub count: u32,
	/// Unix milliseconds of the newest uses.
	pub times: Vec<u64>,
}

/// Keys most used first, as the service ranks them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranking {
	pub messages: Vec<String>,
	pub reactions: Vec<String>,
}
impl Ranking {
	pub fn bytes(&self) -> usize {
		[&self.messages, &self.reactions]
			.into_iter()
			.map(|keys| {
				keys.capacity() * size_of::<String>()
					+ keys.iter().map(String::capacity).sum::<usize>()
			})
			.sum()
	}
	fn valid(&self) -> bool {
		[&self.messages, &self.reactions]
			.into_iter()
			.all(|keys| keys.len() <= MAX_FREQUENT_EMOJI && keys.iter().all(|key| valid_key(key)))
	}
}

#[derive(Default)]
pub struct EmojiFrecency {
	pub ranking: Ranking,
	pending: Vec<EmojiUse>,
	pending_since: Option<Instant>,
	request: u64,
	in_flight: Option<u64>,
}

impl State {
	/// Counts one use toward the account's ranking; saved with the next batch.
	pub fn record_emoji_use(&mut self, key: &str, reaction: bool) {
		if !valid_key(key) {
			return;
		}
		let frecency = &mut self.emoji_frecency;
		let index = match frecency
			.pending
			.iter()
			.position(|used| used.key == key && used.reaction == reaction)
		{
			Some(index) => index,
			None if frecency.pending.len() < MAX_PENDING => {
				frecency.pending.push(EmojiUse {
					key: key.to_owned(),
					reaction,
					count: 0,
					times: Vec::with_capacity(MAX_TIMES),
				});
				frecency.pending.len() - 1
			}
			None => return,
		};
		let used = &mut frecency.pending[index];
		used.count = used.count.saturating_add(1);
		if used.times.len() == MAX_TIMES {
			used.times.remove(0);
		}
		used.times.push(
			SystemTime::now()
				.duration_since(UNIX_EPOCH)
				.map_or(0, |since| since.as_millis() as u64),
		);
		frecency.pending_since.get_or_insert_with(Instant::now);
	}
	/// One batched save once uses have waited [`SAVE_DELAY`], never beside a GIF favorites sync,
	/// since both write the same versioned settings.
	pub fn take_emoji_frecency_command(&mut self, now: Instant) -> Option<Command> {
		let frecency = &mut self.emoji_frecency;
		if frecency.in_flight.is_some()
			|| frecency
				.pending_since
				.is_none_or(|since| now.duration_since(since) < SAVE_DELAY)
			|| !(self.auth == AuthState::Authenticated && self.gateway_connected)
			|| self.gifs.sync_pending.is_some()
		{
			return None;
		}
		frecency.request = frecency.request.wrapping_add(1);
		frecency.in_flight = Some(frecency.request);
		frecency.pending_since = None;
		Some(Command::EmojiFrecency {
			request: frecency.request,
			uses: std::mem::take(&mut frecency.pending),
		})
	}
	/// When the waiting uses become due, so an idle host can wake for the save.
	pub fn emoji_frecency_due(&self) -> Option<Instant> {
		self.emoji_frecency
			.pending_since
			.map(|since| since + SAVE_DELAY)
	}
	pub fn emoji_frecency_saving(&self) -> bool {
		self.emoji_frecency.in_flight.is_some()
	}
	/// Call before the host replaces or shuts down its account connection.
	pub fn interrupt_emoji_frecency(&mut self) {
		self.emoji_frecency.in_flight = None;
	}
	pub(crate) fn apply_emoji_ranking(&mut self, ranking: Ranking) {
		if ranking.valid() {
			self.emoji_frecency.ranking = ranking;
		}
	}
	pub(crate) fn apply_emoji_frecency(&mut self, request: u64, result: Result<Ranking, Failure>) {
		if self.emoji_frecency.in_flight != Some(request) {
			return;
		}
		self.emoji_frecency.in_flight = None;
		if let Ok(ranking) = result {
			self.apply_emoji_ranking(ranking);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn uses_batch_after_the_delay_and_a_save_never_overlaps_or_replays() {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		};
		for _ in 0..12 {
			state.record_emoji_use("joy", false);
		}
		state.record_emoji_use("joy", true);
		state.record_emoji_use("", false);
		for index in 0..MAX_PENDING {
			state.record_emoji_use(&index.to_string(), false);
		}
		let start = Instant::now();
		assert!(state.take_emoji_frecency_command(start).is_none());
		let later = start + SAVE_DELAY + Duration::from_secs(1);
		let Some(Command::EmojiFrecency { request, uses }) =
			state.take_emoji_frecency_command(later)
		else {
			panic!("batched save");
		};
		assert_eq!(uses.len(), MAX_PENDING);
		assert_eq!((uses[0].count, uses[0].times.len()), (12, MAX_TIMES));
		assert!(uses[1].reaction);
		// One save at a time; uses recorded meanwhile wait for the next batch.
		state.record_emoji_use("skull", false);
		assert!(
			state
				.take_emoji_frecency_command(later + SAVE_DELAY * 2)
				.is_none()
		);
		state.apply_emoji_frecency(request.wrapping_add(1), Ok(Ranking::default()));
		assert!(state.emoji_frecency_saving());
		state.apply_emoji_frecency(request, Err(Failure::Protocol));
		let Some(Command::EmojiFrecency { uses, .. }) =
			state.take_emoji_frecency_command(later + SAVE_DELAY * 2)
		else {
			panic!("next batch");
		};
		assert_eq!(uses.len(), 1);
		assert_eq!(uses[0].key, "skull");
		// A command the connection refuses must not leave saves or GIF writes blocked.
		let rejected = Command::EmojiFrecency {
			request: state.emoji_frecency.request,
			uses,
		};
		state.command_rejected(rejected);
		assert!(!state.emoji_frecency_saving());
	}
}
