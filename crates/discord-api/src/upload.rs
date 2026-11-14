//! User-selected files, staged to Discord's signed storage target before message creation.
//! Paths, signed URLs and file bytes are never serialized into diagnostics or retained as drafts.
pub mod external;

use crate::{DiscordApi, Failure};
use client_core::{
	Command, Event,
	captcha::{self, Challenge, Retry},
};
use std::{
	path::PathBuf,
	time::{Duration, SystemTime},
};
use tokio::{fs::File, io::AsyncReadExt, sync::watch};
use url::Url;
use wreq::Method;

pub const MAX_BYTES: u64 = 500_000_000;
pub const MAX_FILES: usize = 10;
pub const MAX_TOTAL_BYTES: u64 = MAX_BYTES;
const CHUNK_BYTES: usize = 64 * 1024;
const MAX_RESPONSE: usize = 64 * 1024;
const CANCELLED: &str = "Upload cancelled; no message was sent";
const CHANGED: &str = "Selected file changed or disappeared; select it again";
const UNVERIFIED: &str = "Verification expired; retry the message";

/// The attachment send waiting on the user's solution, and where that solution goes.
pub(crate) type Waiting = (captcha::Target, tokio::sync::oneshot::Sender<Box<Retry>>);
/// How a challenged attachment send reaches the session.
#[derive(Clone, Copy)]
pub struct Challenged<'a> {
	/// Shows the challenge to the user; false when nobody can show it.
	pub show: &'a (dyn Fn(Event) -> bool + Sync),
	/// Awaited after the user's answer and before the replay. The session lets other writes
	/// through while the user decides and takes its turn back here.
	pub resume: &'a (dyn Fn() -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + 'a>> + Sync),
}

/// Shows one challenged request of an attachment send to the user and waits for the answer.
/// Staged files stay where they are meanwhile, so a solved message never transfers twice.
struct Verifier<'a, 'c> {
	api: &'a DiscordApi,
	channel: model::Id,
	nonce: &'a str,
	reply: Option<client_core::Reply>,
	challenged: Challenged<'c>,
	progress: &'a watch::Sender<Status>,
}
impl Verifier<'_, '_> {
	async fn solve(&self, challenge: Challenge) -> Result<Box<Retry>, Failure> {
		/// Frees the slot only while it still belongs to this send.
		struct Release<'a>(&'a DiscordApi, &'a captcha::Target);
		impl Drop for Release<'_> {
			fn drop(&mut self) {
				if let Ok(mut waiting) = self.0.upload_verification.lock() {
					waiting.take_if(|(target, _)| target == self.1);
				}
			}
		}
		let target = captcha::Target::Message {
			channel: self.channel,
			nonce: self.nonce.to_owned(),
		};
		let (send, solved) = tokio::sync::oneshot::channel();
		{
			let mut waiting = self
				.api
				.upload_verification
				.lock()
				.map_err(|_| Failure::Protocol)?;
			// One send waits at a time; a second one must not take over the first's answer.
			if waiting.is_some() {
				return Err(Failure::ProtocolAt(
					"Verification unavailable; no message was sent",
				));
			}
			*waiting = Some((target.clone(), send));
		}
		let _release = Release(self.api, &target);
		self.progress.send_replace(Status::Verifying);
		if !(self.challenged.show)(Event::SendChallenge {
			nonce: self.nonce.to_owned(),
			reply: self.reply,
			challenge: Box::new(challenge),
		}) {
			return Err(Failure::ProtocolAt(
				"Verification unavailable; no message was sent",
			));
		}
		match tokio::time::timeout(captcha::LIFETIME, solved).await {
			Ok(Ok(retry)) if retry.matches_target(&target) => {
				(self.challenged.resume)().await;
				Ok(retry)
			}
			_ => Err(Failure::ProtocolAt(UNVERIFIED)),
		}
	}
}

/// VenSerein (AnonymiseFileNames): Global toggle for anonymizing uploaded file names.
static ANONYMISE_FILENAMES: std::sync::atomic::AtomicBool =
	std::sync::atomic::AtomicBool::new(false);

/// VenSerein: Enable or disable AnonymiseFileNames mode.
pub fn set_anonymise_filenames(enable: bool) {
	ANONYMISE_FILENAMES.store(enable, std::sync::atomic::Ordering::Relaxed);
}

