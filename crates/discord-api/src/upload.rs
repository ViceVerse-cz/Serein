//! User-selected files, staged to Discord's signed storage target before message creation.
//! Paths, signed URLs and file bytes are never serialized into diagnostics or retained as drafts.
use crate::{DiscordApi, Failure};
use client_core::{Command, Event};
use reqwest::{Method, Url};
use std::{
	path::PathBuf,
	time::{Duration, SystemTime},
};
use tokio::{fs::File, io::AsyncReadExt, sync::watch};

pub const MAX_BYTES: u64 = 20_000_000;
pub const MAX_FILES: usize = 10;
pub const MAX_TOTAL_BYTES: u64 = MAX_BYTES;
const CHUNK_BYTES: usize = 64 * 1024;
const MAX_RESPONSE: usize = 64 * 1024;
const CANCELLED: &str = "Upload cancelled; no message was sent";
const CHANGED: &str = "Selected file changed or disappeared; select it again";

// Deliberately neither Debug nor Serialize: local paths must not enter logs or session caches.
#[derive(Clone)]
pub struct Source {
	path: PathBuf,
	bytes: Option<std::sync::Arc<[u8]>>,
	filename: String,
	size: u64,
	modified: SystemTime,
}
impl Source {
	pub async fn inspect(path: PathBuf) -> Result<Self, &'static str> {
		if path.as_os_str().as_encoded_bytes().len() > 4096 {
			return Err("Selected path is too long");
		}
		let filename = path
			.file_name()
			.and_then(|n| n.to_str())
			.ok_or("Unsupported filename")?;
		if filename.trim().is_empty()
			|| matches!(filename, "." | "..")
			|| filename.len() > 256
			|| filename
				.chars()
				.any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
		{
			return Err("Unsupported filename");
		}
		let metadata = tokio::fs::symlink_metadata(&path)
			.await
			.map_err(|_| CHANGED)?;
		if !metadata.is_file() || metadata.file_type().is_symlink() {
			return Err("Choose a regular file");
		}
		if metadata.len() == 0 || metadata.len() > MAX_BYTES {
			return Err("Choose a nonempty file up to 20 MB");
		}
		Ok(Self {
			bytes: None,
			filename: filename.into(),
			path,
			size: metadata.len(),
			modified: metadata
				.modified()
				.map_err(|_| "File modification time is unavailable")?,
		})
	}
	/// A pasted PNG stays in bounded session memory, never in a temporary file.
	pub fn pasted_png(bytes: Vec<u8>) -> Result<Self, &'static str> {
		if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
			return Err("Choose a nonempty image up to 20 MB");
		}
		Ok(Self {
			path: PathBuf::new(),
			filename: "pasted-image.png".into(),
			size: bytes.len() as u64,
			modified: SystemTime::UNIX_EPOCH,
			bytes: Some(bytes.into()),
		})
	}
	pub fn filename(&self) -> &str {
		&self.filename
	}
	pub fn size(&self) -> u64 {
		self.size
	}
	/// Whole selection bytes for a local preview, only while the file is at most `limit` bytes
	/// and still matches the inspected metadata. Pasted images reuse their in-memory buffer.
	pub async fn preview_bytes(&self, limit: u64) -> Option<std::sync::Arc<[u8]>> {
		if self.size > limit {
			return None;
		}
		if let Some(bytes) = &self.bytes {
			return Some(bytes.clone());
		}
		let metadata = tokio::fs::symlink_metadata(&self.path).await.ok()?;
		if !self.matches(&metadata) {
			return None;
		}
		let bytes = tokio::fs::read(&self.path).await.ok()?;
		(bytes.len() as u64 == self.size).then(|| bytes.into())
	}
	fn matches(&self, metadata: &std::fs::Metadata) -> bool {
		metadata.is_file()
			&& !metadata.file_type().is_symlink()
			&& metadata.len() == self.size
			&& metadata.modified().ok() == Some(self.modified)
	}
	async fn validate(&self) -> Result<(), Failure> {
		if self.bytes.is_some() {
			return Ok(());
		}
		let metadata = tokio::fs::symlink_metadata(&self.path)
			.await
			.map_err(|_| Failure::ProtocolAt(CHANGED))?;
		if self.matches(&metadata) {
			Ok(())
		} else {
			Err(Failure::ProtocolAt(CHANGED))
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
	Preparing,
	Uploading { sent: u64, total: u64 },
	Sending,
	Finished,
	Cancelled,
	Failed(&'static str),
}

#[derive(serde::Deserialize)]
struct UploadResponse {
	attachments: Vec<Target>,
}
#[derive(serde::Deserialize)]
struct Target {
	#[serde(default)]
	id: Option<serde_json::Value>,
	upload_url: String,
	upload_filename: String,
}

/// What the staged files belong to: a message in an existing channel, or a new forum post.
enum Destination {
	Send {
		nonce: String,
		reply: Option<client_core::Reply>,
	},
	Post {
		guild: model::Id,
		title: String,
		request: u64,
	},
}
impl Destination {
	/// Report a failure as the event the caller's flow expects; no write was accepted.
	fn failed(self, channel: model::Id, failure: Failure) -> Event {
		match self {
			Self::Send { nonce, .. } => Event::SendResult {
				nonce,
				result: Err(failure),
			},
			Self::Post { request, .. } => Event::PostCreated {
				parent: channel,
				request,
				result: Err(failure),
			},
		}
	}
}
fn status<T>(result: &Result<T, Failure>) -> Status {
	match result {
		Ok(_) => Status::Finished,
		Err(Failure::ProtocolAt(CANCELLED)) => Status::Cancelled,
		Err(failure) => Status::Failed(failure.label()),
	}
}

impl DiscordApi {
	pub async fn upload_message(
		&self,
		command: Command,
		source: Source,
		progress: watch::Sender<Status>,
		cancel: watch::Receiver<bool>,
	) -> Event {
		self.upload_messages(command, vec![source], progress, cancel)
			.await
	}

	pub async fn upload_messages(
		&self,
		command: Command,
		sources: Vec<Source>,
		progress: watch::Sender<Status>,
		mut cancel: watch::Receiver<bool>,
	) -> Event {
		let (channel, content, target) = match command {
			Command::Send {
				channel,
				content,
				nonce,
				reply,
			} => (channel, content, Destination::Send { nonce, reply }),
			// A forum post is one request: its files are staged before the thread exists.
			Command::CreatePost {
				parent,
				guild,
				title,
				content,
				request,
				..
			} => (
				parent,
				content,
				Destination::Post {
					guild,
					title,
					request,
				},
			),
			_ => {
				progress.send_replace(Status::Failed("Invalid upload request"));
				return Event::Failure(Failure::ProtocolAt("Invalid upload request"));
			}
		};
		if content.chars().count() > client_core::MAX_CONTENT {
			let failure = Failure::ProtocolAt("Message is too long; no file was uploaded");
			progress.send_replace(Status::Failed(failure.label()));
			return target.failed(channel, failure);
		}
		if sources.is_empty()
			|| sources.len() > MAX_FILES
			|| sources.iter().map(Source::size).sum::<u64>() > MAX_TOTAL_BYTES
		{
			let failure = Failure::ProtocolAt("Choose up to 10 files totaling at most 20 MB");
			progress.send_replace(Status::Failed(failure.label()));
			return target.failed(channel, failure);
		}
		let total = sources.iter().map(Source::size).sum::<u64>();
		progress.send_replace(Status::Preparing);
		let prepare = async {
			for source in &sources {
				source.validate().await?;
			}
			let mut attachments = Vec::with_capacity(sources.len());
			let mut completed = 0;
			for (id, source) in sources.iter().enumerate() {
				attachments.push(
					self.upload_file(channel, source, id, &progress, completed, total)
						.await?,
				);
				completed += source.size();
			}
			Ok::<_, Failure>(attachments)
		};
		let prepared = tokio::select! {
			biased;
			_ = cancelled(&mut cancel) => Err(Failure::ProtocolAt(CANCELLED)),
			result = prepare => result,
		};
		// Past this boundary cancellation may race Discord's acceptance. Never claim
		// cancellation deleted a message or a post, and never retry the POST.
		let attachment = match prepared {
			Ok(attachment) if !*cancel.borrow() && cancel.has_changed().is_ok() => Some(attachment),
			Ok(_) => None,
			Err(failure) => {
				progress.send_replace(status(&Err::<(), _>(failure)));
				return target.failed(channel, failure);
			}
		};
		let Some(attachment) = attachment else {
			let failure = Failure::ProtocolAt(CANCELLED);
			progress.send_replace(Status::Cancelled);
			return target.failed(channel, failure);
		};
		progress.send_replace(Status::Sending);
		match target {
			Destination::Send { nonce, reply } => {
				let result = tokio::select! {
					biased;
					_ = cancelled(&mut cancel) => Err(Failure::Ambiguous),
					result = self.send_message(channel, &content, &nonce, reply, Some(attachment)) => result,
				};
				progress.send_replace(status(&result));
				Event::SendResult { nonce, result }
			}
			Destination::Post {
				guild,
				title,
				request,
			} => {
				let result = tokio::select! {
					biased;
					_ = cancelled(&mut cancel) => Err(Failure::Ambiguous),
					result = self.create_post(channel, guild, &title, &content, Some(attachment)) => result,
				};
				progress.send_replace(status(&result));
				Event::PostCreated {
					parent: channel,
					request,
					result,
				}
			}
		}
	}

	async fn upload_file(
		&self,
		channel: model::Id,
		source: &Source,
		id: usize,
		progress: &watch::Sender<Status>,
		completed: u64,
		batch_total: u64,
	) -> Result<serde_json::Value, Failure> {
		source.validate().await?;
		let (file, original): (Box<dyn tokio::io::AsyncRead + Send + Unpin>, Option<File>) =
			if let Some(bytes) = &source.bytes {
				(Box::new(std::io::Cursor::new(bytes.clone())), None)
			} else {
				let file = File::open(&source.path)
					.await
					.map_err(|_| Failure::ProtocolAt(CHANGED))?;
				if !source.matches(
					&file
						.metadata()
						.await
						.map_err(|_| Failure::ProtocolAt(CHANGED))?,
				) {
					return Err(Failure::ProtocolAt(CHANGED));
				}
				let original = file
					.try_clone()
					.await
					.map_err(|_| Failure::ProtocolAt(CHANGED))?;
				(Box::new(file), Some(original))
			};
		let body = serde_json::json!({"files":[{"id":id.to_string(),"filename":source.filename(),"file_size":source.size(),"is_clip":false}]});
		let response = self
			.request_limited(
				Method::POST,
				&format!("/channels/{channel}/attachments"),
				Some(body),
				MAX_RESPONSE,
			)
			.await
			.map_err(|f| {
				if f == Failure::Ambiguous {
					Failure::ProtocolAt("Upload preparation failed; no message was sent")
				} else {
					f
				}
			})?;
		let mut response: UploadResponse = serde_json::from_slice(&response)
			.map_err(|_| Failure::ProtocolAt("Upload preparation response unsupported"))?;
		if response.attachments.len() != 1 {
			return Err(Failure::ProtocolAt(
				"Upload preparation response unsupported",
			));
		}
		let target = response.attachments.remove(0);
		if target
			.id
			.as_ref()
			.is_some_and(|value| value != &id.to_string() && value != id)
			|| target.upload_filename.is_empty()
			|| target.upload_filename.len() > 1024
			|| target.upload_filename.chars().any(char::is_control)
		{
			return Err(Failure::ProtocolAt(
				"Upload preparation response unsupported",
			));
		}
		let url = self.upload_url(&target.upload_url)?;
		if self.stopped() {
			return Err(Failure::Expired);
		}
		let client = self
			.upload_client
			.get_or_try_init(|| async {
				reqwest::Client::builder()
					.redirect(reqwest::redirect::Policy::none())
					.retry(reqwest::retry::never())
					.no_proxy()
					.connect_timeout(Duration::from_secs(10))
					.read_timeout(Duration::from_secs(30))
					.timeout(Duration::from_secs(300))
					.pool_max_idle_per_host(1)
					.pool_idle_timeout(Duration::from_secs(30))
					.user_agent(client_core::fingerprint::user_agent())
					.build()
					.map_err(|_| Failure::Network)
			})
			.await?;
		let total = source.size();
		progress.send_replace(Status::Uploading {
			sent: completed,
			total: batch_total,
		});
		let updates = progress.clone();
		let stream = futures_util::stream::try_unfold((file, 0u64), move |(mut file, sent)| {
			let updates = updates.clone();
			async move {
				if sent == total {
					return Ok::<_, std::io::Error>(None);
				}
				let mut bytes = vec![0; (total - sent).min(CHUNK_BYTES as u64) as usize];
				file.read_exact(&mut bytes).await?;
				let sent = sent + bytes.len() as u64;
				// Latest-value progress cannot fill the session event queue. Bytes count
				// data supplied to HTTP, not a remote receipt or confirmed message.
				updates.send_replace(Status::Uploading {
					sent: completed + sent,
					total: batch_total,
				});
				Ok(Some((bytes, (file, sent))))
			}
		});
		let mut response = client
			.put(url)
			.header(
				reqwest::header::CONTENT_TYPE,
				content_type(source.filename()),
			)
			.header(reqwest::header::CONTENT_LENGTH, total)
			.body(reqwest::Body::wrap_stream(stream))
			.send()
			.await
			.map_err(|_| Failure::ProtocolAt("File upload failed; no message was sent"))?;
		if !response.status().is_success() {
			return Err(Failure::ProtocolAt(
				"File upload rejected; no message was sent",
			));
		}
		if *progress.borrow()
			!= (Status::Uploading {
				sent: completed + total,
				total: batch_total,
			}) {
			return Err(Failure::ProtocolAt(
				"File upload incomplete; no message was sent",
			));
		}
		if response
			.content_length()
			.is_some_and(|n| n > MAX_RESPONSE as u64)
		{
			return Err(Failure::ProtocolAt("Upload response exceeded its limit"));
		}
		let mut received = 0;
		while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Network)? {
			received += chunk.len();
			if received > MAX_RESPONSE {
				return Err(Failure::ProtocolAt("Upload response exceeded its limit"));
			}
		}
		source.validate().await?;
		if let Some(original) = original
			&& !source.matches(
				&original
					.metadata()
					.await
					.map_err(|_| Failure::ProtocolAt(CHANGED))?,
			) {
			return Err(Failure::ProtocolAt(CHANGED));
		}
		if self.stopped() {
			return Err(Failure::Expired);
		}
		Ok(
			serde_json::json!({"id":id.to_string(),"filename":source.filename(),"uploaded_filename":target.upload_filename}),
		)
	}

	fn upload_url(&self, value: &str) -> Result<Url, Failure> {
		let invalid = Failure::ProtocolAt("Upload storage address rejected; no file was uploaded");
		if value.len() > 4096 {
			return Err(invalid);
		}
		let url = Url::parse(value).map_err(|_| invalid)?;
		if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
			return Err(invalid);
		}
		let allowed = url.scheme() == "https"
			&& url.host_str() == Some("discord-attachments-uploads-prd.storage.googleapis.com")
			&& url.port_or_known_default() == Some(443);
		#[cfg(test)]
		let allowed = allowed
			|| (url.scheme() == "http"
				&& self.upload_origin.is_some_and(|address| {
					address.ip().is_loopback()
						&& url.host_str() == Some("127.0.0.1")
						&& url.port() == Some(address.port())
				}));
		if allowed { Ok(url) } else { Err(invalid) }
	}
}

