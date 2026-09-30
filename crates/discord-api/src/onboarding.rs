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
		match self
			.request_limited(Method::GET, path, None, MAX_FORM_WIRE)
			.await
		{
			Ok(bytes) => Ok(Some(bytes)),
			Err(Failure::Forbidden | Failure::Protocol) => Ok(None),
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
