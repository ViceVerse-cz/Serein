//! Shared session snapshot budgets; allocations grow with received data, never with the ceiling.
pub const MAX_ENTRIES: usize = 131_072;
pub const MAX_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_PERMISSION_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ROLES: usize = 131_072;
pub const MAX_OVERWRITES: usize = 1_048_576;

/// Only feature categories cross the diagnostics boundary, never payloads or parser errors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Warnings {
	pub read_state: bool,
	pub notifications: bool,
	pub sessions: bool,
	pub presence: bool,
	pub emojis: bool,
	pub stickers: bool,
}
