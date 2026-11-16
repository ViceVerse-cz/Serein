//! Local slowmode cooldowns. Discord sends no deadline with a channel; the wait starts when
//! the service confirms this account's message and runs for the channel's documented
//! `rate_limit_per_user`.
use crate::State;
use model::{Id, permissions as p};
use std::time::{Duration, Instant};

/// Channels waited on at once; the oldest send makes room beyond it.
const MAX_COOLDOWNS: usize = 64;
/// Documented ceiling of `rate_limit_per_user`; an older send binds nothing.
const LONGEST: Duration = Duration::from_secs(21600);

/// When this account last sent in each slowmode channel. The wait is measured from there
/// against the channel's current limit, so a changed limit applies at once.
#[derive(Default)]
pub struct Cooldowns(Vec<(Id, Instant)>);

impl State {
	/// Slowmode seconds binding this account in `channel`; `None` when off or bypassed.
	/// Documented: only BYPASS_SLOWMODE (and administrators) skip the wait.
	pub fn slowmode(&self, channel: Id) -> Option<u16> {
		let seconds = self.channel(channel)?.slowmode;
		(seconds > 0 && self.permission(channel, p::BYPASS_SLOWMODE) != Some(true))
			.then_some(seconds)
	}
	/// Time left before this account may send in `channel` again.
	pub fn slowmode_remaining(&self, channel: Id, now: Instant) -> Option<Duration> {
		let seconds = self.slowmode(channel)?;
		let (_, sent) = self
			.slowmode_cooldowns
			.0
			.iter()
			.find(|(id, _)| *id == channel)?;
		(*sent + Duration::from_secs(seconds.into()))
			.checked_duration_since(now)
			.filter(|left| !left.is_zero())
	}
	pub(crate) fn start_slowmode(&mut self, channel: Id) {
		if self.slowmode(channel).is_none() {
			return;
		}
		let now = Instant::now();
		let sends = &mut self.slowmode_cooldowns.0;
		sends.retain(|(id, sent)| *id != channel && now.duration_since(*sent) < LONGEST);
		if sends.len() >= MAX_COOLDOWNS
			&& let Some(oldest) = (0..sends.len()).min_by_key(|index| sends[*index].1)
		{
			sends.swap_remove(oldest);
		}
		sends.push((channel, now));
	}
}
