//! Explicit anonymous public-file hosting, isolated from authenticated Discord transports.
use super::{CHANGED, CHUNK_BYTES, Source, Status};
use std::{io, time::Duration};
use tokio::{io::AsyncReadExt, sync::watch};

pub use model::public_upload::{Error, MAX_BYTES, eligible};
const RESPONSE_BYTES: usize = 4096;
const ENDPOINT: &str = "https://catbox.moe/user/api.php";

pub fn validated_link(value: &[u8]) -> Result<String, Error> {
	if value.len() > RESPONSE_BYTES {
		return Err(Error::InvalidLink);
	}
	let value = std::str::from_utf8(value)
		.map_err(|_| Error::InvalidLink)?
		.trim();
	if !value.starts_with("https://files.catbox.moe/")
		|| value.chars().any(|c| c.is_control() || c.is_whitespace())
	{
		return Err(Error::InvalidLink);
	}
	let url = reqwest::Url::parse(value).map_err(|_| Error::InvalidLink)?;
	let filename = value
		.strip_prefix("https://files.catbox.moe/")
		.unwrap_or_default();
	if url.scheme() != "https"
		|| url.host_str() != Some("files.catbox.moe")
		|| url.port().is_some()
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.query().is_some()
		|| url.fragment().is_some()
		|| filename.is_empty()
		|| filename.len() > 256
		|| !filename
			.bytes()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
		|| matches!(filename, "." | "..")
	{
		return Err(Error::InvalidLink);
	}
	Ok(url.into())
}

/// One attempt only. Cancellation cannot erase bytes already received by the public host.
pub async fn upload(
	source: Source,
	progress: watch::Sender<Status>,
	mut cancel: watch::Receiver<bool>,
) -> Result<String, Error> {
	if *cancel.borrow() {
		return Err(Error::Cancelled);
	}
	let client = reqwest::Client::builder()
		.https_only(true)
		.no_proxy()
		.redirect(reqwest::redirect::Policy::none())
		.retry(reqwest::retry::never())
		.no_gzip()
		.connect_timeout(Duration::from_secs(10))
		.read_timeout(Duration::from_secs(30))
		.timeout(Duration::from_secs(300))
		.build()
		.map_err(|_| Error::Prepare)?;
	attempt(&client, ENDPOINT, source, progress, &mut cancel).await
}

async fn attempt(
	client: &reqwest::Client,
	endpoint: &str,
	source: Source,
	progress: watch::Sender<Status>,
	cancel: &mut watch::Receiver<bool>,
) -> Result<String, Error> {
	tokio::select! {
		biased;
		_ = super::cancelled(cancel) => Err(Error::Cancelled),
		result = transfer(client, endpoint, source, progress) => result,
	}
}

