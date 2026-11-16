use crate::{DiscordApi, Failure};
use model::{
	Id,
	archives::{Cursor, Kind, Page},
};
use wreq::Method;

impl DiscordApi {
	pub(super) async fn archives(
		&self,
		parent: Id,
		guild: Id,
		kind: Kind,
		before: Option<Cursor>,
	) -> Result<Page, Failure> {
		if parent.0 == 0 || guild.0 == 0 {
			return Err(Failure::Protocol);
		}
		let route = match kind {
			Kind::Public => "threads/archived/public",
			Kind::Private => "threads/archived/private",
			Kind::JoinedPrivate => "users/@me/threads/archived/private",
		};
		let mut path = format!("/channels/{parent}/{route}?limit=25");
		if let Some(before) = before {
			let cursor = match (kind, before) {
				(Kind::Public | Kind::Private, Cursor::Time(value)) => {
					discord_protocol::pins::format_cursor(value).map_err(|_| Failure::Protocol)?
				}
				(Kind::JoinedPrivate, Cursor::Id(id)) if id.0 > 0 => id.to_string(),
				_ => return Err(Failure::Protocol),
			};
			let encoded: String = cursor.bytes().map(|byte| format!("%{byte:02X}")).collect();
			path.push_str(&format!("&before={encoded}"));
		}
		let bytes = self
			.request_limited(
				Method::GET,
				&path,
				None,
				discord_protocol::archives::MAX_WIRE,
			)
			.await?;
		discord_protocol::decode::<discord_protocol::archives::Reply>(&bytes)
			.map_err(|_| Failure::Protocol)?
			.into_page(parent, guild, kind, before)
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
	async fn archive_routes_are_read_only_scoped_and_encode_the_correct_cursor() {
		tokio::time::timeout(Duration::from_secs(10),async {
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut api=DiscordApi::new(Arc::new(SessionSecret::from_owner_input("SYNTHETIC_ARCHIVE_TOKEN".into()).unwrap())).unwrap();
            api.base=format!("http://{}",listener.local_addr().unwrap());
            let server=tokio::spawn(async move {
                for (route,status,body) in [
                    ("/channels/2/threads/archived/public?limit=25","200 OK",r#"{"threads":[{"id":"9","guild_id":"1","parent_id":"2","type":11,"name":"Synthetic","thread_metadata":{"archived":true,"archive_timestamp":"2026-09-10T12:00:00Z"}}],"members":[],"has_more":true}"#),
                    ("/channels/2/threads/archived/public?limit=25&before=%32%30%32%36%2D%30%39%2D%31%30%54%31%32%3A%30%30%3A%30%30%5A","200 OK",r#"{"threads":[],"members":[],"has_more":false}"#),
                    ("/channels/2/threads/archived/private?limit=25","403 Forbidden",r#"{"code":50013}"#),
                    ("/channels/2/users/@me/threads/archived/private?limit=25&before=%31%30","200 OK",r#"{"threads":[{"id":"9","guild_id":"1","parent_id":"2","type":12,"name":"Synthetic","thread_metadata":{"archived":true,"archive_timestamp":"2026-09-10T12:00:00Z"}}],"has_more":false}"#),
                    ("/channels/2/users/@me/threads/archived/private?limit=25&before=%39","200 OK",r#"{"threads":[{"id":"9","guild_id":"1","parent_id":"2","type":12,"name":"Synthetic","thread_metadata":{"archived":true,"archive_timestamp":"2026-09-10T12:00:00Z"}}],"has_more":true}"#),
                    ("/channels/2/threads/archived/public?limit=25","200 OK",r#"{"threads":[{"id":"9","guild_id":"7","parent_id":"2","type":11,"name":"Synthetic","thread_metadata":{"archived":true,"archive_timestamp":"2026-09-10T12:00:00Z"}}],"has_more":false}"#),
                ] {
                    let (mut socket,_)=listener.accept().await.unwrap();let mut request=Vec::new();
                    loop {let mut buffer=[0;1024];let n=socket.read(&mut buffer).await.unwrap();assert!(n>0);request.extend_from_slice(&buffer[..n]);assert!(request.len()<4096);if request.windows(4).any(|w|w==b"\r\n\r\n"){break;}}
                    let text=std::str::from_utf8(&request).unwrap();
                    assert!(text.starts_with(&format!("GET {route} HTTP/1.1\r\n")));
                    assert!(text.contains("SYNTHETIC_ARCHIVE_TOKEN"));
                    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                }
            });
            let Event::Archives {parent:Id(2),request:3,result:Ok(page)}=api.execute(Command::Archives{parent:Id(2),guild:Id(1),kind:Kind::Public,before:None,request:3}).await else {panic!()};
            assert_eq!(page.threads[0].id,Id(9));
            let exhausted=api.archives(Id(2),Id(1),Kind::Public,page.next).await.unwrap();
            assert!(exhausted.threads.is_empty()&&exhausted.next.is_none());
            assert!(matches!(api.archives(Id(2),Id(1),Kind::Private,None).await,Err(Failure::Forbidden)));
            let joined=api.archives(Id(2),Id(1),Kind::JoinedPrivate,Some(Cursor::Id(Id(10)))).await.unwrap();
            assert_eq!(joined.threads[0].id,Id(9));
            assert!(matches!(api.archives(Id(2),Id(1),Kind::JoinedPrivate,Some(Cursor::Id(Id(9)))).await,Err(Failure::Protocol)));
            assert!(matches!(api.archives(Id(2),Id(1),Kind::Public,None).await,Err(Failure::Protocol)));
            for (kind,cursor) in [(Kind::Public,Cursor::Id(Id(2))),(Kind::JoinedPrivate,Cursor::Time(0)),(Kind::Private,Cursor::Time(i128::MAX))] {
                assert!(matches!(api.archives(Id(2),Id(1),kind,Some(cursor)).await,Err(Failure::Protocol)));
            }
            server.await.unwrap();
        }).await.unwrap();
	}
}
