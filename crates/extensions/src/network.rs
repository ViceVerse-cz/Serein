//! Plugin HTTPS requests limited to the host names a manifest declares.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Error, MAX_IO_BYTES};

/// Most host names one manifest may declare in `network_hosts`.
pub const MAX_NETWORK_HOSTS: usize = 16;
/// Most requests one output may start.
pub const MAX_FETCHES: usize = 4;
/// Most request headers a plugin may set.
pub const MAX_FETCH_HEADERS: usize = 16;
/// Largest response body handed back to a plugin; it must also fit one invocation.
pub const MAX_FETCH_RESPONSE_BYTES: usize = 192 * 1024;
/// Largest request body a plugin may send.
pub const MAX_FETCH_BODY_BYTES: usize = 64 * 1024;
const MAX_URL_BYTES: usize = 4096;

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
#[serde(deny_unknown_fields)]
pub struct FetchRequest {
	/// Chosen by the plugin and echoed in the matching [`FetchResult`].
	pub request_id: String,
	pub method: FetchMethod,
	pub url: String,
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub headers: BTreeMap<String, String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<String>,
}

/// Why a request produced no HTTP response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchError {
	/// The host could not be resolved, reached or completed the exchange.
	Unavailable,
	TimedOut,
	/// The response was larger than [`MAX_FETCH_RESPONSE_BYTES`] or compressed.
	TooLarge,
	/// The response was not UTF-8 text.
	NotText,
	/// A redirect or address left the declared hosts or the public internet.
	Blocked,
	/// The plugin was disabled, the account changed or too many requests were waiting.
	Cancelled,
}

/// The outcome of one [`FetchRequest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchResult {
	pub request_id: String,
	/// The HTTP status; absent when `error` is set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub status: Option<u16>,
	/// The response's `Content-Type`, when it sent one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub content_type: Option<String>,
	#[serde(default)]
	pub body: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<FetchError>,
}

/// Whether `host` is a lowercase DNS name with at least two labels, not an address.
pub fn valid_network_host(host: &str) -> bool {
	host.len() <= 253
		&& host.contains('.')
		&& host.parse::<std::net::IpAddr>().is_err()
		&& host.split('.').all(|label| {
			!label.is_empty()
				&& label.len() <= 63
				&& !label.starts_with('-')
				&& !label.ends_with('-')
				&& label
					.bytes()
					.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
		}) && host
		.rsplit('.')
		.next()
		.is_some_and(|tld| !tld.bytes().all(|b| b.is_ascii_digit()))
}

/// The host name of a credential-free `https` URL on the default port, if it is one.
pub fn https_host(url: &str) -> Option<String> {
	if url.len() > MAX_URL_BYTES || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
		return None;
	}
	let parsed = url::Url::parse(url).ok()?;
	if parsed.scheme() != "https"
		|| !parsed.username().is_empty()
		|| parsed.password().is_some()
		|| parsed.port().is_some_and(|port| port != 443)
		|| parsed.fragment().is_some()
	{
		return None;
	}
	match parsed.host()? {
		url::Host::Domain(host) => Some(host.to_owned()),
		_ => None,
	}
}

/// Headers the host owns: connection framing, cookies and anything naming another origin.
fn reserved_header(name: &str) -> bool {
	let name = name.to_ascii_lowercase();
	matches!(
		name.as_str(),
		"host"
			| "cookie"
			| "connection"
			| "content-length"
			| "transfer-encoding"
			| "te" | "trailer"
			| "upgrade"
			| "keep-alive"
			| "proxy-authorization"
			| "proxy-connection"
			| "accept-encoding"
			| "origin"
			| "referer"
			| "expect"
			| "via"
	) || name.starts_with("sec-")
		|| name.starts_with("proxy-")
}

