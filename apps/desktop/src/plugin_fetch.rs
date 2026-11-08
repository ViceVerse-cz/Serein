//! Runs plugin HTTPS requests off the UI thread, limited to each manifest's declared hosts.
//!
//! Every request yields exactly one [`FetchResult`]: a response, a failure or `cancelled`
//! when too many are already running.
use std::{sync::mpsc, time::Duration};

use extensions::{FetchError, FetchMethod, FetchRequest, FetchResult, MAX_FETCH_RESPONSE_BYTES};

/// Requests running at once across all plugins.
const MAX_RUNNING: usize = 8;
/// Whole-exchange limit, including redirects.
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 4;
/// The response must still fit an invocation once JSON-escaped.
const MAX_ESCAPED_BODY: usize = extensions::MAX_IO_BYTES - 16 * 1024;

/// A finished request for the plugin `id`, tagged with the caller's context.
pub struct Finished<C> {
	pub id: String,
	pub context: C,
	pub result: FetchResult,
}

pub struct Fetcher<C> {
	send: mpsc::Sender<Finished<C>>,
	receive: mpsc::Receiver<Finished<C>>,
	running: usize,
}

impl<C> Default for Fetcher<C> {
	fn default() -> Self {
		let (send, receive) = mpsc::channel();
		Self {
			send,
			receive,
			running: 0,
		}
	}
}

impl<C: Clone + Send + 'static> Fetcher<C> {
	/// Starts `requests` for plugin `id`; they may reach only `hosts`. `wake` runs once per
	/// finished request so the UI collects it promptly.
	pub fn start(
		&mut self,
		id: &str,
		context: &C,
		hosts: &[String],
		requests: Vec<FetchRequest>,
		wake: impl Fn() + Clone + Send + 'static,
	) {
		for request in requests {
			let finished = |result| Finished {
				id: id.to_owned(),
				context: context.clone(),
				result,
			};
			if self.running >= MAX_RUNNING {
				let _ = self.send.send(finished(failure(
					&request.request_id,
					FetchError::Cancelled,
				)));
				wake();
				continue;
			}
			self.running += 1;
			let send = self.send.clone();
			let hosts = hosts.to_vec();
			let worker_wake = wake.clone();
			let request_id = request.request_id.clone();
			let mut done = finished(failure(&request_id, FetchError::Unavailable));
			let worker_done = Finished {
				id: done.id.clone(),
				context: done.context.clone(),
				result: done.result.clone(),
			};
			let spawned = std::thread::Builder::new()
				.name("plugin-fetch".into())
				.spawn(move || {
					let mut done = worker_done;
					done.result = run(&request, &hosts);
					let _ = send.send(done);
					worker_wake();
				});
			if spawned.is_err() {
				// Still answer the plugin; the result counts down `running` like any other.
				done.result = failure(&request_id, FetchError::Unavailable);
				let _ = self.send.send(done);
				wake();
			}
		}
	}

	/// Results that finished since the last call.
	pub fn finished(&mut self) -> Vec<Finished<C>> {
		let finished: Vec<_> = self.receive.try_iter().collect();
		// Rejected requests never counted as running; saturate instead of tracking each kind.
		self.running = self.running.saturating_sub(
			finished
				.iter()
				.filter(|done| done.result.error != Some(FetchError::Cancelled))
				.count(),
		);
		finished
	}
}

fn failure(request_id: &str, error: FetchError) -> FetchResult {
	FetchResult {
		request_id: request_id.to_owned(),
		status: None,
		content_type: None,
		body: String::new(),
		error: Some(error),
	}
}

fn run(request: &FetchRequest, hosts: &[String]) -> FetchResult {
	let outcome = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.map_err(|_| FetchError::Unavailable)
		.and_then(|runtime| {
			runtime.block_on(async {
				tokio::time::timeout(TIMEOUT, exchange(request, hosts))
					.await
					.map_err(|_| FetchError::TimedOut)?
			})
		});
	match outcome {
		Ok((status, content_type, body)) => FetchResult {
			request_id: request.request_id.clone(),
			status: Some(status),
			content_type,
			body,
			error: None,
		},
		Err(error) => failure(&request.request_id, error),
	}
}

type Response = (u16, Option<String>, String);

