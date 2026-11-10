use crate::{DiscordApi, Failure};
use discord_protocol::server_audit_log as wire;
use model::{
	Id,
	server_audit_log::{Page, Query},
};
use wreq::Method;
impl DiscordApi {
	pub(super) async fn server_audit_log(&self, guild: Id, query: &Query) -> Result<Page, Failure> {
		if guild.0 == 0 || !query.valid() {
			return Err(Failure::Protocol);
		}
		let mut path = format!("/guilds/{guild}/audit-logs?limit=50");
		if let Some(user) = query.user {
			path.push_str(&format!("&user_id={user}"));
		}
		if let Some(action) = query.action {
			path.push_str(&format!("&action_type={action}"));
		}
		if let Some(before) = query.before {
			path.push_str(&format!("&before={before}"));
		}
		let bytes = self
			.request_limited(Method::GET, &path, None, wire::MAX_WIRE)
			.await?;
		wire::page(&bytes, guild, query).map_err(|_| {
			Failure::ProtocolAt("Audit log exceeded safe bounds or returned unexpected entries")
		})
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
	async fn audit_log_http_filters_cursor_and_forbidden_without_retry() {
		tokio::time::timeout(Duration::from_secs(10),async {
			let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api=DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_AUDIT_TOKEN".into()).unwrap())).unwrap();
			api.base=format!("http://{}",listener.local_addr().unwrap());
			let server=tokio::spawn(async move {
				for (path,status,response) in [
					("/guilds/9/audit-logs?limit=50&user_id=2&action_type=25&before=100",200,r#"{"audit_log_entries":[{"id":"99","user_id":"2","target_id":"3","action_type":25}],"users":[]}"#),
					("/guilds/9/audit-logs?limit=50&before=99",200,r#"{"audit_log_entries":[],"users":[]}"#),
					("/guilds/9/audit-logs?limit=50",403,"{}"),
				] {
					let (mut stream,_)=listener.accept().await.unwrap(); let mut bytes=Vec::new();
					loop {let mut chunk=[0;4096];let n=stream.read(&mut chunk).await.unwrap();assert!(n>0);bytes.extend_from_slice(&chunk[..n]);assert!(bytes.len()<=16384);if bytes.windows(4).any(|part|part==b"\r\n\r\n"){break;}}
					let headers=std::str::from_utf8(&bytes).unwrap();assert!(headers.starts_with(&format!("GET {path} HTTP/1.1\r\n")));assert!(!headers.to_ascii_lowercase().contains("content-length:"));
					stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
				}
				assert!(tokio::time::timeout(Duration::from_millis(80),listener.accept()).await.is_err());
			});
			let query=Query {user:Some(Id(2)),action:Some(25),before:Some(Id(100))};
			let page=api.server_audit_log(Id(9),&query).await.unwrap();assert_eq!(page.entries[0].id,Id(99));
			let page=api.server_audit_log(Id(9),&Query {before:Some(Id(99)),..Default::default()}).await.unwrap();assert!(page.entries.is_empty()&&!page.has_more);
			assert!(matches!(api.server_audit_log(Id(9),&Query::default()).await,Err(Failure::Forbidden)));
			assert!(matches!(api.server_audit_log(Id(0),&Query::default()).await,Err(Failure::Protocol)));
			server.await.unwrap();
		}).await.unwrap();
	}
}
