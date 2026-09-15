use crate::{DiscordApi, Failure};
use model::{Id, MemberList};
use reqwest::Method;

impl DiscordApi {
	pub async fn thread_members(
		&self,
		guild: Id,
		channel: Id,
		request: u64,
	) -> Result<MemberList, Failure> {
		if guild.0 == 0 || channel.0 == 0 {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/channels/{channel}/thread-members?with_member=true&limit=100"),
				None,
				discord_protocol::thread_members::MAX_WIRE,
			)
			.await?;
		discord_protocol::thread_members::members(&bytes, guild, channel, request)
			.map_err(|_| Failure::Protocol)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::auth::SessionSecret;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	#[tokio::test]
	async fn thread_members_use_bounded_read_only_route_and_propagate_forbidden() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_THREAD_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				for (status, body) in [
					(
						"200 OK",
						r#"[{"id":"2","user_id":"3","member":{"user":{"id":"3","username":"Synthetic"}}}]"#,
					),
					("403 Forbidden", r#"{"code":50013}"#),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = Vec::new();
					loop {
						let mut buffer = [0; 1024];
						let n = socket.read(&mut buffer).await.unwrap();
						assert!(n > 0);
						request.extend_from_slice(&buffer[..n]);
						assert!(request.len() < 8192);
						if request.windows(4).any(|window| window == b"\r\n\r\n") {
							break;
						}
					}
					assert!(std::str::from_utf8(&request).unwrap().starts_with(
						"GET /channels/2/thread-members?with_member=true&limit=100 HTTP/1.1\r\n"
					));
					socket
						.write_all(
							format!(
								"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
								body.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
				}
			});
			let list = api.thread_members(Id(1), Id(2), 7).await.unwrap();
			assert_eq!((list.channel, list.request, list.total), (Id(2), 7, 1));
			assert_eq!(list.rows[0].as_ref().unwrap().user.id, Id(3));
			assert!(matches!(
				api.thread_members(Id(1), Id(2), 8).await,
				Err(Failure::Forbidden)
			));
			assert!(matches!(
				api.thread_members(Id(0), Id(2), 9).await,
				Err(Failure::Protocol)
			));
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
