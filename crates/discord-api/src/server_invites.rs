use crate::{DiscordApi, Failure};
use discord_protocol::server_invites as wire;
use model::{
	Id,
	server_invites::{Action, Snapshot},
};
use wreq::Method;
impl DiscordApi {
	async fn invite_features(&self, guild: Id) -> Result<Vec<String>, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}"),
				None,
				wire::MAX_WIRE,
			)
			.await?;
		wire::features(&bytes, guild)
			.map_err(|_| Failure::ProtocolAt("Invite settings are unavailable or unsupported"))
	}
	async fn invite_snapshot(&self, guild: Id) -> Result<Snapshot, Failure> {
		let features = self.invite_features(guild).await?;
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/invites"),
				None,
				wire::MAX_WIRE,
			)
			.await?;
		wire::snapshot(&bytes, guild, features).map_err(|_| {
			Failure::ProtocolAt(
				"Server invites exceeded safe bounds or used an unsupported response",
			)
		})
	}
	pub(super) async fn server_invite_action(
		&self,
		guild: Id,
		action: &Action,
	) -> Result<Snapshot, Failure> {
		if guild.0 == 0 || !action.valid() {
			return Err(Failure::Protocol);
		}
		match action {
			Action::Load => return self.invite_snapshot(guild).await,
			Action::Revoke { code } => {
				let latest = self.invite_snapshot(guild).await?;
				if !latest.items.iter().any(|invite| invite.code == *code) {
					return Ok(latest);
				}
				let bytes = self
					.request_limited(Method::DELETE, &format!("/invites/{code}"), None, 64 * 1024)
					.await
					.map_err(write_failure)?;
				wire::revoked(&bytes, guild, code).map_err(|_| Failure::Ambiguous)?;
			}
			Action::SetPaused { paused } => {
				let features = self.invite_features(guild).await?;
				if features
					.iter()
					.any(|feature| feature == model::server_invites::PAUSED_FEATURE)
					!= *paused
				{
					let body =
						wire::pause_body(features, *paused).map_err(|_| Failure::Protocol)?;
					let bytes = self
						.request_limited(
							Method::PATCH,
							&format!("/guilds/{guild}"),
							Some(body),
							wire::MAX_WIRE,
						)
						.await
						.map_err(write_failure)?;
					let saved = wire::features(&bytes, guild).map_err(|_| Failure::Ambiguous)?;
					if saved
						.iter()
						.any(|feature| feature == model::server_invites::PAUSED_FEATURE)
						!= *paused
					{
						return Err(Failure::Ambiguous);
					}
				}
			}
		}
		let page = self.invite_snapshot(guild).await.map_err(|failure| {
			if failure.ends_session() {
				failure
			} else {
				Failure::Ambiguous
			}
		})?;
		if matches!(action, Action::Revoke { code } if page.items.iter().any(|invite| invite.code == *code))
			|| matches!(action, Action::SetPaused { paused } if page.paused() != *paused)
		{
			return Err(Failure::Ambiguous);
		}
		Ok(page)
	}
}
fn write_failure(failure: Failure) -> Failure {
	if failure == Failure::Capacity {
		Failure::Ambiguous
	} else {
		failure
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::SessionSecret;
	use serde_json::json;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};
	#[tokio::test]
	async fn invites_http_pause_preserves_features_and_revoke_reconciles_without_retry() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_INVITES_TOKEN".into()).unwrap())).unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let initial = json!({"id":"2","features":["FUTURE_FEATURE","COMMUNITY"]});
			let paused = json!({"id":"2","features":["FUTURE_FEATURE","COMMUNITY","INVITES_DISABLED"]});
			let invite = json!({"code":"synthetic","guild":{"id":"2"},"channel":{"id":"3","name":"chat"},"uses":1});
			let server = tokio::spawn(async move {
				for (method, path, status, response) in [
					("GET","/guilds/2",200,initial),
					("PATCH","/guilds/2",200,paused.clone()),
					("GET","/guilds/2",200,paused.clone()),
					("GET","/guilds/2/invites",200,json!([invite.clone()])),
					("GET","/guilds/2",200,paused.clone()),
					("GET","/guilds/2/invites",200,json!([invite.clone()])),
					("DELETE","/invites/synthetic",200,invite.clone()),
					("GET","/guilds/2",200,paused.clone()),
					("GET","/guilds/2/invites",200,json!([])),
					("GET","/guilds/2",200,paused),
					("GET","/guilds/2/invites",200,json!([invite])),
					("DELETE","/invites/synthetic",500,json!({})),
				] {
					let (mut stream, _) = listener.accept().await.unwrap(); let mut bytes = Vec::new();
					let end = loop { let mut chunk = [0;4096]; let n=stream.read(&mut chunk).await.unwrap(); assert!(n>0); bytes.extend_from_slice(&chunk[..n]); assert!(bytes.len()<=16384); if let Some(end)=bytes.windows(4).position(|part|part==b"\r\n\r\n") {break end+4;} };
					let headers = std::str::from_utf8(&bytes[..end]).unwrap(); assert!(headers.starts_with(&format!("{method} {path} HTTP/1.1\r\n")));
					let length=headers.lines().find_map(|line| { let (name,value)=line.split_once(':')?; name.eq_ignore_ascii_case("content-length").then(||value.trim().parse::<usize>().unwrap()) }).unwrap_or(0); assert!(length<=4096);
					while bytes.len()<end+length { let mut chunk=[0;4096];let n=stream.read(&mut chunk).await.unwrap();assert!(n>0);bytes.extend_from_slice(&chunk[..n]); }
					if method=="PATCH" { assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes[end..end+length]).unwrap(),json!({"features":["FUTURE_FEATURE","COMMUNITY","INVITES_DISABLED"]})); } else {assert_eq!(length,0);}
					let body=response.to_string();stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
				}
				assert!(tokio::time::timeout(Duration::from_millis(80),listener.accept()).await.is_err());
			});
			let paused = api.server_invite_action(Id(2), &Action::SetPaused { paused:true }).await.unwrap(); assert!(paused.paused()); assert_eq!(paused.items.len(),1);
			let revoked = api.server_invite_action(Id(2), &Action::Revoke { code:"synthetic".into() }).await.unwrap(); assert!(revoked.items.is_empty());
			assert!(matches!(api.server_invite_action(Id(2), &Action::Revoke { code:"synthetic".into() }).await,Err(Failure::Ambiguous)));
			server.await.unwrap();
		}).await.unwrap();
	}
}