async fn transfer(
	client: &reqwest::Client,
	endpoint: &str,
	source: Source,
	progress: watch::Sender<Status>,
) -> Result<String, Error> {
	if !eligible(source.filename(), source.size()) {
		return Err(Error::Unsupported);
	}
	source.validate().await.map_err(|_| Error::Changed)?;
	let (file, original): (Box<dyn tokio::io::AsyncRead + Send + Unpin>, _) =
		if let Some(bytes) = &source.bytes {
			(Box::new(io::Cursor::new(bytes.clone())), None)
		} else {
			let file = tokio::fs::File::open(&source.path)
				.await
				.map_err(|_| Error::Changed)?;
			if !source.matches(&file.metadata().await.map_err(|_| Error::Changed)?) {
				return Err(Error::Changed);
			}
			let original = file.try_clone().await.map_err(|_| Error::Changed)?;
			(Box::new(file), Some(std::sync::Arc::new(original)))
		};
	// Local filenames never form paths or headers without escaping quotes; controls were rejected at selection.
	let boundary = format!(
		"serein-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::SystemTime::UNIX_EPOCH)
			.map_err(|_| Error::Prepare)?
			.as_nanos()
	);
	let name = source.filename().replace(['"', '\\'], "_");
	let prefix = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"reqtype\"\r\n\r\nfileupload\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"fileToUpload\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
	let suffix = format!("\r\n--{boundary}--\r\n").into_bytes();
	let total = source.size();
	let body_size = total + prefix.len() as u64 + suffix.len() as u64;
	let updates = progress.clone();
	let source = std::sync::Arc::new(source);
	let streamed_source = source.clone();
	let streamed_original = original.clone();
	let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
	let completed_body = completed.clone();
	let stream = futures_util::stream::try_unfold(
		(file, 0, Some(prefix), Some(suffix)),
		move |(mut file, sent, mut prefix, mut suffix)| {
			let updates = updates.clone();
			let source = streamed_source.clone();
			let original = streamed_original.clone();
			let completed = completed_body.clone();
			async move {
				let chunk = if let Some(prefix) = prefix.take() {
					prefix
				} else if sent < total {
					let mut chunk = vec![0; (total - sent).min(CHUNK_BYTES as u64) as usize];
					file.read_exact(&mut chunk).await?;
					updates.send_replace(Status::Uploading {
						sent: sent + chunk.len() as u64,
						total,
					});
					let sent = sent + chunk.len() as u64;
					return Ok::<_, io::Error>(Some((chunk, (file, sent, prefix, suffix))));
				} else if let Some(suffix) = suffix.take() {
					source
						.validate()
						.await
						.map_err(|_| io::Error::other(CHANGED))?;
					if let Some(original) = original
						&& !source.matches(&original.metadata().await?)
					{
						return Err(io::Error::other(CHANGED));
					}
					completed.store(true, std::sync::atomic::Ordering::Release);
					suffix
				} else {
					return Ok(None);
				};
				Ok(Some((chunk, (file, sent, prefix, suffix))))
			}
		},
	);
	let mut response = client
		.post(endpoint)
		.header(
			reqwest::header::CONTENT_TYPE,
			format!("multipart/form-data; boundary={boundary}"),
		)
		.header(reqwest::header::CONTENT_LENGTH, body_size)
		.body(reqwest::Body::wrap_stream(stream))
		.send()
		.await
		.map_err(|_| Error::Failed)?;
	if !response.status().is_success() {
		return Err(Error::Rejected);
	}
	if !completed.load(std::sync::atomic::Ordering::Acquire)
		|| *progress.borrow() != (Status::Uploading { sent: total, total })
	{
		return Err(Error::Incomplete);
	}
	if response
		.content_length()
		.is_some_and(|length| length > RESPONSE_BYTES as u64)
	{
		return Err(Error::ResponseLimit);
	}
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await.map_err(|_| Error::Interrupted)? {
		if chunk.len() > RESPONSE_BYTES - bytes.len() {
			return Err(Error::ResponseLimit);
		}
		bytes.extend_from_slice(&chunk);
	}
	// Recheck after the response too: an early or delayed server reply must not conceal a
	// changed source. Writers restoring identical metadata remain outside observable checks.
	source.validate().await.map_err(|_| Error::Changed)?;
	if let Some(original) = original
		&& !source.matches(&original.metadata().await.map_err(|_| Error::Changed)?)
	{
		return Err(Error::Changed);
	}
	validated_link(&bytes)
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::{io::AsyncWriteExt, net::TcpListener};
	#[test]
	fn limits_and_public_url_admission() {
		assert!(eligible("video.mp4", MAX_BYTES));
		for (name, size) in [
			("file.zip", 0),
			("file.zip", MAX_BYTES + 1),
			("private.DOCX", 1),
			("app.EXE", 1),
			("image.gif", 20_000_001),
		] {
			assert!(!eligible(name, size));
		}
		assert_eq!(
			validated_link(b"https://files.catbox.moe/abc123.png\n").unwrap(),
			"https://files.catbox.moe/abc123.png"
		);
		assert_eq!(
			validated_link(b"https://files.catbox.moe/abc123").unwrap(),
			"https://files.catbox.moe/abc123"
		);
		for link in [
			"http://files.catbox.moe/a.png",
			"https://files.catbox.moe.evil/a.png",
			"https://user@files.catbox.moe/a.png",
			"https://files.catbox.moe:443/a.png",
			"https://files.catbox.moe/a.png?secret=1",
			"https://files.catbox.moe/a%2fb.png",
			"https://files.catbox.moe/../a.png",
			"https://files.catbox.moe/a/../a.png",
			"https://files.catbox.moe/a.png#fragment",
			"https://files.catbox.moe/",
			"https://files.catbox.moe/a\n.png",
		] {
			assert!(validated_link(link.as_bytes()).is_err(), "{link}");
		}
		assert!(validated_link(&vec![b'a'; RESPONSE_BYTES + 1]).is_err());
	}
	async fn read_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
		let mut bytes = [0; 1024];
		let mut request = Vec::new();
		loop {
			let count = socket.read(&mut bytes).await.unwrap();
			assert!(count > 0);
			request.extend_from_slice(&bytes[..count]);
			assert!(request.len() <= 4096);
			if let Some(at) = request.windows(4).position(|s| s == b"\r\n\r\n") {
				let headers = String::from_utf8_lossy(&request[..at]);
				let size = headers
					.lines()
					.find_map(|line| {
						line.to_ascii_lowercase()
							.strip_prefix("content-length: ")
							.and_then(|s| s.parse::<usize>().ok())
					})
					.unwrap();
				if request.len() >= at + 4 + size {
					return request;
				}
			}
		}
	}
	async fn mock_response(response: String) -> (String, tokio::task::JoinHandle<()>) {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let endpoint = format!("http://{}/", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			read_request(&mut socket).await;
			socket.write_all(response.as_bytes()).await.unwrap();
		});
		(endpoint, server)
	}

	#[tokio::test]
	async fn rejection_redirect_and_response_limits_do_not_return_links() {
		let client = reqwest::Client::builder()
			.no_proxy()
			.redirect(reqwest::redirect::Policy::none())
			.retry(reqwest::retry::never())
			.build()
			.unwrap();
		let rejected = Error::Rejected;
		let exceeded = Error::ResponseLimit;
		for (response, expected) in [
			(
				"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".to_owned(),
				rejected,
			),
			(
				"HTTP/1.1 302 Found\r\nLocation: https://example.org/\r\nContent-Length: 0\r\n\r\n"
					.to_owned(),
				rejected,
			),
			(
				"HTTP/1.1 200 OK\r\nContent-Length: 4097\r\n\r\n".to_owned(),
				exceeded,
			),
			(
				format!(
					"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1001\r\n{}\r\n0\r\n\r\n",
					"a".repeat(4097)
				),
				exceeded,
			),
		] {
			let (endpoint, server) = mock_response(response).await;
			let (progress, _) = watch::channel(Status::Preparing);
			assert_eq!(
				transfer(
					&client,
					&endpoint,
					Source::pasted_png(vec![1]).unwrap(),
					progress
				)
				.await
				.unwrap_err(),
				expected
			);

			server.await.unwrap();
		}
	}
	#[tokio::test]
	async fn cancellation_retires_a_stalled_public_upload() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let endpoint = format!("http://{}/", listener.local_addr().unwrap());
		let (ready, received) = tokio::sync::oneshot::channel();
		let server = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			read_request(&mut socket).await;
			let mut bytes = [0; 4096];
			let _ = ready.send(());
			let _ = socket.read(&mut bytes).await;
		});
		let client = reqwest::Client::builder().no_proxy().build().unwrap();
		let (progress, _) = watch::channel(Status::Preparing);
		let (cancel, mut cancellation) = watch::channel(false);
		let task = tokio::spawn(async move {
			attempt(
				&client,
				&endpoint,
				Source::pasted_png(vec![1]).unwrap(),
				progress,
				&mut cancellation,
			)
			.await
		});
		received.await.unwrap();
		cancel.send_replace(true);
		let result = tokio::time::timeout(Duration::from_secs(1), task)
			.await
			.unwrap()
			.unwrap();
		assert_eq!(result.unwrap_err(), Error::Cancelled);
		server.await.unwrap();
	}
	#[tokio::test]
	async fn changed_local_source_is_rejected_before_networking() {
		let path = std::env::temp_dir().join(format!(
			"serein-public-upload-{}-{}.txt",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::SystemTime::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		tokio::fs::write(&path, b"before").await.unwrap();
		let source = Source::inspect(path.clone()).await.unwrap();
		tokio::fs::write(&path, b"changed contents").await.unwrap();
		let (progress, _) = watch::channel(Status::Preparing);
		assert_eq!(
			transfer(
				&reqwest::Client::new(),
				"http://127.0.0.1:1/",
				source,
				progress
			)
			.await
			.unwrap_err(),
			Error::Changed
		);
		tokio::fs::remove_file(path).await.unwrap();
	}
	#[tokio::test]
	async fn local_source_changed_after_streaming_does_not_return_a_link() {
		let path = std::env::temp_dir().join(format!(
			"serein-public-upload-late-{}-{}.txt",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::SystemTime::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		tokio::fs::write(&path, b"synthetic bytes").await.unwrap();
		let source = Source::inspect(path.clone()).await.unwrap();
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let endpoint = format!("http://{}/", listener.local_addr().unwrap());
		let changed_path = path.clone();
		let server = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			read_request(&mut socket).await;
			tokio::fs::write(changed_path, b"changed synthetic bytes")
				.await
				.unwrap();
			let link = "https://files.catbox.moe/abc123.txt";
			socket
				.write_all(
					format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{link}",
						link.len()
					)
					.as_bytes(),
				)
				.await
				.unwrap();
		});
		let (progress, _) = watch::channel(Status::Preparing);
		let result = transfer(
			&reqwest::Client::builder().no_proxy().build().unwrap(),
			&endpoint,
			source,
			progress,
		)
		.await;
		server.await.unwrap();
		tokio::fs::remove_file(path).await.unwrap();
		assert_eq!(result.unwrap_err(), Error::Changed);
	}
	#[tokio::test]
	async fn already_cancelled_upload_never_contacts_service() {
		let (progress, _) = watch::channel(Status::Preparing);
		let (_sender, cancel) = watch::channel(true);
		assert_eq!(
			upload(Source::pasted_png(vec![1]).unwrap(), progress, cancel)
				.await
				.unwrap_err(),
			Error::Cancelled
		);
	}

	#[tokio::test]
	async fn synthetic_multipart_is_streamed_without_credentials() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let endpoint = format!("http://{}/", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			let request = read_request(&mut socket).await;
			let text = String::from_utf8(request).unwrap();
			assert!(!text.to_ascii_lowercase().contains("authorization:"));
			assert!(!text.to_ascii_lowercase().contains("cookie:"));
			assert!(text.contains("name=\"reqtype\"\r\n\r\nfileupload"));
			assert!(text.contains("name=\"fileToUpload\"; filename=\"pasted-image.png\""));
			assert!(text.contains("synthetic bytes"));
			let body = "https://files.catbox.moe/abc123.png";
			socket
				.write_all(
					format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
						body.len()
					)
					.as_bytes(),
				)
				.await
				.unwrap();
		});
		let (progress, _) = watch::channel(Status::Preparing);
		let client = reqwest::Client::builder()
			.no_proxy()
			.redirect(reqwest::redirect::Policy::none())
			.build()
			.unwrap();
		assert_eq!(
			transfer(
				&client,
				&endpoint,
				Source::pasted_png(b"synthetic bytes".to_vec()).unwrap(),
				progress
			)
			.await
			.unwrap(),
			"https://files.catbox.moe/abc123.png"
		);
		server.await.unwrap();
	}
}