async fn exchange(request: &FetchRequest, hosts: &[String]) -> Result<Response, FetchError> {
	let mut url = url::Url::parse(&request.url).map_err(|_| FetchError::Blocked)?;
	let mut method = request.method;
	let mut body = request.body.clone();
	for _ in 0..=MAX_REDIRECTS {
		// Every hop must stay on a declared host and a public address.
		let host = extensions::https_host(url.as_str())
			.filter(|host| hosts.iter().any(|allowed| allowed == host))
			.ok_or(FetchError::Blocked)?;
		let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), 443))
			.await
			.map_err(|_| FetchError::Unavailable)?
			.take(16)
			.collect();
		if addresses.is_empty()
			|| addresses
				.iter()
				.any(|address| !crate::extensions::public_ip(address.ip()))
		{
			return Err(FetchError::Blocked);
		}
		// Pin the checked addresses so the connection cannot resolve somewhere else.
		let client = reqwest::Client::builder()
			.no_proxy()
			.no_gzip()
			.redirect(reqwest::redirect::Policy::none())
			.resolve_to_addrs(&host, &addresses)
			.user_agent(concat!("Serein/", env!("CARGO_PKG_VERSION")))
			.build()
			.map_err(|_| FetchError::Unavailable)?;
		let verb = match method {
			FetchMethod::Get => reqwest::Method::GET,
			FetchMethod::Post => reqwest::Method::POST,
			FetchMethod::Put => reqwest::Method::PUT,
			FetchMethod::Patch => reqwest::Method::PATCH,
			FetchMethod::Delete => reqwest::Method::DELETE,
		};
		let mut builder = client
			.request(verb, url.clone())
			.header(reqwest::header::ACCEPT_ENCODING, "identity");
		for (name, value) in &request.headers {
			builder = builder.header(name.as_str(), value.as_str());
		}
		if let Some(body) = &body {
			builder = builder.body(body.clone());
		}
		let mut response = builder.send().await.map_err(|_| FetchError::Unavailable)?;
		let status = response.status();
		if status.is_redirection()
			&& let Some(location) = response.headers().get(reqwest::header::LOCATION)
		{
			let location = location.to_str().map_err(|_| FetchError::Blocked)?;
			url = url.join(location).map_err(|_| FetchError::Blocked)?;
			// 303, and 301/302 after a POST, continue as a body-less GET like browsers.
			if status == reqwest::StatusCode::SEE_OTHER
				|| matches!(status.as_u16(), 301 | 302) && method == FetchMethod::Post
			{
				method = FetchMethod::Get;
				body = None;
			}
			continue;
		}
		if response
			.headers()
			.get(reqwest::header::CONTENT_ENCODING)
			.is_some_and(|encoding| encoding != "identity")
			|| response
				.content_length()
				.is_some_and(|size| size > MAX_FETCH_RESPONSE_BYTES as u64)
		{
			return Err(FetchError::TooLarge);
		}
		let content_type = response
			.headers()
			.get(reqwest::header::CONTENT_TYPE)
			.and_then(|value| value.to_str().ok())
			.filter(|value| value.len() <= 256 && !value.chars().any(char::is_control))
			.map(str::to_owned);
		let mut bytes = Vec::new();
		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|_| FetchError::Unavailable)?
		{
			if bytes.len() + chunk.len() > MAX_FETCH_RESPONSE_BYTES {
				return Err(FetchError::TooLarge);
			}
			bytes.extend_from_slice(&chunk);
		}
		let text = String::from_utf8(bytes).map_err(|_| FetchError::NotText)?;
		if serde_json::to_string(&text).map_or(true, |escaped| escaped.len() > MAX_ESCAPED_BODY) {
			return Err(FetchError::TooLarge);
		}
		return Ok((status.as_u16(), content_type, text));
	}
	Err(FetchError::Blocked)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn request(url: &str) -> FetchRequest {
		FetchRequest {
			request_id: "r".into(),
			method: FetchMethod::Get,
			url: url.into(),
			headers: Default::default(),
			body: None,
		}
	}

	#[test]
	fn undeclared_and_private_hosts_are_blocked_before_connecting() {
		let hosts = vec!["localhost.localdomain".to_owned()];
		assert_eq!(
			run(&request("https://example.com/"), &hosts).error,
			Some(FetchError::Blocked)
		);
		assert_eq!(
			run(&request("http://localhost.localdomain/"), &hosts).error,
			Some(FetchError::Blocked)
		);
	}

	#[test]
	fn requests_past_the_running_limit_are_cancelled_with_a_result() {
		let mut fetcher = Fetcher::<()> {
			running: MAX_RUNNING,
			..Default::default()
		};
		fetcher.start(
			"plugin",
			&(),
			&[],
			vec![
				request("https://example.com/a"),
				request("https://example.com/b"),
			],
			|| {},
		);
		let finished = fetcher.finished();
		assert_eq!(finished.len(), 2);
		assert!(
			finished
				.iter()
				.all(|done| done.id == "plugin" && done.result.error == Some(FetchError::Cancelled))
		);
		assert_eq!(fetcher.running, MAX_RUNNING);
	}
}
