//! Public breach-source lookups for identifiers a profile owner publishes themselves.
//! These calls carry no credentials and never touch Discord; they ask free public
//! breach indexes (LeakCheck Public, XposedOrNot) which report source names and exposed
//! field categories only — never passwords or raw records.
use std::time::Duration;

use client_core::breach::{self, BreachReport, Source, push_bounded};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::Failure;

/// Responses are small JSON summaries; anything larger is treated as malformed.
const MAX_RESPONSE: usize = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Bound on one identifier so a hostile profile cannot amplify outbound queries.
pub const MAX_QUERY_CHARS: usize = 254;

static CLIENT: OnceCell<Client> = OnceCell::const_new();

async fn client() -> Result<&'static Client, Failure> {
	CLIENT
		.get_or_try_init(|| async {
			Client::builder()
				.timeout(REQUEST_TIMEOUT)
				.redirect(reqwest::redirect::Policy::limited(3))
				.build()
				.map_err(|_| Failure::Protocol)
		})
		.await
}

#[derive(Deserialize)]
struct LeakCheckReply {
	success: bool,
	#[serde(default)]
	found: usize,
	#[serde(default)]
	fields: Vec<String>,
	#[serde(default)]
	sources: Vec<LeakCheckSource>,
}
#[derive(Deserialize)]
struct LeakCheckSource {
	name: String,
}

#[derive(Deserialize)]
struct XposedReply {
	status: Option<String>,
	#[serde(default)]
	breaches: Vec<Vec<serde_json::Value>>,
}

/// A query is an email or a handle: printable, no whitespace, bounded length.
pub fn valid_query(query: &str) -> bool {
	!query.is_empty()
		&& query.chars().count() <= MAX_QUERY_CHARS
		&& query
			.chars()
			.all(|c| c.is_ascii() && !c.is_control() && !c.is_whitespace())
}

async fn get(url: &str) -> Result<Vec<u8>, Failure> {
	let response = client()
		.await?
		.get(url)
		.send()
		.await
		.map_err(|_| Failure::Network)?;
	if !response.status().is_success() {
		return Err(Failure::Protocol);
	}
	let bytes = response.bytes().await.map_err(|_| Failure::Network)?;
	if bytes.len() > MAX_RESPONSE {
		return Err(Failure::Protocol);
	}
	Ok(bytes.to_vec())
}

fn parse_leakcheck(bytes: &[u8]) -> Result<BreachReport, Failure> {
	let reply: LeakCheckReply = serde_json::from_slice(bytes).map_err(|_| Failure::Protocol)?;
	if !reply.success {
		return Err(Failure::Protocol);
	}
	let mut report = BreachReport {
		found: reply.found.min(10_000),
		source: Source::LeakCheck.name(),
		..Default::default()
	};
	for field in &reply.fields {
		push_bounded(&mut report.fields, field, breach::MAX_FIELDS);
	}
	for source in &reply.sources {
		push_bounded(&mut report.sources, &source.name, breach::MAX_SOURCES);
	}
	report.sources.sort();
	Ok(report)
}

fn parse_xposed(bytes: &[u8]) -> Result<BreachReport, Failure> {
	let reply: XposedReply = serde_json::from_slice(bytes).map_err(|_| Failure::Protocol)?;
	if reply.status.as_deref() != Some("success") {
		return Err(Failure::Protocol);
	}
	let mut report = BreachReport {
		found: reply.breaches.len(),
		source: Source::XposedOrNot.name(),
		..Default::default()
	};
	for breach in &reply.breaches {
		if let Some(name) = breach.first().and_then(|v| v.as_str()) {
			push_bounded(&mut report.sources, name, breach::MAX_SOURCES);
		}
	}
	report.sources.sort();
	Ok(report)
}

