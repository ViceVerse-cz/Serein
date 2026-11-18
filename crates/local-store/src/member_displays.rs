//! Bounded, account-isolated presentation hints. No permission, presence or role state is stored.
use super::{Id, LocalStore, OptionalExtension, Result, StoreError, params};
use model::MemberDisplay;
use std::collections::BTreeSet;

const LIMIT: usize = 4096;
const DISK_LIMIT: usize = 250_000;
const DISK_BYTES: usize = 256 * 1024 * 1024;
const MAX_BYTES: usize = 2 * 1024 * 1024;
// Includes IDs, the DTO, per-row bookkeeping and field allocations. Allocated SQLite
// pages and journals also share the application-wide cache budget.
const CHARGE: usize = 256;
const ROW_CHARGE: &str = "256+length(CAST(name AS BLOB))+coalesce(length(CAST(username AS BLOB)),0)+coalesce(length(CAST(nick AS BLOB)),0)+length(CAST(search_key AS BLOB))";

impl LocalStore {
	/// Sparse authoritative display updates; callers serialize them with departures/cleanup.
	pub fn save_member_displays(&mut self, account: Id, entries: &[MemberDisplay]) -> Result<()> {
		self.save_member_displays_with_limits(account, entries, DISK_LIMIT, DISK_BYTES)
	}
	fn save_member_displays_with_limits(
		&mut self,
		account: Id,
		entries: &[MemberDisplay],
		disk_limit: usize,
		disk_bytes: usize,
	) -> Result<()> {
		let mut seen = BTreeSet::new();
		if account.0 == 0
			|| entries.len() > LIMIT
			|| entries
				.iter()
				.any(|entry| !entry.valid() || !seen.insert((entry.guild, entry.user)))
			|| entries
				.iter()
				.map(|entry| {
					CHARGE
						+ entry.name.len() + entry.username.as_deref().map_or(0, str::len)
						+ entry.nick.as_ref().map_or(0, String::len)
				})
				.sum::<usize>()
				> MAX_BYTES
		{
			return Err(StoreError::Capacity);
		}
		if entries.is_empty() {
			return Ok(());
		}
		let transaction = self.0.transaction()?;
		let (mut count, mut bytes, touched): (i64, i64, i64) = transaction.query_row(
			"SELECT count,bytes,(SELECT coalesce(max(touched),0) FROM member_displays) FROM member_cache_totals WHERE singleton=1",
			[],
			|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
		)?;
		let touched = touched.checked_add(1).ok_or(StoreError::Incompatible)?;
		let account = account.to_string();
		// A snapshot can contain hundreds of peers, most of them unchanged. Prepare
		// the lookup once for the transaction rather than once per peer.
		let mut lookup = transaction.prepare(&format!(
			"SELECT {ROW_CHARGE},name=?4 AND username IS ?5 AND nick IS ?6 FROM member_displays WHERE account=?1 AND guild=?2 AND user=?3"
		))?;
		for entry in entries {
			let guild = entry.guild.to_string();
			let user = entry.user.to_string();
			let old: Option<(i64, bool)> = lookup
				.query_row(
					params![
						account,
						guild,
						user,
						entry.name,
						entry.username.as_deref(),
						entry.nick
					],
					|row| Ok((row.get(0)?, row.get(1)?)),
				)
				.optional()?;
			if let Some((_, true)) = old {
				// A snapshot can contain hundreds of unchanged peers after one identity changes.
				// Keep this read-only; eviction recency follows changed observations.
				continue;
			}
			if let Some((old_bytes, _)) = old {
				transaction.execute(
					"DELETE FROM member_displays WHERE account=?1 AND guild=?2 AND user=?3",
					params![account, guild, user],
				)?;
				count -= 1;
				bytes -= old_bytes;
			}
			let search_key = member_search_key(
				&entry.name,
				entry.username.as_deref(),
				entry.nick.as_deref(),
			);
			let charge = (CHARGE
				+ entry.name.len()
				+ entry.username.as_deref().map_or(0, str::len)
				+ entry.nick.as_ref().map_or(0, String::len)
				+ search_key.len()) as i64;
			while count >= disk_limit as i64 || bytes + charge > disk_bytes as i64 {
				let (a,g,u,old_bytes):(String,String,String,i64) = transaction.query_row(
					&format!("SELECT account,guild,user,{ROW_CHARGE} FROM member_displays ORDER BY touched,account,guild,user LIMIT 1"),
					[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
				)?;
				transaction.execute(
					"DELETE FROM member_displays WHERE account=?1 AND guild=?2 AND user=?3",
					params![a, g, u],
				)?;
				count -= 1;
				bytes -= old_bytes;
			}
			transaction.execute("INSERT INTO member_displays(account,guild,user,name,username,nick,touched,search_key) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![account,guild,user,entry.name,entry.username.as_deref(),entry.nick,touched,search_key])?;
			count += 1;
			bytes += charge;
		}
		drop(lookup);
		transaction.commit()?;
		Ok(())
	}
	/// Display hints only. Current membership/access must be checked before hydrating them.
	pub fn load_member_displays(&self, account: Id, guild: Id) -> Result<Vec<MemberDisplay>> {
		if account.0 == 0 || guild.0 == 0 {
			return Err(StoreError::Capacity);
		}
		self.read_member_displays(
			account,
			guild,
			" ORDER BY touched DESC,user LIMIT 4096",
			Vec::new(),
		)
	}
	/// The initial presentation overlay holds at most 128 disk hints.
	pub fn load_recent_member_displays(
		&self,
		account: Id,
		guild: Id,
	) -> Result<Vec<MemberDisplay>> {
		if account.0 == 0 || guild.0 == 0 {
			return Err(StoreError::Capacity);
		}
		self.read_member_displays(
			account,
			guild,
			" ORDER BY touched DESC,user LIMIT 128",
			Vec::new(),
		)
	}
	/// Resolve only referenced people without loading a server's complete directory.
	pub fn load_member_displays_for_users(
		&self,
		account: Id,
		guild: Id,
		users: &[Id],
	) -> Result<Vec<MemberDisplay>> {
		if users.len() > 100 || users.iter().any(|id| id.0 == 0) {
			return Err(StoreError::Capacity);
		}
		if users.is_empty() {
			return Ok(Vec::new());
		}
		let placeholders = (0..users.len())
			.map(|index| format!("?{}", index + 3))
			.collect::<Vec<_>>()
			.join(",");
		self.read_member_displays(
			account,
			guild,
			&format!(" AND user IN({placeholders}) ORDER BY touched DESC,user LIMIT 100"),
			users.iter().map(ToString::to_string).collect(),
		)
	}
	/// Presentation-only local search. Results never establish membership or permissions.
	pub fn search_member_displays(
		&self,
		account: Id,
		guild: Id,
		query: &str,
		limit: usize,
	) -> Result<Vec<MemberDisplay>> {
		let query = query.trim();
		if query.is_empty()
			|| query.len() > 256
			|| query.chars().count() > 64
			|| query.chars().any(char::is_control)
			|| limit == 0
			|| limit > 100
		{
			return Err(StoreError::Capacity);
		}
		if let Ok(user) = query.parse::<Id>() {
			return self.read_member_displays(
				account,
				guild,
				" AND user=?3",
				vec![user.to_string()],
			);
		}
		self.read_member_displays(
			account,
			guild,
			&format!(" AND instr(search_key,?3)>0 ORDER BY touched DESC,user LIMIT {limit}"),
			vec![query.to_lowercase()],
		)
	}
	fn read_member_displays(
		&self,
		account: Id,
		guild: Id,
		suffix: &str,
		extra: Vec<String>,
	) -> Result<Vec<MemberDisplay>> {
		if account.0 == 0 || guild.0 == 0 {
			return Err(StoreError::Capacity);
		}
		let sql = format!(
			"SELECT user,name,username,nick FROM member_displays WHERE account=?1 AND guild=?2{suffix}"
		);
		let mut statement = self.0.prepare_cached(&sql)?;
		let mut values = vec![account.to_string(), guild.to_string()];
		values.extend(extra);
		let mut rows = statement.query(rusqlite::params_from_iter(values))?;
		let mut entries = Vec::new();
		let mut bytes = 0;
		while let Some(row) = rows.next()? {
			if entries.len() == LIMIT {
				return Err(StoreError::Capacity);
			}
			for (column, maximum) in [(0, 20), (1, 512), (2, 512), (3, 512)] {
				let value = row.get_ref(column)?;
				if !matches!(value, rusqlite::types::ValueRef::Null) {
					let value = value.as_str().map_err(|_| StoreError::Incompatible)?;
					if value.len() > maximum {
						return Err(StoreError::Capacity);
					}
				}
			}
			let entry = MemberDisplay {
				guild,
				user: row
					.get_ref(0)?
					.as_str()
					.map_err(|_| StoreError::Incompatible)?
					.parse()
					.map_err(|_| StoreError::Incompatible)?,
				name: row.get(1)?,
				username: row.get::<_, Option<String>>(2)?.map(String::into_boxed_str),
				nick: row.get(3)?,
			};
			if !entry.valid() {
				return Err(StoreError::Incompatible);
			}
			bytes += CHARGE
				+ entry.name.len()
				+ entry.username.as_deref().map_or(0, str::len)
				+ entry.nick.as_ref().map_or(0, String::len);
			if bytes > MAX_BYTES {
				break;
			}
			entries.push(entry);
		}
		Ok(entries)
	}
	pub fn remove_member_display(&mut self, account: Id, guild: Id, user: Id) -> Result<()> {
		self.remove_member_displays(account, guild, &[user])
	}
	/// Confirmed departures/404s only; omission from a partial lookup is not a departure.
	pub fn remove_member_displays(&mut self, account: Id, guild: Id, users: &[Id]) -> Result<()> {
		if account.0 == 0 || guild.0 == 0 || users.len() > 100 || users.iter().any(|id| id.0 == 0) {
			return Err(StoreError::Capacity);
		}
		let mut seen = BTreeSet::new();
		let mut query = self.0.prepare_cached(&format!(
			"SELECT {ROW_CHARGE} FROM member_displays WHERE account=?1 AND guild=?2 AND user=?3"
		))?;
		let mut bytes = 0i64;
		for user in users {
			if !seen.insert(*user) {
				continue;
			}
			let charge: Option<i64> = query
				.query_row(
					params![account.to_string(), guild.to_string(), user.to_string()],
					|row| row.get(0),
				)
				.optional()?;
			let charge = charge.unwrap_or(0);
			if charge < 0 {
				return Err(StoreError::Incompatible);
			}
			bytes = bytes.saturating_add(charge);
			if bytes > crate::cache_cleanup::CLEANUP_BYTES {
				return Err(StoreError::Capacity);
			}
		}
		drop(query);
		self.delete_member_display_batch(account, guild, &seen.into_iter().collect::<Vec<_>>())?;
		Ok(())
	}
	/// Reconcile a completed authoritative roster. Callers must never invoke this
	/// after cancellation, a missing chunk or a partial/limited directory result.
	pub fn reconcile_member_displays(
		&mut self,
		account: Id,
		guild: Id,
		users: &[Id],
	) -> Result<usize> {
		if account.0 == 0
			|| guild.0 == 0
			|| users.len() > DISK_LIMIT
			|| users.iter().any(|id| id.0 == 0)
			|| users.windows(2).any(|ids| ids[0] >= ids[1])
		{
			return Err(StoreError::Capacity);
		}
		let account_text = account.to_string();
		let guild_text = guild.to_string();
		let mut cursor = None::<String>;
		let mut removed = 0;
		loop {
			// `user` is stored as decimal TEXT. Seek by the primary key's exact
			// lexical order; numeric membership lookup uses the sorted typed roster.
			let mut values = vec![account_text.clone(), guild_text.clone()];
			let suffix = if let Some(cursor) = &cursor {
				values.push(cursor.clone());
				" AND user>?3"
			} else {
				""
			};
			let mut query = self.0.prepare_cached(&format!("SELECT user,{ROW_CHARGE} FROM member_displays WHERE account=?1 AND guild=?2{suffix} ORDER BY user LIMIT 128"))?;
			let mut rows = query.query(rusqlite::params_from_iter(values))?;
			let mut victims = Vec::new();
			let mut bytes = 0i64;
			let mut seen = 0;
			while let Some(row) = rows.next()? {
				let text = row
					.get_ref(0)?
					.as_str()
					.map_err(|_| StoreError::Incompatible)?;
				if text.is_empty() || text.len() > 20 {
					return Err(StoreError::Incompatible);
				}
				let user = text.parse::<Id>().map_err(|_| StoreError::Incompatible)?;
				if user.0 == 0 {
					return Err(StoreError::Incompatible);
				}
				if users.binary_search(&user).is_err() {
					let charge: i64 = row.get(1)?;
					if charge < 0 {
						return Err(StoreError::Incompatible);
					}
					if charge > crate::cache_cleanup::CLEANUP_BYTES {
						return Err(StoreError::Capacity);
					}
					if !victims.is_empty()
						&& bytes.saturating_add(charge) > crate::cache_cleanup::CLEANUP_BYTES
					{
						break;
					}
					bytes = bytes.saturating_add(charge);
					victims.push(user);
				}
				cursor = Some(text.to_owned());
				seen += 1;
			}
			drop(rows);
			drop(query);
			if seen == 0 {
				return Ok(removed);
			}
			removed += self.delete_member_display_batch(account, guild, &victims)?;
		}
	}
	fn delete_member_display_batch(
		&mut self,
		account: Id,
		guild: Id,
		users: &[Id],
	) -> Result<usize> {
		if users.is_empty() {
			return Ok(0);
		}
		let transaction = self.0.transaction()?;
		let mut removed = 0;
		{
			let mut delete = transaction.prepare_cached(
				"DELETE FROM member_displays WHERE account=?1 AND guild=?2 AND user=?3",
			)?;
			for user in users {
				removed += delete.execute(params![
					account.to_string(),
					guild.to_string(),
					user.to_string()
				])?;
			}
		}
		transaction.commit()?;
		if removed > 0 {
			self.reclaim_history_space()?;
		}
		Ok(removed)
	}
	pub fn clear_member_guild(&mut self, account: Id, guild: Id) -> Result<()> {
		if account.0 == 0 || guild.0 == 0 {
			return Err(StoreError::Capacity);
		}
		self.delete_cache_rows("member_displays", account, Some(("guild", guild)))?;
		Ok(())
	}
}

fn member_search_key(name: &str, username: Option<&str>, nick: Option<&str>) -> String {
	format!(
		"{}\n{}\n{}",
		name.to_lowercase(),
		username.unwrap_or_default().to_lowercase(),
		nick.unwrap_or_default().to_lowercase()
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use rusqlite::Connection;
	fn store() -> LocalStore {
		LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap()
	}
	fn entry(guild: u64, user: u64) -> MemberDisplay {
		MemberDisplay {
			guild: Id(guild),
			user: Id(user),
			name: format!("Display {user}"),
			username: Some(format!("user_{user}").into()),
			nick: Some(format!("Nickname {user}")),
		}
	}
	#[test]
	fn numeric_member_search_matches_only_the_exact_scoped_id() {
		let mut store = store();
		let target = entry(10, 876_543_210);
		let mut alias = entry(10, 99);
		alias.nick = Some(format!("Nickname {}", target.user));
		store
			.save_member_displays(Id(1), &[target.clone(), alias])
			.unwrap();
		assert_eq!(
			store
				.search_member_displays(Id(1), Id(10), " 876543210 ", 1)
				.unwrap(),
			vec![target]
		);
		for (account, guild) in [(Id(2), Id(10)), (Id(1), Id(11))] {
			assert!(
				store
					.search_member_displays(account, guild, "876543210", 1)
					.unwrap()
					.is_empty()
			);
		}
		assert!(
			store
				.search_member_displays(Id(1), Id(10), "543210", 1)
				.unwrap()
				.is_empty()
		);
	}
	#[test]
	fn recent_member_hints_are_bounded_and_scoped() {
		let mut store = store();
		let older: Vec<_> = (1..=200).map(|user| entry(10, user)).collect();
		store.save_member_displays(Id(1), &older).unwrap();
		let recent = entry(10, 9000);
		store
			.save_member_displays(Id(1), std::slice::from_ref(&recent))
			.unwrap();
		store
			.save_member_displays(Id(2), &[entry(10, 9999)])
			.unwrap();
		store
			.save_member_displays(Id(1), &[entry(11, 9998)])
			.unwrap();
		let rows = store.load_recent_member_displays(Id(1), Id(10)).unwrap();
		assert_eq!(rows.len(), 128);
		assert_eq!(rows[0], recent);
		assert!(
			rows.iter()
				.all(|row| row.guild == Id(10) && row.user != Id(9999))
		);
		assert_eq!(
			store.load_member_displays(Id(1), Id(10)).unwrap().len(),
			201
		);
		let plan: String = store.0.query_row(
			"EXPLAIN QUERY PLAN SELECT user FROM member_displays WHERE account='1' AND guild='10' ORDER BY touched DESC,user LIMIT 128",
			[],
			|row| row.get(3),
		).unwrap();
		assert!(plan.contains("member_displays_guild_recency"), "{plan}");
	}
	#[test]
	fn display_cache_migrates_and_preserves_settings_and_account_scope() {
		let mut store = store();
		store.0.execute_batch("DROP TABLE member_displays; PRAGMA user_version=29; INSERT INTO drafts VALUES('1','2','unchanged draft');").unwrap();
		store = LocalStore::initialize(store.0).unwrap();
		assert_eq!(
			store
				.0
				.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
				.unwrap(),
			crate::NATIVE_SCHEMA
		);
		let one = entry(10, 20);
		let other = entry(10, 21);
		store
			.save_member_displays(Id(1), std::slice::from_ref(&one))
			.unwrap();
		store
			.save_member_displays(Id(2), std::slice::from_ref(&other))
			.unwrap();
		assert_eq!(
			store.load_member_displays(Id(1), Id(10)).unwrap(),
			vec![one]
		);
		assert_eq!(
			store.load_member_displays(Id(2), Id(10)).unwrap(),
			vec![other]
		);
		assert!(
			store
				.load_member_displays(Id(1), Id(11))
				.unwrap()
				.is_empty()
		);
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "unchanged draft");
	}
	#[test]
	fn explicit_nickname_removal_departure_and_clear_are_scoped() {
		let mut store = store();
		store
			.save_member_displays(Id(1), &[entry(10, 20), entry(10, 21), entry(11, 20)])
			.unwrap();
		store.save_member_displays(Id(2), &[entry(10, 20)]).unwrap();
		let mut removed = entry(10, 20);
		removed.nick = None;
		removed.username = None;
		store
			.save_member_displays(Id(1), std::slice::from_ref(&removed))
			.unwrap();
		assert!(
			store
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.iter()
				.any(|e| e.user == Id(20) && e.nick.is_none() && e.username.is_none())
		);
		store.remove_member_display(Id(1), Id(10), Id(21)).unwrap();
		assert_eq!(
			store.load_member_displays(Id(1), Id(10)).unwrap(),
			vec![removed]
		);
		store.clear_member_guild(Id(1), Id(10)).unwrap();
		assert!(
			store
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.is_empty()
		);
		assert_eq!(store.load_member_displays(Id(1), Id(11)).unwrap().len(), 1);
		store.clear_history(Id(1)).unwrap();
		assert!(
			store
				.load_member_displays(Id(1), Id(11))
				.unwrap()
				.is_empty()
		);
		assert_eq!(store.load_member_displays(Id(2), Id(10)).unwrap().len(), 1);
		store.forget_account(Id(2)).unwrap();
		assert!(
			store
				.load_member_displays(Id(2), Id(10))
				.unwrap()
				.is_empty()
		);
	}
	#[test]
	fn display_cache_global_count_byte_eviction_and_rejected_batch_rollback() {
		let mut store = store();
		let rows: Vec<_> = (1..=4096).map(|id| entry(10, id)).collect();
		store.save_member_displays(Id(1), &rows).unwrap();
		store.save_member_displays(Id(2), &[entry(11, 1)]).unwrap();
		let count: i64 = store
			.0
			.query_row("SELECT count(*) FROM member_displays", [], |r| r.get(0))
			.unwrap();
		assert_eq!(count, 4097);
		assert_eq!(
			store.load_member_displays(Id(1), Id(10)).unwrap().len(),
			4096
		);
		let large: Vec<_> = (1..=1000)
			.map(|id| MemberDisplay {
				guild: Id(12),
				user: Id(id),
				name: "😀".repeat(128),
				username: Some("😀".repeat(128).into()),
				nick: Some("😀".repeat(128)),
			})
			.collect();
		store.save_member_displays(Id(3), &large).unwrap();
		let bytes: i64 = store
			.0
			.query_row(
				&format!("SELECT sum({ROW_CHARGE}) FROM member_displays"),
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert!(bytes <= DISK_BYTES as i64);
		let retained = store.load_member_displays(Id(3), Id(12)).unwrap();
		assert_eq!(retained.len(), 1000);
		let invalid = MemberDisplay {
			name: "x".repeat(513),
			..entry(12, 2000)
		};
		assert_eq!(
			store.save_member_displays(Id(3), &[entry(12, 2001), invalid]),
			Err(StoreError::Capacity)
		);
		assert_eq!(store.load_member_displays(Id(3), Id(12)).unwrap(), retained);
		assert_eq!(
			store.save_member_displays(Id(3), &[entry(12, 20), entry(12, 20)]),
			Err(StoreError::Capacity)
		);
	}
	#[test]
	fn display_cache_rejects_corruption_before_large_field_allocation() {
		let mut store = store();
		store.save_member_displays(Id(1), &[entry(10, 20)]).unwrap();
		store
			.0
			.execute_batch(
				"PRAGMA ignore_check_constraints=ON; UPDATE member_displays SET name=zeroblob(1048576);",
			)
			.unwrap();
		assert_eq!(
			store.load_member_displays(Id(1), Id(10)),
			Err(StoreError::Incompatible)
		);
	}
	#[test]
	fn repeated_identity_snapshots_do_not_write_unchanged_peers() {
		let mut store = store();
		let mut entries: Vec<_> = (1..=512).map(|user| entry(10, user)).collect();
		store.save_member_displays(Id(1), &entries).unwrap();
		store.0.execute_batch("CREATE TABLE display_writes(n INTEGER); INSERT INTO display_writes VALUES(0);
            CREATE TRIGGER count_display_inserts AFTER INSERT ON member_displays BEGIN UPDATE display_writes SET n=n+1; END;
            CREATE TRIGGER count_display_updates AFTER UPDATE ON member_displays BEGIN UPDATE display_writes SET n=n+1; END;
            CREATE TRIGGER count_display_deletes AFTER DELETE ON member_displays BEGIN UPDATE display_writes SET n=n+1; END;").unwrap();
		for _ in 0..3 {
			store.save_member_displays(Id(1), &entries).unwrap();
		}
		let writes = |store: &LocalStore| {
			store
				.0
				.query_row::<i64, _, _>("SELECT n FROM display_writes", [], |r| r.get(0))
				.unwrap()
		};
		assert_eq!(writes(&store), 0);
		entries[250].nick = Some("Changed nickname".into());
		store.save_member_displays(Id(1), &entries).unwrap();
		// The changed identity is replaced; its 511 peers remain untouched.
		assert_eq!(writes(&store), 2);
		store.save_member_displays(Id(1), &entries).unwrap();
		assert_eq!(writes(&store), 2);
		assert!(
			store
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.iter()
				.any(|entry| entry.user == Id(251)
					&& entry.nick.as_deref() == Some("Changed nickname"))
		);

		// Reusing the lookup must retain account isolation and the existing eviction
		// order: unchanged observations do not make older rows recently used.
		let other_account = entry(10, 251);
		store
			.save_member_displays(Id(2), std::slice::from_ref(&other_account))
			.unwrap();
		let fill: Vec<_> = (1..=(LIMIT - entries.len() - 1) as u64)
			.map(|user| entry(11, user))
			.collect();
		store.save_member_displays(Id(2), &fill).unwrap();
		store.save_member_displays(Id(1), &entries).unwrap();
		store
			.save_member_displays(Id(2), &[entry(11, 5000)])
			.unwrap();
		let remaining = store.load_member_displays(Id(1), Id(10)).unwrap();
		assert_eq!(remaining.len(), entries.len());
		assert!(remaining.iter().any(|row| row.user == Id(1)));
		assert!(
			remaining.iter().any(|row| {
				row.user == Id(251) && row.nick.as_deref() == Some("Changed nickname")
			})
		);
		assert_eq!(
			store.load_member_displays(Id(2), Id(10)).unwrap(),
			vec![other_account]
		);
	}
	#[test]
	fn expanded_member_disk_cache_retains_pages_and_eviction_preserves_recent_hints() {
		let mut store = store();
		let first = vec![entry(10, 1), entry(10, 2)];
		store
			.save_member_displays_with_limits(Id(1), &first, 2, MAX_BYTES)
			.unwrap();
		store
			.save_member_displays_with_limits(Id(1), &[entry(10, 3)], 2, MAX_BYTES)
			.unwrap();
		let rows = store.load_member_displays(Id(1), Id(10)).unwrap();
		assert_eq!(rows.len(), 2);
		assert!(!rows.iter().any(|row| row.user == Id(1)));
		assert!(rows.iter().any(|row| row.user == Id(3)));
		let page: Vec<_> = (10..4106).map(|user| entry(11, user)).collect();
		store.save_member_displays(Id(1), &page).unwrap();
		store
			.save_member_displays(Id(1), &[entry(11, 5000)])
			.unwrap();
		assert_eq!(
			store.load_member_displays(Id(1), Id(11)).unwrap().len(),
			4096
		);
		let count: i64 = store
			.0
			.query_row(
				"SELECT count(*) FROM member_displays WHERE guild='11'",
				[],
				|row| row.get(0),
			)
			.unwrap();
		assert_eq!(count, 4097);
	}
	#[test]
	fn targeted_hints_and_unicode_nickname_search_are_scoped_and_track_removal() {
		let mut store = store();
		let row = MemberDisplay {
			nick: Some("Émilie 🦊".into()),
			..entry(10, 20)
		};
		store
			.save_member_displays(Id(1), std::slice::from_ref(&row))
			.unwrap();
		store.save_member_displays(Id(2), &[entry(10, 20)]).unwrap();
		assert_eq!(
			store
				.search_member_displays(Id(1), Id(10), "émilie", 100)
				.unwrap(),
			vec![row.clone()]
		);
		assert!(
			store
				.search_member_displays(Id(2), Id(10), "émilie", 100)
				.unwrap()
				.is_empty()
		);
		assert_eq!(
			store
				.load_member_displays_for_users(Id(1), Id(10), &[Id(20), Id(99)])
				.unwrap(),
			vec![row.clone()]
		);
		assert!(
			store
				.load_member_displays_for_users(Id(1), Id(11), &[Id(20)])
				.unwrap()
				.is_empty()
		);
		store
			.save_member_displays(Id(1), &[MemberDisplay { nick: None, ..row }])
			.unwrap();
		assert!(
			store
				.search_member_displays(Id(1), Id(10), "émilie", 100)
				.unwrap()
				.is_empty()
		);
		assert_eq!(
			store.search_member_displays(Id(1), Id(10), " ", 100),
			Err(StoreError::Capacity)
		);
		assert_eq!(
			store.load_member_displays_for_users(Id(1), Id(10), &[Id(20); 101]),
			Err(StoreError::Capacity)
		);
		let (count, bytes): (i64, i64) = store
			.0
			.query_row(
				"SELECT count,bytes FROM member_cache_totals WHERE singleton=1",
				[],
				|row| Ok((row.get(0)?, row.get(1)?)),
			)
			.unwrap();
		let actual: (i64, i64) = store
			.0
			.query_row(
				&format!("SELECT count(*),sum({ROW_CHARGE}) FROM member_displays"),
				[],
				|row| Ok((row.get(0)?, row.get(1)?)),
			)
			.unwrap();
		assert_eq!((count, bytes), actual);
	}

	#[test]
	fn complete_roster_reconciliation_pages_over_removed_keys_and_preserves_other_scopes() {
		let mut store = store();
		let entries: Vec<_> = (1..=300).map(|user| entry(10, user)).collect();
		store.save_member_displays(Id(1), &entries).unwrap();
		store.save_member_displays(Id(1), &[entry(11, 1)]).unwrap();
		store.save_member_displays(Id(2), &[entry(10, 1)]).unwrap();
		let users: Vec<_> = std::iter::once(Id(2))
			.chain(std::iter::once(Id(10)))
			.chain((100..=150).map(Id))
			.collect();
		assert_eq!(
			store
				.reconcile_member_displays(Id(1), Id(10), &users)
				.unwrap(),
			300 - users.len()
		);
		let retained = store.load_member_displays(Id(1), Id(10)).unwrap();
		assert_eq!(retained.len(), users.len());
		assert!(
			retained
				.iter()
				.all(|entry| users.binary_search(&entry.user).is_ok())
		);
		assert_eq!(
			store.load_member_displays(Id(1), Id(11)).unwrap(),
			vec![entry(11, 1)]
		);
		assert_eq!(
			store.load_member_displays(Id(2), Id(10)).unwrap(),
			vec![entry(10, 1)]
		);
		assert_eq!(
			store
				.reconcile_member_displays(Id(1), Id(10), &users)
				.unwrap(),
			0
		);
		assert_eq!(
			store.reconcile_member_displays(Id(1), Id(10), &[]).unwrap(),
			users.len()
		);
		assert!(
			store
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.is_empty()
		);
		let totals: (i64, i64) = store
			.0
			.query_row(
				"SELECT count,bytes FROM member_cache_totals WHERE singleton=1",
				[],
				|row| Ok((row.get(0)?, row.get(1)?)),
			)
			.unwrap();
		let actual: (i64, i64) = store
			.0
			.query_row(
				&format!("SELECT count(*),sum({ROW_CHARGE}) FROM member_displays"),
				[],
				|row| Ok((row.get(0)?, row.get(1)?)),
			)
			.unwrap();
		assert_eq!(totals, actual);
	}
	#[test]
	fn invalid_or_incomplete_roster_input_does_not_prune_hints() {
		let mut store = store();
		let entries = vec![entry(10, 1), entry(10, 2)];
		store.save_member_displays(Id(1), &entries).unwrap();
		let before = store.load_member_displays(Id(1), Id(10)).unwrap();
		for users in [
			vec![Id(0)],
			vec![Id(2), Id(1)],
			vec![Id(1), Id(1)],
			vec![Id(1); DISK_LIMIT + 1],
		] {
			assert_eq!(
				store.reconcile_member_displays(Id(1), Id(10), &users),
				Err(StoreError::Capacity)
			);
			assert_eq!(store.load_member_displays(Id(1), Id(10)).unwrap(), before);
		}
		// A partial successful page only upserts supplied identities. It never calls
		// complete-roster reconciliation and must retain omitted peers.
		store.save_member_displays(Id(1), &[entry(10, 1)]).unwrap();
		assert_eq!(store.load_member_displays(Id(1), Id(10)).unwrap(), before);
	}
	#[test]
	fn confirmed_missing_member_batch_is_scoped_deduplicated_and_atomic() {
		let mut store = store();
		store
			.save_member_displays(Id(1), &[entry(10, 10), entry(10, 20), entry(11, 10)])
			.unwrap();
		store.save_member_displays(Id(2), &[entry(10, 10)]).unwrap();
		store.0.execute_batch("CREATE TRIGGER reject_member_removal BEFORE DELETE ON member_displays WHEN old.user='20' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
		assert_eq!(
			store.remove_member_displays(Id(1), Id(10), &[Id(10), Id(20)]),
			Err(StoreError::Unavailable)
		);
		assert_eq!(store.load_member_displays(Id(1), Id(10)).unwrap().len(), 2);
		store
			.0
			.execute_batch("DROP TRIGGER reject_member_removal;")
			.unwrap();
		store
			.remove_member_displays(Id(1), Id(10), &[Id(10), Id(10), Id(999)])
			.unwrap();
		assert_eq!(
			store.load_member_displays(Id(1), Id(10)).unwrap(),
			vec![entry(10, 20)]
		);
		assert_eq!(
			store.load_member_displays(Id(1), Id(11)).unwrap(),
			vec![entry(11, 10)]
		);
		assert_eq!(
			store.load_member_displays(Id(2), Id(10)).unwrap(),
			vec![entry(10, 10)]
		);
		assert_eq!(
			store.remove_member_displays(Id(1), Id(10), &[Id(20); 101]),
			Err(StoreError::Capacity)
		);
		assert_eq!(
			store.remove_member_displays(Id(1), Id(10), &[Id(0)]),
			Err(StoreError::Capacity)
		);
	}
	#[test]
	fn display_cache_survives_file_reopen() {
		let root = std::env::temp_dir().join(format!(
			"serein-member-cache-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::create_dir(&root).unwrap();
		let path = root.join("test.sqlite3");
		let expected = entry(10, 20);
		let mut store = LocalStore::open(&path).unwrap();
		store
			.save_member_displays(Id(1), std::slice::from_ref(&expected))
			.unwrap();
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		assert_eq!(
			store.load_member_displays(Id(1), Id(10)).unwrap(),
			vec![expected]
		);
		store.forget_account(Id(1)).unwrap();
		drop(store);
		assert!(
			LocalStore::open(&path)
				.unwrap()
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.is_empty()
		);
		std::fs::remove_dir_all(root).unwrap();
	}
}