/// VenSerein: Check if AnonymiseFileNames mode is enabled.
pub fn anonymise_filenames() -> bool {
	ANONYMISE_FILENAMES.load(std::sync::atomic::Ordering::Relaxed)
}

/// VenSerein: Anonymise a filename while preserving its file extension.
pub fn anonymise_filename(original: &str) -> String {
	let (stem, ext) = match original.rsplit_once('.') {
		Some((s, e)) => (s, Some(e)),
		None => (original, None),
	};
	let mut hash = 0x811c9dc5u32;
	for b in stem.as_bytes() {
		hash ^= *b as u32;
		hash = hash.wrapping_mul(0x01000193);
	}
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_millis();
	hash ^= (now as u32) & 0xffff;
	match ext {
		Some(ext) => format!("upload_{:08x}.{ext}", hash),
		None => format!("upload_{:08x}", hash),
	}
}

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
			return Err("Choose a nonempty file up to Discord's 500 MB maximum");
		}
		let final_filename = if anonymise_filenames() {
			anonymise_filename(filename)
		} else {
			filename.into()
		};
		Ok(Self {
			bytes: None,
			filename: final_filename,
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
			return Err("Choose a nonempty image up to Discord's 500 MB maximum");
		}
		Ok(Self {
			path: PathBuf::new(),
			filename: "pasted-image.png".into(),
			size: bytes.len() as u64,
			modified: SystemTime::UNIX_EPOCH,
			bytes: Some(bytes.into()),
		})
	}
	/// Pasted text too long for one message becomes an in-memory `message.txt`, like Discord.
	pub fn pasted_text(text: String) -> Result<Self, &'static str> {
		if text.is_empty() || text.len() as u64 > MAX_BYTES {
			return Err("Choose nonempty text up to Discord's 500 MB maximum");
		}
		Ok(Self {
			path: PathBuf::new(),
			filename: "message.txt".into(),
			size: text.len() as u64,
			modified: SystemTime::UNIX_EPOCH,
			bytes: Some(text.into_bytes().into()),
		})
	}
	/// Public artwork already decoded and validated by the host image worker.
	pub fn image_bytes(filename: String, bytes: Vec<u8>) -> Result<Self, &'static str> {
		let valid_name = filename
			.strip_prefix("emoji-")
			.or_else(|| filename.strip_prefix("sticker-"))
			.and_then(|name| name.rsplit_once('.'))
			.is_some_and(|(id, extension)| {
				matches!(extension, "png" | "gif")
					&& id.bytes().all(|b| b.is_ascii_digit())
					&& id.parse::<model::Id>().is_ok_and(|id| id.0 != 0)
			});
		if !valid_name || filename.len() > 40 || bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
			return Err("Choose valid emoji or sticker artwork up to 8 MiB");
		}
		Ok(Self {
			path: PathBuf::new(),
			filename,
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
	/// Bytes this selection holds in session memory; chosen files hold only their metadata.
	pub fn resident_bytes(&self) -> u64 {
		self.bytes.as_ref().map_or(0, |bytes| bytes.len() as u64)
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
	Uploading {
		sent: u64,
		total: u64,
	},
	/// Discord challenged this send; nothing moves until the user solves or dismisses it.
	Verifying,
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
	Interaction(client_core::interactions::Request),
	Send {
		nonce: String,
		reply: Option<client_core::Reply>,
	},
	Post {
		guild: model::Id,
		title: String,
		tags: Vec<model::Id>,
		request: u64,
	},
}
impl Destination {
	/// Report a failure as the event the caller's flow expects; no write was accepted.
	fn failed(self, channel: model::Id, failure: Failure) -> Event {
		match self {
			Self::Interaction(request) => {
				Event::Interaction(client_core::interactions::Event::Submitted {
					nonce: request.nonce,
					result: Err(failure),
				})
			}
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
		self.upload_messages(command, vec![source], progress, cancel, None)
			.await
	}

	/// Hands the user's solution to the attachment send waiting on it. Returns the failure to
	/// report when that send is gone: a text resend would silently drop its files.
	pub fn resume_upload(
		&self,
		channel: model::Id,
		nonce: String,
		captcha: Box<Retry>,
	) -> Option<Event> {
		let target = captcha::Target::Message { channel, nonce };
		let waiting = self
			.upload_verification
			.lock()
			.ok()
			.and_then(|mut waiting| waiting.take_if(|(waiting, _)| *waiting == target));
		if waiting.is_some_and(|(_, solved)| solved.send(captcha).is_ok()) {
			return None;
		}
		let captcha::Target::Message { nonce, .. } = target else {
			unreachable!()
		};
		Some(Event::SendResult {
			nonce,
			result: Err(Failure::ProtocolAt(UNVERIFIED)),
		})
	}

	pub async fn upload_messages(
		&self,
		command: Command,
		sources: Vec<Source>,
		progress: watch::Sender<Status>,
		mut cancel: watch::Receiver<bool>,
		challenged: Option<Challenged<'_>>,
	) -> Event {
		let (channel, content, target) = match command {
			Command::Interaction(request) => {
				if !request.valid()
					|| !crate::interactions::valid_uploads(&request, sources.len())
					|| !crate::interactions::valid_file_types(&request, &sources)
					|| !matches!(&request.data, client_core::interactions::Data::Modal { .. })
				{
					progress
						.send_replace(Status::Failed("Invalid modal upload; reselect the files"));
					return Event::Interaction(client_core::interactions::Event::Submitted {
						nonce: request.nonce,
						result: Err(Failure::ProtocolAt(
							"Invalid modal upload; reselect the files",
						)),
					});
				}
				(
					request.channel_id,
					String::new(),
					Destination::Interaction(request),
				)
			}
			Command::Send {
				channel,
				content,
				nonce,
				reply,
				sticker: None,
			} => (channel, content, Destination::Send { nonce, reply }),
			// A forum post is one request: its files are staged before the thread exists.
			Command::CreatePost {
				parent,
				guild,
				title,
				content,
				tags,
				request,
				..
			} => (
				parent,
				content,
				Destination::Post {
					guild,
					title,
					tags,
					request,
				},
			),
			_ => {
				progress.send_replace(Status::Failed("Invalid upload request"));
				return Event::Failure(Failure::ProtocolAt("Invalid upload request"));
			}
		};
		if content.chars().count() > client_core::MAX_PREMIUM_CONTENT {
			let failure = Failure::ProtocolAt("Message is too long; no file was uploaded");
			progress.send_replace(Status::Failed(failure.label()));
			return target.failed(channel, failure);
		}
		if sources.is_empty()
			|| sources.len() > MAX_FILES
			|| sources.iter().map(Source::size).sum::<u64>() > MAX_TOTAL_BYTES
		{
			let failure = Failure::ProtocolAt(
				"Choose up to 10 files totaling at most 500 MB; account limits may be lower",
			);
			progress.send_replace(Status::Failed(failure.label()));
			return target.failed(channel, failure);
		}
		let total = sources.iter().map(Source::size).sum::<u64>();
		// Only a message has a pending row the user can verify; a modal or forum post that
		// Discord challenges still fails with the existing "use the official client" reason.
		let verifier = match (&target, challenged) {
			(Destination::Send { nonce, reply }, Some(challenged)) => Some(Verifier {
				api: self,
				channel,
				nonce,
				reply: *reply,
				challenged,
				progress: &progress,
			}),
			_ => None,
		};
		let verifier = verifier.as_ref();
		progress.send_replace(Status::Preparing);
		let prepare = async {
			for source in &sources {
				source.validate().await?;
			}
			let mut attachments = Vec::with_capacity(sources.len());
			let mut completed = 0;
			for (id, source) in sources.iter().enumerate() {
				attachments.push(
					self.upload_file(channel, source, id, &progress, (completed, total), verifier)
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
		if let (Destination::Send { nonce, reply }, Some(verifier)) = (&target, verifier) {
			let mut challenge = None;
			let mut result = tokio::select! {
				biased;
				_ = cancelled(&mut cancel) => Err(Failure::Ambiguous),
				result = self.send_message_with_captcha(channel, &content, nonce, (*reply, Some(attachment.clone()), None), None, Some(&mut challenge)) => result,
			};
			if let Some(challenge) = challenge {
				// Discord refused the challenged POST, so no message exists while the user
				// decides: cancelling here is still a clean "nothing was sent".
				let solved = tokio::select! {
					biased;
					_ = cancelled(&mut cancel) => Err(Failure::ProtocolAt(CANCELLED)),
					solved = verifier.solve(challenge) => solved,
				};
				result = match solved {
					Ok(retry) => {
						progress.send_replace(Status::Sending);
						// The one replay reuses the staged names; a second challenge is final.
						tokio::select! {
							biased;
							_ = cancelled(&mut cancel) => Err(Failure::Ambiguous),
							result = self.send_message_with_captcha(channel, &content, nonce, (*reply, Some(attachment), None), Some(&retry), None) => result,
						}
					}
					Err(failure) => Err(failure),
				};
			}
			progress.send_replace(status(&result));
			let Destination::Send { nonce, .. } = target else {
				unreachable!()
			};
			return Event::SendResult { nonce, result };
		}
		match target {
			Destination::Interaction(request) => {
				let result = tokio::select! {
					biased;
					_ = cancelled(&mut cancel) => Err(Failure::Ambiguous),
					result = self.interaction(&request,Some(attachment)) => result,
				};
				progress.send_replace(status(&result));
				Event::Interaction(client_core::interactions::Event::Submitted {
					nonce: request.nonce,
					result,
				})
			}
			Destination::Send { nonce, reply } => {
				let result = tokio::select! {
					biased;
					_ = cancelled(&mut cancel) => Err(Failure::Ambiguous),
					result = self.send_message(channel, &content, &nonce, reply, Some(attachment), None) => result,
				};
				progress.send_replace(status(&result));
				Event::SendResult { nonce, result }
			}
			Destination::Post {
				guild,
				title,
				tags,
				request,
			} => {
				let result = tokio::select! {
					biased;
					_ = cancelled(&mut cancel) => Err(Failure::Ambiguous),
					result = self.create_post(channel, guild, (&title, &tags), &content, Some(attachment)) => result,
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
		(completed, batch_total): (u64, u64),
		verifier: Option<&Verifier<'_, '_>>,
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
		let mut body = serde_json::json!({"files":[{"id":id.to_string(),"filename":source.filename(),"file_size":source.size(),"is_clip":false}]});
		if let Some(mime) = mime_type(source.filename()) {
			body["files"][0]["original_content_type"] = mime.into();
		}
		let path = format!("/channels/{channel}/attachments");
		let mut challenge = None;
		let mut response = self
			.request_with_captcha(
				Method::POST,
				&path,
				Some(body.clone()),
				MAX_RESPONSE,
				None,
				verifier.map(|_| &mut challenge),
			)
			.await;
		if let (Some(challenge), Some(verifier)) = (challenge, verifier) {
			let retry = verifier.solve(challenge).await?;
			progress.send_replace(Status::Preparing);
			// One replay of this file's staging request; a second challenge is final.
			response = self
				.request_with_captcha(
					Method::POST,
					&path,
					Some(body),
					MAX_RESPONSE,
					Some(&retry),
					None,
				)
				.await;
		}
		let response = response.map_err(|f| {
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
		if let Some(failure) = self.stop_reason() {
			return Err(failure);
		}
		let client = self
			.upload_client
			.get_or_try_init(|| async {
				crate::browser_transport(wreq::Client::builder())
					.redirect(wreq::redirect::Policy::none())
					.retry(wreq::retry::Policy::never())
					.no_proxy()
					.connect_timeout(Duration::from_secs(10))
					.read_timeout(Duration::from_secs(30))
					.timeout(Duration::from_secs(300))
					.pool_max_idle_per_host(1)
					.pool_idle_timeout(Duration::from_secs(30))
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
		let response = client
			.put(url.as_str())
			// Unofficial: the web client labels every stored file as opaque bytes.
			.header(wreq::header::CONTENT_TYPE, "application/octet-stream")
			.header(wreq::header::CONTENT_LENGTH, total)
			.body(wreq::Body::wrap_stream(stream))
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
		let mut body = std::pin::pin!(response.bytes_stream());
		while let Some(chunk) = futures_util::StreamExt::next(&mut body)
			.await
			.transpose()
			.map_err(|_| Failure::Network)?
		{
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
		if let Some(failure) = self.stop_reason() {
			return Err(failure);
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

/// MIME type declared as `original_content_type` when staging, from the filename extension.
/// The web client sends the browser's `File.type` and omits the field when that is empty,
/// so an extension without a well-known type is left out rather than guessed.
fn mime_type(filename: &str) -> Option<&'static str> {
	let extension = filename
		.rsplit_once('.')
		.map(|(_, e)| e.to_ascii_lowercase())
		.unwrap_or_default();
	Some(match extension.as_str() {
		"png" => "image/png",
		"jpg" | "jpeg" => "image/jpeg",
		"gif" => "image/gif",
		"webp" => "image/webp",
		"heic" => "image/heic",
		"heif" => "image/heif",
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
		"txt" => "text/plain",
		"md" => "text/markdown",
		"csv" => "text/csv",
		_ => return None,
	})
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
	#[test]
	fn test_venserein_anonymise_filenames() {
		assert!(!super::anonymise_filenames());
		super::set_anonymise_filenames(true);
		assert!(super::anonymise_filenames());

		let anon = super::anonymise_filename("Personal_Resume_2026.pdf");
		assert!(anon.starts_with("upload_"));
		assert!(anon.ends_with(".pdf"));
		assert!(!anon.contains("Personal"));

		let anon_no_ext = super::anonymise_filename("secret_notes");
		assert!(anon_no_ext.starts_with("upload_"));
		assert!(!anon_no_ext.contains("secret"));

		super::set_anonymise_filenames(false);
	}
	#[test]
	fn image_sources_only_accept_bounded_generated_raster_names() {
		for name in ["emoji-7.gif", "sticker-8.png"] {
			let source = super::Source::image_bytes(name.into(), vec![1]).unwrap();
			assert_eq!(source.filename(), name);
			assert_eq!(source.size(), 1);
		}
		for name in [
			"emoji-0.png",
			"emoji-+7.png",
			"sticker-7.json",
			"../emoji-7.png",
			"sticker-7.png?x=1",
		] {
			assert!(super::Source::image_bytes(name.into(), vec![1]).is_err());
		}
		assert!(super::Source::image_bytes("emoji-7.png".into(), vec![]).is_err());
		assert!(
			super::Source::image_bytes("emoji-7.png".into(), vec![0; 8 * 1024 * 1024 + 1]).is_err()
		);
	}
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
			sticker: None,
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
		{
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
	                assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(), serde_json::json!({"files":[{"id":id.to_string(),"filename":filename,"file_size":CHUNK_BYTES*2+9,"is_clip":false,"original_content_type":"text/plain"}]}));
	                respond(&mut socket, "200 OK", &serde_json::json!({"attachments":[{"id":id,"upload_url":upload_url,"upload_filename":format!("synthetic-upload/{id}/file.txt")}]}).to_string()).await;
	                let (mut socket, _) = storage.accept().await.unwrap();
	                let (head, bytes) = request(&mut socket).await;
	                assert!(head.starts_with("PUT /signed?upload_id=synthetic HTTP/1.1"));
	                assert!(!head.to_ascii_lowercase().contains("authorization"));
	                assert!(!head.to_ascii_lowercase().contains("cookie"));
	                assert!(!head.contains("SYNTHETIC_UPLOAD_TOKEN"));
	                assert!(!head.to_ascii_lowercase().contains("x-super-properties"));
	                assert!(head.to_ascii_lowercase().contains("content-type: application/octet-stream"));
	                assert!(!head.to_ascii_lowercase().contains("content-md5"));
	                assert_eq!(bytes, vec![b'x'; CHUNK_BYTES*2+9]);
	                respond(&mut socket, "200 OK", "").await;
	                }
	                let (mut socket, _) = api_listener.accept().await.unwrap();
	                let (head, bytes) = request(&mut socket).await;
	                assert!(head.starts_with("POST /channels/1/messages HTTP/1.1"));
	                assert!(head.contains("SYNTHETIC_UPLOAD_TOKEN"));
	                let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
	                assert_eq!(body, serde_json::json!({
	                    "mobile_network_type":"unknown","content":"","nonce":"synthetic-upload","tts":false,"flags":0,
	                    "allowed_mentions":{"parse":[],"users":[],"roles":[],"replied_user":true},
	                    "message_reference":{"message_id":"2","channel_id":"1"},
	                    "attachments":[{"id":"0","filename":filename,"uploaded_filename":"synthetic-upload/0/file.txt"},{"id":"1","filename":filename,"uploaded_filename":"synthetic-upload/1/file.txt"}],
	                }));
	                respond(&mut socket, "200 OK", r#"{"id":"3","channel_id":"1","author":{"id":"4","username":"Synthetic"},"nonce":"synthetic-upload"}"#).await;
	            });
	            let (progress, status) = watch::channel(Status::Preparing);
	            let (_cancel, cancelled) = watch::channel(false);
	            assert!(matches!(api.upload_messages(command(), vec![source.clone(), source], progress, cancelled, None).await, Event::SendResult { result: Ok(message), .. } if message.id == model::Id(3)));
	            assert_eq!(*status.borrow(), Status::Finished);
	            server.await.unwrap();
	        }).await.unwrap();
		}
		{
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
			large.set_len(20_000_001).await.unwrap();
			assert!(Source::inspect(empty.0.clone()).await.is_ok());
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
	}

	/// Serves one API request and returns its socket and body. With `challenge`, the first
	/// attempt is answered with a captcha and the attempt returned is its solved replay.
	async fn exchange(
		listener: &TcpListener,
		start: &str,
		challenge: bool,
	) -> (TcpStream, serde_json::Value) {
		let mut first = None;
		loop {
			let (mut socket, _) = listener.accept().await.unwrap();
			let (head, bytes) = request(&mut socket).await;
			assert!(head.starts_with(start), "{head}");
			let head = head.to_ascii_lowercase();
			let solved = first.is_some();
			assert_eq!(head.contains("x-captcha-key: synthetic-solution"), solved);
			assert_eq!(
				head.contains("x-captcha-rqtoken: synthetic-rqtoken"),
				solved
			);
			assert_eq!(
				head.contains("x-captcha-session-id: synthetic-session"),
				solved
			);
			let body = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
			if challenge && !solved {
				respond(&mut socket, "400 Bad Request", r#"{"captcha_key":["required"],"captcha_service":"hcaptcha","captcha_sitekey":"synthetic-sitekey","captcha_rqdata":"synthetic-rqdata","captcha_rqtoken":"synthetic-rqtoken","captcha_session_id":"synthetic-session"}"#).await;
				first = Some(body);
				continue;
			}
			// The replay is the same request.
			assert!(first.is_none_or(|first| first == body));
			return (socket, body);
		}
	}

	/// A challenge on staging replays only that staging request, and one on message create
	/// replays only that POST: the file is transferred once either way.
	#[tokio::test]
	async fn challenged_attachment_requests_replay_once_without_transferring_again() {
		for staging in [false, true] {
			tokio::time::timeout(Duration::from_secs(10), async {
				let fixture = Fixture::new(b"synthetic").await;
				let source = Source::inspect(fixture.0.clone()).await.unwrap();
				let api_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let storage = TcpListener::bind("127.0.0.1:0").await.unwrap();
				let mut api = api();
				api.base = format!("http://{}", api_listener.local_addr().unwrap());
				api.upload_origin = Some(storage.local_addr().unwrap());
				let target = format!("http://{}/signed", storage.local_addr().unwrap());
				let server = tokio::spawn(async move {
					let (mut socket, _) =
						exchange(&api_listener, "POST /channels/1/attachments HTTP/1.1", staging).await;
					respond(&mut socket, "200 OK", &serde_json::json!({"attachments":[{"upload_url":target,"upload_filename":"synthetic/file"}]}).to_string()).await;
					let (mut socket, _) = storage.accept().await.unwrap();
					request(&mut socket).await;
					respond(&mut socket, "200 OK", "").await;
					let (mut socket, body) =
						exchange(&api_listener, "POST /channels/1/messages HTTP/1.1", !staging).await;
					assert_eq!(body["attachments"][0]["uploaded_filename"], "synthetic/file");
					respond(&mut socket, "200 OK", r#"{"id":"3","channel_id":"1","author":{"id":"4","username":"Synthetic"}}"#).await;
					assert!(tokio::time::timeout(Duration::from_millis(50), api_listener.accept()).await.is_err());
					assert!(tokio::time::timeout(Duration::from_millis(50), storage.accept()).await.is_err());
				});
				let mut state = client_core::State {
					channels: vec![model::Channel {
						id: model::Id(1),
						guild: None,
						parent_id: None,
						kind: 1,
						name: "Synthetic DM".into(),
						position: 0,
						recipients: vec![],
						last_message: None,
						icon: None,
						member_list_id: None,
						tags: None,
						message_count: None,
						user_limit: None,
					}],
					selected: Some(model::Id(1)),
					auth: client_core::auth::AuthState::Authenticated,
					freshness: model::Freshness::Fresh,
					gateway_connected: true,
					..Default::default()
				};
				let command = state
					.prepare_send_with_attachment(Some(source.filename()))
					.unwrap();
				let (events, mut challenges) = tokio::sync::mpsc::unbounded_channel();
				let show = move |event| events.send(event).is_ok();
				let resumed = std::sync::atomic::AtomicBool::new(false);
				let resume = || -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
					Box::pin(async { resumed.store(true, std::sync::atomic::Ordering::SeqCst) })
				};
				let (progress, status) = watch::channel(Status::Preparing);
				let (_cancel, cancelled) = watch::channel(false);
				let upload = api.upload_messages(
					command,
					vec![source],
					progress,
					cancelled,
					Some(Challenged {
						show: &show,
						resume: &resume,
					}),
				);
				let solve = async {
					let event = challenges.recv().await.unwrap();
					assert!(matches!(event, Event::SendChallenge { .. }));
					assert_eq!(*status.borrow(), Status::Verifying);
					assert!(!api.stopped());
					state.apply(client_core::Envelope {
						generation: state.generation,
						event,
					});
					let (verification, _) = state.verification().unwrap();
					// The solved row resumes the waiting upload; a text resend would drop its file.
					let Some(Command::VerifiedUpload {
						channel,
						nonce,
						captcha,
					}) = state.resume_verification(
						verification,
						captcha::Solution::new("synthetic-solution".into()).unwrap(),
					)
					else {
						panic!("expected the upload to be resumed");
					};
					assert!(api.resume_upload(channel, nonce, captcha).is_none());
				};
				let (event, ()) = tokio::join!(upload, solve);
				assert!(
					matches!(event, Event::SendResult { result: Ok(message), .. } if message.id == model::Id(3))
				);
				assert_eq!(*status.borrow(), Status::Finished);
				// The session's turn is taken back before the replay goes out.
				assert!(resumed.load(std::sync::atomic::Ordering::SeqCst));
				server.await.unwrap();
			})
			.await
			.unwrap();
		}
	}

	#[tokio::test]
	async fn cancelling_staging_verification_never_puts_or_posts_a_message() {
		use futures_util::FutureExt;
		let fixture = Fixture::new(b"synthetic").await;
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let storage = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = api();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		api.upload_origin = Some(storage.local_addr().unwrap());
		let server = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			let (head, _) = request(&mut socket).await;
			assert!(head.starts_with("POST /channels/1/attachments HTTP/1.1"));
			respond(&mut socket, "400 Bad Request", r#"{"captcha_key":["required"],"captcha_service":"hcaptcha","captcha_sitekey":"synthetic-sitekey"}"#).await;
			listener
		});
		let (cancel, cancelled) = watch::channel(false);
		let show = |event| {
			assert!(matches!(event, Event::SendChallenge { .. }));
			cancel.send_replace(true);
			true
		};
		let resume = || -> std::pin::Pin<Box<dyn Future<Output = ()> + Send + '_>> {
			Box::pin(async { panic!("cancelled staging must not resume") })
		};
		let (progress, status) = watch::channel(Status::Preparing);
		let event = api
			.upload_messages(
				command(),
				vec![Source::inspect(fixture.0.clone()).await.unwrap()],
				progress,
				cancelled,
				Some(Challenged {
					show: &show,
					resume: &resume,
				}),
			)
			.await;
		assert_eq!(failed(event), Failure::ProtocolAt(CANCELLED));
		assert_eq!(*status.borrow(), Status::Cancelled);
		assert!(api.upload_verification.lock().unwrap().is_none());
		let listener = server.await.unwrap();
		assert!(
			listener.accept().now_or_never().is_none(),
			"staging replay or message create was dispatched"
		);
		assert!(
			storage.accept().now_or_never().is_none(),
			"storage PUT was dispatched"
		);
	}

	#[tokio::test]
	async fn upload_cancel_before_write_and_redirect_never_send_message() {
		{
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
		{
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
		{
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
}
