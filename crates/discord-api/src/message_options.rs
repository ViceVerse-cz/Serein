//! Composer syntax shared by text, replies, uploads, stickers and forum starters.
pub(super) const SUPPRESS_NOTIFICATIONS: u32 = 1 << 12;

pub(super) use model::message_options::{content, valid};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::DiscordApi;
	use client_core::{
		Reply,
		auth::{Failure, SessionSecret},
	};
	use model::Id;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	#[tokio::test]
	async fn quiet_messages_set_flags_preserve_mentions_and_reject_empty_text() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_SILENT_TOKEN".into()).unwrap(),
			)).unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				for (expected, silent, attachments, sticker, forum) in [
					("hello <@7>", true, false, false, false),
					("\n    indented\n  ", true, false, false, false),
					("hello", false, false, false, false),
					("@silently hello", false, false, false, false),
					("", true, true, false, false),
					("", true, false, true, false),
					("quiet starter", true, false, false, true),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut bytes = Vec::new();
					let end = loop {
						let mut chunk = [0; 1024];
						let count = socket.read(&mut chunk).await.unwrap();
						assert!(count > 0 && bytes.len() + count <= 8192);
						bytes.extend_from_slice(&chunk[..count]);
						if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") { break end + 4; }
					};
					let headers = std::str::from_utf8(&bytes[..end]).unwrap();
					let path = if forum { "/channels/2/threads" } else { "/channels/2/messages" };
					assert!(headers.starts_with(&format!("POST {path} HTTP/1.1\r\n")));
					let length = headers.lines().find_map(|line| {
						let (name, value) = line.split_once(':')?;
						name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
					}).unwrap();
					assert!(end + length <= 8192);
					while bytes.len() < end + length {
						let mut chunk = [0; 1024];
						let count = socket.read(&mut chunk).await.unwrap();
						assert!(count > 0 && bytes.len() + count <= 8192);
						bytes.extend_from_slice(&chunk[..count]);
					}
					let body: serde_json::Value = serde_json::from_slice(&bytes[end..end + length]).unwrap();
					let message = if forum { &body["message"] } else { &body };
					assert_eq!(message["content"], expected);
					assert_eq!(message["flags"].as_u64(), silent.then_some(4096));
					assert_eq!(message.get("attachments").is_some(), attachments);
					assert_eq!(message.get("sticker_ids").is_some(), sticker);
					if expected.contains("<@7>") {
						assert_eq!(message["allowed_mentions"]["users"], serde_json::json!(["7"]));
						assert_eq!(message["allowed_mentions"]["replied_user"], true);
						assert_eq!(message["message_reference"]["message_id"], "50");
					}
					let reply = if forum {
						serde_json::json!({"id":"10","guild_id":"1","parent_id":"2","type":11,"name":"Synthetic post"})
					} else {
						serde_json::json!({"id":"100","channel_id":"2","author":{"id":"1","username":"Synthetic"},"type":0,"content":expected})
					}.to_string();
					socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).await.unwrap();
				}
			});
			assert!(matches!(api.send_message(Id(2), "@silent \n", "local", None, None, None).await, Err(Failure::Capacity)));
			assert!(matches!(api.send_message(Id(2), "@silent", "local", None, Some(vec![]), None).await, Err(Failure::Capacity)));
			for attachments in [None, Some(vec![])] {
				assert!(matches!(api.create_post(Id(2), Id(1), ("Synthetic post", &[]), "@silent", attachments).await, Err(Failure::Capacity)));
			}
			for (text, reply, attachment, sticker) in [
				("@silent hello <@7>", Some(Reply::to(Id(50))), None, None),
				("@silent \n    indented\n  ", None, None, None),
				("hello", None, None, None),
				("@silently hello", None, None, None),
				("@silent", None, Some(vec![serde_json::json!({"id":"0","filename":"synthetic.txt","uploaded_filename":"synthetic"})]), None),
				("@silent", None, None, Some(Id(9))),
			] {
				api.send_message(Id(2), text, "local", reply, attachment, sticker).await.unwrap();
			}
			api.create_post(Id(2), Id(1), ("Synthetic post", &[]), "@silent\nquiet starter", None).await.unwrap();
			server.await.unwrap();
		}).await.unwrap();
		for text in [
			"@silentword",
			"hello @silent",
			"`@silent`",
			"\\@silent hello",
			"@Silent hello",
		] {
			assert_eq!(content(text), (text, false));
		}
		assert_eq!(content("@silent\thello"), ("hello", true));
	}
}
