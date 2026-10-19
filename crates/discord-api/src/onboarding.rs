use crate::{DiscordApi, Failure};
use client_core::onboarding::{Action, Event};
use discord_protocol::onboarding::{self as wire, MAX_FORM_WIRE};
use model::{
	Id,
	onboarding::{Form, Outcome, Submission},
};
use reqwest::Method;

impl DiscordApi {
	/// A missing or inaccessible form (404/403) means that step does not apply.
	async fn optional_form(&self, path: &str) -> Result<Option<Vec<u8>>, Failure> {
		match self.request_optional_limited(path, MAX_FORM_WIRE).await {
			Ok(bytes) => Ok(bytes),
			Err(Failure::Forbidden) => Ok(None),
			Err(failure) => Err(failure),
		}
	}
	async fn load_onboarding(
		&self,
		guild: Id,
		verification: bool,
		onboarding: bool,
	) -> Result<Box<Form>, Failure> {
		let mut form = Form::default();
		// Unofficial user endpoints; documented by discord-userdoccers (resources/guild, 2026-09-30).
		if verification
			&& let Some(bytes) = self
				.optional_form(&format!(
					"/guilds/{guild}/member-verification?with_guild=false"
				))
				.await?
		{
			form.verification = wire::verification(&bytes)
				.map_err(|_| Failure::ProtocolAt("Server rules response was unsupported"))?;
		}
		if onboarding
			&& let Some(bytes) = self
				.optional_form(&format!("/guilds/{guild}/onboarding"))
				.await?
		{
			form.onboarding = wire::onboarding(&bytes)
				.map_err(|_| Failure::ProtocolAt("Server onboarding response was unsupported"))?;
		}
		Ok(Box::new(form))
	}
	async fn submit_onboarding(
		&self,
		guild: Id,
		form: &Form,
		submission: &Submission,
	) -> Result<Outcome, Failure> {
		if let (Some(onboarding), Some(chosen)) = (&form.onboarding, &submission.onboarding) {
			let now = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map_or(0, |d| d.as_millis() as u64);
			self.request_limited(
				Method::POST,
				&format!("/guilds/{guild}/onboarding-responses"),
				Some(wire::onboarding_body(onboarding, chosen, now)),
				MAX_FORM_WIRE,
			)
			.await
			.map_err(|f| f.protocol_at("Discord rejected the onboarding answers"))?;
		}
		let (Some(verification), Some(answers)) = (&form.verification, &submission.verification)
		else {
			return Ok(Outcome::Approved);
		};
		let bytes = self
			.request_limited(
				Method::PUT,
				&format!("/guilds/{guild}/requests/@me"),
				Some(wire::join_request_body(verification, answers)),
				MAX_FORM_WIRE,
			)
			.await
			.map_err(|f| f.protocol_at("Discord rejected the rules answers"))?;
		wire::join_request(&bytes).map_err(|_| Failure::Ambiguous)
	}
	pub(super) async fn onboarding(&self, guild: Id, request: u64, action: Action) -> Event {
		if guild.0 == 0 {
			return Event::Loaded {
				guild,
				request,
				result: Err(Failure::Protocol),
			};
		}
		match action {
			Action::Load {
				verification,
				onboarding,
			} => Event::Loaded {
				guild,
				request,
				result: self.load_onboarding(guild, verification, onboarding).await,
			},
			Action::Submit { form, submission } => Event::Submitted {
				guild,
				request,
				result: self.submit_onboarding(guild, &form, &submission).await,
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::auth::SessionSecret;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	async fn fixture(
		responses: Vec<(&'static str, &'static str)>,
	) -> (DiscordApi, tokio::task::JoinHandle<()>) {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_ONBOARDING_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			for (status, body) in responses {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut request = Vec::new();
				while !request.ends_with(b"\r\n\r\n") {
					assert!(request.len() < 4096);
					request.push(socket.read_u8().await.unwrap());
				}
				assert!(request.starts_with(b"GET /guilds/1/"));
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
		});
		(api, server)
	}

	#[tokio::test]
	async fn onboarding_unexpected_http_failures_remain_errors() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let (api, server) = fixture(vec![
				("500 Internal Server Error", "{}"),
				("502 Bad Gateway", "{}"),
				("400 Bad Request", "{}"),
			])
			.await;
			let mut outcomes = Vec::new();
			for _ in 0..3 {
				outcomes.push(api.load_onboarding(Id(1), true, false).await);
			}
			server.await.unwrap();
			assert!(outcomes.iter().all(Result::is_err));
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn onboarding_missing_forms_are_optional_but_malformed_success_is_rejected() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let (api, server) = fixture(vec![
				("404 Not Found", "{}"),
				("403 Forbidden", "{}"),
				("200 OK", "not json"),
			])
			.await;
			for _ in 0..2 {
				assert!(
					api.load_onboarding(Id(1), true, false)
						.await
						.unwrap()
						.is_empty()
				);
			}
			assert!(api.load_onboarding(Id(1), true, false).await.is_err());
			server.await.unwrap();
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn onboarding_optional_statuses_preserve_session_failures() {
		tokio::time::timeout(Duration::from_secs(10), async {
			for (status, body, failure) in [
				("401 Unauthorized", "{}", Failure::Expired),
				("403 Forbidden", r#"{"code":60003}"#, Failure::Challenged),
				("404 Not Found", r#"{"code":50014}"#, Failure::Challenged),
			] {
				let (api, server) = fixture(vec![(status, body)]).await;
				assert_eq!(
					api.load_onboarding(Id(1), true, false).await.unwrap_err(),
					failure
				);
				assert!(api.stopped());
				server.await.unwrap();
			}
		})
		.await
		.unwrap();
	}
}
