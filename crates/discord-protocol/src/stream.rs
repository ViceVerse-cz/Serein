//! Unofficial normal-user screen-stream Gateway dispatch payloads.
use model::Id;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Key {
	pub stream_key: String,
}

#[derive(Deserialize)]
pub struct Created {
	pub stream_key: String,
	pub rtc_server_id: Id,
	/// Viewer payloads have been seen without it; the RTC server then stands in.
	#[serde(default)]
	pub rtc_channel_id: Option<Id>,
}

#[derive(Deserialize)]
pub struct ServerUpdate {
	pub stream_key: String,
	pub token: String,
	pub endpoint: Option<String>,
}

#[derive(Deserialize)]
pub struct Deleted {
	pub stream_key: String,
	/// Why Discord ended the stream, for example `user_requested` or `stream_ended`.
	#[serde(default)]
	pub reason: Option<String>,
}