/// One bounded lookup against a single public index. Rate limiting is the caller's job.
pub async fn lookup(source: Source, query: &str) -> Result<BreachReport, Failure> {
	if !valid_query(query) {
		return Err(Failure::Protocol);
	}
	let encoded = urlencoding(query);
	let url = match source {
		Source::LeakCheck => format!("https://leakcheck.io/api/public?check={encoded}"),
		Source::XposedOrNot => {
			// XposedOrNot only accepts email queries on the free endpoint.
			if !query.contains('@') {
				return Err(Failure::Protocol);
			}
			format!("https://api.xposedornot.com/v1/check-email/{encoded}")
		}
	};
	let bytes = get(&url).await?;
	match source {
		Source::LeakCheck => parse_leakcheck(&bytes),
		Source::XposedOrNot => parse_xposed(&bytes),
	}
}

/// Percent-encode the query's non-unreserved bytes; inputs are already charset-validated.
fn urlencoding(query: &str) -> String {
	let mut out = String::with_capacity(query.len());
	for byte in query.bytes() {
		match byte {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
				out.push(byte as char)
			}
			other => out.push_str(&format!("%{other:02X}")),
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn queries_stay_bounded_and_single_token() {
		assert!(valid_query("user@example.com"));
		assert!(valid_query("some.handle_01"));
		assert!(!valid_query(""));
		assert!(!valid_query("has space"));
		assert!(!valid_query("line\nbreak"));
		assert!(!valid_query(&"a".repeat(MAX_QUERY_CHARS + 1)));
		assert!(valid_query(&"a".repeat(MAX_QUERY_CHARS)));
	}

	#[test]
	fn leakcheck_parses_sources_and_fields() {
		let body = br#"{"success":true,"found":3,"fields":["username","first_name"],"sources":[{"name":"Evony.com","date":"2016-07"},{"name":"Zynga.com","date":"2019-09"}]}"#;
		let report = parse_leakcheck(body).unwrap();
		assert_eq!(report.found, 3);
		assert_eq!(report.sources, vec!["Evony.com", "Zynga.com"]);
		assert_eq!(report.fields, vec!["username", "first_name"]);
		assert_eq!(report.source, "leakcheck");
	}

	#[test]
	fn leakcheck_rejects_unsuccessful_and_malformed() {
		assert!(parse_leakcheck(br#"{"success":false}"#).is_err());
		assert!(parse_leakcheck(b"not json").is_err());
		assert!(parse_leakcheck(br#"{"found":1}"#).is_err());
	}

	#[test]
	fn xposed_parses_breach_names_only() {
		let body = br#"{"status":"success","email":"a@b.c","breaches":[["Adobe","2013"],["LinkedIn","2012"]]}"#;
		let report = parse_xposed(body).unwrap();
		assert_eq!(report.found, 2);
		assert_eq!(report.sources, vec!["Adobe", "LinkedIn"]);
		assert_eq!(report.source, "xposedornot");
	}

	#[test]
	fn xposed_rejects_non_success() {
		assert!(parse_xposed(br#"{"status":"error"}"#).is_err());
		assert!(parse_xposed(br#"{"breaches":[]}"#).is_err());
	}

	#[test]
	fn reports_bound_source_and_field_counts() {
		let sources = (0..40)
			.map(|i| format!("{{\"name\":\"Breach{i}\"}}"))
			.collect::<Vec<_>>()
			.join(",");
		let body = format!(
			"{{\"success\":true,\"found\":40,\"fields\":[\"{}\"],\"sources\":[{}]}}",
			"a".repeat(200),
			sources
		);
		let report = parse_leakcheck(body.as_bytes()).unwrap();
		assert!(report.sources.len() <= breach::MAX_SOURCES);
		assert!(
			report
				.fields
				.iter()
				.all(|f| f.chars().count() <= breach::MAX_NAME_CHARS)
		);
	}

	#[test]
	fn urlencoding_covers_reserved_bytes() {
		assert_eq!(urlencoding("a@b.c"), "a%40b.c");
		assert_eq!(urlencoding("user-name_1.x"), "user-name_1.x");
	}
}