/// MIME type the official client declares on the storage PUT, from the filename extension.
fn content_type(filename: &str) -> &'static str {
	let extension = filename
		.rsplit_once('.')
		.map(|(_, e)| e.to_ascii_lowercase())
		.unwrap_or_default();
	match extension.as_str() {
		"png" => "image/png",
		"jpg" | "jpeg" => "image/jpeg",
		"gif" => "image/gif",
		"webp" => "image/webp",
		"svg" => "image/svg+xml",
		"mp4" => "video/mp4",
		"webm" => "video/webm",
		"mov" => "video/quicktime",
		"mp3" => "audio/mpeg",
		"ogg" => "audio/ogg",
		"wav" => "audio/wav",
		"pdf" => "application/pdf",
		"zip" => "application/zip",
		"json" => "application/json",
		"txt" | "md" | "log" | "rs" | "toml" | "csv" => "text/plain",
		_ => "application/octet-stream",
	}
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
	loop {
		let requested = *cancel.borrow_and_update();
		if requested || cancel.changed().await.is_err() {
			return;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Reply, auth::SessionSecret};
	use std::sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	};
	use tokio::{
		io::AsyncWriteExt,
		net::{TcpListener, TcpStream},
	};

	struct Fixture(PathBuf);
	impl Fixture {
		async fn new(bytes: &[u8]) -> Self {
			static NEXT: AtomicU64 = AtomicU64::new(0);
			let path = std::env::temp_dir().join(format!(
				"serein-upload-{}-{}-{}.txt",
				std::process::id(),
				SystemTime::now()
					.duration_since(SystemTime::UNIX_EPOCH)
					.unwrap()
					.as_nanos(),
				NEXT.fetch_add(1, Ordering::Relaxed)
			));
			let mut file = tokio::fs::OpenOptions::new()
				.write(true)
				.create_new(true)
				.open(&path)
				.await
				.unwrap();
			file.write_all(bytes).await.unwrap();
			file.flush().await.unwrap();
			Self(path)
		}
	}
	impl Drop for Fixture {
		fn drop(&mut self) {
			let _ = std::fs::remove_file(&self.0);
		}
	}
	fn api() -> DiscordApi {
		DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_UPLOAD_TOKEN".into()).unwrap(),
		))
		.unwrap()
	}
	fn command() -> Command {
		Command::Send {
			channel: model::Id(1),
			content: String::new(),
			nonce: "synthetic-upload".into(),
			reply: Some(Reply::to(model::Id(2))),
		}
	}
	async fn request(socket: &mut TcpStream) -> (String, Vec<u8>) {
		let mut data = Vec::new();
		loop {
			let mut chunk = [0; 4096];
			let n = socket.read(&mut chunk).await.unwrap();
			assert!(n > 0);
			data.extend_from_slice(&chunk[..n]);
			assert!(data.len() < 256 * 1024);
			if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
				let head = String::from_utf8(data[..end].to_vec()).unwrap();
				let length: usize = head
					.lines()
					.find_map(|line| {
						line.to_ascii_lowercase()
							.strip_prefix("content-length: ")
							.map(str::to_owned)
					})
					.unwrap()
					.parse()
					.unwrap();
				if data.len() >= end + 4 + length {
					return (head, data[end + 4..end + 4 + length].to_vec());
				}
			}
		}
	}
	async fn respond(socket: &mut TcpStream, status: &str, body: &str) {
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
	fn failed(event: Event) -> Failure {
		let Event::SendResult {
			nonce,
			result: Err(failure),
		} = event
		else {
			panic!("expected scoped send failure");
		};
		assert_eq!(nonce, "synthetic-upload");
		failure
	}

	#[tokio::test]
	async fn staged_upload_streams_without_credentials_and_reconciles_message() {
		tokio::time::timeout(Duration::from_secs(10), async {
            let fixture = Fixture::new(&vec![b'x'; CHUNK_BYTES * 2 + 9]).await;
            let source = Source::inspect(fixture.0.clone()).await.unwrap();
            let filename = source.filename().to_owned();
            let api_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let storage = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut api = api();
            api.base = format!("http://{}", api_listener.local_addr().unwrap());
            api.upload_origin = Some(storage.local_addr().unwrap());
            let upload_url = format!("http://{}/signed?upload_id=synthetic", storage.local_addr().unwrap());
            let server = tokio::spawn(async move {
                for id in 0..2 {
                let (mut socket, _) = api_listener.accept().await.unwrap();
                let (head, bytes) = request(&mut socket).await;
                assert!(head.starts_with("POST /channels/1/attachments HTTP/1.1"));
                assert!(head.contains("SYNTHETIC_UPLOAD_TOKEN"));
                assert!(head.to_ascii_lowercase().contains("x-super-properties: "));
                assert!(head.contains(&format!("user-agent: {}", client_core::fingerprint::user_agent())));
                assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(), serde_json::json!({"files":[{"id":id.to_string(),"filename":filename,"file_size":CHUNK_BYTES*2+9,"is_clip":false}]}));
                respond(&mut socket, "200 OK", &serde_json::json!({"attachments":[{"id":id,"upload_url":upload_url,"upload_filename":format!("synthetic-upload/{id}/file.txt")}]}).to_string()).await;
                let (mut socket, _) = storage.accept().await.unwrap();
                let (head, bytes) = request(&mut socket).await;
                assert!(head.starts_with("PUT /signed?upload_id=synthetic HTTP/1.1"));
                assert!(!head.to_ascii_lowercase().contains("authorization"));
                assert!(!head.to_ascii_lowercase().contains("cookie"));
                assert!(!head.contains("SYNTHETIC_UPLOAD_TOKEN"));
                assert!(!head.to_ascii_lowercase().contains("x-super-properties"));
                assert!(head.to_ascii_lowercase().contains("content-type: text/plain"));
                assert_eq!(bytes, vec![b'x'; CHUNK_BYTES*2+9]);
                respond(&mut socket, "200 OK", "").await;
                }
                let (mut socket, _) = api_listener.accept().await.unwrap();
                let (head, bytes) = request(&mut socket).await;
                assert!(head.starts_with("POST /channels/1/messages HTTP/1.1"));
                assert!(head.contains("SYNTHETIC_UPLOAD_TOKEN"));
                let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(body["content"], "");
                assert_eq!(body["nonce"], "synthetic-upload");
                assert_eq!(body["attachments"], serde_json::json!([{"id":"0","filename":filename,"uploaded_filename":"synthetic-upload/0/file.txt"},{"id":"1","filename":filename,"uploaded_filename":"synthetic-upload/1/file.txt"}]));
                assert_eq!(body["allowed_mentions"], serde_json::json!({"parse":[],"users":[],"roles":[],"replied_user":true}));
                assert_eq!(body["message_reference"], serde_json::json!({"message_id":"2","channel_id":"1"}));
                respond(&mut socket, "200 OK", r#"{"id":"3","channel_id":"1","author":{"id":"4","username":"Synthetic"},"nonce":"synthetic-upload"}"#).await;
            });
            let (progress, status) = watch::channel(Status::Preparing);
            let (_cancel, cancelled) = watch::channel(false);
            assert!(matches!(api.upload_messages(command(), vec![source.clone(), source], progress, cancelled).await, Event::SendResult { result: Ok(message), .. } if message.id == model::Id(3)));
            assert_eq!(*status.borrow(), Status::Finished);
            server.await.unwrap();
        }).await.unwrap();
	}

	#[tokio::test]
	async fn upload_rejects_changed_missing_oversized_sources_and_untrusted_targets() {
		let fixture = Fixture::new(b"synthetic").await;
		let source = Source::inspect(fixture.0.clone()).await.unwrap();
		tokio::fs::write(&fixture.0, b"changed").await.unwrap();
		assert_eq!(source.validate().await, Err(Failure::ProtocolAt(CHANGED)));
		tokio::fs::remove_file(&fixture.0).await.unwrap();
		assert!(Source::inspect(fixture.0.clone()).await.is_err());
		let empty = Fixture::new(&[]).await;
		assert!(Source::inspect(empty.0.clone()).await.is_err());
		let large = tokio::fs::OpenOptions::new()
			.write(true)
			.open(&empty.0)
			.await
			.unwrap();
		large.set_len(MAX_BYTES + 1).await.unwrap();
		assert!(Source::inspect(empty.0.clone()).await.is_err());
		drop(large);
		let api = api();
		let host = "discord-attachments-uploads-prd.storage.googleapis.com";
		assert!(
			api.upload_url(&format!("https://{host}/opaque?upload_id=synthetic"))
				.is_ok()
		);
		for url in [
			format!("http://{host}/x"),
			format!("https://{host}.evil.test/x"),
			format!("https://user@{host}/x"),
			format!("https://{host}:444/x"),
			format!("https://{host}/x#fragment"),
			"http://127.0.0.1:1234/x".into(),
			format!("https://{host}/{}", "x".repeat(4096)),
		] {
			assert!(api.upload_url(&url).is_err());
		}
	}

	#[tokio::test]
	async fn upload_cancel_before_write_and_redirect_never_send_message() {
		tokio::time::timeout(Duration::from_secs(10), async {
            let fixture = Fixture::new(b"synthetic").await;
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut api = api();
            api.base = format!("http://{}", listener.local_addr().unwrap());
            let (progress, status) = watch::channel(Status::Preparing);
            let (_cancel, cancelled) = watch::channel(true);
            assert_eq!(failed(api.upload_message(command(), Source::inspect(fixture.0.clone()).await.unwrap(), progress, cancelled).await), Failure::ProtocolAt(CANCELLED));
            assert_eq!(*status.borrow(), Status::Cancelled);
            assert!(tokio::time::timeout(Duration::from_millis(30), listener.accept()).await.is_err());

            let storage = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let redirected = TcpListener::bind("127.0.0.1:0").await.unwrap();
            api.upload_origin = Some(storage.local_addr().unwrap());
            let target = format!("http://{}/signed", storage.local_addr().unwrap());
            let redirect = format!("http://{}/must-not-open", redirected.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                request(&mut socket).await;
                respond(&mut socket, "200 OK", &serde_json::json!({"attachments":[{"upload_url":target,"upload_filename":"synthetic/file"}]}).to_string()).await;
                let (mut socket, _) = storage.accept().await.unwrap();
                request(&mut socket).await;
                socket.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                assert!(tokio::time::timeout(Duration::from_millis(50), listener.accept()).await.is_err());
                assert!(tokio::time::timeout(Duration::from_millis(50), redirected.accept()).await.is_err());
            });
            let (progress, _) = watch::channel(Status::Preparing);
            let (_cancel, cancelled) = watch::channel(false);
            assert_eq!(failed(api.upload_message(command(), Source::inspect(fixture.0.clone()).await.unwrap(), progress, cancelled).await), Failure::ProtocolAt("File upload rejected; no message was sent"));
            server.await.unwrap();
        }).await.unwrap();
	}

	#[tokio::test]
	async fn cancellation_during_message_post_keeps_outcome_unknown() {
		tokio::time::timeout(Duration::from_secs(10), async {
            let fixture = Fixture::new(b"synthetic").await;
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let storage = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut api = api();
            api.base = format!("http://{}", listener.local_addr().unwrap());
            api.upload_origin = Some(storage.local_addr().unwrap());
            let target = format!("http://{}/signed", storage.local_addr().unwrap());
            let (cancel, cancelled) = watch::channel(false);
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                request(&mut socket).await;
                respond(&mut socket, "200 OK", &serde_json::json!({"attachments":[{"upload_url":target,"upload_filename":"synthetic/file"}]}).to_string()).await;
                let (mut socket, _) = storage.accept().await.unwrap();
                request(&mut socket).await;
                respond(&mut socket, "200 OK", "").await;
                let (mut socket, _) = listener.accept().await.unwrap();
                let (head, _) = request(&mut socket).await;
                assert!(head.starts_with("POST /channels/1/messages HTTP/1.1"));
                cancel.send_replace(true);
                // Keep the cancellation sender and unanswered POST alive until its receiver ends.
                cancel.closed().await;
                assert!(tokio::time::timeout(Duration::from_millis(50), listener.accept()).await.is_err());
            });
            let (progress, status) = watch::channel(Status::Preparing);
            assert_eq!(failed(api.upload_message(command(), Source::inspect(fixture.0.clone()).await.unwrap(), progress, cancelled).await), Failure::Ambiguous);
            assert_eq!(*status.borrow(), Status::Failed(Failure::Ambiguous.label()));
            server.await.unwrap();
        }).await.unwrap();
	}

	#[tokio::test]
	async fn cancelling_put_or_changing_its_source_never_creates_message() {
		tokio::time::timeout(Duration::from_secs(10), async {
            for cancel_put in [true, false] {
                let fixture = Fixture::new(&vec![b'x'; CHUNK_BYTES * 2 + 9]).await;
                let source = Source::inspect(fixture.0.clone()).await.unwrap();
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let storage = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let mut api = api();
                api.base = format!("http://{}", listener.local_addr().unwrap());
                api.upload_origin = Some(storage.local_addr().unwrap());
                let target = format!("http://{}/signed", storage.local_addr().unwrap());
                let path = fixture.0.clone();
                let (cancel, cancelled) = watch::channel(false);
                let server = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    request(&mut socket).await;
                    respond(&mut socket, "200 OK", &serde_json::json!({"attachments":[{"upload_url":target,"upload_filename":"synthetic/file"}]}).to_string()).await;
                    let (mut socket, _) = storage.accept().await.unwrap();
                    if cancel_put {
                        let mut first_bytes = [0; 1024];
                        assert!(socket.read(&mut first_bytes).await.unwrap() > 0);
                        cancel.send_replace(true);
                        cancel.closed().await;
                    } else {
                        request(&mut socket).await;
                        tokio::fs::write(path, b"changed during upload").await.unwrap();
                        respond(&mut socket, "200 OK", "").await;
                        cancel.closed().await;
                    }
                    assert!(tokio::time::timeout(Duration::from_millis(50), listener.accept()).await.is_err());
                });
                let (progress, _) = watch::channel(Status::Preparing);
                assert_eq!(failed(api.upload_message(command(), source, progress, cancelled).await), Failure::ProtocolAt(if cancel_put { CANCELLED } else { CHANGED }));
                server.await.unwrap();
            }
        }).await.unwrap();
	}
}
