//! HTTPS requests to the host names a `network` plugin declares in `network_hosts`.
//!
//! A request returns at once. Its [`FetchResult`] arrives later by running the same action
//! again with the same input plus `fetch_result`, so a message action can answer with the
//! translated text in its result. Every request gets exactly one result.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Most requests one output may start.
pub const MAX_FETCHES: usize = 4;
/// Most request headers a plugin may set; `Cookie`, `Host` and framing headers are refused.
pub const MAX_FETCH_HEADERS: usize = 16;
/// Largest response body handed back; larger responses fail with [`FetchError::TooLarge`].
pub const MAX_FETCH_RESPONSE_BYTES: usize = 192 * 1024;
/// Largest request body a plugin may send.
pub const MAX_FETCH_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FetchMethod {
	Get,
	Post,
	Put,
	Patch,
	Delete,
}

/// One HTTPS request to a host the manifest lists. The host adds no cookies or Discord
/// credentials; `headers` may carry the plugin's own API key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchRequest {
	/// Chosen by the plugin, at most 64 bytes, and echoed in the matching [`FetchResult`].
	pub request_id: String,
	pub method: FetchMethod,
	pub url: String,
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub headers: BTreeMap<String, String>,
	/// Not allowed with `GET`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<String>,
}

impl FetchRequest {
	/// A `GET` request with no headers.
	pub fn get(request_id: impl Into<String>, url: impl Into<String>) -> Self {
		Self {
			request_id: request_id.into(),
			method: FetchMethod::Get,
			url: url.into(),
			headers: BTreeMap::new(),
			body: None,
		}
	}
}

/// Why a request produced no HTTP response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchError {
	Unavailable,
	TimedOut,
	TooLarge,
	NotText,
	/// A redirect or address left the declared hosts or the public internet.
	Blocked,
	/// Too many requests were already running, or the original input plus the response no
	/// longer fit one invocation.
	Cancelled,
}

/// The outcome of one [`FetchRequest`]: an HTTP status with its body, or an error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchResult {
	pub request_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub status: Option<u16>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub content_type: Option<String>,
	#[serde(default)]
	pub body: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<FetchError>,
}

impl FetchResult {
	/// Decode a successful (2xx) JSON body. Failures and other statuses are `None`.
	pub fn json<T: serde::de::DeserializeOwned>(&self) -> Option<T> {
		self.status
			.filter(|status| (200..300).contains(status))
			.and_then(|_| serde_json::from_str(&self.body).ok())
	}
}

/// Any action's input that may carry a web response; `T` is the input type the action
/// already decodes, such as `MessageActionInvocation`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInvocation<T = crate::ExtendedAppInvocation> {
	#[serde(flatten)]
	pub invocation: T,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fetch_result: Option<FetchResult>,
}

/// Output that starts web requests, preserving `AppOutput`. Requires `network`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkOutput {
	#[serde(flatten)]
	pub output: crate::AppOutput,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub fetches: Vec<FetchRequest>,
}
