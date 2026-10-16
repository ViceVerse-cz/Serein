//! Read-only badge catalog enrichment and the account's own badge settings write.
use crate::{DiscordApi, Failure};
use discord_protocol::badges;
use model::{
	Id,
	badges::{BadgeCatalog, BadgeSettings},
};
use reqwest::Method;

impl DiscordApi {
	/// Optional catalog beside a profile. Failures other than a lost session leave the
	/// profile with its own badge list.
	pub(super) async fn load_badge_catalog(
		&self,
		user: Id,
		own: bool,
	) -> Result<Option<BadgeCatalog>, Failure> {
		match self
			.request_limited(
				Method::GET,
				&catalog_path(user, own),
				None,
				badges::MAX_CATALOG_WIRE,
			)
			.await
		{
			Ok(bytes) => Ok(badges::decode_catalog(&bytes).ok()),
			// An oversized optional response must not discard the already loaded profile.
			Err(Failure::Capacity) => Ok(None),
			Err(failure) if failure.ends_session() => Err(failure),
			Err(_) => Ok(None),
		}
	}

	/// One explicitly requested write, without retries.
	pub(super) async fn save_badge_settings(
		&self,
		settings: BadgeSettings,
	) -> Result<BadgeSettings, Failure> {
		let body = badges::encode_settings(&settings).map_err(|_| Failure::Protocol)?;
		self.request_limited(
			Method::PATCH,
			"/users/@me/badges/settings",
			Some(body),
			64 * 1024,
		)
		.await?;
		Ok(settings)
	}
}

/// The official client reads the account's own catalog through `@me`.
fn catalog_path(user: Id, own: bool) -> String {
	if own {
		"/users/@me/badges".into()
	} else {
		format!("/users/{user}/badges")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn own_catalog_uses_the_current_user_route() {
		assert_eq!(catalog_path(Id(5), true), "/users/@me/badges");
		assert_eq!(catalog_path(Id(5), false), "/users/5/badges");
	}
}
