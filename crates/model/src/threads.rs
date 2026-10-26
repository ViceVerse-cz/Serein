//! Session-only thread facts the channel list needs: the account's membership and the
//! service's archive metadata. Unofficial: shapes follow Discord's Gateway thread objects.

/// The current account's membership of one thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member {
	/// When the account joined, in Unix milliseconds; orders the channel-list rows.
	pub joined: i64,
	pub muted: bool,
	/// End of a timed mute in Unix milliseconds; `None` mutes until changed.
	pub mute_end: Option<i64>,
}
impl Member {
	pub fn muted_at(&self, now: i64) -> bool {
		self.muted && self.mute_end.is_none_or(|end| end > now)
	}
}

/// Archive metadata of an active thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
	/// Inactivity in minutes after which the service archives the thread.
	pub auto_archive_minutes: u32,
	/// Latest non-message activity (`archive_timestamp`) in Unix milliseconds.
	pub activity: i64,
}

/// One thread's update. Absent membership keeps the known one; `Null` means the account left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
	pub id: crate::Id,
	pub member: crate::Patch<Member>,
	pub meta: Option<Meta>,
	/// Pinned to the top of its forum; `None` when the update carried no `flags`.
	pub pinned: Option<bool>,
}

/// What a thread row needs, as last reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
	pub member: Option<Member>,
	pub meta: Option<Meta>,
	pub pinned: bool,
}
impl Info {
	pub fn apply(&mut self, change: &Change) {
		match change.member {
			crate::Patch::Value(member) => self.member = Some(member),
			crate::Patch::Null => self.member = None,
			crate::Patch::Absent => {}
		}
		if change.meta.is_some() {
			self.meta = change.meta;
		}
		if let Some(pinned) = change.pinned {
			self.pinned = pinned;
		}
	}
}

/// Bit 1 of a thread's `flags` pins it in its forum.
pub fn pinned(flags: u64) -> bool {
	flags & (1 << 1) != 0
}

/// Unix milliseconds encoded in a Discord snowflake.
pub fn snowflake_ms(id: crate::Id) -> i64 {
	((id.0 >> 22) + 1_420_070_400_000) as i64
}