fn valid_header_name(name: &str) -> bool {
	!name.is_empty()
		&& name.len() <= 64
		&& name
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

impl FetchRequest {
	pub fn validate(&self, hosts: &[String]) -> Result<(), Error> {
		crate::app::label(&self.request_id, 64)?;
		let host = https_host(&self.url).ok_or(Error::Invalid)?;
		if !hosts.contains(&host) {
			return Err(Error::Capability);
		}
		if self.headers.len() > MAX_FETCH_HEADERS {
			return Err(Error::Limit);
		}
		for (name, value) in &self.headers {
			if !valid_header_name(name)
				|| reserved_header(name)
				|| value.len() > 4096
				|| value.chars().any(char::is_control)
			{
				return Err(Error::Invalid);
			}
		}
		match &self.body {
			Some(body) if body.len() > MAX_FETCH_BODY_BYTES => Err(Error::Limit),
			Some(_) if matches!(self.method, FetchMethod::Get) => Err(Error::Invalid),
			_ => Ok(()),
		}
	}
}

impl FetchResult {
	pub fn validate(&self) -> Result<(), Error> {
		crate::app::label(&self.request_id, 64)?;
		if self.status.is_some() == self.error.is_some()
			|| self
				.status
				.is_some_and(|status| !(100..=599).contains(&status))
			|| self.error.is_some() && !self.body.is_empty()
		{
			return Err(Error::Invalid);
		}
		if self.body.len() > MAX_FETCH_RESPONSE_BYTES
			|| self
				.content_type
				.as_ref()
				.is_some_and(|kind| kind.len() > 256 || kind.chars().any(char::is_control))
		{
			return Err(Error::Limit);
		}
		const _: () = assert!(MAX_FETCH_RESPONSE_BYTES < MAX_IO_BYTES);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn request(url: &str) -> FetchRequest {
		FetchRequest {
			request_id: "r1".into(),
			method: FetchMethod::Get,
			url: url.into(),
			headers: BTreeMap::new(),
			body: None,
		}
	}

	#[test]
	fn requests_reach_only_declared_https_hosts() {
		let hosts = vec!["api.example.com".to_owned()];
		assert!(
			request("https://api.example.com/v1?q=a")
				.validate(&hosts)
				.is_ok()
		);
		assert!(
			request("https://api.example.com:443/v1")
				.validate(&hosts)
				.is_ok()
		);
		for (url, error) in [
			("https://other.example.com/", Error::Capability),
			("https://example.com/", Error::Capability),
			("https://api.example.com.evil.test/", Error::Capability),
			("http://api.example.com/", Error::Invalid),
			("https://api.example.com:8443/", Error::Invalid),
			("https://user@api.example.com/", Error::Invalid),
			("https://93.184.215.14/", Error::Invalid),
			("https://api.example.com/#frag", Error::Invalid),
			("https://api.example.com/ x", Error::Invalid),
		] {
			let got = request(url).validate(&hosts).unwrap_err();
			assert_eq!(
				std::mem::discriminant(&got),
				std::mem::discriminant(&error),
				"{url}"
			);
		}
	}

	#[test]
	fn requests_cannot_set_host_owned_headers_or_a_get_body() {
		let hosts = vec!["api.example.com".to_owned()];
		let mut keyed = request("https://api.example.com/");
		keyed
			.headers
			.insert("Authorization".into(), "Bearer key".into());
		keyed
			.headers
			.insert("Content-Type".into(), "application/json".into());
		assert!(keyed.validate(&hosts).is_ok());
		for name in [
			"Cookie",
			"Host",
			"sec-fetch-mode",
			"Proxy-Authorization",
			"Bad Name",
		] {
			let mut bad = request("https://api.example.com/");
			bad.headers.insert(name.into(), "x".into());
			assert!(bad.validate(&hosts).is_err(), "{name}");
		}
		let mut get_body = request("https://api.example.com/");
		get_body.body = Some("{}".into());
		assert!(get_body.validate(&hosts).is_err());
		get_body.method = FetchMethod::Post;
		assert!(get_body.validate(&hosts).is_ok());
		get_body.body = Some("x".repeat(MAX_FETCH_BODY_BYTES + 1));
		assert!(matches!(get_body.validate(&hosts), Err(Error::Limit)));
	}

	#[test]
	fn host_names_are_lowercase_dns_names() {
		for host in ["api.example.com", "translate.googleapis.com", "a-b.c0.io"] {
			assert!(valid_network_host(host), "{host}");
		}
		for host in [
			"localhost",
			"API.example.com",
			"127.0.0.1",
			"-a.com",
			"a..com",
			"a.123",
			"*.example.com",
		] {
			assert!(!valid_network_host(host), "{host}");
		}
	}

	#[test]
	fn results_carry_either_a_status_or_an_error() {
		let ok = FetchResult {
			request_id: "r1".into(),
			status: Some(200),
			content_type: Some("application/json".into()),
			body: "{}".into(),
			error: None,
		};
		assert!(ok.validate().is_ok());
		let failed = FetchResult {
			status: None,
			body: String::new(),
			error: Some(FetchError::TimedOut),
			..ok.clone()
		};
		assert!(failed.validate().is_ok());
		for bad in [
			FetchResult {
				error: Some(FetchError::Blocked),
				..ok.clone()
			},
			FetchResult {
				status: None,
				..ok.clone()
			},
			FetchResult {
				status: Some(42),
				..ok.clone()
			},
			FetchResult {
				body: "x".repeat(MAX_FETCH_RESPONSE_BYTES + 1),
				..ok.clone()
			},
		] {
			assert!(bad.validate().is_err(), "{bad:?}");
		}
	}
}
