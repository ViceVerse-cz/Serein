use crate::{DiscordApi, Failure};
use model::{Id, forum::Page};
use reqwest::Method;

impl DiscordApi {
	pub(super) async fn forum_summary(
		&self,
		channel: Id,
	) -> Result<model::forum::Summary, Failure> {
		if channel.0 == 0 {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/channels/{channel}/messages?limit=50"),
				None,
				discord_protocol::forum::MAX_WIRE,
			)
			.await?;
		discord_protocol::decode::<discord_protocol::forum::Recent>(&bytes)
			.map_err(|_| Failure::Protocol)?
			.into_summary(channel)
			.map_err(|_| Failure::Protocol)
	}

	/// Active posts of one forum. The gateway only syncs joined threads, so the list is fetched.
	///
	/// The per-forum search route is unofficial client behavior; a service that rejects it falls
	/// back to the documented guild-wide active list for the first page.
	pub(super) async fn forum_posts(
		&self,
		parent: Id,
		guild: Id,
		offset: usize,
	) -> Result<Page, Failure> {
		if parent.0 == 0 || guild.0 == 0 || offset > model::forum::MAX_POSTS {
			return Err(Failure::Protocol);
		}
		match self.searched_posts(parent, guild, offset).await {
			Err(Failure::Protocol) if offset == 0 => self.active_guild_posts(parent, guild).await,
			outcome => outcome,
		}
	}

	async fn searched_posts(&self, parent: Id, guild: Id, offset: usize) -> Result<Page, Failure> {
		let path = format!(
			"/channels/{parent}/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit={}&offset={offset}",
			model::forum::PAGE_SIZE
		);
		let bytes = self
			.request_limited(Method::GET, &path, None, discord_protocol::forum::MAX_WIRE)
			.await?;
		discord_protocol::decode::<discord_protocol::forum::Reply>(&bytes)
			.map_err(|_| Failure::Protocol)?
			.into_page(parent, guild)
			.map_err(|_| Failure::Protocol)
	}

	async fn active_guild_posts(&self, parent: Id, guild: Id) -> Result<Page, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/threads/active"),
				None,
				discord_protocol::forum::GUILD_MAX_WIRE,
			)
			.await?;
		discord_protocol::decode::<discord_protocol::forum::GuildActive>(&bytes)
			.map_err(|_| Failure::Protocol)?
			.into_page(parent, guild)
			.map_err(|_| Failure::Protocol)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Command, Event, auth::SessionSecret};
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	#[tokio::test]
	async fn forum_posts_are_scoped_paged_and_reject_foreign_rows() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_FORUM_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let row = |guild: &str| format!(r#"{{"id":"9","guild_id":"{guild}","parent_id":"2","type":11,"name":"Synthetic","thread_metadata":{{"archived":false}}}}"#);
			let server = tokio::spawn(async move {
				for (route, status, body) in [
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0","200 OK",format!(r#"{{"threads":[{}],"members":[],"has_more":true,"total_results":2}}"#, row("1"))),
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=25","200 OK",r#"{"threads":[],"members":[],"has_more":false}"#.to_owned()),
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0","404 Not Found",r#"{"code":0}"#.to_owned()),
					("/guilds/1/threads/active","200 OK",format!(r#"{{"threads":[{}],"members":[]}}"#, row("1"))),
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0","200 OK",format!(r#"{{"threads":[{}],"members":[],"has_more":false}}"#, row("7"))),
					("/guilds/1/threads/active","403 Forbidden",r#"{"code":50013}"#.to_owned()),
					("/channels/2/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25&offset=0","403 Forbidden",r#"{"code":50013}"#.to_owned()),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = Vec::new();
					loop {
						let mut buffer = [0; 1024];
						let n = socket.read(&mut buffer).await.unwrap();
						assert!(n > 0);
						request.extend_from_slice(&buffer[..n]);
						assert!(request.len() < 4096);
						if request.windows(4).any(|w| w == b"\r\n\r\n") {
							break;
						}
					}
					let text = std::str::from_utf8(&request).unwrap();
					assert!(text.starts_with(&format!("GET {route} HTTP/1.1\r\n")));
					assert!(text.contains("SYNTHETIC_FORUM_TOKEN"));
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
			let Event::ForumPosts {
				parent: Id(2),
				request: 4,
				result: Ok(page),
			} = api
				.execute(Command::ForumPosts {
					parent: Id(2),
					guild: Id(1),
					offset: 0,
					request: 4,
				})
				.await
			else {
				panic!()
			};
			assert_eq!(page.threads[0].id, Id(9));
			assert!(page.more);
			let exhausted = api.forum_posts(Id(2), Id(1), 25).await.unwrap();
			assert!(exhausted.threads.is_empty() && !exhausted.more);
			// A rejected search route falls back to the documented guild-wide active list.
			let fallback = api.forum_posts(Id(2), Id(1), 0).await.unwrap();
			assert_eq!(fallback.threads[0].id, Id(9));
			assert!(!fallback.more);
			// A foreign-scoped row is rejected, and so is its fallback.
			assert!(matches!(
				api.forum_posts(Id(2), Id(1), 0).await,
				Err(Failure::Forbidden)
			));
			// Only a rejected response falls back; a denial is reported as one.
			assert!(matches!(
				api.forum_posts(Id(2), Id(1), 0).await,
				Err(Failure::Forbidden)
			));
			assert!(matches!(
				api.forum_posts(Id(0), Id(1), 0).await,
				Err(Failure::Protocol)
			));
			assert!(matches!(
				api.forum_posts(Id(2), Id(1), model::forum::MAX_POSTS + 1).await,
				Err(Failure::Protocol)
			));
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
