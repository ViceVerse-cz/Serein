//! Host-generated recordings, kept in bounded session memory throughout a single send.
//! Wire contract: https://docs.discord.com/developers/resources/message#voice-messages
use super::{CANCELLED, Source, Status, cancelled, status};
use crate::{DiscordApi, Failure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use client_core::{Command, Event};
use reqwest::Method;
use std::{path::PathBuf, sync::Arc, time::SystemTime};
use tokio::sync::watch;

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DURATION_SECS: f64 = 120.0;
const INVALID: &str = "Invalid voice message; no recording was uploaded";

/// Audio already encoded as Ogg Opus by the native recorder. No paths or plugin bytes.
/// Deliberately neither Debug nor Serialize: recordings cannot enter logs or saved drafts.
pub struct VoiceMessage {
	source: Source,
	duration_secs: f64,
	waveform: String,
}
impl VoiceMessage {
	pub fn new(
		bytes: Arc<[u8]>,
		duration_secs: f64,
		waveform: Vec<u8>,
	) -> Result<Self, &'static str> {
		if bytes.is_empty()
			|| bytes.len() > MAX_BYTES
			|| !duration_secs.is_finite()
			|| duration_secs <= 0.0
			|| duration_secs > MAX_DURATION_SECS
			|| waveform.is_empty()
			|| waveform.len() > 256
		{
			return Err(INVALID);
		}
		Ok(Self {
			source: Source {
				path: PathBuf::new(),
				filename: "voice-message.ogg".into(),
				size: bytes.len() as u64,
				modified: SystemTime::UNIX_EPOCH,
				bytes: Some(bytes),
			},
			duration_secs,
			waveform: STANDARD.encode(waveform),
		})
	}
}

impl DiscordApi {
	/// Upload one native recording and send it once. Caller rechecks channel/voice permissions.
	/// Cancellation after message submission is ambiguous; nothing is automatically replayed.
	pub async fn upload_voice_message(
		&self,
		command: Command,
		voice: VoiceMessage,
		progress: watch::Sender<Status>,
		mut cancel: watch::Receiver<bool>,
	) -> Event {
		let Command::Send {
			channel,
			content,
			nonce,
			reply,
			sticker,
		} = command
		else {
			progress.send_replace(Status::Failed(INVALID));
			return Event::Failure(Failure::ProtocolAt(INVALID));
		};
		if channel.0 == 0
			|| !content.is_empty()
			|| sticker.is_some()
			|| nonce.is_empty()
			|| nonce.len() > 128
			|| nonce.chars().any(char::is_control)
		{
			progress.send_replace(Status::Failed(INVALID));
			return Event::SendResult {
				nonce,
				result: Err(Failure::ProtocolAt(INVALID)),
			};
		}
		progress.send_replace(Status::Preparing);
		let prepared = tokio::select! {
			biased;
			_ = cancelled(&mut cancel) => Err(Failure::ProtocolAt(CANCELLED)),
			result = self.upload_file(channel, &voice.source, 0, &progress, 0, voice.source.size()) => result,
		};
		let result = match prepared {
			Err(failure) => Err(failure),
			Ok(_) if *cancel.borrow() || cancel.has_changed().is_err() => {
				Err(Failure::ProtocolAt(CANCELLED))
			}
			Ok(mut attachment) => {
				attachment["duration_secs"] = serde_json::json!(voice.duration_secs);
				attachment["waveform"] = serde_json::json!(voice.waveform);
				let mut body = serde_json::json!({
					"content":"", "nonce":nonce, "flags":8192,
					"allowed_mentions":crate::allowed_mentions("", reply),
					"attachments":[attachment]
				});
				if let Some(reply) = reply {
					body["message_reference"] =
						serde_json::json!({"message_id":reply.target(),"channel_id":channel});
				}
				progress.send_replace(Status::Sending);
				let path = format!("/channels/{channel}/messages");
				tokio::select! {
					biased;
					_ = cancelled(&mut cancel) => Err(Failure::Ambiguous),
					result = self.request_limited(Method::POST, &path, Some(body), crate::MAX_WIRE) => {
						result.and_then(|bytes| {
							let message = discord_protocol::decode::<discord_protocol::MessageDto>(&bytes)
								.map_err(|_| Failure::Ambiguous)?;
							if message.channel_id != channel { return Err(Failure::Ambiguous); }
							Ok(message.into_model())
						})
					}
				}
			}
		};
		progress.send_replace(status(&result));
		Event::SendResult { nonce, result }
	}
}
