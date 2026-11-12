use crate::{DiscordApi, Failure};
use discord_protocol::{server_admin as members_wire, server_roles as wire};
use model::{
	Id,
	server_roles::{Action, Catalog, Result as Outcome},
};
use serde_json::json;
use wreq::Method;
impl DiscordApi {
	async fn load_role_catalog(&self, guild: Id) -> Result<Catalog, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}"),
				None,
				members_wire::MAX_WIRE,
			)
			.await?;
		let mut catalog = wire::catalog(&bytes, guild)
			.map_err(|_| Failure::ProtocolAt("Role catalog is unavailable or unsupported"))?;
		match self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/roles/member-counts"),
				None,
				64 * 1024,
			)
			.await
		{
			Ok(bytes) => {
				let _ = wire::counts(&bytes, &mut catalog);
			}
			Err(failure) if failure.ends_session() => return Err(failure),
			Err(_) => {} // Missing counts are unknown, never an invented zero or a scraped directory.
		}
		Ok(catalog)
	}
	pub(super) async fn server_role_action(
		&self,
		guild: Id,
		action: &Action,
	) -> Result<Outcome, Failure> {
		if guild.0 == 0 || !action.valid() {
			return Err(Failure::Protocol);
		}
		if let Action::Members { role, query } = action {
			let now = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map_or(0, |value| value.as_millis().min(i64::MAX as u128) as i64);
			let mut body = members_wire::query(query, now).map_err(|_| Failure::Protocol)?;
			if let Some(role) = role.filter(|role| *role != guild) {
				if body.get("and_query").is_none() {
					body["and_query"] = json!({});
				}
				body["and_query"]["role_ids"] = json!({"and_query":[role]});
			}
			let bytes = self
				.request_limited(
					Method::POST,
					&format!("/guilds/{guild}/members-search"),
					Some(body),
					members_wire::MAX_WIRE,
				)
				.await?;
			let metadata = self.admin_metadata(guild).await?;
			let page = members_wire::members(&bytes, guild, metadata).map_err(|_| {
				Failure::ProtocolAt("Role member search is unavailable or still indexing")
			})?;
			if role.is_some_and(|role| {
				role != guild
					&& page
						.items
						.iter()
						.any(|member| !member.roles.contains(&role))
			}) {
				return Err(Failure::Protocol);
			}
			return Ok(Outcome::Members { role: *role, page });
		}
		let latest = self.load_role_catalog(guild).await?;
		let selected = match action {
			Action::Load => {
				return Ok(Outcome::Catalog {
					catalog: latest,
					selected: None,
				});
			}
			Action::Create(edit) => {
				let body = wire::encode_edit(edit, None).map_err(|_| Failure::Protocol)?;
				let bytes = self
					.request_limited(
						Method::POST,
						&format!("/guilds/{guild}/roles"),
						Some(body),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let created = wire::role(&bytes).map_err(|_| Failure::Ambiguous)?;
				if latest.items.iter().any(|role| role.id == created.id)
					|| !wire::matches_edit(edit, &created, true)
				{
					return Err(Failure::Ambiguous);
				}
				Some(created.id)
			}
			Action::Edit { id, edit } => {
				let existing = latest
					.items
					.iter()
					.find(|role| role.id == *id)
					.ok_or(Failure::Forbidden)?;
				if existing.managed || *id == guild && !edit.only_permissions() {
					return Err(Failure::Forbidden);
				}
				let body =
					wire::encode_edit(edit, Some(existing)).map_err(|_| Failure::Protocol)?;
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/guilds/{guild}/roles/{id}"),
						Some(body),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let saved = wire::role(&bytes).map_err(|_| Failure::Ambiguous)?;
				if saved.id != *id || !wire::matches_edit(edit, &saved, false) {
					return Err(Failure::Ambiguous);
				}
				Some(*id)
			}
			Action::Delete(id) => {
				if *id == guild
					|| latest
						.items
						.iter()
						.find(|role| role.id == *id)
						.is_none_or(|role| role.managed)
				{
					return Err(Failure::Forbidden);
				}
				let bytes = self
					.request_limited(
						Method::DELETE,
						&format!("/guilds/{guild}/roles/{id}"),
						None,
						4096,
					)
					.await
					.map_err(write_failure)?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
				None
			}
			Action::Move { id, position } => {
				if *id == guild
					|| latest
						.items
						.iter()
						.find(|role| role.id == *id)
						.is_none_or(|role| role.managed)
				{
					return Err(Failure::Forbidden);
				}
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/guilds/{guild}/roles"),
						Some(json!([{"id":id,"position":position}])),
						members_wire::MAX_WIRE,
					)
					.await
					.map_err(write_failure)?;
				// The response is a complete role list; a subsequent GET confirms current ordering.
				if bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
				Some(*id)
			}
			Action::Members { .. } => unreachable!(),
		};
		let catalog = self
			.load_role_catalog(guild)
			.await
			.map_err(reconcile_failure)?;
		if selected.is_some_and(|id| !catalog.items.iter().any(|role| role.id == id))
			|| matches!(action, Action::Delete(id) if catalog.items.iter().any(|role| role.id == *id))
		{
			return Err(Failure::Ambiguous);
		}
		if let Action::Move { id, position } = action
			&& catalog
				.items
				.iter()
				.find(|role| role.id == *id)
				.is_none_or(|role| role.position != *position)
		{
			return Err(Failure::Ambiguous);
		}
		Ok(Outcome::Catalog { catalog, selected })
	}
}
fn write_failure(failure: Failure) -> Failure {
	if failure == Failure::Capacity {
		Failure::Ambiguous
	} else {
		failure
	}
}
fn reconcile_failure(failure: Failure) -> Failure {
	if failure.ends_session() {
		failure
	} else {
		Failure::Ambiguous
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::SessionSecret;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};
	#[tokio::test]
	async fn role_edit_http_preserves_fresh_unknown_bits_and_reconciles() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_ROLE_TOKEN".into()).unwrap())).unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let unknown = 1_u128 << 110;
			let role = |id: &str, name: &str, bits: u128, position: i32| json!({"id":id,"name":name,"permissions":bits.to_string(),"position":position,"color":123,"hoist":false,"mentionable":false,"managed":false});
			let before = role("4", "Member", unknown | 1024, 1);
			let after = role("4", "Renamed", unknown | 2048, 1);
			let initial = json!({"id":"2","roles":[role("2","@everyone",0,0),before],"features":[]});
			let final_catalog = json!({"id":"2","roles":[role("2","@everyone",0,0),after.clone()],"features":[]});
			let server = tokio::spawn(async move {
				for (method, path, expected, response) in [
					("GET", "/guilds/2", None, initial),
					("GET", "/guilds/2/roles/member-counts", None, json!({"4":7})),
					("PATCH", "/guilds/2/roles/4", Some(json!({"name":"Renamed","permissions":(unknown | 2048).to_string()})), after),
					("GET", "/guilds/2", None, final_catalog),
					("GET", "/guilds/2/roles/member-counts", None, json!({"4":8})),
				] {
					let (mut stream, _) = listener.accept().await.unwrap();
					let mut bytes = Vec::new();
					let header_end = loop {
						let mut chunk = [0; 4096]; let n = stream.read(&mut chunk).await.unwrap(); assert!(n > 0); bytes.extend_from_slice(&chunk[..n]); assert!(bytes.len() <= 16384);
						if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") { break end + 4; }
					};
					let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
					assert!(headers.starts_with(&format!("{method} {path} HTTP/1.1\r\n")));
					let length = headers.lines().find_map(|line| { let (name, value) = line.split_once(':')?; name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap()) }).unwrap_or(0);
					assert!(length <= 4096);
					while bytes.len() < header_end + length { let mut chunk = [0;4096]; let n = stream.read(&mut chunk).await.unwrap(); assert!(n > 0); bytes.extend_from_slice(&chunk[..n]); }
					if let Some(expected) = expected { assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes[header_end..header_end + length]).unwrap(), expected); } else { assert_eq!(length, 0); }
					let body = response.to_string();
					stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
				}
			});
			let result = api.server_role_action(Id(2), &Action::Edit { id: Id(4), edit: model::server_roles::Edit { name: Some("Renamed".into()), permissions: Some(2048), permission_mask: 1024 | 2048, ..Default::default() } }).await.unwrap();
			let Outcome::Catalog { catalog, selected } = result else { panic!() };
			assert_eq!(selected, Some(Id(4)));
			let saved = catalog.items.iter().find(|role| role.id == Id(4)).unwrap();
			assert_eq!(saved.permissions, unknown | 2048);
			assert_eq!(saved.member_count, Some(8));
			assert_eq!(saved.colors.primary, 123);
			server.await.unwrap();
		}).await.unwrap();
	}
}
