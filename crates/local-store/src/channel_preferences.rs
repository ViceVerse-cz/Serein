use super::{LocalStore, Result, StoreError};
use model::{ChannelPreferences, Id};
use rusqlite::{OptionalExtension, params};

impl LocalStore {
	pub fn channel_preferences(&self, account: Id) -> Result<ChannelPreferences> {
		let value: Option<Option<String>> = self.0.query_row(
			"SELECT CASE WHEN typeof(value)='text' AND length(CAST(value AS BLOB))<=8192 THEN value ELSE NULL END FROM channel_preferences WHERE account=?1",
			[account.to_string()], |row| row.get(0)).optional()?;
		let value = match value {
			None => ChannelPreferences::default(),
			Some(None) => return Err(StoreError::Incompatible),
			Some(Some(value)) => serde_json::from_str::<ChannelPreferences>(&value)
				.map_err(|_| StoreError::Incompatible)?,
		};
		if !value.is_valid() {
			return Err(StoreError::Incompatible);
		}
		Ok(value)
	}

	pub fn save_channel_preferences(&self, account: Id, value: &ChannelPreferences) -> Result<()> {
		if !value.is_valid() {
			return Err(StoreError::Capacity);
		}
		let value = serde_json::to_string(value).map_err(|_| StoreError::Incompatible)?;
		if value.len() > ChannelPreferences::MAX_JSON_BYTES {
			return Err(StoreError::Capacity);
		}
		self.0.execute(
			"INSERT INTO channel_preferences(account,value) VALUES(?1,?2) ON CONFLICT(account) DO UPDATE SET value=excluded.value",
			params![account.to_string(), value],
		)?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::{PreferenceEdit, Shortcut};

	#[test]
	fn channel_shortcuts_round_trip_are_bounded_and_cleared_only_for_the_account() {
		let mut store =
			LocalStore::initialize(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
		let mut value = ChannelPreferences::default();
		assert_eq!(
			value.set(Shortcut::Favorite, Id(0), true),
			PreferenceEdit::Unchanged
		);
		assert_eq!(
			value.set(Shortcut::Favorite, Id(9), true),
			PreferenceEdit::Changed
		);
		assert_eq!(
			value.set(Shortcut::Pinned, Id(9), true),
			PreferenceEdit::Changed
		);
		store.save_channel_preferences(Id(1), &value).unwrap();
		store.save_channel_preferences(Id(2), &value).unwrap();
		assert_eq!(store.channel_preferences(Id(1)).unwrap(), value);
		assert_eq!(
			store.channel_preferences(Id(3)).unwrap(),
			ChannelPreferences::default()
		);
		assert_eq!(
			value.set(Shortcut::Favorite, Id(9), false),
			PreferenceEdit::Changed
		);
		for id in 1..ChannelPreferences::MAX_ENTRIES as u64 {
			assert_eq!(
				value.set(Shortcut::Favorite, Id(id), true),
				PreferenceEdit::Changed
			);
		}
		assert_eq!(
			value.set(Shortcut::Pinned, Id(10), true),
			PreferenceEdit::CapacityReached
		);
		assert_eq!(value.favorites[0], Id(255));
		store.save_channel_preferences(Id(1), &value).unwrap();
		assert!(serde_json::to_vec(&value).unwrap().len() <= ChannelPreferences::MAX_JSON_BYTES);
		value.favorites.push(Id(999));
		assert_eq!(
			store.save_channel_preferences(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.favorites.pop();
		value.favorites[0] = value.favorites[1];
		assert_eq!(
			store.save_channel_preferences(Id(1), &value),
			Err(StoreError::Capacity)
		);
		store.forget_account(Id(1)).unwrap();
		assert_eq!(
			store.channel_preferences(Id(1)).unwrap(),
			ChannelPreferences::default()
		);
		assert_eq!(
			store.channel_preferences(Id(2)).unwrap().favorites,
			vec![Id(9)]
		);
		store
			.0
			.execute(
				"UPDATE channel_preferences SET value=?1 WHERE account='2'",
				[r#"{"favorites":["9","9"],"pinned":[]}"#],
			)
			.unwrap();
		assert_eq!(
			store.channel_preferences(Id(2)),
			Err(StoreError::Incompatible)
		);
		// Bound reads even if an externally modified database bypassed its SQL constraints.
		store
			.0
			.execute_batch("PRAGMA ignore_check_constraints=ON;")
			.unwrap();
		store
			.0
			.execute(
				"UPDATE channel_preferences SET value=?1 WHERE account='2'",
				["x".repeat(ChannelPreferences::MAX_JSON_BYTES + 1)],
			)
			.unwrap();
		assert_eq!(
			store.channel_preferences(Id(2)),
			Err(StoreError::Incompatible)
		);
	}
}
