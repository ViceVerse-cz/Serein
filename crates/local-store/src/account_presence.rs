use super::{LocalStore, Result, StoreError};
use model::{Id, MAX_SAVED_ACCOUNTS, OwnPresence, PresenceStatus};
use std::collections::BTreeMap;

impl LocalStore {
	pub fn account_presences(&self) -> Result<BTreeMap<Id, OwnPresence>> {
		let mut query = self.0.prepare(
			"SELECT account, status, CASE WHEN typeof(custom_status)='text' AND length(CAST(custom_status AS BLOB))<=512 THEN custom_status ELSE NULL END, expires FROM account_presence",
		)?;
		let mut rows = query.query([])?;
		let mut presences = BTreeMap::new();
		while presences.len() < MAX_SAVED_ACCOUNTS {
			let Some(row) = rows.next()? else {
				break;
			};
			let account: String = row.get(0)?;
			let status: String = row.get(1)?;
			let custom_status: Option<String> = row.get(2)?;
			let expires: Option<i64> = row.get(3)?;
			let Some(custom_status) = custom_status else {
				continue;
			};
			let Ok(account) = account.parse::<Id>() else {
				continue;
			};
			if account.0 == 0 {
				continue;
			}
			let Some(status) = PresenceStatus::parse(&status) else {
				continue;
			};
			let expires_at_ms = match expires {
				None => None,
				Some(expires) if expires > 0 => u64::try_from(expires).ok(),
				Some(_) => continue,
			};
			let presence = OwnPresence {
				status,
				custom_status,
				expires_at_ms,
			};
			if !presence.custom_status.is_empty() && !presence.valid() {
				continue;
			}
			presences.insert(account, presence);
		}
		Ok(presences)
	}

	pub fn save_account_presence(&self, account: Id, presence: &OwnPresence) -> Result<()> {
		if account.0 == 0 || !presence.valid() {
			return Err(StoreError::Capacity);
		}
		let expires = match presence.expires_at_ms {
			None => None,
			Some(expires) => Some(i64::try_from(expires).map_err(|_| StoreError::Capacity)?),
		};
		self.0.execute(
			"INSERT INTO account_presence(account,status,custom_status,expires) VALUES(?1,?2,?3,?4) ON CONFLICT(account) DO UPDATE SET status=excluded.status, custom_status=excluded.custom_status, expires=excluded.expires",
			rusqlite::params![
				account.to_string(),
				presence.status.wire(),
				presence.custom_status,
				expires
			],
		)?;
		Ok(())
	}
}
