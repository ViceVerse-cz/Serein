use crate::{DiscordApi, Failure};
use client_core::inbox::{MAX_WIRE, PAGE_SIZE};
use discord_protocol::{MessageDto, RecentMentions, decode};
use model::{Id, Message};
use reqwest::Method;

impl DiscordApi {
	pub(super) async fn mentions(&self, before: Option<Id>) -> Result<Vec<Message>, Failure> {
		if before.is_some_and(|id| id.0 == 0) {
			return Err(Failure::Protocol);
		}
		let mut path = format!("/users/@me/mentions?limit={PAGE_SIZE}&roles=true&everyone=true");
		if let Some(before) = before {
			path.push_str(&format!("&before={before}"));
		}
		let bytes = self
			.request_limited(Method::GET, &path, None, MAX_WIRE)
			.await?;
		let RecentMentions(messages) =
			decode::<RecentMentions>(&bytes).map_err(|_| Failure::Protocol)?;
		if messages.len() > PAGE_SIZE {
			return Err(Failure::Capacity);
		}
		let mut messages: Vec<_> = messages.into_iter().map(MessageDto::into_model).collect();
		messages.sort_unstable_by_key(|message| std::cmp::Reverse(message.id));
		messages.shrink_to_fit();
		if messages.iter().map(Message::bytes).sum::<usize>() > client_core::inbox::MAX_BYTES {
			return Err(Failure::Capacity);
		}
		Ok(messages)
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
	async fn mentions_http_includes_role_everyone_and_before_parameters() {
		let message = serde_json::json!({"id":"10", "channel_id":"20", "author":{"id":"2", "username":"Synthetic"}, "content":"Mention"});
		assert!(
			decode::<RecentMentions>(
				&serde_json::to_vec(&vec![message.clone(); PAGE_SIZE]).unwrap()
			)
			.is_ok()
		);
		assert!(
			decode::<RecentMentions>(&serde_json::to_vec(&vec![message; PAGE_SIZE + 1]).unwrap())
				.is_err()
		);
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_MENTIONS_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				for suffix in ["", "&before=80"] {
					let (mut stream, _) = listener.accept().await.unwrap();
					let mut bytes = vec![];
					loop {
						let mut chunk = [0; 4096];
						let n = stream.read(&mut chunk).await.unwrap();
						assert!(n > 0);
						bytes.extend_from_slice(&chunk[..n]);
						assert!(bytes.len() < 16_384);
						if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
							break;
						}
					}
					assert!(std::str::from_utf8(&bytes).unwrap().starts_with(&format!(
						"GET /users/@me/mentions?limit=25&roles=true&everyone=true{suffix} HTTP/1.1\r\n"
					)));
					stream
						.write_all(
							b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]",
						)
						.await
						.unwrap();
				}
			});
			assert!(api.mentions(None).await.unwrap().is_empty());
			assert!(api.mentions(Some(Id(80))).await.unwrap().is_empty());
			assert!(matches!(
				api.mentions(Some(Id(0))).await,
				Err(Failure::Protocol)
			));
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
